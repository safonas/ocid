//! Resolution of DNS publisher names for the registry (see `ocid_core::dns`
//! for the record format and trust model).
//!
//! The service owns all DNS I/O and the TOFU pin store; the registry calls
//! [`DnsService::resolve_zone`] for repository paths whose first segment
//! looks like a domain. Verified answers and misses are cached briefly so
//! the per-request resolve path stays off the wire; unusable records
//! (bad signature, stale, pin mismatch) are returned as errors and never
//! cached, so every pull re-checks them.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use hickory_resolver::config::{ConnectionConfig, NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::{RData, RecordType};
use hickory_resolver::{Resolver, TokioResolver};
use ocid_core::config::{Config, DnsMode};
use ocid_core::dns::{self, DnsPins, DnsRecord, DnsValidated};
use ocid_core::identity::PublisherId;
use ocid_core::paths::Paths;
use tokio::sync::Mutex as AsyncMutex;

/// How long a verified zone->publisher answer is reused before re-querying
/// (also capped by the DNS record's own TTL).
const POSITIVE_TTL_CAP: Duration = Duration::from_secs(60);
/// How long a "no record" answer is remembered — keeps dotted self-names
/// (`my.app`) from hitting the resolver on every request.
const NEGATIVE_TTL: Duration = Duration::from_secs(30);

/// Result of resolving a zone.
#[derive(Debug, PartialEq, Eq)]
pub enum ZoneOutcome {
    /// Zone has a verified, fresh, pin-consistent record.
    Resolved {
        publisher: PublisherId,
        /// True when this resolution pinned the zone (TOFU first sight).
        newly_pinned: bool,
    },
    /// No usable record — fall through to the alias/self name rules.
    NoRecord,
    /// DNS resolution is switched off in the config.
    Disabled,
}

enum Cached {
    Positive {
        publisher: PublisherId,
        expires: Instant,
    },
    Negative {
        expires: Instant,
    },
}

pub struct DnsService {
    enabled: bool,
    max_age_secs: u64,
    resolver: TokioResolver,
    cache: Mutex<HashMap<String, Cached>>,
    pins: AsyncMutex<DnsPins>,
    paths: Paths,
}

impl DnsService {
    pub fn from_config(config: &Config, paths: Paths) -> Result<Arc<Self>> {
        let resolver = match &config.dns_nameserver {
            Some(ns) => Self::custom_resolver(ns)?,
            None => TokioResolver::builder_tokio()
                .context("reading the system resolver config")?
                .build()
                .context("building the system resolver")?,
        };
        let pins = DnsPins::load(&paths)?;
        Ok(Arc::new(Self {
            enabled: config.dns == DnsMode::On,
            max_age_secs: config.dns_max_age_secs,
            resolver,
            cache: Mutex::new(HashMap::new()),
            pins: AsyncMutex::new(pins),
            paths,
        }))
    }

    /// Build a resolver pointed at an explicit nameserver (`host:port`,
    /// plain UDP + TCP; port defaults to 53). IP literals only.
    fn custom_resolver(ns: &str) -> Result<TokioResolver> {
        let (host, port) = match ns.rsplit_once(':') {
            Some((h, p)) => {
                let port: u16 = p.parse().context("dns_nameserver port must be numeric")?;
                (h, port)
            }
            None => (ns, 53),
        };
        let ip: IpAddr = host
            .parse()
            .with_context(|| format!("dns_nameserver must be an IP literal, got {host:?}"))?;
        let mut udp = ConnectionConfig::udp();
        udp.port = port;
        let nameserver = NameServerConfig::new(ip, true, vec![udp]);
        let config = ResolverConfig::from_name_servers(vec![nameserver]);
        Resolver::builder_with_config(config, TokioRuntimeProvider::default())
            .build()
            .context("building the DNS resolver")
    }

    /// Resolve `zone` to a publisher id; see [`ZoneOutcome`]. Errors mean a
    /// record exists but is unusable and must be surfaced to the caller.
    pub async fn resolve_zone(&self, zone: &str) -> Result<ZoneOutcome> {
        if !self.enabled {
            return Ok(ZoneOutcome::Disabled);
        }
        let now = Instant::now();
        if let Some(hit) = self.cache.lock().expect("dns cache").get(zone) {
            match hit {
                Cached::Positive { publisher, expires } if *expires > now => {
                    return Ok(ZoneOutcome::Resolved {
                        publisher: *publisher,
                        newly_pinned: false,
                    });
                }
                Cached::Negative { expires } if *expires > now => {
                    return Ok(ZoneOutcome::NoRecord);
                }
                _ => {}
            }
        }

        let query = format!("_ocid.{zone}");
        let answer = match self.resolver.lookup(&query, RecordType::TXT).await {
            Ok(answer) => answer,
            Err(_) => {
                self.cache_negative(zone, now);
                return Ok(ZoneOutcome::NoRecord);
            }
        };

        // TXT records may carry several strings; the first parseable ocid
        // record wins (ours always fits one 255-byte string).
        let mut record = None;
        let mut ttl = u32::MAX;
        for r in answer.answers() {
            ttl = ttl.min(r.ttl);
            if let RData::TXT(txt) = &r.data {
                for s in txt.txt_data.iter() {
                    if let Ok(parsed) = DnsRecord::parse(&String::from_utf8_lossy(s)) {
                        record = Some(parsed);
                        break;
                    }
                }
                if record.is_some() {
                    break;
                }
            }
        }
        let Some(record) = record else {
            self.cache_negative(zone, now);
            return Ok(ZoneOutcome::NoRecord);
        };

        // Signature + freshness + pin agreement (pin write inside the lock).
        let mut pins = self.pins.lock().await;
        let validated = dns::validate_record(zone, &record, &pins, self.max_age_secs);
        let mut newly_pinned = false;
        let publisher = match validated {
            Ok(DnsValidated::New(p)) => {
                newly_pinned = true;
                pins.pin(zone, &p.to_string());
                pins.save(&self.paths)?;
                p
            }
            Ok(DnsValidated::Pinned(p)) => p,
            Err(e) => return Err(anyhow!("{e}")),
        };
        drop(pins);

        let cap = POSITIVE_TTL_CAP;
        let ttl = Duration::from_secs(u64::from(ttl)).min(cap);
        self.cache.lock().expect("dns cache").insert(
            zone.to_string(),
            Cached::Positive {
                publisher,
                expires: now + ttl,
            },
        );
        Ok(ZoneOutcome::Resolved {
            publisher,
            newly_pinned,
        })
    }

    /// Remove the TOFU pin for `zone`; true if one existed. The next
    /// resolution re-pins whatever the zone then publishes (the
    /// deliberate rotation path after a key change).
    pub async fn unpin(&self, zone: &str) -> Result<bool> {
        let mut pins = self.pins.lock().await;
        let removed = pins.unpin(zone);
        pins.save(&self.paths)?;
        self.cache.lock().expect("dns cache").remove(zone);
        Ok(removed)
    }

    fn cache_negative(&self, zone: &str, now: Instant) {
        self.cache.lock().expect("dns cache").insert(
            zone.to_string(),
            Cached::Negative {
                expires: now + NEGATIVE_TTL,
            },
        );
    }
}
