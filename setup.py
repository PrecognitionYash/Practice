#!/usr/bin/env python3
"""
setup.py for harness-browser Python library.
Supports 'pip install .' and 'pip install -e .'
"""

from setuptools import find_packages, setup

setup(
    name="harness-browser",
    version="0.1.0",
    description="Autonomous Browser Harness Python SDK & AI Agent Tooling",
    packages=find_packages(include=["harness_browser*"]),
    python_requires=">=3.9",
    install_requires=[
        "typing-extensions>=4.0.0",
    ],
    extras_require={
        "crypto": ["cryptography>=41.0.0"],
        "llamaindex": ["llama-index-core>=0.10.0"],
        "langchain": ["langchain>=0.1.0"],
        "all": [
            "cryptography>=41.0.0",
            "llama-index-core>=0.10.0",
            "langchain>=0.1.0",
        ],
    },
    entry_points={
        "console_scripts": [
            "harness-py=harness_browser.cli:main",
            "harness-ai=harness_browser.cli:main",
        ],
    },
)
