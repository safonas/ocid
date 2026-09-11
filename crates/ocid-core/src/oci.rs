//! OCI image types, digests and image reference parsing.

use std::{collections::BTreeMap, fmt, str::FromStr};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    config::{AliasTarget, Policy},
    identity::{is_hex_publisher, parse_publisher, PublisherId},
};

pub const MT_OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub const MT_OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub const MT_DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";
pub const MT_DOCKER_LIST: &str = "application/vnd.docker.distribution.manifest.list.v2+json";

pub fn is_manifest_media_type(mt: &str) -> bool {
    matches!(
        mt,
        MT_OCI_MANIFEST | MT_OCI_INDEX | MT_DOCKER_MANIFEST | MT_DOCKER_LIST
    )
}

// ---------------------------------------------------------------------------
// Digest
// ---------------------------------------------------------------------------

/// A `sha256:<hex>` content digest.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Digest(String);

impl Digest {
    pub fn sha256(data: &[u8]) -> Self {
        Self(format!("sha256:{}", hex::encode(Sha256::digest(data))))
    }

    pub fn from_sha256_bytes(bytes: &[u8]) -> Self {
        Self(format!("sha256:{}", hex::encode(bytes)))
    }

    /// The hex part without the algorithm prefix.
    pub fn hex(&self) -> &str {
        &self.0[7..]
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Digest {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let hex = s
            .strip_prefix("sha256:")
            .ok_or_else(|| anyhow!("unsupported digest {s:?} (only sha256 is supported)"))?;
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            bail!("malformed sha256 digest {s:?}");
        }
        Ok(Self(s.to_string()))
    }
}

impl TryFrom<String> for Digest {
    type Error = anyhow::Error;
    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<Digest> for String {
    fn from(d: Digest) -> String {
        d.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Manifest types (only the fields we need; unknown fields are preserved by
// storing manifests as opaque bytes — these structs are used for traversal).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: String,
    pub digest: Digest,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageManifest {
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub artifact_type: Option<String>,
    pub config: Descriptor,
    #[serde(default)]
    pub layers: Vec<Descriptor>,
    /// OCI 1.1: the manifest this one refers to (signatures, SBOMs, ...).
    #[serde(default)]
    pub subject: Option<Descriptor>,
    #[serde(default)]
    pub annotations: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndex {
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub artifact_type: Option<String>,
    #[serde(default)]
    pub manifests: Vec<Descriptor>,
    #[serde(default)]
    pub subject: Option<Descriptor>,
    #[serde(default)]
    pub annotations: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone)]
pub enum Manifest {
    Image(ImageManifest),
    Index(ImageIndex),
}

impl Manifest {
    /// Parse manifest bytes. `content_type` (from the HTTP request) is used as
    /// a hint when the manifest itself lacks `mediaType`.
    pub fn parse(bytes: &[u8], content_type: Option<&str>) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_slice(bytes).context("manifest is not JSON")?;
        let mt = v
            .get("mediaType")
            .and_then(|m| m.as_str())
            .or(content_type)
            .unwrap_or("");
        let is_index = matches!(mt, MT_OCI_INDEX | MT_DOCKER_LIST)
            || (mt.is_empty() && v.get("manifests").is_some());
        if is_index {
            Ok(Self::Index(
                serde_json::from_value(v).context("invalid image index")?,
            ))
        } else {
            Ok(Self::Image(
                serde_json::from_value(v).context("invalid image manifest")?,
            ))
        }
    }

    pub fn media_type(&self) -> &str {
        match self {
            Manifest::Image(m) => m.media_type.as_deref().unwrap_or(MT_OCI_MANIFEST),
            Manifest::Index(i) => i.media_type.as_deref().unwrap_or(MT_OCI_INDEX),
        }
    }

    /// The `subject` this manifest refers to, if any.
    pub fn subject(&self) -> Option<&Descriptor> {
        match self {
            Manifest::Image(m) => m.subject.as_ref(),
            Manifest::Index(i) => i.subject.as_ref(),
        }
    }

    /// The artifact type as defined by the referrers API: explicit
    /// `artifactType`, else the config media type of an image manifest.
    pub fn artifact_type(&self) -> Option<String> {
        match self {
            Manifest::Image(m) => m
                .artifact_type
                .clone()
                .or_else(|| Some(m.config.media_type.clone())),
            Manifest::Index(i) => i.artifact_type.clone(),
        }
    }

    pub fn annotations(&self) -> Option<&BTreeMap<String, String>> {
        match self {
            Manifest::Image(m) => m.annotations.as_ref(),
            Manifest::Index(i) => i.annotations.as_ref(),
        }
    }

    /// Descriptors that this manifest directly references (not the subject).
    pub fn referenced(&self) -> Vec<Descriptor> {
        match self {
            Manifest::Image(m) => {
                let mut v = Vec::with_capacity(m.layers.len() + 1);
                v.push(m.config.clone());
                v.extend(m.layers.iter().cloned());
                v
            }
            Manifest::Index(i) => i.manifests.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Names, tags, references
// ---------------------------------------------------------------------------

/// Validate an OCI repository name component path (`a/b/c`).
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 255 {
        bail!("invalid image name {name:?}");
    }
    for comp in name.split('/') {
        let ok = !comp.is_empty()
            && comp
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
            && comp
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphanumeric())
            && comp != "."
            && comp != "..";
        if !ok {
            bail!("invalid image name component {comp:?} in {name:?}");
        }
    }
    Ok(())
}

pub fn validate_tag(tag: &str) -> Result<()> {
    let ok = !tag.is_empty()
        && tag.len() <= 128
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && tag
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
    if !ok {
        bail!("invalid tag {tag:?}");
    }
    Ok(())
}

/// A fully resolved image reference: `<publisher>/<name>[:<tag>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub publisher: PublisherId,
    pub name: String,
    pub tag: Option<String>,
}

impl ImageRef {
    /// Parse a reference that MUST carry an explicit publisher
    /// (hex or did:key), e.g. `did:key:z6Mk.../app:1.0` or `<hex>/app`.
    pub fn parse_explicit(s: &str) -> Result<Self> {
        let s = s.trim().trim_end_matches('/');
        let (pubs, rest) = s
            .split_once('/')
            .with_context(|| format!("reference {s:?} must be <publisher>/<name>[:<tag>]"))?;
        let publisher = parse_publisher(pubs)?;
        let (name, tag) = split_tag(rest)?;
        Ok(Self {
            publisher,
            name,
            tag,
        })
    }

    /// Hybrid parse: explicit publisher, alias, or implicit self.
    pub fn parse(s: &str, policy: &Policy, self_id: &PublisherId) -> Result<Self> {
        let s = s.trim();
        // did:key form contains ':' so must be handled before tag splitting.
        if s.starts_with("did:key:") {
            return Self::parse_explicit(s);
        }
        let (repo, tag) = split_tag(s)?;
        let (publisher, name) = resolve_repo(&repo, policy, self_id)?;
        Ok(Self {
            publisher,
            name,
            tag,
        })
    }

    pub fn tag_or_latest(&self) -> &str {
        self.tag.as_deref().unwrap_or("latest")
    }
}

impl fmt::Display for ImageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.publisher, self.name)?;
        if let Some(t) = &self.tag {
            write!(f, ":{t}")?;
        }
        Ok(())
    }
}

/// Split `name[:tag]`. The tag separator is the last ':' after the last '/'.
fn split_tag(s: &str) -> Result<(String, Option<String>)> {
    let last_slash = s.rfind('/').map(|i| i + 1).unwrap_or(0);
    let (name, tag) = match s[last_slash..].rfind(':') {
        Some(i) => (&s[..last_slash + i], Some(&s[last_slash + i + 1..])),
        None => (s, None),
    };
    validate_name(name)?;
    if let Some(t) = tag {
        validate_tag(t)?;
    }
    Ok((name.to_string(), tag.map(str::to_string)))
}

/// Resolve a registry repository path (no tag) to `(publisher, name)`:
///
/// 1. `<64-hex>/<name>`          explicit publisher
/// 2. `<alias>/<name>`           alias -> publisher
/// 3. `<alias>`                  alias -> publisher/name
/// 4. `<name>`                   implicit: our own publisher id
pub fn resolve_repo(
    repo: &str,
    policy: &Policy,
    self_id: &PublisherId,
) -> Result<(PublisherId, String)> {
    let repo = repo.trim_matches('/');
    if repo.is_empty() {
        bail!("empty repository name");
    }
    let (first, rest) = match repo.split_once('/') {
        Some((f, r)) => (f, Some(r)),
        None => (repo, None),
    };

    if is_hex_publisher(first) {
        let name = rest.context("missing image name after publisher")?;
        validate_name(name)?;
        return Ok((parse_publisher(first)?, name.to_string()));
    }

    if let Some(AliasTarget::Image { publisher, name }) = policy.resolve_alias(repo) {
        return Ok((publisher, name));
    }
    if let Some(AliasTarget::Publisher(p)) = policy.resolve_alias(first) {
        let name = rest.context("missing image name after alias")?;
        validate_name(name)?;
        return Ok((p, name.to_string()));
    }

    validate_name(repo)?;
    Ok((*self_id, repo.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn split_tags() {
        assert_eq!(split_tag("app").unwrap(), ("app".into(), None));
        assert_eq!(
            split_tag("app:1.0").unwrap(),
            ("app".into(), Some("1.0".into()))
        );
        assert_eq!(
            split_tag("team/app:v1").unwrap(),
            ("team/app".into(), Some("v1".into()))
        );
        assert!(split_tag("App").is_err());
    }

    #[test]
    fn hybrid_resolution() {
        let me = Identity::generate();
        let alice = Identity::generate();
        let mut policy = Policy::default();
        policy.set_alias("alice", &alice.id().to_string()).unwrap();
        policy
            .set_alias("team-app", &format!("{}/my-app", alice.id()))
            .unwrap();

        let r = ImageRef::parse("web:1.0", &policy, &me.id()).unwrap();
        assert_eq!(
            (r.publisher, r.name.as_str(), r.tag.as_deref()),
            (me.id(), "web", Some("1.0"))
        );

        let r = ImageRef::parse("alice/web", &policy, &me.id()).unwrap();
        assert_eq!((r.publisher, r.name.as_str()), (alice.id(), "web"));

        let r = ImageRef::parse("team-app:2", &policy, &me.id()).unwrap();
        assert_eq!(
            (r.publisher, r.name.as_str(), r.tag.as_deref()),
            (alice.id(), "my-app", Some("2"))
        );

        let r = ImageRef::parse(&format!("{}/x/y:z", alice.id()), &policy, &me.id()).unwrap();
        assert_eq!(
            (r.publisher, r.name.as_str(), r.tag.as_deref()),
            (alice.id(), "x/y", Some("z"))
        );

        let r = ImageRef::parse(&format!("{}/x:z", alice.did()), &policy, &me.id()).unwrap();
        assert_eq!(
            (r.publisher, r.name.as_str(), r.tag.as_deref()),
            (alice.id(), "x", Some("z"))
        );
    }

    #[test]
    fn digest_parse() {
        let d = Digest::sha256(b"hi");
        assert!(d.as_str().starts_with("sha256:"));
        assert_eq!(d.hex().len(), 64);
        assert!("sha256:zz".parse::<Digest>().is_err());
        assert!("md5:abcd".parse::<Digest>().is_err());
    }
}
