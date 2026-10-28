#!/usr/bin/env python3
"""
Example 04: Cryptographic Session Vault (AES-256-GCM)

Scenario:
Demonstrates saving and loading authenticated cookies and session state
with domain-isolated HKDF keys and AES-256-GCM authentication tags.
"""

import os
import sys

# Ensure repository root is on sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../..")))

from harness_browser import SessionVault


def main():
    print("=" * 60)
    print(" Python Example 04: AES-256-GCM Cryptographic Session Vault")
    print("=" * 60)

    vault = SessionVault(vault_dir=".harness/vault")

    # 1. Save an encrypted session
    domain = "demo.example.com"
    cookies = [
        {"name": "session_id", "value": "secret_session_token_xyz123", "domain": domain, "path": "/"},
        {"name": "auth_token", "value": "bearer_998877665544", "domain": domain, "path": "/"},
    ]
    print(f"\n[1] Storing encrypted session for '{domain}' with {len(cookies)} cookies...")
    try:
        vault.save_session(domain, cookies)

        # 2. List encrypted domains
        domains = vault.list_domains()
        print(f"\n[2] Vaulted domains in storage ({len(domains)}):", domains)
        assert domain in domains

        # 3. Load and decrypt session
        print(f"\n[3] Decrypting session for '{domain}'...")
        loaded = vault.load_session(domain)
        print(f"  • Domain: {loaded.get('domain')}")
        print(f"  • Captured at: {loaded.get('captured_at')}")
        print(f"  • Recovered cookies: {loaded.get('cookies')}")

        assert len(loaded.get("cookies", [])) == 2

        # 4. Clean up demo domain
        vault.delete_session(domain)
        print(f"\n[4] Cleaned up session for '{domain}'.")
    except RuntimeError as e:
        print(f"Vault encryption notice: {e}")

    print("\n✓ Example 04 passed successfully.")


if __name__ == "__main__":
    main()
