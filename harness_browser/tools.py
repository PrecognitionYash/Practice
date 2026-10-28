"""
harness_browser.tools — LlamaIndex and LangChain Tool Integrations

Wraps AutonomousBrowserHarness capabilities as typed agent tools.
"""

import logging
from typing import Any, List, Optional

from harness_browser.client import AutonomousBrowserHarness

logger = logging.getLogger("harness_browser.tools")


def get_harness_tools(
    db_path: Optional[str] = None,
    framework: str = "llamaindex",
) -> List[Any]:
    """
    Returns native browser harness capabilities wrapped as agent tools.

    Parameters:
      - db_path: Optional SQLite harvest database path.
      - framework: 'llamaindex' (default) or 'langchain'.
    """
    harness = AutonomousBrowserHarness(db_path=db_path)

    if framework.lower() == "llamaindex":
        try:
            from llama_index.core.tools import FunctionTool  # type: ignore

            tools = [
                FunctionTool.from_defaults(
                    fn=harness.triage,
                    name="harness_preflight_triage",
                    description=(
                        "Evaluates whether a web browsing task is capable and returns "
                        "target classification envelope without allocating a browser."
                    ),
                ),
                FunctionTool.from_defaults(
                    fn=harness.resolve_search,
                    name="harness_zero_url_search",
                    description=(
                        "Converts natural language search queries into canonical target URLs "
                        "in <120ms without launching Chromium."
                    ),
                ),
                FunctionTool.from_defaults(
                    fn=harness.crawl,
                    name="harness_fast_path_crawler",
                    description=(
                        "High-speed recursive HTTP/2 crawler (>150 pages/sec) sinking "
                        "extracted records directly into SQLite WAL."
                    ),
                ),
                FunctionTool.from_defaults(
                    fn=harness.query,
                    name="harness_query_datastore",
                    description=(
                        "Retrieves structured JSON records from the local SQLite harvest datastore."
                    ),
                ),
            ]
            logger.info("Successfully constructed 4 LlamaIndex FunctionTools.")
            return tools

        except ImportError:
            logger.debug("llama-index is not installed. Returning raw callable list.")
            return [
                harness.triage,
                harness.resolve_search,
                harness.crawl,
                harness.query,
            ]

    elif framework.lower() == "langchain":
        try:
            from langchain.tools import Tool  # type: ignore

            tools = [
                Tool(
                    name="harness_preflight_triage",
                    func=lambda p: str(harness.triage(p)),
                    description="Evaluates whether a browsing task is capable.",
                ),
                Tool(
                    name="harness_zero_url_search",
                    func=lambda q: str(harness.resolve_search(q)),
                    description="Converts queries to canonical URLs in <120ms.",
                ),
                Tool(
                    name="harness_fast_path_crawler",
                    func=lambda u: str(harness.crawl(u)),
                    description="High-speed HTTP/2 crawler sinking to SQLite.",
                ),
                Tool(
                    name="harness_query_datastore",
                    func=lambda t: str(harness.query(t)),
                    description="Queries harvested SQLite records.",
                ),
            ]
            return tools
        except ImportError:
            logger.debug("langchain is not installed. Returning raw callable list.")
            return [
                harness.triage,
                harness.resolve_search,
                harness.crawl,
                harness.query,
            ]

    return [
        harness.triage,
        harness.resolve_search,
        harness.crawl,
        harness.query,
    ]
