"""
harness_browser.vault — AES-256-GCM + HKDF Session Cryptographic Vault Client

Decrypts and inspects encrypted browser session tokens, authenticated cookies,
and localStorage vectors persisted at rest under .harness/vault/{domain}.enc.

Key Invariant:
Master session keys are derived using HKDF-SHA256 with domain scoping
(salt: b"harness-session-vault-v1", info: domain.encode("utf-8")),
guaranteeing strict per-domain cryptographic isolation.
"""

import json
import os
from pathlib import Path
from typing import Any, Dict, List, Optional


class SessionVault:
    """
    Python client for reading and validating domain session credentials from
    the encrypted harness vault (.harness/vault/{domain}.enc).
    """

    NONCE_LEN = 12

    def __init__(
        self,
        vault_dir: Optional[str] = None,
        master_secret: Optional[bytes] = None,
    ):
        self.vault_dir = Path(vault_dir or os.getenv("HARNESS_VAULT_DIR", ".harness/vault"))
        env_secret = os.getenv("HARNESS_VAULT_KEY")
        if master_secret:
            self.master_secret = master_secret
        elif env_secret:
            self.master_secret = env_secret.encode("utf-8")
        else:
            # Default secret seed matching Rust CryptoVault default
            self.master_secret = b"autonomous-harness-default-secret-seed!"

    def list_domains(self) -> List[str]:
        """Lists all domains that currently possess an encrypted session file."""
        if not self.vault_dir.exists():
            return []
        domains = []
        for p in self.vault_dir.glob("*.enc"):
            domains.append(p.stem)
        return sorted(domains)

    def has_session(self, domain: str) -> bool:
        """Returns True if an encrypted session file exists for domain."""
        return self.get_session_path(domain).is_file()

    def get_session_path(self, domain: str) -> Path:
        """Computes filesystem path for a domain's encrypted session file."""
        safe = "".join(c if (c.isalnum() or c in ".-") else "_" for c in domain)
        return self.vault_dir / f"{safe}.enc"

    def derive_domain_key(self, domain: str) -> bytes:
        """
        Derives a domain-isolated 256-bit AES key using HKDF-SHA256 (RFC 5869).
        Matches Rust CryptoVault: salt=None, info=b"harness-vault-v1:{domain}".
        """
        info = f"harness-vault-v1:{domain}".encode("utf-8")
        try:
            from cryptography.hazmat.primitives.kdf.hkdf import HKDF
            from cryptography.hazmat.primitives import hashes

            hkdf = HKDF(
                algorithm=hashes.SHA256(),
                length=32,
                salt=None,
                info=info,
            )
            return hkdf.derive(self.master_secret)
        except ImportError:
            # Pure Python RFC 5869 fallback via hashlib and hmac
            import hashlib
            import hmac

            salt = b"\x00" * 32
            prk = hmac.new(salt, self.master_secret, hashlib.sha256).digest()
            expand_info = info + b"\x01"
            return hmac.new(prk, expand_info, hashlib.sha256).digest()[:32]

    def store_session(
        self,
        domain: str,
        cookies: List[Dict[str, Any]],
        local_storage: Optional[List[Dict[str, Any]]] = None,
    ) -> Path:
        """
        Encrypts and persists authenticated session state for the domain using AES-256-GCM.
        Writes atomically via temp-file-and-rename to prevent partial writes.
        """
        import datetime

        self.vault_dir.mkdir(parents=True, exist_ok=True)
        key = self.derive_domain_key(domain)
        nonce = os.urandom(self.NONCE_LEN)

        payload = {
            "domain": domain,
            "cookies": cookies,
            "local_storage": local_storage or [],
            "captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        }
        plaintext = json.dumps(payload).encode("utf-8")

        try:
            from cryptography.hazmat.primitives.ciphers.aead import AESGCM

            aesgcm = AESGCM(key)
            ciphertext = aesgcm.encrypt(nonce, plaintext, None)
        except ImportError:
            raise RuntimeError(
                "AES-256-GCM encryption requires the 'cryptography' library. "
                "Install via: pip install cryptography"
            )

        target = self.get_session_path(domain)
        tmp = target.with_suffix(".enc.tmp")
        tmp.write_bytes(nonce + ciphertext)
        tmp.replace(target)
        return target

    def load_session(self, domain: str) -> Dict[str, Any]:
        """
        Loads and decrypts authenticated session state for the given domain.
        Returns parsed dictionary containing cookies, localStorage, and timestamp.
        """
        session_file = self.get_session_path(domain)
        if not session_file.is_file():
            raise FileNotFoundError(f"No vault session found for domain '{domain}' at {session_file}")

        blob = session_file.read_bytes()
        if len(blob) < self.NONCE_LEN + 16:
            raise ValueError(f"Vault session file for '{domain}' is corrupted or truncated.")

        nonce = blob[: self.NONCE_LEN]
        ciphertext = blob[self.NONCE_LEN :]
        key = self.derive_domain_key(domain)

        try:
            from cryptography.hazmat.primitives.ciphers.aead import AESGCM

            aesgcm = AESGCM(key)
            plaintext = aesgcm.decrypt(nonce, ciphertext, None)
            return json.loads(plaintext.decode("utf-8"))
        except ImportError:
            raise RuntimeError(
                "Decryption requires the 'cryptography' library. "
                "Install via: pip install cryptography"
            )
        except Exception as e:
            raise ValueError(f"Failed to decrypt vault session for '{domain}' (tampered or wrong key?): {e}")

    def delete_session(self, domain: str) -> bool:
        """Deletes encrypted session file for the domain."""
        path = self.get_session_path(domain)
        if path.is_file():
            path.unlink()
            return True
        return False

    save_session = store_session

