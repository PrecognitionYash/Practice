"""
harness_browser.cli — Command-Line Interface for the Harness Python Library
"""

import argparse
import json
import sys

from harness_browser.client import AutonomousBrowserHarness


def main():
    parser = argparse.ArgumentParser(
        prog="harness-py",
        description="Autonomous Browser Harness — Python CLI Tools",
    )
    subparsers = parser.add_subparsers(dest="command", help="Available subcommands")

    # triage
    p_triage = subparsers.add_parser("triage", help="Run pre-flight capability triage on a prompt")
    p_triage.add_argument("prompt", help="Instruction or task to triage")

    # search
    p_search = subparsers.add_parser("search", help="Resolve query to canonical URL via Zero-URL search")
    p_search.add_argument("query", help="Query to resolve")

    # query
    p_query = subparsers.add_parser("query", help="Query harvested SQLite table rows")
    p_query.add_argument("table", help="Table name")
    p_query.add_argument("--limit", type=int, default=10, help="Row limit (default 10)")

    # tables
    subparsers.add_parser("tables", help="List all harvested user tables in SQLite")

    # vault
    p_vault = subparsers.add_parser("vault", help="Inspect cryptographic session vault")
    p_vault.add_argument("--list", action="store_true", help="List vaulted domains")
    p_vault.add_argument("--domain", help="Check session status for domain")

    # crawl
    p_crawl = subparsers.add_parser("crawl", help="Recursively harvest pages from a seed URL with scope boundary")
    p_crawl.add_argument("url", help="Seed URL to start crawl")
    p_crawl.add_argument("--depth", type=int, default=3, help="Max depth (0 = unbounded)")
    p_crawl.add_argument("--pages", type=int, default=0, help="Max pages (0 = crawl forever)")
    p_crawl.add_argument("--prefix", help="Strict path prefix scope (e.g. /rust-lang/rust)")
    p_crawl.add_argument("--no-seed-prefix", action="store_true", help="Do not restrict to seed URL path prefix")
    p_crawl.add_argument("--table", help="SQLite destination table")
    p_crawl.add_argument("--selectors", help="JSON dictionary of CSS selectors, e.g. '{\"title\": \"h1\"}'")
    p_crawl.add_argument("--attributes", "-a", help="Comma-separated target attributes to extract, e.g. 'title,price,rating'")
    p_crawl.add_argument("--ai-goal", "-g", help="High-level extraction goal describing what information is valuable")

    # export
    p_export = subparsers.add_parser("export", help="Export harvested table to CSV or JSONL")
    p_export.add_argument("table", help="Table name")
    p_export.add_argument("--format", default="csv", choices=["csv", "jsonl"], help="Export format")
    p_export.add_argument("--out", help="Optional output path")

    args = parser.parse_args()
    if not args.command:
        parser.print_help()
        sys.exit(0)

    harness = AutonomousBrowserHarness()

    if args.command == "triage":
        res = harness.triage(args.prompt)
        print(json.dumps(res, indent=2))

    elif args.command == "search":
        res = harness.resolve_search(args.query)
        print(json.dumps(res, indent=2))

    elif args.command == "query":
        rows = harness.query(args.table, limit=args.limit)
        print(json.dumps(rows, indent=2))

    elif args.command == "tables":
        tables = harness.list_tables()
        if not tables:
            print("No harvested user tables found.")
        else:
            print("Harvested tables:")
            for t in tables:
                print(f"  • {t}")

    elif args.command == "vault":
        if args.domain:
            has = harness.vault.has_session(args.domain)
            print(f"Domain '{args.domain}': {'Encrypted session present' if has else 'No session found'}")
        else:
            domains = harness.vault.list_domains()
            if not domains:
                print("Vault contains 0 encrypted sessions.")
            else:
                print("Vaulted domains:")
                for d in domains:
                    print(f"  • {d}")

    elif args.command == "crawl":
        selectors_dict = None
        if args.selectors:
            try:
                selectors_dict = json.loads(args.selectors)
            except json.JSONDecodeError as e:
                print(f"Error parsing --selectors JSON: {e}", file=sys.stderr)
                sys.exit(1)

        attrs_list = [a.strip() for a in args.attributes.split(",") if a.strip()] if args.attributes else None

        print(f"Starting crawl on: {args.url}")
        print(f"  • Depth constraint : {'unbounded' if args.depth == 0 else args.depth}")
        print(f"  • Pages constraint : {'forever (unbounded)' if args.pages == 0 else args.pages}")
        print(f"  • Scope prefix     : {args.prefix or ('auto from seed' if not args.no_seed_prefix else 'entire host')}")
        if attrs_list:
            print(f"  • Attributes       : {attrs_list} (strictly storing requested fields)")
        if args.ai_goal:
            print(f"  • AI Extraction    : {args.ai_goal}")
        if selectors_dict:
            print(f"  • Custom selectors : {list(selectors_dict.keys())}")
        print("  • Stop Mechanism   : Press Ctrl+C anytime to stop; all harvested data is saved in SQLite.")

        try:
            count = harness.crawl(
                seed_url=args.url,
                max_depth=args.depth,
                max_pages=args.pages,
                path_prefix=args.prefix,
                restrict_to_seed_path=not args.no_seed_prefix,
                selectors=selectors_dict,
                table_name=args.table,
                ai_goal=args.ai_goal,
                attributes=attrs_list,
            )
            print(f"\nCrawl completed. Harvested {count} pages into table '{args.table or 'auto'}'.")
        except KeyboardInterrupt:
            print("\n🛑 Crawl stopped by user (Ctrl+C). All harvested records safely retained in SQLite WAL.")
        except Exception as e:
            print(f"Crawl error: {e}", file=sys.stderr)
            sys.exit(1)

    elif args.command == "export":
        try:
            path = harness.export_table(args.table, format=args.format, destination=args.out)
            print(f"Successfully exported table '{args.table}' to: {path}")
        except Exception as e:
            print(f"Export error: {e}", file=sys.stderr)
            sys.exit(1)


if __name__ == "__main__":
    main()
