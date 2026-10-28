// ──────────────────────────────────────────────────────────────────────
// harness-agent/src/lib.rs — Public API for harness-agent crate
//
// Provides:
//  • triage — Pre-flight capability triage & rejection gatekeeper
//  • search — Zero-URL canonical search resolver (<120ms DuckDuckGo)
//  • groq   — Native async Groq Llama-3.3-70B ReAct client
// ──────────────────────────────────────────────────────────────────────

pub mod groq;
pub mod router;
pub mod search;
pub mod triage;

// Public re-exports for downstream use
pub use groq::{
    ChatMessage, GroqClient, GroqConfig, MessageRole, DEFAULT_MODEL,
    MAX_PLANNING_COMPLETION_TOKENS, MAX_TRIAGE_COMPLETION_TOKENS,
};
pub use router::ToolRouter;
pub use search::{SearchResult, ZeroUrlResolver};
pub use triage::{TriageDecision, TriageGatekeeper, REJECTION_LITERAL};
