#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Example 06: AES-256-GCM Session Vault Management
#
# Scenario:
# Demonstrates listing, inspecting, and managing authenticated browser sessions.
#
# Usage:
#   ./examples/cli/06_session_vault.sh
# ─────────────────────────────────────────────────────────────────────────────

set -e
cd "$(dirname "$0")/../.."

echo "================================================================="
echo " [Scenario 6] Encrypted Session Vault Management"
echo "================================================================="

echo "Step 1: Listing stored encrypted sessions in .harness/vault/:"
./target/debug/harness /vault list

echo -e "\nStep 2: Checking session status for a specific domain:"
./target/debug/harness /vault github.com || echo "No session saved for github.com yet."
