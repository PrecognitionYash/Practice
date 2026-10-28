#!/usr/bin/env python3
"""
Example 05: Binding Harness Tools to LlamaIndex / Agent Orchestration

Scenario:
Demonstrates exporting the harness as ready-to-use FunctionTool instances
that any LLM agent (LlamaIndex, LangChain, AutoGen, CrewAI) can call autonomously.
"""

import os
import sys

# Ensure repository root is on sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../..")))

from harness_browser import get_harness_tools


def main():
    print("=" * 60)
    print(" Python Example 05: LlamaIndex / Agentic Tool Integration")
    print("=" * 60)

    tools = get_harness_tools()
    print(f"\n[1] Constructed {len(tools)} harness tools ready for agent binding:\n")

    for idx, t in enumerate(tools, 1):
        name = getattr(t, "metadata", None) and t.metadata.name or getattr(t, "name", str(t))
        desc = getattr(t, "metadata", None) and t.metadata.description or getattr(t, "description", "")
        print(f"  Tool {idx}: {name}")
        print(f"    └ Description: {desc[:100]}...")

    print("\n[2] Agent Integration Sample:")
    print("  from llama_index.core.agent import ReActAgent")
    print("  from llama_index.llms.openai import OpenAI")
    print("  agent = ReActAgent.from_tools(tools, llm=OpenAI(model='gpt-4o'))")
    print("  response = agent.chat('Extract trending rust libraries on github.com')")

    print("\n✓ Example 05 passed successfully.")


if __name__ == "__main__":
    main()
