# High-Performance Autonomous Browser Harness
### Industrial-Grade Native Rust Web Agent Framework with Groq AI Planning & Python SDK

[![CI/CD Multi-Arch Release](https://github.com/yash/harness_browser/actions/workflows/release.yml/badge.svg)](https://github.com/yash/harness_browser/actions/workflows/release.yml)
[![Rust Version](https://img.shields.io/badge/rust-2021%20edition-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Groq Llama-3.3-70B](https://img.shields.io/badge/Groq-Llama--3.3--70B-violet.svg)](https://groq.com/)
[![Render Cadence](https://img.shields.io/badge/TUI-60%20FPS%20Double--Buffered-green.svg)](https://github.com/ratatui/ratatui)
[![Python SDK](https://img.shields.io/badge/Python-3.9%20%7C%203.10%20%7C%203.11%20%7C%203.12-blue.svg)](https://pypi.org/)

---

## 1. Executive Summary & Core Engineering Philosophy

Commercial browser automation agents frequently fail in production because they pass multi-megabyte Document Object Model (DOM) snapshots or high-resolution full-page screenshots into multi-modal models for every micro-interaction. Under strict operational constraints—specifically **Groq's 8,192-token context window ceiling**—this naive approach causes immediate context exhaustion, severe latency (>5 seconds per interaction), and compounding operational failure.

```text
┌──────────────────────────────────────────────────────────────────────────────────┐
│                             SYSTEM DESIGN INVARIANT                              │
│                                                                                  │
│   "All mechanical execution, DOM parsing, network interception, heuristic        │
│    traversal, and terminal rendering execute deterministically in native Rust    │
│    at bare-metal speed. The LLM is an isolated strategic supervisor invoked      │
│    exclusively for intent triage, high-level planning, and bounded repair."      │
└──────────────────────────────────────────────────────────────────────────────────┘
```

### Key Performance Targets

* **Perception Token Footprint:** Dynamic compression of $100\text{ KB} - 5\text{ MB}$ web pages down to $\le 1,200\text{ tokens}$ ($\approx 95\%$ reduction) via Set-of-Marks AXTree pruning.
* **Context Budget Preservation:** Guarantees total agent execution footprint measures $\le 3,424\text{ tokens}$ ($\le 42\%$ of Groq's 8,192 limit), preserving $>4,768\text{ tokens}$ for multi-turn conversational resilience.
* **Local Deterministic Latency:** Sub-$5\text{ ms}$ element resolution and synthetic event dispatch via native Chrome DevTools Protocol (CDP) sockets.
* **Zero-DOM Network Sniffing:** Backend `application/json` API streams intercepted directly from memory into SQLite WAL in $<200\text{ ms}$.
* **FastPath Multi-Page Crawler:** $\ge 150\text{ pages/second}$ on static leaf nodes via cookie-migrated asynchronous HTTP/2 connection pooling with $O(1)$ `xxHash64` cycle detection and strict same-domain boundary guards.
* **Glitch-Free Interface:** Double-buffered ANSI diffing locked at a constant $60\text{ FPS}$ with $\le 1\%$ idle CPU utilization.

---

## 2. Autonomous AI Tool Registry & Intent Router

The system features an **Autonomous AI Tool Calling & Dynamic Intent Router**. Instead of relying on brittle keyword matching or forcing users to type strict slash commands, the AI is equipped with a structured suite of native system tools:

```text
                               ┌────────────────────────────────┐
                               │   Arbitrary User Prompt        │
                               │   (Phylogeny, typos, syntax)   │
                               └───────────────┬────────────────┘
                                               │
                                               ▼
                              ┌──────────────────────────────────┐
                              │  AI Intent & Tool Router         │
                              │  (Groq LLM / Heuristic Engine)   │
                              └────────────────┬─────────────────┘
                                               │
         ┌──────────────┬──────────────┬───────┴──────┬──────────────┬──────────────┐
         ▼              ▼              ▼              ▼              ▼              ▼
  ┌──────────────┐┌──────────────┐┌──────────┐ ┌──────────┐ ┌─────────────┐ ┌──────────────┐
  │ BrowserAgent ││ FastCrawler  ││QueryData │ │ExportData│ │SessionVault │ │StopExecution │
  │ (CDP Stealth)││(Recursive)   ││(SQLite)  │ │(Parquet) │ │(AES-256-GCM)│ │ (Halt Tasks) │
  └──────────────┘└──────────────┘└──────────┘ └──────────┘ └─────────────┘ └──────────────┘
```

### Available Native Tools

| Tool | Action Dispatched | Description |
| :--- | :--- | :--- |
| **`BrowserAgent`** | Live CDP Chrome Automation | Navigates complex, dynamic websites (Amazon, Twitter, GitHub) using Ultra Stealth fingerprint masking, Set-of-Marks perception, and interactive actuation. |
| **`FastCrawler`** | Multi-Page Spidering | High-speed, multi-page recursive crawling across a domain with noise rejection and target attribute filtering. |
| **`QueryData`** | Database Inspection | Displays, inspects, and previews harvested SQLite tables and row counts directly in the TUI. |
| **`ExportData`** | Dataset Export | Streams harvested records into Apache Parquet (Snappy-compressed Arrow), CSV, or JSONL. |
| **`SessionVault`** | Cryptographic Auth Store | Inspects, saves, or loads authenticated session cookies encrypted with domain-isolated AES-256-GCM keys. |
| **`StopExecution`** | Immediate Task Cancellation | Halts running crawlers or browser loops, immediately draining collected records into SQLite WAL. |
| **`WebSearch`** | Zero-URL Fast Resolver | Lightweight HTTP/2 search engine query resolution when no explicit domain is supplied. |
| **`RejectTask`** | Rejection Invariant Guard | Deterministically rejects requests outside the capability envelope (e.g., CAPTCHAs, wire transfers). |

### Example: Natural Phrasing with Typo Tolerance & Strict Attribute Filtering

```text
[harness] > go to amazon and serach for toys and list down price and product name only
```

1. **Autonomous Tool Selection**: The router automatically selects `BrowserAgent` (interactive CDP).
2. **Typo & Endpoint Resolution**: Recognizes the brand `amazon` and typo `"serach"` $\to$ maps directly to `https://www.amazon.com/s?k=toys`.
3. **Attribute Projection**: Recognizes `"price and product name only"` $\to$ parses `target_attributes = ["title", "price"]`.
4. **Stealth Extraction**: Navigates with Ultra Stealth CDP masking, harvests Amazon search cards (`div[data-component-type="s-search-result"]`), filters out all fields outside `title` and `price`, and streams rows into SQLite table `amazon_toys`.
5. **Instant View Switching**: Automatically switches the TUI to `[1] Extracted Data` previewing the clean dataset.

---

## 3. Quickstart & Installation

### Prerequisites
* **Rust**: Stable toolchain (1.80+) — `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
* **Chromium**: Any installed Google Chrome or Chromium browser
* **Python** (optional): Python 3.9+ for the Python SDK
* **Groq API Key**: Obtain from [console.groq.com/keys](https://console.groq.com/keys)

### Step 1: Configure Environment (.env)
```bash
cp .env.example .env
```
Edit `.env`:
```env
# [REQUIRED for online Groq AI planning]
GROQ_API_KEY=gsk_your_actual_groq_api_key_here

# [OPTIONAL: Execution parameters]
HARNESS_HEADLESS=true
HARNESS_STEALTH=ultra
HARNESS_DB_PATH=.harness/harvest.db
HARNESS_VAULT_KEY=custom-session-encryption-seed-32
```

### Step 2: Build the Workspace
```bash
# Build both ./target/debug/harness and ./target/debug/harness-tui
cargo build

# Or build optimized release binaries
make release
```

---

## 4. Running the Harness

### 4.1 Interactive 60 FPS Terminal UI (TUI)
Launch the double-buffered TUI in interactive mode:
```bash
./harness
# or
./target/debug/harness
```

### 4.2 Running Autonomous Prompts Directly from Terminal
You can run any natural language prompt directly from your shell without opening the TUI first:
```bash
# Extract GitHub trending repositories
./target/debug/harness "Extract top trending repositories on github.com"

# Amazon search with target attribute projection
./target/debug/harness "go to amazon and serach for toys and list down price and product name only"

# HackerNews front page extraction
./target/debug/harness "Extract top stories on news.ycombinator.com"

# Recursive multi-page crawl
./target/debug/harness "/crawl https://books.toscrape.com --max-pages 20 extract only title, price, rating"
```

---

## 5. Interactive TUI Navigation & Hotkeys

The terminal UI runs at a double-buffered 60 FPS with differential ANSI rendering:

```text
┌─ AUTONOMOUS BROWSER HARNESS [Industrial Edition] ────────────────────────────────────────┐
│ Status: IDLE   Stealth: Ultra   URL: https://www.amazon.com/s?k=toys   1,120/7,000 Tokens│
├──────────────────────────────────────────────────────────────────────────────────────────┤
│  [1] Extracted Data   │   [2] Perception Frame   │   [3] Execution Log                   │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│  Showing 1-15 of 24 records from 'amazon_toys'                                           │
│  #   │ PRODUCT_NAME                                             │ PRICE                  │
│  1   │ LEGO Star Wars Millennium Falcon Building Kit           │ $159.99                │
│  2   │ Hot Wheels 10-Pack Toy Cars Variety Pack                │ $12.99                 │
│  3   │ Play-Doh Modeling Compound 24-Pack Variety              │ $21.99                 │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│ Command Buffer [/api, /export, /tables, /extract, /reset, /tab, /quit | Press 1/2/3]     │
│ [harness] > █                                                                            │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

### Hotkeys & Viewport Controls

| Key | Action | Description |
| :--- | :--- | :--- |
| **`1`** or **`F1`** | Tab 1: Extracted Data | View harvested SQLite dataset tables, row records, and schema columns. |
| **`2`** or **`F2`** | Tab 2: Perception Frame | View live Set-of-Marks AXTree layout and element bounding boxes. |
| **`3`** or **`F3`** | Tab 3: Execution Log | View live CDP, SQLite WAL, and agent thought trajectory logs. |
| **`Up` / `Down`** | Query History | Browse previously executed prompts and slash commands. |
| **`PageUp` / `PageDown`** | Viewport Scroll | Scroll the active tab view up or down by 5 units. |
| **`Tab`** | Auto-Complete | Cycle tab completion for slash commands and database table names. |
| **`ESC`** | Halt / Cancel | Immediately stops any active crawl, halts execution, or dismisses alerts. |

---

## 6. Built-in Slash Commands Reference

Slash commands execute bare-metal routines instantly without consuming LLM tokens. They work **inside the TUI** or **directly from the CLI** (e.g. `./target/debug/harness /tables`):

| Command | Syntax | Description |
| :--- | :--- | :--- |
| **`/help`** | `/help` | Displays the comprehensive command reference guide. |
| **`/status`** | `/status` | Displays real-time operational status, stealth profile, token load, and active REST API. |
| **`/tables`** | `/tables [table_name]` | Lists all harvested tables in SQLite WAL. If a table name is provided, switches to previewing it. |
| **`/extract`** | `/extract [table_name]` | Explicitly harvests structured records from the active browser page into SQLite on demand. |
| **`/export`** | `/export <table_name> <parquet\|csv\|jsonl>` | Exports database records to Apache Arrow Snappy Parquet, CSV, or JSONL under `.harness/exports/`. |
| **`/api`** | `/api <table_name> [port]` | Launches an embedded Axum REST microservice serving live JSON on `http://127.0.0.1:<port>/api/<table_name>` (default port: 8080). |
| **`/crawl`** | `/crawl <url> [options]` | Recursive fast crawler. Options: `--max-pages <n>`, `--depth <n>`, `--restrict-path`, `--attributes <a,b>`. |
| **`/stop`** | `/stop` | Signals active crawls or agent actions to halt and safely persist all extracted records. |
| **`/vault`** | `/vault [list\|<domain>\|save\|load]` | Cryptographic session vault for inspecting, saving, and loading authenticated cookies. |
| **`/stealth`** | `/stealth [fast\|ultra]` | Toggles or sets CDP stealth anti-detection profile (audio, WebGL, canvas spoofing). |
| **`/reset`** | `/reset` | Drops all user data tables, purges the macro replay cache, clears exports, and resets tokens. |
| **`/undo`** | `/undo` | Traverses internal DOM action stack to roll back the previous interaction. |
| **`/clear`** | `/clear` | Clears the live execution log buffer. |
| **`/tab`** | `/tab <data\|dom\|logs\|1\|2\|3>` | Switches display view tab. |
| **`/scroll`** | `/scroll <up\|down\|top\|bottom> [n]`| Scrolls the active tab view. |
| **`/quit`** | `/quit` (or `/exit`) | Performs clean RAII terminal restoration and terminates runtime. |

---

## 7. Python SDK & LlamaIndex Agent Integration

The harness provides a first-class Python SDK (`harness_browser`) with resilient fallbacks:

### Installation
```bash
# Editable install in your Python virtualenv
pip install -e .

# Or build native C-ABI PyO3 wheel via maturin
make py-build
# or: maturin develop --manifest-path crates/harness-bridge/Cargo.toml --release
```

### 7.1 Python SDK Usage Example

```python
from harness_browser import AutonomousBrowserHarness, SessionVault

harness = AutonomousBrowserHarness()

# 1. Pre-Flight Capability Triage (<80 tokens, no Chromium launch)
triage = harness.triage("Extract top machine learning repositories on github.com")
print("Triage:", triage)

# 2. Recursive Crawl with Target Attribute Projection
pages_crawled = harness.crawl(
    seed_url="https://books.toscrape.com",
    max_pages=20,
    attributes=["title", "price", "rating"],
    table_name="books_dataset",
    ai_goal="Extract book titles, prices, and star ratings",
)
print(f"Crawled {pages_crawled} pages.")

# 3. Querying Harvested SQLite WAL Records
rows = harness.query("books_dataset", limit=50)
for r in rows:
    print(r["payload"]["title"], r["payload"]["price"])

# 4. Multi-Format Dataset Export (CSV / Parquet / JSONL)
csv_file = harness.export("books_dataset", format="csv")
parquet_file = harness.export("books_dataset", format="parquet")
print(f"Exported CSV to: {csv_file}")
print(f"Exported Parquet to: {parquet_file}")

# 5. Encrypted Session Vault (AES-256-GCM)
vault = SessionVault(vault_dir=".harness/vault")
vault.save_session("amazon.com", [{"name": "session_id", "value": "sec_tok_123"}])
session = vault.load_session("amazon.com")
print("Vaulted cookies:", session["cookies"])
```

### 7.2 Binding Harness Tools into LlamaIndex / LangChain Agents

The harness exports ready-to-use typed `FunctionTool` instances for LLM agent frameworks:

```python
from harness_browser import get_harness_tools
from llama_index.core.agent import ReActAgent
from llama_index.llms.openai import OpenAI

# Construct harness agent tools
tools = get_harness_tools()

# Bind into LlamaIndex autonomous agent
agent = ReActAgent.from_tools(tools, llm=OpenAI(model="gpt-4o"))
response = agent.chat("Go to books.toscrape.com and extract top rated books with prices")
print(response)
```

---

## 8. Ready-to-Run Examples Directory

The repository includes a dedicated [`examples/`](examples/) directory covering every scenario:

```text
examples/
├── cli/                                # CLI & Shell Prompts
│   ├── 01_github_trending.sh           # Extract top trending repositories on github.com
│   ├── 02_amazon_toys.sh               # Amazon search with target attribute projection
│   ├── 03_books_recursive_crawl.sh     # FastPath recursive crawl on books.toscrape.com
│   ├── 04_hackernews_scrape.sh         # HackerNews extraction
│   ├── 05_export_and_api.sh            # Exporting to CSV/Parquet and launching Axum REST API
│   ├── 06_session_vault.sh             # Encrypted session cookie management
│   └── run_all_cli_examples.sh         # Automated test runner for CLI examples
└── python/                             # Python SDK & LlamaIndex Agent Scenarios
    ├── 01_quickstart_triage.py         # Sub-80 token triage & capability envelope enforcement
    ├── 02_fast_crawl_attributes.py     # Multi-page crawl with strict attribute extraction
    ├── 03_query_and_export.py          # SQLite WAL querying & multi-format export
    ├── 04_crypto_vault_sessions.py     # AES-256-GCM authenticated session vaulting
    ├── 05_llamaindex_agent.py          # Binding harness tools into autonomous agent
    └── run_all_python_examples.py      # Automated test runner for Python examples
```

Run all verification examples:
```bash
# Run CLI test suite
./examples/cli/run_all_cli_examples.sh

# Run Python test suite
python3 examples/python/run_all_python_examples.py
```

---

## 9. Verification & Quality Assurance

The codebase complies with strict release standards (0 warnings permitted):

```bash
# 1. Type-check across all workspace crates
make check

# 2. Execute complete 59-test verification suite
make test

# 3. Strict Clippy linter enforcement (0 warnings)
make clippy

# 4. Standard formatting check
make fmt
```

---

## 10. Architecture & Workspace Structure

```text
.
├── Cargo.toml                    # Virtual workspace root manifest
├── Makefile                      # Standard build, test, and release targets
├── harness                       # Standalone CLI launcher script
├── harness_browser/              # Python client package (client, vault, tools, cli)
├── examples/                     # Ready-to-run CLI & Python scenario examples
├── .env.example                  # Environment configuration template
│
├── .github/workflows/
│   └── release.yml               # Multi-arch automated CI/CD release pipeline
│
└── crates/
    ├── harness-core/             # Shared schemas, error taxonomy, AES-256-GCM vault, SQLite store
    ├── harness-browser/          # Native CDP driver, Bézier mouse, AXTree Set-of-Marks, sniffer, crawler
    ├── harness-agent/            # Groq Llama-3.3-70B client, pre-flight triage, AI tool router
    ├── harness-tui/              # 60 FPS double-buffered TUI, slash commands, Axum server, Parquet exporter
    └── harness-bridge/           # PyO3 C-ABI non-blocking Python extension module
```

---

## License

This project is licensed under the [MIT License](LICENSE).
