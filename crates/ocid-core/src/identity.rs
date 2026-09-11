//! Node identity: a single Ed25519 keypair used both as the iroh endpoint
//! identity (transport) and as the publisher signing key (releases).
//!
//! Two textual forms of the public key are used:
//! * hex (64 lowercase chars) — this is iroh's native `EndpointId` display and
//!   is valid inside OCI repository names, e.g. `localhost:5050/<hex>/app:tag`;
//! * `did:key:z6Mk...` — the W3C did:key form (multicodec 0xed01 + base58btc),
//!   used for display and accepted everywhere on the CLI.

use std::{fs, os::unix::fs::PermissionsExt};

use anyhow::{anyhow, bail, Context, Result};
use iroh_base::{PublicKey, SecretKey, Signature};

use crate::paths::Paths;

/// The publisher identity is just an iroh public key.
pub type PublisherId = PublicKey;

#[derive(Clone)]
pub struct Identity {
    secret: SecretKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("id", &self.id()).finish()
    }
}

impl Identity {
    pub fn generate() -> Self {
        Self {
            secret: SecretKey::generate(),
        }
    }

    pub fn load(paths: &Paths) -> Result<Self> {
        let path = paths.secret_key();
        let text = fs::read_to_string(&path)
            .with_context(|| format!("reading {} (run `ocid init` first?)", path.display()))?;
        let text = text.trim();
        let bytes: [u8; 32] = hex::decode(text)
            .context("secret key is not valid hex")?
            .try_into()
            .map_err(|_| anyhow!("secret key must be 32 bytes"))?;
        Ok(Self {
            secret: SecretKey::from_bytes(&bytes),
        })
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let path = paths.secret_key();
        if path.exists() {
            bail!("{} already exists", path.display());
        }
        fs::create_dir_all(&paths.home)?;
        fs::write(&path, hex::encode(self.secret.to_bytes()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    pub fn secret(&self) -> &SecretKey {
        &self.secret
    }

    pub fn id(&self) -> PublisherId {
        self.secret.public()
    }

    pub fn did(&self) -> String {
        did_key(&self.id())
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        self.secret.sign(msg)
    }
}

/// Render a public key as `did:key:z6Mk...`.
pub fn did_key(pk: &PublicKey) -> String {
    // multicodec ed25519-pub = 0xed, varint-encoded as [0xed, 0x01]
    let mut buf = Vec::with_capacity(34);
    buf.extend_from_slice(&[0xed, 0x01]);
    buf.extend_from_slice(pk.as_bytes());
    format!("did:key:z{}", bs58::encode(buf).into_string())
}

/// Parse a publisher id from hex, base32, or `did:key:z...`.
pub fn parse_publisher(s: &str) -> Result<PublisherId> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("did:key:") {
        let b58 = rest
            .strip_prefix('z')
            .context("did:key must use base58btc ('z' prefix)")?;
        let bytes = bs58::decode(b58)
            .into_vec()
            .context("did:key is not valid base58")?;
        if bytes.len() != 34 || bytes[0] != 0xed || bytes[1] != 0x01 {
            bail!("did:key is not an ed25519 key");
        }
        let key: [u8; 32] = bytes[2..].try_into().unwrap();
        return PublicKey::from_bytes(&key).map_err(|e| anyhow!("invalid did:key: {e}"));
    }
    s.parse::<PublicKey>()
        .map_err(|e| anyhow!("invalid publisher id {s:?}: {e}"))
}

/// True if `s` looks like a 64-char lowercase hex public key (the form used in
/// registry repository paths).
pub fn is_hex_publisher(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Verify `sig` over `msg` by `pk`.
pub fn verify(pk: &PublicKey, msg: &[u8], sig: &Signature) -> Result<()> {
    pk.verify(msg, sig)
        .map_err(|_| anyhow!("invalid signature for {}", pk.fmt_short()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_roundtrip() {
        let id = Identity::generate();
        let did = id.did();
        assert!(did.starts_with("did:key:z6Mk"));
        assert_eq!(parse_publisher(&did).unwrap(), id.id());
        assert_eq!(parse_publisher(&id.id().to_string()).unwrap(), id.id());
        assert!(is_hex_publisher(&id.id().to_string()));
    }

    #[test]
    fn sign_verify() {
        let id = Identity::generate();
        let sig = id.sign(b"hello");
        verify(&id.id(), b"hello", &sig).unwrap();
        assert!(verify(&id.id(), b"hellp", &sig).is_err());
    }
}
