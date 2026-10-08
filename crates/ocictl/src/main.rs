//! `ocictl` — control CLI for the ocid daemon.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use comfy_table::{presets::NOTHING, Table};
use ocid_core::{
    api::{
        AddPeerResp, AnnounceResp, DnsResolveResp, DnsUnpinReq, DnsUnpinResp, GcReport, OkResp,
        PeerInfo, PolicyChangeResp, ReleaseInfo, RmResp, Status, SyncResp,
    },
    client::Client,
    config::{Config, Mode, Policy},
    identity::{did_key, parse_publisher, Identity},
    index::Index,
    oci::ImageRef,
    paths::Paths,
    release::ReleaseSummary,
};

#[derive(Debug, Parser)]
#[command(
    name = "ocictl",
    version,
    about = "Control the ocid daemon: publish, seed, follow, pull OCI images over p2p",
    long_about = None,
)]
struct Cli {
    /// Node home directory.
    #[arg(long, env = "OCID_HOME", global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a node identity and default configuration.
    Init,
    /// Print this node's identity.
    Whoami,
    /// Print the signed DNS TXT record binding a zone to this node's
    /// publisher id — publish it as `_ocid.<zone>` to make
    /// `podman pull <registry>/<zone>/<name>` resolve to this node.
    DnsRecord {
        /// DNS zone this node's images will be served under.
        zone: String,
        /// Sign with this unix timestamp instead of now (tests and
        /// tooling; records older than the freshness window are rejected).
        #[arg(long)]
        ts: Option<u64>,
    },
    /// Resolve a DNS publisher name against the running daemon: which
    /// publisher it maps to, and the pin state (pins on first sight).
    Resolve { zone: String },
    /// Remove the locally pinned publisher key for a zone (deliberate
    /// key rotation; the next resolve re-pins whatever the zone publishes).
    DnsUnpin { zone: String },
    /// Show status of the running daemon.
    Status,
    /// Print this node's connection ticket (daemon must be running).
    Ticket,
    /// List known peers.
    Peers,
    /// Connect to a peer by ticket and remember it.
    Connect { ticket: String },
    /// List images held by this node.
    Ls,
    /// Fetch an image from the swarm into the local store.
    Pull { reference: String },
    /// (Re-)announce local releases to the swarm.
    Publish {
        /// Specific reference; default: everything published by this node.
        reference: Option<String>,
    },
    /// Seed an image: replicate it and serve it to peers.
    ///
    /// `<publisher>/<name>` keeps releases per --mode (default: latest);
    /// `<publisher>/<name>:<tag>` keeps exactly that tag.
    Seed {
        reference: String,
        #[command(flatten)]
        mode: ModeArgs,
    },
    /// Stop seeding an image (all its rules, or one tag rule).
    Unseed { reference: String },
    /// Follow a publisher: seed every image they publish, per --mode.
    Follow {
        publisher: String,
        #[command(flatten)]
        mode: ModeArgs,
    },
    /// Stop following a publisher.
    Unfollow { publisher: String },
    /// Pin a release: always kept and fetched, never garbage-collected.
    Pin { reference: String },
    /// Remove a pin.
    Unpin { reference: String },
    /// Create a local alias for a publisher or an image.
    ///
    ///   ocictl track <publisher> --as alice        -> localhost:5050/alice/<name>
    ///   ocictl track <publisher>/<name> --as app   -> localhost:5050/app
    Track {
        target: String,
        #[arg(long = "as")]
        alias: String,
    },
    /// Remove an alias.
    Untrack { alias: String },
    /// Show the seeding policy.
    Policy,
    /// Sync with peers now (all known peers, or one).
    Sync { peer: Option<String> },
    /// Remove an image release from this node (local only; not propagated).
    Rm {
        /// `<publisher>/<name>:<tag>` (or an alias). Without a tag, pass --all.
        reference: String,
        /// Remove every tag of the image.
        #[arg(long)]
        all: bool,
    },
    /// Garbage-collect: prune replicated releases the policy does not want
    /// and delete blobs no release references.
    Gc {
        /// Report only; remove nothing.
        #[arg(long)]
        dry_run: bool,
        /// Ignore the grace period for recently fetched releases.
        #[arg(long)]
        force: bool,
    },
}

/// `--full`, `--latest` or `--last N` (mutually exclusive; default latest).
#[derive(Debug, clap::Args)]
#[group(multiple = false)]
struct ModeArgs {
    /// Keep every release.
    #[arg(long)]
    full: bool,
    /// Keep only the most recent release (default).
    #[arg(long)]
    latest: bool,
    /// Keep the N most recent releases.
    #[arg(long, value_name = "N")]
    last: Option<u32>,
}

impl ModeArgs {
    fn mode(&self) -> Result<Mode> {
        Ok(match (self.full, self.latest, self.last) {
            (true, _, _) => Mode::Full,
            (_, _, Some(n)) => format!("last:{n}").parse()?,
            _ => Mode::Latest,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::resolve(cli.home)?;
    run(cli.command, &paths).await
}

async fn run(command: Command, paths: &Paths) -> Result<()> {
    match command {
        Command::Init => init(paths),
        Command::Whoami => whoami(paths),
        Command::DnsRecord { zone, ts } => dns_record(paths, &zone, ts),
        Command::Resolve { zone } => dns_resolve(paths, &zone).await,
        Command::DnsUnpin { zone } => dns_unpin(paths, &zone).await,
        Command::Status => {
            let s: Status = client(paths)?.get("/_ocid/status").await?;
            println!("version    {}", s.version);
            println!("id         {}", s.id);
            println!("did        {}", s.did);
            println!("registry   {}", s.registry);
            println!("uptime     {}s", s.uptime_secs);
            println!("neighbors  {}", s.neighbors.len());
            println!("peers      {}", s.known_peers);
            println!("releases   {}", s.releases);
            println!("seeds      {}", s.seeds.len());
            println!("follows    {}", s.follows.len());
            println!("pins       {}", s.pins.len());
            println!("ticket     {}", s.ticket);
            Ok(())
        }
        Command::Ticket => {
            let s: Status = client(paths)?.get("/_ocid/status").await?;
            println!("{}", s.ticket);
            Ok(())
        }
        Command::Peers => {
            let peers: Vec<PeerInfo> = client(paths)?.get("/_ocid/peers").await?;
            if peers.is_empty() {
                println!("no peers known (use `ocictl connect <ticket>`)");
                return Ok(());
            }
            let mut t = table(["PEER", "DID", "NEIGHBOR", "KNOWN", "LAST SEEN"]);
            for p in peers {
                t.add_row([
                    p.id.to_string(),
                    did_key(&p.id),
                    yes_no(p.neighbor),
                    yes_no(p.known),
                    p.last_seen_secs
                        .map(|s| format!("{s}s ago"))
                        .unwrap_or_else(|| "-".into()),
                ]);
            }
            println!("{t}");
            Ok(())
        }
        Command::Connect { ticket } => {
            let v: AddPeerResp = client(paths)?
                .post("/_ocid/peers", &serde_json::json!({ "ticket": ticket }))
                .await?;
            println!("connected to {}", v.id);
            Ok(())
        }
        Command::Ls => ls(paths).await,
        Command::Pull { reference } => {
            let s: ReleaseSummary = client(paths)?
                .post(
                    "/_ocid/pull",
                    &serde_json::json!({ "reference": reference }),
                )
                .await?;
            let cfg = Config::load(paths)?;
            println!(
                "{}/{}:{}  {}",
                s.publisher, s.name, s.tag, s.manifest_digest
            );
            println!(
                "podman pull {}/{}/{}:{}",
                cfg.listen, s.publisher, s.name, s.tag
            );
            Ok(())
        }
        Command::Publish { reference } => {
            let v: AnnounceResp = client(paths)?
                .post(
                    "/_ocid/announce",
                    &serde_json::json!({ "reference": reference }),
                )
                .await?;
            println!("announced {} release(s)", v.announced);
            Ok(())
        }
        Command::Seed { reference, mode } => {
            let mode = mode.mode()?;
            if let Some(c) = live_client(paths).await {
                let v: PolicyChangeResp = c
                    .post(
                        "/_ocid/policy/seed",
                        &serde_json::json!({ "reference": reference, "mode": mode }),
                    )
                    .await?;
                let how = tag_or_mode(&v.reference, mode);
                println!(
                    "{} {} ({how})",
                    if v.changed {
                        "seeding"
                    } else {
                        "already seeding"
                    },
                    v.reference
                );
                sync_all(&c).await;
            } else {
                offline_domain_check(&reference)?;
                let id = Identity::load(paths)?;
                let mut policy = Policy::load(paths)?;
                let r = ImageRef::parse(&reference, &policy, &id.id())?;
                if r.publisher == id.id() {
                    bail!("{r} is published by this node; it is always seeded");
                }
                let changed = policy.add_seed(&r.to_string(), mode)?;
                policy.save(paths)?;
                println!(
                    "{} {r} ({})",
                    if changed {
                        "seeding"
                    } else {
                        "already seeding"
                    },
                    tag_or_mode(&r.to_string(), mode)
                );
                offline_note();
            }
            Ok(())
        }
        Command::Unseed { reference } => {
            if let Some(c) = live_client(paths).await {
                let v: PolicyChangeResp = c
                    .post(
                        "/_ocid/policy/unseed",
                        &serde_json::json!({ "reference": reference }),
                    )
                    .await?;
                println!(
                    "{} {}",
                    if v.changed {
                        "no longer seeding"
                    } else {
                        "was not seeding"
                    },
                    v.reference
                );
            } else {
                offline_domain_check(&reference)?;
                let id = Identity::load(paths)?;
                let mut policy = Policy::load(paths)?;
                let r = ImageRef::parse(&reference, &policy, &id.id())?;
                let removed = policy.remove_seed(&r.to_string())?;
                policy.save(paths)?;
                println!(
                    "{} {r}",
                    if removed {
                        "no longer seeding"
                    } else {
                        "was not seeding"
                    }
                );
                offline_note();
            }
            Ok(())
        }
        Command::Follow { publisher, mode } => {
            let mode = mode.mode()?;
            if let Some(c) = live_client(paths).await {
                let v: PolicyChangeResp = c
                    .post(
                        "/_ocid/policy/follow",
                        &serde_json::json!({ "publisher": publisher, "mode": mode }),
                    )
                    .await?;
                // The daemon echoes the rule back in the form it stored
                // (domain form kept when a DNS name was given).
                println!(
                    "{} {} mode {mode}",
                    if v.changed {
                        "following"
                    } else {
                        "already following"
                    },
                    v.reference
                );
                sync_all(&c).await;
            } else {
                offline_domain_check(&publisher)?;
                let p = parse_publisher(&publisher)?;
                let mut policy = Policy::load(paths)?;
                let changed = policy.add_follow(&p.to_string(), mode)?;
                policy.save(paths)?;
                println!(
                    "{} {p} ({}) mode {mode}",
                    if changed {
                        "following"
                    } else {
                        "already following"
                    },
                    did_key(&p)
                );
                offline_note();
            }
            Ok(())
        }
        Command::Pin { reference } => {
            if let Some(c) = live_client(paths).await {
                let v: PolicyChangeResp = c
                    .post(
                        "/_ocid/policy/pin",
                        &serde_json::json!({ "reference": reference }),
                    )
                    .await?;
                println!(
                    "{} {}",
                    if v.changed {
                        "pinned"
                    } else {
                        "already pinned"
                    },
                    v.reference
                );
                sync_all(&c).await;
            } else {
                offline_domain_check(&reference)?;
                let id = Identity::load(paths)?;
                let mut policy = Policy::load(paths)?;
                let r = ImageRef::parse(&reference, &policy, &id.id())?;
                if r.tag.is_none() {
                    bail!("a pin needs a tag: {r}:<tag>");
                }
                let added = policy.add_pin(&r.to_string())?;
                policy.save(paths)?;
                println!("{} {r}", if added { "pinned" } else { "already pinned" });
                offline_note();
            }
            Ok(())
        }
        Command::Unpin { reference } => {
            if let Some(c) = live_client(paths).await {
                let v: PolicyChangeResp = c
                    .post(
                        "/_ocid/policy/unpin",
                        &serde_json::json!({ "reference": reference }),
                    )
                    .await?;
                println!(
                    "{} {}",
                    if v.changed {
                        "unpinned"
                    } else {
                        "was not pinned"
                    },
                    v.reference
                );
            } else {
                offline_domain_check(&reference)?;
                let id = Identity::load(paths)?;
                let mut policy = Policy::load(paths)?;
                let r = ImageRef::parse(&reference, &policy, &id.id())?;
                let removed = policy.remove_pin(&r.to_string())?;
                policy.save(paths)?;
                println!(
                    "{} {r}",
                    if removed {
                        "unpinned"
                    } else {
                        "was not pinned"
                    }
                );
                offline_note();
            }
            Ok(())
        }
        Command::Unfollow { publisher } => {
            if let Some(c) = live_client(paths).await {
                let v: PolicyChangeResp = c
                    .post(
                        "/_ocid/policy/unfollow",
                        &serde_json::json!({ "publisher": publisher }),
                    )
                    .await?;
                println!(
                    "{} {}",
                    if v.changed {
                        "unfollowed"
                    } else {
                        "was not following"
                    },
                    v.reference
                );
            } else {
                offline_domain_check(&publisher)?;
                let p = parse_publisher(&publisher)?;
                let mut policy = Policy::load(paths)?;
                let removed = policy.remove_follow(&p.to_string())?;
                policy.save(paths)?;
                println!(
                    "{} {p}",
                    if removed {
                        "unfollowed"
                    } else {
                        "was not following"
                    }
                );
                offline_note();
            }
            Ok(())
        }
        Command::Track { target, alias } => {
            let mut policy = Policy::load(paths)?;
            policy.set_alias(&alias, &target)?;
            policy.save(paths)?;
            let cfg = Config::load(paths)?;
            println!("{alias} -> {}", policy.alias[&alias]);
            println!("use it as {}/{alias}[/<name>]:<tag>", cfg.listen);
            reload(paths).await;
            Ok(())
        }
        Command::Untrack { alias } => {
            let mut policy = Policy::load(paths)?;
            let removed = policy.remove_alias(&alias);
            policy.save(paths)?;
            println!(
                "{}",
                if removed {
                    "alias removed"
                } else {
                    "no such alias"
                }
            );
            reload(paths).await;
            Ok(())
        }
        Command::Policy => {
            let policy = Policy::load(paths)?;
            print!("{}", toml::to_string_pretty(&policy)?);
            Ok(())
        }
        Command::Sync { peer } => {
            let v: SyncResp = client(paths)?
                .post("/_ocid/sync", &serde_json::json!({ "peer": peer }))
                .await?;
            println!("synced with {} peer(s)", v.synced);
            for f in v.failed {
                println!("failed: {f}");
            }
            Ok(())
        }
        Command::Rm { reference, all } => {
            let v: RmResp = client(paths)?
                .post(
                    "/_ocid/rm",
                    &serde_json::json!({ "reference": reference, "all_tags": all }),
                )
                .await?;
            if v.removed.is_empty() {
                println!("nothing removed");
            }
            for r in &v.removed {
                println!("removed {r}");
            }
            if v.still_wanted {
                println!("note: the policy still seeds/follows this image; it will be replicated again (see `ocictl unseed` / `unfollow`)");
            }
            Ok(())
        }
        Command::Gc { dry_run, force } => {
            let r: GcReport = client(paths)?
                .post(
                    "/_ocid/gc",
                    &serde_json::json!({ "dry_run": dry_run, "force": force }),
                )
                .await?;
            let verb = if r.dry_run { "would remove" } else { "removed" };
            for rel in &r.releases_removed {
                println!("{verb} release {rel}");
            }
            println!(
                "{verb} {} release(s), {} blob(s), {} freed{}",
                r.releases_removed.len(),
                r.blobs_removed,
                human_size(r.bytes_freed),
                if r.uploads_removed > 0 {
                    format!(", {} stale upload(s)", r.uploads_removed)
                } else {
                    String::new()
                }
            );
            Ok(())
        }
    }
}

fn init(paths: &Paths) -> Result<()> {
    if paths.is_initialized() {
        let id = Identity::load(paths)?;
        println!("already initialized at {}", paths.home.display());
        println!("id   {}", id.id());
        println!("did  {}", id.did());
        return Ok(());
    }
    paths.ensure_dirs()?;
    let id = Identity::generate();
    id.save(paths)?;
    Config::default().save(paths)?;
    Policy::default().save(paths)?;
    println!("initialized {}", paths.home.display());
    println!("id   {}", id.id());
    println!("did  {}", id.did());
    println!();
    println!("next: start `ocid`, then `podman push <image> 127.0.0.1:5050/<name>:<tag>`");
    Ok(())
}

fn whoami(paths: &Paths) -> Result<()> {
    let id = Identity::load(paths)?;
    println!("id    {}", id.id());
    println!("did   {}", id.did());
    println!("home  {}", paths.home.display());
    Ok(())
}

/// `ocictl dns-record [--ts <unix>] <zone>` — print the signed TXT record
/// binding `zone` to this node's publisher id. Fully offline: signs with
/// the local identity; publishing into the DNS zone is the admin's job.
fn dns_record(paths: &Paths, zone: &str, ts: Option<u64>) -> Result<()> {
    let id = Identity::load(paths)?;
    let rec = match ts {
        Some(ts) => ocid_core::dns::DnsRecord::sign_at(zone, &id, ts)?,
        None => ocid_core::dns::DnsRecord::new(zone, &id)?,
    };
    let cfg = Config::load(paths)?;
    let scheme = match cfg.tls {
        ocid_core::config::TlsMode::Auto => "https",
        ocid_core::config::TlsMode::Off => "http",
    };
    println!("Publish this record in the {zone} zone:");
    println!();
    println!("  _ocid.{zone}. 300 IN TXT \"{}\"", rec.to_txt());
    println!();
    println!("Once it propagates, pulls use the name instead of hex:");
    println!();
    println!(
        "  podman pull {scheme}://{}/{zone}/<name>:<tag>",
        cfg.listen
    );
    println!();
    println!("The signature binds the record to this zone and publisher");
    println!("  ({}; did:key form {}).", id.id(), did_key(&id.id()));
    println!("Peers verify it and pin the key locally on first use; re-run");
    println!("and re-publish to refresh the timestamp when it ages out.");
    Ok(())
}

/// `ocictl resolve <zone>` — ask the daemon what a publisher name maps to.
/// Pins the key on first verified sight (TOFU); a mismatch or stale record
/// is reported as an error, never silently trusted.
async fn dns_resolve(paths: &Paths, zone: &str) -> Result<()> {
    let c = client(paths)?;
    // The zone is a validated DNS name (lowercase hostname charset), so it
    // needs no percent-encoding; build the query string directly.
    let resp: DnsResolveResp = c.get(&format!("/_ocid/dns/resolve?zone={zone}")).await?;
    match resp.state.as_str() {
        "new" | "pinned" => {
            let publisher = resp.publisher.context("missing publisher")?;
            println!("zone       {}", resp.zone);
            println!("publisher  {publisher}");
            println!("state      {} (verified, fresh, TOFU-pinned)", resp.state);
            println!();
            println!("pulls via:  podman pull <registry>/{zone}/<name>:<tag>");
        }
        "no-record" => {
            println!("zone       {}", resp.zone);
            println!("state      no record — falls through to the name rules");
        }
        "disabled" => {
            println!("zone       {}", resp.zone);
            println!("state      disabled (dns = \"off\" in config.toml)");
        }
        "error" => {
            println!("zone       {}", resp.zone);
            println!("state      error");
            println!();
            println!("{}", resp.detail.context("missing error detail")?);
            println!();
            println!(
                "if this key change is intentional: ocictl dns-unpin {}",
                resp.zone
            );
            bail!("DNS resolution failed for {}", resp.zone);
        }
        other => bail!("daemon returned unknown dns state {other:?}"),
    }
    Ok(())
}

/// `ocictl dns-unpin <zone>` — drop the TOFU pin (deliberate rotation).
async fn dns_unpin(paths: &Paths, zone: &str) -> Result<()> {
    let c = client(paths)?;
    let resp: DnsUnpinResp = c
        .post(
            "/_ocid/dns/unpin",
            &DnsUnpinReq {
                zone: zone.to_string(),
            },
        )
        .await?;
    if resp.removed {
        println!("unpinned {zone} — the next resolve re-pins whatever the zone publishes");
    } else {
        println!("no pin existed for {zone}");
    }
    Ok(())
}

async fn ls(paths: &Paths) -> Result<()> {
    let cfg = Config::load(paths)?;
    let policy = Policy::load(paths)?;
    let c = Client::from_config(&cfg, paths);
    let (releases, offline): (Vec<ReleaseInfo>, bool) = if c.is_running().await {
        (c.get("/_ocid/releases").await?, false)
    } else {
        // Daemon not running: read the on-disk index directly. Completeness
        // is approximated by "every blob has an index entry".
        let id = Identity::load(paths)?;
        let index = Index::open(paths)?;
        let out = index
            .list_releases()?
            .into_iter()
            .map(|r| ReleaseInfo {
                complete: index.is_indexed(&r),
                size: r.total_size(),
                blobs: r.all_blobs().count(),
                mine: r.publisher() == &id.id(),
                summary: ReleaseSummary::from(&r),
            })
            .collect();
        (out, true)
    };
    if releases.is_empty() {
        println!(
            "no images. push one: podman push <image> {}/<name>:<tag>",
            cfg.listen
        );
        return Ok(());
    }
    // reverse alias map for nicer display
    let mut aliases: std::collections::HashMap<String, String> = Default::default();
    for (alias, target) in &policy.alias {
        aliases.insert(target.clone(), alias.clone());
    }
    let mut t = table([
        "PUBLISHER",
        "IMAGE",
        "TAG",
        "DIGEST",
        "SIZE",
        "BLOBS",
        "STATE",
        "POLICY",
    ]);
    for r in releases {
        let pubs = r.summary.publisher.to_string();
        let rule = if r.mine {
            "own".to_string()
        } else if policy.is_pinned(&r.summary.publisher, &r.summary.name, &r.summary.tag) {
            "pin".to_string()
        } else {
            policy
                .window(&r.summary.publisher, &r.summary.name)
                .describe()
        };
        let publisher = if r.mine {
            "(me)".to_string()
        } else if let Some(a) = aliases.get(&pubs) {
            a.clone()
        } else {
            format!("{}…", &pubs[..12])
        };
        let image = aliases
            .get(&format!("{pubs}/{}", r.summary.name))
            .map(|a| format!("{a} ({})", r.summary.name))
            .unwrap_or_else(|| r.summary.name.clone());
        t.add_row([
            publisher,
            image,
            r.summary.tag.clone(),
            format!("{}…", &r.summary.manifest_digest.hex()[..12]),
            human_size(r.size),
            r.blobs.to_string(),
            if r.complete { "complete" } else { "partial" }.to_string(),
            rule,
        ]);
    }
    println!("{t}");
    if offline {
        println!("(daemon not running; listing from on-disk index)");
    }
    Ok(())
}

fn client(paths: &Paths) -> Result<Client> {
    let cfg = Config::load(paths).context("loading config")?;
    Ok(Client::from_config(&cfg, paths))
}

/// A client for a running daemon, or `None` when it is not reachable: policy
/// commands then fall back to editing `policy.toml` locally.
async fn live_client(paths: &Paths) -> Option<Client> {
    let c = client(paths).ok()?;
    c.is_running().await.then_some(c)
}

/// Pull content per the (just-changed) policy from all known peers.
async fn sync_all(c: &Client) {
    if let Ok(v) = c
        .post::<SyncResp>("/_ocid/sync", &serde_json::json!({ "peer": null }))
        .await
    {
        println!("synced with {} peer(s)", v.synced);
        for f in v.failed {
            println!("failed: {f}");
        }
    }
}

fn offline_note() {
    println!("(daemon not running; will take effect when it starts)");
}

/// Reject domain-form policy arguments on the offline path: without a
/// daemon there is no DNS resolution, so a domain cannot be mapped to a
/// publisher key here. The daemon resolves it (and pins it) when online.
fn offline_domain_check(arg: &str) -> Result<()> {
    let first = arg.split_once('/').map(|(f, _)| f).unwrap_or(arg);
    if ocid_core::dns::is_domain_name(first) {
        bail!(
            "{first:?} is a DNS publisher name; run the daemon to resolve it \
             (ocictl resolve {first}) or use the publisher id / an alias"
        );
    }
    Ok(())
}

/// `"this tag"` for a tagged rule, `"mode <mode>"` otherwise: the canonical
/// rule format only ever has a colon in front of the tag.
fn tag_or_mode(reference: &str, mode: Mode) -> String {
    if reference.contains(':') {
        "this tag".to_string()
    } else {
        format!("mode {mode}")
    }
}

/// Ask a running daemon to reload policy; silently ignore if not running.
async fn reload(paths: &Paths) {
    if let Ok(c) = client(paths) {
        if c.is_running().await {
            let _: Result<OkResp> = c.post("/_ocid/policy/reload", &serde_json::json!({})).await;
        }
    }
}

fn table<const N: usize>(headers: [&str; N]) -> Table {
    let mut t = Table::new();
    t.load_style(NOTHING);
    t.set_header(headers);
    t
}

fn yes_no(b: bool) -> String {
    if b { "yes" } else { "no" }.to_string()
}

fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}
