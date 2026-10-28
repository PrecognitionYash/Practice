"""
harness_browser — Autonomous Browser Harness Python SDK

A high-performance Python client and agent tooling suite powered by a bare-metal
Rust Chromium DevTools Protocol (CDP) driver, Set-of-Marks perception,
AES-256-GCM session vaulting, and native LlamaIndex / LangChain tool integrations.
"""

from harness_browser.client import AutonomousBrowserHarness
from harness_browser.tools import get_harness_tools
from harness_browser.vault import SessionVault

__version__ = "0.1.0"
__all__ = [
    "AutonomousBrowserHarness",
    "get_harness_tools",
    "SessionVault",
    "__version__",
]
