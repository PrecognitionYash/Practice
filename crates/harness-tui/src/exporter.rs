// ──────────────────────────────────────────────────────────────────────
// harness-tui/src/exporter.rs — Multi-Format Arrow / Parquet / CSV / JSONL Exporter
//
// Streams datasets from the SQLite WAL store into high-performance
// formats:
//  • Apache Parquet (zero-copy Arrow RecordBatch with Snappy compression)
//  • Comma-Separated Values (CSV)
//  • Newline-Delimited JSON (JSONL)
// ──────────────────────────────────────────────────────────────────────

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use harness_core::error::{HarnessError, HarnessResult};
use harness_core::store::DataStore;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use tracing::{debug, info, instrument};

use crate::commands::ExportFormat;

/// Summary of an export operation.
#[derive(Debug, Clone)]
pub struct ExportSummary {
    /// Destination file path on disk.
    pub file_path: PathBuf,
    /// Number of records successfully exported.
    pub records_exported: usize,
    /// Format used.
    pub format: ExportFormat,
}

/// The multi-format dataset exporter.
#[derive(Debug, Clone)]
pub struct DataExporter {
    store: DataStore,
    output_dir: PathBuf,
}

impl DataExporter {
    /// Create a new data exporter targeting the given directory (default: `.harness/exports`).
    pub fn new(store: DataStore, output_dir: impl AsRef<Path>) -> HarnessResult<Self> {
        let dir = output_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(HarnessError::Io)?;
        Ok(Self {
            store,
            output_dir: dir,
        })
    }

    /// Export a named SQLite table to the requested format.
    #[instrument(skip(self), fields(table = %table, format = ?format))]
    pub async fn export_table(
        &self,
        table: &str,
        format: ExportFormat,
    ) -> HarnessResult<ExportSummary> {
        // Query raw rows from SQLite WAL
        let rows = self.store.query_table(table, None).await?;
        if rows.is_empty() {
            info!(table, "table is empty; 0 rows to export");
        }

        let extension = match format {
            ExportFormat::Parquet => "parquet",
            ExportFormat::Csv => "csv",
            ExportFormat::Jsonl => "jsonl",
        };

        let file_path = self.output_dir.join(format!("{table}.{extension}"));

        match format {
            ExportFormat::Parquet => self.write_parquet(&file_path, &rows)?,
            ExportFormat::Csv => self.write_csv(&file_path, &rows)?,
            ExportFormat::Jsonl => self.write_jsonl(&file_path, &rows)?,
        }

        info!(
            path = %file_path.display(),
            records = rows.len(),
            format = ?format,
            "table export complete"
        );

        Ok(ExportSummary {
            file_path,
            records_exported: rows.len(),
            format,
        })
    }

    /// Writes data using Apache Arrow RecordBatches to a Snappy-compressed Parquet file.
    fn write_parquet(&self, path: &Path, rows: &[serde_json::Value]) -> HarnessResult<()> {
        let mut ids = Vec::with_capacity(rows.len());
        let mut urls = Vec::with_capacity(rows.len());
        let mut payloads = Vec::with_capacity(rows.len());
        let mut timestamps = Vec::with_capacity(rows.len());

        for r in rows {
            ids.push(r["id"].as_i64().unwrap_or(0));
            urls.push(r["source_url"].as_str().unwrap_or(""));
            let p_str = r["payload"].to_string();
            payloads.push(p_str);
            timestamps.push(r["extracted_at"].as_str().unwrap_or(""));
        }

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("source_url", DataType::Utf8, false),
            Field::new("payload", DataType::Utf8, false),
            Field::new("extracted_at", DataType::Utf8, false),
        ]));

        let id_array = Arc::new(Int64Array::from(ids));
        let url_array = Arc::new(StringArray::from(urls));
        let payload_array = Arc::new(StringArray::from(
            payloads.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        ));
        let ts_array = Arc::new(StringArray::from(timestamps));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![id_array, url_array, payload_array, ts_array],
        )
        .map_err(|e| {
            HarnessError::Internal(format!("failed to construct Arrow RecordBatch: {e}"))
        })?;

        let file = File::create(path).map_err(HarnessError::Io)?;
        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();

        let mut writer = ArrowWriter::try_new(file, schema, Some(props))
            .map_err(|e| HarnessError::Internal(format!("failed to create Parquet writer: {e}")))?;

        writer.write(&batch).map_err(|e| {
            HarnessError::Internal(format!("failed to write Parquet RecordBatch: {e}"))
        })?;

        writer
            .close()
            .map_err(|e| HarnessError::Internal(format!("failed to finalize Parquet file: {e}")))?;

        debug!(path = %path.display(), "wrote Parquet file with Snappy compression");
        Ok(())
    }

    /// Writes data rows to a standard CSV file.
    fn write_csv(&self, path: &Path, rows: &[serde_json::Value]) -> HarnessResult<()> {
        let file = File::create(path).map_err(HarnessError::Io)?;
        let mut wtr = csv::Writer::from_writer(file);

        // Header
        wtr.write_record(["id", "source_url", "payload", "extracted_at"])
            .map_err(|e| HarnessError::Internal(format!("CSV write header error: {e}")))?;

        for r in rows {
            let id = r["id"].as_i64().unwrap_or(0).to_string();
            let url = r["source_url"].as_str().unwrap_or("").to_string();
            let payload = r["payload"].to_string();
            let ts = r["extracted_at"].as_str().unwrap_or("").to_string();

            wtr.write_record([&id, &url, &payload, &ts])
                .map_err(|e| HarnessError::Internal(format!("CSV write row error: {e}")))?;
        }

        wtr.flush()
            .map_err(|e| HarnessError::Internal(format!("CSV flush error: {e}")))?;

        debug!(path = %path.display(), "wrote CSV file");
        Ok(())
    }

    /// Writes data rows as newline-delimited JSON.
    fn write_jsonl(&self, path: &Path, rows: &[serde_json::Value]) -> HarnessResult<()> {
        use std::io::Write;
        let mut file = File::create(path).map_err(HarnessError::Io)?;

        for r in rows {
            let line = serde_json::to_string(r).map_err(HarnessError::Json)?;
            writeln!(file, "{line}").map_err(HarnessError::Io)?;
        }

        file.flush().map_err(HarnessError::Io)?;
        debug!(path = %path.display(), "wrote JSONL file");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use harness_core::schema::DataRow;

    #[tokio::test]
    async fn test_parquet_and_csv_export() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("harvest.db");
        let store = DataStore::open(&db_path).await.unwrap();

        // Insert sample rows
        let row1 = DataRow {
            source_url: "https://example.com/item/1".into(),
            table_name: "test_export".into(),
            payload: serde_json::json!({"name": "Alpha", "score": 98}),
            extracted_at: Utc::now(),
        };
        store.insert_row(&row1).await.unwrap();

        let exporter = DataExporter::new(store, dir.path().join("exports")).unwrap();

        // 1. Export Parquet
        let p_res = exporter
            .export_table("test_export", ExportFormat::Parquet)
            .await
            .unwrap();
        assert_eq!(p_res.records_exported, 1);
        assert!(p_res.file_path.exists());

        // 2. Export CSV
        let c_res = exporter
            .export_table("test_export", ExportFormat::Csv)
            .await
            .unwrap();
        assert_eq!(c_res.records_exported, 1);
        assert!(c_res.file_path.exists());

        // 3. Export JSONL
        let j_res = exporter
            .export_table("test_export", ExportFormat::Jsonl)
            .await
            .unwrap();
        assert_eq!(j_res.records_exported, 1);
        assert!(j_res.file_path.exists());
    }
}
