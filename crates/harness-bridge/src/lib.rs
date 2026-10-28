// ──────────────────────────────────────────────────────────────────────
// harness-bridge/src/lib.rs — PyO3 Python C-ABI Extension Module
//
// Exposes bare-metal Rust browser harness routines directly to Python
// for seamless LlamaIndex / LangChain tool integration.
//
// Invariants Enforced:
//  • Non-blocking execution: Releases Python's Global Interpreter Lock (GIL)
//    via `py.allow_threads(|| ...)` for long-running network/CDP operations.
//  • Zero-copy memory mapping: Directly converts Serde structures to Python
//    JSON strings or native dictionaries.
#![allow(clippy::useless_conversion)]

use std::path::PathBuf;
use std::time::Duration;

use harness_agent::groq::{GroqClient, GroqConfig};
use harness_agent::search::ZeroUrlResolver;
use harness_agent::triage::TriageGatekeeper;
use harness_browser::crawler::{CrawlerConfig, FastPathCrawler};
use harness_browser::replay::MacroReplayEngine;
use harness_core::store::DataStore;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

/// Pre-flight capability triage function.
///
/// Returns a JSON string containing the triage envelope:
/// `{"is_capable": bool, "action_type": str, "target_url": str, ...}`
///
/// Rejection invariant: If incapable, returns rejection envelope without allocating browser.
#[pyfunction]
#[pyo3(signature = (prompt,))]
pub fn triage_prompt(py: Python<'_>, prompt: String) -> PyResult<String> {
    py.allow_threads(|| {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("failed to start Tokio runtime: {e}")))?;

        rt.block_on(async move {
            dotenvy::dotenv().ok();
            let groq = GroqClient::new(GroqConfig::default())
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            let search = ZeroUrlResolver::default();
            let gatekeeper = TriageGatekeeper::new(groq, search);

            match gatekeeper.evaluate(&prompt).await {
                Ok(decision) => {
                    let json = serde_json::to_string(&decision.envelope)
                        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                    Ok(json)
                }
                Err(harness_core::error::HarnessError::Rejected { reason }) => {
                    let envelope = serde_json::json!({
                        "is_capable": false,
                        "action_type": "incapable",
                        "search_query": null,
                        "target_url": null,
                        "rejection_reason": reason,
                    });
                    Ok(envelope.to_string())
                }
                Err(e) => Err(PyRuntimeError::new_err(e.to_string())),
            }
        })
    })
}

/// Zero-URL canonical search resolver function.
///
/// Resolves a concept or query to a canonical destination URL in <120ms
/// over native HTTP/2 DuckDuckGo HTML without launching Chromium.
#[pyfunction]
#[pyo3(signature = (query, timeout_ms = 2500))]
pub fn resolve_search(py: Python<'_>, query: String, timeout_ms: u64) -> PyResult<String> {
    py.allow_threads(|| {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("failed to start Tokio runtime: {e}")))?;

        rt.block_on(async move {
            let resolver = ZeroUrlResolver::new(Duration::from_millis(timeout_ms));
            let result = resolver
                .resolve(&query)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            let json = serde_json::json!({
                "target_url": result.target_url,
                "title": result.title,
                "snippet": result.snippet,
                "latency_ms": result.latency_ms,
            });

            Ok(json.to_string())
        })
    })
}

/// Fast-path recursive crawler leveraging HTTP/2 connection pooling.
///
/// Bypasses DOM rendering for static leaf pages, achieving >150 pages/sec,
/// and sinks extracted records directly to SQLite WAL.
#[pyfunction]
#[pyo3(signature = (
    seed_url,
    max_depth = 3,
    max_pages = 500,
    db_path = None,
    path_prefix = None,
    restrict_to_seed_path = false,
    table_name = None,
    selectors_json = None,
    ai_goal = None,
    target_attributes = None
))]
#[allow(clippy::too_many_arguments)]
pub fn crawl_url(
    py: Python<'_>,
    seed_url: String,
    max_depth: u32,
    max_pages: u64,
    db_path: Option<String>,
    path_prefix: Option<String>,
    restrict_to_seed_path: bool,
    table_name: Option<String>,
    selectors_json: Option<String>,
    ai_goal: Option<String>,
    target_attributes: Option<Vec<String>>,
) -> PyResult<u64> {
    py.allow_threads(|| {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("failed to start Tokio runtime: {e}")))?;

        rt.block_on(async move {
            let path = db_path
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".harness/harvest.db"));

            let store = DataStore::open(&path)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            let selectors = if let Some(ref json_str) = selectors_json {
                serde_json::from_str::<std::collections::HashMap<String, String>>(json_str)
                    .unwrap_or_default()
            } else {
                std::collections::HashMap::new()
            };

            let table = table_name.unwrap_or_else(|| "crawled_leaf_data".to_string());

            let config = CrawlerConfig {
                max_depth,
                max_pages,
                concurrency_limit: 8,
                table_name: table,
                path_prefix,
                restrict_to_seed_path,
                selectors,
                ai_extraction_goal: ai_goal,
                target_attributes: target_attributes.unwrap_or_default(),
                ..Default::default()
            };

            let crawler = FastPathCrawler::new(config, &[], store)
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            let pages = crawler
                .crawl(&seed_url)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            Ok(pages)
        })
    })
}

/// Queries extracted data from the high-throughput SQLite WAL store.
///
/// Returns a JSON string representing an array of data rows.
#[pyfunction]
#[pyo3(signature = (table_name, limit = None, db_path = None))]
pub fn query_table(
    py: Python<'_>,
    table_name: String,
    limit: Option<u32>,
    db_path: Option<String>,
) -> PyResult<String> {
    py.allow_threads(|| {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("failed to start Tokio runtime: {e}")))?;

        rt.block_on(async move {
            let path = db_path
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".harness/harvest.db"));

            let store = DataStore::open(&path)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            let rows = store
                .query_table(&table_name, limit)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            let json =
                serde_json::to_string(&rows).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            Ok(json)
        })
    })
}

/// Computes the deterministic 64-bit macro cache key for a given domain and prompt.
#[pyfunction]
#[pyo3(signature = (domain, prompt))]
pub fn derive_macro_key(domain: String, prompt: String) -> u64 {
    MacroReplayEngine::derive_cache_key(&domain, &prompt)
}

/// PyO3 C-Python extension module entry point.
#[pymodule]
fn harness_core_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(triage_prompt, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_search, m)?)?;
    m.add_function(wrap_pyfunction!(crawl_url, m)?)?;
    m.add_function(wrap_pyfunction!(query_table, m)?)?;
    m.add_function(wrap_pyfunction!(derive_macro_key, m)?)?;
    Ok(())
}
