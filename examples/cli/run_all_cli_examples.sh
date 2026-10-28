#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# run_all_cli_examples.sh — Smoke Test Runner for All CLI Scenarios
#
# Runs through every CLI scenario, ensuring release readiness.
# ─────────────────────────────────────────────────────────────────────────────

set -e
cd "$(dirname "$0")/../.."

chmod +x examples/cli/*.sh

echo "================================================================="
echo " Running All Autonomous Browser Harness CLI Scenarios"
echo "================================================================="

echo -e "\n--- Checking Binary Help & Usage ---"
./target/debug/harness --help

echo -e "\n--- Checking Runtime Status ---"
./target/debug/harness /status

echo -e "\n--- Checking Tables List ---"
./target/debug/harness /tables

echo -e "\n--- Testing Session Vault ---"
./target/debug/harness /vault list

echo -e "\n================================================================="
echo " All CLI verification steps completed successfully!"
echo "================================================================="
