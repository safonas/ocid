//! `ocid` — the daemon. Runs the p2p endpoint, replication, and the local
//! OCI registry. Control it with `ocictl`.

mod metrics;
mod node;
mod p2p;
mod registry;
mod store;

use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use ocid_core::paths::Paths;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "ocid",
    version,
    about = "ocid daemon: local-first, peer-to-peer OCI image distribution",
    long_about = None,
)]
struct Args {
    /// Node home directory.
    #[arg(long, env = "OCID_HOME")]
    home: Option<PathBuf>,
    /// Registry/control/metrics listen address (default from config, 127.0.0.1:5050).
    #[arg(long, env = "OCID_LISTEN")]
    listen: Option<SocketAddr>,
    /// Bootstrap peer ticket(s) (from `ocictl ticket` on another node).
    #[arg(long = "peer")]
    peers: Vec<String>,
    /// Disable relay servers (direct/LAN connections only).
    #[arg(long)]
    no_relay: bool,
    /// Do not expose `GET /metrics`.
    #[arg(long)]
    no_metrics: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("ocid=info,warn")),
        )
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    let args = Args::parse();
    let paths = Paths::resolve(args.home)?;
    node::run(
        paths,
        node::RunOptions {
            listen: args.listen,
            peers: args.peers,
            no_relay: args.no_relay,
            no_metrics: args.no_metrics,
        },
    )
    .await
}
