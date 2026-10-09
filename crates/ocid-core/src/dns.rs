//! DNS publisher records — human names instead of hex publisher ids.
//!
//! A DNS zone claims a human name for a publisher key via a TXT record at
//! `_ocid.<zone>`:
//!
//! ```text
//! v=ocid1 k=<64-hex publisher id> ts=<unix secs> sig=<128-hex>
//! ```
//!
//! The signature is made by the publisher key itself over the canonical
//! payload `ocid1 <zone> <k> <ts>`, which proves mutual consent: the zone
//! admin points the name at the key, and only the key's owner could have
//! signed the record. Releases under that publisher are signed by the same
//! key, so trusting the name is the only leap — artifacts stay
//! self-verifying.
//!
//! Plain DNS is spoofable, so trust is anchored locally (TOFU): the first
//! verified resolution is pinned in `$OCID_HOME/dns-pins.json` and later
//! records for the zone must match the pin or are rejected. DNS is an
//! untrusted lookup; the pins are the root.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use iroh_base::Signature;
use serde::{Deserialize, Serialize};

use crate::identity::{is_hex_publisher, verify, Identity, PublisherId};
use crate::paths::Paths;

/// The TXT record version this build understands.
const VERSION: &str = "ocid1";

// ---------------------------------------------------------------------------
// Record
// ---------------------------------------------------------------------------

/// A parsed and structurally valid `_ocid` TXT record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsRecord {
    /// The publisher the zone claims.
    pub key: PublisherId,
    /// When the record was signed (unix seconds).
    pub ts: u64,
    /// Signature by `key` over the canonical payload.
    pub sig: Signature,
}

impl DnsRecord {
    /// Sign a fresh record for `zone` with the local identity (ts = now).
    pub fn new(zone: &str, id: &Identity) -> Result<Self> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before the unix epoch")?
            .as_secs();
        Self::sign_at(zone, id, ts)
    }

    /// Sign a record for `zone` with an explicit timestamp (tests, tooling).
    pub fn sign_at(zone: &str, id: &Identity, ts: u64) -> Result<Self> {
        validate_zone(zone)?;
        let key = id.id();
        let sig = id
            .secret()
            .sign(canonical_payload(zone, &key, ts).as_bytes());
        Ok(Self { key, ts, sig })
    }

    /// The TXT payload: one space-separated string, fits a single 255-byte
    /// DNS TXT string.
    pub fn to_txt(&self) -> String {
        format!(
            "v={VERSION} k={} ts={} sig={}",
            self.key,
            self.ts,
            hex::encode(self.sig.to_bytes())
        )
    }

    /// Parse a TXT string. Structural only — call [`DnsRecord::verify`]
    /// before trusting it.
    pub fn parse(txt: &str) -> Result<Self> {
        let mut key = None;
        let mut ts = None;
        let mut sig = None;
        let mut version = None;
        for tok in txt.split_whitespace() {
            let (k, v) = tok.split_once('=').context("record fields are key=value")?;
            match k {
                "v" => version = Some(v),
                "k" => {
                    if !is_hex_publisher(v) {
                        bail!("record field k is not a 64-hex publisher id");
                    }
                    key = Some(
                        v.parse::<PublisherId>()
                            .map_err(|e| anyhow::anyhow!("record field k: {e}"))?,
                    );
                }
                "ts" => {
                    ts = Some(
                        v.parse::<u64>()
                            .context("record field ts must be unix seconds")?,
                    );
                }
                "sig" => {
                    let bytes = hex::decode(v).context("record field sig is not hex")?;
                    let arr: [u8; 64] = bytes
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("record field sig must be 128 hex chars"))?;
                    sig = Some(Signature::from_bytes(&arr));
                }
                _ => bail!("unknown record field {k:?}"),
            }
        }
        if version != Some(VERSION) {
            bail!("record must start with v={VERSION}");
        }
        Ok(Self {
            key: key.context("record is missing k=")?,
            ts: ts.context("record is missing ts=")?,
            sig: sig.context("record is missing sig=")?,
        })
    }

    /// Verify the signature over the canonical payload for `zone`. A record
    /// cannot be replayed across zones: the zone is part of the signed
    /// bytes.
    pub fn verify_sig(&self, zone: &str) -> Result<()> {
        validate_zone(zone)?;
        let payload = canonical_payload(zone, &self.key, self.ts);
        verify(&self.key, payload.as_bytes(), &self.sig)
    }

    /// Seconds elapsed since the record was signed.
    pub fn age_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(self.ts)
    }
}

/// The exact bytes a record signature covers.
fn canonical_payload(zone: &str, key: &PublisherId, ts: u64) -> String {
    format!("{VERSION} {zone} {key} {ts}")
}

/// A zone we are willing to look up: a syntactically valid DNS name. Strict
/// (lowercase, no trailing dot, no underscores) — it becomes a DNS query
/// and part of a signed payload.
fn validate_zone(zone: &str) -> Result<()> {
    if zone.len() > 253 || !zone.contains('.') {
        bail!("zone {zone:?} is not a valid DNS name");
    }
    for label in zone.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || label.starts_with('-')
            || label.ends_with('-')
            || label.bytes().all(|b| b.is_ascii_digit())
        {
            bail!("zone {zone:?} is not a valid DNS name");
        }
    }
    Ok(())
}

/// True if `s` looks like a DNS zone rather than an image name: ≥1 dot,
/// lowercase hostname charset, at least one alphabetic label. Image names
/// may contain dots too (`my.app`), so this is only a gate — lookups that
/// find no record fall through to the existing name rules.
pub fn is_domain_name(s: &str) -> bool {
    validate_zone(s).is_ok()
}

// ---------------------------------------------------------------------------
// TOFU pin store
// ---------------------------------------------------------------------------

/// Locally pinned publisher keys per zone (`$OCID_HOME/dns-pins.json`).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DnsPins {
    zones: BTreeMap<String, PinEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinEntry {
    /// Pinned publisher id (hex).
    pub key: String,
    /// When the pin was recorded (unix seconds).
    pub pinned_at: u64,
}

impl DnsPins {
    /// Load the pin store; a missing file is an empty store (nothing pinned
    /// yet, TOFU will pin on first resolution).
    pub fn load(paths: &Paths) -> Result<Self> {
        let path = paths.dns_pins();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let path = paths.dns_pins();
        let text = serde_json::to_string_pretty(self).context("encoding dns pins")?;
        crate::paths::write_atomic(&path, text.as_bytes())
    }

    /// The pinned key for `zone`, if any.
    pub fn get(&self, zone: &str) -> Option<&PinEntry> {
        self.zones.get(zone)
    }

    /// Pin `zone` -> `key` (hex). Returns true if this was a new pin.
    pub fn pin(&mut self, zone: &str, key_hex: &str) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match self.zones.get(zone) {
            Some(p) if p.key == key_hex => false,
            _ => {
                self.zones.insert(
                    zone.to_string(),
                    PinEntry {
                        key: key_hex.to_string(),
                        pinned_at: now,
                    },
                );
                true
            }
        }
    }

    /// Remove the pin for `zone`; true if one existed.
    pub fn unpin(&mut self, zone: &str) -> bool {
        self.zones.remove(zone).is_some()
    }
}

// ---------------------------------------------------------------------------
// Validation (pure part of resolution; the daemon adds lookup + caching)
// ---------------------------------------------------------------------------

/// Outcome of validating a record against the pin store.
#[derive(Debug, PartialEq, Eq)]
pub enum DnsValidated {
    /// Record verified, fresh, and matches the existing pin.
    Pinned(PublisherId),
    /// Record verified and fresh; `DnsPins::pin` it now (TOFU).
    New(PublisherId),
}

/// Verify a record for `zone`: signature, freshness, and pin agreement.
/// The caller persists the new pin for [`DnsValidated::New`].
pub fn validate_record(
    zone: &str,
    record: &DnsRecord,
    pins: &DnsPins,
    max_age_secs: u64,
) -> Result<DnsValidated> {
    record.verify_sig(zone)?;
    let age = record.age_secs();
    if age > max_age_secs {
        bail!(
            "DNS record for {zone} is stale: signed {age}s ago (max {max_age_secs}s) — \
             the zone admin must re-publish"
        );
    }
    let key_hex = record.key.to_string();
    match pins.get(zone) {
        Some(p) if p.key != key_hex => bail!(
            "DNS record for {zone} gives publisher {key_hex}, but this machine pinned {} \
             — refusing to switch keys (possible hijack). If this change is intentional, \
             remove the pin and re-resolve",
            p.key
        ),
        Some(_) => Ok(DnsValidated::Pinned(record.key)),
        None => Ok(DnsValidated::New(record.key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> &'static str {
        "images.example.com"
    }

    #[test]
    fn record_roundtrip() {
        let id = Identity::generate();
        let rec = DnsRecord::new(zone(), &id).unwrap();
        let parsed = DnsRecord::parse(&rec.to_txt()).unwrap();
        assert_eq!(parsed, rec);
        assert!(parsed.verify_sig(zone()).is_ok());
    }

    #[test]
    fn record_is_bound_to_its_zone() {
        let id = Identity::generate();
        let rec = DnsRecord::new(zone(), &id).unwrap();
        assert!(rec.verify_sig("other.example.com").is_err());
    }

    #[test]
    fn tampered_record_is_rejected() {
        let id = Identity::generate();
        let rec = DnsRecord::new(zone(), &id).unwrap();
        // Bump the timestamp inside the record text: still parses, but the
        // signature no longer covers the payload.
        let tampered = rec
            .to_txt()
            .replace(&format!("ts={}", rec.ts), &format!("ts={}", rec.ts + 1));
        let parsed = DnsRecord::parse(&tampered).unwrap();
        assert!(parsed.verify_sig(zone()).is_err());
    }

    #[test]
    fn unknown_version_or_field_is_rejected() {
        assert!(DnsRecord::parse("v=ocid2 k=00 ts=1 sig=00").is_err());
        assert!(DnsRecord::parse("v=ocid1").is_err());
        assert!(DnsRecord::parse("v=ocid1 k=zz ts=1 sig=00").is_err());
    }

    #[test]
    fn freshness_window_is_enforced() {
        let id = Identity::generate();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let fresh = DnsRecord::sign_at(zone(), &id, now - 100).unwrap();
        let stale = DnsRecord::sign_at(zone(), &id, now - 10_000).unwrap();
        let pins = DnsPins::default();
        assert!(matches!(
            validate_record(zone(), &fresh, &pins, 1_000).unwrap(),
            DnsValidated::New(_)
        ));
        assert!(validate_record(zone(), &stale, &pins, 1_000).is_err());
    }

    #[test]
    fn pins_are_tofu_and_sticky() {
        let id = Identity::generate();
        let rec = DnsRecord::new(zone(), &id).unwrap();
        let key_hex = id.id().to_string();
        let mut pins = DnsPins::default();

        // First sight: valid, must be pinned by the caller.
        assert!(matches!(
            validate_record(zone(), &rec, &pins, 60).unwrap(),
            DnsValidated::New(k) if k == id.id()
        ));
        assert!(pins.pin(zone(), &key_hex));

        // Same key again: pinned path, pin() is idempotent.
        assert!(matches!(
            validate_record(zone(), &rec, &pins, 60).unwrap(),
            DnsValidated::Pinned(_)
        ));
        assert!(!pins.pin(zone(), &key_hex));

        // A different key for the same zone: rejected as a hijack.
        let other = DnsRecord::new(zone(), &Identity::generate()).unwrap();
        let err = validate_record(zone(), &other, &pins, 60).unwrap_err();
        assert!(err.to_string().contains("pinned"));

        // Unpin, re-resolve: the new key is accepted and pinned.
        assert!(pins.unpin(zone()));
        assert!(matches!(
            validate_record(zone(), &other, &pins, 60).unwrap(),
            DnsValidated::New(_)
        ));
        assert!(!pins.unpin(zone()));
    }

    #[test]
    fn zone_gate_accepts_domains_not_names() {
        assert!(is_domain_name("images.example.com"));
        assert!(is_domain_name("a.io"));
        assert!(is_domain_name("my.app")); // gated, but falls through on miss
        assert!(!is_domain_name("buildah")); // no dot
        assert!(!is_domain_name("my_app.io")); // underscore is not a hostname char
        assert!(!is_domain_name("1.2.3.4")); // all-numeric labels (IP literal)
        assert!(!is_domain_name("images.example.com.")); // trailing dot
        assert!(!is_domain_name("-bad.example.com")); // bad label
        assert!(!is_domain_name("a".repeat(64).as_str())); // overlong label
    }

    #[test]
    fn pin_store_roundtrips_through_disk() {
        let dir = std::env::temp_dir().join(format!("ocid-dns-{}", uuid::Uuid::new_v4()));
        let paths = Paths { home: dir.clone() };
        std::fs::create_dir_all(&dir).unwrap();
        let mut pins = DnsPins::default();
        assert!(pins.pin("a.example.com", &"0".repeat(64)));
        pins.save(&paths).unwrap();
        let loaded = DnsPins::load(&paths).unwrap();
        assert_eq!(loaded.get("a.example.com").unwrap().key, "0".repeat(64));
        assert!(DnsPins::load(&Paths {
            home: dir.join("missing")
        })
        .unwrap()
        .get("a.example.com")
        .is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
