# Edge Kubernetes Delivery (Flux / k3s Integration Story)

> **Priority 06 · Tier 3 — scale:** the durable differentiator (Spegel/Dragonfly slot); gated on registry auth (03).

## Context
The durable value case is edge fleets: a gateway node follows upstream
publishers, edge devices run `mode = "latest"` windows and cross-seed over
mDNS, so WAN bandwidth is paid once per site. Today reaching that story
requires hand-wiring: the registry binds to loopback only, and GitOps
controllers (Flux `OCIRepository`, image automation) have no documented path
to consume it. Comparable slot: Spegel (cluster-internal P2P mirror) — ocid
adds signed releases, publisher identity, and retention windows.

## Proposal
- Make the registry consumable from a cluster, not just loopback:
  - configurable bind address / NodePort or host-network deployment
    (docs + Helm chart / kustomize manifests for k3s, KubeEdge, MicroK8s)
  - a documented trust story for non-loopback binds (even minimal: token or
    mTLS gate in front of `/v2`) — unblocks the "no auth on registry"
    limitation for this deployment shape
- Flux integration path:
  - ~~`OCIRepository` pointing at the ocid registry service~~ — **proven
    2026-10-31**: Flux v2.10 pulls OCI artifacts from the in-cluster ocid
    Service (`insecure: true`, plain HTTP) with zero ocid changes; artifacts
    pushed on the host replicate via `follow` and are deployed by a
    `Kustomization` — see todo 16 Status. Still to verify: Flux auth options
    and digest-pin semantics against signed release records.
  - document `ImageRepository` (image pulls, not just artifacts) pointing at
    the ocid registry service
  - optionally a small notifier: on release announcement, ping Flux's
    image-update automation so deployments roll on publish, not on poll
- Ship one reference topology end-to-end (CI publishes → gateway follows →
  edge k3s cluster pulls through ocid → Flux reconciles), tested in
  `scripts/e2e.sh` style automation.

## Why this priority
Higher effort and risk than the Podman Desktop extension (auth story on
non-loopback binds is a prerequisite), but it is the long-term differentiator:
retention windows + P2P offload only pay off at fleet scale, and it positions
ocid next to Spegel/Dragonfly in the k8s distribution conversation.
