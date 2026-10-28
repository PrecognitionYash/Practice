#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Example 03: FastPath Recursive Crawling with Domain Boundaries
#
# Scenario:
# Recursively spiders pages across books.toscrape.com, evaluates page value,
# and stores structured data rows into SQLite WAL.
#
# Usage:
#   ./examples/cli/03_books_recursive_crawl.sh
# ─────────────────────────────────────────────────────────────────────────────

set -e
cd "$(dirname "$0")/../.."

echo "================================================================="
echo " [Scenario 3] FastPath Recursive Crawl on books.toscrape.com"
echo "================================================================="

./target/debug/harness "/crawl https://books.toscrape.com --max-pages 20 extract only title, price, rating"
