//! Node configuration (`config.toml`) and seeding policy (`policy.toml`).
//!
//! The policy is the heart of the Radicle-style replication model:
//!
//! * `seed` — image refs we replicate and serve to others
//!   (`<publisher>/<name>` kept per `mode`, or `<publisher>/<name>:<tag>`)
//! * `follow` — publishers whose every image we replicate, per `mode`
//! * `pin` — `<publisher>/<name>:<tag>` releases that are always kept
//! * `alias` — human-friendly local names for publishers or images, so that
//!   `localhost:5050/alice/app:1.0` works instead of the 64-hex form.
//!
//! A `mode` (`latest`, `last:N`, `full`) says how much history a rule keeps;
//! rules for the same image combine into a [`Window`].

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::SocketAddr,
};

use anyhow::{bail, Context, Result};
use iroh_base::EndpointAddr;
use serde::{Deserialize, Serialize};

use crate::{
    identity::{parse_publisher, PublisherId},
    oci::ImageRef,
    paths::{write_atomic, Paths},
};

// ---------------------------------------------------------------------------
// config.toml
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Where the local OCI registry + control API listens.
    pub listen: SocketAddr,
    /// Relay mode: "default" (n0 relays), "disabled".
    pub relay: RelayMode,
    /// Fixed UDP bind port for iroh (0 = random).
    pub p2p_port: u16,
    /// Discover other ocid nodes on the local network via mDNS.
    pub mdns: bool,
    /// Max seconds a registry request waits for an on-demand p2p fetch.
    pub fetch_timeout_secs: u64,
    /// Expose Prometheus/OpenMetrics at `GET /metrics` on `listen`.
    pub metrics: bool,
    /// Run garbage collection every N seconds (0 = manual `ocictl gc` only).
    pub gc_interval_secs: u64,
    /// Replicated releases outside the policy are kept at least this long
    /// after they were fetched before automatic GC prunes them.
    pub gc_grace_secs: u64,
    /// How often the blob store deletes unpinned data (seconds).
    pub blob_gc_interval_secs: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RelayMode {
    Default,
    Disabled,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:5050".parse().unwrap(),
            relay: RelayMode::Default,
            p2p_port: 0,
            mdns: true,
            fetch_timeout_secs: 120,
            metrics: true,
            gc_interval_secs: 3600,
            gc_grace_secs: 86400,
            blob_gc_interval_secs: 60,
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let p = paths.config();
        if !p.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&p)?;
        toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_atomic(&paths.config(), toml::to_string_pretty(self)?.as_bytes())
    }
}

// ---------------------------------------------------------------------------
// policy.toml
// ---------------------------------------------------------------------------
//
// ```toml
// [[seed]]
// ref = "<publisher>/<name>"      # all tags of this image, kept per `mode`
// mode = "last:3"                 # "full" | "latest" | "last:N"   (default: latest)
//
// [[seed]]
// ref = "<publisher>/<name>:1.0"  # exactly this tag (mode ignored)
//
// [[follow]]
// publisher = "<publisher>"       # every image of this publisher, kept per `mode`
// mode = "latest"
//
// pin = ["<publisher>/<name>:1.0"]  # always kept and fetched, never garbage-collected
//
// [alias]
// alice = "<publisher>"
// ```

/// How many releases of an image to keep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Mode {
    /// Every tag, forever.
    Full,
    /// Only the most recently published release (== `last:1`).
    #[default]
    Latest,
    /// The N most recently published releases.
    Last(u32),
}

impl Mode {
    /// Window size; `None` for unlimited.
    pub fn window(&self) -> Option<usize> {
        match self {
            Mode::Full => None,
            Mode::Latest => Some(1),
            Mode::Last(n) => Some((*n).max(1) as usize),
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Full => f.write_str("full"),
            Mode::Latest => f.write_str("latest"),
            Mode::Last(n) => write!(f, "last:{n}"),
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim() {
            "full" | "all" => Ok(Mode::Full),
            "latest" => Ok(Mode::Latest),
            other => {
                let n = other
                    .strip_prefix("last:")
                    .or_else(|| other.strip_prefix("last="))
                    .with_context(|| {
                        format!("mode must be full, latest or last:N, got {other:?}")
                    })?;
                let n: u32 = n.parse().context("last:N needs a number")?;
                if n == 0 {
                    bail!("last:N must be at least 1");
                }
                Ok(if n == 1 { Mode::Latest } else { Mode::Last(n) })
            }
        }
    }
}

impl TryFrom<String> for Mode {
    type Error = anyhow::Error;
    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<Mode> for String {
    fn from(m: Mode) -> String {
        m.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedEntry {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "is_default_mode")]
    pub mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FollowEntry {
    pub publisher: String,
    #[serde(default, skip_serializing_if = "is_default_mode")]
    pub mode: Mode,
}

fn is_default_mode(m: &Mode) -> bool {
    *m == Mode::default()
}

// Accept both the table form and a bare string (older policy files).
impl<'de> Deserialize<'de> for SeedEntry {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            Table {
                #[serde(rename = "ref")]
                reference: String,
                #[serde(default)]
                mode: Mode,
            },
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Str(reference) => SeedEntry {
                reference,
                mode: Mode::default(),
            },
            Raw::Table { reference, mode } => SeedEntry { reference, mode },
        })
    }
}

impl<'de> Deserialize<'de> for FollowEntry {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            Table {
                publisher: String,
                #[serde(default)]
                mode: Mode,
            },
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Str(publisher) => FollowEntry {
                publisher,
                mode: Mode::default(),
            },
            Raw::Table { publisher, mode } => FollowEntry { publisher, mode },
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    /// Images to replicate.
    pub seed: Vec<SeedEntry>,
    /// Publishers whose every image we replicate.
    pub follow: Vec<FollowEntry>,
    /// `<publisher>/<name>:<tag>` releases that are always kept and fetched.
    pub pin: Vec<String>,
    /// alias -> publisher (hex) or publisher/name
    pub alias: BTreeMap<String, String>,
}

/// One parsed seed rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedRule {
    pub publisher: PublisherId,
    pub name: String,
    /// `Some` = exactly this tag; `None` = the image, kept per `mode`.
    pub tag: Option<String>,
    pub mode: Mode,
}

/// Effective retention for one image, combining every matching rule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Window {
    /// Keep everything.
    pub full: bool,
    /// Keep the N most recent releases (0 = no window rule).
    pub last: usize,
    /// Specific tags to keep (seed rules with a tag, pins).
    pub tags: BTreeSet<String>,
}

impl Window {
    pub fn is_empty(&self) -> bool {
        !self.full && self.last == 0 && self.tags.is_empty()
    }

    fn absorb(&mut self, mode: Mode) {
        match mode.window() {
            None => self.full = true,
            Some(n) => self.last = self.last.max(n),
        }
    }

    /// Given every known release of the image (`(tag, timestamp)`), decide
    /// which tags are inside the window. Newest first; releases published in
    /// the same second (timestamps are second-resolution) are ordered by tag,
    /// greater tag first, so `:5` pushed right after `:4` still counts as newer.
    pub fn select<'a>(&self, known: impl IntoIterator<Item = (&'a str, u64)>) -> BTreeSet<String> {
        let mut known: Vec<(&str, u64)> = known.into_iter().collect();
        known.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(a.0)));
        known.dedup_by(|a, b| a.0 == b.0);
        let mut out: BTreeSet<String> = self.tags.clone();
        if self.full {
            out.extend(known.iter().map(|(t, _)| t.to_string()));
        } else if self.last > 0 {
            out.extend(known.iter().take(self.last).map(|(t, _)| t.to_string()));
        }
        out
    }

    /// Short human form: `full`, `last:3`, `latest`, `tags`, `-`.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.full {
            parts.push("full".to_string());
        } else if self.last == 1 {
            parts.push("latest".to_string());
        } else if self.last > 1 {
            parts.push(format!("last:{}", self.last));
        }
        if !self.tags.is_empty() {
            parts.push(format!("{} tag(s)", self.tags.len()));
        }
        if parts.is_empty() {
            "-".to_string()
        } else {
            parts.join("+")
        }
    }
}

#[derive(Debug, Clone)]
pub enum AliasTarget {
    Publisher(PublisherId),
    Image {
        publisher: PublisherId,
        name: String,
    },
}

impl Policy {
    pub fn load(paths: &Paths) -> Result<Self> {
        let p = paths.policy();
        if !p.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&p)?;
        toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_atomic(&paths.policy(), toml::to_string_pretty(self)?.as_bytes())
    }

    pub fn seed_rules(&self) -> Vec<SeedRule> {
        self.seed
            .iter()
            .filter_map(|e| match parse_seed_rule(&e.reference, e.mode) {
                Ok(r) => Some(r),
                Err(err) => {
                    tracing::warn!("ignoring invalid seed rule {:?}: {err}", e.reference);
                    None
                }
            })
            .collect()
    }

    pub fn follows(&self) -> Vec<(PublisherId, Mode)> {
        self.follow
            .iter()
            .filter_map(|e| match parse_publisher(&e.publisher) {
                Ok(p) => Some((p, e.mode)),
                Err(err) => {
                    tracing::warn!("ignoring invalid follow {:?}: {err}", e.publisher);
                    None
                }
            })
            .collect()
    }

    pub fn follow_mode(&self, publisher: &PublisherId) -> Option<Mode> {
        self.follows()
            .into_iter()
            .find(|(p, _)| p == publisher)
            .map(|(_, m)| m)
    }

    pub fn pins(&self) -> Vec<SeedRule> {
        self.pin
            .iter()
            .filter_map(|s| match parse_seed_rule(s, Mode::Full) {
                Ok(r) if r.tag.is_some() => Some(r),
                Ok(_) => {
                    tracing::warn!("ignoring pin without tag {s:?}");
                    None
                }
                Err(err) => {
                    tracing::warn!("ignoring invalid pin {s:?}: {err}");
                    None
                }
            })
            .collect()
    }

    /// Every publisher the policy refers to (follows, seeds, pins). These are
    /// the publishers whose announcement topics a node subscribes to.
    pub fn publishers(&self) -> BTreeSet<PublisherId> {
        self.follows()
            .into_iter()
            .map(|(p, _)| p)
            .chain(self.seed_rules().into_iter().map(|r| r.publisher))
            .chain(self.pins().into_iter().map(|r| r.publisher))
            .collect()
    }

    pub fn is_pinned(&self, publisher: &PublisherId, name: &str, tag: &str) -> bool {
        self.pins()
            .iter()
            .any(|r| &r.publisher == publisher && r.name == name && r.tag.as_deref() == Some(tag))
    }

    /// Effective retention window for an image from all matching rules.
    pub fn window(&self, publisher: &PublisherId, name: &str) -> Window {
        let mut w = Window::default();
        if let Some(mode) = self.follow_mode(publisher) {
            w.absorb(mode);
        }
        for r in self.seed_rules() {
            if &r.publisher != publisher || r.name != name {
                continue;
            }
            match &r.tag {
                Some(t) => {
                    w.tags.insert(t.clone());
                }
                None => w.absorb(r.mode),
            }
        }
        for r in self.pins() {
            if &r.publisher == publisher && r.name == name {
                if let Some(t) = r.tag {
                    w.tags.insert(t);
                }
            }
        }
        w
    }

    /// Is there any rule at all for this image?
    pub fn covers(&self, publisher: &PublisherId, name: &str) -> bool {
        !self.window(publisher, name).is_empty()
    }

    pub fn resolve_alias(&self, alias: &str) -> Option<AliasTarget> {
        let target = self.alias.get(alias)?;
        parse_alias_target(target).ok()
    }

    /// Add or update a seed rule. Returns `true` if the policy changed.
    pub fn add_seed(&mut self, rule: &str, mode: Mode) -> Result<bool> {
        let parsed = parse_seed_rule(rule, mode)?;
        let canonical = seed_rule_to_string(&parsed);
        if let Some(existing) = self.seed.iter_mut().find(|e| e.reference == canonical) {
            if existing.mode == mode || parsed.tag.is_some() {
                return Ok(false);
            }
            existing.mode = mode;
            return Ok(true);
        }
        self.seed.push(SeedEntry {
            reference: canonical,
            mode,
        });
        Ok(true)
    }

    /// Remove a seed rule. Without a tag, removes every rule for that image.
    pub fn remove_seed(&mut self, rule: &str) -> Result<bool> {
        let parsed = parse_seed_rule(rule, Mode::Full)?;
        let before = self.seed.len();
        self.seed
            .retain(|e| match parse_seed_rule(&e.reference, e.mode) {
                Ok(r) => {
                    !(r.publisher == parsed.publisher
                        && r.name == parsed.name
                        && (parsed.tag.is_none() || r.tag == parsed.tag))
                }
                Err(_) => true,
            });
        Ok(self.seed.len() != before)
    }

    /// Add or update a follow. Returns `true` if the policy changed.
    pub fn add_follow(&mut self, publisher: &PublisherId, mode: Mode) -> bool {
        if let Some(existing) = self
            .follow
            .iter_mut()
            .find(|e| parse_publisher(&e.publisher).ok().as_ref() == Some(publisher))
        {
            if existing.mode == mode {
                return false;
            }
            existing.mode = mode;
            return true;
        }
        self.follow.push(FollowEntry {
            publisher: publisher.to_string(),
            mode,
        });
        true
    }

    pub fn remove_follow(&mut self, publisher: &PublisherId) -> bool {
        let before = self.follow.len();
        self.follow
            .retain(|e| parse_publisher(&e.publisher).ok().as_ref() != Some(publisher));
        self.follow.len() != before
    }

    pub fn add_pin(&mut self, reference: &str) -> Result<bool> {
        let parsed = parse_seed_rule(reference, Mode::Full)?;
        if parsed.tag.is_none() {
            bail!("a pin needs a tag: {reference}");
        }
        let canonical = seed_rule_to_string(&parsed);
        if self.pin.contains(&canonical) {
            return Ok(false);
        }
        self.pin.push(canonical);
        Ok(true)
    }

    pub fn remove_pin(&mut self, reference: &str) -> Result<bool> {
        let parsed = parse_seed_rule(reference, Mode::Full)?;
        let canonical = seed_rule_to_string(&parsed);
        let before = self.pin.len();
        self.pin.retain(|p| p != &canonical);
        Ok(self.pin.len() != before)
    }

    pub fn set_alias(&mut self, alias: &str, target: &str) -> Result<()> {
        validate_alias(alias)?;
        let t = parse_alias_target(target)?;
        let canonical = match t {
            AliasTarget::Publisher(p) => p.to_string(),
            AliasTarget::Image { publisher, name } => format!("{publisher}/{name}"),
        };
        self.alias.insert(alias.to_string(), canonical);
        Ok(())
    }

    pub fn remove_alias(&mut self, alias: &str) -> bool {
        self.alias.remove(alias).is_some()
    }
}

fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty()
        || alias.len() == 64 && alias.bytes().all(|b| b.is_ascii_hexdigit())
        || !alias
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    {
        bail!("alias must be lowercase [a-z0-9._-] and not look like a key: {alias:?}");
    }
    Ok(())
}

pub fn parse_seed_rule(s: &str, mode: Mode) -> Result<SeedRule> {
    let r = ImageRef::parse_explicit(s)?;
    Ok(SeedRule {
        publisher: r.publisher,
        name: r.name,
        tag: r.tag,
        mode,
    })
}

pub fn seed_rule_to_string(r: &SeedRule) -> String {
    match &r.tag {
        Some(t) => format!("{}/{}:{}", r.publisher, r.name, t),
        None => format!("{}/{}", r.publisher, r.name),
    }
}

fn parse_alias_target(target: &str) -> Result<AliasTarget> {
    if let Ok(p) = parse_publisher(target) {
        return Ok(AliasTarget::Publisher(p));
    }
    let (pubs, name) = target
        .split_once('/')
        .context("alias target must be <publisher> or <publisher>/<name>")?;
    let publisher = parse_publisher(pubs)?;
    crate::oci::validate_name(name)?;
    Ok(AliasTarget::Image {
        publisher,
        name: name.to_string(),
    })
}

// ---------------------------------------------------------------------------
// peers.json — known peer addresses used to bootstrap gossip / sync
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Peers {
    pub peers: Vec<EndpointAddr>,
}

impl Peers {
    pub fn load(paths: &Paths) -> Result<Self> {
        let p = paths.peers();
        if !p.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&p)?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_atomic(&paths.peers(), serde_json::to_vec_pretty(self)?.as_slice())
    }

    /// Insert or replace by endpoint id. Returns true if new.
    pub fn upsert(&mut self, addr: EndpointAddr) -> bool {
        if let Some(existing) = self.peers.iter_mut().find(|p| p.id == addr.id) {
            // merge transport addrs
            existing.addrs.extend(addr.addrs);
            false
        } else {
            self.peers.push(addr);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn mode_roundtrip() {
        for (s, m) in [
            ("full", Mode::Full),
            ("latest", Mode::Latest),
            ("last:1", Mode::Latest),
            ("last:3", Mode::Last(3)),
        ] {
            assert_eq!(s.parse::<Mode>().unwrap(), m);
        }
        assert!("last:0".parse::<Mode>().is_err());
        assert!("weird".parse::<Mode>().is_err());
        assert_eq!(Mode::Last(3).to_string(), "last:3");
    }

    #[test]
    fn policy_toml_compat_and_window() {
        let alice = Identity::generate().id();
        let text = format!(
            r#"
seed = ["{alice}/old-style", {{ ref = "{alice}/app", mode = "last:2" }}, "{alice}/app:1.0"]
follow = ["{alice}"]
pin = ["{alice}/app:0.9"]
"#
        );
        let p: Policy = toml::from_str(&text).unwrap();
        assert_eq!(p.seed[0].mode, Mode::Latest);
        assert_eq!(p.seed[1].mode, Mode::Last(2));
        assert_eq!(p.follow[0].mode, Mode::Latest);

        let w = p.window(&alice, "app");
        assert!(!w.full);
        assert_eq!(w.last, 2);
        assert_eq!(
            w.tags,
            BTreeSet::from(["1.0".to_string(), "0.9".to_string()])
        );

        // newest two + explicit tags
        let known = [
            ("3.0", 30),
            ("2.0", 20),
            ("1.0", 10),
            ("0.9", 9),
            ("0.5", 5),
        ];
        let sel = w.select(known.iter().map(|(t, ts)| (*t, *ts)));
        assert_eq!(
            sel,
            BTreeSet::from_iter(["3.0", "2.0", "1.0", "0.9"].map(String::from))
        );

        // follow-only image: latest
        let w = p.window(&alice, "other");
        assert_eq!(w.last, 1);
        assert_eq!(
            w.select([("b", 2), ("a", 1)]),
            BTreeSet::from(["b".to_string()])
        );
        // same-second pushes: greater tag wins
        assert_eq!(
            w.select([("4", 7), ("5", 7), ("3", 6)]),
            BTreeSet::from(["5".to_string()])
        );

        assert_eq!(p.publishers(), BTreeSet::from([alice]));

        // round trip keeps table form
        let back: Policy = toml::from_str(&toml::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.seed, p.seed);
        assert_eq!(back.pin, p.pin);
    }
}
