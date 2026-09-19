# ocid — Podman Desktop extension

A dashboard for the [ocid](https://github.com/safonas/ocid) daemon inside Podman
Desktop: status, releases, peers, live events, connection ticket (copy + QR),
and the policy actions (`seed` / `follow` / `pin`, sync, GC, remove) — the same
`/_ocid` control API `ocictl` talks to. The extension is a thin view; every
capability maps 1:1 to a daemon endpoint and the daemon stays the single policy
writer.

Live transfers appear as a progress strip (byte-accurate bars with throughput)
whenever the daemon fetches an image from peers — your own pulls *and*
background replication of followed publishers. Publishers get a stable color
across the releases table, peers list, and event stream.

The extension also hooks into Podman Desktop itself:

- **Toasts** when a followed publisher ships a release, with a one-click
  *Pull* that runs `podman pull` as a task in the task widget.
- **Push image to ocid peers** on the Images page context menu: pushes the
  image to the daemon's registry as a visible task — the daemon signs it and
  announces it, so peers following you replicate it automatically.
- **Setup card**: starts the daemon (found on `PATH` or in common install
  locations) and registers `127.0.0.1:5050` with podman as an insecure
  registry (`~/.config/containers/registries.conf.d/100-ocid.conf` on Linux),
  so push/pull need no `--tls-verify=false`. Rootful podman and podman
  machines don't read the user drop-in; those fall back to a TLS bypass
  automatically.

## Status

Phase 2 of the onboarding flow is in progress (see
[#15](https://github.com/safonas/ocid/issues/15)). Supported: Linux with a
rootless podman connection. Known gaps: automatic registration is not
implemented on macOS yet (TLS bypass covers it), and the daemon is not bundled
with the extension — it must be installed separately.

## Install (testing round)

1. Install the daemon on this machine — Homebrew
   (`brew install safonas/tap/ocid`) or a release bundle (contains `ocid`,
   `ocictl`, `ocitop`).
2. In Podman Desktop: **Settings → Extensions → Install from OCI image** →
   `ghcr.io/safonas/ocid-extension:testing` (versioned tags track releases).
3. Open the ocid dashboard. If the daemon is not running, the setup card
   starts it; then register the registry with podman from the same card.
4. To try the peer flow, repeat on a second machine (or a second
   `OCID_HOME`) and paste one node's ticket into the other's dashboard.

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
just ext-build     # build backend (dist/) + frontend (media/)
just ext-check     # tsc + svelte-check
just ext-test      # unit tests for extension internals (no daemon)
just ext-smoke     # run the extension client against a throwaway daemon
just ext-image     # build the OCI artifact (for the catalog)
just ext-install   # force npm install (e.g. after package.json changes)
```

`build`, `check` and `smoke` auto-run npm install when `node_modules` is
missing (first run, or after `just ext-clean`); `just ext-install` forces a
reinstall, e.g. after changing `package.json`.

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
2. `just ext-build` — build the extension (installs node_modules on first run).
3. In Podman Desktop: **Settings → Extensions → Add a local folder
   extension...** and select this directory (`extensions/podman-desktop`).
   Podman Desktop watches the folder and reloads when it changes.
4. Iterate: rerun `just ext-build` after your changes (Podman Desktop picks
   the new `dist/` + `media/` up on reload), and drive the daemon from a
   terminal with `just ctl ...` (`ls`, `peers`, `status`, ...).

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
2. On every published release, the
   [`extension` workflow](../.github/workflows/extension.yml) builds the
   multi-arch image from the tag and pushes it to
   `ghcr.io/safonas/ocid-extension:<tag>` and `:testing`.
3. Catalog submission (Settings → Extensions discoverability) is a follow-up
   once the testing round stabilizes.

## Layout

- `src/` — extension backend: typed `/_ocid` client, 2s poller, SSE watcher
  with a ring buffer, transfer tracker, action dispatcher, registries.conf
  drop-in writer, daemon start, webview wiring.
- `frontend/` — Svelte 5 webview (built into `media/`): status header,
  transfers strip, releases table, peers, events log, ticket + QR.
- `Containerfile` — scratch OCI artifact for catalog distribution.
