// ──────────────────────────────────────────────────────────────────────
// harness-core/src/lib.rs — Crate root & public API re-exports
//
// This crate provides the foundational types shared across the entire
// harness workspace:
//   • error  — Unified error taxonomy (`HarnessError`, `HarnessResult`)
//   • schema — Serde exchange models for all inter-module boundaries
//   • vault  — AES-256-GCM + HKDF cryptographic session vault
//   • store  — High-throughput SQLite WAL datastore
// ──────────────────────────────────────────────────────────────────────

pub mod error;
pub mod schema;
pub mod store;
pub mod vault;

// Re-export primary types for ergonomic downstream usage.
pub use error::{HarnessError, HarnessResult};
pub use store::DataStore;
pub use vault::CryptoVault;
