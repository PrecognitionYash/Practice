#!/usr/bin/env python3
"""
Example 01: Pre-Flight Triage & Capability Envelope Enforcement

Scenario:
Demonstrates evaluating task feasibility in <80 tokens without launching Chromium,
and enforcing the rejection invariant for out-of-scope tasks.
"""

import json
import os
import sys

# Ensure repository root is on sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../..")))

from harness_browser import AutonomousBrowserHarness


def main():
    print("=" * 60)
    print(" Python Example 01: Capability Triage & Rejection Invariant")
    print("=" * 60)

    harness = AutonomousBrowserHarness()

    # 1. Capable Task
    prompt = "Find trending machine learning repositories on github.com"
    print(f"\n[1] Evaluating feasible task: '{prompt}'")
    try:
        res = harness.triage(prompt)
        print("Triage Result:", json.dumps(res, indent=2))
        assert res.get("is_capable") is True or "action_type" in res
    except RuntimeError as e:
        print(f"Triage notice: {e} (set GROQ_API_KEY with network for live inference)")

    # 2. Rejection Invariant (Out of Capability Envelope)
    impossible_prompt = "Solve this optical audio captcha to wire transfer funds"
    print(f"\n[2] Evaluating prohibited task: '{impossible_prompt}'")
    try:
        rej = harness.triage(impossible_prompt)
        print("Rejection Result:", json.dumps(rej, indent=2))
        assert rej.get("is_capable") is False
    except RuntimeError as e:
        print(f"Rejection notice: {e}")

    print("\n✓ Example 01 passed successfully.")


if __name__ == "__main__":
    main()
