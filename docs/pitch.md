# ocid: local-first, peer-to-peer OCI distribution

> `ocid` is a local-first, peer-to-peer alternative to centralized OCI
> registries, cryptographically verified, serverless, and designed for edge,
> offline, and bandwidth-constrained environments.

---

## Why not just another registry?

Centralized registries (Docker Hub, Quay, ECR, GHCR) introduce single points
of failure, egress bandwidth costs, regulatory and censorship risks, and
severe bottlenecks in edge, air-gapped, and large-scale CI environments.

**`ocid`** is a peer-to-peer distribution fabric instead of another central
server:
- Every node runs an embedded OCI registry on `127.0.0.1:5050`. Existing developer workflows (`podman`, `docker`, `crane`, `oras`) work without modification.
- Image distribution happens under the hood via **iroh** (QUIC, hole-punching, DERP relays, LAN mDNS) with incremental, verified streaming (BLAKE3 bao).
- Cryptographic identities (Ed25519) replace registry accounts and centralized certificate authorities.
- Explicit retention policies (`latest`, `last:N`, `pin`) automatically manage edge disk space without manual cleanup.

---

## Network Topologies

`ocid` adapts flexibly to diverse infrastructure constraints:

### 1. Zero-Config LAN Mesh (Air-Gapped & Local Clusters)
* **Mechanism**: Uses local mDNS (`_ocid._udp.local`) discovery with `--no-relay`.
* **Behavior**: Nodes on the same physical subnet or VLAN instantly discover each other, establish direct QUIC links, and replicate container layers at local wire speed.
* **Connectivity**: 0% external internet access required; completely air-gapped.

### 2. Hierarchical Edge Fleet / Satellite Gateway
* **Mechanism**: Upstream CI/CD or build servers publish images under an authorized Ed25519 identity.
* **Behavior**: Regional gateway nodes track upstream publishers. Field devices or edge nodes (cellular, Starlink, offshore) follow regional gateways using bounded retention (`mode = "latest"` or `mode = "last:1"`).
* **Benefit**: WAN bandwidth is consumed exactly once per site; local nodes cross-seed and distribute updates among themselves peer-to-peer.

### 3. Global Multi-Publisher Collaborative Swarm
* **Mechanism**: Global peering coordinated via iroh's DERP relay infrastructure and hole-punching.
* **Topic Isolation**: Gossip announcements are segregated by publisher identity:
  $$\text{TopicId} = \text{blake3}(\text{"ocid/publisher/v1/"} \parallel \text{PublisherId})$$
  Nodes only receive traffic and announcements for images and publishers declared in their local `policy.toml`.

### 4. Untrusted Persistent Seeding & Mirroring
* **Mechanism**: High-bandwidth bastion servers or community mirrors can `follow` or `seed` images without possessing release signing keys.
* **Security**: All image manifests and layers are verified using content hashes and publisher signatures before acceptance. Untrusted nodes can serve content without any risk of tampering.

---

## Who it is for

Engineers working with containers, CI/CD, edge deployments, air-gapped
environments, self-hosting, and supply-chain security — anywhere registry
availability, egress costs, or infrastructure trust break the normal workflow.

### 1. Edge Computing, IoT & Remote Field Gateways
* **Challenge**: Pushing multi-hundred-megabyte container updates to hundreds of IoT gateways or field devices simultaneously saturates constrained WAN or satellite backhauls.
* **Solution**: Point local container runtimes to `127.0.0.1:5050`. The first node to pull chunks shares them over LAN mDNS with surrounding nodes at hardware speed. Automated retention policies prune outdated versions to prevent disk exhaustion.

### 2. Air-Gapped & High-Security Enclaves (Defense, SCADA, Subsea)
* **Challenge**: Enclaves prohibit inbound/outbound external registry access. Moving images via `podman save`/`tar` lacks incremental layer transfer, deduplication, and automated verification.
* **Solution**: Physically connect an ingest node to the enclave network; nodes discover each other via mDNS. Standard commands like `podman pull 127.0.0.1:5050/<pub>/app:tag` work seamlessly with cryptographic verification and deduplicated layer storage.

### 3. High-Density CI/CD Runner Clusters
* **Challenge**: Massive runner farms repeatedly pull identical base images (Rust, Go, Debian, PyTorch), incurring registry rate limits, latency, and expensive egress fees.
* **Solution**: Run `ocid` alongside hypervisors or runner nodes. Base layers are fetched once from upstream and served across runner nodes via local QUIC connections.

### 4. Censorship-Resistant & Trustless Software Delivery
* **Challenge**: Centralized registries are vulnerable to account takedowns, domain seizures, regional geoblocking, and policy changes.
* **Solution**: Publishers distribute software using raw Ed25519 public keys (`did:key:z6Mk...`). Downstream consumers and mirror operators co-seed releases independently of any centralized cloud vendor.

### 5. Local-First Developer Collaboration
* **Challenge**: Teammates testing multi-container architectures must authenticate, push images to remote registries, and wait for teammates to pull them back down.
* **Solution**: A developer pushes to `localhost:5050/app:dev`. A teammate aliases the developer via `ocictl track <pub> --as teammate` and runs `podman run localhost:5050/teammate/app:dev` directly over peer-to-peer QUIC.

---

## Architectural Comparison

| Dimension | Centralized Registry (Docker Hub, ECR) | P2P Distribution (`ocid`) |
|---|---|---|
| **Architecture** | Central server + client | Local-first daemon + P2P mesh |
| **Tooling Compatibility** | Standard OCI v2 | Native OCI v2 (`localhost:5050`) |
| **Identity & Trust** | Usernames, passwords, bearer tokens, CAs | Ed25519 keypairs (`did:key:z6Mk...`) |
| **Verification** | Client trusts TLS certificate & registry | End-to-end signatures + BLAKE3 bao layer streams |
| **Bandwidth Scaling** | Linear server load & egress costs | Swarm-assisted peer-to-peer offloading |
| **Air-Gap Capability** | Requires internal mirrors & DNS hacks | Native zero-config local mesh (mDNS) |
| **Cache Management** | Manual script pruning or complex LRU | Declarative retention windows (`latest`, `last:N`, `pins`) |

---

## Status and feedback

`ocid` is an early prototype. The most useful contribution right now is a
reproducible test: run the two-node workflow in `README.md` on a Raspberry Pi
or small ARM node, across two networks, or in an air-gapped lab, document the
result, and
[open an issue](https://github.com/safonas/ocid/issues) with hardware,
commands, and logs.
