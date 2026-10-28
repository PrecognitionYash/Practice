# Autonomous Browser Harness — Ready-to-Run Examples

This directory contains standalone, production-ready examples demonstrating all capabilities of the **Autonomous Browser Harness**, in both **CLI / Shell** and **Python SDK**.

---

## Directory Overview

```text
examples/
├── cli/                                # CLI & Autonomous Natural Language Prompts
│   ├── 01_github_trending.sh           # Extract top trending repositories on github.com
│   ├── 02_amazon_toys.sh               # Amazon search with target attribute projection
│   ├── 03_books_recursive_crawl.sh     # FastPath recursive crawl on books.toscrape.com
│   ├── 04_hackernews_scrape.sh         # HackerNews extraction
│   ├── 05_export_and_api.sh            # Exporting to CSV/Parquet and launching Axum REST API
│   ├── 06_session_vault.sh             # Encrypted session cookie management
│   └── run_all_cli_examples.sh         # Smoke test runner for all CLI examples
└── python/                             # Python SDK & LlamaIndex Agent Scenarios
    ├── 01_quickstart_triage.py         # Sub-80 token triage & capability envelope enforcement
    ├── 02_fast_crawl_attributes.py     # Multi-page crawl with strict attribute extraction
    ├── 03_query_and_export.py          # SQLite WAL querying & multi-format export
    ├── 04_crypto_vault_sessions.py     # AES-256-GCM authenticated session vaulting
    ├── 05_llamaindex_agent.py          # Binding harness tools into autonomous agent
    └── run_all_python_examples.py      # Smoke test runner for all Python examples
```

---

## 1. CLI Scenarios

Make scripts executable and run:
```bash
chmod +x examples/cli/*.sh
```

### Scenario 1: GitHub Trending Repositories
Autonomous CDP browser navigation, DOM perception, and structured record extraction into SQLite.
```bash
./target/debug/harness "Extract top trending repositories on github.com"
```

### Scenario 2: Amazon Product Search & Target Attributes
Typo-tolerant natural language search with strict attribute filtering (stores only requested fields: `product_name`, `price`).
```bash
./target/debug/harness "go to amazon and serach for toys and list down price and product name only"
```

### Scenario 3: Recursive Domain Crawling
Spiders pages recursively, evaluating page value and strictly isolating domain boundaries.
```bash
./target/debug/harness "/crawl https://books.toscrape.com --max-pages 20 extract only title, price, rating"
```

### Scenario 4: HackerNews Top Stories
Extracts top tech stories directly into SQLite WAL:
```bash
./target/debug/harness "Extract top stories on news.ycombinator.com"
```

### Scenario 5: Database Inspection, Multi-Format Export & REST API
```bash
# List harvested tables
./target/debug/harness /tables

# Export to CSV
./target/debug/harness /export amazon_toys csv

# Export to Apache Parquet
./target/debug/harness /export amazon_toys parquet

# Start embedded Axum REST API
./target/debug/harness /api amazon_toys 8080
```

### Scenario 6: Cryptographic Session Vault
```bash
# List encrypted sessions
./target/debug/harness /vault list

# Inspect session status
./target/debug/harness /vault amazon.com
```

### Run All CLI Tests
```bash
./examples/cli/run_all_cli_examples.sh
```

---

## 2. Python SDK Scenarios

Run the Python suite:
```bash
python3 examples/python/run_all_python_examples.py
```

### Python Scenario 1: Pre-Flight Capability Triage
```python
from harness_browser import AutonomousBrowserHarness

harness = AutonomousBrowserHarness()
result = harness.triage("Find trending machine learning repositories on github.com")
print(result)
```

### Python Scenario 2: Recursive Crawl with Target Attributes
```python
from harness_browser import AutonomousBrowserHarness

harness = AutonomousBrowserHarness()
count = harness.crawl(
    seed_url="https://books.toscrape.com",
    max_pages=20,
    attributes=["title", "price", "rating"],
    table_name="books_extracted",
)
print(f"Evaluated {count} pages.")
```

### Python Scenario 3: Querying SQLite & Exporting
```python
from harness_browser import AutonomousBrowserHarness

harness = AutonomousBrowserHarness()
rows = harness.query("books_extracted", limit=50)
csv_file = harness.export("books_extracted", format="csv")
parquet_file = harness.export("books_extracted", format="parquet")
```

### Python Scenario 4: AES-256-GCM Session Vault
```python
from harness_browser import SessionVault

vault = SessionVault(vault_dir=".harness/vault")
vault.save_session("amazon.com", [{"name": "session_id", "value": "secret_token"}])
session = vault.load_session("amazon.com")
```

### Python Scenario 5: LlamaIndex / Agentic Tool Binding
```python
from harness_browser import get_harness_tools
from llama_index.core.agent import ReActAgent
from llama_index.llms.openai import OpenAI

tools = get_harness_tools()
agent = ReActAgent.from_tools(tools, llm=OpenAI(model="gpt-4o"))
response = agent.chat("Extract trending rust libraries on github.com")
```
