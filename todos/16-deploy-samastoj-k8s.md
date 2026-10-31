# Deploy ocid into samastoj (minikube / Flux) — reference k8s deployment

> **Priority 16 · narrow slice of 06:** first real Kubernetes deployment —
> run the daemon in the samastoj homelab (minikube, Flux CD), publish a
> daemon image, add k8s e2e coverage. Deliberately skips 06's auth and
> Flux-consumes-ocid stories: this deployment *hosts* ocid, it doesn't pull
> through it.

## Status: DONE 2026-10-31 — daemon deployed, host↔cluster P2P live, Flux consumes ocid

As-built on samastoj `main` (commits `e480f7c`, `315a1d9`/`96c5267`, `ff1756f`, `7a98334`):

- **Daemon in cluster:** `kubernetes/apps/base/ocid/` — Namespace, 50Gi PVC
  (`standard`), Deployment, ClusterIP Service on 5050. Image
  `localhost/ocid-daemon:ext` (this repo's `Containerfile` daemon stage, built
  via `just ext daemon-image`), imported into minikube with
  `podman save localhost/ocid-daemon:ext | podman exec -i minikube ctr -n k8s.io images import -`
  (`minikube image load` cannot see the rootless podman image store).
- **Onboarding = follow of the host via ticket:** ConfigMap `ocid-config`
  carries the host's `ocictl ticket` + `policy.toml` with
  `[[follow]] publisher = <host-id>`; the pod runs `ocid --peer <ticket>`
  (trailing newline stripped in a `/bin/sh -c` wrapper). Identity
  (`secret.key`) is generated inside the PVC on first start — no secrets in
  git, no SOPS. Verified both directions: cluster `/_ocid/status` shows
  `neighbors: [<host-id>]`, `follows: [<host-id> [latest]]`; host
  `ocictl peers` lists the cluster node as neighbor.
- **Flux consumes ocid (06's OCIRepository story, proven):**
  `clusters/local/oci-apps.yaml` adds `OCIRepository hugo`
  (`oci://ocid.ocid.svc:5050/<host-id>/hugo`, `insecure: true` — in-cluster
  ocid serves plain HTTP on the ClusterIP Service) + `Kustomization hugo-oci`
  (`path: ./`). Push flow on the host: `just push-oci hugo v1`
  (`flux push artifact oci://127.0.0.1:5050/hugo:v1 --insecure-registry`;
  the host registry is self-signed TLS). The host-follow replicates the
  artifact to the cluster ocid; Flux pulls it and deploys. Verified
  end-to-end, including re-pushing the same tag with a new digest →
  cluster ocid serves the new manifest → Flux rolls the new revision.
  Reference app: hugo blog, source in `kubernetes/oci/hugo/`, public at
  `blog.salera.app` (azguard Caddy, no auth), private via tailscale Ingress.
- **Zero ocid code changes were needed** for ocid to act as a Flux OCI
  registry: oras/flux push+pull worked first try against the existing
  `/v2/` surface (monolithic + chunked uploads, tags/list, HEAD/GET
  manifests, Range blob GETs).

### Still open from the plan below

- §1: published `ghcr.io/safonas/ocid` daemon image + workflow + Flux image
  automation — today the cluster runs a local-only image.
- §2: `scripts/e2e-k8s.sh` + `just k8s-e2e`.
- PodMonitor + `ocid.felis-ule.ts.net` ts-ingress (registry stays
  cluster-internal only — correct per 03).
- containerd mirror pulls *through* ocid on minikube.
- Public exposure behind auth (03) — deliberately not done.

## Context

[samastoj](https://codeberg.org/safonas/samastoj) is the homelab GitOps repo
(minikube, Flux v2.9.4, envoy-gateway, kube-prometheus-stack, loki,
cloudnative-pg, tailscale-operator). All apps so far are HelmReleases
(immich, jellyfin) — ocid would be the **first chartless app** (plain
Deployment + Service + PVC + PodMonitor). No ocid daemon image is published
anywhere today; only the Podman Desktop extension image
(`ghcr.io/safonas/ocid-extension`) exists.

Decisions already made:

- **Image source:** publish `ghcr.io/safonas/ocid` (multi-arch), referenced
  by the manifests + Flux image automation.
- **Exposure:** cluster-internal Service + tailscale ingress
  (`ocid.felis-ule.ts.net`, valid TLS) — **never** a public HTTPRoute: the
  registry has no auth (03), anyone who can reach it can push as us.
- **E2E depth:** core (deploy, probes, push/pull, metrics); P2P host↔cluster
  leg and containerd-mirror pulls are follow-ups.

## Plan — ocid repo

### 1. `.github/workflows/image.yml` — publish daemon image

Mirror `extension.yml`: buildah in a pinned container, qemu binfmt for arm64,
per-arch builds from `Containerfile` **default target** (`runtime` — ships
`ocictl`, needed for the samastoj initContainer and `kubectl exec`), assemble
a manifest list, push `ghcr.io/safonas/ocid:vX.Y.Z` + mutable `:testing`.

- Triggers: `release: published` + `workflow_dispatch(tag)`; permissions
  `packages: write`.
- Chain-dispatch from `slsa.yml` after it publishes the release (same as it
  does for `extension.yml`).
- Pass `SOURCE_DATE_EPOCH` build-arg from the tag commit time (release-flow
  convention for reproducible layer timestamps).
- **Backfill:** after merge, `workflow_dispatch(tag=v0.7.1)` so the pinned
  tag exists for samastoj (v0.7.1 predates this workflow).

### 2. `scripts/e2e-k8s.sh` + `just k8s-e2e` recipe

Reuses `e2e.sh` helper style (`wait_for`, `assert_*`, `tick`, `log/ok/fail`).
`SAMASTOJ_DIR` env, default `../samastoj`. Not part of `just ci` (needs a
live minikube). Document in `docs/development.md`.

| # | Scenario | Asserts |
|---|---|---|
| 1 | `just image` → `minikube image load` → `kubectl apply -k $SAMASTOJ_DIR/kubernetes/apps/overlays/local` | rollout ready ≤120s, probes green |
| 2 | `kubectl exec deploy/ocid -- ocictl status` | valid JSON: id, uptime |
| 3 | port-forward `svc/ocid` → `curl /v2/` 200; `podman push --tls-verify=false alpine 127.0.0.1:5050/e2e-k8s:1` | `ocictl ls` shows release; pull back; `podman run` ok |
| 4 | `curl /metrics` | OpenMetrics content-type, `# EOF`, `ocid_releases_published_total`, `ocid_http_requests_total{status="201"}` |
| 5 | soft check: `curl https://ocid.felis-ule.ts.net/v2/` | skip when off tailnet; assert 200 + valid TLS when on it |
| 6 | cleanup: `ocictl rm` + `gc --force` via exec, kill port-forward, `podman rmi` | clean exit |

## Plan — samastoj repo

```
kubernetes/apps/base/ocid/
  kustomization.yaml
  deployment.yaml     # Namespace + Deployment + Service (multi-doc, like immich's helmrelease.yaml)
  pvc.yaml            # ocid-data-pvc, 10Gi, RWO, storageClassName local-storage (base)
  podmonitor.yaml     # label app.kubernetes.io/component: monitoring — required by
                      # kube-prometheus-stack's podMonitorSelector
  ts-ingress.yaml     # ingressClassName: tailscale, backend ocid:5050 → ocid.felis-ule.ts.net
kubernetes/apps/base/kustomization.yaml                  # + ocid
kubernetes/apps/overlays/local/kustomization.yaml        # + ocid patches
kubernetes/apps/overlays/local/ocid/
  pvc-patch.yaml        # local-storage → standard (minikube default class)
  deployment-patch.yaml # image → localhost/ocid:dev + imagePullPolicy: Never
                        # (both init and main container) for fast iteration
kubernetes/infra/base/automation/  # + ImageRepository ocid (ghcr.io/safonas/ocid, anonymous, 1h)
                                   # + ImagePolicy ocid-policy (semver 0.x.x)
justfile                         # ocid-image, ocid-port-forward recipes
```

Deployment shape (essentials):

- `replicas: 1` + `strategy: Recreate` — **single writer per `OCID_HOME`**
  (redb takes an exclusive lock on `blobs.db`); never two pods on one volume.
- Image `ghcr.io/safonas/ocid:v0.7.1` with the imagepolicy marker comment
  `# {"$imagepolicy": "flux-system:ocid-policy:tag"}` (immich convention,
  Flux strips the `v`); ImageUpdateAutomation already scans
  `./kubernetes/apps/base`, so tag bumps are automatic once the
  ImageRepository/Policy exist.
- initContainer `init-home`: `[ -f /data/secret.key ] || ocictl init`, then
  `sed -i 's/^mdns = .*/mdns = false/' /data/config.toml`. Rationale: the
  daemon **overwrites `config.toml` on first start** when `secret.key` is
  absent (flag/env overrides get persisted), so the home must be initialized
  before patching; mDNS multicast is useless in k8s, relay handles discovery.
  Same pattern `scripts/e2e.sh` uses.
- Env: `OCID_HOME=/data`, `OCID_LISTEN=0.0.0.0:5050` (registry + `/_ocid`
  API + `/metrics` on one port), `RUST_LOG=ocid=info,warn`.
- securityContext: `runAsUser/runAsGroup/fsGroup: 1000` (image uid),
  `readOnlyRootFilesystem: true` (daemon writes only under `/data`),
  drop ALL capabilities, seccomp RuntimeDefault. No extra privileges needed:
  one TCP socket + one UDP (QUIC, random port; does not need to be
  reachable — n0 relay covers discovery).
- Probes: `GET /v2/` for liveness + readiness (instant, no p2p IO).
  **Never** probe manifest-by-tag — an unknown tag blocks up to
  `fetch_timeout_secs` (120s) on on-demand p2p fetch.
- TLS **off** in-cluster: `tls = "auto"` certs have SANs only for
  localhost/127.0.0.1/host.containers.internal, no Service DNS. Tailnet TLS
  terminates at the tailscale ingress.
- `terminationGracePeriodSeconds: 10` — SIGTERM is handled (PID-1 safe),
  quick shutdown.
- Resources: requests 100m/128Mi, limits 1/512Mi (jellyfin-style envelope).
- justfile (host-only): `ocid-image` (build in ../ocid, `podman save` →
  `minikube image load`), `ocid-port-forward`
  (`kubectl -n ocid port-forward svc/ocid 5050:5050`).

## Constraints & gotchas (from both codebases)

- **No registry auth** (todo 03): cluster-internal + tailnet only. Any
  tailnet device can push as us — accepted for the homelab.
- **Verify the Flux branch before committing:** live Kustomizations track
  `dev@sha1:0af85fa1` while `gotk-sync.yaml` and the ImageUpdateAutomation
  reportedly reference `main`. Check `flux get sources git -A` and
  `infra/base/automation/`; the samastoj PR must target the branch Flux
  actually tracks, or automation tag bumps never land.
- samastoj pre-commit (`kustomize build` + kubeconform against Flux CRD
  schemas, yamllint 120 cols) validates the new manifests automatically —
  run `pre-commit run --all-files` before committing.
- samastoj naming conventions to follow: ns `ocid`, files
  `deployment/pvc/podmonitor/ts-ingress.yaml`, labels
  `app.kubernetes.io/{name,component,part-of}`, PVC suffix `-pvc`.

## Implementation order

1. ocid: image workflow → merge → backfill `v0.7.1` (+ push `:testing`).
2. samastoj: manifests + overlay + automation + justfile → pre-commit →
   commit to the Flux-tracked branch → `flux reconcile kustomization apps
   --with-source` → verify rollout, PodMonitor target up,
   `ocid.felis-ule.ts.net/v2/` 200.
3. ocid: `scripts/e2e-k8s.sh` + `just k8s-e2e` + docs → run against the live
   cluster.
4. Commit/PR both: ocid `feat: publish daemon image to ghcr + k8s e2e`,
   samastoj `feat: deploy ocid daemon`.

## Follow-ups (out of scope here)

- ~~P2P host↔cluster e2e leg (host daemon + ticket + follow)~~ — **done**, see Status above.
- ~~Flux `OCIRepository` consuming ocid~~ — **done** (hugo-oci), see Status above; that was the easy half of 06.
- Prometheus target assertion via the monitoring API.
- containerd mirror pulls *through* ocid on minikube (the rest of the 06
  story: `ImageRepository`/hosts.toml against the ocid registry).
- Public exposure behind auth (03) + Caddy/HTTPRoute via terraform
  `public_endpoints`.

## Why this priority

Cheap, concrete first step into k8s: exercises the container story end to
end, produces the missing daemon image artifact, and gives a reference
deployment to point edge/air-gap users at — while 06's hard parts (auth,
Flux consumption) stay parked.
