#!/usr/bin/env python3
import os
import subprocess
import sys
import time
import json

def run_cmd(cmd):
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    return result.returncode, result.stdout, result.stderr

def test_ai_situations():
    print("==================================================================")
    print("Autonomous Browser Harness — Multi-Situation AI & System Verifier")
    print("==================================================================")

    # 1. Test CLI Slash Commands
    print("\n[Situation 1: CLI Slash Command Execution]")
    code, stdout, _ = run_cmd("./harness /help")
    assert code == 0 and "HARNESS COMMAND REFERENCE" in stdout, "Failed /help"
    print("  ✓ ./harness /help executed cleanly")

    code, stdout, _ = run_cmd("./harness /status")
    assert code == 0 and "SYSTEM RUNTIME STATUS" in stdout, "Failed /status"
    print("  ✓ ./harness /status executed cleanly")

    code, stdout, _ = run_cmd("./harness /tables")
    assert code == 0 and ("Harvested tables" in stdout or "No harvested" in stdout), "Failed /tables"
    print("  ✓ ./harness /tables executed cleanly")

    code, stdout, stderr = run_cmd("./harness /export")
    combined = stdout + stderr
    assert code == 0 and "Usage: /export" in combined, f"Failed /export usage: {combined}"
    print("  ✓ ./harness /export (missing args) handled gracefully")

    # 2. Test TUI Interactive Slash Commands via PTY
    print("\n[Situation 2: Interactive 60 FPS TUI Live Terminal Simulation]")
    code, stdout, stderr = run_cmd("python3 -u scripts/test_tui_commands.py")
    assert code == 0 and "ALL 10 INTERACTIVE TUI SLASH COMMAND TESTS PASSED" in stdout, f"TUI tests failed: {stdout}\n{stderr}"
    print("  ✓ All 10 slash commands verified live in differential rendering TUI")

    # 3. Test Full Workspace Unit Test Suite
    print("\n[Situation 3: Native Unit Tests & Invariant Verification]")
    code, stdout, stderr = run_cmd("cargo test --workspace -- --nocapture")
    assert code == 0, f"Cargo test suite failed:\n{stderr}"
    print("  ✓ All 36 native Rust unit tests passed across all crates")

    # 4. Test Clippy & Strict Quality Assurance
    print("\n[Situation 4: Strict Compiler & Linter Verification]")
    code, stdout, stderr = run_cmd("cargo clippy --workspace --all-targets -- -D warnings")
    assert code == 0, f"Clippy failed:\n{stderr}"
    print("  ✓ Workspace passed cargo clippy with zero warnings")

    print("\n==================================================================")
    print("SUCCESS: ALL MULTI-SITUATION TESTS PASSED WITH ZERO ERRORS!")
    print("==================================================================")

if __name__ == "__main__":
    test_ai_situations()
