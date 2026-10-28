#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Example 05: Database Inspection, Multi-Format Export & REST API
#
# Scenario:
# 1. Lists harvested user tables in SQLite WAL.
# 2. Exports table data to Parquet and CSV in `.harness/exports/`.
# 3. Explains how to spin up the embedded Axum microservice REST API.
#
# Usage:
#   ./examples/cli/05_export_and_api.sh
# ─────────────────────────────────────────────────────────────────────────────

set -e
cd "$(dirname "$0")/../.."

echo "================================================================="
echo " [Scenario 5] Database Inspection & Multi-Format Exporter"
echo "================================================================="

echo "Step 1: Listing harvested database tables:"
./target/debug/harness /tables

echo -e "\nStep 2: Exporting table to CSV:"
./target/debug/harness /export amazon_toys csv || echo "(Run scenario 2 first to create amazon_toys table)"

echo -e "\nStep 3: Exporting table to Apache Parquet (Snappy compressed Arrow):"
./target/debug/harness /export amazon_toys parquet || echo "(Run scenario 2 first to create amazon_toys table)"

echo -e "\nStep 4: Launching Axum REST API server:"
echo "To serve harvested records as JSON over REST, run:"
echo "  ./target/debug/harness /api amazon_toys 8080"
