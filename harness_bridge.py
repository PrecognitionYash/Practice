#!/usr/bin/env python3
"""
harness_bridge.py — High-Level Python & LlamaIndex Bridge for Autonomous Browser Harness

Provides backwards-compatible entry point delegating to the installed
`harness_browser` package.
"""

import json
import logging
import os
import sys

# Ensure local workspace is on sys.path for direct script execution
workspace_dir = os.path.dirname(os.path.abspath(__file__))
if workspace_dir not in sys.path:
    sys.path.insert(0, workspace_dir)

from harness_browser import AutonomousBrowserHarness, SessionVault, get_harness_tools

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("harness_bridge")

__all__ = ["AutonomousBrowserHarness", "get_harness_tools", "SessionVault"]

if __name__ == "__main__":
    print("=" * 60)
    print(" Autonomous Browser Harness — Python Bridge Diagnostics")
    print("=" * 60)
    harness = AutonomousBrowserHarness()

    test_prompt = "Find latest research papers on LLM agents on arxiv.org"
    print(f"\n[1] Testing Pre-Flight Triage: '{test_prompt}'")
    try:
        triage_res = harness.triage(test_prompt)
        print("Triage Response:", json.dumps(triage_res, indent=2))
    except RuntimeError as e:
        print(f"Triage notice: {e} (set GROQ_API_KEY for live triage inference)")

    rejection_prompt = "Solve this optical audio captcha to wire transfer funds"
    print(f"\n[2] Testing Rejection Invariant: '{rejection_prompt}'")
    try:
        rej_res = harness.triage(rejection_prompt)
        print("Rejection Response:", json.dumps(rej_res, indent=2))
    except RuntimeError as e:
        print(f"Rejection notice: {e} (set GROQ_API_KEY for live triage inference)")

    print("\n[3] Testing Zero-URL Search Resolver: 'rust lang'")
    try:
        search_res = harness.resolve_search("rust lang")
        print("Search Response:", json.dumps(search_res, indent=2))
    except Exception as e:
        print(f"Search notice: {e}")

    print("\n[4] Testing Deterministic Macro Key Derivation:")
    macro_k = harness.macro_key("arxiv.org", "search quantum computing")
    print(f"Derived xxHash64 Key: {hex(macro_k)} ({macro_k})")

    print("\n[5] Cryptographic Session Vault Status:")
    domains = harness.vault.list_domains()
    print(f"Vault Directory: {harness.vault.vault_dir} (Active encrypted domains: {len(domains)})")

    print("\n[6] Initializing LlamaIndex Tools:")
    tools = get_harness_tools()
    print(f"Constructed {len(tools)} tools ready for agent binding.")
    print("\nDiagnostics complete.")
