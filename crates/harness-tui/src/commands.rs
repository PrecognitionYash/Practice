// ──────────────────────────────────────────────────────────────────────
// harness-tui/src/commands.rs — Zero-Allocation Slash-Command Parser
//
// Intercepts input starting with `/` and routes directly to local native
// handlers without invoking LLM inference.
//
// Commands:
//  • /api <table_name> [port]            — Start embedded Axum REST server
//  • /export <table_name> <parquet|csv|jsonl> — Export SQLite data to disk
//  • /stealth <fast|ultra>               — Toggle CDP fingerprint profile
//  • /undo                               — Roll back last executed DOM action
//  • /clear                              — Clear execution log buffer
//  • /quit                               — Gracefully terminate runtime
// ──────────────────────────────────────────────────────────────────────

use harness_browser::driver::StealthMode;
use harness_core::error::{HarnessError, HarnessResult};

/// Supported file formats for data export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Parquet,
    Csv,
    Jsonl,
}

impl std::str::FromStr for ExportFormat {
    type Err = HarnessError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "parquet" => Ok(ExportFormat::Parquet),
            "csv" => Ok(ExportFormat::Csv),
            "jsonl" | "json" => Ok(ExportFormat::Jsonl),
            other => Err(HarnessError::InvalidArgs {
                command: "export".into(),
                reason: format!(
                    "unsupported export format '{other}', expected parquet, csv, or jsonl"
                ),
            }),
        }
    }
}

/// Parsed representation of a slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// Start embedded Axum server on table and optional port (default 8080).
    Api { table: Option<String>, port: u16 },
    /// Export table to specified format.
    Export {
        table: Option<String>,
        format: Option<ExportFormat>,
    },
    /// Set or toggle CDP stealth level.
    Stealth { mode: Option<StealthMode> },
    /// List all non-system tables, or inspect a specific table.
    Tables { table: Option<String> },
    /// Explicitly harvest active page into an SQLite table.
    Extract { table: Option<String> },
    /// Switch active TUI display tab ([1] data, [2] dom, [3] logs).
    Tab { tab: Option<String> },
    /// Scroll active view tab (up, down, top, bottom, or reset).
    Scroll {
        direction: Option<String>,
        count: Option<usize>,
    },
    /// Inspect or manage encrypted AES-256-GCM session vault.
    Vault {
        action: Option<String>,
        domain: Option<String>,
    },
    /// Recursively crawl and harvest pages with strict relative domain & path prefix boundary.
    Crawl {
        seed_url: String,
        table: Option<String>,
        restrict_path: bool,
        ai_goal: Option<String>,
        attributes: Vec<String>,
    },
    /// Stop an active recursive crawl or task in progress.
    Stop,
    /// Display current browser and runtime status.
    Status,
    /// Display help and command usage manual.
    Help,
    /// Undo last action.
    Undo,
    /// Clear log buffer.
    Clear,
    /// Completely reset harvested database datasets and state.
    Reset,
    /// Terminate harness.
    Quit,
}

impl SlashCommand {
    /// Parses a raw input string into a typed `SlashCommand`.
    ///
    /// Returns `Ok(Some(cmd))` if input starts with `/`.
    /// Returns `Ok(None)` if input is a normal prompt for LLM evaluation.
    /// Returns `Err(HarnessError)` if the slash command syntax is invalid.
    pub fn parse(input: &str) -> HarnessResult<Option<Self>> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return Ok(None);
        }

        let body = &trimmed[1..];
        let mut tokens = body.split_whitespace();

        let command_name = match tokens.next() {
            Some(cmd) => cmd.to_lowercase(),
            None => {
                return Err(HarnessError::UnknownCommand {
                    command: String::new(),
                })
            }
        };

        match command_name.as_str() {
            "api" => {
                let table = tokens.next().map(|s| s.to_string());
                let port = if let Some(port_str) = tokens.next() {
                    port_str
                        .parse::<u16>()
                        .map_err(|_| HarnessError::InvalidArgs {
                            command: "api".into(),
                            reason: format!("invalid port number '{port_str}'"),
                        })?
                } else {
                    8080
                };

                Ok(Some(SlashCommand::Api { table, port }))
            }

            "export" => {
                let table = tokens.next().map(|s| s.to_string());
                let format = tokens
                    .next()
                    .map(|f| f.parse::<ExportFormat>())
                    .transpose()?;

                Ok(Some(SlashCommand::Export { table, format }))
            }

            "stealth" => {
                let mode = match tokens.next() {
                    Some(mode_str) => match mode_str.to_lowercase().as_str() {
                        "fast" => Some(StealthMode::Fast),
                        "ultra" => Some(StealthMode::Ultra),
                        other => {
                            return Err(HarnessError::InvalidArgs {
                                command: "stealth".into(),
                                reason: format!(
                                    "unknown stealth mode '{other}', expected 'fast' or 'ultra'"
                                ),
                            })
                        }
                    },
                    None => None,
                };

                Ok(Some(SlashCommand::Stealth { mode }))
            }

            "tables" => {
                let table = tokens.next().map(|s| s.to_string());
                Ok(Some(SlashCommand::Tables { table }))
            }
            "extract" | "harvest" => {
                let table = tokens.next().map(|s| s.to_string());
                Ok(Some(SlashCommand::Extract { table }))
            }
            "tab" | "view" => {
                let tab = tokens.next().map(|s| s.to_string());
                Ok(Some(SlashCommand::Tab { tab }))
            }
            "scroll" => {
                let direction = tokens.next().map(|s| s.to_lowercase());
                let count = if let Some(cnt_str) = tokens.next() {
                    cnt_str.parse::<usize>().ok()
                } else {
                    None
                };
                Ok(Some(SlashCommand::Scroll { direction, count }))
            }
            "vault" => {
                let first = tokens.next().map(|s| s.to_string());
                let second = tokens.next().map(|s| s.to_string());
                match first.as_deref() {
                    Some("save") | Some("store") => Ok(Some(SlashCommand::Vault {
                        action: Some("save".into()),
                        domain: second,
                    })),
                    Some("load") | Some("restore") => Ok(Some(SlashCommand::Vault {
                        action: Some("load".into()),
                        domain: second,
                    })),
                    Some("list") | Some("ls") => Ok(Some(SlashCommand::Vault {
                        action: Some("list".into()),
                        domain: None,
                    })),
                    Some(d) => Ok(Some(SlashCommand::Vault {
                        action: Some("inspect".into()),
                        domain: Some(d.to_string()),
                    })),
                    None => Ok(Some(SlashCommand::Vault {
                        action: None,
                        domain: None,
                    })),
                }
            }
            "crawl" => {
                let all_tokens: Vec<&str> = tokens.collect();
                if all_tokens.is_empty() {
                    return Err(HarnessError::InvalidArgs {
                        command: "crawl".into(),
                        reason:
                            "seed URL required: /crawl <url> [--attributes attr1,attr2] [query]"
                                .into(),
                    });
                }

                if all_tokens[0] == "stop" || all_tokens[0] == "cancel" || all_tokens[0] == "halt" {
                    return Ok(Some(SlashCommand::Stop));
                }

                // 1. Locate the URL token in the arguments (can appear anywhere)
                let mut url_idx = None;
                for (i, tok) in all_tokens.iter().enumerate() {
                    if is_url_candidate(tok) {
                        url_idx = Some(i);
                        break;
                    }
                }

                let url_pos = match url_idx {
                    Some(idx) => idx,
                    None => {
                        return Err(HarnessError::InvalidArgs {
                            command: "crawl".into(),
                            reason: "no valid target URL found in /crawl command. Example: /crawl https://github.com/rust-lang/rust [query]".into(),
                        });
                    }
                };

                let seed_url = normalize_url(all_tokens[url_pos]);

                // 2. Scan remaining tokens for flags (--all-paths, --no-prefix, --table, --attributes)
                let mut restrict_path = true;
                let mut explicit_table = None;
                let mut attributes = Vec::new();
                let mut query_words = Vec::new();

                let mut skip_next = false;
                for (i, tok) in all_tokens.iter().enumerate() {
                    if i == url_pos {
                        continue;
                    }
                    if skip_next {
                        skip_next = false;
                        continue;
                    }

                    if *tok == "--all-paths" || *tok == "--no-prefix" {
                        restrict_path = false;
                    } else if *tok == "--prefix" || *tok == "--restrict-prefix" {
                        restrict_path = true;
                    } else if *tok == "--table" || *tok == "-t" {
                        if let Some(next) = all_tokens.get(i + 1) {
                            explicit_table = Some(next.to_string());
                            skip_next = true;
                        }
                    } else if *tok == "--attributes" || *tok == "--attrs" || *tok == "--fields" {
                        if let Some(next) = all_tokens.get(i + 1) {
                            for a in next.split(',') {
                                let clean = a.trim();
                                if !clean.is_empty() {
                                    attributes.push(clean.to_string());
                                }
                            }
                            skip_next = true;
                        }
                    } else {
                        let lower = tok.to_lowercase();
                        if i + 1 == url_pos
                            && (lower == "from" || lower == "at" || lower == "on" || lower == "in")
                        {
                            continue;
                        }
                        query_words.push(*tok);
                    }
                }

                // Parse natural language "extract only a, b, c" if --attributes flag was not explicitly given
                if attributes.is_empty() {
                    let full_q = query_words.join(" ");
                    let lower_q = full_q.to_lowercase();
                    if let Some(idx) = lower_q.find("extract only ") {
                        let after = &full_q[idx + 13..];
                        for item in after.split([',', ';']) {
                            let clean = item.trim().trim_start_matches("and ").trim();
                            if !clean.is_empty() && !clean.contains("http") {
                                attributes.push(clean.to_string());
                            }
                        }
                    } else if let Some(idx) = lower_q.find("attributes:") {
                        let after = &full_q[idx + 11..];
                        for item in after.split([',', ';']) {
                            let clean = item.trim().trim_start_matches("and ").trim();
                            if !clean.is_empty() && !clean.contains("http") {
                                attributes.push(clean.to_string());
                            }
                        }
                    }
                }

                let (table, ai_goal) = if let Some(t) = explicit_table {
                    let goal = if query_words.is_empty() {
                        None
                    } else {
                        Some(query_words.join(" "))
                    };
                    (Some(t), goal)
                } else if query_words.len() == 1 {
                    let first = query_words[0];
                    let first_lower = first.to_lowercase();
                    if first_lower == "extract"
                        || first_lower == "find"
                        || first_lower == "scrape"
                        || first_lower == "get"
                    {
                        (None, Some(first.to_string()))
                    } else {
                        (Some(first.to_string()), None)
                    }
                } else if query_words.is_empty() {
                    (None, None)
                } else {
                    (None, Some(query_words.join(" ")))
                };

                Ok(Some(SlashCommand::Crawl {
                    seed_url,
                    table,
                    restrict_path,
                    ai_goal,
                    attributes,
                }))
            }
            "stop" | "cancel" | "halt" => Ok(Some(SlashCommand::Stop)),
            "status" => Ok(Some(SlashCommand::Status)),
            "help" | "h" | "?" => Ok(Some(SlashCommand::Help)),
            "undo" => Ok(Some(SlashCommand::Undo)),
            "clear" | "cls" => Ok(Some(SlashCommand::Clear)),
            "reset" => Ok(Some(SlashCommand::Reset)),
            "quit" | "exit" | "q" => Ok(Some(SlashCommand::Quit)),

            unknown => Err(HarnessError::UnknownCommand {
                command: unknown.to_string(),
            }),
        }
    }
}

/// Determines if a string token looks like a URL or bare web domain.
pub fn is_url_candidate(s: &str) -> bool {
    let clean = s.trim().trim_matches(|c: char| {
        c == ','
            || c == ';'
            || c == '('
            || c == ')'
            || c == '['
            || c == ']'
            || c == '<'
            || c == '>'
            || c == '\''
            || c == '"'
    });
    if clean.starts_with("http://") || clean.starts_with("https://") {
        return true;
    }
    if clean.starts_with("localhost") || clean.starts_with("127.0.0.1") {
        return true;
    }
    if let Some(slash_idx) = clean.find('/') {
        let domain_part = &clean[..slash_idx];
        if domain_part.contains('.') && !domain_part.starts_with('.') && !domain_part.ends_with('.')
        {
            return true;
        }
    } else if clean.contains('.') {
        let suffixes = [
            ".com", ".org", ".net", ".edu", ".gov", ".io", ".dev", ".ai", ".app", ".co", ".uk",
            ".de", ".jp", ".fr", ".au", ".ru", ".ch", ".it", ".nl", ".se", ".no", ".es", ".mil",
            ".info", ".biz", ".me", ".tv", ".cc", ".rs",
        ];
        let lower = clean.to_lowercase();
        if suffixes.iter().any(|suf| lower.ends_with(suf)) {
            return true;
        }
    }
    false
}

/// Normalizes a candidate URL string to include https:// if protocol was omitted.
pub fn normalize_url(s: &str) -> String {
    let clean = s.trim().trim_matches(|c: char| {
        c == ','
            || c == ';'
            || c == '('
            || c == ')'
            || c == '['
            || c == ']'
            || c == '<'
            || c == '>'
            || c == '\''
            || c == '"'
    });
    if !clean.starts_with("http://") && !clean.starts_with("https://") {
        format!("https://{clean}")
    } else {
        clean.to_string()
    }
}

/// Extracts a target URL and remaining query text from an arbitrary prompt,
/// regardless of whether the URL was placed at the start, middle, or end.
/// Returns `(Some(url), remaining_query)` if a URL was detected, or `(None, original_trimmed)` if no URL was present.
#[allow(dead_code)]
pub fn extract_url_and_query(input: &str) -> (Option<String>, String) {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.is_empty() {
        return (None, String::new());
    }

    let mut url_idx = None;
    for (i, tok) in tokens.iter().enumerate() {
        if is_url_candidate(tok) {
            url_idx = Some(i);
            break;
        }
    }

    let url_pos = match url_idx {
        Some(idx) => idx,
        None => return (None, input.trim().to_string()),
    };

    let target_url = normalize_url(tokens[url_pos]);

    // Build remaining query, scrubbing connector words immediately preceding or succeeding the URL
    let mut remaining = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        if i == url_pos {
            continue;
        }
        let lower = tok.to_lowercase();
        if i + 1 == url_pos && (lower == "from" || lower == "at" || lower == "on" || lower == "in")
        {
            continue;
        }
        if i == url_pos + 1 && (lower == "for" || lower == "to") {
            continue;
        }
        remaining.push(*tok);
    }

    (Some(target_url), remaining.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_api_command() {
        let cmd = SlashCommand::parse("/api products").unwrap().unwrap();
        assert_eq!(
            cmd,
            SlashCommand::Api {
                table: Some("products".into()),
                port: 8080
            }
        );

        let cmd_port = SlashCommand::parse("/api users 3000").unwrap().unwrap();
        assert_eq!(
            cmd_port,
            SlashCommand::Api {
                table: Some("users".into()),
                port: 3000
            }
        );

        let cmd_empty = SlashCommand::parse("/api").unwrap().unwrap();
        assert_eq!(
            cmd_empty,
            SlashCommand::Api {
                table: None,
                port: 8080
            }
        );
    }

    #[test]
    fn test_parse_export_command() {
        let cmd = SlashCommand::parse("/export logs parquet")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd,
            SlashCommand::Export {
                table: Some("logs".into()),
                format: Some(ExportFormat::Parquet)
            }
        );

        let cmd_empty = SlashCommand::parse("/export").unwrap().unwrap();
        assert_eq!(
            cmd_empty,
            SlashCommand::Export {
                table: None,
                format: None
            }
        );
    }

    #[test]
    fn test_parse_stealth_and_system_commands() {
        assert_eq!(
            SlashCommand::parse("/stealth ultra").unwrap().unwrap(),
            SlashCommand::Stealth {
                mode: Some(StealthMode::Ultra)
            }
        );
        assert_eq!(
            SlashCommand::parse("/stealth").unwrap().unwrap(),
            SlashCommand::Stealth { mode: None }
        );
        assert_eq!(
            SlashCommand::parse("/tables").unwrap().unwrap(),
            SlashCommand::Tables { table: None }
        );
        assert_eq!(
            SlashCommand::parse("/tables products").unwrap().unwrap(),
            SlashCommand::Tables {
                table: Some("products".into())
            }
        );
        assert_eq!(
            SlashCommand::parse("/extract stories").unwrap().unwrap(),
            SlashCommand::Extract {
                table: Some("stories".into())
            }
        );
        assert_eq!(
            SlashCommand::parse("/tab data").unwrap().unwrap(),
            SlashCommand::Tab {
                tab: Some("data".into())
            }
        );
        assert_eq!(
            SlashCommand::parse("/status").unwrap().unwrap(),
            SlashCommand::Status
        );
        assert_eq!(
            SlashCommand::parse("/help").unwrap().unwrap(),
            SlashCommand::Help
        );
        assert_eq!(
            SlashCommand::parse("/?").unwrap().unwrap(),
            SlashCommand::Help
        );
        assert_eq!(
            SlashCommand::parse("/undo").unwrap().unwrap(),
            SlashCommand::Undo
        );
        assert_eq!(
            SlashCommand::parse("/clear").unwrap().unwrap(),
            SlashCommand::Clear
        );
        assert_eq!(
            SlashCommand::parse("/quit").unwrap().unwrap(),
            SlashCommand::Quit
        );
        assert_eq!(
            SlashCommand::parse("/exit").unwrap().unwrap(),
            SlashCommand::Quit
        );
    }

    #[test]
    fn test_non_slash_input_returns_none() {
        let res = SlashCommand::parse("search for cheap flights to Tokyo").unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn test_invalid_command_errors() {
        assert!(SlashCommand::parse("/unknown").is_err());
        assert!(SlashCommand::parse("/export logs invalid_fmt").is_err());
        assert!(SlashCommand::parse("/stealth invalid").is_err());
        assert!(SlashCommand::parse("/api table not_a_port").is_err());
    }

    #[test]
    fn test_parse_scroll_command() {
        let cmd = SlashCommand::parse("/scroll up 10").unwrap().unwrap();
        assert_eq!(
            cmd,
            SlashCommand::Scroll {
                direction: Some("up".into()),
                count: Some(10)
            }
        );

        let cmd_default = SlashCommand::parse("/scroll").unwrap().unwrap();
        assert_eq!(
            cmd_default,
            SlashCommand::Scroll {
                direction: None,
                count: None
            }
        );
    }

    #[test]
    fn test_parse_crawl_command() {
        let cmd = SlashCommand::parse("/crawl https://github.com/rust-lang/rust rust_repo")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd,
            SlashCommand::Crawl {
                seed_url: "https://github.com/rust-lang/rust".into(),
                table: Some("rust_repo".into()),
                restrict_path: true,
                ai_goal: None,
                attributes: vec![],
            }
        );

        let cmd_all = SlashCommand::parse("/crawl https://news.ycombinator.com hn --all-paths")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd_all,
            SlashCommand::Crawl {
                seed_url: "https://news.ycombinator.com".into(),
                table: Some("hn".into()),
                restrict_path: false,
                ai_goal: None,
                attributes: vec![],
            }
        );

        // Flexible sequence: query first, url second
        let cmd_query_first = SlashCommand::parse(
            "/crawl extract all release tags from https://github.com/rust-lang/rust --all-paths",
        )
        .unwrap()
        .unwrap();
        match cmd_query_first {
            SlashCommand::Crawl {
                seed_url,
                restrict_path,
                ai_goal,
                attributes,
                ..
            } => {
                assert_eq!(seed_url, "https://github.com/rust-lang/rust");
                assert!(!restrict_path);
                assert_eq!(ai_goal, Some("extract all release tags".to_string()));
                assert!(attributes.is_empty());
            }
            other => panic!("expected Crawl, got {:?}", other),
        }

        // Bare domain
        let cmd_bare =
            SlashCommand::parse("/crawl github.com/rust-lang/rust extract pull requests")
                .unwrap()
                .unwrap();
        match cmd_bare {
            SlashCommand::Crawl {
                seed_url, ai_goal, ..
            } => {
                assert_eq!(seed_url, "https://github.com/rust-lang/rust");
                assert_eq!(ai_goal, Some("extract pull requests".to_string()));
            }
            other => panic!("expected Crawl, got {:?}", other),
        }

        // Target attributes flag
        let cmd_attrs = SlashCommand::parse(
            "/crawl https://books.toscrape.com books --attributes title,price,rating",
        )
        .unwrap()
        .unwrap();
        match cmd_attrs {
            SlashCommand::Crawl {
                seed_url,
                table,
                attributes,
                ..
            } => {
                assert_eq!(seed_url, "https://books.toscrape.com");
                assert_eq!(table, Some("books".into()));
                assert_eq!(
                    attributes,
                    vec![
                        "title".to_string(),
                        "price".to_string(),
                        "rating".to_string()
                    ]
                );
            }
            other => panic!("expected Crawl, got {:?}", other),
        }

        // Natural language attribute extraction
        let cmd_nl = SlashCommand::parse(
            "/crawl https://books.toscrape.com extract only title, price, and rating",
        )
        .unwrap()
        .unwrap();
        match cmd_nl {
            SlashCommand::Crawl {
                seed_url,
                attributes,
                ai_goal,
                ..
            } => {
                assert_eq!(seed_url, "https://books.toscrape.com");
                assert_eq!(
                    attributes,
                    vec![
                        "title".to_string(),
                        "price".to_string(),
                        "rating".to_string()
                    ]
                );
                assert_eq!(
                    ai_goal,
                    Some("extract only title, price, and rating".to_string())
                );
            }
            other => panic!("expected Crawl, got {:?}", other),
        }

        // Stop commands
        assert_eq!(
            SlashCommand::parse("/stop").unwrap().unwrap(),
            SlashCommand::Stop
        );
        assert_eq!(
            SlashCommand::parse("/cancel").unwrap().unwrap(),
            SlashCommand::Stop
        );
        assert_eq!(
            SlashCommand::parse("/crawl stop").unwrap().unwrap(),
            SlashCommand::Stop
        );
    }

    #[test]
    fn test_extract_url_and_query() {
        // URL first
        let (u1, q1) =
            extract_url_and_query("https://github.com/rust-lang/rust extract all release tags");
        assert_eq!(u1, Some("https://github.com/rust-lang/rust".into()));
        assert_eq!(q1, "extract all release tags");

        // Query first
        let (u2, q2) = extract_url_and_query(
            "extract all release tags from https://github.com/rust-lang/rust",
        );
        assert_eq!(u2, Some("https://github.com/rust-lang/rust".into()));
        assert_eq!(q2, "extract all release tags");

        // Bare URL without scheme
        let (u3, q3) = extract_url_and_query("github.com/rust-lang/cargo pull requests");
        assert_eq!(u3, Some("https://github.com/rust-lang/cargo".into()));
        assert_eq!(q3, "pull requests");

        // URL only
        let (u4, q4) = extract_url_and_query("https://news.ycombinator.com");
        assert_eq!(u4, Some("https://news.ycombinator.com".into()));
        assert_eq!(q4, "");

        // No URL
        let (u5, q5) = extract_url_and_query("search for rust 2024 features");
        assert_eq!(u5, None);
        assert_eq!(q5, "search for rust 2024 features");
    }
}
