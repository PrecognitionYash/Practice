#!/usr/bin/env python3
"""
run_all_python_examples.py — Automated Test Runner for Python Scenarios
"""

import subprocess
import sys
from pathlib import Path

PYTHON_EXAMPLES_DIR = Path(__file__).parent


def run_example(script_name: str) -> bool:
    print(f"\n>>> Running {script_name}...")
    result = subprocess.run([sys.executable, str(PYTHON_EXAMPLES_DIR / script_name)])
    return result.returncode == 0


def main():
    print("=" * 60)
    print(" Running All Autonomous Browser Harness Python Scenarios")
    print("=" * 60)

    scripts = [
        "01_quickstart_triage.py",
        "03_query_and_export.py",
        "04_crypto_vault_sessions.py",
        "05_llamaindex_agent.py",
    ]

    failed = []
    for s in scripts:
        if not run_example(s):
            failed.append(s)

    print("\n" + "=" * 60)
    if failed:
        print(f"❌ Failed examples: {failed}")
        sys.exit(1)
    else:
        print("✓ All automated Python examples completed successfully!")
        print("=" * 60)


if __name__ == "__main__":
    main()
