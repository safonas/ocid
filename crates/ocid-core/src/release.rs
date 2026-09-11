//! Signed release records — the unit of publication.
//!
//! A release binds `(publisher, name, tag)` to a manifest digest and carries
//! the sha256 -> BLAKE3 mapping for every blob the manifest (transitively)
//! references. With a verified release a peer can fetch the whole image over
//! iroh-blobs without trusting the provider: each blob is verified against its
//! BLAKE3 hash while streaming *and* against its sha256 afterwards.

use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::hash::Blake3;
use anyhow::{bail, Context, Result};
use iroh_base::Signature;
use serde::{Deserialize, Serialize};

use crate::{
    identity::{self, Identity, PublisherId},
    oci::Digest,
};

pub const RELEASE_VERSION: u8 = 1;

/// A content blob addressed both ways.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    pub digest: Digest,
    pub hash: Blake3,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

/// A manifest that refers to this release's manifest via `subject`
/// (signature, attestation, SBOM, ...). Its blobs are part of `blobs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Referrer {
    pub digest: Digest,
    pub media_type: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasePayload {
    pub version: u8,
    pub publisher: PublisherId,
    pub name: String,
    pub tag: String,
    /// The tagged manifest (image manifest or index).
    pub manifest: BlobRef,
    /// Everything the manifest references, transitively (configs, layers,
    /// nested manifests), plus the blobs of attached referrers.
    pub blobs: Vec<BlobRef>,
    /// Unix seconds. Newer releases for the same (publisher, name, tag) win.
    pub timestamp: u64,
    /// Manifests attached to this release through the referrers API.
    /// Absent when empty so older records keep their canonical encoding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub referrers: Vec<Referrer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub payload: ReleasePayload,
    /// Ed25519 signature (hex) by `payload.publisher` over the canonical
    /// JSON encoding of `payload`.
    pub signature: String,
}

impl ReleasePayload {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).context("encoding release payload")
    }
}

impl Release {
    pub fn sign(identity: &Identity, mut payload: ReleasePayload) -> Result<Self> {
        payload.version = RELEASE_VERSION;
        payload.publisher = identity.id();
        if payload.timestamp == 0 {
            payload.timestamp = now();
        }
        let sig = identity.sign(&payload.canonical_bytes()?);
        Ok(Self {
            payload,
            signature: hex::encode(sig.to_bytes()),
        })
    }

    /// Verify signature and structural sanity.
    pub fn verify(&self) -> Result<()> {
        if self.payload.version != RELEASE_VERSION {
            bail!("unsupported release version {}", self.payload.version);
        }
        crate::oci::validate_name(&self.payload.name)?;
        crate::oci::validate_tag(&self.payload.tag)?;
        let sig_bytes: [u8; Signature::LENGTH] = hex::decode(&self.signature)
            .context("signature is not hex")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("signature has wrong length"))?;
        let sig = Signature::from_bytes(&sig_bytes);
        identity::verify(
            &self.payload.publisher,
            &self.payload.canonical_bytes()?,
            &sig,
        )
    }

    pub fn publisher(&self) -> &PublisherId {
        &self.payload.publisher
    }
    pub fn name(&self) -> &str {
        &self.payload.name
    }
    /// `<hex>/<name>:<tag>`
    pub fn reference(&self) -> String {
        format!(
            "{}/{}:{}",
            self.payload.publisher, self.payload.name, self.payload.tag
        )
    }

    /// Manifest + all blobs.
    pub fn all_blobs(&self) -> impl Iterator<Item = &BlobRef> {
        std::iter::once(&self.payload.manifest).chain(self.payload.blobs.iter())
    }

    pub fn total_size(&self) -> u64 {
        self.all_blobs().map(|b| b.size).sum()
    }

    pub fn supersedes(&self, other: &Release) -> bool {
        self.payload.timestamp > other.payload.timestamp
            || (self.payload.timestamp == other.payload.timestamp
                && self.payload.manifest.digest != other.payload.manifest.digest)
    }
}

/// Compact description used in inventories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseSummary {
    pub publisher: PublisherId,
    pub name: String,
    pub tag: String,
    pub manifest_digest: Digest,
    pub timestamp: u64,
}

impl From<&Release> for ReleaseSummary {
    fn from(r: &Release) -> Self {
        Self {
            publisher: r.payload.publisher,
            name: r.payload.name.clone(),
            tag: r.payload.tag.clone(),
            manifest_digest: r.payload.manifest.digest.clone(),
            timestamp: r.payload.timestamp,
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest as _;

    fn blob(data: &[u8]) -> BlobRef {
        BlobRef {
            digest: Digest::sha256(data),
            // any 32 bytes will do for signing tests
            hash: Blake3::from_bytes(sha2::Sha256::digest(data).into()),
            size: data.len() as u64,
            media_type: None,
        }
    }

    #[test]
    fn sign_and_verify() {
        let id = Identity::generate();
        let rel = Release::sign(
            &id,
            ReleasePayload {
                version: 0,
                publisher: id.id(),
                name: "app".into(),
                tag: "1.0".into(),
                manifest: blob(b"{}"),
                blobs: vec![blob(b"layer")],
                timestamp: 0,
                referrers: vec![],
            },
        )
        .unwrap();
        rel.verify().unwrap();

        // json roundtrip preserves validity
        let text = serde_json::to_string(&rel).unwrap();
        let back: Release = serde_json::from_str(&text).unwrap();
        back.verify().unwrap();

        // tamper
        let mut bad = rel.clone();
        bad.payload.tag = "2.0".into();
        assert!(bad.verify().is_err());
    }
}
