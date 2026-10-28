// ──────────────────────────────────────────────────────────────────────
// harness-core/src/store.rs — High-Throughput SQLite WAL Datastore
//
// Central persistence layer for the harness.  All extracted data,
// macro caches, and execution logs flow through this store.
//
// Design decisions:
//  • WAL journal mode for concurrent read/write without blocking.
//  • Synchronous=NORMAL for balanced durability/performance.
//  • Busy timeout of 5 s to handle transient writer contention.
//  • Dynamic table creation: tables are created on-demand from
//    incoming DataRow payloads, so the sniffer/crawler never needs
//    to know the schema in advance.
// ──────────────────────────────────────────────────────────────────────

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use tracing::{debug, info, instrument};

use crate::error::{HarnessError, HarnessResult};
use crate::schema::{DataRow, MacroRecord, PlannedAction};

/// The SQLite WAL datastore.
///
/// Thread-safe via interior `SqlitePool` (connection pool).
/// All public methods are `&self`.
#[derive(Debug, Clone)]
pub struct DataStore {
    pool: SqlitePool,
    db_path: PathBuf,
}

impl DataStore {
    /// Open (or create) the SQLite database at the given path.
    ///
    /// Applies WAL journal mode and runs the bootstrap migration
    /// to create system tables (`_meta`, `macro_cache`).
    ///
    /// # Errors
    /// Returns `HarnessError::Database` on connection or migration failures.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub async fn open(path: impl AsRef<Path> + std::fmt::Debug) -> HarnessResult<Self> {
        let db_path = path.as_ref().to_path_buf();

        // Ensure the parent directory exists.
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HarnessError::VaultIo {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;

        let store = Self { pool, db_path };
        store.run_migrations().await?;

        info!(path = %store.db_path.display(), "datastore opened");
        Ok(store)
    }

    /// Run the bootstrap migration to create system tables.
    async fn run_migrations(&self) -> HarnessResult<()> {
        // ── _meta table: stores harness metadata / versioning.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS _meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Set schema version.
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO _meta (key, value)
            VALUES ('schema_version', '1')
            "#,
        )
        .execute(&self.pool)
        .await?;

        // ── macro_cache: stores deterministic macro recordings.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS macro_cache (
                cache_key   INTEGER PRIMARY KEY,
                domain      TEXT    NOT NULL,
                prompt      TEXT    NOT NULL,
                actions     TEXT    NOT NULL,
                verified_at TEXT    NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // ── execution_log: audit trail of all actions taken.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS execution_log (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   TEXT    NOT NULL DEFAULT (datetime('now')),
                event_type  TEXT    NOT NULL,
                detail      TEXT    NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        debug!("migrations applied");
        Ok(())
    }

    // ── Data Row Operations ─────────────────────────────────────────

    /// Insert a generic data row into a dynamically-named table.
    ///
    /// If the table does not exist it is created with columns
    /// `(id, source_url, payload, extracted_at)`.  The payload is
    /// stored as a JSON text blob.
    #[instrument(skip(self, row), fields(table = %row.table_name))]
    pub async fn insert_row(&self, row: &DataRow) -> HarnessResult<i64> {
        self.ensure_data_table(&row.table_name).await?;

        let payload_json = serde_json::to_string(&row.payload)?;
        let ts = row.extracted_at.to_rfc3339();

        let result = sqlx::query(&format!(
            "INSERT INTO \"{}\" (source_url, payload, extracted_at) VALUES (?, ?, ?)",
            Self::sanitise_table_name(&row.table_name)
        ))
        .bind(&row.source_url)
        .bind(&payload_json)
        .bind(&ts)
        .execute(&self.pool)
        .await?;

        let id = result.last_insert_rowid();
        debug!(id, "row inserted");
        Ok(id)
    }

    /// Insert a batch of data rows in a single transaction.
    ///
    /// Significantly faster than individual inserts for bulk sniffer
    /// or crawler output.
    #[instrument(skip(self, rows), fields(count = rows.len()))]
    pub async fn insert_rows(&self, rows: &[DataRow]) -> HarnessResult<Vec<i64>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        // Ensure all destination tables exist before starting the txn.
        let mut tables_seen = std::collections::HashSet::new();
        for row in rows {
            if tables_seen.insert(row.table_name.clone()) {
                self.ensure_data_table(&row.table_name).await?;
            }
        }

        let mut tx = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(rows.len());

        for row in rows {
            let payload_json = serde_json::to_string(&row.payload)?;
            let ts = row.extracted_at.to_rfc3339();

            let result = sqlx::query(&format!(
                "INSERT INTO \"{}\" (source_url, payload, extracted_at) VALUES (?, ?, ?)",
                Self::sanitise_table_name(&row.table_name)
            ))
            .bind(&row.source_url)
            .bind(&payload_json)
            .bind(&ts)
            .execute(&mut *tx)
            .await?;

            ids.push(result.last_insert_rowid());
        }

        tx.commit().await?;
        debug!(inserted = ids.len(), "batch insert complete");
        Ok(ids)
    }

    /// Query all rows from a named table, returning them as JSON values.
    #[instrument(skip(self))]
    pub async fn query_table(
        &self,
        table: &str,
        limit: Option<u32>,
    ) -> HarnessResult<Vec<serde_json::Value>> {
        if !self.table_exists(table).await? {
            return Err(HarnessError::TableNotFound {
                table: table.to_string(),
            });
        }

        let safe_table = Self::sanitise_table_name(table);
        let limit_clause = limit.map_or(String::new(), |n| format!(" LIMIT {n}"));
        let query_str = format!(
            "SELECT id, source_url, payload, extracted_at FROM \"{safe_table}\"{limit_clause}"
        );

        let rows = sqlx::query(&query_str).fetch_all(&self.pool).await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: i64 = row.try_get("id")?;
            let source_url: String = row.try_get("source_url")?;
            let payload_str: String = row.try_get("payload")?;
            let extracted_at: String = row.try_get("extracted_at")?;

            let payload: serde_json::Value = serde_json::from_str(&payload_str)
                .unwrap_or(serde_json::Value::String(payload_str));

            results.push(serde_json::json!({
                "id": id,
                "source_url": source_url,
                "payload": payload,
                "extracted_at": extracted_at,
            }));
        }

        Ok(results)
    }

    /// Return the row count for a given table.
    pub async fn row_count(&self, table: &str) -> HarnessResult<u64> {
        if !self.table_exists(table).await? {
            return Err(HarnessError::TableNotFound {
                table: table.to_string(),
            });
        }
        let safe = Self::sanitise_table_name(table);
        let row = sqlx::query(&format!("SELECT COUNT(*) as cnt FROM \"{safe}\""))
            .fetch_one(&self.pool)
            .await?;
        let count: i64 = row.try_get("cnt")?;
        Ok(count as u64)
    }

    /// List all user-created data tables (excludes system and SQLite internal tables).
    pub async fn list_tables(&self) -> HarnessResult<Vec<String>> {
        let rows = sqlx::query(
            r#"
            SELECT name FROM sqlite_master
            WHERE type = 'table'
              AND name NOT LIKE '\_%' ESCAPE '\'
              AND name NOT LIKE 'sqlite_%'
              AND name NOT IN ('macro_cache', 'execution_log')
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let tables: Vec<String> = rows.iter().map(|r| r.get("name")).collect();
        Ok(tables)
    }

    /// Completely reset all harvested datasets and runtime state in the database.
    ///
    /// Drops all user-created data tables and clears logs and macro cache.
    /// Returns the number of user data tables dropped.
    pub async fn reset_all_data(&self) -> HarnessResult<usize> {
        let tables = self.list_tables().await?;
        let count = tables.len();

        for t in &tables {
            let safe = Self::sanitise_table_name(t);
            sqlx::query(&format!("DROP TABLE IF EXISTS \"{safe}\""))
                .execute(&self.pool)
                .await?;
        }

        // Clear execution log
        let _ = sqlx::query("DELETE FROM execution_log")
            .execute(&self.pool)
            .await;

        // Clear macro cache
        let _ = sqlx::query("DELETE FROM macro_cache")
            .execute(&self.pool)
            .await;

        // Reset autoincrement sequence if present
        let _ = sqlx::query("DELETE FROM sqlite_sequence")
            .execute(&self.pool)
            .await;

        // Checkpoint WAL and optimize
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await;

        Ok(count)
    }

    // ── Macro Cache Operations ──────────────────────────────────────

    /// Store a macro recording in the cache.
    #[instrument(skip(self, record), fields(key = record.cache_key, domain = %record.domain))]
    pub async fn store_macro(&self, record: &MacroRecord) -> HarnessResult<()> {
        let actions_json = serde_json::to_string(&record.actions)?;
        let verified = record.verified_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT OR REPLACE INTO macro_cache (cache_key, domain, prompt, actions, verified_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(record.cache_key as i64)
        .bind(&record.domain)
        .bind(&record.prompt)
        .bind(&actions_json)
        .bind(&verified)
        .execute(&self.pool)
        .await?;

        debug!("macro stored");
        Ok(())
    }

    /// Look up a macro by cache key.
    pub async fn lookup_macro(&self, cache_key: u64) -> HarnessResult<Option<MacroRecord>> {
        let row = sqlx::query(
            "SELECT cache_key, domain, prompt, actions, verified_at FROM macro_cache WHERE cache_key = ?",
        )
        .bind(cache_key as i64)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            None => Ok(None),
            Some(r) => {
                let actions_str: String = r.try_get("actions")?;
                let actions: Vec<PlannedAction> = serde_json::from_str(&actions_str)?;
                let verified_str: String = r.try_get("verified_at")?;
                let verified_at = chrono::DateTime::parse_from_rfc3339(&verified_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| HarnessError::Internal(format!("bad timestamp: {e}")))?;

                Ok(Some(MacroRecord {
                    cache_key: r.try_get::<i64, _>("cache_key")? as u64,
                    domain: r.try_get("domain")?,
                    prompt: r.try_get("prompt")?,
                    actions,
                    verified_at,
                }))
            }
        }
    }

    /// Delete a macro cache entry (used during self-healing fallback).
    pub async fn purge_macro(&self, cache_key: u64) -> HarnessResult<bool> {
        let result = sqlx::query("DELETE FROM macro_cache WHERE cache_key = ?")
            .bind(cache_key as i64)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── Execution Log ───────────────────────────────────────────────

    /// Append an entry to the execution audit log.
    pub async fn log_event(&self, event_type: &str, detail: &str) -> HarnessResult<()> {
        sqlx::query("INSERT INTO execution_log (event_type, detail) VALUES (?, ?)")
            .bind(event_type)
            .bind(detail)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Retrieve recent log entries.
    pub async fn recent_logs(&self, limit: u32) -> HarnessResult<Vec<serde_json::Value>> {
        let rows = sqlx::query(
            "SELECT id, timestamp, event_type, detail FROM execution_log ORDER BY id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let entries: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<i64, _>("id"),
                    "timestamp": r.get::<String, _>("timestamp"),
                    "event_type": r.get::<String, _>("event_type"),
                    "detail": r.get::<String, _>("detail"),
                })
            })
            .collect();
        Ok(entries)
    }

    // ── Internal helpers ────────────────────────────────────────────

    /// Ensure a data table with the standard schema exists.
    async fn ensure_data_table(&self, name: &str) -> HarnessResult<()> {
        let safe = Self::sanitise_table_name(name);
        sqlx::query(&format!(
            r#"
            CREATE TABLE IF NOT EXISTS "{safe}" (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                source_url   TEXT    NOT NULL,
                payload      TEXT    NOT NULL,
                extracted_at TEXT    NOT NULL
            )
            "#,
        ))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Check whether a table exists in the database.
    async fn table_exists(&self, name: &str) -> HarnessResult<bool> {
        let safe = Self::sanitise_table_name(name);
        let row =
            sqlx::query("SELECT COUNT(*) as cnt FROM sqlite_master WHERE type='table' AND name=?")
                .bind(&safe)
                .fetch_one(&self.pool)
                .await?;
        let cnt: i64 = row.try_get("cnt")?;
        Ok(cnt > 0)
    }

    /// Sanitise a user-provided table name to prevent SQL injection.
    ///
    /// Only allows alphanumeric characters and underscores.
    fn sanitise_table_name(name: &str) -> String {
        name.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    }

    /// Return a reference to the underlying connection pool (for
    /// advanced queries in downstream crates).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Return the database file path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{InteractionKind, PlannedAction};
    use chrono::Utc;

    async fn temp_store() -> (DataStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join(".harness").join("harvest.db");
        let store = DataStore::open(&db_path).await.unwrap();
        (store, dir)
    }

    #[tokio::test]
    async fn open_creates_tables() {
        let (store, _dir) = temp_store().await;
        // System tables should exist.
        let row = sqlx::query("SELECT value FROM _meta WHERE key = 'schema_version'")
            .fetch_one(store.pool())
            .await
            .unwrap();
        let version: String = row.get("value");
        assert_eq!(version, "1");
    }

    #[tokio::test]
    async fn insert_and_query_rows() {
        let (store, _dir) = temp_store().await;

        let row = DataRow {
            source_url: "https://example.com/api/products".into(),
            table_name: "products".into(),
            payload: serde_json::json!({"name": "Widget", "price": 9.99}),
            extracted_at: Utc::now(),
        };

        let id = store.insert_row(&row).await.unwrap();
        assert_eq!(id, 1);

        let results = store.query_table("products", None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["payload"]["name"], "Widget");
    }

    #[tokio::test]
    async fn batch_insert() {
        let (store, _dir) = temp_store().await;

        let rows: Vec<DataRow> = (0..50)
            .map(|i| DataRow {
                source_url: format!("https://example.com/item/{i}"),
                table_name: "items".into(),
                payload: serde_json::json!({"index": i}),
                extracted_at: Utc::now(),
            })
            .collect();

        let ids = store.insert_rows(&rows).await.unwrap();
        assert_eq!(ids.len(), 50);

        let count = store.row_count("items").await.unwrap();
        assert_eq!(count, 50);
    }

    #[tokio::test]
    async fn query_missing_table_returns_error() {
        let (store, _dir) = temp_store().await;
        let result = store.query_table("nonexistent", None).await;
        assert!(matches!(result, Err(HarnessError::TableNotFound { .. })));
    }

    #[tokio::test]
    async fn list_tables_excludes_system() {
        let (store, _dir) = temp_store().await;

        // Insert a row to create a user table.
        store
            .insert_row(&DataRow {
                source_url: "https://x.com".into(),
                table_name: "my_data".into(),
                payload: serde_json::json!({}),
                extracted_at: Utc::now(),
            })
            .await
            .unwrap();

        let tables = store.list_tables().await.unwrap();
        assert!(tables.contains(&"my_data".to_string()));
        // System and internal tables must NOT appear.
        assert!(!tables.contains(&"_meta".to_string()));
        assert!(!tables.contains(&"macro_cache".to_string()));
        assert!(!tables.contains(&"execution_log".to_string()));
        assert!(!tables.contains(&"sqlite_sequence".to_string()));
    }

    #[tokio::test]
    async fn reset_all_data_works() {
        let (store, _dir) = temp_store().await;

        store
            .insert_row(&DataRow {
                source_url: "https://x.com".into(),
                table_name: "test_purge".into(),
                payload: serde_json::json!({"test": 123}),
                extracted_at: Utc::now(),
            })
            .await
            .unwrap();

        assert_eq!(store.list_tables().await.unwrap().len(), 1);

        let purged = store.reset_all_data().await.unwrap();
        assert_eq!(purged, 1);
        assert_eq!(store.list_tables().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn macro_store_lookup_purge() {
        let (store, _dir) = temp_store().await;

        let record = MacroRecord {
            cache_key: 0xDEAD_BEEF_CAFE_1234,
            domain: "app.example.com".into(),
            prompt: "login and export report".into(),
            actions: vec![
                PlannedAction {
                    mark_id: 1,
                    action: InteractionKind::Click,
                    text: None,
                },
                PlannedAction {
                    mark_id: 2,
                    action: InteractionKind::Type,
                    text: Some("admin@example.com".into()),
                },
            ],
            verified_at: Utc::now(),
        };

        store.store_macro(&record).await.unwrap();

        let found = store
            .lookup_macro(0xDEAD_BEEF_CAFE_1234)
            .await
            .unwrap()
            .expect("macro should exist");
        assert_eq!(found.domain, "app.example.com");
        assert_eq!(found.actions.len(), 2);

        // Purge
        assert!(store.purge_macro(0xDEAD_BEEF_CAFE_1234).await.unwrap());
        assert!(store
            .lookup_macro(0xDEAD_BEEF_CAFE_1234)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn execution_log() {
        let (store, _dir) = temp_store().await;

        store
            .log_event("navigate", "https://example.com")
            .await
            .unwrap();
        store.log_event("click", "@3 Submit").await.unwrap();

        let logs = store.recent_logs(10).await.unwrap();
        assert_eq!(logs.len(), 2);
        // Most recent first.
        assert_eq!(logs[0]["event_type"], "click");
        assert_eq!(logs[1]["event_type"], "navigate");
    }
}
