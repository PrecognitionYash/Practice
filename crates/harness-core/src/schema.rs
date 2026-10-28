// ──────────────────────────────────────────────────────────────────────
// harness-core/src/schema.rs — Structured Serde Exchange Models
//
// Every JSON payload crossing a subsystem boundary (LLM ↔ Rust,
// TUI ↔ Agent, Sniffer ↔ Store) is defined here as a strongly-typed
// Serde struct.  This prevents stringly-typed data from leaking
// between modules and gives us compile-time schema validation.
// ──────────────────────────────────────────────────────────────────────

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Pre-Flight Triage Envelope ──────────────────────────────────────

/// The 80-token classification payload sent to and returned by Groq
/// during the pre-flight capability triage.
///
/// `temperature=0.0`, `max_tokens=80`, strict JSON schema enforcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageEnvelope {
    /// Whether the system believes it can execute the task.
    pub is_capable: bool,

    /// The high-level classification of the task.
    pub action_type: ActionType,

    /// An optional search query if the user did not provide a URL
    /// and the task requires discovering a target page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,

    /// The resolved target URL, if already known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,

    /// Human-readable rejection reason (only set when `is_capable == false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

/// Enumeration of high-level task classifications returned by the
/// pre-flight gatekeeper.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    /// Navigate to a known URL and interact with the page.
    DirectNavigation,
    /// The user's prompt implies a search; trigger the zero-URL resolver.
    DirectSearch,
    /// Extract structured data from a dynamic SPA via network sniffing.
    DataExtraction,
    /// Crawl a set of pages recursively.
    RecursiveCrawl,
    /// The task is out of scope (CAPTCHA bypass, illegal, etc.).
    Incapable,
}

// ── Autonomous Tool Calling System ──────────────────────────────────

/// Strongly-typed tool invocation emitted by the autonomous AI intent router.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "tool", content = "parameters", rename_all = "snake_case")]
pub enum HarnessToolCall {
    /// Fast-path recursive crawl or scrape extracting only specified attributes into SQLite WAL.
    FastCrawler {
        url: String,
        #[serde(default)]
        table: Option<String>,
        #[serde(default)]
        attributes: Vec<String>,
        #[serde(default)]
        ai_goal: Option<String>,
        #[serde(default)]
        max_pages: Option<u64>,
        #[serde(default)]
        restrict_path: bool,
    },
    /// Interactive CDP browser automation for live page interaction, typing, clicking, SPA JS rendering.
    BrowserAgent {
        url: String,
        goal: String,
        #[serde(default)]
        search_query: Option<String>,
        #[serde(default)]
        attributes: Vec<String>,
    },
    /// Direct search query resolution to locate an authoritative endpoint or product list.
    WebSearch { query: String },
    /// Query or preview harvested tables in SQLite WAL store.
    QueryData {
        #[serde(default)]
        table: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Export a harvested SQLite table to CSV or Parquet.
    ExportData {
        table: String,
        #[serde(default = "default_export_format")]
        format: String,
    },
    /// Manage cryptographic session vault (list, check, load).
    SessionVault {
        action: String,
        #[serde(default)]
        domain: Option<String>,
    },
    /// Stop any running crawl or active execution safely preserving all records.
    StopExecution,
    /// Pre-flight capability rejection for impossible/prohibited operations.
    RejectTask { reason: String },
}

fn default_export_format() -> String {
    "csv".to_string()
}

// ── LLM Action Plan ─────────────────────────────────────────────────

/// A single mechanical action in a macro sequence, as planned by the
/// LLM or recorded during macro caching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAction {
    /// The Set-of-Marks integer ID of the target element (e.g. `@3`).
    pub mark_id: u32,

    /// The interaction type to execute.
    pub action: InteractionKind,

    /// Optional text payload for typing actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// The set of mechanical interactions the driver can execute.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    Click,
    Type,
    Select,
    Scroll,
    Hover,
    Wait,
    Navigate,
}

/// An ordered list of actions returned by the LLM planner in a single
/// turn.  This is bounded by the `max_tokens=1024` completion budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlan {
    /// The natural-language rationale for this plan (for TUI display).
    pub reasoning: String,

    /// The ordered list of mechanical actions.
    pub actions: Vec<PlannedAction>,

    /// Whether the plan is considered complete (no more LLM turns needed).
    pub is_complete: bool,
}

// ── Macro Cache Record ──────────────────────────────────────────────

/// A deterministic macro: a recorded sequence of actions that can be
/// replayed without invoking the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroRecord {
    /// The xxHash64 cache key derived from domain + intent hash.
    pub cache_key: u64,

    /// The domain this macro was recorded on.
    pub domain: String,

    /// The original user prompt that triggered the recording.
    pub prompt: String,

    /// The ordered action sequence.
    pub actions: Vec<PlannedAction>,

    /// Timestamp of when this macro was last successfully verified.
    pub verified_at: DateTime<Utc>,
}

// ── Network Sniffer Intercept ───────────────────────────────────────

/// A JSON response intercepted by the CDP network sniffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptedResponse {
    /// The CDP request ID that produced this response.
    pub request_id: String,

    /// The request URL.
    pub url: String,

    /// The HTTP status code.
    pub status: u16,

    /// The MIME type (expected: `application/json`).
    pub mime_type: String,

    /// The raw JSON body (already validated as parseable).
    pub body: serde_json::Value,

    /// Timestamp of interception.
    pub intercepted_at: DateTime<Utc>,
}

// ── Session Vault Payload ───────────────────────────────────────────

/// The plaintext session payload encrypted at rest by the vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPayload {
    /// The origin domain.
    pub domain: String,

    /// Authenticated cookies as `name=value` pairs.
    pub cookies: Vec<CookieEntry>,

    /// localStorage key-value pairs.
    #[serde(default)]
    pub local_storage: Vec<StorageEntry>,

    /// When this session was last refreshed.
    pub captured_at: DateTime<Utc>,
}

/// A single browser cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieEntry {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<f64>,
}

/// A single localStorage entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageEntry {
    pub key: String,
    pub value: String,
}

// ── Crawl Node ──────────────────────────────────────────────────────

/// A node in the recursive crawler's frontier queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlNode {
    /// The URL to visit.
    pub url: String,

    /// The depth at which this node was discovered.
    pub depth: u32,

    /// The URL of the page that linked to this node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
}

// ── Set-of-Marks Entry ──────────────────────────────────────────────

/// A single element in the compressed Set-of-Marks accessibility index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkEntry {
    /// The integer mark ID (`@1`, `@2`, …).
    pub id: u32,

    /// The accessibility role (e.g. "button", "link", "textbox").
    pub role: String,

    /// The accessible name / visible label.
    pub name: String,

    /// The current value (for text fields, combo boxes, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// The bounding box in viewport coordinates `[x, y, width, height]`.
    pub bounds: [f64; 4],

    /// The CDP backend node ID for direct targeting.
    pub backend_node_id: i64,
}

// ── Extracted Data Row ──────────────────────────────────────────────

/// A generic key-value row written to the SQLite datastore by either
/// the network sniffer or the DOM extraction pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataRow {
    /// The source URL from which this row was extracted.
    pub source_url: String,

    /// The table/collection name for routing.
    pub table_name: String,

    /// The row payload as a flat JSON object.
    pub payload: serde_json::Value,

    /// Extraction timestamp.
    pub extracted_at: DateTime<Utc>,
}

// ── HITL Notification ───────────────────────────────────────────────

/// Notification payload sent to the TUI when human intervention is required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlNotification {
    /// The type of security challenge detected.
    pub challenge_type: ChallengeKind,

    /// A human-readable description of what the user needs to do.
    pub message: String,

    /// The current page URL.
    pub url: String,
}

/// Classification of security challenges that trigger HITL handoff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeKind {
    Captcha,
    Mfa,
    SmsOtp,
    EmailVerification,
    Unknown,
}

// ── AI Autonomous Page Evaluation & Extraction ─────────────────────

/// Evaluation and structured extraction result emitted by the AI evaluator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPageEvaluation {
    /// Whether the page content is valuable and valid according to the extraction goal.
    pub is_valuable: bool,
    /// Confidence or relevance score between 0.0 and 1.0.
    pub relevance_score: f64,
    /// Semantic category of the page (e.g., "repository", "article", "documentation", "noise", "login_wall").
    pub category: String,
    /// Summary of the valuable information on the page.
    pub summary: String,
    /// Structured entities and key-value records extracted autonomously by AI.
    #[serde(default)]
    pub extracted_data: serde_json::Value,
    /// Quality rationale explaining why this was accepted or rejected.
    #[serde(default)]
    pub rationale: String,
}
