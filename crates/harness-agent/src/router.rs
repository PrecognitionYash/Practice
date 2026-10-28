// ──────────────────────────────────────────────────────────────────────
// harness-agent/src/router.rs — Autonomous AI Tool Router & Intent Dispatcher
//
// Equips the autonomous agent with a full suite of system tools:
//  • fast_crawler   — High-throughput recursive crawler with target attribute filters
//  • browser_agent  — Interactive CDP browser automation for live page actions
//  • web_search     — Zero-URL authoritative endpoint resolution
//  • query_data     — SQLite WAL data inspection & table previews
//  • export_data    — Parquet & CSV file exporter
//  • session_vault  — Cryptographic authenticated credential manager
//  • stop_execution — Instant graceful task cancellation
//  • reject_task    — Pre-flight capability boundary enforcement
//
// Resolves arbitrary natural language queries into specific tools without
// relying on brittle syntax or keywords.
// ──────────────────────────────────────────────────────────────────────

use harness_core::error::{HarnessError, HarnessResult};
use harness_core::schema::HarnessToolCall;
use tracing::{debug, info, instrument};

use crate::groq::{extract_json_str, ChatMessage, GroqClient};

/// Autonomous Tool Calling & Intent Dispatcher.
#[derive(Clone)]
pub struct ToolRouter {
    groq: GroqClient,
}

impl ToolRouter {
    /// Create a new tool router with the given Groq client.
    pub fn new(groq: GroqClient) -> Self {
        Self { groq }
    }

    /// Evaluates any natural-language prompt or command and resolves it into a strongly-typed tool call.
    #[instrument(skip(self, prompt), fields(prompt = %prompt))]
    pub async fn resolve_tool_call(&self, prompt: &str) -> HarnessResult<HarnessToolCall> {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            return Err(HarnessError::InvalidArgs {
                command: "prompt".into(),
                reason: "prompt cannot be empty".into(),
            });
        }

        // Fast rejection invariant check (safety envelope)
        let lower = trimmed.to_lowercase();
        if lower.contains("wire transfer")
            || lower.contains("bank transfer")
            || lower.contains("audio captcha")
            || lower.contains("credit card fraud")
        {
            return Ok(HarnessToolCall::RejectTask {
                reason:
                    "Financial transactions and CAPTCHA bypass exceed operational safety envelope."
                        .into(),
            });
        }

        // If GROQ_API_KEY is available, invoke AI Tool Calling via LLM
        if self.groq.has_api_key() {
            match self.llm_tool_calling(trimmed).await {
                Ok(tool_call) => {
                    info!(tool = ?tool_call, "AI successfully selected tool via Groq LLM");
                    return Ok(tool_call);
                }
                Err(err) => {
                    debug!(error = %err, "Groq LLM tool calling failed; falling back to heuristic router");
                }
            }
        }

        // Offline / Fast-path Heuristic Tool Routing Fallback
        let heuristic_call = Self::heuristic_tool_routing(trimmed);
        info!(tool = ?heuristic_call, "Resolved tool call via autonomous heuristic router");
        Ok(heuristic_call)
    }

    /// High-speed LLM Tool Calling over Groq with strict JSON output.
    async fn llm_tool_calling(&self, prompt: &str) -> HarnessResult<HarnessToolCall> {
        let system_prompt = r#"You are the Autonomous Tool Calling & Intent Router for the Industrial Browser Harness.
Analyze the user's natural language request and select the single most appropriate tool from the tool registry.

AVAILABLE TOOLS:
1. "fast_crawler":
   Use when the user wants to crawl, scrape, search & extract, list items, or harvest data from a site/service.
   Especially when the user asks to "go to <site> and search/find <query> and list/extract <attributes>".
   Parameters:
     "url": Authoritative URL (e.g. for "amazon toys" -> "https://www.amazon.com/s?k=toys", for "github rust" -> "https://github.com/rust-lang/rust")
     "table": Short SQLite table name (e.g. "amazon_toys", "hn_stories")
     "attributes": Array of strings of ONLY requested attributes (e.g. ["title", "price"], ["rating", "author"]). Empty if all.
     "ai_goal": Descriptive extraction objective (e.g. "search for toys and extract product name and price")
     "max_pages": Integer limit (default 50, 0 for unbounded)
     "restrict_path": Boolean

2. "browser_agent":
   Use for interactive live browser automation (CDP): logging in, clicking buttons, filling multi-step dynamic forms, visual UI testing.
   Parameters:
     "url": Target URL
     "goal": Action goal
     "search_query": Optional search text
     "attributes": Array of attributes

3. "query_data":
   Use when the user asks to view, inspect, or list database tables, previews, or collected rows (e.g. "show all tables", "preview table books").
   Parameters:
     "table": Optional table name
     "limit": Optional row limit (default 50)

4. "export_data":
   Use when user asks to export or save data into CSV or Parquet (e.g. "export table books to csv").
   Parameters:
     "table": Target table name
     "format": "csv" or "parquet"

5. "session_vault":
   Use when user asks about saved sessions, cookies, or credentials (e.g. "vault list", "check login session for amazon").
   Parameters:
     "action": "list" | "check" | "load"
     "domain": Optional domain string

6. "stop_execution":
   Use when user asks to stop, halt, abort, or cancel active tasks.
   Parameters: {}

7. "reject_task":
   Use for out-of-scope requests (wire transfers, audio CAPTCHAs, illicit tasks).
   Parameters:
     "reason": String

Respond ONLY with raw JSON object strictly adhering to this schema:
{
  "tool": "fast_crawler" | "browser_agent" | "query_data" | "export_data" | "session_vault" | "stop_execution" | "reject_task",
  "parameters": { ... }
}
Zero prose. Zero markdown."#;

        let messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(prompt),
        ];

        let raw_json = self.groq.chat_completion(&messages, 0.0, 512, true).await?;
        let cleaned = extract_json_str(&raw_json);
        let tool_call: HarnessToolCall =
            serde_json::from_str(cleaned).map_err(HarnessError::Json)?;

        Ok(tool_call)
    }

    /// Autonomous heuristic tool router for zero-latency, offline, or fallback query resolution.
    pub fn heuristic_tool_routing(prompt: &str) -> HarnessToolCall {
        let trimmed = prompt.trim();
        let lower = trimmed.to_lowercase();

        // 1. Stop / Cancel commands
        if lower == "stop"
            || lower == "/stop"
            || lower == "cancel"
            || lower == "halt"
            || lower.contains("stop crawl")
            || lower.contains("cancel crawl")
            || lower.contains("cancel the crawl")
            || lower.contains("stop the crawl")
            || lower.starts_with("abort")
        {
            return HarnessToolCall::StopExecution;
        }

        // 2. Query / List Tables
        if lower.starts_with("/tables")
            || lower == "show tables"
            || lower == "list tables"
            || lower == "tables"
            || lower.contains("show all tables")
            || lower.contains("what tables")
            || lower.contains("show harvested tables")
        {
            let table = if lower.starts_with("/tables ") {
                Some(trimmed["/tables ".len()..].trim().to_string())
            } else {
                None
            };
            return HarnessToolCall::QueryData {
                table,
                limit: Some(50),
            };
        }

        // 3. Export Data
        if lower.starts_with("/export")
            || lower.starts_with("export ")
            || lower.contains("save as csv")
            || lower.contains("export table")
        {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            let mut table_name = "data".to_string();
            let mut format = "csv".to_string();

            for (i, p) in parts.iter().enumerate() {
                let p_low = p.to_lowercase();
                if p_low == "parquet" || p_low == "csv" {
                    format = p_low;
                } else if i > 0
                    && !p.starts_with('/')
                    && p_low != "table"
                    && p_low != "to"
                    && p_low != "as"
                    && p_low != "export"
                {
                    table_name = p.to_string();
                }
            }

            return HarnessToolCall::ExportData {
                table: table_name,
                format,
            };
        }

        // 4. Session Vault
        if lower.starts_with("/vault")
            || lower.starts_with("vault ")
            || lower.contains("session cookies")
        {
            let mut action = "list".to_string();
            let mut domain = None;
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            for p in &parts {
                let p_low = p.to_lowercase();
                if p_low == "list" || p_low == "check" || p_low == "load" {
                    action = p_low;
                } else if p.contains('.') {
                    domain = Some(p.to_string());
                }
            }
            return HarnessToolCall::SessionVault { action, domain };
        }

        // 5. Detect Brand / Service / Domain
        let (base_domain, direct_search_pattern) = Self::detect_service_or_domain(&lower);

        // 6. Detect Search Term (supporting typos like "serach", "seach", "search for toys")
        let search_query = Self::extract_search_query_heuristic(&lower);

        // 7. Detect Target Attributes (e.g. "price and product name only", "price, rating")
        let target_attributes = Self::extract_target_attributes_heuristic(&lower);

        // 8. Construct authoritative URL
        let encode_param =
            |s: &str| -> String { url::form_urlencoded::byte_serialize(s.as_bytes()).collect() };

        let resolved_url = if let Some(ref brand_url) = base_domain {
            if let Some(ref query) = search_query {
                if let Some(pattern) = direct_search_pattern {
                    let enc = encode_param(query);
                    pattern.replace("{query}", &enc)
                } else {
                    format!("{brand_url}/search?q={}", encode_param(query))
                }
            } else {
                brand_url.clone()
            }
        } else if let Some((u, _)) = Self::find_candidate_url(trimmed) {
            u
        } else if let Some(ref query) = search_query {
            format!("https://duckduckgo.com/html/?q={}", encode_param(query))
        } else {
            format!("https://duckduckgo.com/html/?q={}", encode_param(trimmed))
        };

        // 9. Derive clean table name
        let target_table = Self::derive_table_from_intent(&resolved_url, search_query.as_deref());

        // 10. Check if user asked for interactive CDP action (clicks, form submission)
        let is_browser_agent = lower.contains("click on")
            || lower.contains("type into")
            || lower.contains("fill form")
            || lower.contains("log into")
            || lower.contains("sign in to");

        if is_browser_agent {
            HarnessToolCall::BrowserAgent {
                url: resolved_url,
                goal: trimmed.to_string(),
                search_query,
                attributes: target_attributes,
            }
        } else {
            HarnessToolCall::FastCrawler {
                url: resolved_url,
                table: Some(target_table),
                attributes: target_attributes,
                ai_goal: Some(trimmed.to_string()),
                max_pages: Some(50),
                restrict_path: false,
            }
        }
    }

    /// Detects well-known brands, services, or domain names in the user's prompt.
    fn detect_service_or_domain(lower: &str) -> (Option<String>, Option<&'static str>) {
        if lower.contains("amazon") {
            (
                Some("https://www.amazon.com".into()),
                Some("https://www.amazon.com/s?k={query}"),
            )
        } else if lower.contains("github") {
            (
                Some("https://github.com".into()),
                Some("https://github.com/search?q={query}"),
            )
        } else if lower.contains("wikipedia") {
            (
                Some("https://en.wikipedia.org".into()),
                Some("https://en.wikipedia.org/w/index.php?search={query}"),
            )
        } else if lower.contains("reddit") {
            (
                Some("https://www.reddit.com".into()),
                Some("https://www.reddit.com/search/?q={query}"),
            )
        } else if lower.contains("ebay") {
            (
                Some("https://www.ebay.com".into()),
                Some("https://www.ebay.com/sch/i.html?_nkw={query}"),
            )
        } else if lower.contains("walmart") {
            (
                Some("https://www.walmart.com".into()),
                Some("https://www.walmart.com/search?q={query}"),
            )
        } else if lower.contains("youtube") {
            (
                Some("https://www.youtube.com".into()),
                Some("https://www.youtube.com/results?search_query={query}"),
            )
        } else if lower.contains("books.toscrape") {
            (Some("https://books.toscrape.com".into()), None)
        } else if lower.contains("news.ycombinator")
            || lower.contains("hacker news")
            || lower.contains("hackernews")
        {
            (Some("https://news.ycombinator.com".into()), None)
        } else {
            (None, None)
        }
    }

    /// Extracts a search query keyword phrase from expressions like "search for toys", "serach for X", "find X".
    fn extract_search_query_heuristic(lower: &str) -> Option<String> {
        let triggers = [
            "search for ",
            "serach for ",
            "seach for ",
            "search ",
            "serach ",
            "find ",
            "look for ",
            "query for ",
            "lookup ",
        ];

        for trig in &triggers {
            if let Some(pos) = lower.find(trig) {
                let remainder = &lower[pos + trig.len()..];
                // Strip trailing attribute requests like "and list down price..."
                let stop_markers = [
                    " and list",
                    " and extract",
                    " and get",
                    " and show",
                    " only",
                    " with price",
                ];
                let mut clean_query = remainder;
                for marker in &stop_markers {
                    if let Some(stop_pos) = clean_query.find(marker) {
                        clean_query = &clean_query[..stop_pos];
                    }
                }
                let q = clean_query.trim();
                if !q.is_empty() {
                    return Some(q.to_string());
                }
            }
        }
        None
    }

    /// Extracts target attributes from natural language queries like:
    /// "list down price and product name only", "extract title, rating and price"
    fn extract_target_attributes_heuristic(lower: &str) -> Vec<String> {
        let mut attributes = Vec::new();

        // Check common semantic attribute targets
        let has_price = lower.contains("price") || lower.contains("cost");
        let has_product_name = lower.contains("product name")
            || lower.contains("item name")
            || lower.contains("title")
            || lower.contains("name only");
        let has_rating =
            lower.contains("rating") || lower.contains("stars") || lower.contains("score");
        let has_author = lower.contains("author") || lower.contains("byline");
        let has_date = lower.contains("date") || lower.contains("published");
        let has_url = lower.contains("link") || lower.contains("url");
        let has_review = lower.contains("review");

        if has_product_name {
            attributes.push("title".to_string());
        }
        if has_price {
            attributes.push("price".to_string());
        }
        if has_rating {
            attributes.push("rating".to_string());
        }
        if has_author {
            attributes.push("author".to_string());
        }
        if has_date {
            attributes.push("date".to_string());
        }
        if has_url && !attributes.contains(&"url".to_string()) {
            attributes.push("url".to_string());
        }
        if has_review {
            attributes.push("reviews".to_string());
        }

        // Check explicit `--attributes` or "attributes: a, b" syntax
        if let Some(pos) = lower.find("--attributes") {
            let sub = &lower[pos + "--attributes".len()..].trim();
            if let Some(first_word) = sub.split_whitespace().next() {
                for attr in first_word.split(',') {
                    let clean = attr.trim();
                    if !clean.is_empty() && !attributes.contains(&clean.to_string()) {
                        attributes.push(clean.to_string());
                    }
                }
            }
        }

        attributes
    }

    /// Finds any candidate URL in the prompt.
    fn find_candidate_url(prompt: &str) -> Option<(String, String)> {
        for word in prompt.split_whitespace() {
            let clean = word.trim_matches(|c: char| {
                c == ',' || c == ';' || c == '(' || c == ')' || c == '\'' || c == '"'
            });
            if clean.starts_with("http://") || clean.starts_with("https://") {
                return Some((clean.to_string(), clean.to_string()));
            }
            if clean.contains('.') && !clean.ends_with('.') && !clean.starts_with('.') {
                let suffixes = [
                    ".com", ".org", ".net", ".io", ".dev", ".ai", ".app", ".co", ".edu",
                ];
                if suffixes
                    .iter()
                    .any(|suf| clean.to_lowercase().ends_with(suf))
                {
                    return Some((format!("https://{clean}"), clean.to_string()));
                }
            }
        }
        None
    }

    /// Derives a clean SQLite table name from destination URL and search query.
    fn derive_table_from_intent(url: &str, query: Option<&str>) -> String {
        let domain = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| "data".to_string());

        let clean_domain = domain.replace(['.', '-'], "_");
        if let Some(q) = query {
            let clean_q: String = q
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .take(20)
                .collect();
            format!("{clean_domain}_{clean_q}")
        } else {
            format!("{clean_domain}_items")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heuristic_tool_routing_amazon_toys() {
        let prompt = "go to amazon and serach for toys and list down price and product name only";
        let tool_call = ToolRouter::heuristic_tool_routing(prompt);

        match tool_call {
            HarnessToolCall::FastCrawler {
                url,
                table,
                attributes,
                ai_goal,
                ..
            } => {
                assert!(url.contains("amazon.com/s?k=toys"));
                assert!(table.unwrap().contains("amazon"));
                assert!(attributes.contains(&"price".to_string()));
                assert!(attributes.contains(&"title".to_string()));
                assert!(ai_goal.is_some());
            }
            other => panic!("expected FastCrawler, got {:?}", other),
        }
    }

    #[test]
    fn test_heuristic_tool_routing_stop() {
        assert_eq!(
            ToolRouter::heuristic_tool_routing("stop"),
            HarnessToolCall::StopExecution
        );
        assert_eq!(
            ToolRouter::heuristic_tool_routing("cancel the crawl right now"),
            HarnessToolCall::StopExecution
        );
    }

    #[test]
    fn test_heuristic_tool_routing_tables() {
        match ToolRouter::heuristic_tool_routing("show all tables in sqlite") {
            HarnessToolCall::QueryData { table, limit } => {
                assert_eq!(table, None);
                assert_eq!(limit, Some(50));
            }
            other => panic!("expected QueryData, got {:?}", other),
        }
    }

    #[test]
    fn test_heuristic_tool_routing_export() {
        match ToolRouter::heuristic_tool_routing("export amazon_toys to csv") {
            HarnessToolCall::ExportData { table, format } => {
                assert_eq!(table, "amazon_toys");
                assert_eq!(format, "csv");
            }
            other => panic!("expected ExportData, got {:?}", other),
        }
    }

    #[test]
    fn test_heuristic_tool_routing_books_crawl() {
        let prompt = "crawl https://books.toscrape.com and extract title, rating and price";
        let tool_call = ToolRouter::heuristic_tool_routing(prompt);
        match tool_call {
            HarnessToolCall::FastCrawler {
                url, attributes, ..
            } => {
                assert_eq!(url, "https://books.toscrape.com");
                assert!(attributes.contains(&"title".to_string()));
                assert!(attributes.contains(&"price".to_string()));
                assert!(attributes.contains(&"rating".to_string()));
            }
            other => panic!("expected FastCrawler, got {:?}", other),
        }
    }
}
