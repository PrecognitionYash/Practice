#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Example 04: HackerNews Top Headlines Extraction
#
# Scenario:
# Extracts the top tech news stories from news.ycombinator.com into SQLite.
#
# Usage:
#   ./examples/cli/04_hackernews_scrape.sh
# ─────────────────────────────────────────────────────────────────────────────

set -e
cd "$(dirname "$0")/../.."

echo "================================================================="
echo " [Scenario 4] Extract Top Stories from HackerNews"
echo "================================================================="

./target/debug/harness "Extract top stories on news.ycombinator.com"
