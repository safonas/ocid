//! Gossip announcements.
//!
//! Announcements are deliberately tiny (they must fit in a gossip message):
//! they say *that* something was released, not *what*. Interested peers then
//! fetch the signed release record over the sync protocol from whoever
//! delivered the announcement (or the publisher) and verify it themselves.
//!
//! Two kinds of topics are used (see [`crate::p2p`]): the global swarm topic
//! for membership, and one topic per publisher for that publisher's
//! announcements. The same receive loop serves both; the [`Scope`] says which.

use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use iroh_gossip::api::{Event, GossipReceiver};
use n0_future::StreamExt;
use serde::{Deserialize, Serialize};

use ocid_core::{identity::PublisherId, release::ReleaseSummary};

use crate::node::Node;

/// Which topic a receive loop is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The global swarm topic: neighbors here are tracked as *the* neighbor set.
    Swarm,
    /// A publisher's announcement topic: only announcements from that
    /// publisher are accepted.
    Publisher(PublisherId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Announcement {
    /// A publisher released (or re-announced) `name:tag`.
    Release(ReleaseSummary),
}

impl Announcement {
    pub fn encode(&self) -> Result<Bytes> {
        Ok(postcard::to_stdvec(self)?.into())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

/// Receive loop: track neighbors, react to announcements.
pub async fn run(node: Arc<Node>, scope: Scope, mut receiver: GossipReceiver) {
    let topic = match scope {
        Scope::Swarm => "swarm".to_string(),
        Scope::Publisher(p) => p.fmt_short().to_string(),
    };
    while let Some(event) = receiver.next().await {
        let event = match event {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(%topic, "gossip receiver error: {e}");
                break;
            }
        };
        match event {
            Event::NeighborUp(id) => {
                tracing::info!(peer = %id.fmt_short(), %topic, "gossip: neighbor up");
                match scope {
                    Scope::Swarm => node.neighbor_up(id).await,
                    Scope::Publisher(_) => node.note_peer_seen(id).await,
                }
                let node = node.clone();
                tokio::spawn(async move {
                    if let Err(e) = node.sync_with(id).await {
                        tracing::debug!(peer = %id.fmt_short(), "initial sync failed: {e}");
                    }
                });
            }
            Event::NeighborDown(id) => {
                tracing::info!(peer = %id.fmt_short(), %topic, "gossip: neighbor down");
                if scope == Scope::Swarm {
                    node.neighbor_down(id).await;
                }
            }
            Event::Lagged => {
                tracing::warn!(%topic, "gossip: lagged, some announcements were dropped");
            }
            Event::Received(msg) => {
                let ann = match Announcement::decode(&msg.content) {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::debug!(%topic, "gossip: undecodable message: {e}");
                        continue;
                    }
                };
                match ann {
                    Announcement::Release(summary) => {
                        if let Scope::Publisher(p) = scope {
                            if summary.publisher != p {
                                tracing::debug!(
                                    %topic,
                                    from = %msg.delivered_from.fmt_short(),
                                    "gossip: dropping announcement for {} on another publisher's topic",
                                    summary.publisher.fmt_short()
                                );
                                continue;
                            }
                        }
                        tracing::info!(
                            from = %msg.delivered_from.fmt_short(),
                            "gossip: {}/{}:{} announced",
                            summary.publisher.fmt_short(),
                            summary.name,
                            summary.tag
                        );
                        let node = node.clone();
                        tokio::spawn(async move {
                            if let Err(e) = node.on_announcement(summary, msg.delivered_from).await
                            {
                                tracing::warn!("handling announcement failed: {e}");
                            }
                        });
                    }
                }
            }
        }
    }
    tracing::debug!(%topic, "gossip receive loop ended");
}
