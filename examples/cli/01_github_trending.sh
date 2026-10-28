#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Example 01: Extract Trending Repositories on GitHub
#
# Scenario:
# Demonstrates natural language autonomous navigation and structured extraction
# using the BrowserAgent CDP pipeline.
#
# Usage:
#   ./examples/cli/01_github_trending.sh
# ─────────────────────────────────────────────────────────────────────────────

set -e
cd "$(dirname "$0")/../.."

echo "================================================================="
echo " [Scenario 1] Extract Top Trending Repositories on github.com"
echo "================================================================="

./target/debug/harness "Extract top trending repositories on github.com"
