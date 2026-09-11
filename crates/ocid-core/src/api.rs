//! Request/response types of the daemon's `/_ocid/` control API, shared with
//! `ocictl`.

use iroh_base::EndpointId;
use serde::{Deserialize, Serialize};

use crate::{identity::PublisherId, release::ReleaseSummary};

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
pub struct ErrorResp {
    pub error: String,
}
