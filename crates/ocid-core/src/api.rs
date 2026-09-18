//! Request/response types of the daemon's `/_ocid/` control API, shared with
//! `ocictl`.

use iroh_base::EndpointId;
use serde::{Deserialize, Serialize};

use crate::{config::Mode, identity::PublisherId, release::ReleaseSummary};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub id: PublisherId,
    pub did: String,
    pub ticket: String,
    pub registry: String,
    pub uptime_secs: u64,
    pub neighbors: Vec<EndpointId>,
    pub known_peers: usize,
    pub releases: usize,
    pub seeds: Vec<String>,
    pub follows: Vec<String>,
    pub pins: Vec<String>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: EndpointId,
    pub neighbor: bool,
    pub known: bool,
    pub last_seen_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    #[serde(flatten)]
    pub summary: ReleaseSummary,
    pub complete: bool,
    pub size: u64,
    pub blobs: usize,
    pub mine: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddPeerReq {
    pub ticket: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddPeerResp {
    pub id: EndpointId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefReq {
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceReq {
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceResp {
    pub announced: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReq {
    pub peer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResp {
    pub synced: usize,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GcReq {
    /// Report only; remove nothing.
    #[serde(default)]
    pub dry_run: bool,
    /// Ignore the grace period for unseeded releases.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GcReport {
    pub dry_run: bool,
    /// References of releases removed (or that would be removed).
    pub releases_removed: Vec<String>,
    pub blobs_removed: usize,
    pub bytes_freed: u64,
    pub uploads_removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RmReq {
    pub reference: String,
    /// Remove every tag of the image (reference must not carry a tag).
    #[serde(default)]
    pub all_tags: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RmResp {
    pub removed: Vec<String>,
    /// The policy still wants this image; it will be replicated again unless unseeded.
    pub still_wanted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkResp {
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedReq {
    pub reference: String,
    #[serde(default)]
    pub mode: Mode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnseedReq {
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowReq {
    pub publisher: String,
    #[serde(default)]
    pub mode: Mode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnfollowReq {
    pub publisher: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinReq {
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnpinReq {
    pub reference: String,
}

/// Reply of the policy mutation endpoints: whether the policy file changed,
/// and the canonical rule (alias-resolved) it now covers — or no longer
/// covers, for the removal endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyChangeResp {
    pub changed: bool,
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResp {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_dtos_wire_format() {
        let req: SeedReq =
            serde_json::from_str(r#"{"reference": "abc/app", "mode": "last:3"}"#).unwrap();
        assert_eq!(req.mode, Mode::Last(3));

        // omitted mode defaults to latest, and serializes as a plain string
        let req: SeedReq = serde_json::from_str(r#"{"reference": "abc/app"}"#).unwrap();
        assert_eq!(req.mode, Mode::Latest);
        assert_eq!(
            serde_json::to_value(&req).unwrap()["mode"],
            serde_json::json!("latest")
        );

        let req: FollowReq = serde_json::from_str(r#"{"publisher": "abc"}"#).unwrap();
        assert_eq!(req.mode, Mode::Latest);
        serde_json::from_str::<PinReq>(r#"{"reference": "abc/app:1.0"}"#).unwrap();
        serde_json::from_str::<UnpinReq>(r#"{"reference": "abc/app:1.0"}"#).unwrap();
        serde_json::from_str::<UnseedReq>(r#"{"reference": "abc/app"}"#).unwrap();
        serde_json::from_str::<UnfollowReq>(r#"{"publisher": "abc"}"#).unwrap();

        let resp = PolicyChangeResp {
            changed: true,
            reference: "abc/app:1.0".to_string(),
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["changed"], serde_json::json!(true));
        assert_eq!(v["reference"], serde_json::json!("abc/app:1.0"));
    }
}

/// Real-time event emitted by the daemon over the `GET /_ocid/events` SSE stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonEvent {
    /// A release announcement was received or broadcast on gossip.
    Gossip {
        publisher: PublisherId,
        name: String,
        tag: String,
        outbound: bool,
    },
    /// A release was pruned or kept by window enforcement.
    Pruned {
        publisher: PublisherId,
        name: String,
        tag: String,
        reason: String,
    },
    /// A release was successfully replicated or published.
    ReleaseSaved {
        publisher: PublisherId,
        name: String,
        tag: String,
        blobs: usize,
    },
    /// Incremental progress of a release being fetched from peers.
    /// Terminal state is `release_saved` (success) or `fetch_failed`.
    PullProgress {
        publisher: PublisherId,
        name: String,
        tag: String,
        blobs_done: usize,
        blobs_total: usize,
        bytes_done: u64,
        bytes_total: u64,
    },
    /// A release fetch from peers failed.
    FetchFailed {
        publisher: PublisherId,
        name: String,
        tag: String,
        error: String,
    },
    /// A peer connected or disconnected.
    PeerChange { id: EndpointId, connected: bool },
    /// An HTTP request was processed by the registry or control API.
    HttpRequest {
        method: String,
        path: String,
        status: u16,
    },
}
