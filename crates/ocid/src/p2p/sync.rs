//! `ocid/sync/1`: a minimal request/response protocol over a QUIC bi-stream.
//!
//! Every request opens one bi-directional stream, writes a postcard-encoded
//! [`Request`], finishes the send side, and reads one [`Response`].

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use iroh::{
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
    Endpoint, EndpointAddr,
};
use serde::{Deserialize, Serialize};

use ocid_core::{
    identity::PublisherId,
    release::{Release, ReleaseSummary},
};

use crate::{metrics::SyncLabels, node::Node};

use super::MAX_MESSAGE;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Everything the peer seeds.
    Inventory,
    /// One release record.
    GetRelease {
        publisher: PublisherId,
        name: String,
        tag: String,
    },
    /// Tags of one image.
    ListTags {
        publisher: PublisherId,
        name: String,
    },
    /// Introduce ourselves so the peer can dial us back / bootstrap from us.
    Hello { addr: EndpointAddr },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Inventory(Vec<ReleaseSummary>),
    /// A signed release record as its exact JSON bytes. JSON (not postcard)
    /// because the record is what was signed, and because its schema has
    /// optional fields that positional encodings cannot skip.
    Release(Option<Vec<u8>>),
    Tags(Vec<String>),
    Ok,
    Error(String),
}

impl Response {
    /// Decode a `Release` response; errors on other variants.
    pub fn into_release(self) -> Result<Option<Release>> {
        match self {
            Response::Release(None) => Ok(None),
            Response::Release(Some(bytes)) => {
                let r: Release = serde_json::from_slice(&bytes).context("decoding release")?;
                Ok(Some(r))
            }
            other => Err(anyhow!("unexpected response {other:?}")),
        }
    }
}

/// Server side.
#[derive(Debug, Clone)]
pub struct SyncProtocol {
    node: Arc<Node>,
}

impl SyncProtocol {
    pub fn new(node: Arc<Node>) -> Self {
        Self { node }
    }

    async fn handle(&self, from: PublisherId, req: Request) -> Response {
        let kind = match &req {
            Request::Inventory => "inventory",
            Request::GetRelease { .. } => "get_release",
            Request::ListTags { .. } => "list_tags",
            Request::Hello { .. } => "hello",
        };
        self.node
            .metrics
            .sync_requests
            .get_or_create(&SyncLabels { request: kind })
            .inc();
        self.node.metrics.sync_requests_total.inc();
        match self.handle_inner(from, req).await {
            Ok(r) => r,
            Err(e) => Response::Error(e.to_string()),
        }
    }

    async fn handle_inner(&self, from: PublisherId, req: Request) -> Result<Response> {
        match req {
            Request::Inventory => {
                let mut out = Vec::new();
                for r in self.node.store.list_releases()? {
                    if self.node.store.is_complete(&r).await? {
                        out.push(ReleaseSummary::from(&r));
                    }
                }
                Ok(Response::Inventory(out))
            }
            Request::GetRelease {
                publisher,
                name,
                tag,
            } => {
                let rel = self.node.store.get_release(&publisher, &name, &tag)?;
                // Only hand out releases we can actually serve blobs for.
                let rel = match rel {
                    Some(r) if self.node.store.is_complete(&r).await? => {
                        Some(serde_json::to_vec(&r)?)
                    }
                    _ => None,
                };
                Ok(Response::Release(rel))
            }
            Request::ListTags { publisher, name } => Ok(Response::Tags(
                self.node.store.list_tags(&publisher, &name)?,
            )),
            Request::Hello { addr } => {
                if addr.id == from {
                    self.node.remember_peer(addr).await;
                }
                Ok(Response::Ok)
            }
        }
    }
}

impl ProtocolHandler for SyncProtocol {
    async fn accept(&self, connection: Connection) -> std::result::Result<(), AcceptError> {
        let from = connection.remote_id();
        tracing::debug!(peer = %from.fmt_short(), "sync: connection accepted");
        self.node.note_peer_seen(from).await;
        loop {
            let (mut send, mut recv) = match connection.accept_bi().await {
                Ok(s) => s,
                Err(_) => break, // connection closed
            };
            let this = self.clone();
            tokio::spawn(async move {
                let res: Result<()> = async {
                    let buf = recv.read_to_end(MAX_MESSAGE).await?;
                    let req: Request = postcard::from_bytes(&buf).context("decoding request")?;
                    tracing::debug!(peer = %from.fmt_short(), ?req, "sync: request");
                    let resp = this.handle(from, req).await;
                    let out = postcard::to_stdvec(&resp)?;
                    send.write_all(&out).await?;
                    send.finish()?;
                    Ok(())
                }
                .await;
                if let Err(e) = res {
                    tracing::debug!(peer = %from.fmt_short(), "sync: request failed: {e}");
                }
            });
        }
        connection.closed().await;
        Ok(())
    }
}

/// Client side: connect (or reuse `conn`) and perform one request.
pub async fn request(
    endpoint: &Endpoint,
    peer: impl Into<EndpointAddr>,
    req: &Request,
) -> Result<Response> {
    let conn = endpoint
        .connect(peer, super::SYNC_ALPN)
        .await
        .map_err(|e| anyhow!("connect: {e}"))?;
    let resp = request_on(&conn, req).await;
    conn.close(0u32.into(), b"done");
    resp
}

pub async fn request_on(conn: &Connection, req: &Request) -> Result<Response> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&postcard::to_stdvec(req)?).await?;
    send.finish()?;
    let buf = recv.read_to_end(MAX_MESSAGE).await?;
    let resp: Response = postcard::from_bytes(&buf).context("decoding response")?;
    if let Response::Error(e) = &resp {
        return Err(anyhow!("peer error: {e}"));
    }
    Ok(resp)
}
