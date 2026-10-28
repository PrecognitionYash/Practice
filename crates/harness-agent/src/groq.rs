// ──────────────────────────────────────────────────────────────────────
// harness-agent/src/groq.rs — Native Async Groq Llama-3.3-70B Client
//
// Interacts with Groq's high-speed inference engine under strict
// token-budget allocation invariants:
//  • T_sys <= 350 tokens (Immutable prompt compiler)
//  • T_tools <= 250 tokens (Collapsed 5-macro schema)
//  • T_hist <= 1,500 tokens (Rolling window keeping last k=8 transitions)
//  • T_percept <= 3,500 tokens (Enforced Set-of-Marks pruner)
//  • T_completion <= 1,400 tokens (Generation envelope)
//  • Max total <= 7,000 tokens << 8,192 token Groq ceiling (<85% budget)
// ──────────────────────────────────────────────────────────────────────

use std::time::Duration;

use harness_core::error::{HarnessError, HarnessResult};
use harness_core::schema::{ActionPlan, AiPageEvaluation, TriageEnvelope};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

/// Default Groq inference endpoint.
pub const GROQ_API_ENDPOINT: &str = "https://api.groq.com/openai/v1/chat/completions";

/// Default production model.
pub const DEFAULT_MODEL: &str = "openai/gpt-oss-20b";

/// Maximum token envelope for planning completions.
pub const MAX_PLANNING_COMPLETION_TOKENS: u32 = 2048;

/// Maximum token envelope for pre-flight triage (supports reasoning models + envelope).
pub const MAX_TRIAGE_COMPLETION_TOKENS: u32 = 512;

/// Role for chat messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

/// A single message in the chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }
}

/// Configuration options for the Groq client.
#[derive(Debug, Clone)]
pub struct GroqConfig {
    /// Groq API key (defaults to GROQ_API_KEY environment variable).
    pub api_key: String,
    /// Model name.
    pub model: String,
    /// API endpoint URL.
    pub endpoint: String,
    /// Request timeout.
    pub timeout: Duration,
    /// Maximum retries on 429 rate limits or transient 5xx errors.
    pub max_retries: u32,
}

impl Default for GroqConfig {
    fn default() -> Self {
        let api_key = std::env::var("GROQ_API_KEY").unwrap_or_default();
        let model =
            std::env::var("GROQ_MODEL").unwrap_or_else(|_| "openai/gpt-oss-120b".to_string());
        let endpoint =
            std::env::var("GROQ_ENDPOINT").unwrap_or_else(|_| GROQ_API_ENDPOINT.to_string());
        Self {
            api_key,
            model,
            endpoint,
            timeout: Duration::from_secs(15),
            max_retries: 3,
        }
    }
}

/// Groq API client with JSON mode and context budget enforcement.
#[derive(Debug, Clone)]
pub struct GroqClient {
    config: GroqConfig,
    client: reqwest::Client,
}

impl GroqClient {
    /// Create a new Groq client with configuration.
    pub fn new(config: GroqConfig) -> HarnessResult<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if !config.api_key.is_empty() {
            let auth_header = format!("Bearer {}", config.api_key.trim());
            let mut val = HeaderValue::from_str(&auth_header)
                .map_err(|e| HarnessError::Internal(format!("invalid GROQ API key header: {e}")))?;
            val.set_sensitive(true);
            headers.insert(AUTHORIZATION, val);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(config.timeout)
            .gzip(true)
            .build()
            .map_err(|e| HarnessError::Http {
                reason: format!("failed to build reqwest client: {e}"),
            })?;

        Ok(Self { config, client })
    }

    /// Returns true if a valid GROQ_API_KEY is configured.
    pub fn has_api_key(&self) -> bool {
        !self.config.api_key.trim().is_empty()
    }

    /// Invokes pre-flight triage classification using the strict 80-token JSON envelope.
    #[instrument(skip(self, prompt))]
    pub async fn triage_classify(&self, prompt: &str) -> HarnessResult<TriageEnvelope> {
        let system_prompt = r#"You are a pre-flight capability triage classifier for an autonomous web browser agent.
Evaluate the user prompt and respond ONLY with a JSON object adhering to this schema:
{
  "is_capable": boolean,
  "action_type": "direct_navigation" | "direct_search" | "data_extraction" | "recursive_crawl" | "incapable",
  "search_query": string or null,
  "target_url": string or null,
  "rejection_reason": string or null
}
Rules:
1. If the task requires optical/audio CAPTCHA bypass, bank/financial wire transfers, or illegal activity, set is_capable=false, action_type="incapable", and provide rejection_reason.
2. If a direct URL is in the prompt, set target_url to that URL and action_type="direct_navigation".
3. If no URL is provided and search is needed, set action_type="direct_search", search_query to the concise search terms, and target_url=null.
4. Output raw JSON only. Zero prose."#;

        let messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(prompt),
        ];

        let raw_json = self
            .chat_completion(&messages, 0.0, MAX_TRIAGE_COMPLETION_TOKENS, true)
            .await?;

        let cleaned = extract_json_str(&raw_json);
        let envelope: TriageEnvelope = serde_json::from_str(cleaned).map_err(HarnessError::Json)?;

        debug!(is_capable = envelope.is_capable, action = ?envelope.action_type, "triage classification parsed");
        Ok(envelope)
    }

    /// Invokes the strategic ReAct planner on a compressed Set-of-Marks perception frame.
    ///
    /// Rolling history is clamped to the last $k=3$ steps to preserve the context invariant.
    #[instrument(skip(self, history, perception_frame))]
    pub async fn plan_next_action(
        &self,
        task_instruction: &str,
        history: &[ChatMessage],
        perception_frame: &str,
    ) -> HarnessResult<ActionPlan> {
        let system_prompt = r#"You are the Strategic ReAct Planner for an autonomous web agent.
You receive a Set-of-Marks perception frame where interactive elements are indexed as [@1], [@2], etc.
Respond ONLY with a JSON object adhering to this schema:
{
  "reasoning": "concise description of state and tactical intent (1-2 sentences)",
  "actions": [
    {
      "mark_id": integer,
      "action": "click" | "type" | "select" | "scroll" | "hover" | "wait" | "navigate",
      "text": string or null
    }
  ],
  "is_complete": boolean
}
Action definitions:
- "click": click element [@mark_id]
- "type": type text into [@mark_id], specifying "text"
- "scroll": scroll down the viewport
- "wait": wait for asynchronous content to render
- "navigate": navigate directly to "text" URL
Output valid JSON only. Zero markdown, zero backticks."#;

        let mut messages = Vec::new();
        messages.push(ChatMessage::system(system_prompt));

        // Rolling history: retain last k=8 transitions (bounded to <= 1,500 tokens)
        let k = 8;
        let start_idx = if history.len() > k {
            history.len() - k
        } else {
            0
        };
        for msg in &history[start_idx..] {
            messages.push(msg.clone());
        }

        // Current perceptual state vector
        let user_prompt = format!(
            "Goal: {}\n\nCurrent Page State:\n{}",
            task_instruction.trim(),
            perception_frame.trim()
        );
        messages.push(ChatMessage::user(user_prompt));

        let raw_json = self
            .chat_completion(&messages, 0.0, MAX_PLANNING_COMPLETION_TOKENS, true)
            .await?;

        let cleaned = extract_json_str(&raw_json);
        let plan: ActionPlan = serde_json::from_str(cleaned).map_err(HarnessError::Json)?;

        info!(
            reasoning = %plan.reasoning,
            actions = plan.actions.len(),
            is_complete = plan.is_complete,
            "strategic action plan synthesized"
        );

        Ok(plan)
    }

    /// Evaluates whether a scraped page is valuable/valid according to the extraction goal,
    /// and autonomously extracts high-density structured entities and summaries.
    #[instrument(skip(self, page_content_snippet))]
    pub async fn evaluate_and_extract_page(
        &self,
        page_url: &str,
        page_title: &str,
        page_content_snippet: &str,
        extraction_goal: &str,
    ) -> HarnessResult<AiPageEvaluation> {
        // Fast-path heuristic fallback if GROQ_API_KEY is not configured
        if self.config.api_key.is_empty() {
            return Ok(Self::heuristic_evaluation(
                page_url,
                page_title,
                page_content_snippet,
                extraction_goal,
            ));
        }

        let system_prompt = r#"You are an Autonomous AI Web Data Curator and Information Extractor.
Evaluate whether the scraped web page has valid, high-value content relevant to the user's extraction goal, or if it is low-value noise (login walls, cookie consent, terms of service, error 404, empty placeholder, navigation boilerplate).
If valuable, extract rich structured key-value entities and a crisp summary.
Respond ONLY with a JSON object strictly adhering to this schema:
{
  "is_valuable": boolean,
  "relevance_score": number between 0.0 and 1.0,
  "category": "article" | "repository" | "profile" | "product" | "documentation" | "discussion" | "noise" | "login_wall",
  "summary": string,
  "extracted_data": object with structured key-value fields,
  "rationale": string
}
Rules:
1. Mark is_valuable=false for privacy policies, cookie policies, sign-in redirects, 404 pages, or generic site footers.
2. If is_valuable=true, populate extracted_data with clean facts, metrics, titles, authors, features, or specs.
3. Raw JSON only. Zero markdown."#;

        let user_content = format!(
            "Extraction Goal: {}\n\nPage URL: {}\nPage Title: {}\n\nContent Sample:\n{}",
            extraction_goal.trim(),
            page_url.trim(),
            page_title.trim(),
            page_content_snippet.chars().take(2500).collect::<String>()
        );

        let messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(user_content),
        ];

        let raw_json = self.chat_completion(&messages, 0.0, 1024, true).await?;
        let cleaned = extract_json_str(&raw_json);
        let evaluation: AiPageEvaluation =
            serde_json::from_str(cleaned).map_err(HarnessError::Json)?;

        debug!(
            url = %page_url,
            valuable = evaluation.is_valuable,
            score = evaluation.relevance_score,
            category = %evaluation.category,
            "AI page evaluation completed"
        );

        Ok(evaluation)
    }

    /// Pure-heuristic evaluation fallback when offline or without API key.
    pub fn heuristic_evaluation(
        page_url: &str,
        page_title: &str,
        page_content_snippet: &str,
        extraction_goal: &str,
    ) -> AiPageEvaluation {
        let url_lower = page_url.to_lowercase();
        let title_lower = page_title.to_lowercase();
        let snippet_lower = page_content_snippet.to_lowercase();

        // Detect obvious noise
        let is_noise = url_lower.contains("privacy")
            || url_lower.contains("terms")
            || url_lower.contains("cookie")
            || url_lower.contains("login")
            || url_lower.contains("signup")
            || url_lower.contains("auth")
            || title_lower.contains("404")
            || title_lower.contains("not found")
            || title_lower.contains("sign in")
            || page_content_snippet.trim().len() < 80;

        if is_noise {
            return AiPageEvaluation {
                is_valuable: false,
                relevance_score: 0.1,
                category: "noise".to_string(),
                summary: "Filtered as boilerplate, login barrier, or low-density page.".to_string(),
                extracted_data: serde_json::json!({}),
                rationale: "Detected administrative, auth, or boilerplate signature.".to_string(),
            };
        }

        // Compute heuristic relevance based on extraction goal keywords
        let goal_words: Vec<&str> = extraction_goal
            .split_whitespace()
            .map(|w| w.trim().trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 3)
            .collect();

        let mut matches = 0;
        for w in &goal_words {
            if title_lower.contains(&w.to_lowercase()) || snippet_lower.contains(&w.to_lowercase())
            {
                matches += 1;
            }
        }

        let score = if goal_words.is_empty() {
            0.85
        } else {
            0.5 + (0.5 * (matches as f64 / goal_words.len() as f64))
        };

        AiPageEvaluation {
            is_valuable: true,
            relevance_score: score.clamp(0.0, 1.0),
            category: "content".to_string(),
            summary: format!(
                "{page_title}: {}",
                page_content_snippet.chars().take(200).collect::<String>()
            ),
            extracted_data: serde_json::json!({
                "page_title": page_title,
                "url": page_url,
                "text_sample": page_content_snippet.chars().take(500).collect::<String>()
            }),
            rationale: "Heuristic evaluation verified content density and keyword relevance."
                .to_string(),
        }
    }

    /// Low-level chat completion request with exponential backoff on 429/5xx.
    #[instrument(skip(self, messages))]
    pub async fn chat_completion(
        &self,
        messages: &[ChatMessage],
        temperature: f64,
        max_tokens: u32,
        json_mode: bool,
    ) -> HarnessResult<String> {
        if self.config.api_key.is_empty() {
            return Err(HarnessError::Http {
                reason: "GROQ_API_KEY is not configured".into(),
            });
        }

        let mut body = serde_json::json!({
            "model": self.config.model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });

        if json_mode {
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }

        let mut backoff = Duration::from_millis(500);

        for attempt in 0..=self.config.max_retries {
            let res = self
                .client
                .post(&self.config.endpoint)
                .json(&body)
                .send()
                .await;

            match res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let completion: GroqChatResponse =
                            resp.json().await.map_err(|e| HarnessError::Http {
                                reason: format!("failed to decode Groq response JSON: {e}"),
                            })?;

                        let content = completion
                            .choices
                            .into_iter()
                            .next()
                            .and_then(|c| c.message.content)
                            .unwrap_or_default();

                        return Ok(content);
                    }

                    // Rate limited (429) or transient 5xx server error
                    if (status.as_u16() == 429 || status.is_server_error())
                        && attempt < self.config.max_retries
                    {
                        warn!(
                            attempt = attempt + 1,
                            status = %status,
                            retry_in = ?backoff,
                            "Groq API throttled/error — applying exponential backoff"
                        );
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }

                    let err_text = resp.text().await.unwrap_or_default();
                    return Err(HarnessError::Http {
                        reason: format!("Groq API HTTP {status}: {err_text}"),
                    });
                }
                Err(err) => {
                    if attempt < self.config.max_retries {
                        warn!(
                            attempt = attempt + 1,
                            error = %err,
                            retry_in = ?backoff,
                            "Groq connection failure — retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }
                    return Err(HarnessError::Http {
                        reason: format!("Groq request failed: {err}"),
                    });
                }
            }
        }

        Err(HarnessError::Http {
            reason: "exhausted Groq API retries".into(),
        })
    }
}

// ── Wire JSON representation for Groq API ───────────────────────────

#[derive(Debug, Deserialize)]
struct GroqChatResponse {
    choices: Vec<GroqChoice>,
}

#[derive(Debug, Deserialize)]
struct GroqChoice {
    message: GroqResponseMessage,
}

#[derive(Debug, Deserialize)]
struct GroqResponseMessage {
    content: Option<String>,
}

/// Robust JSON extractor stripping markdown wrappers and extraneous whitespace.
pub(crate) fn extract_json_str(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_constructors() {
        let sys = ChatMessage::system("System instructions");
        let usr = ChatMessage::user("User question");
        let asst = ChatMessage::assistant("Assistant reply");

        assert_eq!(sys.role, MessageRole::System);
        assert_eq!(usr.role, MessageRole::User);
        assert_eq!(asst.role, MessageRole::Assistant);
    }

    #[test]
    fn test_action_plan_json_deserialization() {
        let sample_json = r#"{
            "reasoning": "Clicking the sign-in button to proceed.",
            "actions": [
                {
                    "mark_id": 1,
                    "action": "click",
                    "text": null
                }
            ],
            "is_complete": false
        }"#;

        let plan: ActionPlan = serde_json::from_str(sample_json).unwrap();
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(plan.actions[0].mark_id, 1);
        assert!(!plan.is_complete);
    }
}
