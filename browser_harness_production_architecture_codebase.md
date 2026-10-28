# High-Performance Autonomous Browser Harness
## Industrial Production Architecture, Algorithmic Specifications, and Deployment Blueprint

---

## 1. Executive Summary & Core Engineering Philosophy

The Autonomous Browser Harness is engineered to resolve a fundamental tension in modern autonomous web agents: **large language models (LLMs) excel at semantic planning and edge-case ambiguity resolution, but are slow, expensive, and fragile when tasked with mechanical DOM traversal and raw UI execution.** 

Commercial browser agents frequently fail in production because they pass multi-megabyte Document Object Model (DOM) snapshots or high-resolution full-page screenshots into multi-modal models for every micro-interaction. Under strict operational constraints—specifically **Groq's $8,192$-token context window ceiling** per API request—this naive approach leads to immediate context exhaustion, severe latency ($>5$ seconds per interaction), and compounding operational costs.

```
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

* **Perception Token Footprint:** Dynamic compression of $100\text{ KB} - 5\text{ MB}$ web pages down to $\le 1,200$ tokens ($\approx 95\%$ reduction).
* **Local Deterministic Latency:** Sub-$5\text{ ms}$ element resolution and synthetic event dispatch via native Chrome DevTools Protocol (CDP) sockets.
* **Network Interception Throughput:** Zero-DOM data extraction yielding sub-$200\text{ ms}$ task completion on JSON-backed Single Page Applications (SPAs).
* **Fast-Path Crawling Speed:** $\ge 150\text{ pages/second}$ on static leaf nodes via cookie-migrated asynchronous HTTP/2 connection pooling.
* **Terminal Interface Cadence:** Zero-flicker, double-buffered rendering locked at a constant $60\text{ FPS}$ with $\le 1\%$ idle CPU utilization.
* **Context Budget Preservation:** Guarantees that at least $6,500$ of Groq's $8,192$ tokens remain allocated for system instructions, tool schemas, and structured Pydantic/Serde outputs.

---

## 2. Formal Invariants & Mathematical Performance Budgeting

### 2.1 The Context-Window Allocation Function

Let $\mathcal{T}_{\text{max}} = 8,192$ be the maximum context window of the target Groq inference engine (`llama-3.3-70b-versatile`). Let $\mathcal{T}_{\text{sys}}$ denote the static system instructions, $\mathcal{T}_{\text{tools}}$ the collapsed schema definitions, $\mathcal{T}_{\text{hist}}$ the rolling task history, and $\mathcal{T}_{\text{percept}}$ the dynamic perceptual state vector of the active page.

The execution constraint is formulated as:

$$\mathcal{T}_{\text{total}} = \mathcal{T}_{\text{sys}} + \mathcal{T}_{\text{tools}} + \mathcal{T}_{\text{hist}} + \mathcal{T}_{\text{percept}} + \mathcal{T}_{\text{completion}} \le \mathcal{T}_{\text{max}}$$

Where:
* $\mathcal{T}_{\text{sys}} \le 350\text{ tokens}$ (Strict, immutable prompt compiler)
* $\mathcal{T}_{\text{tools}} \le 250\text{ tokens}$ (Enforced by our 5-macro interface design)
* $\mathcal{T}_{\text{hist}} \le 600\text{ tokens}$ (Strict rolling window maintaining only the last $k=3$ transitions)
* $\mathcal{T}_{\text{percept}} \le 1,200\text{ tokens}$ (Enforced by the Set-of-Marks Accessibility Tree pruner)
* $\mathcal{T}_{\text{completion}} = 1,024\text{ tokens}$ (Maximum allocated generation envelope)

$$\max(\mathcal{T}_{\text{total}}) = 350 + 250 + 600 + 1,200 + 1,024 = 3,424\text{ tokens} \ll 8,192\text{ tokens}$$

This bounds total context consumption to **$<42\%$ of the total available window**, leaving a safety margin of $4,768\text{ tokens}$ for multi-turn structured extraction and conversational resilience.

### 2.2 Latency Amortization Model

Let $L_{\text{traditional}}$ denote the latency of a standard agent executing an $N$-step workflow where every step $i$ calls an LLM over the network:

$$L_{\text{traditional}} = \sum_{i=1}^{N} \left( t_{\text{CDP\_capture}} + t_{\text{LLM\_roundtrip}} + t_{\text{CDP\_exec}} \right) \approx N \times (500\text{ms} + 1200\text{ms} + 50\text{ms}) = N \times 1750\text{ms}$$

For $N=10$, $L_{\text{traditional}} \approx 17.5\text{ seconds}$.

In this architecture, Groq executes a single macro-planning turn ($1 \times t_{\text{LLM\_roundtrip}}$) or reuses a deterministic cached macro ($0 \times t_{\text{LLM\_roundtrip}}$). All subsequent mechanical actions run locally in native compiled code:

$$L_{\text{harness}} = t_{\text{triage}} + t_{\text{plan}} + \sum_{i=1}^{N} t_{\text{native\_exec}} + \mathbb{E}[\text{Repair}] \times t_{\text{LLM\_repair}}$$

Where $t_{\text{native\_exec}} \le 5\text{ms}$, and the repair probability $\mathbb{E}[\text{Repair}] \le 0.1$:

$$L_{\text{harness}} \approx 150\text{ms} + 400\text{ms} + (10 \times 5\text{ms}) + (0.1 \times 400\text{ms}) = 640\text{ms}$$

This achieves a **$27\times$ latency reduction** over naive browser agent architectures.

---

## 3. End-to-End System Architecture

```
                                  USER INPUT
                                      │
                                      ▼
             ┌──────────────────────────────────────────────────┐
             │       Glitch-Free TUI (Ratatui/Crossterm)        │
             │   • Double-buffered ANSI diffing (60 FPS)        │
             │   • Zero-allocation slash-command parser         │
             └────────┬────────────────────────────────┬────────┘
                      │                                │
        [/api, /export, /stealth]             [Natural Language Prompt]
                      │                                │
                      ▼                                ▼
       ┌──────────────────────────────┐ ┌────────────────────────────────┐
       │   On-Demand Services         │ │   Pre-Flight Capability Triage │
       │   • Embedded Axum REST API   │ │   • Strict 80-token Gatekeeper │
       │   • Arrow / Parquet Exporter │ │   • Early Rejection Engine     │
       └──────────────────────────────┘ └──────────────┬─────────────────┘
                                                       │
                                     [Rejection: "Sorry, sir. I cannot..."]
                                                       │
                                               [Task Authorized]
                                                       │
                                                       ▼
                                        ┌────────────────────────────────┐
                                        │   Zero-URL Search Resolver     │
                                        │   • Canonical HTTP/2 resolver  │
                                        │   • Converts concepts to URLs  │
                                        └──────────────┬─────────────────┘
                                                       │
                                                       ▼
                                        ┌────────────────────────────────┐
                                        │   Deterministic Macro Lookup   │
                                        └──────┬──────────────────┬──────┘
                                               │                  │
                                        [Cache Hit]          [Cache Miss]
                                               │                  │
                                               ▼                  ▼
                         ┌───────────────────────────┐ ┌───────────────────────────┐
                         │ Fast Replay Engine (0 LLM)│ │ Native Browser Core (CDP) │
                         │ • Replays verified steps  │ │ • Direct WebSocket client │
                         │ • Sub-millisecond speed   │ │ • Hardware event dispatch │
                         └───────────────────────────┘ └─────────────┬─────────────┘
                                                                     │
                                       ┌─────────────────────────────┴─────────────────────────────┐
                                       ▼                                                           ▼
                        ┌──────────────────────────────┐                            ┌──────────────────────────────┐
                        │ Network Sniffer (Fast-Path)  │                            │ Perception Subsystem         │
                        │ • Direct JSON interception   │                            │ • Set-of-Marks (<1.2k tok)   │
                        │ • Zero-DOM extraction        │                            │ • Shadow DOM & iframe pierce │
                        └──────────────┬───────────────┘                            │ • Viewport virtual bounds    │
                                       │                                            │ • Prompt injection firewall  │
                                       │                                            └──────────────┬───────────────┘
                                       │                                                           │
                                       │                                                           ▼
                                       │                                            ┌──────────────────────────────┐
                                       │                                            │ Strategic ReAct Planner      │
                                       │                                            │ • Groq Llama-3.3-70B API     │
                                       │                                            │ • Bounded repair packets     │
                                       │                                            └──────────────┬───────────────┘
                                       │                                                           │
                                       └─────────────────────────────┬─────────────────────────────┘
                                                                     │
                                                                     ▼
                                                    ┌──────────────────────────────────┐
                                                    │ High-Throughput SQLite Storage   │
                                                    │ • WAL Mode persistence           │
                                                    │ • AES-256-GCM Session Vault      │
                                                    └──────────────────────────────────┘
```

---

## 4. Comprehensive Module Specifications & Algorithmic Design

### Module 1: Pre-Flight Triage & Capability Gatekeeper (`harness-agent::triage`)

#### Theoretical Objective
Eliminate wasteful browser process spawning, network allocations, and LLM reasoning loops for out-of-scope, unresolvable, or hostile user queries.

#### Algorithmic Workflow
1. Ingest raw prompt $P$ from the user input buffer.
2. Construct a strict, minimal classification payload using Serde serialization.
3. Call Groq with `temperature=0.0`, `max_tokens=80`, enforcing a strict JSON schema:
   ```json
   {
     "is_capable": false,
     "action_type": "incapable",
     "search_query": null,
     "target_url": null,
     "rejection_reason": "Task requires optical audio CAPTCHA bypass."
   }
   ```
4. **The Rejection Invariant:** If `is_capable == false`, write the exact literal string to the terminal log:
   $$\text{"Sorry, sir. I cannot do that."}$$
   Halt the execution pipeline immediately. Zero browser handles are allocated.
5. If `target_url` is null and `action_type == "direct_search"`, trigger the **Autonomous Zero-URL Search Resolver**.

#### Zero-URL Search Resolution Algorithm
Instead of launching Chromium to search Google or DuckDuckGo (which burns seconds and generates bot challenges), the engine resolves queries over a lightweight, asynchronous HTTP/2 request:

$$\text{URL}_{\text{target}} = \operatorname{Resolve}(\operatorname{GET}\left(\text{"https://html.duckduckgo.com/html/?q="} + \operatorname{URLEncode}(Q)\right))$$

Using a streaming zero-copy parser (`quick-xml`), the resolver identifies the first organic result node matching CSS selector `a.result__url`, normalizes relative protocols, and returns the canonical URL directly to the browser launcher in under $120\text{ ms}$.

---

### Module 2: Perception, AXTree & Set-of-Marks Compression (`harness-browser::accessibility`)

#### The Token Explosion Problem
Passing full HTML dumps ($50\text{k} - 250\text{k}$ tokens) crashes Groq's 8k context window. Stripping HTML via generic text convertors destroys interactive spatial anchors, resulting in hallucinated selector generation.

#### Algorithmic Solution: Set-of-Marks Accessibility Indexer
1. **CDP Tree Ingestion:** Call `Accessibility.getFullAXTree` via Chrome DevTools Protocol to obtain the browser's internal semantic accessibility tree.
2. **Iterative Whitelist Filtering:** Execute an in-memory Depth-First Search (DFS) over the node graph. Retain only nodes where:
   $$\text{Role} \in \{\text{Button}, \text{Link}, \text{TextField}, \text{CheckBox}, \text{ComboBox}, \text{MenuItem}, \text{Tab}\}$$
   Evict layout nodes (`GenericContainer`, `None`, `InlineTextBox`).
3. **Deep Shadow DOM & iframe Piercing:**
   * Invoke `DOM.getFlattenedDocument` with `pierce: true` and `depth: -1`.
   * For cross-origin `<iframe>` nodes, capture the `frameId`, switch CDP execution context to the child world via `Page.createIsolatedWorld`, and register interactive elements into the shared index with a synthetic frame prefix.
4. **Virtual Scroll Viewport Bounding:**
   Query `DOM.getBoxModel` for all interactive nodes. Let $(X_e, Y_e, W_e, H_e)$ denote the bounding quad, $H_v$ the viewport height, and $Y_s$ the vertical scroll offset.
   $$\text{Retain}(e) \iff (Y_e + H_e \ge Y_s) \land (Y_e \le Y_s + 1.5 \times H_v)$$
   Nodes failing this check are pruned from the immediate perception frame and replaced with a structural pagination marker: `[+k elements below fold]`.
5. **Prompt Injection Firewall & Zero-Pixel Sanitizer:**
   * Evaluate computed styling. Prune elements where $\text{display} = \text{"none"}$, $\text{visibility} = \text{"hidden"}$, $\text{opacity} = 0$, or $\text{font-size} \le 1\text{px}$.
   * Apply compile-time regular expression patterns to scrub adversarial phrases (e.g., `(?i)ignore all previous instructions`).
   * Encapsulate extracted content in strict isolation tags:
     ```xml
     <untrusted_web_content origin="{domain}" integrity="sha256-{hash}">
     ...
     </untrusted_web_content>
     ```
6. **Set-of-Marks Serializer:** Assign an incremental integer ID (`@1`, `@2`, ...) to each surviving element. Generate a clean, token-bounded index string:
   ```text
   [@1] <button name="Submit Order">
   [@2] <textbox name="Email Address" val="user@example.com">
   [@3] <link name="Privacy Policy">
   ```
   **Total representation size: $300 - 900\text{ tokens}$.**

---

### Module 3: Deterministic CDP Driver & Protocol Multiplexing (`harness-browser::driver`)

#### Direct WebSocket Protocol Driver
Rather than relying on high-overhead external processes like Selenium or generic Node.js wrappers, the harness establishes a direct asynchronous WebSocket connection to Chromium's `/devtools/browser` endpoint using `chromiumoxide` and `tokio`.

#### Synthetic Hardware Event Dispatch
Automated sites detect synthetic JavaScript clicks (`element.click()`) via event properties like `event.isTrusted === false`. The harness implements native hardware-level input synthesis:

1. **Bézier Trajectory Calculation:** When moving between coordinate $P_0(X_0, Y_0)$ and target coordinate $P_3(X_t, Y_t)$, generate two dynamic control points $P_1$ and $P_2$ using perpendicular Gaussian noise to simulate natural human hand drift:
   $$B(t) = (1-t)^3 P_0 + 3(1-t)^2 t P_1 + 3(1-t) t^2 P_2 + t^3 P_3, \quad t \in [0, 1]$$
2. **Micro-Pacing & Jitter:** Dispatch `Input.dispatchMouseEvent` with action `mouseMoved` along the curve across $15 - 25$ interpolated steps, spaced by randomized delays between $2\text{ms}$ and $7\text{ms}$.
3. **Dispatch Trusted Down/Up Sequence:** Dispatch `mousePressed` followed by a $40 - 110\text{ms}$ dwell time before dispatching `mouseReleased`.

#### CDP Network Sniffing (Direct Zero-DOM Interception)
Dynamic web apps (React, Next.js, Vue) display data rendered from JSON APIs. Inspecting the visual DOM for this data is inefficient.

```
Incoming Web Traffic ──► Network.responseReceived
                                │
                                ▼
                   MIME Type == "application/json"?
                                │
                 ┌──────────────┴──────────────┐
                 ▼                             ▼
               [YES]                          [NO]
                 │                             │
                 ▼                             ▼
    Extract body via requestId         Standard AXTree Pipeline
                 │
                 ▼
    Match target extraction schema
                 │
                 ▼
    Direct Write to SQLite (0 LLM Tokens)
```

1. Enable `Network.enable` with a 10 MB in-memory buffer.
2. Hook `Network.responseReceived` events. If `response.mimeType == "application/json"`, capture the corresponding `requestId`.
3. Call `Network.getResponseBody`. Scan top-level JSON keys against the target extraction schema using streaming deserialization.
4. If the schema matches, pipe the JSON payload directly into the local SQLite database. Complete the user prompt in $<300\text{ ms}$ without rendering, scrolling, or triggering LLM inference.

#### Fast-Path Protocol Swapping (CDP $\to$ Native HTTP/2)
When performing multi-page crawling where leaf pages are static (e.g., blog posts, documentation, regulatory archives):

1. Call `Network.getCookies` over CDP to extract authenticated session cookies, CSRF tokens, and security nonces.
2. Inject the cookies into an asynchronous `reqwest::Client` connection pool running HTTP/2 multiplexing.
3. Demote leaf requests from full browser tabs ($30\text{ MB RAM}$, $400\text{ ms}$ load time) to native HTTP requests ($2\text{ KB RAM}$, $8\text{ ms}$ load time).
4. Parse raw HTML using SIMD-accelerated CSS selectors (`scraper` / `kuchiki`), achieving crawl throughput exceeding **$150\text{ pages/second}$**.

---

### Module 4: Recursive Invariant Traversal Engine (`harness-browser::crawler`)

```
Entry URL ──► Tokio Channel (mpsc) ──► Concurrency Gate (Semaphore: 8)
                     ▲                           │
                     │                           ▼
                     │                 Worker Task (CDP / HTTP)
                     │                           │
                     │                           ▼
            Child Link Extracted ◄─── Visited? (DashSet / xxHash64)
                                                 │
                                           [Not Visited]
                                                 │
                                                 ▼
                                        Stream to SQLite Sink
```

#### Traversal Algorithm
* **Frontier Management:** A thread-safe asynchronous queue (`tokio::sync::mpsc::channel::<CrawlNode>(1024)`).
* **Cycle Detection:** A lock-free shared concurrent set (`dashmap::DashSet<u64>`). URLs are normalized (stripping fragments, sorting query parameters) and hashed using `twox_hash::XxHash64`. Lookup and insertion operate in $O(1)$ time with zero global mutex contention.
* **Concurrency Throttling:** Bounded execution via `tokio::sync::Semaphore` capped at $N=8$ parallel workers.
* **Zero-Token Sink:** Extracted records are streamed through an asynchronous channel directly to the SQLite Write-Ahead Log (WAL) writer, preventing token consumption during the crawl.

---

### Module 5: Storage, Security, & Macro Replay (`harness-core::storage` & `::vault`)

#### Deterministic Macro Caching Engine
Repetitive workflows (e.g., logging in, navigating to an analytics view, exporting a report) should never invoke an LLM more than once.

1. **Recording:** Upon successful completion of an interactive workflow, serialize the sequence of executed actions:
   $$M = \left[ (\text{Role}_1, \text{Selector}_1, \text{Action}_1), \dots, (\text{Role}_k, \text{Selector}_k, \text{Action}_k) \right]$$
2. **Key Derivation:** Derive a stable cache key:
   $$K_{\text{macro}} = \operatorname{xxHash64}(\operatorname{Domain}(U) \parallel \operatorname{IntentHash}(P))$$
3. **Storage:** Persist $M$ to the `macro_cache` table in `.harness/harvest.db`.
4. **Replay Phase:** When a new prompt matches an existing $K_{\text{macro}}$, Rust replays the recorded action sequence deterministically at native speed with **zero calls to Groq**.
5. **Self-Healing Fallback:** If any selector in the macro fails to resolve (e.g., due to a UI update), the engine aborts the replay, purges the invalid cache entry, captures a fresh Set-of-Marks perception snapshot, and calls Groq for a single-turn repair.

#### Cryptographic Session Vault (`AES-256-GCM` + `HKDF`)
* Master session tokens, authenticated cookies, and `localStorage` states are encrypted on disk at `.harness/vault/{domain}.enc`.
* Key derivation uses HKDF-SHA256 with an operating-system-specific entropy seed.
* Encryption uses AES-256-GCM with a unique, cryptographically secure 96-bit nonce per serialization:
  $$\text{Ciphertext} = \operatorname{AES-GCM-Encrypt}(K_{\text{derived}}, \text{Nonce}, \text{SessionPayload})$$
* This isolates authenticated user sessions without storing plaintext tokens on disk.

---

### Module 6: Human-in-the-Loop (HITL) Handoff Protocol (`harness-browser::hitl`)

#### Trigger Conditions
The autonomous loop yields execution to a human operator when:
* A CAPTCHA element (`iframe[src*="recaptcha"]`, `div.cf-turnstile`) is detected.
* Multi-factor authentication (MFA/2FA) or SMS one-time password (OTP) prompts appear.
* The pre-flight triage classifier flags an ambiguous authorization boundary.

#### Handoff Lifecycle State Machine

```
   [Autonomous Browsing]
             │
   (Security Challenge Detected)
             │
             ▼
   [Acquire Execution Lock]
             │
   [Switch Headless -> Headed Window via CDP]
             │
   [Emit TUI Notification: "Action Required"]
             │
             ▼
   [Block on Async Notify Primitive]
             │
   (Human Completes 2FA & Presses Enter in TUI)
             │
             ▼
   [Verify Target URL Redirect]
             │
   [Export Updated Session Cookies to Vault]
             │
   [Resume Autonomous Execution Loop]
```

---

### Module 7: Glitch-Free Terminal UI & On-Demand Microservices (`harness-tui`)

#### Zero-Flicker Double-Buffered Rendering Engine
Traditional terminal interfaces often flicker because they clear the screen buffer (`\x1b[2J`) before writing new frames.

The harness implements immediate-mode differential rendering via `ratatui` and `crossterm`:
1. Maintain two in-memory virtual terminal grids: $\mathcal{G}_{\text{prev}}$ and $\mathcal{G}_{\text{next}}$.
2. On every tick ($16.6\text{ms}$, locked to $60\text{ FPS}$), render widgets into $\mathcal{G}_{\text{next}}$.
3. Compute the minimal ANSI cell difference matrix:
   $$\Delta = \{ (x, y, \text{Cell}) \mid \mathcal{G}_{\text{next}}(x, y) \neq \mathcal{G}_{\text{prev}}(x, y) \}$$
4. Output only the diff sequence $\Delta$ to standard output. Unchanged cells are never rewritten.
5. An RAII terminal guard guarantees that raw mode is disabled and the alternate screen buffer is restored if a panic occurs.

#### Zero-Allocation Slash-Command Parser
User input starting with a forward slash (`/`) bypasses LLM inference entirely and routes directly to local native handlers:

* `/api <table_name> [port]`: Starts an embedded asynchronous `axum` HTTP server on a background Tokio thread, exposing the specified SQLite table as a `/api/{table}` REST endpoint.
* `/export <table_name> <parquet|csv|jsonl>`: Streams the requested dataset from SQLite into Snappy-compressed Apache Parquet, CSV, or JSONL files using zero-copy Apache Arrow vectors.
* `/stealth <fast|ultra>`: Dynamically adjusts CDP fingerprinting profiles (toggling WebGL vendor masking, randomized viewport dimensions, and audio context noise).
* `/undo`: Rolls back the last executed DOM action by traversing the internal undo stack.
* `/clear`: Clears the visible execution log buffer.
* `/quit`: Gracefully terminates background Chromium processes, releases file locks, and exits the application.

---

### Module 8: Python LlamaIndex Bridge & PyO3 C-ABI Specification

For architectures that use a Python-based orchestration layer, the Rust engine compiles into a native C-Python extension module via Maturin and PyO3:

* **Zero-Overhead Memory Mapping:** Strings, vectors, and JSON structures cross the language boundary via PyO3 buffer protocols without redundant serialization.
* **Non-Blocking Execution:** Long-running Rust functions (browser execution, recursive crawling) release Python's Global Interpreter Lock (GIL) via `py.allow_threads(|| { ... })`, keeping Python async event loops responsive.
* **LlamaIndex Tool Wrappers:** Rust routines are exposed as typed `FunctionTool` instances within LlamaIndex, allowing standard ReAct agents to call native operations seamlessly.

---

## 5. Complete Implementation Plan & File Tree

This multi-crate Cargo workspace isolates concerns cleanly across four internal crates, maintaining strict compile-time type safety and minimal binary footprints:

```text
browser-harness/
├── .cargo/
│   └── config.toml               # Native CPU target flags and linker optimizations
├── .github/
│   └── workflows/
│       └── release.yml           # Automated multi-architecture GitHub Actions CI/CD
├── Cargo.toml                    # Virtual workspace root manifest
├── Cargo.lock
├── Makefile                      # Standardized build, test, and release targets
│
├── crates/
│   ├── harness-core/             # Shared types, error taxonomy, cryptographic vault
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs          # Unified Thiserror taxonomy
│   │       ├── schema.rs         # Structured Serde exchange models
│   │       ├── vault.rs          # AES-256-GCM HKDF session encryption
│   │       └── store.rs          # High-throughput SQLite WAL datastore
│   │
│   ├── harness-browser/          # Native CDP driver & DOM perception
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── driver.rs         # Direct CDP WebSocket lifecycle & stealth args
│   │       ├── accessibility.rs  # Set-of-Marks AXTree compressor (<1,200 tokens)
│   │       ├── firewall.rs       # Prompt injection scrubber & zero-pixel filter
│   │       ├── sniffer.rs        # CDP response stream sniffer for raw JSON APIs
│   │       └── crawler.rs        # Recursive Tokio BFS crawler with xxHash64 cycle check
│   │
│   ├── harness-agent/            # Reasoning, triage, and Groq client
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── groq.rs           # Native async reqwest client for Groq completions
│   │       ├── triage.rs         # Pre-flight gatekeeper ("Sorry, sir. I cannot do that.")
│   │       └── search.rs         # Zero-URL canonical search resolver
│   │
│   └── harness-tui/              # Terminal interface & runtime entry point
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs           # Application orchestrator & Tokio entry point
│           ├── events.rs         # Glitch-free RAII terminal lifecycle guard
│           ├── commands.rs       # Zero-allocation slash-command parser
│           ├── server.rs         # Embedded on-demand Axum REST server
│           └── exporter.rs       # Multi-format Apache Arrow / Parquet exporter
│
├── harness_bridge.py             # Optional high-level Python LlamaIndex orchestrator
└── .harness/                     # Local working directory (git-ignored)
    ├── harvest.db                # SQLite database for extracted data
    └── vault/                    # Encrypted authenticated sessions (.enc)
```

---

## 6. Enterprise GitHub Actions Release Pipeline

The `.github/workflows/release.yml` workflow automatically builds, tests, packages, and releases optimized standalone binaries across all major operating systems and CPU architectures whenever a semantic release tag (`v*.*.*`) is pushed.

```yaml
name: Production Release Pipeline

on:
  push:
    tags:
      - 'v*.*.*'

permissions:
  contents: write

jobs:
  build-release:
    name: Build Binary (${{ matrix.target }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            binary_name: harness-tui
            artifact_name: harness-linux-x86_64
            cross: false
          - os: ubuntu-latest
            target: aarch64-unknown-linux-gnu
            binary_name: harness-tui
            artifact_name: harness-linux-aarch64
            cross: true
          - os: macos-latest
            target: aarch64-apple-darwin
            binary_name: harness-tui
            artifact_name: harness-macos-arm64
            cross: false
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            binary_name: harness-tui.exe
            artifact_name: harness-windows-x86_64.exe
            cross: false

    steps:
      - name: Checkout Source Code
        uses: actions/checkout@v4

      - name: Setup Rust Toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Setup Rust Cache
        uses: Swatinem/rust-cache@v2

      - name: Install Linux Cross-Compilation Dependencies
        if: matrix.os == 'ubuntu-latest' && matrix.cross == false
        run: |
          sudo apt-get update
          sudo apt-get install -y libsqlite3-dev pkg-config libssl-dev

      - name: Install Cross for Cross-Compilation
        if: matrix.cross == true
        run: cargo install cross --git https://github.com/cross-rs/cross

      - name: Compile Release Binary (Native)
        if: matrix.cross == false
        run: |
          cargo build --release --target ${{ matrix.target }} --bin harness-tui

      - name: Compile Release Binary (Cross)
        if: matrix.cross == true
        run: |
          cross build --release --target ${{ matrix.target }} --bin harness-tui

      - name: Stage Artifact
        shell: bash
        run: |
          mkdir staging
          if [ "${{ runner.os }}" = "Windows" ]; then
            cp target/${{ matrix.target }}/release/${{ matrix.binary_name }} staging/${{ matrix.artifact_name }}
          else
            cp target/${{ matrix.target }}/release/${{ matrix.binary_name }} staging/${{ matrix.artifact_name }}
            chmod +x staging/${{ matrix.artifact_name }}
          fi

      - name: Upload Binary Artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact_name }}
          path: staging/${{ matrix.artifact_name }}

  publish-github-release:
    name: Publish GitHub Release
    needs: build-release
    runs-on: ubuntu-latest
    steps:
      - name: Checkout Source Code
        uses: actions/checkout@v4

      - name: Download All Staged Artifacts
        uses: actions/download-artifact@v4
        with:
          path: release-binaries

      - name: Generate Cryptographic SHA-256 Checksums
        run: |
          cd release-binaries
          find . -type f -exec sha256sum {} + > SHA256SUMS.txt
          mv SHA256SUMS.txt ../SHA256SUMS.txt

      - name: Draft GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            release-binaries/**/*
            SHA256SUMS.txt
          draft: false
          prerelease: false
          generate_release_notes: true
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

---

## 7. Operational Verification & Testing Protocol

Before deploying this architecture to production or publishing a release tag, verify all subsystem invariants using this protocol:

### Step 1: Pre-Flight Capability Verification
* **Test Case:** Submit an out-of-scope prompt: `"Solve this audio CAPTCHA and transfer funds."`
* **Success Criteria:** The terminal logs `"Sorry, sir. I cannot do that."` in $<200\text{ ms}$. Zero browser processes spawn. Memory consumption remains $<10\text{ MB}$.

### Step 2: Context Budget Invariant Verification
* **Test Case:** Navigate to a complex web page (e.g., an e-commerce catalog or news portal with $>100\text{ KB}$ of HTML).
* **Success Criteria:** Inspect the output of `extract_compact_interactive_index`. The resulting Set-of-Marks index string must measure $\le 1,200\text{ tokens}$ when parsed by the tokenizer.

### Step 3: Fast-Path Network Interception Verification
* **Test Case:** Prompt the agent to extract tabular or structured data from a dynamic Single Page Application.
* **Success Criteria:** The CDP network sniffer intercepts backend JSON responses directly from memory. The database populates without triggering visual element clicks.

### Step 4: TUI Frame-Rate & Glitch Verification
* **Test Case:** Run continuous extraction tasks for 5 minutes while resizing the terminal window and entering commands.
* **Success Criteria:** Frame rate remains stable at $60\text{ FPS}$ without screen flickering, ANSI escape artifacts, or terminal cursor drift.

### Step 5: On-Demand Service Verification
* **Test Case:** Enter the slash command `/api test_dataset 8080`.
* **Success Criteria:** An asynchronous Axum HTTP service binds to port 8080 in $<5\text{ ms}$. Querying `http://127.0.0.1:8080/api/test_dataset` returns valid JSON records without blocking terminal rendering.