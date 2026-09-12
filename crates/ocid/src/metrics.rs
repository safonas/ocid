//! Prometheus / OpenMetrics exposition for the daemon.
//!
//! `GET /metrics` on the registry port returns our own `ocid_*` metrics plus
//! the metric groups of the iroh endpoint and gossip (prefixed `iroh_*`).

use std::sync::Arc;

use iroh_metrics::{Counter, EncodeLabelSet, Family, Gauge, MetricsGroup, MetricsSource, Registry};
use ocid_core::api::MetricsSnapshot;

/// Labels for registry HTTP requests.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, EncodeLabelSet)]
pub struct RequestLabels {
    pub method: String,
    /// Coarse route family: root, manifests, blobs, uploads, tags, catalog,
    /// control, metrics, other.
    pub route: &'static str,
    pub status: u16,
}

/// Labels for the p2p sync protocol.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, EncodeLabelSet)]
pub struct SyncLabels {
    pub request: &'static str,
}

#[derive(Debug, Default, MetricsGroup)]
#[metrics(name = "ocid")]
pub struct Metrics {
    /// HTTP requests handled by the local registry and control API
    pub http_requests: Family<RequestLabels, Counter>,
    /// Total HTTP requests (sum over all label combinations; families cannot
    /// be iterated, so this is maintained alongside `http_requests` for the
    /// JSON snapshot consumed by `ocitop`).
    pub http_requests_total: Counter,
    /// Bytes of blob/manifest content served to local clients
    pub http_bytes_served: Counter,
    /// Bytes of blob content uploaded by local clients
    pub http_bytes_received: Counter,

    /// Releases published (signed) by this node
    pub releases_published: Counter,
    /// Releases replicated from peers
    pub releases_replicated: Counter,
    /// Failed release replications
    pub releases_failed: Counter,
    /// Blobs downloaded from peers
    pub blobs_fetched: Counter,
    /// Bytes downloaded from peers
    pub blobs_fetched_bytes: Counter,

    /// Gossip announcements received
    pub announcements_received: Counter,
    /// Gossip announcements broadcast
    pub announcements_sent: Counter,
    /// Sync protocol requests served, by request type
    pub sync_requests: Family<SyncLabels, Counter>,
    /// Total sync protocol requests (see `http_requests_total`).
    pub sync_requests_total: Counter,
    /// ocid nodes discovered on the local network via mDNS
    pub mdns_discovered: Counter,
    /// Publisher announcement topics currently subscribed (incl. our own)
    pub gossip_topics: Gauge,

    /// Garbage collection runs
    pub gc_runs: Counter,
    /// Release records removed by GC
    pub gc_releases_removed: Counter,
    /// Blobs removed by GC
    pub gc_blobs_removed: Counter,
    /// Bytes freed by GC
    pub gc_bytes_freed: Counter,

    /// Current gossip neighbors
    pub neighbors: Gauge,
    /// Known (remembered) peers
    pub peers_known: Gauge,
    /// Release records held locally
    pub releases: Gauge,
    /// Seed rules in the policy
    pub policy_seeds: Gauge,
    /// Followed publishers in the policy
    pub policy_follows: Gauge,
    /// Seconds since the daemon started
    pub uptime_seconds: Gauge,
}

/// The assembled registry, ready to be encoded on each scrape.
pub struct Exporter {
    registry: Registry,
}

impl Metrics {
    /// Point-in-time copy for `GET /_ocid/metrics`.
    ///
    /// Callers should invoke `Node::refresh_gauges` first so the gauges
    /// reflect current state rather than the last `/metrics` scrape.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            http_requests_total: self.http_requests_total.get(),
            http_bytes_served: self.http_bytes_served.get(),
            http_bytes_received: self.http_bytes_received.get(),
            releases_published: self.releases_published.get(),
            releases_replicated: self.releases_replicated.get(),
            releases_failed: self.releases_failed.get(),
            blobs_fetched: self.blobs_fetched.get(),
            blobs_fetched_bytes: self.blobs_fetched_bytes.get(),
            announcements_received: self.announcements_received.get(),
            announcements_sent: self.announcements_sent.get(),
            sync_requests_total: self.sync_requests_total.get(),
            mdns_discovered: self.mdns_discovered.get(),
            gossip_topics: self.gossip_topics.get(),
            gc_runs: self.gc_runs.get(),
            gc_releases_removed: self.gc_releases_removed.get(),
            gc_blobs_removed: self.gc_blobs_removed.get(),
            gc_bytes_freed: self.gc_bytes_freed.get(),
            neighbors: self.neighbors.get(),
            peers_known: self.peers_known.get(),
            releases: self.releases.get(),
            policy_seeds: self.policy_seeds.get(),
            policy_follows: self.policy_follows.get(),
            uptime_seconds: self.uptime_seconds.get(),
        }
    }
}

impl Exporter {
    pub fn new(
        ocid: Arc<Metrics>,
        endpoint: &iroh::metrics::EndpointMetrics,
        gossip: Arc<iroh_gossip::metrics::Metrics>,
    ) -> Self {
        let mut registry = Registry::default();
        registry.register(ocid);
        let iroh = registry.sub_registry_with_prefix("iroh");
        iroh.register_all(endpoint);
        iroh.register(gossip);
        Self { registry }
    }

    pub fn encode(&self) -> Result<String, iroh_metrics::Error> {
        self.registry.encode_openmetrics_to_string()
    }
}

pub const CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Map a request path to a coarse route family for labels.
pub fn route_family(path: &str) -> &'static str {
    if path == "/metrics" {
        return "metrics";
    }
    if path.starts_with("/_ocid/") {
        return "control";
    }
    let Some(rest) = path.strip_prefix("/v2") else {
        return "other";
    };
    if rest.is_empty() || rest == "/" {
        "root"
    } else if rest == "/_catalog" {
        "catalog"
    } else if rest.ends_with("/tags/list") {
        "tags"
    } else if rest.contains("/blobs/uploads") {
        "uploads"
    } else if rest.contains("/manifests/") {
        "manifests"
    } else if rest.contains("/referrers/") {
        "referrers"
    } else if rest.contains("/blobs/") {
        "blobs"
    } else {
        "other"
    }
}
