// ──────────────────────────────────────────────────────────────────────
// harness-core/src/error.rs — Unified Error Taxonomy
//
// Every subsystem error in the harness workspace routes through this
// single, exhaustive enum. We use `thiserror` for zero-boilerplate
// Display/Error derivation and `#[from]` for transparent conversions.
// ──────────────────────────────────────────────────────────────────────

use std::path::PathBuf;
use thiserror::Error;

/// Canonical error type for the entire harness workspace.
///
/// Subsystems propagate domain-specific error variants upward through
/// this enum. All variants carry structured context sufficient for
/// post-mortem debugging without inspecting stack traces.
#[derive(Debug, Error)]
pub enum HarnessError {
    // ── Cryptographic Vault ─────────────────────────────────────────
    /// The master key derivation function (HKDF-SHA256) failed.
    #[error("vault: key derivation failed — {reason}")]
    KeyDerivation { reason: String },

    /// AES-256-GCM encryption failed for domain `{domain}`.
    #[error("vault: encryption failed for domain '{domain}' — {reason}")]
    Encryption { domain: String, reason: String },

    /// AES-256-GCM decryption failed — ciphertext may be tampered.
    #[error("vault: decryption failed for domain '{domain}' — {reason}")]
    Decryption { domain: String, reason: String },

    /// A vault file could not be read or written.
    #[error("vault: I/O error on path '{}' — {source}", path.display())]
    VaultIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ── SQLite Datastore ────────────────────────────────────────────
    /// A database query or migration failed.
    #[error("store: database error — {0}")]
    Database(#[from] sqlx::Error),

    /// The requested table does not exist.
    #[error("store: table '{table}' does not exist in the datastore")]
    TableNotFound { table: String },

    /// A migration script failed to apply.
    #[error("store: migration failed — {reason}")]
    Migration { reason: String },

    // ── Schema / Serialisation ──────────────────────────────────────
    /// JSON serialisation or deserialisation failed.
    #[error("schema: JSON error — {0}")]
    Json(#[from] serde_json::Error),

    /// A required field was missing in an exchange payload.
    #[error("schema: missing field '{field}' in {context}")]
    MissingField { field: String, context: String },

    // ── Pre-Flight Triage ───────────────────────────────────────────
    /// The pre-flight gatekeeper rejected the user prompt.
    #[error("Sorry, sir. I cannot do that.")]
    Rejected {
        /// The reason returned by the classifier.
        reason: String,
    },

    // ── Browser / CDP ───────────────────────────────────────────────
    /// The CDP WebSocket connection dropped or timed out.
    #[error("browser: CDP connection error — {reason}")]
    CdpConnection { reason: String },

    /// An element could not be located in the current DOM frame.
    #[error("browser: element not found — selector '{selector}'")]
    ElementNotFound { selector: String },

    /// The browser process exited unexpectedly.
    #[error("browser: process exited with code {code}")]
    BrowserCrash { code: i32 },

    // ── Network / HTTP ──────────────────────────────────────────────
    /// An HTTP request failed.
    #[error("network: HTTP error — {reason}")]
    Http { reason: String },

    /// DNS or TCP connection could not be established.
    #[error("network: connection failed to '{host}' — {reason}")]
    Connection { host: String, reason: String },

    // ── HITL (Human-in-the-Loop) ────────────────────────────────────
    /// The autonomous loop detected a challenge requiring human input.
    #[error("hitl: human intervention required — {challenge_type}")]
    HumanInterventionRequired { challenge_type: String },

    // ── TUI / Commands ──────────────────────────────────────────────
    /// An invalid slash command was entered.
    #[error("tui: unknown command '/{command}'")]
    UnknownCommand { command: String },

    /// A slash command received invalid arguments.
    #[error("tui: invalid arguments for '/{command}' — {reason}")]
    InvalidArgs { command: String, reason: String },

    // ── Generic ─────────────────────────────────────────────────────
    /// An I/O error that does not fit a more specific variant.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Catch-all for unexpected internal errors. Prefer specific
    /// variants wherever possible.
    #[error("internal: {0}")]
    Internal(String),
}

/// Convenience alias used throughout the workspace.
pub type HarnessResult<T> = std::result::Result<T, HarnessError>;
