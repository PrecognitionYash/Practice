// ──────────────────────────────────────────────────────────────────────
// harness-core/src/vault.rs — AES-256-GCM + HKDF Cryptographic Vault
//
// Encrypts/decrypts session payloads (cookies, localStorage) at rest.
// Each serialisation uses a unique 96-bit nonce.  Key material is
// derived from a master secret via HKDF-SHA256 with a domain-scoped
// info string, ensuring per-domain key isolation.
//
// Storage layout:
//   .harness/vault/{domain}.enc
//   Bytes: [12-byte nonce][ciphertext + 16-byte GCM tag]
// ──────────────────────────────────────────────────────────────────────

use std::path::{Path, PathBuf};

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use tracing::{debug, instrument};

use crate::error::{HarnessError, HarnessResult};
use crate::schema::SessionPayload;

/// Size of the AES-256-GCM nonce in bytes.
const NONCE_LEN: usize = 12;

/// The cryptographic session vault.
///
/// Manages per-domain encrypted session files under a configurable
/// vault directory.  Thread-safe (all methods take `&self`).
#[derive(Debug, Clone)]
pub struct CryptoVault {
    /// Root directory for encrypted vault files.
    vault_dir: PathBuf,
    /// The HKDF-derived 256-bit master key material.
    /// We keep the raw IKM here and derive per-domain keys on demand.
    master_ikm: Vec<u8>,
}

impl CryptoVault {
    /// Create a new vault backed by the given directory and master secret.
    ///
    /// The `master_secret` should be sourced from a secure entropy pool
    /// (e.g. OS keychain, environment variable, or hardware RNG).
    /// The vault directory is created if it does not exist.
    ///
    /// # Errors
    /// Returns `HarnessError::VaultIo` if the directory cannot be created.
    #[instrument(skip(master_secret))]
    pub fn new(
        vault_dir: impl AsRef<Path> + std::fmt::Debug,
        master_secret: &[u8],
    ) -> HarnessResult<Self> {
        let vault_dir = vault_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&vault_dir).map_err(|e| HarnessError::VaultIo {
            path: vault_dir.clone(),
            source: e,
        })?;
        debug!(dir = %vault_dir.display(), "vault initialised");
        Ok(Self {
            vault_dir,
            master_ikm: master_secret.to_vec(),
        })
    }

    /// Encrypt and persist a session payload for the given domain.
    ///
    /// The file is written atomically (write-to-temp then rename) to
    /// prevent partial writes on crash.
    ///
    /// # Errors
    /// Returns `HarnessError::Encryption` on cipher failure or
    /// `HarnessError::VaultIo` on filesystem errors.
    #[instrument(skip(self, payload), fields(domain = %payload.domain))]
    pub fn store_session(&self, payload: &SessionPayload) -> HarnessResult<PathBuf> {
        let domain = &payload.domain;
        let key = self.derive_domain_key(domain)?;
        let cipher = Aes256Gcm::new(&key.into());

        // Serialize payload to compact JSON.
        let plaintext = serde_json::to_vec(payload).map_err(|e| HarnessError::Encryption {
            domain: domain.clone(),
            reason: format!("serialisation failed: {e}"),
        })?;

        // Generate a unique 96-bit nonce per encryption.
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        // Encrypt (ciphertext includes the 16-byte GCM authentication tag).
        let ciphertext =
            cipher
                .encrypt(&nonce, plaintext.as_ref())
                .map_err(|e| HarnessError::Encryption {
                    domain: domain.clone(),
                    reason: format!("AES-GCM encrypt failed: {e}"),
                })?;

        // Assemble wire format: [nonce (12 bytes)][ciphertext+tag].
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);

        // Atomic write via temp file + rename.
        let target = self.domain_path(domain);
        let tmp = target.with_extension("enc.tmp");
        std::fs::write(&tmp, &blob).map_err(|e| HarnessError::VaultIo {
            path: tmp.clone(),
            source: e,
        })?;
        std::fs::rename(&tmp, &target).map_err(|e| HarnessError::VaultIo {
            path: target.clone(),
            source: e,
        })?;

        debug!(path = %target.display(), bytes = blob.len(), "session stored");
        Ok(target)
    }

    /// Load and decrypt a session payload for the given domain.
    ///
    /// # Errors
    /// Returns `HarnessError::Decryption` if the ciphertext is invalid
    /// or tampered, or `HarnessError::VaultIo` if the file is missing.
    #[instrument(skip(self), fields(domain = %domain))]
    pub fn load_session(&self, domain: &str) -> HarnessResult<SessionPayload> {
        let path = self.domain_path(domain);
        let blob = std::fs::read(&path).map_err(|e| HarnessError::VaultIo {
            path: path.clone(),
            source: e,
        })?;

        if blob.len() < NONCE_LEN + 16 {
            return Err(HarnessError::Decryption {
                domain: domain.to_string(),
                reason: "file too short to contain nonce + GCM tag".into(),
            });
        }

        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);

        let key = self.derive_domain_key(domain)?;
        let cipher = Aes256Gcm::new(&key.into());

        let plaintext =
            cipher
                .decrypt(nonce, ciphertext)
                .map_err(|e| HarnessError::Decryption {
                    domain: domain.to_string(),
                    reason: format!("AES-GCM decrypt failed (tampered?): {e}"),
                })?;

        let payload: SessionPayload =
            serde_json::from_slice(&plaintext).map_err(|e| HarnessError::Decryption {
                domain: domain.to_string(),
                reason: format!("deserialisation failed: {e}"),
            })?;

        debug!(path = %path.display(), "session loaded");
        Ok(payload)
    }

    /// Check whether an encrypted session exists for the given domain.
    pub fn has_session(&self, domain: &str) -> bool {
        self.domain_path(domain).exists()
    }

    /// Delete the encrypted session file for a domain.
    ///
    /// Returns `Ok(true)` if a file was removed, `Ok(false)` if none existed.
    pub fn delete_session(&self, domain: &str) -> HarnessResult<bool> {
        let path = self.domain_path(domain);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| HarnessError::VaultIo {
                path: path.clone(),
                source: e,
            })?;
            debug!(domain, "session deleted");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List all domains that have stored sessions.
    pub fn list_domains(&self) -> HarnessResult<Vec<String>> {
        let mut domains = Vec::new();
        let entries = std::fs::read_dir(&self.vault_dir).map_err(|e| HarnessError::VaultIo {
            path: self.vault_dir.clone(),
            source: e,
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| HarnessError::VaultIo {
                path: self.vault_dir.clone(),
                source: e,
            })?;
            if let Some(name) = entry.path().file_stem() {
                if entry.path().extension().is_some_and(|ext| ext == "enc") {
                    domains.push(name.to_string_lossy().into_owned());
                }
            }
        }
        Ok(domains)
    }

    // ── Internal helpers ────────────────────────────────────────────

    /// Derive a 256-bit per-domain encryption key using HKDF-SHA256.
    ///
    /// info = `"harness-vault-v1:{domain}"` to ensure domain isolation.
    fn derive_domain_key(&self, domain: &str) -> HarnessResult<[u8; 32]> {
        let hk = Hkdf::<Sha256>::new(None, &self.master_ikm);
        let info = format!("harness-vault-v1:{domain}");
        let mut okm = [0u8; 32];
        hk.expand(info.as_bytes(), &mut okm)
            .map_err(|_| HarnessError::KeyDerivation {
                reason: format!("HKDF expand failed for domain '{domain}'"),
            })?;
        Ok(okm)
    }

    /// Compute the filesystem path for a domain's encrypted session.
    fn domain_path(&self, domain: &str) -> PathBuf {
        // Sanitise the domain for filesystem safety: replace non-alnum
        // characters with underscores.
        let safe: String = domain
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.vault_dir.join(format!("{safe}.enc"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_payload() -> SessionPayload {
        SessionPayload {
            domain: "example.com".into(),
            cookies: vec![crate::schema::CookieEntry {
                name: "session_id".into(),
                value: "abc123xyz".into(),
                domain: ".example.com".into(),
                path: "/".into(),
                secure: true,
                http_only: true,
                expires: Some(1_700_000_000.0),
            }],
            local_storage: vec![crate::schema::StorageEntry {
                key: "theme".into(),
                value: "dark".into(),
            }],
            captured_at: Utc::now(),
        }
    }

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let dir = tempfile::tempdir().unwrap();
        let secret = b"test-master-secret-32-bytes-long!";
        let vault = CryptoVault::new(dir.path().join("vault"), secret).unwrap();

        let payload = sample_payload();
        vault.store_session(&payload).unwrap();

        assert!(vault.has_session("example.com"));

        let recovered = vault.load_session("example.com").unwrap();
        assert_eq!(recovered.domain, "example.com");
        assert_eq!(recovered.cookies.len(), 1);
        assert_eq!(recovered.cookies[0].value, "abc123xyz");
        assert_eq!(recovered.local_storage.len(), 1);
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let dir = tempfile::tempdir().unwrap();
        let secret = b"test-master-secret-32-bytes-long!";
        let vault = CryptoVault::new(dir.path().join("vault"), secret).unwrap();

        vault.store_session(&sample_payload()).unwrap();

        // Tamper with the encrypted file.
        let path = vault.domain_path("example.com");
        let mut blob = std::fs::read(&path).unwrap();
        if let Some(byte) = blob.last_mut() {
            *byte ^= 0xFF;
        }
        std::fs::write(&path, &blob).unwrap();

        let result = vault.load_session("example.com");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("decrypt failed"));
    }

    #[test]
    fn delete_session_works() {
        let dir = tempfile::tempdir().unwrap();
        let secret = b"test-master-secret-32-bytes-long!";
        let vault = CryptoVault::new(dir.path().join("vault"), secret).unwrap();

        vault.store_session(&sample_payload()).unwrap();
        assert!(vault.has_session("example.com"));

        assert!(vault.delete_session("example.com").unwrap());
        assert!(!vault.has_session("example.com"));

        // Deleting again is a no-op.
        assert!(!vault.delete_session("example.com").unwrap());
    }

    #[test]
    fn list_domains_works() {
        let dir = tempfile::tempdir().unwrap();
        let secret = b"test-master-secret-32-bytes-long!";
        let vault = CryptoVault::new(dir.path().join("vault"), secret).unwrap();

        let mut p1 = sample_payload();
        p1.domain = "alpha.com".into();
        vault.store_session(&p1).unwrap();

        let mut p2 = sample_payload();
        p2.domain = "beta.org".into();
        vault.store_session(&p2).unwrap();

        let mut domains = vault.list_domains().unwrap();
        domains.sort();
        assert_eq!(domains, vec!["alpha.com", "beta.org"]);
    }

    #[test]
    fn different_domains_get_different_keys() {
        let dir = tempfile::tempdir().unwrap();
        let secret = b"test-master-secret-32-bytes-long!";
        let vault = CryptoVault::new(dir.path().join("vault"), secret).unwrap();

        let key_a = vault.derive_domain_key("alpha.com").unwrap();
        let key_b = vault.derive_domain_key("beta.com").unwrap();
        assert_ne!(key_a, key_b);
    }
}
