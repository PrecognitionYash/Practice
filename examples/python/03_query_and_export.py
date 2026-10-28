#!/usr/bin/env python3
"""
Example 03: SQLite WAL Data Querying & Multi-Format Dataset Export

Scenario:
Demonstrates listing all harvested tables, fetching structured rows, and
exporting them to CSV or Parquet files.
"""

import os
import sys

# Ensure repository root is on sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../..")))

from harness_browser import AutonomousBrowserHarness


def main():
    print("=" * 60)
    print(" Python Example 03: Data Querying & Multi-Format Export")
    print("=" * 60)

    harness = AutonomousBrowserHarness()

    tables = harness.list_tables()
    print(f"\n[1] Discovered harvested tables ({len(tables)}):", tables)

    if tables:
        target_table = tables[0]
        print(f"\n[2] Querying first 5 rows from table '{target_table}':")
        rows = harness.query(target_table, limit=5)
        for idx, row in enumerate(rows, 1):
            print(f"  Row {idx}: {row.get('payload')}")

        print(f"\n[3] Exporting '{target_table}' to CSV:")
        csv_path = harness.export(target_table, format="csv")
        print(f"  Exported to: {csv_path}")

        print(f"\n[4] Exporting '{target_table}' to Parquet:")
        parquet_path = harness.export(target_table, format="parquet")
        print(f"  Exported to: {parquet_path}")

    print("\n✓ Example 03 passed successfully.")


if __name__ == "__main__":
    main()
