//! High-Performance Autonomous Browser Harness — Primary CLI binary.

#[tokio::main]
async fn main() -> harness_core::error::HarnessResult<()> {
    harness_tui::run_app().await
}
