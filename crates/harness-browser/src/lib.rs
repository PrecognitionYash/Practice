// ──────────────────────────────────────────────────────────────────────
// harness-browser/src/lib.rs — Public API for harness-browser crate
//
// Provides:
//  • driver        — Direct CDP WebSocket lifecycle, stealth, and Bézier input
//  • accessibility — Set-of-Marks AXTree compressor (<1,200 tokens)
//  • firewall      — Prompt injection scrubber & zero-pixel sanitizer
//  • sniffer       — Zero-DOM network JSON sniffer directly to SQLite WAL
//  • crawler       — Recursive fast-path HTTP/2 crawler with xxHash64 cycle check
//  • replay        — Deterministic macro caching & self-healing replay engine
// ──────────────────────────────────────────────────────────────────────

pub mod accessibility;
pub mod crawler;
pub mod driver;
pub mod firewall;
pub mod replay;
pub mod sniffer;

// Public re-exports for ergonomic usage across the workspace
pub use accessibility::{AxTreeCompressor, PerceptionSnapshot, MAX_PERCEPTION_TOKENS};
pub use chromiumoxide::Page;
pub use crawler::{CrawlerConfig, CrawlerMetrics, FastPathCrawler};
pub use driver::{BrowserDriver, BrowserDriverConfig, Point, StealthMode};
pub use firewall::{PromptFirewall, REDACTION_TOKEN};
pub use replay::{MacroReplayEngine, ReplayOutcome};
pub use sniffer::{NetworkSniffer, SnifferConfig, SnifferMetrics};
