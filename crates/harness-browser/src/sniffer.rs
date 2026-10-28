// ──────────────────────────────────────────────────────────────────────
// harness-browser/src/sniffer.rs — Zero-DOM CDP Network Sniffer
//
// Intercepts JSON network responses directly from the CDP socket
// stream, bypassing the DOM entirely.
//
// Features:
//  • Enables `Network.enable` on the active page
//  • Filters for `application/json` MIME types
//  • Streams `Network.getResponseBody` payloads asynchronously
//  • Pipes matching JSON objects directly into high-throughput SQLite WAL
//  • Broadcasts intercepted responses for TUI/agent consumption
//  • Thread-safe, non-blocking asynchronous pipeline
// ──────────────────────────────────────────────────────────────────────

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams, EventResponseReceived, GetResponseBodyParams,
};
use chromiumoxide::Page;
use chrono::Utc;
use futures::StreamExt;
use harness_core::error::{HarnessError, HarnessResult};
use harness_core::schema::{DataRow, InterceptedResponse};
use harness_core::store::DataStore;
use tokio::sync::broadcast;
use tracing::{debug, error, info, instrument, warn};

/// Configuration for the CDP network sniffer.
#[derive(Debug, Clone)]
pub struct SnifferConfig {
    /// Maximum post data size in bytes (default: 10 MB).
    pub max_post_data_size: i64,
    /// Capacity of the internal broadcast channel.
    pub channel_capacity: usize,
    /// Destination SQLite table name when auto-persisting.
    pub auto_persist_table: Option<String>,
}

impl Default for SnifferConfig {
    fn default() -> Self {
        Self {
            max_post_data_size: 10 * 1024 * 1024, // 10 MB
            channel_capacity: 512,
            auto_persist_table: Some("intercepted_api_data".to_string()),
        }
    }
}

/// Statistics collected by the network sniffer.
#[derive(Debug, Default)]
pub struct SnifferMetrics {
    pub responses_scanned: AtomicU64,
    pub json_intercepted: AtomicU64,
    pub bytes_processed: AtomicU64,
    pub storage_errors: AtomicU64,
}

/// Network sniffer that captures API traffic directly from Chromium DevTools.
pub struct NetworkSniffer {
    config: SnifferConfig,
    metrics: Arc<SnifferMetrics>,
    broadcast_tx: broadcast::Sender<InterceptedResponse>,
}

impl NetworkSniffer {
    /// Create a new network sniffer instance.
    pub fn new(config: SnifferConfig) -> Self {
        let (broadcast_tx, _) = broadcast::channel(config.channel_capacity);
        Self {
            config,
            metrics: Arc::new(SnifferMetrics::default()),
            broadcast_tx,
        }
    }

    /// Subscribe to the stream of intercepted JSON responses.
    pub fn subscribe(&self) -> broadcast::Receiver<InterceptedResponse> {
        self.broadcast_tx.subscribe()
    }

    /// Access collected operational metrics.
    pub fn metrics(&self) -> Arc<SnifferMetrics> {
        self.metrics.clone()
    }

    /// Attach to a page and begin asynchronous response interception.
    ///
    /// Spawns a background Tokio task that runs for the lifetime of the page.
    /// If `store` is provided and `auto_persist_table` is configured, intercepted
    /// records are immediately written to SQLite.
    #[instrument(skip(self, page, store))]
    pub async fn attach(&self, page: &Page, store: Option<DataStore>) -> HarnessResult<()> {
        // 1. Enable network tracking in CDP.
        let enable = EnableParams::builder()
            .max_post_data_size(self.config.max_post_data_size)
            .build();

        page.execute(enable)
            .await
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to execute Network.enable: {e}"),
            })?;

        // 2. Register event listener for ResponseReceived.
        let mut response_events = page
            .event_listener::<EventResponseReceived>()
            .await
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to attach EventResponseReceived listener: {e}"),
            })?;

        let tx = self.broadcast_tx.clone();
        let metrics = self.metrics.clone();
        let page_clone = page.clone();
        let auto_table = self.config.auto_persist_table.clone();

        info!("CDP network sniffer attached and listening for application/json responses");

        tokio::spawn(async move {
            while let Some(event) = response_events.next().await {
                metrics.responses_scanned.fetch_add(1, Ordering::Relaxed);

                let mime = &event.response.mime_type;
                if !mime.contains("application/json") {
                    continue;
                }

                let req_id = event.request_id.clone();
                let url = event.response.url.clone();
                let status = event.response.status as u16;

                debug!(url = %url, status = status, "intercepted JSON HTTP response");

                // Fetch raw response body over CDP socket.
                let body_cmd = GetResponseBodyParams::new(req_id.clone());
                match page_clone.execute(body_cmd).await {
                    Ok(body_res) => {
                        let raw_body = &body_res.body;
                        metrics
                            .bytes_processed
                            .fetch_add(raw_body.len() as u64, Ordering::Relaxed);

                        // Parse string body as JSON Value.
                        match serde_json::from_str::<serde_json::Value>(raw_body) {
                            Ok(json_val) => {
                                metrics.json_intercepted.fetch_add(1, Ordering::Relaxed);

                                let record = InterceptedResponse {
                                    request_id: req_id.into(),
                                    url: url.clone(),
                                    status,
                                    mime_type: mime.clone(),
                                    body: json_val.clone(),
                                    intercepted_at: Utc::now(),
                                };

                                // Broadcast to active subscribers.
                                let _ = tx.send(record);

                                // Auto-persist directly to SQLite if configured.
                                if let (Some(ref ds), Some(ref table)) = (&store, &auto_table) {
                                    let row = DataRow {
                                        source_url: url.clone(),
                                        table_name: table.clone(),
                                        payload: json_val,
                                        extracted_at: Utc::now(),
                                    };

                                    if let Err(err) = ds.insert_row(&row).await {
                                        metrics.storage_errors.fetch_add(1, Ordering::Relaxed);
                                        error!(url = %url, error = %err, "failed to persist intercepted JSON to datastore");
                                    } else {
                                        debug!(url = %url, table = %table, "persisted intercepted JSON to SQLite");
                                    }
                                }
                            }
                            Err(parse_err) => {
                                warn!(url = %url, error = %parse_err, "failed to parse application/json body as valid JSON");
                            }
                        }
                    }
                    Err(cdp_err) => {
                        debug!(url = %url, error = %cdp_err, "could not fetch response body (likely cached or aborted)");
                    }
                }
            }
            info!("CDP network sniffer listener loop terminated");
        });

        Ok(())
    }

    /// Wait for the next JSON response matching a URL predicate with a timeout.
    pub async fn wait_for_response<F>(
        &self,
        predicate: F,
        timeout: std::time::Duration,
    ) -> HarnessResult<InterceptedResponse>
    where
        F: Fn(&str) -> bool + Send + 'static,
    {
        let mut rx = self.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(HarnessError::Http {
                    reason: "timeout waiting for matching intercepted response".to_string(),
                });
            }

            tokio::select! {
                res = rx.recv() => {
                    match res {
                        Ok(item) => {
                            if predicate(&item.url) {
                                return Ok(item);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(skipped, "sniffer subscriber lagged behind stream");
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return Err(HarnessError::Http {
                                reason: "sniffer stream closed".to_string(),
                            });
                        }
                    }
                }
                _ = tokio::time::sleep(remaining) => {
                    return Err(HarnessError::Http {
                        reason: "timeout waiting for matching intercepted response".to_string(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sniffer_broadcast_channel() {
        let sniffer = NetworkSniffer::new(SnifferConfig::default());
        let mut rx = sniffer.subscribe();

        let sample = InterceptedResponse {
            request_id: "req-123".into(),
            url: "https://example.com/api/v1/users".into(),
            status: 200,
            mime_type: "application/json".into(),
            body: serde_json::json!({"users": [{"id": 1, "name": "Alice"}]}),
            intercepted_at: Utc::now(),
        };

        let _ = sniffer.broadcast_tx.send(sample.clone());

        let received = rx.recv().await.unwrap();
        assert_eq!(received.request_id, "req-123");
        assert_eq!(received.status, 200);
        assert_eq!(received.body["users"][0]["name"], "Alice");
    }
}
