// ──────────────────────────────────────────────────────────────────────
// harness-agent/src/triage.rs — Pre-Flight Triage & Capability Gatekeeper
//
// Eliminates wasteful browser process spawning, network allocations,
// and LLM reasoning loops for out-of-scope, unresolvable, or hostile queries.
//
// System Invariants Enforced:
//  • The Rejection Invariant: If is_capable == false, logs the exact
//    literal string: "Sorry, sir. I cannot do that." and halts immediately
//    with zero browser handles allocated.
//  • Strict 80-token classification envelope over Groq API.
//  • Zero-URL Search Resolver delegation when action_type == DirectSearch.
// ──────────────────────────────────────────────────────────────────────

use harness_core::error::{HarnessError, HarnessResult};
use harness_core::schema::{ActionType, TriageEnvelope};
use tracing::{error, info, instrument};

use crate::groq::GroqClient;
use crate::search::ZeroUrlResolver;

/// The canonical rejection message mandated by the system invariants.
pub const REJECTION_LITERAL: &str = "Sorry, sir. I cannot do that.";

/// Decision outcome emitted by the triage gatekeeper.
#[derive(Debug, Clone)]
pub struct TriageDecision {
    /// The structured classification envelope.
    pub envelope: TriageEnvelope,
    /// The target URL for navigation or crawling.
    pub target_url: String,
}

/// The pre-flight capability triage gatekeeper.
#[derive(Clone)]
pub struct TriageGatekeeper {
    groq: GroqClient,
    search_resolver: ZeroUrlResolver,
}

impl TriageGatekeeper {
    /// Create a new triage gatekeeper with the given Groq client and search resolver.
    pub fn new(groq: GroqClient, search_resolver: ZeroUrlResolver) -> Self {
        Self {
            groq,
            search_resolver,
        }
    }

    /// Evaluates a raw user prompt through the pre-flight capability gatekeeper.
    ///
    /// If rejected, emits "Sorry, sir. I cannot do that." and returns `HarnessError::Rejected`.
    /// If authorized, resolves any missing target URL via zero-URL search and returns `TriageDecision`.
    #[instrument(skip(self, prompt), fields(prompt = %prompt))]
    pub async fn evaluate(&self, prompt: &str) -> HarnessResult<TriageDecision> {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            return Err(HarnessError::InvalidArgs {
                command: "prompt".into(),
                reason: "prompt cannot be empty".into(),
            });
        }

        info!("running pre-flight capability triage classification (80-token envelope)");

        // 1. Invoke classification envelope on Groq
        let envelope = self.groq.triage_classify(trimmed).await?;

        // 2. The Rejection Invariant
        if !envelope.is_capable || envelope.action_type == ActionType::Incapable {
            let reason = envelope
                .rejection_reason
                .clone()
                .unwrap_or_else(|| "task exceeds operational capability envelope".to_string());

            // Mandated exact literal log output
            println!("{REJECTION_LITERAL}");
            error!(reason = %reason, "{REJECTION_LITERAL}");

            return Err(HarnessError::Rejected { reason });
        }

        // 3. Resolve target URL
        let target_url = if let Some(ref url) = envelope.target_url {
            url.clone()
        } else if let Some(ref query) = envelope.search_query {
            // Trigger Autonomous Zero-URL Search Resolver
            info!(query = %query, "resolving query to canonical URL via fast-path search");
            let result = self.search_resolver.resolve(query).await?;
            result.target_url
        } else {
            // Fallback: search for the full prompt text directly
            info!(query = %trimmed, "no explicit search query in envelope; resolving prompt text");
            let result = self.search_resolver.resolve(trimmed).await?;
            result.target_url
        };

        info!(
            action = ?envelope.action_type,
            target_url = %target_url,
            "task authorized by pre-flight gatekeeper"
        );

        Ok(TriageDecision {
            envelope,
            target_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rejection_literal_exact_string() {
        assert_eq!(REJECTION_LITERAL, "Sorry, sir. I cannot do that.");
    }
}
