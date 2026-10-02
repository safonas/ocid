# Flatpak P2P Remote (apps over the same rails)

> **Priority 15 · Tier 2 — ecosystem:** the P2P rails are generic and the OCI registry is only their first front end; flatpak is the same shape (static OSTree repo over plain HTTP) and would make ocid distribute GUI apps, not just containers. Tracked in [#79](https://github.com/safonas/ocid/issues/79) (mirrored: `rad:d4aee0a4`).

## Context

Flatpak distributes apps as static OSTree repos served over plain HTTP —
`summary`, `refs/`, `objects/`, `deltas/` — which is exactly the shape ocid
already serves on the loopback façade. Meanwhile every other ocid primitive
is content-address-first and object-model-agnostic: BLAKE3 blobs via
iroh, a SHA256→blob index, signed release records, replication by policy,
and pull-on-demand for unknown digests. Nothing in that stack knows about
OCI manifests except the `/v2` endpoints.

So a flatpak remote is not a second product — it is a second façade over
the same store, the way `/_ocid` and `/v2` already coexist:

    flatpak remote-add --if-not-exists ocid https://127.0.0.1:5050/flatpak.d/ocid.flatpakrepo
    flatpak install ocid app/stable

Peers replicate and fetch the app's objects over QUIC exactly like image
layers; a teammate's `flatpak install` triggers ocid's lazy peer fetch.

## Proposal

- **URL space**: serve `/.flatpak/<publisher>/<app>/` as an OSTree repo
  (summary, refs, objects, optional deltas); generate the matching
  `.flatpakrepo` file (Url= pointing at the local façade) so stock
  `flatpak remote-add` works unchanged.
- **Object mapping**: every OSTree object (commit, dirtree, dirmeta,
  filez, delta part) is content-addressed by SHA256 — ingest via the
  existing `index/digests/<sha256>` → blob mapping. No new GC, replication
  or on-demand-fetch code; those paths are object-agnostic today.
- **Release records**: gain a repo/ref flavor — "publisher X ships flatpak
  app Y, branch stable" — announced, signed and replicated like image
  releases. ocid's signed records are the authenticity story; GPG signing
  of the underlying repo stays optional (`--no-gpg-verify` covers the LAN
  case).
- **Export path**: publishing needs an exporter — `flatpak build-export`
  equivalent or ostree bindings in the daemon — to turn a build dir into
  commit/tree/dirtree objects and write the summary.
- **Trust plumbing**: GLib's TLS reads the *system* trust store, not
  podman's `certs.d`, so the daemon CA install for flatpak clients goes
  to `~/.local/share/ca-certificates` + `update-ca-trust` — the same
  dance the extension does for podman, different door.

## Wrinkles

- The OSTree object model is the real work: an exporter, a summary
  generator, and delta handling (static deltas are chunky tarballs — fine
  as blobs, but decide early whether deltas ship or P2P pulls force loose
  objects).
- Release-record semantics differ from image tags: refs are branch-shaped
  (`app/x86_64/stable`), updates are commit-chained — the follow/seed
  policy vocabulary needs a small mapping.
- e2e needs flatpak on the test host (or a container with flatpak + a
  fake system helper); start with `ostree`-level tests before the
  flatpak CLI layer.

## Why this priority

Solid Tier 2: no new infrastructure, but a new object model and an
exporter — bigger than a façade, smaller than the extension. Parked until
the Podman Desktop extension (#15) stabilizes; it reuses everything that
work is proving out (TLS loopback façade, certs trust install, bundled
daemon pod, pull-on-demand).
