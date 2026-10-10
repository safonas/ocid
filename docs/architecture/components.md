# Components

ocid is a **local-first, peer-to-peer** distribution system for OCI container images. It consists of several components that work together to provide a seamless experience for users of standard OCI tooling (podman, docker, oras) while enabling direct peer-to-peer synchronization.

## High-Level Architecture

The following diagram shows the **high-level architecture** of ocid, including all major components and their interactions:

```mermaid
flowchart TB
    subgraph host["Your Machine (Host)"]
        direction TB
        subgraph tools["Tooling"]
            podman["podman / docker / oras"]
            cli["ocictl (CLI)"]
            tui["ocitop (TUI)"]
            pdext["Podman Desktop Extension"]
            prom["Prometheus"]
        end

        subgraph daemon["ocid (Daemon)"]
            direction TB
            subgraph registry["Registry Layer"]
                reg["OCI v2 Registry<br/>127.0.0.1:5050/v2"]
                ctl["Control API<br/>/_ocid/*"]
                met["OpenMetrics<br/>/metrics"]
                sse["SSE Events<br/>/_ocid/events"]
            end

            subgraph core["Core Layer"]
                identity["Identity<br/>Ed25519 Keypair"]
                policy["Policy Engine<br/>policy.toml"]
                windows["Retention Windows"]
                fetch["Fetch/Replicate"]
                gc["Garbage Collection"]
            end

            subgraph store["Storage Layer"]
                blobstore["Blob Store<br/>iroh-blobs FsStore"]
                index["Index<br/>JSON (digests, releases, referrers)"]
            end

            subgraph p2p["P2P Layer"]
                gossip["iroh-gossip<br/>Swarm + Publisher Topics"]
                sync["ocid/sync/1"]
                blobs["iroh-blobs"]
                mdns["mDNS<br/>_ocid._udp"]
                ep["iroh Endpoint<br/>QUIC + NAT Traversal"]
            end
        end
    end

    subgraph peers["Peer Network"]
        peer1["Peer A"]
        peer2["Peer B"]
        peer3["Peer C"]
    end

    %% Connections
    podman -- "HTTP /v2" --> reg
    cli -- "HTTP /_ocid" --> ctl
    tui -- "HTTP + SSE" --> ctl
    tui -- "HTTP + SSE" --> sse
    pdext -- "HTTP /_ocid" --> ctl
    prom -- "Scrape" --> met

    reg --> core
    ctl --> core
    met --> core
    sse --> core

    core --> identity
    core --> policy
    core --> windows
    core --> fetch
    core --> gc

    fetch --> store
    gc --> store

    store --> blobstore
    store --> index

    core --> p2p
    p2p --> gossip
    p2p --> sync
    p2p --> blobs
    p2p --> mdns
    p2p --> ep

    ep <--> peer1
    ep <--> peer2
    ep <--> peer3

    %% Styling
    classDef tooling fill:#f9f,stroke:#333,stroke-width:2px
    classDef daemon fill:#e6f3ff,stroke:#333,stroke-width:2px
    classDef peers fill:#ffe6e6,stroke:#333,stroke-width:2px

    class podman,cli,tui,pdext,prom tooling
    class reg,ctl,met,sse,identity,policy,windows,fetch,gc,blobstore,index,gossip,sync,blobs,mdns,ep daemon
    class peer1,peer2,peer3 peers
```

### Component Descriptions

| Component | Description | Responsibilities |
|-----------|-------------|------------------|
| **OCI Registry** | Embedded OCI v2 registry | Handle `podman/docker/oras` requests, serve manifests/blobs |
| **Control API** | REST API for ocid management | Handle `ocictl` requests, manage policy, peers, releases |
| **OpenMetrics** | Metrics endpoint | Expose Prometheus-compatible metrics for monitoring |
| **SSE Events** | Server-Sent Events | Stream real-time events to `ocitop` and Podman Desktop |
| **Identity** | Ed25519 keypair | Authenticate as a publisher, sign releases |
| **Policy Engine** | Retention policy | Enforce `follow`, `seed`, `pin`, and window rules |
| **Fetch/Replicate** | Replication logic | Fetch releases/blobs from peers, verify signatures |
| **Garbage Collection** | Cleanup | Remove unpinned blobs, enforce retention windows |
| **Blob Store** | iroh-blobs FsStore | Store content-addressed blobs (BLAKE3 hashes) |
| **Index** | JSON index | Map SHA256 digests to BLAKE3 hashes, track releases/referrers |
| **iroh-gossip** | Gossip protocol | Broadcast announcements, discover peers |
| **ocid/sync/1** | Sync protocol | Fetch releases/blobs from peers |
| **iroh-blobs** | Blob transfer | Download/upload blobs with incremental verification |
| **mDNS** | Local discovery | Discover other ocid nodes on the local network |
| **iroh Endpoint** | QUIC endpoint | Handle NAT traversal, hole-punching, relays |

---

## Crate Structure

The ocid project is organized into the following Rust crates:

```mermaid
flowchart TB
    subgraph crates["Rust Crates"]
        core["ocid-core"] -->|Library| daemon["ocid"]
        core -->|Library| cli["ocictl"]
        core -->|Library| tui["ocitop"]

        daemon -->|Daemon| registry["OCI Registry"]
        daemon -->|Daemon| node["Node"]
        daemon -->|Daemon| p2p["P2P"]

        cli -->|CLI| daemon
        tui -->|TUI| daemon
    end

    subgraph ext["Extensions"]
        pdext["Podman Desktop Extension"] -->|HTTP /_ocid| daemon
    end

    %% Styling
    classDef crates fill:#e6f3ff,stroke:#333
    classDef ext fill:#f9f,stroke:#333

    class core,daemon,cli,tui crates
    class pdext ext
```

| Crate | Description | Purpose |
|-------|-------------|---------|
| `ocid-core` | Library crate | Identity, config/policy, OCI types, release records, index, API DTOs |
| `ocid` | Daemon binary | Run iroh endpoint, gossip, blob store, OCI registry, control API |
| `ocictl` | CLI binary | Interact with the daemon over HTTP, manage `policy.toml` |
| `ocitop` | TUI binary | Interactive terminal dashboard for daemon status |
| `podman-desktop` | Extension | Dashboard over the `/_ocid` API (TypeScript + Svelte) |

---

## Data Flow

The following diagram shows how data flows through the system when publishing and pulling images:

```mermaid
flowchart LR
    subgraph publish["Publish Flow"]
        podman -->|PUT /v2/app/manifests/1.0| reg
        reg -->|Store blobs| store
        reg -->|Sign release| core
        core -->|Announce| gossip
        gossip -->|Broadcast| peers
    end

    subgraph pull["Pull Flow"]
        podman -->|GET /v2/A/app/manifests/1.0| reg
        reg -->|Check local| store
        reg -->|Fetch from peers| sync
        sync -->|Download blobs| blobs
        blobs -->|Store| store
        store -->|Return manifest| reg
        reg -->|Return to podman| podman
    end

    %% Styling
    classDef publish fill:#d5e8d4,stroke:#333
    classDef pull fill:#ffd966,stroke:#333

    class podman,reg,store,core,gossip,peers publish
    class podman,reg,store,sync,blobs pull
```

---

## Ports and Protocols

ocid uses the following ports and protocols:

| Port | Protocol | Purpose | Direction |
|------|----------|---------|-----------|
| 5050 | HTTP | OCI v2 Registry (`/v2/...`) | Inbound |
| 5050 | HTTP | Control API (`/_ocid/...`) | Inbound |
| 5050 | HTTP | OpenMetrics (`/metrics`) | Inbound |
| 5050 | SSE | Events (`/_ocid/events`) | Outbound |
| 127.0.0.1:0 | QUIC | iroh Endpoint (loopback) | Both |
| *:0 | QUIC | iroh Endpoint (external) | Both |
| UDP 5353 | mDNS | Local discovery (`_ocid._udp`) | Outbound |

---

## Wire Protocols

ocid uses the following wire protocols for communication:

| ALPN | Purpose | Encoding |
|------|---------|----------|
| `/iroh-gossip/1` | Gossip announcements | Postcard |
| `ocid/sync/1` | Sync requests (GetRelease, Inventory) | Postcard |
| `/iroh-bytes/...` | Blob download (BLAKE3 hash) | BAO |
| `mDNS _ocid._udp` | LAN discovery | iroh-mdns-address-lookup |

---

## Deployment Topologies

ocid can be deployed in various topologies:

### Single Node (Development)
```mermaid
flowchart TB
    podman --> ocid[ocid Daemon]
    ocictl --> ocid
    ocitop --> ocid
```

### Multi-Node (LAN)
```mermaid
flowchart TB
    subgraph nodeA["Node A"]
        podmanA --> ocidA[ocid]
    end
    subgraph nodeB["Node B"]
        podmanB --> ocidB[ocid]
    end
    subgraph nodeC["Node C"]
        podmanC --> ocidC[ocid]
    end

    ocidA <-->|mDNS| ocidB
    ocidA <-->|mDNS| ocidC
    ocidB <-->|mDNS| ocidC
```

### Multi-Node (WAN with Relays)
```mermaid
flowchart TB
    subgraph nodeA["Node A"]
        podmanA --> ocidA[ocid]
    end
    subgraph nodeB["Node B"]
        podmanB --> ocidB[ocid]
    end
    subgraph relay["Relay"]
        relay1[Relay Node]
    end

    ocidA <-->|QUIC| relay1
    ocidB <-->|QUIC| relay1
```
