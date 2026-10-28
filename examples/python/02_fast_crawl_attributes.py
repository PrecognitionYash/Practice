#!/usr/bin/env python3
"""
Example 02: Multi-Page Recursive Crawling with Target Attribute Projection

Scenario:
Crawls a website and extracts ONLY the specific attributes requested by the user,
storing them in SQLite WAL.
"""

import os
import sys

# Ensure repository root is on sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../..")))

from harness_browser import AutonomousBrowserHarness


def main():
    print("=" * 60)
    print(" Python Example 02: Recursive Crawling & Target Attributes")
    print("=" * 60)

    harness = AutonomousBrowserHarness()

    seed_url = "https://books.toscrape.com"
    target_attributes = ["title", "price", "rating"]

    print(f"\n[1] Starting recursive crawl on: {seed_url}")
    print(f"    • Target attributes: {target_attributes} (strictly stored)")
    print("    • Max pages: 10")

    pages_crawled = harness.crawl(
        seed_url=seed_url,
        max_depth=2,
        max_pages=10,
        table_name="books_extracted",
        attributes=target_attributes,
        ai_goal="Extract book titles, prices, and ratings while ignoring pagination noise",
    )

    print(f"\n[2] Crawl finished! Pages evaluated: {pages_crawled}")

    # Inspect the extracted rows
    rows = harness.query("books_extracted", limit=5)
    print(f"\n[3] Harvested records in 'books_extracted' ({len(rows)} preview rows):")
    for r in rows:
        payload = r.get("payload", {})
        print(f"  • Title: {payload.get('title', 'N/A')} | Price: {payload.get('price', 'N/A')}")

    print("\n✓ Example 02 passed successfully.")


if __name__ == "__main__":
    main()
