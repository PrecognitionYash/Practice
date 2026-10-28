#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Example 02: Amazon Search with Attribute Filtering & Typo Tolerance
#
# Scenario:
# Dispatches an arbitrary natural language prompt with typos ("serach") and
# extracts strictly the requested attributes ("price and product name only").
#
# Usage:
#   ./examples/cli/02_amazon_toys.sh
# ─────────────────────────────────────────────────────────────────────────────

set -e
cd "$(dirname "$0")/../.."

echo "================================================================="
echo " [Scenario 2] Amazon Product Search & Target Attribute Filtering"
echo "================================================================="

./target/debug/harness "go to amazon and serach for toys and list down price and product name only"
