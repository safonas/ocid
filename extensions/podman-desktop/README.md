# ocid — Podman Desktop extension

A dashboard for the [ocid](https://github.com/safonas/ocid) daemon inside Podman
Desktop: status, releases, peers, live events, connection ticket (copy + QR),
and the policy actions (`seed` / `follow` / `pin`, sync, GC, remove) — the same
`/_ocid` control API `ocictl` talks to. The extension is a thin view; every
capability maps 1:1 to a daemon endpoint and the daemon stays the single policy
writer.

## Status

Phase 1 (dashboard). Phase 2 (daemon lifecycle, insecure-registry
registration, ticket bootstrap) is tracked in
[#15](https://github.com/safonas/ocid/issues/15).

## Design notes

- Follows the current Podman Desktop extension conventions (see the
  [developing](https://podman-desktop.io/docs/extensions/developing) and
  [webview messaging](https://podman-desktop.io/docs/extensions/developing/webview-messaging)
  docs, and the official templates):
  - Backend does all network I/O (the daemon has no CORS headers); the webview
    is a pure view talking over `postMessage`.
  - Interactive components come from `@podman-desktop/ui-svelte`; colors follow
    Podman Desktop's `--pd-*` theme custom properties (with dark fallbacks).
  - The UI polls every 2s like `ocitop`; the SSE stream only adds the live
    event view — nothing depends on it.
- `src/types.ts` mirrors `crates/ocid-core/src/api.rs` — keep them in sync when
  the control API changes.

## Develop

```sh
just ext-install   # npm install (runs in a pinned wolfi node container)
just ext-build     # build backend (dist/) + frontend (media/)
just ext-check     # tsc + svelte-check
just ext-smoke     # run the extension client against a throwaway daemon
just ext-image     # build the OCI artifact (for the catalog)
```

Caches persist across runs: npm's download cache lives in the `ocid-npm-cache`
volume, and the OCI build caches `npm ci` in its own layer (rebuilt only when
`package-lock.json` changes).

## Test locally against Podman Desktop

The extension talks to the daemon on the host's loopback (default
`http://127.0.0.1:5050`), so the daemon must bind the **host's** 127.0.0.1 —
`just run` does exactly that (host networking inside the builder container),
with state under `./.dev/ocid-home`:

1. `just run` — starts the daemon on the host's `127.0.0.1:5050`
   (state in `.dev/ocid-home`; `rm -rf .dev/ocid-home` resets it to defaults).
2. `just ext-install && just ext-build` — build the extension.
3. In Podman Desktop: **Settings → Extensions → Add a local folder
   extension...** and select this directory (`extensions/podman-desktop`).
   Podman Desktop watches the folder and reloads when it changes.
4. Iterate: keep `just ext-watch` running (rebuilds `dist/` + `media/` on
   every change) and drive the daemon from a terminal with `just ctl ...`
   (`ls`, `peers`, `status`, ...).

Notes:

- The daemon **persists** a `--listen` override into `.dev/ocid-home/config.toml`
  (so `ocictl` knows where to find it). If yours points at a different port,
  either reset the home directory or set the extension's `ocid.registryUrl`
  setting to match (e.g. `http://127.0.0.1:15060`).
- Daemon flags pass straight through to `just run` (no `--` separator):
  `just run --listen 127.0.0.1:5051`, `just run --no-relay`, ...
- Second local node (to drive the peers/follow UI yourself):
  `OCID_HOME=.dev/node-b just run --listen 127.0.0.1:15061` and
  `OCID_HOME=.dev/node-b just ctl ...` — connect them by pasting the
  first node's ticket (`just ctl ticket`), or let mDNS find it.

## Publish (to the catalog)

1. `just ext-image` builds the scratch-based OCI artifact per the
   [publishing guide](https://podman-desktop.io/docs/extensions/publish).
2. Push it to a registry (`podman push <registry>/ocid-extension:tag`).
3. Add the extension to `extensions.json` in the
   [podman-desktop-catalog](https://github.com/podman-desktop/podman-desktop-catalog)
   repository and open a PR.

## Layout

- `src/` — extension backend: typed `/_ocid` client, 2s poller, SSE watcher
  with a ring buffer, action dispatcher, webview wiring.
- `frontend/` — Svelte 5 webview (built into `media/`): status header,
  releases table, peers, events log, ticket + QR.
- `Containerfile` — scratch OCI artifact for catalog distribution.
