//! The running node: identity + store + iroh endpoint + gossip + policy.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use iroh::{
    address_lookup::memory::MemoryLookup, endpoint::presets, protocol::Router, Endpoint,
    EndpointAddr, EndpointId, RelayMode,
};
use iroh_blobs::{
    api::downloader::{Downloader, Shuffled},
    BlobsProtocol,
};
use iroh_gossip::{
    api::GossipSender,
    net::{Gossip, GOSSIP_ALPN},
};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use iroh_tickets::endpoint::EndpointTicket;
use ocid_core::{
    api::{DaemonEvent, GcReport, GcReq, PeerInfo, RmResp, Status},
    config::{self, Config, Peers, Policy},
    hash::Blake3,
    identity::{did_key, Identity, PublisherId},
    oci::Digest,
    paths::Paths,
    release::{BlobRef, Referrer, Release, ReleaseSummary},
};
use tokio::sync::{Mutex, RwLock};
use tokio::task::AbortHandle;

use crate::{
    metrics::{Exporter, Metrics},
    p2p::{
        self,
        gossip::{Announcement, Scope},
        sync::{self, Request, Response, SyncProtocol},
    },
    registry,
    store::{HashExt, Store},
};

pub struct Node {
    pub paths: Paths,
    pub identity: Identity,
    pub config: Config,
    pub store: Store,
    pub endpoint: Endpoint,
    pub metrics: Arc<Metrics>,
    pub exporter: Option<Exporter>,
    lookup: MemoryLookup,
    downloader: Downloader,
    gossip: Gossip,
    /// Global membership topic.
    swarm: GossipSender,
    /// Per-publisher announcement topics we are subscribed to (always
    /// includes our own); see `sync_topics`.
    topics: Mutex<BTreeMap<PublisherId, TopicSub>>,
    policy: RwLock<Policy>,
    peers: RwLock<Peers>,
    neighbors: RwLock<BTreeSet<EndpointId>>,
    last_seen: RwLock<BTreeMap<EndpointId, Instant>>,
    inflight: Mutex<HashSet<String>>,
    /// Fetches hold this for reading, GC for writing: a sweep must never run
    /// while a download is between "complete" and "pinned".
    gc_lock: RwLock<()>,
    pub events: tokio::sync::broadcast::Sender<DaemonEvent>,
    started: Instant,
}

/// A live subscription to one publisher's announcement topic.
struct TopicSub {
    sender: GossipSender,
    receiver: AbortHandle,
}

impl Drop for TopicSub {
    fn drop(&mut self) {
        // Dropping the sender and stopping the receiver leaves the topic.
        self.receiver.abort();
    }
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("id", &self.identity.id())
            .finish()
    }
}

/// Overrides from the command line.
#[derive(Debug, Default, Clone)]
pub struct RunOptions {
    pub listen: Option<std::net::SocketAddr>,
    pub peers: Vec<String>,
    pub no_relay: bool,
    pub no_metrics: bool,
}

/// Start the node and run until ctrl-c.
pub async fn run(paths: Paths, opts: RunOptions) -> Result<()> {
    // --- identity / config -------------------------------------------------
    if !paths.is_initialized() {
        paths.ensure_dirs()?;
        let id = Identity::generate();
        id.save(&paths)?;
        Config::default().save(&paths)?;
        Policy::default().save(&paths)?;
        tracing::info!(
            "initialized new identity {} at {}",
            id.id(),
            paths.home.display()
        );
    }
    let identity = Identity::load(&paths)?;
    let mut config = Config::load(&paths)?;
    let mut config_changed = false;
    if let Some(l) = opts.listen {
        config_changed |= config.listen != l;
        config.listen = l;
    }
    if opts.no_relay {
        config_changed |= config.relay != config::RelayMode::Disabled;
        config.relay = config::RelayMode::Disabled;
    }
    if opts.no_metrics {
        config_changed |= config.metrics;
        config.metrics = false;
    }
    if config_changed {
        // Persist so the CLI knows where to reach this node.
        config.save(&paths)?;
    }
    let policy = Policy::load(&paths)?;
    let mut peers = Peers::load(&paths)?;
    for p in &opts.peers {
        let ticket: EndpointTicket = p
            .parse()
            .with_context(|| format!("invalid peer ticket {p:?}"))?;
        peers.upsert(ticket.endpoint_addr().clone());
    }
    peers.save(&paths)?;

    let store = Store::open(
        &paths,
        Duration::from_secs(config.blob_gc_interval_secs.max(5)),
    )
    .await?;

    // --- iroh endpoint -----------------------------------------------------
    let lookup = MemoryLookup::new();
    for p in &peers.peers {
        lookup.add_endpoint_info(p.clone());
    }
    let mut builder = Endpoint::builder(presets::N0)
        .secret_key(identity.secret().clone())
        .address_lookup(lookup.clone());
    if config.relay == config::RelayMode::Disabled {
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    if config.p2p_port != 0 {
        builder = builder.bind_addr(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.p2p_port))?;
    }
    let endpoint = builder
        .bind()
        .await
        .map_err(|e| anyhow!("binding iroh endpoint: {e}"))?;

    // mDNS: find other ocid nodes on the LAN, no relay or DNS needed.
    let mdns = if config.mdns {
        let mdns = MdnsAddressLookup::builder()
            .service_name("ocid")
            .build(endpoint.id())
            .map_err(|e| anyhow!("mdns: {e}"))?;
        endpoint
            .address_lookup()
            .map_err(|e| anyhow!("address lookup: {e}"))?
            .add(mdns.clone());
        Some(mdns)
    } else {
        None
    };

    // --- protocols ---------------------------------------------------------
    let blobs = BlobsProtocol::new(store.blobs(), None);
    let downloader = store.blobs().downloader(&endpoint);
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let bootstrap: Vec<EndpointId> = peers.peers.iter().map(|p| p.id).collect();
    let (swarm_tx, swarm_rx) = gossip
        .subscribe(p2p::swarm_topic(), bootstrap.clone())
        .await
        .map_err(|e| anyhow!("subscribing swarm topic: {e}"))?
        .split();

    // --- metrics -----------------------------------------------------------
    let metrics = Arc::new(Metrics::default());
    let exporter = config.metrics.then(|| {
        Exporter::new(
            metrics.clone(),
            endpoint.metrics(),
            gossip.metrics().clone(),
        )
    });

    let (events, _) = tokio::sync::broadcast::channel(512);

    let node = Arc::new(Node {
        paths: paths.clone(),
        identity,
        config: config.clone(),
        store,
        endpoint: endpoint.clone(),
        metrics,
        exporter,
        lookup,
        downloader,
        gossip: gossip.clone(),
        swarm: swarm_tx,
        topics: Mutex::new(BTreeMap::new()),
        policy: RwLock::new(policy),
        peers: RwLock::new(peers),
        neighbors: RwLock::new(BTreeSet::new()),
        last_seen: RwLock::new(BTreeMap::new()),
        inflight: Mutex::new(HashSet::new()),
        gc_lock: RwLock::new(()),
        events,
        started: Instant::now(),
    });

    let router = Router::builder(endpoint.clone())
        .accept(iroh_blobs::ALPN, blobs)
        .accept(GOSSIP_ALPN, gossip.clone())
        .accept(p2p::SYNC_ALPN, SyncProtocol::new(node.clone()))
        .spawn();

    tokio::spawn(p2p::gossip::run(node.clone(), Scope::Swarm, swarm_rx));
    node.sync_topics().await;

    if let Some(mdns) = mdns {
        let node = node.clone();
        tokio::spawn(async move {
            use n0_future::StreamExt;
            let mut events = mdns.subscribe().await;
            while let Some(ev) = events.next().await {
                if let DiscoveryEvent::Discovered { endpoint_info, .. } = ev {
                    let id = endpoint_info.endpoint_id;
                    if id == node.id() {
                        continue;
                    }
                    node.on_lan_peer(id).await;
                }
            }
        });
    }

    // --- local registry + control API --------------------------------------
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("binding registry on {}", config.listen))?;
    tokio::spawn(registry::serve(node.clone(), listener));

    // --- banner ------------------------------------------------------------
    if config.relay != config::RelayMode::Disabled {
        let _ = tokio::time::timeout(Duration::from_secs(5), endpoint.online()).await;
    }
    let id = node.identity.id();
    eprintln!("ocid {}", env!("CARGO_PKG_VERSION"));
    eprintln!("  id        {id}");
    eprintln!("  did       {}", did_key(&id));
    eprintln!("  registry  http://{}", config.listen);
    if config.metrics {
        eprintln!("  metrics   http://{}/metrics", config.listen);
    }
    if config.mdns {
        eprintln!("  mdns      on (_ocid._udp.local)");
    }
    eprintln!("  ticket    {}", node.ticket());
    eprintln!("  home      {}", paths.home.display());
    if !bootstrap.is_empty() {
        eprintln!("  peers     {}", bootstrap.len());
    }
    eprintln!();
    eprintln!("push:  podman push <image> {}/<name>:<tag>", config.listen);
    eprintln!(
        "pull:  podman pull {}/<publisher>/<name>:<tag>",
        config.listen
    );

    // Periodic garbage collection.
    if config.gc_interval_secs > 0 {
        let node = node.clone();
        let every = Duration::from_secs(config.gc_interval_secs);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                match node.gc(GcReq::default()).await {
                    Ok(r) if !r.releases_removed.is_empty() || r.blobs_removed > 0 => {
                        tracing::info!(
                            "gc: removed {} release(s), {} blob(s), {} bytes",
                            r.releases_removed.len(),
                            r.blobs_removed,
                            r.bytes_freed
                        );
                    }
                    Ok(_) => tracing::debug!("gc: nothing to do"),
                    Err(e) => tracing::warn!("gc failed: {e}"),
                }
            }
        });
    }

    // Initial sync with known peers (best effort, in background).
    for peer in bootstrap {
        let node = node.clone();
        tokio::spawn(async move {
            if let Err(e) = node.sync_with(peer).await {
                tracing::debug!(peer = %peer.fmt_short(), "bootstrap sync failed: {e}");
            }
        });
    }

    tokio::signal::ctrl_c().await?;
    eprintln!("shutting down");
    router.shutdown().await.ok();
    node.store.shutdown().await.ok();
    Ok(())
}

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------

impl Node {
    pub fn emit(&self, event: DaemonEvent) {
        let _ = self.events.send(event);
    }

    pub fn id(&self) -> PublisherId {
        self.identity.id()
    }

    pub fn ticket(&self) -> String {
        EndpointTicket::new(self.endpoint.addr()).to_string()
    }

    pub async fn policy(&self) -> Policy {
        self.policy.read().await.clone()
    }

    pub async fn reload_policy(self: &Arc<Self>) -> Result<()> {
        let p = Policy::load(&self.paths)?;
        *self.policy.write().await = p;
        tracing::info!("policy reloaded");
        self.sync_topics().await;
        // Windows may have shrunk: prune right away.
        if let Err(e) = self.enforce_all_windows().await {
            tracing::warn!("enforcing windows after reload: {e}");
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // gossip topics
    // -----------------------------------------------------------------------

    /// Bring the set of subscribed publisher topics in line with the policy:
    /// our own topic plus one per publisher we follow, seed or pin.
    pub async fn sync_topics(self: &Arc<Self>) {
        let mut wanted = self.policy().await.publishers();
        wanted.insert(self.id());
        let bootstrap = self.gossip_bootstrap().await;
        let mut topics = self.topics.lock().await;
        let stale: Vec<PublisherId> = topics
            .keys()
            .filter(|p| !wanted.contains(*p))
            .copied()
            .collect();
        for p in stale {
            topics.remove(&p);
            tracing::info!(publisher = %p.fmt_short(), "left publisher topic");
        }
        for p in wanted {
            if topics.contains_key(&p) {
                continue;
            }
            let sub = match self
                .gossip
                .subscribe(p2p::publisher_topic(&p), bootstrap.clone())
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(publisher = %p.fmt_short(), "subscribing publisher topic: {e}");
                    continue;
                }
            };
            let (sender, receiver) = sub.split();
            let task = tokio::spawn(p2p::gossip::run(
                self.clone(),
                Scope::Publisher(p),
                receiver,
            ));
            topics.insert(
                p,
                TopicSub {
                    sender,
                    receiver: task.abort_handle(),
                },
            );
            tracing::info!(publisher = %p.fmt_short(), "joined publisher topic");
        }
        self.metrics.gossip_topics.set(topics.len() as i64);
    }

    /// Peers to bootstrap a topic from: current neighbors plus every
    /// remembered peer. Peers not on the topic simply ignore the join.
    async fn gossip_bootstrap(&self) -> Vec<EndpointId> {
        let mut out: BTreeSet<EndpointId> = self.neighbors.read().await.iter().copied().collect();
        out.extend(self.peers.read().await.peers.iter().map(|p| p.id));
        out.remove(&self.id());
        out.into_iter().collect()
    }

    /// Introduce a newly reachable peer to every topic we are on.
    async fn join_topics(&self, id: EndpointId) {
        if let Err(e) = self.swarm.join_peers(vec![id]).await {
            tracing::debug!(peer = %id.fmt_short(), "swarm join failed: {e}");
        }
        for (p, sub) in self.topics.lock().await.iter() {
            if let Err(e) = sub.sender.join_peers(vec![id]).await {
                tracing::debug!(peer = %id.fmt_short(), publisher = %p.fmt_short(), "topic join failed: {e}");
            }
        }
    }

    pub async fn status(&self) -> Result<Status> {
        let policy = self.policy().await;
        Ok(Status {
            id: self.id(),
            did: self.identity.did(),
            ticket: self.ticket(),
            registry: format!("http://{}", self.config.listen),
            uptime_secs: self.started.elapsed().as_secs(),
            neighbors: self.neighbors.read().await.iter().copied().collect(),
            known_peers: self.peers.read().await.peers.len(),
            releases: self.store.list_releases()?.len(),
            seeds: policy
                .seed
                .iter()
                .map(|e| format!("{} [{}]", e.reference, e.mode))
                .collect(),
            follows: policy
                .follow
                .iter()
                .map(|e| format!("{} [{}]", e.publisher, e.mode))
                .collect(),
            pins: policy.pin.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    /// Refresh gauges before a scrape.
    pub async fn refresh_gauges(&self) {
        let m = &self.metrics;
        m.neighbors.set(self.neighbors.read().await.len() as i64);
        m.peers_known
            .set(self.peers.read().await.peers.len() as i64);
        m.releases
            .set(self.store.list_releases().map(|r| r.len()).unwrap_or(0) as i64);
        let policy = self.policy.read().await;
        m.policy_seeds.set(policy.seed.len() as i64);
        m.policy_follows.set(policy.follow.len() as i64);
        m.uptime_seconds
            .set(self.started.elapsed().as_secs() as i64);
    }

    pub async fn peer_infos(&self) -> Vec<PeerInfo> {
        let neighbors = self.neighbors.read().await.clone();
        let known: BTreeSet<EndpointId> =
            self.peers.read().await.peers.iter().map(|p| p.id).collect();
        let seen = self.last_seen.read().await.clone();
        let mut ids: BTreeSet<EndpointId> = BTreeSet::new();
        ids.extend(neighbors.iter().copied());
        ids.extend(known.iter().copied());
        ids.extend(seen.keys().copied());
        ids.into_iter()
            .map(|id| PeerInfo {
                id,
                neighbor: neighbors.contains(&id),
                known: known.contains(&id),
                last_seen_secs: seen.get(&id).map(|t| t.elapsed().as_secs()),
            })
            .collect()
    }

    pub async fn remember_peer(&self, addr: EndpointAddr) {
        if addr.id == self.id() {
            return;
        }
        self.lookup.add_endpoint_info(addr.clone());
        let mut peers = self.peers.write().await;
        let new = peers.upsert(addr.clone());
        if let Err(e) = peers.save(&self.paths) {
            tracing::warn!("saving peers: {e}");
        }
        if new {
            tracing::info!(peer = %addr.id.fmt_short(), "new peer remembered");
        }
    }

    /// Add a peer from a ticket at runtime and try to sync with it.
    pub async fn add_peer_ticket(&self, ticket: &str) -> Result<EndpointId> {
        let t: EndpointTicket = ticket.parse().context("invalid ticket")?;
        let addr = t.endpoint_addr().clone();
        let id = addr.id;
        self.remember_peer(addr).await;
        self.join_topics(id).await;
        Ok(id)
    }

    /// A node appeared on the LAN (mDNS): join gossip with it and sync once.
    pub async fn on_lan_peer(&self, id: EndpointId) {
        let first = self.last_seen.read().await.get(&id).is_none()
            && !self.neighbors.read().await.contains(&id);
        if !first {
            return;
        }
        tracing::info!(peer = %id.fmt_short(), "mdns: discovered ocid node on the local network");
        self.note_peer_seen(id).await;
        self.metrics.mdns_discovered.inc();
        self.join_topics(id).await;
        if let Err(e) = self.sync_with(id).await {
            tracing::debug!(peer = %id.fmt_short(), "mdns: sync failed: {e}");
        }
    }

    pub async fn note_peer_seen(&self, id: EndpointId) {
        self.last_seen.write().await.insert(id, Instant::now());
    }

    pub async fn neighbor_up(&self, id: EndpointId) {
        let new = self.neighbors.write().await.insert(id);
        self.note_peer_seen(id).await;
        if new {
            self.emit(DaemonEvent::PeerChange {
                id,
                connected: true,
            });
            // A swarm neighbor may be on publisher topics we could not
            // bootstrap yet (e.g. the publisher itself just came online).
            for sub in self.topics.lock().await.values() {
                let _ = sub.sender.join_peers(vec![id]).await;
            }
        }
    }

    pub async fn neighbor_down(&self, id: EndpointId) {
        if self.neighbors.write().await.remove(&id) {
            self.emit(DaemonEvent::PeerChange {
                id,
                connected: false,
            });
        }
    }

    /// Peers worth asking about `publisher`'s images.
    async fn candidate_peers(&self, publisher: &PublisherId) -> Vec<EndpointId> {
        let me = self.id();
        let mut out: Vec<EndpointId> = Vec::new();
        for id in self.neighbors.read().await.iter() {
            out.push(*id);
        }
        for p in &self.peers.read().await.peers {
            if !out.contains(&p.id) {
                out.push(p.id);
            }
        }
        if !out.contains(publisher) {
            out.push(*publisher);
        }
        out.retain(|id| id != &me);
        out
    }

    // -----------------------------------------------------------------------
    // replication
    // -----------------------------------------------------------------------

    /// Say hello, fetch the peer's inventory and replicate what policy wants.
    pub async fn sync_with(&self, peer: EndpointId) -> Result<()> {
        let conn = self
            .endpoint
            .connect(peer, p2p::SYNC_ALPN)
            .await
            .map_err(|e| anyhow!("connect {}: {e}", peer.fmt_short()))?;
        self.note_peer_seen(peer).await;
        let _ = sync::request_on(
            &conn,
            &Request::Hello {
                addr: self.endpoint.addr(),
            },
        )
        .await;
        let inv = match sync::request_on(&conn, &Request::Inventory).await? {
            Response::Inventory(v) => v,
            other => bail!("unexpected response {other:?}"),
        };
        tracing::debug!(peer = %peer.fmt_short(), "inventory: {} releases", inv.len());
        let policy = self.policy().await;

        // Group by image, then pick per image what the window allows.
        let mut by_repo: BTreeMap<(PublisherId, String), Vec<ReleaseSummary>> = BTreeMap::new();
        for s in inv {
            if s.publisher == self.id() {
                continue;
            }
            by_repo
                .entry((s.publisher, s.name.clone()))
                .or_default()
                .push(s);
        }
        for ((publisher, name), remote) in by_repo {
            let wanted = self.select_wanted(&policy, &publisher, &name, &remote)?;
            if wanted.is_empty() {
                continue;
            }
            for s in wanted {
                let rel = match sync::request_on(
                    &conn,
                    &Request::GetRelease {
                        publisher: s.publisher,
                        name: s.name.clone(),
                        tag: s.tag.clone(),
                    },
                )
                .await?
                .into_release()?
                {
                    Some(r) => r,
                    None => continue,
                };
                if let Err(e) = self.fetch_release(&rel, vec![peer]).await {
                    tracing::warn!("replicating {}: {e}", rel.reference());
                }
            }
            self.enforce_window(&publisher, &name).await?;
        }
        conn.close(0u32.into(), b"done");
        Ok(())
    }

    /// From a set of remotely known releases of one image, those we should
    /// fetch: inside the policy window (computed over local + remote
    /// knowledge) and newer than what we hold.
    fn select_wanted(
        &self,
        policy: &Policy,
        publisher: &PublisherId,
        name: &str,
        remote: &[ReleaseSummary],
    ) -> Result<Vec<ReleaseSummary>> {
        let window = policy.window(publisher, name);
        if window.is_empty() {
            return Ok(Vec::new());
        }
        let local: BTreeMap<String, u64> = self
            .local_releases_of(publisher, name)?
            .into_iter()
            .map(|r| (r.payload.tag.clone(), r.payload.timestamp))
            .collect();
        let known = local
            .iter()
            .map(|(t, ts)| (t.as_str(), *ts))
            .chain(remote.iter().map(|s| (s.tag.as_str(), s.timestamp)));
        let selected = window.select(known);
        Ok(remote
            .iter()
            .filter(|s| selected.contains(&s.tag))
            .filter(|s| local.get(&s.tag).is_none_or(|ts| s.timestamp > *ts))
            .cloned()
            .collect())
    }

    fn local_releases_of(&self, publisher: &PublisherId, name: &str) -> Result<Vec<Release>> {
        let mut out = Vec::new();
        for t in self.store.list_tags(publisher, name)? {
            if let Some(r) = self.store.get_release(publisher, name, &t)? {
                out.push(r);
            }
        }
        Ok(out)
    }

    pub async fn on_announcement(&self, s: ReleaseSummary, from: EndpointId) -> Result<()> {
        self.metrics.announcements_received.inc();
        self.note_peer_seen(from).await;
        self.emit(DaemonEvent::Gossip {
            publisher: s.publisher,
            name: s.name.clone(),
            tag: s.tag.clone(),
            outbound: false,
        });
        let policy = self.policy().await;
        let wanted =
            self.select_wanted(&policy, &s.publisher, &s.name, std::slice::from_ref(&s))?;
        if wanted.is_empty() {
            tracing::debug!(
                "ignoring announcement {}/{}:{} (outside policy)",
                s.publisher.fmt_short(),
                s.name,
                s.tag
            );
            return Ok(());
        }
        let providers = {
            let mut v = vec![from];
            if s.publisher != from {
                v.push(s.publisher);
            }
            v
        };
        for p in &providers {
            match sync::request(
                &self.endpoint,
                *p,
                &Request::GetRelease {
                    publisher: s.publisher,
                    name: s.name.clone(),
                    tag: s.tag.clone(),
                },
            )
            .await
            .and_then(Response::into_release)
            {
                Ok(Some(rel)) => {
                    self.fetch_release(&rel, providers.clone()).await?;
                    return self.enforce_window(&s.publisher, &s.name).await;
                }
                Ok(None) => continue,
                Err(e) => tracing::debug!(peer = %p.fmt_short(), "get release: {e}"),
            }
        }
        bail!(
            "no peer could provide release {}/{}:{}",
            s.publisher.fmt_short(),
            s.name,
            s.tag
        )
    }

    /// After an image changed: drop local releases that fell out of its
    /// window (pins always stay) and sweep their blobs.
    pub async fn enforce_window(&self, publisher: &PublisherId, name: &str) -> Result<()> {
        if publisher == &self.id() {
            return Ok(());
        }
        let policy = self.policy().await;
        let window = policy.window(publisher, name);
        if window.is_empty() {
            return Ok(()); // unmanaged (cache); periodic GC applies the grace period
        }
        let _guard = self.gc_lock.write().await;
        let local = self.local_releases_of(publisher, name)?;
        let selected = window.select(
            local
                .iter()
                .map(|r| (r.payload.tag.as_str(), r.payload.timestamp)),
        );
        let mut removed = 0;
        for r in &local {
            if selected.contains(&r.payload.tag) {
                continue;
            }
            if self.store.remove_release(publisher, name, &r.payload.tag)? {
                tracing::info!(
                    "pruned {} (outside window {})",
                    r.reference(),
                    window.describe()
                );
                self.emit(DaemonEvent::Pruned {
                    publisher: *publisher,
                    name: name.to_string(),
                    tag: r.payload.tag.clone(),
                    reason: format!("outside window {}", window.describe()),
                });
                removed += 1;
            }
        }
        if removed > 0 {
            self.metrics.gc_releases_removed.inc_by(removed);
            let (blobs, bytes) = self.sweep_blobs_locked(false, &HashSet::new()).await?;
            self.metrics.gc_blobs_removed.inc_by(blobs as u64);
            self.metrics.gc_bytes_freed.inc_by(bytes);
        }
        Ok(())
    }

    /// Apply windows to every image we hold (after a policy change).
    pub async fn enforce_all_windows(&self) -> Result<()> {
        let mut repos: BTreeSet<(PublisherId, String)> = BTreeSet::new();
        for r in self.store.list_releases()? {
            repos.insert((*r.publisher(), r.name().to_string()));
        }
        for (publisher, name) in repos {
            self.enforce_window(&publisher, &name).await?;
        }
        Ok(())
    }

    /// Verify a release, download all its blobs from `providers`, verify their
    /// sha256, index them, and store the release.
    pub async fn fetch_release(&self, release: &Release, providers: Vec<EndpointId>) -> Result<()> {
        release.verify()?;
        let reference = release.reference();

        // de-duplicate concurrent fetches of the same reference
        loop {
            let mut inflight = self.inflight.lock().await;
            if inflight.insert(reference.clone()) {
                break;
            }
            drop(inflight);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let res = {
            let _guard = self.gc_lock.read().await;
            self.fetch_release_inner(release, providers).await
        };
        self.inflight.lock().await.remove(&reference);
        match &res {
            Ok(()) => self.metrics.releases_replicated.inc(),
            Err(_) => self.metrics.releases_failed.inc(),
        };
        res
    }

    async fn fetch_release_inner(
        &self,
        release: &Release,
        providers: Vec<EndpointId>,
    ) -> Result<()> {
        let providers: Vec<EndpointId> =
            providers.into_iter().filter(|p| p != &self.id()).collect();
        let total = release.all_blobs().count();
        let mut fetched = 0usize;
        for b in release.all_blobs() {
            if self.store.has_hash(b.hash).await? {
                if self.store.blob_ref(&b.digest)?.is_none() {
                    self.store.verify_blob(b).await?;
                    self.store.record_blob(b).await?;
                }
                continue;
            }
            if providers.is_empty() {
                bail!("no providers for blob {}", b.digest);
            }
            tracing::info!(
                "fetching {} ({} bytes) for {}",
                b.digest,
                b.size,
                release.reference()
            );
            // Keep the blob-store GC away until the blob is pinned.
            let _protect = self.store.protect(b.hash).await?;
            self.downloader
                .download(b.hash.to_iroh(), Shuffled::new(providers.clone()))
                .await
                .map_err(|e| anyhow!("downloading {}: {e}", b.digest))?;
            self.store.verify_blob(b).await?;
            self.store.record_blob(b).await?;
            self.metrics.blobs_fetched.inc();
            self.metrics.blobs_fetched_bytes.inc_by(b.size);
            fetched += 1;
        }
        let stored = self.store.put_release(release)?;
        self.emit(DaemonEvent::ReleaseSaved {
            publisher: *release.publisher(),
            name: release.name().to_string(),
            tag: release.payload.tag.clone(),
            blobs: total,
        });
        tracing::info!(
            "replicated {} ({fetched}/{total} blobs fetched{})",
            release.reference(),
            if stored {
                ""
            } else {
                ", release already known"
            }
        );
        Ok(())
    }

    /// Return a complete local release, fetching it from peers if necessary.
    pub async fn get_or_fetch(
        &self,
        publisher: &PublisherId,
        name: &str,
        tag: &str,
    ) -> Result<Option<Release>> {
        if let Some(r) = self.store.get_release(publisher, name, tag)? {
            if self.store.is_complete(&r).await? {
                return Ok(Some(r));
            }
        }
        if publisher == &self.id() {
            return Ok(None);
        }
        let peers = self.candidate_peers(publisher).await;
        tracing::info!(
            "{}/{name}:{tag} not local, asking {} peer(s)",
            publisher.fmt_short(),
            peers.len()
        );
        let req = Request::GetRelease {
            publisher: *publisher,
            name: name.to_string(),
            tag: tag.to_string(),
        };
        for peer in peers {
            let res = tokio::time::timeout(
                Duration::from_secs(15),
                sync::request(&self.endpoint, peer, &req),
            )
            .await;
            match res.map(|r| r.and_then(Response::into_release)) {
                Ok(Ok(Some(rel))) => {
                    let mut providers = vec![peer];
                    if publisher != &peer {
                        providers.push(*publisher);
                    }
                    self.fetch_release(&rel, providers).await?;
                    return Ok(Some(rel));
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => tracing::debug!(peer = %peer.fmt_short(), "get release: {e}"),
                Err(_) => tracing::debug!(peer = %peer.fmt_short(), "get release: timeout"),
            }
        }
        Ok(None)
    }

    // -----------------------------------------------------------------------
    // publishing
    // -----------------------------------------------------------------------

    /// Store our own release and announce it.
    pub async fn publish(&self, release: &Release) -> Result<()> {
        release.verify()?;
        if release.publisher() != &self.id() {
            bail!("can only publish releases signed by this node");
        }
        self.store.put_release(release)?;
        self.metrics.releases_published.inc();
        self.emit(DaemonEvent::ReleaseSaved {
            publisher: *release.publisher(),
            name: release.name().to_string(),
            tag: release.payload.tag.clone(),
            blobs: release.payload.blobs.len(),
        });
        self.announce(release).await
    }

    /// Broadcast a release on its publisher's topic. Only our own releases
    /// are announced; replicas are relayed by gossip itself.
    pub async fn announce(&self, release: &Release) -> Result<()> {
        let publisher = *release.publisher();
        let sender = self
            .topics
            .lock()
            .await
            .get(&publisher)
            .map(|s| s.sender.clone())
            .ok_or_else(|| anyhow!("not subscribed to topic of {}", publisher.fmt_short()))?;
        let ann = Announcement::Release(ReleaseSummary::from(release));
        sender
            .broadcast(ann.encode()?)
            .await
            .map_err(|e| anyhow!("gossip broadcast: {e}"))?;
        self.metrics.announcements_sent.inc();
        self.emit(DaemonEvent::Gossip {
            publisher,
            name: release.name().to_string(),
            tag: release.payload.tag.clone(),
            outbound: true,
        });
        tracing::info!("announced {}", release.reference());
        Ok(())
    }

    /// Attach a referrer (signature, SBOM, ...) to every release of ours whose
    /// manifest is `subject`: extend the blob list, re-sign, re-publish.
    /// Returns how many releases were updated.
    pub async fn attach_referrer(
        &self,
        subject: &Digest,
        referrer: Referrer,
        blobs: Vec<BlobRef>,
    ) -> Result<usize> {
        let me = self.id();
        let mut n = 0;
        for rel in self.store.list_releases()? {
            if rel.publisher() != &me || &rel.payload.manifest.digest != subject {
                continue;
            }
            if rel
                .payload
                .referrers
                .iter()
                .any(|r| r.digest == referrer.digest)
            {
                continue;
            }
            let mut payload = rel.payload.clone();
            payload.referrers.push(referrer.clone());
            for b in &blobs {
                if !payload.blobs.iter().any(|x| x.digest == b.digest) {
                    payload.blobs.push(b.clone());
                }
            }
            payload.timestamp = ocid_core::release::now().max(rel.payload.timestamp + 1);
            let updated = Release::sign(&self.identity, payload)?;
            self.publish(&updated).await?;
            n += 1;
        }
        Ok(n)
    }

    /// Re-announce everything we publish (e.g. after restart or on request).
    pub async fn announce_all_local(&self) -> Result<usize> {
        let me = self.id();
        let mut n = 0;
        for r in self.store.list_releases()? {
            if r.publisher() == &me {
                self.announce(&r).await?;
                n += 1;
            }
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// deletion & garbage collection
// ---------------------------------------------------------------------------

impl Node {
    /// Releases we must keep: ours, pinned, or inside a policy window.
    /// Everything else is cache and subject to the GC grace period.
    fn retained_set(&self, policy: &Policy, releases: &[Release]) -> HashSet<String> {
        let me = self.id();
        let mut by_repo: BTreeMap<(PublisherId, String), Vec<&Release>> = BTreeMap::new();
        for r in releases {
            by_repo
                .entry((*r.publisher(), r.name().to_string()))
                .or_default()
                .push(r);
        }
        let mut keep = HashSet::new();
        for ((publisher, name), rels) in by_repo {
            if publisher == me {
                keep.extend(rels.iter().map(|r| r.reference()));
                continue;
            }
            let window = policy.window(&publisher, &name);
            if window.is_empty() {
                continue;
            }
            let selected = window.select(
                rels.iter()
                    .map(|r| (r.payload.tag.as_str(), r.payload.timestamp)),
            );
            keep.extend(
                rels.iter()
                    .filter(|r| selected.contains(&r.payload.tag))
                    .map(|r| r.reference()),
            );
        }
        keep
    }

    /// Remove release records. With `all_tags`, every tag of the image.
    /// Orphaned blobs are swept afterwards.
    pub async fn remove_release(
        &self,
        publisher: &PublisherId,
        name: &str,
        tag: Option<&str>,
    ) -> Result<RmResp> {
        let tags: Vec<String> = match tag {
            Some(t) => vec![t.to_string()],
            None => self.store.list_tags(publisher, name)?,
        };
        let mut removed = Vec::new();
        for t in tags {
            if self.store.remove_release(publisher, name, &t)? {
                removed.push(format!("{publisher}/{name}:{t}"));
            }
        }
        let policy = self.policy().await;
        let still_wanted = publisher != &self.id() && policy.covers(publisher, name);
        if !removed.is_empty() {
            let _ = self.sweep_blobs(false).await?;
        }
        Ok(RmResp {
            removed,
            still_wanted,
        })
    }

    /// Garbage collection:
    /// 1. prune replicated releases that policy does not want (past grace),
    /// 2. sweep blobs no release references.
    pub async fn gc(&self, req: GcReq) -> Result<GcReport> {
        let _guard = self.gc_lock.write().await;
        self.metrics.gc_runs.inc();
        let policy = self.policy().await;
        let grace = Duration::from_secs(self.config.gc_grace_secs);
        let mut report = GcReport {
            dry_run: req.dry_run,
            ..Default::default()
        };

        let releases = self.store.list_releases()?;
        let keep = self.retained_set(&policy, &releases);
        for r in releases {
            if keep.contains(&r.reference()) {
                continue;
            }
            if !req.force {
                let age = self
                    .store
                    .release_mtime(&r)
                    .and_then(|t| t.elapsed().ok())
                    .unwrap_or(Duration::MAX);
                if age < grace {
                    continue;
                }
            }
            report.releases_removed.push(r.reference());
            if !req.dry_run {
                self.store
                    .remove_release(r.publisher(), r.name(), &r.payload.tag)?;
            }
        }

        let pruned: HashSet<String> = report.releases_removed.iter().cloned().collect();
        let (blobs, bytes) = self.sweep_blobs_locked(req.dry_run, &pruned).await?;
        report.blobs_removed = blobs;
        report.bytes_freed = bytes;
        report.uploads_removed = self.prune_uploads(req.dry_run).await?;
        if !req.dry_run {
            self.store.prune_referrers()?;
        }

        if !req.dry_run {
            self.metrics
                .gc_releases_removed
                .inc_by(report.releases_removed.len() as u64);
            self.metrics.gc_blobs_removed.inc_by(blobs as u64);
            self.metrics.gc_bytes_freed.inc_by(bytes);
        }
        Ok(report)
    }

    /// Sweep orphaned blobs (takes the GC lock).
    pub async fn sweep_blobs(&self, dry_run: bool) -> Result<(usize, u64)> {
        let _guard = self.gc_lock.write().await;
        self.sweep_blobs_locked(dry_run, &HashSet::new()).await
    }

    /// `ignore` lists release references to treat as already removed (dry runs).
    async fn sweep_blobs_locked(
        &self,
        dry_run: bool,
        ignore: &HashSet<String>,
    ) -> Result<(usize, u64)> {
        // Everything any remaining release references stays.
        let mut live: HashSet<Blake3> = HashSet::new();
        for r in self.store.list_releases()? {
            if ignore.contains(&r.reference()) {
                continue;
            }
            live.extend(r.all_blobs().map(|b| b.hash));
        }
        let mut removed = 0usize;
        let mut bytes = 0u64;
        for (digest, hash) in self.store.list_pins().await? {
            if live.contains(&hash) {
                continue;
            }
            removed += 1;
            bytes += self.store.blob_ref(&digest)?.map(|b| b.size).unwrap_or(0);
            if !dry_run {
                self.store.unpin_blob(&digest).await?;
                self.store.remove_blob_ref(&digest)?;
            }
        }
        // Index entries whose pin is already gone (crash between the two steps).
        let pinned: HashSet<Blake3> = self
            .store
            .list_pins()
            .await?
            .into_iter()
            .map(|(_, h)| h)
            .collect();
        for b in self.store.list_blob_refs()? {
            if !live.contains(&b.hash) && !pinned.contains(&b.hash) && !dry_run {
                self.store.remove_blob_ref(&b.digest)?;
            }
        }
        // Unpinned data is deleted by the blob store's own GC on its next tick.
        Ok((removed, bytes))
    }

    /// Remove upload temp files older than a day.
    async fn prune_uploads(&self, dry_run: bool) -> Result<usize> {
        let mut n = 0;
        let Ok(rd) = std::fs::read_dir(self.paths.uploads()) else {
            return Ok(0);
        };
        for entry in rd.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(86400));
            if old {
                n += 1;
                if !dry_run {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(n)
    }
}
