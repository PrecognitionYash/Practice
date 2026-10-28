// ──────────────────────────────────────────────────────────────────────
// harness-browser/src/replay.rs — Deterministic Macro Replay Engine
//
// Eliminates redundant LLM calls on repetitive workflows.
// Replays verified sequences of mechanical actions locally in native Rust.
//
// Features:
//  • Deterministic 64-bit key derivation via xxHash64(Domain || IntentHash(Prompt))
//  • Fast native replay of recorded actions (Click, Type, Scroll, Wait)
//  • Automatic self-healing fallback: on failure, purges invalid cache
//    and signals the agent to capture a fresh perception frame for LLM repair
// ──────────────────────────────────────────────────────────────────────

use std::hash::Hasher;
use std::time::Duration;

use chrono::Utc;
use harness_core::error::{HarnessError, HarnessResult};
use harness_core::schema::{InteractionKind, MacroRecord, PlannedAction};
use harness_core::store::DataStore;
use tracing::{debug, error, info, instrument, warn};
use twox_hash::XxHash64;

use crate::driver::BrowserDriver;

/// Outcome of attempting to replay a cached macro sequence.
#[derive(Debug)]
pub enum ReplayOutcome {
    /// Macro executed to completion with all steps verified.
    Success {
        cache_key: u64,
        steps_executed: usize,
    },
    /// No matching macro found in cache (cache miss).
    CacheMiss { cache_key: u64 },
    /// A step failed during execution; cache entry purged; repair needed.
    FallbackRequired {
        cache_key: u64,
        failed_step: usize,
        action: PlannedAction,
        reason: String,
    },
}

/// The deterministic macro replay engine.
#[derive(Clone)]
pub struct MacroReplayEngine {
    store: DataStore,
}

impl MacroReplayEngine {
    /// Create a new macro replay engine connected to the datastore.
    pub fn new(store: DataStore) -> Self {
        Self { store }
    }

    /// Derives a stable 64-bit cache key from the domain and prompt intent.
    ///
    /// Formula:
    ///   K_macro = xxHash64(Domain(U) || IntentHash(P))
    pub fn derive_cache_key(domain: &str, prompt: &str) -> u64 {
        let mut hasher = XxHash64::default();
        // Domain part
        hasher.write(domain.to_lowercase().trim().as_bytes());
        hasher.write_u8(0xFF); // Separator byte
                               // Normalized prompt part
        let normalized_prompt: String = prompt
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        hasher.write(normalized_prompt.as_bytes());
        hasher.finish()
    }

    /// Records a newly verified action sequence into the cache.
    #[instrument(skip(self, actions))]
    pub async fn record_macro(
        &self,
        domain: &str,
        prompt: &str,
        actions: Vec<PlannedAction>,
    ) -> HarnessResult<u64> {
        let cache_key = Self::derive_cache_key(domain, prompt);
        let record = MacroRecord {
            cache_key,
            domain: domain.to_string(),
            prompt: prompt.to_string(),
            actions,
            verified_at: Utc::now(),
        };

        self.store.store_macro(&record).await?;
        info!(
            cache_key = cache_key,
            domain = domain,
            "recorded macro to datastore"
        );
        Ok(cache_key)
    }

    /// Checks if a cached macro exists for the domain and prompt.
    pub async fn has_macro(&self, domain: &str, prompt: &str) -> HarnessResult<bool> {
        let key = Self::derive_cache_key(domain, prompt);
        let found = self.store.lookup_macro(key).await?;
        Ok(found.is_some())
    }

    /// Attempts to replay a cached macro sequence.
    /// Ensures Set-of-Marks tags exist on the active page DOM for deterministic replay.
    pub async fn ensure_marks_tagged(&self, driver: &BrowserDriver) -> HarnessResult<()> {
        let page = driver.page().await;
        let tag_js = r#"
            (() => {
                if (document.querySelector('[data-mark-id]')) {
                    return true;
                }
                const interactives = Array.from(document.querySelectorAll('a, button, input, select, textarea, [role="button"], [role="link"], [role="checkbox"], [role="combobox"], [role="tab"], [tabindex="0"]'));
                let markId = 1;
                for (const el of interactives) {
                    const rect = el.getBoundingClientRect();
                    const style = window.getComputedStyle(el);
                    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') {
                        continue;
                    }
                    if (rect.width <= 0 || rect.height <= 0) {
                        continue;
                    }
                    el.setAttribute('data-harness-mark', '@' + markId);
                    el.setAttribute('data-mark-id', markId.toString());
                    markId++;
                }
                return true;
            })()
        "#;
        let _ = page.evaluate(tag_js).await;
        Ok(())
    }

    /// Attempts to replay a cached macro sequence.
    ///
    /// If all actions succeed, updates the verification timestamp and returns `ReplayOutcome::Success`.
    /// If any action fails, immediately purges the cache entry and returns `ReplayOutcome::FallbackRequired`.
    #[instrument(skip(self, driver))]
    pub async fn replay(
        &self,
        domain: &str,
        prompt: &str,
        driver: &BrowserDriver,
    ) -> HarnessResult<ReplayOutcome> {
        let cache_key = Self::derive_cache_key(domain, prompt);

        let macro_record = match self.store.lookup_macro(cache_key).await? {
            Some(rec) => rec,
            None => {
                debug!(cache_key, "macro cache miss");
                return Ok(ReplayOutcome::CacheMiss { cache_key });
            }
        };

        // Ensure active page DOM elements are annotated with Set-of-Marks before replaying actions
        let _ = self.ensure_marks_tagged(driver).await;

        info!(
            cache_key = cache_key,
            steps = macro_record.actions.len(),
            "replaying deterministic macro (0 LLM tokens)"
        );

        for (idx, planned) in macro_record.actions.iter().enumerate() {
            debug!(step = idx, action = ?planned.action, mark = planned.mark_id, "executing macro step");

            match self.execute_action(driver, planned).await {
                Ok(_) => {
                    // Step executed successfully
                }
                Err(err) => {
                    warn!(
                        cache_key = cache_key,
                        step = idx,
                        error = %err,
                        "macro step failed — initiating self-healing purge"
                    );

                    // Purge stale macro from SQLite cache.
                    if let Err(purge_err) = self.store.purge_macro(cache_key).await {
                        error!(cache_key = cache_key, error = %purge_err, "failed to purge stale macro");
                    }

                    return Ok(ReplayOutcome::FallbackRequired {
                        cache_key,
                        failed_step: idx,
                        action: planned.clone(),
                        reason: err.to_string(),
                    });
                }
            }
        }

        // Update verification timestamp on clean run.
        let mut updated = macro_record.clone();
        updated.verified_at = Utc::now();
        self.store.store_macro(&updated).await?;

        info!(cache_key = cache_key, "macro replay completed successfully");
        Ok(ReplayOutcome::Success {
            cache_key,
            steps_executed: macro_record.actions.len(),
        })
    }

    /// Internal executor for an individual planned action.
    async fn execute_action(
        &self,
        driver: &BrowserDriver,
        action: &PlannedAction,
    ) -> HarnessResult<()> {
        match action.action {
            InteractionKind::Click => {
                if action.mark_id == 0 {
                    // Mark @0 indicates root/viewport click or fallback
                    driver.click_at(500.0, 500.0).await?;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    return Ok(());
                }

                let page = driver.page().await;
                let mark_selector = format!("[data-mark-id=\"{}\"]", action.mark_id);

                let js_click = format!(
                    r#"
                    (() => {{
                        let el = document.querySelector('{mark_selector}');
                        if (!el) {{
                            const all = Array.from(document.querySelectorAll('*'));
                            el = all.find(e => e.getAttribute('data-harness-mark') === '@{mark_id}');
                        }}
                        if (!el) {{
                            const interactives = Array.from(document.querySelectorAll('a, button, input, select, textarea, [role="button"], [role="link"], [role="checkbox"], [role="combobox"], [role="tab"], [tabindex="0"]'));
                            let markId = 1;
                            for (const item of interactives) {{{{
                                const rect = item.getBoundingClientRect();
                                const style = window.getComputedStyle(item);
                                if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') continue;
                                if (rect.width <= 0 || rect.height <= 0) continue;
                                item.setAttribute('data-harness-mark', '@' + markId);
                                item.setAttribute('data-mark-id', markId.toString());
                                markId++;
                            }}}}
                            el = document.querySelector('{mark_selector}');
                            if (!el) {{{{
                                const all = Array.from(document.querySelectorAll('*'));
                                el = all.find(e => e.getAttribute('data-harness-mark') === '@{mark_id}');
                            }}}}
                        }}
                        if (!el) return false;
                        el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                        const rect = el.getBoundingClientRect();
                        return {{ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }};
                    }})()
                    "#,
                    mark_selector = mark_selector,
                    mark_id = action.mark_id,
                );

                let mut eval_res = page.evaluate(js_click.as_str()).await;

                // If not found, ensure marks are tagged and retry once after 250ms
                let not_found = match &eval_res {
                    Ok(res) => res
                        .value()
                        .map(|v| v.is_boolean() && v.as_bool() == Some(false))
                        .unwrap_or(true),
                    Err(_) => true,
                };

                if not_found {
                    let _ = self.ensure_marks_tagged(driver).await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    eval_res = page.evaluate(js_click.as_str()).await;
                }

                if let Ok(res) = eval_res {
                    if let Some(coords) = res.value() {
                        if !coords.is_boolean() || coords.as_bool() != Some(false) {
                            if let (Some(x), Some(y)) = (coords["x"].as_f64(), coords["y"].as_f64())
                            {
                                driver.click_at(x, y).await?;
                                tokio::time::sleep(Duration::from_millis(300)).await;
                                return Ok(());
                            }
                        }
                    }
                }

                Err(HarnessError::ElementNotFound {
                    selector: format!("@{}", action.mark_id),
                })
            }
            InteractionKind::Type => {
                if let Some(ref text) = action.text {
                    let mark_selector = format!("[data-mark-id=\"{}\"]", action.mark_id);
                    let page = driver.page().await;
                    let js_focus = format!(
                        r#"
                        (() => {{
                            let el = document.querySelector('{mark_selector}');
                            if (!el) {{
                                const all = Array.from(document.querySelectorAll('*'));
                                el = all.find(e => e.getAttribute('data-harness-mark') === '@{mark_id}');
                            }}
                            if (!el) {{
                                const interactives = Array.from(document.querySelectorAll('a, button, input, select, textarea, [role="button"], [role="link"], [role="checkbox"], [role="combobox"], [role="tab"], [tabindex="0"]'));
                                let markId = 1;
                                for (const item of interactives) {{{{
                                    const rect = item.getBoundingClientRect();
                                    const style = window.getComputedStyle(item);
                                    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') continue;
                                    if (rect.width <= 0 || rect.height <= 0) continue;
                                    item.setAttribute('data-harness-mark', '@' + markId);
                                    item.setAttribute('data-mark-id', markId.toString());
                                    markId++;
                                }}}}
                                el = document.querySelector('{mark_selector}');
                                if (!el) {{{{
                                    const all = Array.from(document.querySelectorAll('*'));
                                    el = all.find(e => e.getAttribute('data-harness-mark') === '@{mark_id}');
                                }}}}
                            }}
                            if (!el) return false;
                            el.focus();
                            return true;
                        }})()
                        "#,
                        mark_selector = mark_selector,
                        mark_id = action.mark_id,
                    );

                    let mut res = page.evaluate(js_focus.as_str()).await;
                    let not_found = match &res {
                        Ok(r) => r.value().and_then(|v| v.as_bool()) == Some(false),
                        Err(_) => true,
                    };

                    if not_found {
                        let _ = self.ensure_marks_tagged(driver).await;
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        res = page.evaluate(js_focus.as_str()).await;
                    }

                    if let Ok(r) = res {
                        if r.value().and_then(|v| v.as_bool()) == Some(true) {
                            driver.type_text(text).await?;
                            tokio::time::sleep(Duration::from_millis(150)).await;
                            return Ok(());
                        }
                    }

                    Err(HarnessError::ElementNotFound {
                        selector: format!("@{}", action.mark_id),
                    })
                } else {
                    Err(HarnessError::MissingField {
                        field: "text".into(),
                        context: "InteractionKind::Type".into(),
                    })
                }
            }
            InteractionKind::Scroll => {
                driver.scroll_by(0.0, 300.0).await?;
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(())
            }
            InteractionKind::Wait => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok(())
            }
            InteractionKind::Hover => {
                let page = driver.page().await;
                let mark_selector = format!("[data-mark-id=\"{}\"]", action.mark_id);
                let js_hover = format!(
                    r#"
                    (() => {{
                        let el = document.querySelector('{mark_selector}');
                        if (!el) {{
                            const all = Array.from(document.querySelectorAll('*'));
                            el = all.find(e => e.getAttribute('data-harness-mark') === '@{mark_id}');
                        }}
                        if (!el) {{
                            const interactives = Array.from(document.querySelectorAll('a, button, input, select, textarea, [role="button"], [role="link"], [role="checkbox"], [role="combobox"], [role="tab"], [tabindex="0"]'));
                            let markId = 1;
                            for (const item of interactives) {{{{
                                const rect = item.getBoundingClientRect();
                                const style = window.getComputedStyle(item);
                                if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') continue;
                                if (rect.width <= 0 || rect.height <= 0) continue;
                                item.setAttribute('data-harness-mark', '@' + markId);
                                item.setAttribute('data-mark-id', markId.toString());
                                markId++;
                            }}}}
                            el = document.querySelector('{mark_selector}');
                        }}
                        if (!el) return null;
                        const rect = el.getBoundingClientRect();
                        return {{ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }};
                    }})()
                    "#,
                    mark_selector = mark_selector,
                    mark_id = action.mark_id
                );

                let mut res = page.evaluate(js_hover.as_str()).await;
                let not_found = match &res {
                    Ok(r) => r.value().is_none() || r.value().unwrap().is_null(),
                    Err(_) => true,
                };

                if not_found {
                    let _ = self.ensure_marks_tagged(driver).await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    res = page.evaluate(js_hover.as_str()).await;
                }

                if let Ok(r) = res {
                    if let Some(val) = r.value() {
                        if let (Some(x), Some(y)) = (val["x"].as_f64(), val["y"].as_f64()) {
                            driver.move_mouse_bezier(x, y).await?;
                            tokio::time::sleep(Duration::from_millis(150)).await;
                            return Ok(());
                        }
                    }
                }
                Err(HarnessError::ElementNotFound {
                    selector: format!("@{}", action.mark_id),
                })
            }
            InteractionKind::Select => {
                let page = driver.page().await;
                let mark_selector = format!("[data-mark-id=\"{}\"]", action.mark_id);
                let text_val = action.text.clone().unwrap_or_default();
                let js_select = format!(
                    r#"
                    (() => {{
                        let el = document.querySelector('{mark_selector}');
                        if (!el) {{
                            const all = Array.from(document.querySelectorAll('*'));
                            el = all.find(e => e.getAttribute('data-harness-mark') === '@{mark_id}');
                        }}
                        if (!el) {{
                            const interactives = Array.from(document.querySelectorAll('a, button, input, select, textarea, [role="button"], [role="link"], [role="checkbox"], [role="combobox"], [role="tab"], [tabindex="0"]'));
                            let markId = 1;
                            for (const item of interactives) {{{{
                                const rect = item.getBoundingClientRect();
                                const style = window.getComputedStyle(item);
                                if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') continue;
                                if (rect.width <= 0 || rect.height <= 0) continue;
                                item.setAttribute('data-harness-mark', '@' + markId);
                                item.setAttribute('data-mark-id', markId.toString());
                                markId++;
                            }}}}
                            el = document.querySelector('{mark_selector}');
                        }}
                        if (!el) return false;
                        el.value = '{text}';
                        el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                        return true;
                    }})()
                    "#,
                    mark_selector = mark_selector,
                    mark_id = action.mark_id,
                    text = text_val
                );

                let mut res = page.evaluate(js_select.as_str()).await;
                let not_found = match &res {
                    Ok(r) => r.value().and_then(|v| v.as_bool()) == Some(false),
                    Err(_) => true,
                };

                if not_found {
                    let _ = self.ensure_marks_tagged(driver).await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    res = page.evaluate(js_select.as_str()).await;
                }

                if let Ok(r) = res {
                    if r.value().and_then(|v| v.as_bool()) == Some(true) {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        return Ok(());
                    }
                }
                Err(HarnessError::ElementNotFound {
                    selector: format!("@{}", action.mark_id),
                })
            }
            InteractionKind::Navigate => {
                if let Some(ref url) = action.text {
                    driver.goto(url).await?;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Ok(())
                } else {
                    Err(HarnessError::MissingField {
                        field: "url".into(),
                        context: "InteractionKind::Navigate".into(),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_derivation_deterministic() {
        let k1 = MacroReplayEngine::derive_cache_key("github.com", "search for rust repos");
        let k2 = MacroReplayEngine::derive_cache_key("GITHUB.COM", "search   for   rust   repos");
        assert_eq!(
            k1, k2,
            "keys should match regardless of casing or extra whitespace"
        );

        let k3 = MacroReplayEngine::derive_cache_key("gitlab.com", "search for rust repos");
        assert_ne!(k1, k3, "different domains should yield distinct keys");
    }

    #[tokio::test]
    async fn test_macro_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_macro.db");
        let store = DataStore::open(&db_path).await.unwrap();
        let engine = MacroReplayEngine::new(store);

        let domain = "github.com";
        let prompt = "Extract top trending repositories on github.com";

        assert!(!engine.has_macro(domain, prompt).await.unwrap());

        let actions = vec![
            PlannedAction {
                action: InteractionKind::Click,
                mark_id: 8,
                text: None,
            },
            PlannedAction {
                action: InteractionKind::Wait,
                mark_id: 0,
                text: None,
            },
            PlannedAction {
                action: InteractionKind::Scroll,
                mark_id: 0,
                text: None,
            },
        ];

        let key = engine.record_macro(domain, prompt, actions).await.unwrap();
        assert_ne!(key, 0);
        assert!(engine.has_macro(domain, prompt).await.unwrap());
    }
}
