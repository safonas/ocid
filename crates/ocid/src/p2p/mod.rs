//! Peer-to-peer layer built on iroh.
//!
//! * `iroh-blobs`  — verified streaming of content-addressed blobs (layers)
//! * `iroh-gossip` — tiny announcements: "publisher P released name:tag"
//! * `sync` — our own request/response protocol to fetch signed
//!   release records and inventories from a specific peer

pub mod gossip;
pub mod sync;

use iroh_gossip::proto::TopicId;
use ocid_core::identity::PublisherId;

/// ALPN for the ocid sync protocol.
pub const SYNC_ALPN: &[u8] = b"ocid/sync/1";

/// Global membership topic. Every ocid node joins it; it carries no
/// announcements, it only gives nodes neighbors to sync inventories with and
/// to bootstrap publisher topics from.
pub fn swarm_topic() -> TopicId {
    TopicId::from_bytes(*blake3::hash(b"ocid/swarm/v1").as_bytes())
}

/// Announcement topic of one publisher. A node subscribes to the topics of
/// its own key and of every publisher its policy follows, seeds or pins, so
/// it only receives announcements it may act on.
pub fn publisher_topic(publisher: &PublisherId) -> TopicId {
    let mut h = blake3::Hasher::new();
    h.update(b"ocid/publisher/v1/");
    h.update(publisher.as_bytes());
    TopicId::from_bytes(*h.finalize().as_bytes())
}

/// Upper bound for a single sync request/response body.
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;
