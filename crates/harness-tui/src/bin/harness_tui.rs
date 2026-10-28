//! High-Performance Autonomous Browser Harness — TUI binary alias.

#[tokio::main]
async fn main() -> harness_core::error::HarnessResult<()> {
    harness_tui::run_app().await
}
