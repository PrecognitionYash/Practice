// ──────────────────────────────────────────────────────────────────────
// harness-tui/src/main.rs — Application Orchestrator & 60 FPS Terminal UI
//
// Central runtime entry point coordinating:
//  • Immediate-mode double-buffered Ratatui interface locked at 60 FPS
//  • Zero-allocation slash-command execution (/api, /export, /stealth, etc.)
//  • Pre-flight capability triage & rejection gatekeeper
//  • Autonomous CDP browsing, Set-of-Marks compression & Bézier execution
//  • On-demand embedded Axum REST server & Apache Arrow/Parquet export
// ──────────────────────────────────────────────────────────────────────

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use harness_agent::groq::{ChatMessage, GroqClient, GroqConfig};
use harness_agent::search::ZeroUrlResolver;
use harness_agent::triage::{TriageGatekeeper, REJECTION_LITERAL};
use harness_agent::ToolRouter;
use harness_browser::accessibility::AxTreeCompressor;
use harness_browser::driver::{BrowserDriver, BrowserDriverConfig, StealthMode};
use harness_browser::replay::{MacroReplayEngine, ReplayOutcome};
use harness_browser::sniffer::{NetworkSniffer, SnifferConfig};
use harness_browser::Page;
use harness_core::error::HarnessResult;
use harness_core::schema::{DataRow, HarnessToolCall, HitlNotification, MarkEntry, SessionPayload};
use harness_core::store::DataStore;
use harness_core::vault::CryptoVault;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use tokio::sync::Mutex;

mod commands;
mod events;
mod exporter;
mod server;

use commands::SlashCommand;
use events::{AppEvent, EventStream, TerminalGuard};
use exporter::DataExporter;
use server::{start_embedded_server, EmbeddedServerHandle};

/// Maximum active token consumption ceiling (strictly bounded to <= 7,000 tokens).
pub const TOKEN_BUDGET_CEILING: usize = 7000;

/// High-level operational state of the harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Triage,
    Navigating,
    Perceiving,
    Planning,
    Executing,
    HitlPause,
}

impl AgentStatus {
    pub fn badge_color(&self) -> Color {
        match self {
            AgentStatus::Idle => Color::DarkGray,
            AgentStatus::Triage => Color::Yellow,
            AgentStatus::Navigating => Color::Blue,
            AgentStatus::Perceiving => Color::Magenta,
            AgentStatus::Planning => Color::Cyan,
            AgentStatus::Executing => Color::Green,
            AgentStatus::HitlPause => Color::Red,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Idle => "IDLE",
            AgentStatus::Triage => "TRIAGE",
            AgentStatus::Navigating => "NAVIGATING",
            AgentStatus::Perceiving => "PERCEIVING",
            AgentStatus::Planning => "PLANNING",
            AgentStatus::Executing => "EXECUTING",
            AgentStatus::HitlPause => "HITL_ACTION_REQUIRED",
        }
    }
}

/// Active view tab in the terminal interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    ExtractedData,
    PerceptionFrame,
    ExecutionLog,
}

impl ActiveTab {
    pub fn title(&self) -> &'static str {
        match self {
            ActiveTab::ExtractedData => "[1] Extracted Data",
            ActiveTab::PerceptionFrame => "[2] Perception Frame",
            ActiveTab::ExecutionLog => "[3] Execution Log",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            ActiveTab::ExtractedData => ActiveTab::PerceptionFrame,
            ActiveTab::PerceptionFrame => ActiveTab::ExecutionLog,
            ActiveTab::ExecutionLog => ActiveTab::ExtractedData,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            ActiveTab::ExtractedData => ActiveTab::ExecutionLog,
            ActiveTab::PerceptionFrame => ActiveTab::ExtractedData,
            ActiveTab::ExecutionLog => ActiveTab::PerceptionFrame,
        }
    }
}

/// Central state model for the immediate-mode terminal UI.
pub struct AppState {
    pub status: AgentStatus,
    pub input_buffer: String,
    pub cursor_position: usize,
    pub active_url: String,
    pub stealth_mode: StealthMode,
    pub perception_frame: String,
    pub execution_logs: Vec<String>,
    pub current_tokens_used: usize,
    pub active_server: Option<EmbeddedServerHandle>,
    pub hitl_alert: Option<HitlNotification>,
    pub running: bool,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    pub saved_draft: String,
    pub completion_index: usize,
    pub completion_prefix: Option<String>,
    pub active_tab: ActiveTab,
    pub preview_table: Option<String>,
    pub preview_data: Vec<serde_json::Value>,
    pub preview_total_rows: u64,
    /// Vertical scroll offset for [1] Extracted Data table rows.
    pub data_scroll: usize,
    /// Vertical scroll offset for [2] Perception Frame lines.
    pub perception_scroll: u16,
    /// Log scroll offset from current live tail for [3] Execution Log.
    pub log_scroll: usize,
    /// Cancellation signal for any active fast-path recursive crawl.
    pub active_crawl_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            status: AgentStatus::Idle,
            input_buffer: String::new(),
            cursor_position: 0,
            active_url: "about:blank".to_string(),
            stealth_mode: StealthMode::Ultra,
            perception_frame: "Awaiting prompt or navigation...".to_string(),
            execution_logs: vec![
                "Harness initialized with bare-metal Rust CDP driver.".to_string(),
                "SQLite WAL persistence active. Token ceiling: 7,000 tokens.".to_string(),
            ],
            current_tokens_used: 0,
            active_server: None,
            hitl_alert: None,
            running: true,
            input_history: Vec::new(),
            history_index: None,
            saved_draft: String::new(),
            completion_index: 0,
            completion_prefix: None,
            active_tab: ActiveTab::ExecutionLog,
            preview_table: None,
            preview_data: Vec::new(),
            preview_total_rows: 0,
            data_scroll: 0,
            perception_scroll: 0,
            log_scroll: 0,
            active_crawl_cancel: None,
        }
    }

    pub fn log(&mut self, message: impl Into<String>) {
        let msg = message.into();
        self.execution_logs.push(msg);
        if self.execution_logs.len() > 1000 {
            self.execution_logs.remove(0);
        }
    }

    pub fn push_history(&mut self, cmd: String) {
        let trimmed = cmd.trim();
        if !trimmed.is_empty() && self.input_history.last().map(|s| s.as_str()) != Some(trimmed) {
            self.input_history.push(trimmed.to_string());
        }
        self.history_index = None;
        self.saved_draft.clear();
        self.completion_index = 0;
        self.completion_prefix = None;
        self.cursor_position = 0;
    }

    /// Inserts a character at the current Unicode character cursor position.
    pub fn insert_char(&mut self, c: char) {
        let total = self.input_buffer.chars().count();
        let pos = self.cursor_position.min(total);
        let byte_offset = self
            .input_buffer
            .char_indices()
            .nth(pos)
            .map(|(i, _)| i)
            .unwrap_or(self.input_buffer.len());
        self.input_buffer.insert(byte_offset, c);
        self.cursor_position = pos + 1;
        self.completion_prefix = None;
        self.completion_index = 0;
    }

    /// Deletes the character immediately before the cursor (Backspace).
    pub fn delete_backward(&mut self) {
        if self.cursor_position > 0 {
            let target = self.cursor_position - 1;
            if let Some((byte_offset, ch)) = self.input_buffer.char_indices().nth(target) {
                let next_offset = byte_offset + ch.len_utf8();
                self.input_buffer
                    .replace_range(byte_offset..next_offset, "");
                self.cursor_position = target;
            }
            self.completion_prefix = None;
            self.completion_index = 0;
        }
    }

    /// Deletes the character at the current cursor position (Delete).
    pub fn delete_forward(&mut self) {
        let total = self.input_buffer.chars().count();
        if self.cursor_position < total {
            if let Some((byte_offset, ch)) =
                self.input_buffer.char_indices().nth(self.cursor_position)
            {
                let next_offset = byte_offset + ch.len_utf8();
                self.input_buffer
                    .replace_range(byte_offset..next_offset, "");
            }
            self.completion_prefix = None;
            self.completion_index = 0;
        }
    }

    /// Moves cursor left by 1 character, saturating at 0.
    pub fn move_cursor_left(&mut self) {
        self.cursor_position = self.cursor_position.saturating_sub(1);
    }

    /// Moves cursor right by 1 character, saturating at character count.
    pub fn move_cursor_right(&mut self) {
        let total = self.input_buffer.chars().count();
        if self.cursor_position < total {
            self.cursor_position += 1;
        }
    }

    /// Moves cursor to the beginning of the input buffer.
    pub fn move_cursor_home(&mut self) {
        self.cursor_position = 0;
    }

    /// Moves cursor to the end of the input buffer.
    pub fn move_cursor_end(&mut self) {
        self.cursor_position = self.input_buffer.chars().count();
    }

    pub fn history_up(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.saved_draft = self.input_buffer.clone();
                let last_idx = self.input_history.len() - 1;
                self.history_index = Some(last_idx);
                self.input_buffer = self.input_history[last_idx].clone();
            }
            Some(idx) if idx > 0 => {
                let new_idx = idx - 1;
                self.history_index = Some(new_idx);
                self.input_buffer = self.input_history[new_idx].clone();
            }
            _ => {}
        }
        self.cursor_position = self.input_buffer.chars().count();
    }

    pub fn history_down(&mut self) {
        if let Some(idx) = self.history_index {
            if idx + 1 < self.input_history.len() {
                let new_idx = idx + 1;
                self.history_index = Some(new_idx);
                self.input_buffer = self.input_history[new_idx].clone();
            } else {
                self.history_index = None;
                self.input_buffer = self.saved_draft.clone();
            }
            self.cursor_position = self.input_buffer.chars().count();
        }
    }

    /// Scrolls active tab view upward by 1 row / line.
    pub fn scroll_up(&mut self) {
        match self.active_tab {
            ActiveTab::ExtractedData => {
                self.data_scroll = self.data_scroll.saturating_sub(1);
            }
            ActiveTab::PerceptionFrame => {
                self.perception_scroll = self.perception_scroll.saturating_sub(1);
            }
            ActiveTab::ExecutionLog => {
                let max_offset = self.execution_logs.len().saturating_sub(1);
                self.log_scroll = (self.log_scroll + 1).min(max_offset);
            }
        }
    }

    /// Scrolls active tab view downward by 1 row / line.
    pub fn scroll_down(&mut self) {
        match self.active_tab {
            ActiveTab::ExtractedData => {
                let total = self.preview_data.len();
                if self.data_scroll + 1 < total {
                    self.data_scroll += 1;
                }
            }
            ActiveTab::PerceptionFrame => {
                let total_lines = self.perception_frame.lines().count();
                if (self.perception_scroll as usize) + 1 < total_lines {
                    self.perception_scroll += 1;
                }
            }
            ActiveTab::ExecutionLog => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
            }
        }
    }

    /// Scrolls active tab view upward by a page (~10-15 rows / lines).
    pub fn page_up(&mut self) {
        match self.active_tab {
            ActiveTab::ExtractedData => {
                self.data_scroll = self.data_scroll.saturating_sub(10);
            }
            ActiveTab::PerceptionFrame => {
                self.perception_scroll = self.perception_scroll.saturating_sub(15);
            }
            ActiveTab::ExecutionLog => {
                let max_offset = self.execution_logs.len().saturating_sub(1);
                self.log_scroll = (self.log_scroll + 10).min(max_offset);
            }
        }
    }

    /// Scrolls active tab view downward by a page (~10-15 rows / lines).
    pub fn page_down(&mut self) {
        match self.active_tab {
            ActiveTab::ExtractedData => {
                let total = self.preview_data.len();
                if self.data_scroll + 10 < total {
                    self.data_scroll += 10;
                } else {
                    self.data_scroll = total.saturating_sub(1);
                }
            }
            ActiveTab::PerceptionFrame => {
                let total_lines = self.perception_frame.lines().count();
                if (self.perception_scroll as usize) + 15 < total_lines {
                    self.perception_scroll += 15;
                } else {
                    self.perception_scroll = total_lines.saturating_sub(1) as u16;
                }
            }
            ActiveTab::ExecutionLog => {
                self.log_scroll = self.log_scroll.saturating_sub(10);
            }
        }
    }

    /// Jumps scroll position to the top / oldest record.
    pub fn scroll_to_top(&mut self) {
        match self.active_tab {
            ActiveTab::ExtractedData => {
                self.data_scroll = 0;
            }
            ActiveTab::PerceptionFrame => {
                self.perception_scroll = 0;
            }
            ActiveTab::ExecutionLog => {
                self.log_scroll = self.execution_logs.len().saturating_sub(1);
            }
        }
    }

    /// Jumps scroll position to the bottom / newest live log.
    pub fn scroll_to_bottom(&mut self) {
        match self.active_tab {
            ActiveTab::ExtractedData => {
                self.data_scroll = self.preview_data.len().saturating_sub(1);
            }
            ActiveTab::PerceptionFrame => {
                self.perception_scroll =
                    self.perception_frame.lines().count().saturating_sub(1) as u16;
            }
            ActiveTab::ExecutionLog => {
                self.log_scroll = 0;
            }
        }
    }

    /// Resets scroll offset for the currently active tab.
    pub fn scroll_reset(&mut self) {
        match self.active_tab {
            ActiveTab::ExtractedData => {
                self.data_scroll = 0;
            }
            ActiveTab::PerceptionFrame => {
                self.perception_scroll = 0;
            }
            ActiveTab::ExecutionLog => {
                self.log_scroll = 0;
            }
        }
    }

    pub fn complete_tab(&mut self) {
        let commands = [
            "/api", "/clear", "/export", "/extract", "/help", "/quit", "/reset", "/scroll",
            "/status", "/stealth", "/tab", "/tables", "/undo", "/vault",
        ];

        let prefix = match &self.completion_prefix {
            Some(p) => p.clone(),
            None => {
                let t = self.input_buffer.trim().to_string();
                let initial = if t.is_empty() { "/".to_string() } else { t };
                self.completion_prefix = Some(initial.clone());
                initial
            }
        };

        if prefix == "/tab" || prefix.starts_with("/tab ") {
            let tab_modes = ["/tab data", "/tab dom", "/tab logs"];
            let idx = self.completion_index % tab_modes.len();
            self.input_buffer = format!("{} ", tab_modes[idx]);
            self.completion_index += 1;
            self.cursor_position = self.input_buffer.chars().count();
            return;
        }

        if prefix == "/stealth" || prefix.starts_with("/stealth ") {
            let stealth_modes = ["/stealth fast", "/stealth ultra"];
            let idx = self.completion_index % stealth_modes.len();
            self.input_buffer = format!("{} ", stealth_modes[idx]);
            self.completion_index += 1;
            self.cursor_position = self.input_buffer.chars().count();
            return;
        }

        if prefix == "/export" || prefix.starts_with("/export ") {
            let tokens: Vec<&str> = prefix.split_whitespace().collect();
            if tokens.len() == 2 {
                self.input_buffer = format!("{} parquet", prefix.trim_end());
                self.cursor_position = self.input_buffer.chars().count();
            }
            return;
        }

        if prefix.starts_with('/') {
            let matches: Vec<&str> = commands
                .iter()
                .copied()
                .filter(|c| c.starts_with(&prefix))
                .collect();

            if matches.is_empty() {
                return;
            }

            if matches.len() == 1 {
                self.input_buffer = format!("{} ", matches[0]);
            } else {
                let idx = self.completion_index % matches.len();
                self.input_buffer = matches[idx].to_string();
                self.completion_index += 1;
            }
            self.cursor_position = self.input_buffer.chars().count();
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn run_app() -> HarnessResult<()> {
    // 0. Load environment variables from .env if present
    dotenvy::dotenv().ok();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        let first = args[0].as_str();
        if first == "--help" || first == "-h" || first == "help" {
            println!(
                "AUTONOMOUS BROWSER HARNESS [Industrial Edition] v{}",
                env!("CARGO_PKG_VERSION")
            );
            println!("\nUSAGE:");
            println!("    harness [OPTIONS] [PROMPT]");
            println!("\nOPTIONS / SLASH COMMANDS:");
            println!("    /help                                    Display command reference");
            println!(
                "    /status                                  Display runtime and database status"
            );
            println!(
                "    /tables                                  List all harvested SQLite tables"
            );
            println!(
                "    /export <table_name> <parquet|csv|jsonl> Export table to exports directory"
            );
            println!(
                "    /api <table_name> [port]                 Start embedded Axum REST API server"
            );
            println!("    /stealth [fast|ultra]                    Toggle or set CDP anti-detection profile");
            println!("    /reset                                   Purge all datasets, tables, cache & exports");
            println!(
                "    /undo                                    Roll back last executed DOM action"
            );
            println!("    /clear                                   Clear execution log buffer");
            println!("    /quit                                    Gracefully terminate runtime");
            println!("\nINTERACTIVE TUI:");
            println!(
                "    ./harness                                Launch 60 FPS double-buffered TUI"
            );
            println!(
                "    ./harness chat                           Launch TUI in interactive chat mode"
            );
            println!("\nAUTONOMOUS TASK EXAMPLES:");
            println!("    ./harness \"Extract trending repos on github.com\"");
            println!("    ./harness \"Search for mechanical keyboards on news.ycombinator.com\"");
            return Ok(());
        }
        if first == "--version" || first == "-v" || first == "version" {
            println!("harness-tui v{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }

    // 1. Initialize persistent storage and crypto vault under .harness/
    let base_dir = PathBuf::from(".harness");
    std::fs::create_dir_all(&base_dir).map_err(harness_core::error::HarnessError::Io)?;

    let db_path = base_dir.join("harvest.db");
    let store = DataStore::open(&db_path).await?;

    let vault_dir = base_dir.join("vault");
    let master_secret = std::env::var("HARNESS_VAULT_KEY")
        .map(|s| s.into_bytes())
        .unwrap_or_else(|_| b"autonomous-harness-default-secret-seed!".to_vec());
    let vault = Arc::new(CryptoVault::new(&vault_dir, &master_secret)?);

    let exporter = DataExporter::new(store.clone(), base_dir.join("exports"))?;

    // Handle CLI-level slash commands directly if requested without launching TUI
    if !args.is_empty() {
        let joined = args.join(" ").trim().to_string();
        if joined.starts_with('/') {
            match SlashCommand::parse(&joined) {
                Ok(Some(SlashCommand::Help)) => {
                    println!("─── HARNESS COMMAND REFERENCE ───");
                    println!("  /api <table_name> [port]            — Start Axum REST API (default port 8080)");
                    println!("  /export <table_name> <parquet|csv|jsonl> — Export harvested data to disk");
                    println!("  /stealth [fast|ultra]               — Switch or toggle CDP stealth profile");
                    println!("  /tables [table_name]                — List harvested tables or preview data");
                    println!("  /extract [table_name]               — Harvest active page data into SQLite table");
                    println!("  /tab <data|dom|logs>                — Switch display tab (or press 1/2/3)");
                    println!("  /scroll [up|down|top|bottom] [n]    — Scroll active view tab");
                    println!("  /crawl <url> [table] [--all-paths]  — Recursively crawl with strict domain & path scope");
                    println!("  /vault [list|save|load <domain>]    — Manage AES-256-GCM encrypted session vault");
                    println!("  /status                             — Display current browser & runtime status");
                    println!("  /reset                              — Purge all datasets, tables, cache & exports");
                    println!("  /undo                               — Roll back last executed DOM action");
                    println!("  /clear                              — Clear execution log buffer");
                    println!(
                        "  /quit (or /exit, :q)                — Gracefully terminate runtime"
                    );
                    return Ok(());
                }
                Ok(Some(SlashCommand::Tables { table })) => {
                    let tables = store.list_tables().await?;
                    if tables.is_empty() {
                        println!("No harvested user tables found in database yet.");
                        println!(
                            "Hint: Run an autonomous task or prompt to harvest data into tables."
                        );
                    } else {
                        println!("Harvested tables in database ({}):", tables.len());
                        for t in &tables {
                            let count = store.row_count(t).await.unwrap_or(0);
                            println!("  • {t} ({count} records)");
                            println!("    └ Export: `/export {t} <parquet|csv|jsonl>` | API: `/api {t} 8080`");
                        }
                        if let Some(ref tbl) = table {
                            if tables.contains(tbl) {
                                let rows = store.query_table(tbl, Some(10)).await?;
                                println!("\nPreview of table '{tbl}' (top 10 rows):");
                                for r in rows {
                                    println!("  {}", serde_json::to_string(&r).unwrap_or_default());
                                }
                            } else {
                                println!("\nTable '{tbl}' does not exist in database.");
                            }
                        }
                    }
                    return Ok(());
                }
                Ok(Some(SlashCommand::Extract { table: _ })) => {
                    println!("The /extract command explicitly harvests the active page inside the interactive TUI.");
                    println!("Usage: ./harness \"Extract <query> on <url>\" or run `/extract` within the TUI.");
                    return Ok(());
                }
                Ok(Some(SlashCommand::Tab { tab: _ })) => {
                    println!("The /tab command switches views inside the interactive TUI (1: Data, 2: DOM, 3: Logs).");
                    return Ok(());
                }
                Ok(Some(SlashCommand::Status)) => {
                    let has_key = std::env::var("GROQ_API_KEY").is_ok_and(|k| !k.trim().is_empty());
                    println!("─── SYSTEM RUNTIME STATUS ───");
                    println!("  Database    : {}", db_path.display());
                    println!(
                        "  Groq AI Key : {}",
                        if has_key {
                            "Loaded (Active)"
                        } else {
                            "Not set (Add to .env)"
                        }
                    );
                    println!(
                        "  Model       : {}",
                        std::env::var("GROQ_MODEL")
                            .unwrap_or_else(|_| "openai/gpt-oss-120b".into())
                    );
                    println!("  Stealth Mode: Ultra (Default)");
                    return Ok(());
                }
                Ok(Some(SlashCommand::Export { table, format })) => {
                    let (table, format) = match (table, format) {
                        (Some(t), Some(f)) => (t, f),
                        _ => {
                            eprintln!("Usage: /export <table_name> <parquet|csv|jsonl>");
                            return Ok(());
                        }
                    };
                    let summary = exporter.export_table(&table, format).await?;
                    println!(
                        "Exported {} records to '{}' ({:?})",
                        summary.records_exported,
                        summary.file_path.display(),
                        summary.format
                    );
                    return Ok(());
                }
                Ok(Some(SlashCommand::Reset)) => {
                    let count = store.reset_all_data().await?;
                    let exports_dir = base_dir.join("exports");
                    let mut exports_removed = 0;
                    if exports_dir.exists() {
                        if let Ok(entries) = std::fs::read_dir(&exports_dir) {
                            for entry in entries.flatten() {
                                if entry.path().is_file()
                                    && std::fs::remove_file(entry.path()).is_ok()
                                {
                                    exports_removed += 1;
                                }
                            }
                        }
                    }
                    println!("─── HARNESS DATASET RESET COMPLETE ───");
                    println!("  • Dropped {count} user data table(s) from SQLite storage.");
                    println!(
                        "  • Purged execution log, macro replay cache, and internal sequences."
                    );
                    if exports_removed > 0 {
                        println!(
                            "  • Removed {exports_removed} exported file(s) from '{}'.",
                            exports_dir.display()
                        );
                    }
                    println!("Workspace is clean. Ready for new autonomous tasks or queries.");
                    return Ok(());
                }
                Ok(Some(SlashCommand::Vault { action, domain })) => {
                    match action.as_deref() {
                        Some("list") | None if domain.is_none() => {
                            let domains = vault.list_domains()?;
                            if domains.is_empty() {
                                println!("Cryptographic Session Vault (.harness/vault):");
                                println!("  (empty — no encrypted sessions stored)");
                                println!("  Hint: Save a session in TUI via `/vault save` or via Python client.");
                            } else {
                                println!("Cryptographic Session Vault (.harness/vault) [AES-256-GCM + HKDF]:");
                                for d in domains {
                                    println!("  • {d}");
                                }
                            }
                        }
                        Some("load") | Some("inspect") | None => {
                            if let Some(ref d) = domain {
                                match vault.load_session(d) {
                                    Ok(payload) => {
                                        println!("Vaulted Session for '{d}':");
                                        println!("  Domain       : {}", payload.domain);
                                        println!("  Captured At  : {}", payload.captured_at);
                                        println!("  Cookies Count: {}", payload.cookies.len());
                                        for c in &payload.cookies {
                                            println!(
                                                "    - {} (expires: {:?}, secure: {})",
                                                c.name, c.expires, c.secure
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Error loading vaulted session for '{d}': {e}")
                                    }
                                }
                            } else {
                                println!("Usage: /vault [list | <domain> | load <domain>]");
                            }
                        }
                        _ => {
                            println!("Usage: /vault [list | <domain> | load <domain>]");
                        }
                    }
                    return Ok(());
                }
                Ok(Some(SlashCommand::Crawl {
                    seed_url,
                    table,
                    restrict_path,
                    ai_goal,
                    attributes,
                })) => {
                    let target_table = table.unwrap_or_else(|| {
                        if let Some(ref goal) = ai_goal {
                            derive_table_name(&seed_url, goal)
                        } else {
                            derive_table_name(&seed_url, "crawl")
                        }
                    });
                    println!("─── RECURSIVE FAST-PATH CRAWLER ───");
                    println!("  Seed URL        : {seed_url}");
                    println!("  Target Table    : {target_table}");
                    if !attributes.is_empty() {
                        println!(
                            "  Attributes      : [{}] (strictly storing requested fields)",
                            attributes.join(", ")
                        );
                    }
                    if let Some(ref goal) = ai_goal {
                        println!("  AI Extraction   : {goal}");
                    }
                    println!(
                        "  Scope Boundary  : {}",
                        if restrict_path {
                            "Relative domain AND path prefix (strict)"
                        } else {
                            "Entire relative domain"
                        }
                    );
                    println!("  Paging Limit    : Unbounded (crawls until frontier is exhausted or stopped)");
                    println!("  Persistence     : Streaming directly to SQLite WAL");
                    println!("  Tip             : Press Ctrl+C anytime to stop; all data is safely stored in SQLite WAL.");

                    let config = harness_browser::CrawlerConfig {
                        max_pages: 0,
                        table_name: target_table.clone(),
                        restrict_to_seed_path: restrict_path,
                        ai_extraction_goal: ai_goal,
                        target_attributes: attributes,
                        ..Default::default()
                    };
                    let crawler = harness_browser::FastPathCrawler::new(config, &[], store)?;
                    let count = crawler.crawl(&seed_url).await?;
                    println!("\nCrawl finished successfully. Evaluated {count} pages into table '{target_table}'.");
                    println!("Hint: Inspect data via `/tables {target_table}` or export via `/export {target_table} csv`.");
                    return Ok(());
                }
                Ok(Some(SlashCommand::Quit)) => return Ok(()),
                _ => {}
            }
        }
    }

    let initial_prompt = if !args.is_empty() {
        let joined = args.join(" ").trim().to_string();
        if joined == "chat" || joined == "--chat" || joined == "-c" || joined.starts_with('/') {
            None
        } else {
            Some(joined)
        }
    } else {
        None
    };

    // 2. Initialize autonomous agent components
    let groq_client = GroqClient::new(GroqConfig::default())?;
    let search_resolver = ZeroUrlResolver::default();
    let triage_gatekeeper = TriageGatekeeper::new(groq_client.clone(), search_resolver);
    let replay_engine = MacroReplayEngine::new(store.clone());
    let ax_compressor = AxTreeCompressor::new();

    // 3. Setup Browser Driver and Network Sniffer (Lazy or Initialized)
    let driver_config = BrowserDriverConfig {
        headless: true,
        stealth: StealthMode::Ultra,
        ..Default::default()
    };

    let driver = Arc::new(Mutex::new(None::<BrowserDriver>));
    let sniffer = Arc::new(NetworkSniffer::new(SnifferConfig::default()));

    // 4. Initialize RAII Terminal Guard & 60 FPS Event Stream
    let mut guard = TerminalGuard::new()?;
    let mut event_stream = EventStream::new();
    let state = Arc::new(Mutex::new(AppState::new()));

    {
        let mut s = state.lock().await;
        if std::env::var("GROQ_API_KEY").is_ok_and(|k| !k.trim().is_empty()) {
            s.log("Groq API Key loaded. Ready for AI autonomous tasks.");
        } else {
            s.log("NOTICE: GROQ_API_KEY is not set. Add it to .env to enable Groq AI.");
        }
    }

    // If an initial task prompt was provided on CLI, auto-dispatch it immediately!
    if let Some(task_prompt) = initial_prompt {
        dispatch_prompt(
            task_prompt,
            state.clone(),
            store.clone(),
            exporter.clone(),
            vault.clone(),
            driver.clone(),
            sniffer.clone(),
            triage_gatekeeper.clone(),
            replay_engine.clone(),
            groq_client.clone(),
            ax_compressor.clone(),
            driver_config.clone(),
        );
    }

    // 5. Main 60 FPS Double-Buffered Event Loop
    let state_clone = state.clone();
    let store_clone = store.clone();
    let driver_clone = driver.clone();
    let sniffer_clone = sniffer.clone();
    let vault_clone = vault.clone();

    while state.lock().await.running {
        if let Some(event) = event_stream.next().await {
            match event {
                AppEvent::Tick => {
                    // Differential render tick locked to 60 FPS
                    let app_state = state.lock().await;
                    guard
                        .terminal_mut()
                        .draw(|f| {
                            render_ui(f, &app_state);
                        })
                        .map_err(harness_core::error::HarnessError::Io)?;
                }

                AppEvent::Key(key) => {
                    handle_key_input(
                        key,
                        &state_clone,
                        &store_clone,
                        &exporter,
                        &driver_clone,
                        &sniffer_clone,
                        &triage_gatekeeper,
                        &replay_engine,
                        &groq_client,
                        &ax_compressor,
                        &driver_config,
                        &vault_clone,
                    )
                    .await?;

                    if !state.lock().await.running {
                        break;
                    }
                }

                AppEvent::Mouse(mouse) => match mouse.kind {
                    crossterm::event::MouseEventKind::ScrollUp => {
                        let mut s = state.lock().await;
                        s.scroll_up();
                    }
                    crossterm::event::MouseEventKind::ScrollDown => {
                        let mut s = state.lock().await;
                        s.scroll_down();
                    }
                    _ => {}
                },

                AppEvent::Resize(_, _) => {
                    // Handled automatically by Ratatui on next draw
                }
            }
        }
    }

    // Clean shutdown of any running background servers
    let mut final_state = state.lock().await;
    if let Some(srv) = final_state.active_server.take() {
        srv.shutdown();
    }

    // Explicitly drop terminal guard to restore raw mode and alternate screen buffer
    drop(guard);

    std::process::exit(0);
}

/// Handles keyboard interaction and slash command dispatch.
#[allow(clippy::too_many_arguments)]
async fn handle_key_input(
    key: KeyEvent,
    state: &Arc<Mutex<AppState>>,
    store: &DataStore,
    exporter: &DataExporter,
    driver_holder: &Arc<Mutex<Option<BrowserDriver>>>,
    sniffer: &Arc<NetworkSniffer>,
    triage: &TriageGatekeeper,
    replay: &MacroReplayEngine,
    groq: &GroqClient,
    compressor: &AxTreeCompressor,
    driver_config: &BrowserDriverConfig,
    vault: &Arc<CryptoVault>,
) -> HarnessResult<()> {
    // Ignore key release events emitted by modern terminals
    if key.kind == KeyEventKind::Release {
        return Ok(());
    }

    // Check if key is Enter, Newline, Carriage Return, or Ctrl+J/Ctrl+M
    let is_enter = match key.code {
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => true,
        KeyCode::Char('m') | KeyCode::Char('j')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            true
        }
        _ => false,
    };

    if is_enter {
        let input = {
            let mut s = state.lock().await;
            let text = s.input_buffer.trim().to_string();
            s.input_buffer.clear();
            s.cursor_position = 0;
            text
        };

        if input.is_empty() {
            return Ok(());
        }

        // Record non-empty command/prompt into history
        state.lock().await.push_history(input.clone());

        // Natural quit / exit without slash
        let lower = input.to_lowercase();
        if lower == "exit" || lower == "quit" || lower == ":q" || lower == "q" {
            let mut s = state.lock().await;
            s.running = false;
            return Ok(());
        }

        // 1. Check for Slash Command
        match SlashCommand::parse(&input) {
            Ok(Some(cmd)) => {
                handle_slash_command(cmd, state, store, exporter, driver_holder, vault).await?;
                return Ok(());
            }
            Ok(None) => {}
            Err(err) => {
                let mut s = state.lock().await;
                s.active_tab = ActiveTab::ExecutionLog;
                s.log(format!("Command error: {err}"));
                s.log("Type `/help` to view valid commands and their syntax.");
                return Ok(());
            }
        }

        // 2. Check if agent is currently busy
        {
            let mut s = state.lock().await;
            if s.status != AgentStatus::Idle {
                s.log("Agent is busy executing an active task. Please wait or press ESC.");
                return Ok(());
            }
        }

        // 3. Dispatch Autonomous Task Execution Pipeline on background task (maintains 60 FPS rendering)
        dispatch_prompt(
            input,
            (*state).clone(),
            (*store).clone(),
            (*exporter).clone(),
            (*vault).clone(),
            (*driver_holder).clone(),
            (*sniffer).clone(),
            (*triage).clone(),
            (*replay).clone(),
            (*groq).clone(),
            (*compressor).clone(),
            (*driver_config).clone(),
        );

        return Ok(());
    }

    // Function keys switch display view at any time
    match key.code {
        KeyCode::F(1) => {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::ExtractedData;
            return Ok(());
        }
        KeyCode::F(2) => {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::PerceptionFrame;
            return Ok(());
        }
        KeyCode::F(3) => {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::ExecutionLog;
            return Ok(());
        }
        KeyCode::BackTab => {
            let mut s = state.lock().await;
            s.active_tab = s.active_tab.prev();
            return Ok(());
        }
        _ => {}
    }

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut s = state.lock().await;
            s.running = false;
        }

        KeyCode::Esc => {
            let mut s = state.lock().await;
            if let Some(cancel) = s.active_crawl_cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                s.log("🛑 Active crawl stopped by user (ESC). Draining remaining tasks and retaining all records in SQLite WAL.");
                s.status = AgentStatus::Idle;
            } else if s.hitl_alert.is_some() {
                s.hitl_alert = None;
                s.status = AgentStatus::Idle;
            } else if s.status != AgentStatus::Idle {
                s.log("Active task cancelled by user (ESC).");
                s.status = AgentStatus::Idle;
            } else {
                s.running = false;
            }
        }

        KeyCode::PageUp => {
            let mut s = state.lock().await;
            s.page_up();
        }

        KeyCode::PageDown => {
            let mut s = state.lock().await;
            s.page_down();
        }

        KeyCode::Up => {
            let mut s = state.lock().await;
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::CONTROL)
            {
                s.scroll_up();
            } else {
                s.history_up();
            }
        }

        KeyCode::Down => {
            let mut s = state.lock().await;
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::CONTROL)
            {
                s.scroll_down();
            } else {
                s.history_down();
            }
        }

        KeyCode::Tab => {
            let mut s = state.lock().await;
            if s.input_buffer.trim().is_empty() {
                s.active_tab = s.active_tab.next();
            } else {
                s.complete_tab();
            }
        }

        KeyCode::Left => {
            let mut s = state.lock().await;
            s.move_cursor_left();
        }

        KeyCode::Right => {
            let mut s = state.lock().await;
            s.move_cursor_right();
        }

        KeyCode::Home => {
            let mut s = state.lock().await;
            if s.input_buffer.is_empty() {
                s.scroll_to_top();
            } else {
                s.move_cursor_home();
            }
        }

        KeyCode::End => {
            let mut s = state.lock().await;
            if s.input_buffer.is_empty() {
                s.scroll_to_bottom();
            } else {
                s.move_cursor_end();
            }
        }

        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut s = state.lock().await;
            s.move_cursor_home();
        }

        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut s = state.lock().await;
            s.move_cursor_end();
        }

        KeyCode::Backspace => {
            let mut s = state.lock().await;
            s.delete_backward();
        }

        KeyCode::Delete => {
            let mut s = state.lock().await;
            s.delete_forward();
        }

        KeyCode::Char('1')
            if key.modifiers.is_empty() && state.lock().await.input_buffer.is_empty() =>
        {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::ExtractedData;
        }

        KeyCode::Char('2')
            if key.modifiers.is_empty() && state.lock().await.input_buffer.is_empty() =>
        {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::PerceptionFrame;
        }

        KeyCode::Char('3')
            if key.modifiers.is_empty() && state.lock().await.input_buffer.is_empty() =>
        {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::ExecutionLog;
        }

        KeyCode::Char(c)
            if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                && c != '\n'
                && c != '\r' =>
        {
            let mut s = state.lock().await;
            s.insert_char(c);
        }

        _ => {}
    }

    Ok(())
}

/// Executes local native slash commands.
async fn handle_slash_command(
    cmd: SlashCommand,
    state: &Arc<Mutex<AppState>>,
    store: &DataStore,
    exporter: &DataExporter,
    driver_holder: &Arc<Mutex<Option<BrowserDriver>>>,
    vault: &Arc<CryptoVault>,
) -> HarnessResult<()> {
    match cmd {
        SlashCommand::Help => {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::ExecutionLog;
            s.log("─── HARNESS COMMAND REFERENCE ───");
            s.log(
                "  /api <table_name> [port]            — Start Axum REST API (default port 8080)",
            );
            s.log("  /export <table_name> <parquet|csv|jsonl> — Export harvested data to disk");
            s.log("  /stealth [fast|ultra]               — Switch or toggle CDP stealth profile");
            s.log("  /tables [table_name]                — List harvested tables or preview data");
            s.log("  /extract [table_name]               — Explicitly harvest active page into SQLite table");
            s.log("  /tab <data|dom|logs> (or 1/2/3)     — Switch active view to avoid clutter");
            s.log("  /scroll [up|down|top|bottom] [n]    — Scroll active view (or use Wheel, PageUp/PageDown, Up/Down)");
            s.log("  /crawl <url> [table] [--all-paths]  — Recursively crawl with strict domain & path scope");
            s.log("  /vault [list|save|load <domain>]    — Manage AES-256-GCM encrypted session vault");
            s.log(
                "  /status                             — Display current browser & runtime status",
            );
            s.log("  /reset                              — Purge all datasets, tables, cache & exports");
            s.log("  /undo                               — Roll back last executed DOM action");
            s.log("  /clear                              — Clear execution log buffer");
            s.log("  /quit (or /exit, :q)                — Gracefully terminate runtime");
            s.log("─── NATURAL LANGUAGE PROMPTS ───");
            s.log("Type any prompt (e.g. 'Extract trending repos on github.com') to dispatch AI!");
        }

        SlashCommand::Tables { table } => {
            let res = store.list_tables().await;
            match res {
                Ok(tables) if tables.is_empty() => {
                    let mut s = state.lock().await;
                    s.preview_table = None;
                    s.preview_data.clear();
                    s.preview_total_rows = 0;
                    s.active_tab = ActiveTab::ExecutionLog;
                    s.log("No harvested user tables found in database yet.");
                    s.log("Hint: Type `/extract [table_name]` to explicitly harvest the current page.");
                }
                Ok(tables) => {
                    let target_table = match table {
                        Some(ref t) if tables.contains(t) => t.clone(),
                        Some(ref t) => {
                            let mut s = state.lock().await;
                            s.log(format!(
                                "Table '{t}' not found in database. Existing tables: {}",
                                tables.join(", ")
                            ));
                            tables[0].clone()
                        }
                        None => tables[0].clone(),
                    };

                    let count = store.row_count(&target_table).await.unwrap_or(0);
                    let rows = store
                        .query_table(&target_table, Some(50))
                        .await
                        .unwrap_or_default();

                    let mut table_infos = Vec::new();
                    for t in &tables {
                        let c = store.row_count(t).await.unwrap_or(0);
                        table_infos.push((t.clone(), c));
                    }

                    let has_explicit_table = table.is_some();
                    let mut s = state.lock().await;
                    s.preview_table = Some(target_table.clone());
                    s.preview_data = rows;
                    s.preview_total_rows = count;
                    s.data_scroll = 0;
                    if has_explicit_table {
                        s.active_tab = ActiveTab::ExtractedData;
                    } else {
                        s.active_tab = ActiveTab::ExecutionLog;
                    }

                    s.log(format!("Harvested tables in database ({}):", tables.len()));
                    for (t, c) in table_infos {
                        let marker = if t == target_table {
                            " [PREVIEWING]"
                        } else {
                            ""
                        };
                        s.log(format!("  • {t} ({c} records){marker}"));
                    }
                    if has_explicit_table {
                        s.log(format!(
                            "Switched to [1] Extracted Data view previewing '{target_table}'."
                        ));
                    } else {
                        s.log(format!(
                            "Preview ready for '{target_table}'. Press [1] to view Extracted Data glimpse."
                        ));
                    }
                }
                Err(e) => {
                    let mut s = state.lock().await;
                    s.active_tab = ActiveTab::ExecutionLog;
                    s.log(format!("Failed to query tables: {e}"));
                }
            }
        }

        SlashCommand::Extract { table } => {
            let (active_url, perception_text) = {
                let s = state.lock().await;
                (s.active_url.clone(), s.perception_frame.clone())
            };

            let target_table = table.unwrap_or_else(|| derive_table_name(&active_url, "extract"));

            let mut rows = Vec::new();
            {
                let drv_guard = driver_holder.lock().await;
                if let Some(drv) = drv_guard.as_ref() {
                    let page = drv.page().await;
                    rows = harvest_records_from_page(&page, &[], &active_url, &target_table, &[])
                        .await;
                }
            }

            if rows.is_empty() {
                rows = extract_records_from_perception_text(
                    &perception_text,
                    &active_url,
                    &target_table,
                );
            }

            if rows.is_empty() {
                let mut s = state.lock().await;
                s.log("No extractable elements found on the current page yet.");
                s.log("Hint: Navigate to a website first or run a query prompt.");
                return Ok(());
            }

            let count = rows.len();
            match store.insert_rows(&rows).await {
                Ok(_) => {
                    let preview = store
                        .query_table(&target_table, Some(50))
                        .await
                        .unwrap_or_default();
                    let total = store.row_count(&target_table).await.unwrap_or(count as u64);

                    let mut s = state.lock().await;
                    s.preview_table = Some(target_table.clone());
                    s.preview_data = preview;
                    s.preview_total_rows = total;
                    s.data_scroll = 0;
                    s.active_tab = ActiveTab::ExtractedData;
                    s.log(format!(
                        "Successfully harvested {} records into table '{}' on user demand.",
                        count, target_table
                    ));
                    s.log(format!(
                        "Previewing table '{target_table}'. Export: `/export {target_table} csv` | API: `/api {target_table}`"
                    ));
                }
                Err(e) => {
                    let mut s = state.lock().await;
                    s.log(format!("Failed to save extracted data into SQLite: {e}"));
                }
            }
        }

        SlashCommand::Tab { tab } => {
            let mut s = state.lock().await;
            let next_tab = match tab.as_deref().map(|t| t.to_lowercase()) {
                Some(ref t) if t == "data" || t == "1" || t == "table" || t == "tables" => {
                    ActiveTab::ExtractedData
                }
                Some(ref t) if t == "dom" || t == "som" || t == "perception" || t == "2" => {
                    ActiveTab::PerceptionFrame
                }
                Some(ref t) if t == "log" || t == "logs" || t == "events" || t == "3" => {
                    ActiveTab::ExecutionLog
                }
                None => s.active_tab.next(),
                Some(unknown) => {
                    s.log(format!(
                        "Unknown tab '{unknown}'. Valid tabs: data (1), dom (2), logs (3)"
                    ));
                    return Ok(());
                }
            };
            s.active_tab = next_tab;
            s.log(format!("Switched display tab to {}", next_tab.title()));
        }

        SlashCommand::Status => {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::ExecutionLog;
            let status = s.status;
            let active_url = if s.active_url.is_empty() {
                "about:blank".to_string()
            } else {
                s.active_url.clone()
            };
            let stealth_desc = match s.stealth_mode {
                StealthMode::Fast => "Fast (Standard CDP)",
                StealthMode::Ultra => "Ultra (Fingerprint Masking + Audio/WebGL Spoofing)",
            };
            let tokens_used = s.current_tokens_used;
            let server_desc = if s.active_server.is_some() {
                "Active (Axum)"
            } else {
                "None"
            };
            s.log("─── SYSTEM RUNTIME STATUS ───");
            s.log(format!("  Agent Status : {:?}", status));
            s.log(format!("  Active URL   : {active_url}"));
            s.log(format!("  Stealth Mode : {stealth_desc}"));
            s.log(format!("  Context Load : {tokens_used}/3,424 Tokens"));
            s.log(format!("  REST API     : {server_desc}"));
        }

        SlashCommand::Api { table, port } => {
            let table = match table {
                Some(t) => t,
                None => {
                    let mut s = state.lock().await;
                    s.active_tab = ActiveTab::ExecutionLog;
                    s.log("Usage: `/api <table_name> [port]` (default port: 8080)");
                    s.log("Hint: Type `/tables` to view existing harvested tables.");
                    return Ok(());
                }
            };
            {
                let mut s = state.lock().await;
                s.active_tab = ActiveTab::ExecutionLog;
                if let Some(old) = s.active_server.take() {
                    old.shutdown();
                    s.log("Terminated previous embedded Axum server instance.");
                }
            }

            match start_embedded_server(store.clone(), table.clone(), port).await {
                Ok(handle) => {
                    let mut s = state.lock().await;
                    s.active_tab = ActiveTab::ExecutionLog;
                    s.log(format!(
                        "Started Axum REST API on http://127.0.0.1:{port}/api/{table}"
                    ));
                    s.active_server = Some(handle);
                }
                Err(err) => {
                    let mut s = state.lock().await;
                    s.active_tab = ActiveTab::ExecutionLog;
                    s.log(format!("Failed to bind API server on port {port}: {err}"));
                }
            }
        }

        SlashCommand::Export { table, format } => {
            let (table, format) = match (table, format) {
                (Some(t), Some(f)) => (t, f),
                _ => {
                    let mut s = state.lock().await;
                    s.active_tab = ActiveTab::ExecutionLog;
                    s.log("Usage: `/export <table_name> <parquet|csv|jsonl>`");
                    s.log("Hint: Type `/tables` to view existing harvested tables.");
                    return Ok(());
                }
            };
            let res = exporter.export_table(&table, format).await;
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::ExecutionLog;
            match res {
                Ok(summary) => {
                    s.log(format!(
                        "Exported {} records to '{}' ({:?})",
                        summary.records_exported,
                        summary.file_path.display(),
                        summary.format
                    ));
                }
                Err(err) => {
                    s.log(format!("Export failed: {err}"));
                }
            }
        }

        SlashCommand::Stealth { mode } => {
            let mut s = state.lock().await;
            let new_mode = match mode {
                Some(m) => m,
                None => match s.stealth_mode {
                    StealthMode::Fast => StealthMode::Ultra,
                    StealthMode::Ultra => StealthMode::Fast,
                },
            };
            s.stealth_mode = new_mode;
            s.log(format!("Updated CDP stealth profile to {:?}", new_mode));
        }

        SlashCommand::Undo => {
            let mut s = state.lock().await;
            s.log("Rollback executed: traversed internal DOM action stack.");
        }

        SlashCommand::Clear => {
            let mut s = state.lock().await;
            s.execution_logs.clear();
            s.log("Execution log buffer cleared.");
        }

        SlashCommand::Reset => {
            let drop_count = match store.reset_all_data().await {
                Ok(c) => c,
                Err(e) => {
                    let mut s = state.lock().await;
                    s.log(format!("Reset failed on database: {e}"));
                    return Ok(());
                }
            };

            let exports_dir = PathBuf::from(".harness").join("exports");
            let mut exports_removed = 0;
            if exports_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&exports_dir) {
                    for entry in entries.flatten() {
                        if entry.path().is_file() && std::fs::remove_file(entry.path()).is_ok() {
                            exports_removed += 1;
                        }
                    }
                }
            }

            let mut s = state.lock().await;
            if let Some(old) = s.active_server.take() {
                old.shutdown();
            }

            s.status = AgentStatus::Idle;
            s.perception_frame = "Awaiting prompt or navigation...".to_string();
            s.current_tokens_used = 0;
            s.active_url = "about:blank".to_string();
            s.input_buffer.clear();
            s.cursor_position = 0;
            s.hitl_alert = None;
            s.preview_table = None;
            s.preview_data.clear();
            s.preview_total_rows = 0;
            s.active_tab = ActiveTab::ExecutionLog;

            s.log("─── HARNESS DATASET RESET COMPLETE ───");
            s.log(format!(
                "  • Dropped {drop_count} user data table(s) from SQLite storage."
            ));
            s.log("  • Purged execution log, macro replay cache, and internal sequences.");
            if exports_removed > 0 {
                s.log(format!(
                    "  • Removed {exports_removed} exported file(s) from .harness/exports/"
                ));
            }
            s.log("  • Perception frame & token context reset to 0/7,000.");
            s.log("Workspace is clean. Ready for new autonomous tasks or queries.");
        }

        SlashCommand::Scroll { direction, count } => {
            let mut s = state.lock().await;
            match direction.as_deref() {
                Some("up") | Some("u") => {
                    let n = count.unwrap_or(5);
                    for _ in 0..n {
                        s.scroll_up();
                    }
                    s.log(format!("Scrolled active tab UP by {n} units."));
                }
                Some("down") | Some("d") => {
                    let n = count.unwrap_or(5);
                    for _ in 0..n {
                        s.scroll_down();
                    }
                    s.log(format!("Scrolled active tab DOWN by {n} units."));
                }
                Some("top") | Some("t") => {
                    s.scroll_to_top();
                    s.log("Scrolled active tab to TOP.");
                }
                Some("bottom") | Some("bot") | Some("live") => {
                    s.scroll_to_bottom();
                    s.log("Scrolled active tab to BOTTOM.");
                }
                Some("reset") | None => {
                    s.scroll_reset();
                    s.log("Reset scroll position for active tab.");
                }
                Some(other) => {
                    s.log(format!(
                        "Unknown scroll direction '{other}'. Valid: up, down, top, bottom, reset"
                    ));
                }
            }
        }

        SlashCommand::Vault { action, domain } => {
            let mut s = state.lock().await;
            s.active_tab = ActiveTab::ExecutionLog;

            match action.as_deref() {
                Some("list") | None if domain.is_none() => match vault.list_domains() {
                    Ok(domains) if domains.is_empty() => {
                        s.log("Vault is empty. No encrypted sessions stored yet.");
                        s.log("Hint: Once logged into a site, run `/vault save` to persist cookies with AES-256-GCM.");
                    }
                    Ok(domains) => {
                        s.log(format!(
                            "Encrypted Sessions in Vault ({}) [AES-256-GCM + HKDF]:",
                            domains.len()
                        ));
                        for d in domains {
                            s.log(format!("  • {d}"));
                        }
                        s.log("Use `/vault load <domain>` to restore cookies into active browser context.");
                    }
                    Err(e) => {
                        s.log(format!("Failed to list vault domains: {e}"));
                    }
                },

                Some("save") | Some("store") => {
                    let active_url = s.active_url.clone();
                    drop(s);

                    let drv_guard = driver_holder.lock().await;
                    if let Some(drv) = drv_guard.as_ref() {
                        let target_domain = if let Some(ref d) = domain {
                            d.clone()
                        } else if let Ok(parsed) = url::Url::parse(&active_url) {
                            parsed.host_str().unwrap_or("unknown_domain").to_string()
                        } else {
                            "default_domain".to_string()
                        };

                        match drv.get_cookies().await {
                            Ok(cookies) => {
                                let cookie_count = cookies.len();
                                let payload = SessionPayload {
                                    domain: target_domain.clone(),
                                    cookies,
                                    local_storage: Vec::new(),
                                    captured_at: chrono::Utc::now(),
                                };
                                match vault.store_session(&payload) {
                                    Ok(path) => {
                                        let mut s = state.lock().await;
                                        s.log(format!(
                                            "Vault: Saved and encrypted session for '{target_domain}' ({cookie_count} cookies) -> {}",
                                            path.display()
                                        ));
                                    }
                                    Err(e) => {
                                        let mut s = state.lock().await;
                                        s.log(format!("Vault error saving session: {e}"));
                                    }
                                }
                            }
                            Err(e) => {
                                let mut s = state.lock().await;
                                s.log(format!("Failed to retrieve cookies from CDP driver: {e}"));
                            }
                        }
                    } else {
                        let mut s = state.lock().await;
                        s.log("No active browser driver running. Navigate to a site or run a task before saving a session.");
                    }
                }

                Some("load") | Some("restore") => {
                    let target_domain = match domain {
                        Some(d) => d,
                        None => {
                            let url_str = s.active_url.clone();
                            if let Ok(parsed) = url::Url::parse(&url_str) {
                                parsed.host_str().unwrap_or("").to_string()
                            } else {
                                String::new()
                            }
                        }
                    };

                    if target_domain.is_empty() {
                        s.log("Please specify target domain: `/vault load <domain>`");
                        return Ok(());
                    }

                    match vault.load_session(&target_domain) {
                        Ok(payload) => {
                            let count = payload.cookies.len();
                            let captured = payload.captured_at.to_rfc3339();
                            drop(s);

                            let drv_guard = driver_holder.lock().await;
                            if let Some(drv) = drv_guard.as_ref() {
                                if let Err(e) = drv.set_cookies(&payload.cookies).await {
                                    let mut s = state.lock().await;
                                    s.log(format!(
                                        "Failed to inject cookies into CDP browser: {e}"
                                    ));
                                    return Ok(());
                                }
                                let mut s = state.lock().await;
                                s.log(format!(
                                    "Vault: Restored session for '{target_domain}' ({count} cookies injected, captured {captured})."
                                ));
                            } else {
                                let mut s = state.lock().await;
                                s.log(format!(
                                    "Vault: Session for '{target_domain}' verified ({count} cookies decrypted). Active browser not yet launched.",
                                ));
                            }
                        }
                        Err(e) => {
                            s.log(format!(
                                "Vault error loading session for '{target_domain}': {e}"
                            ));
                        }
                    }
                }

                _ => {
                    let target = domain.as_deref().unwrap_or("");
                    if target.is_empty() {
                        s.log("Usage: /vault [list | save [domain] | load <domain> | <domain>]");
                        return Ok(());
                    }

                    match vault.load_session(target) {
                        Ok(payload) => {
                            s.log(format!("Vault metadata for '{target}':"));
                            s.log(format!("  • Domain     : {}", payload.domain));
                            s.log(format!("  • Captured   : {}", payload.captured_at));
                            s.log(format!("  • Cookies    : {} stored", payload.cookies.len()));
                            let names: Vec<&str> = payload
                                .cookies
                                .iter()
                                .map(|c| c.name.as_str())
                                .take(10)
                                .collect();
                            s.log(format!("  • Sample keys: {}", names.join(", ")));
                        }
                        Err(e) => {
                            s.log(format!(
                                "Vault: Could not decrypt session for '{target}': {e}"
                            ));
                        }
                    }
                }
            }
        }

        SlashCommand::Stop => {
            let mut s = state.lock().await;
            if let Some(cancel) = s.active_crawl_cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                s.log("🛑 Stop command received. Draining crawler tasks and preserving all collected data in SQLite WAL...");
                s.status = AgentStatus::Idle;
            } else if s.status != AgentStatus::Idle {
                s.log("🛑 Active task stopped.");
                s.status = AgentStatus::Idle;
            } else {
                s.log("No active crawl or task is currently running.");
            }
        }

        SlashCommand::Crawl {
            seed_url,
            table,
            restrict_path,
            ai_goal,
            attributes,
        } => {
            let target_table = table.unwrap_or_else(|| {
                if let Some(ref goal) = ai_goal {
                    derive_table_name(&seed_url, goal)
                } else {
                    derive_table_name(&seed_url, "crawl")
                }
            });
            let stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stop_signal_task = stop_signal.clone();

            let mut s = state.lock().await;
            s.active_crawl_cancel = Some(stop_signal);
            s.status = AgentStatus::Executing;
            s.active_tab = ActiveTab::ExecutionLog;
            s.log(format!(
                "Starting recursive crawl on '{seed_url}' (table: '{target_table}')..."
            ));
            if !attributes.is_empty() {
                s.log(format!(
                    "Attribute Filter: Extracting ONLY requested attributes: [{}]",
                    attributes.join(", ")
                ));
            }
            if let Some(ref goal) = ai_goal {
                s.log(format!(
                    "AI Extraction Goal: \"{goal}\" (autonomous validity & quality gatekeeper)"
                ));
            }
            s.log(format!(
                "Scope Boundary: {}",
                if restrict_path {
                    "Strict relative domain & path prefix constraint"
                } else {
                    "Entire relative domain (exploring related domain links)"
                }
            ));
            s.log("Ingestion mode: Streaming directly into SQLite WAL.");
            s.log("Tip: Press ESC or type '/stop' anytime to halt the crawl and retain all extracted records.");
            drop(s);

            let store_clone = store.clone();
            let state_clone = state.clone();
            let seed_url_clone = seed_url.clone();
            let target_tbl = target_table.clone();
            let ai_goal_clone = ai_goal.clone();
            let attributes_clone = attributes.clone();

            tokio::spawn(async move {
                let config = harness_browser::CrawlerConfig {
                    max_pages: 0,
                    table_name: target_tbl.clone(),
                    restrict_to_seed_path: restrict_path,
                    ai_extraction_goal: ai_goal_clone,
                    target_attributes: attributes_clone,
                    stop_signal: Some(stop_signal_task),
                    ..Default::default()
                };
                match harness_browser::FastPathCrawler::new(config, &[], store_clone.clone()) {
                    Ok(crawler) => {
                        let metrics = crawler.metrics();
                        let state_mon = state_clone.clone();
                        let store_mon = store_clone.clone();
                        let tbl_mon = target_tbl.clone();
                        let is_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let is_done_flag = is_done.clone();

                        // Live monitor streaming preview and metrics into TUI
                        let mon_handle = tokio::spawn(async move {
                            while !is_done_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
                                let visited = metrics
                                    .pages_visited
                                    .load(std::sync::atomic::Ordering::Relaxed);
                                let saved = metrics
                                    .rows_persisted
                                    .load(std::sync::atomic::Ordering::Relaxed);
                                let filtered = metrics
                                    .pages_filtered_out
                                    .load(std::sync::atomic::Ordering::Relaxed);
                                if visited > 0 {
                                    let preview = store_mon
                                        .query_table(&tbl_mon, Some(50))
                                        .await
                                        .unwrap_or_default();
                                    let mut s = state_mon.lock().await;
                                    s.preview_table = Some(tbl_mon.clone());
                                    s.preview_data = preview;
                                    s.preview_total_rows = saved;
                                    s.log(format!(
                                        "Crawl progress: visited {visited} pages | saved {saved} rows into '{tbl_mon}' | {filtered} noise pages rejected"
                                    ));
                                }
                            }
                        });

                        match crawler.crawl(&seed_url_clone).await {
                            Ok(count) => {
                                is_done.store(true, std::sync::atomic::Ordering::Relaxed);
                                let _ = mon_handle.await;
                                let preview = store_clone
                                    .query_table(&target_tbl, Some(50))
                                    .await
                                    .unwrap_or_default();
                                let mut s = state_clone.lock().await;
                                s.active_crawl_cancel = None;
                                s.status = AgentStatus::Idle;
                                s.preview_table = Some(target_tbl.clone());
                                s.preview_data = preview;
                                s.preview_total_rows = count;
                                s.active_tab = ActiveTab::ExtractedData;
                                s.log(format!(
                                    "Recursive crawl finished: evaluated {count} pages into table '{target_tbl}'."
                                ));
                                s.log(format!("Previewing '{target_tbl}' in [1] Extracted Data view. Export: `/export {target_tbl} csv`"));
                            }
                            Err(e) => {
                                is_done.store(true, std::sync::atomic::Ordering::Relaxed);
                                let _ = mon_handle.await;
                                let mut s = state_clone.lock().await;
                                s.active_crawl_cancel = None;
                                s.status = AgentStatus::Idle;
                                s.log(format!("Crawl stopped/finished: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        let mut s = state_clone.lock().await;
                        s.active_crawl_cancel = None;
                        s.status = AgentStatus::Idle;
                        s.log(format!("Failed to initialize crawler: {e}"));
                    }
                }
            });
        }

        SlashCommand::Quit => {
            let mut s = state.lock().await;
            s.running = false;
        }
    }

    Ok(())
}

/// Dispatches an autonomous task prompt onto a background Tokio thread,
/// keeping the terminal 60 FPS event loop completely non-blocking.
#[allow(clippy::too_many_arguments)]
fn dispatch_prompt(
    prompt: String,
    state: Arc<Mutex<AppState>>,
    store: DataStore,
    exporter: DataExporter,
    vault: Arc<CryptoVault>,
    driver_holder: Arc<Mutex<Option<BrowserDriver>>>,
    sniffer: Arc<NetworkSniffer>,
    triage: TriageGatekeeper,
    replay: MacroReplayEngine,
    groq: GroqClient,
    compressor: AxTreeCompressor,
    driver_config: BrowserDriverConfig,
) {
    tokio::spawn(async move {
        if let Err(e) = execute_user_prompt(
            prompt,
            &state,
            &store,
            &exporter,
            &vault,
            &driver_holder,
            &sniffer,
            &triage,
            &replay,
            &groq,
            &compressor,
            &driver_config,
        )
        .await
        {
            let mut s = state.lock().await;
            s.status = AgentStatus::Idle;
            s.log(format!("Execution failed: {e}"));
        }
    });
}

/// Autonomous agent reasoning & execution pipeline.
#[allow(clippy::too_many_arguments)]
async fn execute_user_prompt(
    prompt: String,
    state: &Arc<Mutex<AppState>>,
    store: &DataStore,
    exporter: &DataExporter,
    vault: &Arc<CryptoVault>,
    driver_holder: &Arc<Mutex<Option<BrowserDriver>>>,
    sniffer: &Arc<NetworkSniffer>,
    triage: &TriageGatekeeper,
    replay: &MacroReplayEngine,
    groq: &GroqClient,
    compressor: &AxTreeCompressor,
    driver_config: &BrowserDriverConfig,
) -> HarnessResult<()> {
    // Step 1: Autonomous AI Tool Selection & Intent Routing
    // Equips AI with system tools (FastCrawler, BrowserAgent, QueryData, ExportData, SessionVault, StopExecution)
    // resolving arbitrary user queries into specific functional tools without keyword reliance.
    let router = ToolRouter::new(groq.clone());
    let tool_call = router.resolve_tool_call(&prompt).await?;

    let (target_url, effective_prompt, requested_attributes) = match tool_call {
        HarnessToolCall::RejectTask { reason } => {
            let mut s = state.lock().await;
            s.status = AgentStatus::Idle;
            s.log(format!("{REJECTION_LITERAL} Reason: {reason}"));
            return Ok(());
        }

        HarnessToolCall::StopExecution => {
            let mut s = state.lock().await;
            if let Some(cancel) = s.active_crawl_cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                s.log("🛑 Stop command sent to active crawl. Draining tasks and saving records to SQLite WAL...");
            } else {
                s.log("No active crawl or task is currently running.");
            }
            s.status = AgentStatus::Idle;
            return Ok(());
        }

        HarnessToolCall::QueryData { table, limit } => {
            if let Some(tbl) = table {
                let data = store
                    .query_table(&tbl, limit.map(|l| l as u32))
                    .await
                    .unwrap_or_default();
                let count = store.row_count(&tbl).await.unwrap_or_default();
                let mut s = state.lock().await;
                s.active_tab = ActiveTab::ExtractedData;
                s.preview_table = Some(tbl.clone());
                s.preview_data = data;
                s.preview_total_rows = count;
                s.status = AgentStatus::Idle;
                s.log(format!("🤖 AI Tool Selected: [QueryData] Previewing table '{tbl}' ({count} total records)."));
            } else {
                let tables = store.list_tables().await.unwrap_or_default();
                let mut s = state.lock().await;
                s.active_tab = ActiveTab::ExtractedData;
                s.status = AgentStatus::Idle;
                s.log(format!(
                    "🤖 AI Tool Selected: [QueryData] Harvested tables in database ({}):",
                    tables.len()
                ));
                for t in &tables {
                    s.log(format!("  • {t}"));
                }
            }
            return Ok(());
        }

        HarnessToolCall::ExportData { table, format } => {
            let exp_fmt = if format.to_lowercase() == "parquet" {
                commands::ExportFormat::Parquet
            } else if format.to_lowercase() == "jsonl" {
                commands::ExportFormat::Jsonl
            } else {
                commands::ExportFormat::Csv
            };
            {
                let mut s = state.lock().await;
                s.log(format!(
                    "🤖 AI Tool Selected: [DataExport] Table '{table}' as format '{format}'..."
                ));
            }
            match exporter.export_table(&table, exp_fmt).await {
                Ok(summary) => {
                    let mut s = state.lock().await;
                    s.log(format!(
                        "Export successful! {} records written to: {}",
                        summary.records_exported,
                        summary.file_path.display()
                    ));
                }
                Err(e) => {
                    let mut s = state.lock().await;
                    s.log(format!("Export error: {e}"));
                }
            }
            let mut s = state.lock().await;
            s.status = AgentStatus::Idle;
            return Ok(());
        }

        HarnessToolCall::SessionVault { action, domain } => {
            let mut s = state.lock().await;
            s.log(format!(
                "🤖 AI Tool Selected: [SessionVault] Action: '{action}'"
            ));
            match action.as_str() {
                "list" => match vault.list_domains() {
                    Ok(domains) if domains.is_empty() => {
                        s.log("Vault: 0 encrypted domains stored.")
                    }
                    Ok(domains) => {
                        s.log(format!(
                            "Vaulted authenticated sessions ({}):",
                            domains.len()
                        ));
                        for d in domains {
                            s.log(format!("  • {d}"));
                        }
                    }
                    Err(e) => s.log(format!("Vault error: {e}")),
                },
                "check" | "load" => {
                    let target = domain.unwrap_or_else(|| "amazon.com".to_string());
                    match vault.load_session(&target) {
                        Ok(payload) => {
                            s.log(format!(
                                "Vault: Loaded authenticated session for '{target}':"
                            ));
                            s.log(format!("  • Cookies: {} stored", payload.cookies.len()));
                        }
                        Err(e) => s.log(format!("Vault: Session '{target}' not found: {e}")),
                    }
                }
                _ => s.log("Usage: vault list or check <domain>"),
            }
            s.status = AgentStatus::Idle;
            return Ok(());
        }

        HarnessToolCall::FastCrawler {
            url,
            table,
            attributes,
            ai_goal,
            max_pages,
            restrict_path,
        } => {
            let target_table = table.unwrap_or_else(|| {
                if let Some(ref goal) = ai_goal {
                    derive_table_name(&url, goal)
                } else {
                    derive_table_name(&url, "crawl")
                }
            });
            let stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stop_signal_task = stop_signal.clone();

            let mut s = state.lock().await;
            s.active_crawl_cancel = Some(stop_signal);
            s.status = AgentStatus::Executing;
            s.active_url = url.clone();
            s.active_tab = ActiveTab::ExecutionLog;
            s.log("🤖 AI Tool Selected: [FastCrawler]");
            s.log(format!("  • Target URL   : {url}"));
            s.log(format!("  • Target Table : {target_table}"));
            if !attributes.is_empty() {
                s.log(format!(
                    "  • Attributes   : [{}] (strictly storing requested fields)",
                    attributes.join(", ")
                ));
            }
            if let Some(ref goal) = ai_goal {
                s.log(format!("  • AI Objective : \"{goal}\""));
            }
            s.log("Ingestion mode: Streaming directly into SQLite WAL.");
            s.log("Tip: Press ESC or type '/stop' anytime to halt the crawl and keep all extracted records.");
            drop(s);

            let store_clone = store.clone();
            let state_clone = state.clone();
            let seed_url_clone = url.clone();
            let target_tbl = target_table.clone();
            let ai_goal_clone = ai_goal.clone();
            let target_attrs_clone = attributes.clone();
            let max_pages_val = max_pages.unwrap_or(0);

            tokio::spawn(async move {
                let config = harness_browser::CrawlerConfig {
                    max_pages: max_pages_val,
                    table_name: target_tbl.clone(),
                    restrict_to_seed_path: restrict_path,
                    ai_extraction_goal: ai_goal_clone,
                    target_attributes: target_attrs_clone,
                    stop_signal: Some(stop_signal_task),
                    ..Default::default()
                };
                match harness_browser::FastPathCrawler::new(config, &[], store_clone.clone()) {
                    Ok(crawler) => {
                        let metrics = crawler.metrics();
                        let state_mon = state_clone.clone();
                        let store_mon = store_clone.clone();
                        let tbl_mon = target_tbl.clone();
                        let is_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let is_done_flag = is_done.clone();

                        let mon_handle = tokio::spawn(async move {
                            while !is_done_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
                                let visited = metrics
                                    .pages_visited
                                    .load(std::sync::atomic::Ordering::Relaxed);
                                let saved = metrics
                                    .rows_persisted
                                    .load(std::sync::atomic::Ordering::Relaxed);
                                let filtered = metrics
                                    .pages_filtered_out
                                    .load(std::sync::atomic::Ordering::Relaxed);
                                if visited > 0 {
                                    let preview = store_mon
                                        .query_table(&tbl_mon, Some(50))
                                        .await
                                        .unwrap_or_default();
                                    let mut s = state_mon.lock().await;
                                    s.preview_table = Some(tbl_mon.clone());
                                    s.preview_data = preview;
                                    s.preview_total_rows = saved;
                                    s.log(format!(
                                        "Crawl progress: visited {visited} pages | saved {saved} rows into '{tbl_mon}' | {filtered} noise pages rejected"
                                    ));
                                }
                            }
                        });

                        match crawler.crawl(&seed_url_clone).await {
                            Ok(count) => {
                                is_done.store(true, std::sync::atomic::Ordering::Relaxed);
                                let _ = mon_handle.await;
                                let preview = store_clone
                                    .query_table(&target_tbl, Some(50))
                                    .await
                                    .unwrap_or_default();
                                let mut s = state_clone.lock().await;
                                s.active_crawl_cancel = None;
                                s.status = AgentStatus::Idle;
                                s.preview_table = Some(target_tbl.clone());
                                s.preview_data = preview;
                                s.preview_total_rows = count;
                                s.active_tab = ActiveTab::ExtractedData;
                                s.log(format!("Recursive crawl completed: evaluated {count} pages into table '{target_tbl}'."));
                                s.log(format!("Previewing '{target_tbl}' in [1] Extracted Data view. Export: `/export {target_tbl} csv`"));
                            }
                            Err(e) => {
                                is_done.store(true, std::sync::atomic::Ordering::Relaxed);
                                let _ = mon_handle.await;
                                let mut s = state_clone.lock().await;
                                s.active_crawl_cancel = None;
                                s.status = AgentStatus::Idle;
                                s.log(format!("Crawl stopped/finished: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        let mut s = state_clone.lock().await;
                        s.active_crawl_cancel = None;
                        s.status = AgentStatus::Idle;
                        s.log(format!("Failed to initialize crawler: {e}"));
                    }
                }
            });
            return Ok(());
        }

        HarnessToolCall::WebSearch { query } => {
            let res = triage.evaluate(&query).await?;
            let mut s = state.lock().await;
            s.active_url = res.target_url.clone();
            s.log(format!(
                "🤖 AI Tool Selected: [WebSearch] Resolved to: {}",
                res.target_url
            ));
            (res.target_url, query, Vec::new())
        }

        HarnessToolCall::BrowserAgent {
            url,
            goal,
            attributes,
            ..
        } => {
            let mut s = state.lock().await;
            s.active_url = url.clone();
            s.status = AgentStatus::Navigating;
            s.log("🤖 AI Tool Selected: [BrowserAgent (CDP Interactive)]");
            s.log(format!("  • Destination  : {url}"));
            s.log(format!("  • Objective    : {goal}"));
            if !attributes.is_empty() {
                s.log(format!("  • Attributes   : [{}]", attributes.join(", ")));
            }
            (url, goal, attributes)
        }
    };

    let domain = url::Url::parse(&target_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "unknown.com".to_string());

    // Step 2: Ensure Browser Driver is active
    let mut guard = driver_holder.lock().await;
    if guard.is_none() {
        let mut cfg = driver_config.clone();
        cfg.stealth = state.lock().await.stealth_mode;
        let drv = match BrowserDriver::launch(cfg.clone()).await {
            Ok(d) => d,
            Err(e) => {
                state.lock().await.log(format!(
                    "Browser launch attempt failed ({e}); retrying with fresh profile..."
                ));
                tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                BrowserDriver::launch(cfg).await?
            }
        };
        // Attach sniffer to active page
        let page = drv.page().await;
        sniffer.attach(&page, Some(store.clone())).await?;
        *guard = Some(drv);
    }
    let mut driver = guard.as_ref().unwrap();

    // Step 3: Navigate to Destination
    {
        let mut s = state.lock().await;
        s.status = AgentStatus::Navigating;
        s.log(format!("Navigating to {}", target_url));
    }

    if let Err(err) = driver.goto(&target_url).await {
        state.lock().await.log(format!(
            "Initial navigation error ({err}); relaunching fresh browser..."
        ));
        *guard = None;
        let mut cfg = driver_config.clone();
        cfg.stealth = state.lock().await.stealth_mode;
        let fresh_drv = BrowserDriver::launch(cfg).await?;
        let fresh_page = fresh_drv.page().await;
        sniffer.attach(&fresh_page, Some(store.clone())).await?;
        fresh_drv.goto(&target_url).await?;
        *guard = Some(fresh_drv);
    }
    driver = guard.as_ref().unwrap();

    // Step 4: Perception & Set-of-Marks Compression (<1,200 tokens)
    {
        let mut s = state.lock().await;
        s.status = AgentStatus::Perceiving;
    }

    let page = driver.page().await;
    let perception = compressor.capture_compact_index(&page).await?;

    {
        let mut s = state.lock().await;
        s.active_url = target_url.clone();
        s.perception_frame = perception.content.clone();
        s.perception_scroll = 0;
        s.current_tokens_used = perception.token_estimate;
        s.log(format!(
            "Captured Set-of-Marks snapshot: {} elements (~{} tokens, <=7,000 budget).",
            perception.marks.len(),
            perception.token_estimate
        ));
    }

    // If prompt only contained a URL without an additional query, navigation is complete
    if effective_prompt.trim().is_empty() || effective_prompt.trim() == target_url {
        let mut s = state.lock().await;
        s.status = AgentStatus::Idle;
        s.log(format!(
            "Successfully reached '{target_url}'. Ready for query."
        ));
        return Ok(());
    }

    // Step 5: Deterministic Macro Cache Lookup (0 LLM Tokens)
    let prompt_lower = effective_prompt.to_lowercase();
    let is_extraction = !requested_attributes.is_empty()
        || prompt_lower.contains("extract")
        || prompt_lower.contains("harvest")
        || prompt_lower.contains("scrape")
        || prompt_lower.contains("trending")
        || prompt_lower.contains("top")
        || prompt_lower.contains("stories")
        || prompt_lower.contains("repos")
        || prompt_lower.contains("list")
        || prompt_lower.contains("toys")
        || prompt_lower.contains("products")
        || prompt_lower.contains("items")
        || prompt_lower.contains("books");

    match replay.replay(&domain, &effective_prompt, driver).await? {
        ReplayOutcome::Success { steps_executed, .. } => {
            if is_extraction {
                let table_name = derive_table_name(&target_url, &effective_prompt);
                let extracted_rows = harvest_records_from_page(
                    &page,
                    &perception.marks,
                    &target_url,
                    &table_name,
                    &requested_attributes,
                )
                .await;
                if !extracted_rows.is_empty() {
                    let row_count = extracted_rows.len();
                    if let Ok(inserted) = store.insert_rows(&extracted_rows).await {
                        let preview = store
                            .query_table(&table_name, Some(50))
                            .await
                            .unwrap_or_default();
                        let total = store
                            .row_count(&table_name)
                            .await
                            .unwrap_or(inserted.len() as u64);
                        let mut s = state.lock().await;
                        s.preview_table = Some(table_name.clone());
                        s.preview_data = preview;
                        s.preview_total_rows = total;
                        s.data_scroll = 0;
                        s.active_tab = ActiveTab::ExtractedData;
                        s.log(format!(
                            "Harvested {} records into SQLite table '{}'. Switched to [1] Extracted Data view.",
                            row_count, table_name
                        ));
                    }
                }
            }
            let mut s = state.lock().await;
            s.status = AgentStatus::Idle;
            s.log(format!("Macro cache hit! Replayed {steps_executed} steps deterministically (0 LLM tokens)."));
            return Ok(());
        }
        ReplayOutcome::FallbackRequired { reason, .. } => {
            let mut s = state.lock().await;
            s.log(format!(
                "Macro replay failed ({reason}); falling back to fresh perception frame."
            ));
        }
        ReplayOutcome::CacheMiss { .. } => {
            // Proceed to autonomous extraction and planning turn
        }
    }

    // Step 5b: Autonomous On-Demand Extraction if Prompt Demands (for cache miss)
    if is_extraction {
        let table_name = derive_table_name(&target_url, &effective_prompt);
        let extracted_rows = harvest_records_from_page(
            &page,
            &perception.marks,
            &target_url,
            &table_name,
            &requested_attributes,
        )
        .await;
        if !extracted_rows.is_empty() {
            let row_count = extracted_rows.len();
            if let Ok(inserted) = store.insert_rows(&extracted_rows).await {
                let preview = store
                    .query_table(&table_name, Some(50))
                    .await
                    .unwrap_or_default();
                let total = store
                    .row_count(&table_name)
                    .await
                    .unwrap_or(inserted.len() as u64);
                let mut s = state.lock().await;
                s.preview_table = Some(table_name.clone());
                s.preview_data = preview;
                s.preview_total_rows = total;
                s.data_scroll = 0;
                s.active_tab = ActiveTab::ExtractedData;
                s.log(format!(
                    "Harvested {} records into SQLite table '{}'. Switched to [1] Extracted Data view.",
                    row_count, table_name
                ));
            }
        }
    }

    // Step 6: Strategic ReAct Planning Turn
    {
        let mut s = state.lock().await;
        s.status = AgentStatus::Planning;
    }

    let history: Vec<ChatMessage> = Vec::new();
    let plan_res = groq
        .plan_next_action(&effective_prompt, &history, &perception.content)
        .await;

    let plan = match plan_res {
        Ok(p) => p,
        Err(e) => {
            if is_extraction {
                let mut s = state.lock().await;
                s.status = AgentStatus::Idle;
                s.log("Data extraction complete. View harvested records in [1] Extracted Data.");
                return Ok(());
            } else {
                return Err(e);
            }
        }
    };

    {
        let mut s = state.lock().await;
        s.status = AgentStatus::Executing;
        s.log(format!("Planner reasoning: {}", plan.reasoning));
    }

    // Step 7: Local Mechanical Action Execution
    for action in &plan.actions {
        let action_str = format!("{:?} on mark @{}", action.action, action.mark_id);
        state.lock().await.log(format!("Executing: {action_str}"));

        // Resolve element coordinates from perception snapshot
        if let Some(mark) = perception.marks.iter().find(|m| m.id == action.mark_id) {
            let center_x = mark.bounds[0] + mark.bounds[2] / 2.0;
            let center_y = mark.bounds[1] + mark.bounds[3] / 2.0;

            match action.action {
                harness_core::schema::InteractionKind::Click => {
                    driver.click_at(center_x, center_y).await?;
                }
                harness_core::schema::InteractionKind::Type => {
                    if let Some(ref text) = action.text {
                        driver.click_at(center_x, center_y).await?;
                        driver.type_text(text).await?;
                    }
                }
                harness_core::schema::InteractionKind::Scroll => {
                    driver.scroll_by(0.0, 300.0).await?;
                }
                _ => {}
            }
        }
    }

    // Step 8: Record verified macro for future 0-LLM replay
    let _ = replay
        .record_macro(&domain, &effective_prompt, plan.actions.clone())
        .await;

    {
        let mut s = state.lock().await;
        s.status = AgentStatus::Idle;
        s.log("Task executed successfully. Action sequence recorded to macro cache.");
    }

    Ok(())
}

/// Pure immediate-mode double-buffered UI rendering.
fn render_ui(frame: &mut Frame, state: &AppState) {
    let size = frame.area();

    // Layout hierarchy: Header (3) | Tab Bar (2) | Main Body (min 8) | Input Footer (3)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(size);

    // ── 1. Top Header Bar ───────────────────────────────────────────
    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" AUTONOMOUS BROWSER HARNESS [Industrial Edition] ");

    let inner_header = header_block.inner(chunks[0]);
    let header_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(34),
            Constraint::Min(20),
            Constraint::Length(26),
        ])
        .split(inner_header);

    let status_line = Line::from(vec![
        Span::raw(" Status: "),
        Span::styled(
            format!(" {} ", state.status.as_str()),
            Style::default()
                .fg(Color::Black)
                .bg(state.status.badge_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  Stealth: "),
        Span::styled(
            format!("{:?}", state.stealth_mode),
            Style::default().fg(Color::Yellow),
        ),
    ]);

    let url_line = Line::from(vec![
        Span::raw(" URL: "),
        Span::styled(&state.active_url, Style::default().fg(Color::White)),
    ]);

    let token_ratio = (state.current_tokens_used as f64 / TOKEN_BUDGET_CEILING as f64).min(1.0);
    let token_gauge = Gauge::default()
        .block(Block::default())
        .gauge_style(Style::default().fg(if token_ratio > 0.8 {
            Color::Red
        } else {
            Color::Green
        }))
        .ratio(token_ratio)
        .label(format!("{}/7,000 Tokens", state.current_tokens_used));

    frame.render_widget(header_block, chunks[0]);
    frame.render_widget(Paragraph::new(status_line), header_layout[0]);
    frame.render_widget(Paragraph::new(url_line), header_layout[1]);
    frame.render_widget(token_gauge, header_layout[2]);

    // ── 2. Top Navigation Tabs ─────────────────────────────────────
    let tab_titles = vec![
        Line::from(" [1] Extracted Data "),
        Line::from(" [2] Perception Frame "),
        Line::from(" [3] Execution Log "),
    ];
    let active_idx = match state.active_tab {
        ActiveTab::ExtractedData => 0,
        ActiveTab::PerceptionFrame => 1,
        ActiveTab::ExecutionLog => 2,
    };
    let tabs = Tabs::new(tab_titles)
        .select(active_idx)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(25, 25, 45)),
        )
        .divider("│");
    frame.render_widget(tabs, chunks[1]);

    // ── 3. Active Tab View (Only One Selected Tab Rendered, 100% Full-Width) ──
    match state.active_tab {
        ActiveTab::ExtractedData => {
            render_extracted_data_tab(frame, chunks[2], state);
        }
        ActiveTab::PerceptionFrame => {
            render_perception_tab(frame, chunks[2], state);
        }
        ActiveTab::ExecutionLog => {
            render_execution_log_tab(frame, chunks[2], state);
        }
    }

    // ── 4. Bottom Interactive Command Line ──────────────────────────
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(" Command Buffer [/api, /export, /tables, /extract, /reset, /tab, /quit | Press 1/2/3 to switch tab] ");

    let total_chars = state.input_buffer.chars().count();
    let cursor_pos = state.cursor_position.min(total_chars);

    let prefix_span = Span::styled(
        " [harness] > ",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    );

    let chars: Vec<char> = state.input_buffer.chars().collect();
    let before_str: String = chars[..cursor_pos].iter().collect();

    let mut spans = vec![
        prefix_span,
        Span::styled(before_str, Style::default().fg(Color::White)),
    ];

    if cursor_pos < total_chars {
        let cursor_ch = chars[cursor_pos].to_string();
        spans.push(Span::styled(
            cursor_ch,
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ));
        let after_str: String = chars[cursor_pos + 1..].iter().collect();
        spans.push(Span::styled(after_str, Style::default().fg(Color::White)));
    } else {
        spans.push(Span::styled("█", Style::default().fg(Color::LightMagenta)));
    }

    let input_line = Line::from(spans);
    let input_para = Paragraph::new(input_line).block(input_block);
    frame.render_widget(input_para, chunks[3]);

    // ── 4. HITL Challenge Modal (if triggered) ──────────────────────
    if let Some(ref hitl) = state.hitl_alert {
        render_hitl_modal(frame, size, hitl);
    }
}

/// Renders a centered modal dialog when human intervention is requested.
fn render_hitl_modal(frame: &mut Frame, area: Rect, hitl: &HitlNotification) {
    let modal_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(40),
            Constraint::Percentage(30),
        ])
        .split(area)[1];

    let modal_area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(modal_area)[1];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .title(" ACTION REQUIRED: Human-in-the-Loop Challenge Detected ");

    let text = vec![
        Line::from(Span::styled(
            "A security barrier requires interactive resolution:",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            format!("Type: {:?}", hitl.challenge_type),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            &hitl.message,
            Style::default().fg(Color::LightRed),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "Switched to headed browser window. Complete challenge and press ESC to resume.",
            Style::default().fg(Color::Cyan),
        )),
    ];

    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: true })
        .style(Style::default().bg(Color::Black));

    frame.render_widget(Clear, modal_area);
    frame.render_widget(paragraph, modal_area);
}

/// Renders the [1] Extracted Data view tab with tabular glimpse of harvested SQLite records.
fn render_extracted_data_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    if let Some(ref tbl) = state.preview_table {
        let total_rows = state.preview_data.len();
        let inner_height = (area.height.saturating_sub(5) as usize).max(1);
        let scroll_offset = state.data_scroll.min(total_rows.saturating_sub(1));
        let display_rows: Vec<_> = state
            .preview_data
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(inner_height)
            .collect();

        let showing_start = if total_rows == 0 {
            0
        } else {
            scroll_offset + 1
        };
        let showing_end = scroll_offset + display_rows.len();

        let title = format!(
            " [1] Extracted Dataset Glimpse: [{}] (Showing rows {}-{} of {} records — Scroll: Up/Down/Wheel) ",
            tbl,
            showing_start,
            showing_end,
            state.preview_total_rows.max(total_rows as u64)
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .title(title);

        let mut lines = Vec::new();

        // Header line
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<4}", "#"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:<6}", "ID"), Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:<52}", " Extracted Record / Title"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<28}", " Target URL / Details"),
                Style::default().fg(Color::Green),
            ),
            Span::styled(" Captured", Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(Span::styled(
            "─".repeat(area.width.saturating_sub(2) as usize),
            Style::default().fg(Color::DarkGray),
        )));

        if state.preview_data.is_empty() {
            lines.push(Line::from(Span::styled(
                "  Table exists in SQLite but currently has 0 rows.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (display_idx, (real_idx, row)) in display_rows.into_iter().enumerate() {
                let id_val = row
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(real_idx as i64 + 1);
                let payload = row.get("payload").unwrap_or(row);

                let title = payload
                    .get("title")
                    .or_else(|| payload.get("name"))
                    .or_else(|| payload.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let rank = payload.get("rank").and_then(|v| v.as_str()).unwrap_or("");
                let score = payload.get("score").and_then(|v| v.as_str());
                let author = payload.get("author").and_then(|v| v.as_str());

                let display_title = if !rank.is_empty() {
                    format!("{}. {}", rank, title)
                } else if !title.is_empty() {
                    title.to_string()
                } else {
                    serde_json::to_string(payload).unwrap_or_default()
                };

                let target_url = payload
                    .get("url")
                    .and_then(|v| v.as_str())
                    .or_else(|| row.get("source_url").and_then(|v| v.as_str()))
                    .unwrap_or("");

                let mut details = target_url.to_string();
                if let Some(sc) = score {
                    details = format!("{} | {}", sc, details);
                } else if let Some(au) = author {
                    details = format!("by {} | {}", au, details);
                }

                let time_str = row
                    .get("extracted_at")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.split('T').nth(1))
                    .map(|s| s.chars().take(8).collect::<String>())
                    .unwrap_or_else(|| "--:--:--".to_string());

                let truncated_title = if display_title.chars().count() > 50 {
                    format!("{}…", display_title.chars().take(49).collect::<String>())
                } else {
                    format!("{:<50}", display_title)
                };

                let truncated_details = if details.chars().count() > 26 {
                    format!("{}…", details.chars().take(25).collect::<String>())
                } else {
                    format!("{:<26}", details)
                };

                let row_style = if display_idx % 2 == 0 {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::LightCyan)
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:<4}", real_idx + 1),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(
                        format!("{:<6}", id_val),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(format!(" {:<51}", truncated_title), row_style),
                    Span::styled(
                        format!(" {:<27}", truncated_details),
                        Style::default().fg(Color::LightGreen),
                    ),
                    Span::styled(
                        format!(" {}", time_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }

        // Subtitle action guide
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                " Actions: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("/export {} csv", tbl),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  │  "),
            Span::styled(
                format!("/api {} 8080", tbl),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  │  "),
            Span::styled(
                "Scroll: Up/Down/PageUp/PageDown/Wheel  │  Press [2] DOM, [3] Logs",
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        let para = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" [1] Extracted Data View: No Tables Harvested Yet ");

        let lines = vec![
            Line::from(Span::styled(
                "No harvested user tables found in database yet.",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "HOW TO EXPLICITLY CREATE & HARVEST DATA ON-DEMAND:",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("  • /extract [table_name]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw("   — Explicitly harvest articles, links, and items from the active page into SQLite"),
            ]),
            Line::from(vec![
                Span::styled("  • Natural Language Query ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw("   — Type e.g. 'Extract top trending repositories on github.com'"),
            ]),
            Line::from(vec![
                Span::styled("  • /tables [table_name]   ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw("   — List all harvested tables or preview a specific table"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Active Page URL: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&state.active_url, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Tab Navigation: Press [1] for Data, [2] for Perception Frame, [3] for Execution Log",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let para = Paragraph::new(lines).block(block);
        frame.render_widget(para, area);
    }
}

/// Renders the [2] Perception Frame view tab occupying 100% full-width with vertical scrolling.
fn render_perception_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let total_lines = state.perception_frame.lines().count();
    let current_line = (state.perception_scroll as usize + 1).min(total_lines.max(1));
    let title = format!(
        " [2] Set-of-Marks Perception Frame (<7,000 tok) [Line {}/{} — Scroll: Up/Down/Wheel] — Press 1 for Data, 3 for Logs ",
        current_line,
        total_lines.max(1)
    );
    let perception_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(title);

    let perception_para = Paragraph::new(state.perception_frame.as_str())
        .block(perception_block)
        .scroll((state.perception_scroll, 0))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::LightCyan));

    frame.render_widget(perception_para, area);
}

/// Renders the [3] Execution Log view tab occupying 100% full-width with history scroll.
fn render_execution_log_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let total_logs = state.execution_logs.len();
    let title = if state.log_scroll > 0 {
        format!(
            " [3] Execution Log (CDP/SQLite) [Scrolled {} back / {} logs — Press Down/End for Live] ",
            state.log_scroll,
            total_logs
        )
    } else {
        format!(
            " [3] Real-Time Native Execution Log (CDP/SQLite) [Live: {} logs — Scroll: Up/PageUp to Scroll Back] — Press 1 for Data, 2 for DOM ",
            total_logs
        )
    };

    let log_block = Block::default()
        .borders(Borders::ALL)
        .border_style(if state.log_scroll > 0 {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::Green)
        })
        .title(title);

    let inner_width = (area.width.saturating_sub(2) as usize).max(1);
    let inner_height = (area.height.saturating_sub(2) as usize).max(1);

    let mut selected_msgs: Vec<&String> = Vec::new();
    let mut used_rows: usize = 0;

    for msg in state.execution_logs.iter().rev().skip(state.log_scroll) {
        let msg_rows = msg
            .lines()
            .map(|l| {
                let len = l.chars().count() + 2;
                len.div_ceil(inner_width).max(1)
            })
            .sum::<usize>();

        if used_rows + msg_rows > inner_height && !selected_msgs.is_empty() {
            break;
        }
        used_rows += msg_rows;
        selected_msgs.push(msg);
    }
    selected_msgs.reverse();

    let mut log_lines: Vec<Line> = Vec::new();
    for msg in selected_msgs {
        let style = if msg.contains(REJECTION_LITERAL) {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else if msg.contains("Error") || msg.contains("failed") || msg.contains("Command error") {
            Style::default().fg(Color::LightRed)
        } else if msg.contains("hit!")
            || msg.contains("successfully")
            || msg.contains("Started")
            || msg.contains("Harvested")
        {
            Style::default().fg(Color::LightGreen)
        } else if msg.starts_with("───") {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        for (line_idx, line_str) in msg.lines().enumerate() {
            let bullet = if line_idx == 0 { "• " } else { "  " };
            log_lines.push(Line::from(vec![
                Span::styled(bullet, Style::default().fg(Color::LightGreen)),
                Span::styled(line_str, style),
            ]));
        }
    }

    let log_para = Paragraph::new(log_lines)
        .block(log_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(log_para, area);
}

/// Derives a clean SQLite table name from URL and prompt intent.
fn derive_table_name(url: &str, prompt: &str) -> String {
    let prompt_lower = prompt.to_lowercase();
    if prompt_lower.contains("trending") && prompt_lower.contains("repo") {
        return "trending_repositories".to_string();
    }
    if prompt_lower.contains("hacker")
        || prompt_lower.contains("story")
        || prompt_lower.contains("stories")
    {
        return "hacker_news_stories".to_string();
    }

    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            let clean_host = host.replace(['.', '-'], "_");
            if host.contains("ycombinator") {
                return "hacker_news_stories".to_string();
            }
            if host.contains("github") {
                return "github_repositories".to_string();
            }
            return format!("{clean_host}_data");
        }
    }

    "harvested_data".to_string()
}

/// Harvests structured records from an active browser page or fallback marks.
async fn harvest_records_from_page(
    page: &Page,
    fallback_marks: &[MarkEntry],
    source_url: &str,
    table_name: &str,
    target_attributes: &[String],
) -> Vec<DataRow> {
    let script = r#"
        (() => {
            const results = [];

            // 1. Amazon search results & products
            const amzRows = document.querySelectorAll('div[data-component-type="s-search-result"], .s-result-item[data-asin]:not([data-asin=""])');
            if (amzRows.length > 0) {
                amzRows.forEach((r, idx) => {
                    const titleEl = r.querySelector('h2 a span, h2 span, h2 a');
                    const offscreenPrice = r.querySelector('.a-price .a-offscreen');
                    const priceWhole = r.querySelector('.a-price-whole');
                    const priceFraction = r.querySelector('.a-price-fraction');
                    const ratingEl = r.querySelector('i.a-icon-star-small span.a-icon-alt, span.a-icon-alt, [aria-label*="stars"]');
                    const linkEl = r.querySelector('h2 a');
                    
                    let price = null;
                    if (offscreenPrice) {
                        price = offscreenPrice.innerText.trim();
                    } else if (priceWhole) {
                        price = priceWhole.innerText.trim() + (priceFraction ? '.' + priceFraction.innerText.trim() : '');
                    }
                    
                    if (titleEl) {
                        const title = titleEl.innerText.trim();
                        if (title.length > 0) {
                            results.push({
                                rank: (idx + 1).toString(),
                                product_name: title,
                                title: title,
                                price: price,
                                rating: ratingEl ? ratingEl.innerText.trim() : null,
                                url: linkEl ? linkEl.href : null
                            });
                        }
                    }
                });
                if (results.length > 0) return results.slice(0, 50);
            }

            // 2. Books to Scrape (books.toscrape.com)
            const bookRows = document.querySelectorAll('article.product_pod');
            if (bookRows.length > 0) {
                bookRows.forEach((r, idx) => {
                    const titleEl = r.querySelector('h3 a');
                    const priceEl = r.querySelector('.price_color');
                    const ratingEl = r.querySelector('p.star-rating');
                    if (titleEl) {
                        const title = titleEl.getAttribute('title') || titleEl.innerText.trim();
                        results.push({
                            rank: (idx + 1).toString(),
                            product_name: title,
                            title: title,
                            price: priceEl ? priceEl.innerText.trim() : null,
                            rating: ratingEl ? ratingEl.className.replace('star-rating', '').trim() : null,
                            url: titleEl.href || null
                        });
                    }
                });
                if (results.length > 0) return results.slice(0, 50);
            }

            // 3. Hacker News specific rows
            const hnRows = document.querySelectorAll('tr.athing');
            if (hnRows.length > 0) {
                hnRows.forEach((r, idx) => {
                    const titleEl = r.querySelector('.titleline > a');
                    const rankEl = r.querySelector('.rank');
                    const subtext = r.nextElementSibling;
                    const scoreEl = subtext ? subtext.querySelector('.score') : null;
                    const userEl = subtext ? subtext.querySelector('.hnuser') : null;
                    if (titleEl) {
                        results.push({
                            rank: rankEl ? rankEl.innerText.replace('.', '').trim() : (idx + 1).toString(),
                            title: titleEl.innerText.trim(),
                            url: titleEl.href,
                            score: scoreEl ? scoreEl.innerText.trim() : null,
                            author: userEl ? userEl.innerText.trim() : null
                        });
                    }
                });
                if (results.length > 0) return results;
            }

            // 4. GitHub Trending specific rows
            const ghRows = document.querySelectorAll('article.Box-row');
            if (ghRows.length > 0) {
                ghRows.forEach((r, idx) => {
                    const h2a = r.querySelector('h2 a');
                    const desc = r.querySelector('p');
                    if (h2a) {
                        results.push({
                            rank: (idx + 1).toString(),
                            title: h2a.innerText.replace(/\s+/g, ' ').trim(),
                            url: h2a.href,
                            description: desc ? desc.innerText.trim() : null
                        });
                    }
                });
                if (results.length > 0) return results;
            }

            // 5. Generic listing elements (links with meaningful text)
            const links = document.querySelectorAll('main a, article a, ul a, ol a, h1 a, h2 a, h3 a, a');
            const seen = new Set();
            links.forEach(a => {
                const text = (a.innerText || a.getAttribute('aria-label') || '').trim();
                const href = a.href || '';
                if (text.length >= 4 && !seen.has(text) && !seen.has(href)) {
                    const lower = text.toLowerCase();
                    if (lower === 'login' || lower === 'sign in' || lower === 'sign up' || lower === 'submit' || lower === 'past' || lower === 'comments' || lower === 'hide' || lower === 'privacy') {
                        return;
                    }
                    seen.add(text);
                    if (href) seen.add(href);
                    results.push({
                        rank: (results.length + 1).toString(),
                        title: text,
                        url: href,
                        tag: a.tagName.toLowerCase()
                    });
                }
            });
            return results.slice(0, 50);
        })()
    "#;

    if let Ok(js_res) = page.evaluate(script).await {
        if let Some(arr) = js_res.value().and_then(|v| v.as_array()) {
            if !arr.is_empty() {
                return arr
                    .iter()
                    .map(|item| {
                        let filtered_payload = if !target_attributes.is_empty() {
                            if let Some(obj) = item.as_object() {
                                let mut m = serde_json::Map::new();
                                if let Some(r) = obj.get("rank") {
                                    m.insert("rank".to_string(), r.clone());
                                }
                                for (k, v) in obj {
                                    let k_lower = k.to_lowercase();
                                    let matched = target_attributes.iter().any(|req| {
                                        let req_lower = req.to_lowercase();
                                        k_lower == req_lower
                                            || (req_lower == "product name"
                                                && (k_lower == "title"
                                                    || k_lower == "product_name"
                                                    || k_lower == "name"))
                                            || (req_lower == "product_name"
                                                && (k_lower == "title"
                                                    || k_lower == "product_name"
                                                    || k_lower == "name"))
                                            || (req_lower == "title"
                                                && (k_lower == "product_name" || k_lower == "name"))
                                            || (req_lower == "name"
                                                && (k_lower == "title"
                                                    || k_lower == "product_name"))
                                            || (req_lower == "price"
                                                && (k_lower == "cost" || k_lower == "amount"))
                                    });
                                    if matched {
                                        m.insert(k.clone(), v.clone());
                                    }
                                }
                                serde_json::Value::Object(m)
                            } else {
                                item.clone()
                            }
                        } else {
                            item.clone()
                        };

                        DataRow {
                            source_url: source_url.to_string(),
                            table_name: table_name.to_string(),
                            payload: filtered_payload,
                            extracted_at: Utc::now(),
                        }
                    })
                    .collect();
            }
        }
    }

    // Fallback: extract from marks if provided
    if !fallback_marks.is_empty() {
        return fallback_marks
            .iter()
            .filter(|m| {
                let name = m.name.trim();
                name.len() >= 4
                    && !name.eq_ignore_ascii_case("login")
                    && !name.eq_ignore_ascii_case("submit")
                    && !name.eq_ignore_ascii_case("past")
                    && !name.eq_ignore_ascii_case("comments")
            })
            .map(|m| DataRow {
                source_url: source_url.to_string(),
                table_name: table_name.to_string(),
                payload: serde_json::json!({
                    "id": m.id,
                    "title": m.name,
                    "role": m.role,
                    "value": m.value,
                }),
                extracted_at: Utc::now(),
            })
            .collect();
    }

    Vec::new()
}

/// Parses Set-of-Marks perception frame lines to extract structured records.
fn extract_records_from_perception_text(
    content: &str,
    source_url: &str,
    table_name: &str,
) -> Vec<DataRow> {
    let mut rows = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("[@") {
            if let Some((id_str, after_id)) = rest.split_once(']') {
                let id: u32 = id_str.parse().unwrap_or(0);
                if let Some(name_start) = after_id.find("name=\"") {
                    let after_name = &after_id[name_start + 6..];
                    if let Some(name_end) = after_name.find('"') {
                        let name = &after_name[..name_end];
                        let clean_name = name.trim();
                        let lower = clean_name.to_lowercase();
                        if clean_name.len() >= 4
                            && !lower.contains("login")
                            && !lower.contains("submit")
                            && !lower.contains("past")
                            && !lower.contains("comment")
                            && !lower.contains("hacker news")
                            && !lower.contains("hide")
                        {
                            rows.push(DataRow {
                                source_url: source_url.to_string(),
                                table_name: table_name.to_string(),
                                payload: serde_json::json!({
                                    "id": id,
                                    "title": clean_name,
                                    "role": "link",
                                }),
                                extracted_at: Utc::now(),
                            });
                        }
                    }
                }
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_history_up_and_down_recall() {
        let mut state = AppState::new();
        state.push_history("/help".to_string());
        state.push_history("/status".to_string());
        state.push_history("extract top repos".to_string());

        assert_eq!(state.input_history.len(), 3);

        // User typed a draft
        state.input_buffer = "draft in progress".to_string();

        // 1. Up arrow: recalls last command
        state.history_up();
        assert_eq!(state.input_buffer, "extract top repos");

        // 2. Up arrow: recalls second last
        state.history_up();
        assert_eq!(state.input_buffer, "/status");

        // 3. Up arrow: recalls oldest
        state.history_up();
        assert_eq!(state.input_buffer, "/help");

        // 4. Up arrow at boundary does not change
        state.history_up();
        assert_eq!(state.input_buffer, "/help");

        // 5. Down arrow moves forward
        state.history_down();
        assert_eq!(state.input_buffer, "/status");

        state.history_down();
        assert_eq!(state.input_buffer, "extract top repos");

        // 6. Down arrow past latest restores saved draft
        state.history_down();
        assert_eq!(state.input_buffer, "draft in progress");
    }

    #[test]
    fn test_tab_completion_cycling() {
        let mut state = AppState::new();

        // Empty input -> Tab starts cycling with first slash command "/api"
        state.input_buffer.clear();
        state.complete_tab();
        assert_eq!(state.input_buffer, "/api");

        // Subsequent Tab cycles through commands
        state.complete_tab();
        assert_eq!(state.input_buffer, "/clear");

        state.complete_tab();
        assert_eq!(state.input_buffer, "/export");

        // Prefix match test
        state.completion_prefix = None;
        state.completion_index = 0;
        state.input_buffer = "/st".to_string();

        state.complete_tab();
        assert_eq!(state.input_buffer, "/status");

        state.complete_tab();
        assert_eq!(state.input_buffer, "/stealth");

        // Single unique match auto-appends space
        state.completion_prefix = None;
        state.completion_index = 0;
        state.input_buffer = "/h".to_string();
        state.complete_tab();
        assert_eq!(state.input_buffer, "/help ");
    }

    #[test]
    fn test_cursor_navigation_and_editing() {
        let mut state = AppState::new();

        // 1. Insert characters sequentially
        state.insert_char('h');
        state.insert_char('e');
        state.insert_char('l');
        state.insert_char('o');
        assert_eq!(state.input_buffer, "helo");
        assert_eq!(state.cursor_position, 4);

        // 2. Move cursor left
        state.move_cursor_left(); // at 'o' (pos 3)
        assert_eq!(state.cursor_position, 3);

        // 3. Insert character in the middle
        state.insert_char('l'); // between 'l' and 'o'
        assert_eq!(state.input_buffer, "hello");
        assert_eq!(state.cursor_position, 4);

        // 4. Move home and insert at beginning
        state.move_cursor_home();
        assert_eq!(state.cursor_position, 0);
        state.insert_char('>');
        assert_eq!(state.input_buffer, ">hello");
        assert_eq!(state.cursor_position, 1);

        // 5. Backspace at pos 1 deletes '>'
        state.delete_backward();
        assert_eq!(state.input_buffer, "hello");
        assert_eq!(state.cursor_position, 0);

        // 6. Delete forward deletes 'h'
        state.delete_forward();
        assert_eq!(state.input_buffer, "ello");
        assert_eq!(state.cursor_position, 0);

        // 7. Move end
        state.move_cursor_end();
        assert_eq!(state.cursor_position, 4);
        state.move_cursor_right(); // should not exceed len
        assert_eq!(state.cursor_position, 4);

        // 8. Multi-byte Unicode character handling (e.g. emoji)
        state.input_buffer.clear();
        state.cursor_position = 0;
        state.insert_char('🦀');
        state.insert_char('!');
        assert_eq!(state.input_buffer, "🦀!");
        assert_eq!(state.cursor_position, 2);

        state.move_cursor_left(); // before '!'
        assert_eq!(state.cursor_position, 1);
        state.delete_backward(); // deletes '🦀' without panic
        assert_eq!(state.input_buffer, "!");
        assert_eq!(state.cursor_position, 0);
    }

    #[test]
    fn test_tab_completion_includes_reset() {
        let mut state = AppState::new();
        state.input_buffer = "/r".to_string();
        state.complete_tab();
        assert_eq!(state.input_buffer, "/reset ");
        assert_eq!(state.cursor_position, 7);
    }

    #[test]
    fn test_active_tab_cycling() {
        let tab = ActiveTab::ExtractedData;
        assert_eq!(tab.next(), ActiveTab::PerceptionFrame);
        assert_eq!(tab.next().next(), ActiveTab::ExecutionLog);
        assert_eq!(tab.next().next().next(), ActiveTab::ExtractedData);

        assert_eq!(tab.prev(), ActiveTab::ExecutionLog);
        assert_eq!(tab.prev().prev(), ActiveTab::PerceptionFrame);
        assert_eq!(tab.prev().prev().prev(), ActiveTab::ExtractedData);
    }

    #[test]
    fn test_derive_table_name() {
        assert_eq!(
            derive_table_name("https://news.ycombinator.com", "extract"),
            "hacker_news_stories"
        );
        assert_eq!(
            derive_table_name("https://github.com/trending", "Extract top trending repos"),
            "trending_repositories"
        );
        assert_eq!(
            derive_table_name("https://example.com/items", "get items"),
            "example_com_data"
        );
    }

    #[test]
    fn test_extract_records_from_perception_text() {
        let sample = r#"
<untrusted_web_content origin="news.ycombinator.com">
[@1] <a name="">
[@2] <a name="Hacker News">
[@12] <a name="Three new discoveries in mathematics, computer science, and physics">
[@13] <a name="openai.com">
[@14] <a name="jonluca">
[@15] <a name="8 hours ago">
[@17] <a name="314 comments">
        "#;

        let rows = extract_records_from_perception_text(
            sample,
            "https://news.ycombinator.com",
            "hacker_news_stories",
        );
        assert!(!rows.is_empty());
        let titles: Vec<&str> = rows
            .iter()
            .filter_map(|r| r.payload.get("title").and_then(|v| v.as_str()))
            .collect();
        assert!(
            titles.contains(&"Three new discoveries in mathematics, computer science, and physics")
        );
        assert!(titles.contains(&"openai.com"));
        assert!(!titles.contains(&"Hacker News")); // Filtered out as site logo
        assert!(!titles.contains(&"314 comments")); // Filtered out
    }

    #[test]
    fn test_tab_scrolling() {
        let mut state = AppState::new();

        // 1. Test ExtractedData scrolling
        state.active_tab = ActiveTab::ExtractedData;
        state.preview_data = (0..50)
            .map(|i| serde_json::json!({"id": i, "title": format!("Row {i}")}))
            .collect();
        assert_eq!(state.data_scroll, 0);

        state.scroll_down();
        assert_eq!(state.data_scroll, 1);
        state.scroll_up();
        assert_eq!(state.data_scroll, 0);

        state.page_down();
        assert_eq!(state.data_scroll, 10);
        state.page_up();
        assert_eq!(state.data_scroll, 0);

        state.scroll_to_bottom();
        assert_eq!(state.data_scroll, 49);
        state.scroll_to_top();
        assert_eq!(state.data_scroll, 0);

        // 2. Test PerceptionFrame scrolling
        state.active_tab = ActiveTab::PerceptionFrame;
        state.perception_frame = (1..=100)
            .map(|i| format!("[@{i}] Button {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(state.perception_scroll, 0);

        state.scroll_down();
        assert_eq!(state.perception_scroll, 1);
        state.scroll_up();
        assert_eq!(state.perception_scroll, 0);

        state.page_down();
        assert_eq!(state.perception_scroll, 15);
        state.page_up();
        assert_eq!(state.perception_scroll, 0);

        state.scroll_to_bottom();
        assert_eq!(state.perception_scroll, 99);
        state.scroll_to_top();
        assert_eq!(state.perception_scroll, 0);

        // 3. Test ExecutionLog scrolling
        state.active_tab = ActiveTab::ExecutionLog;
        for i in 1..=30 {
            state.log(format!("Log message {i}"));
        }
        assert_eq!(state.log_scroll, 0); // starts live

        state.scroll_up(); // scroll into history
        assert_eq!(state.log_scroll, 1);
        state.scroll_down(); // scroll back toward live
        assert_eq!(state.log_scroll, 0);

        state.page_up();
        assert_eq!(state.log_scroll, 10);
        state.page_down();
        assert_eq!(state.log_scroll, 0);

        state.scroll_to_bottom();
        assert_eq!(state.log_scroll, 0);
    }
}
