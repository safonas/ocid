// ocid Podman Desktop extension — dashboard for the local ocid daemon.
//
// The extension is a thin view over the daemon's /_ocid/ control API (the
// same endpoints ocictl uses): status, releases, peers, live SSE events,
// and the policy-mutation endpoints. All network I/O happens here in the
// extension backend; the webview is a pure view (the daemon has no CORS
// headers and the webview must not depend on streaming fetches).
//
// It also hooks into Podman Desktop itself: toasts when a followed
// publisher ships a release (with a one-click podman pull), and a
// "push to ocid peers" entry on the Images page. Setup integrations:
// registering the local registry with podman (registries.conf drop-in)
// and starting the daemon from the host.

import * as api from '@podman-desktop/api';
import { spawn } from 'node:child_process';
import { access, constants, mkdir, open, readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { isPodmanEngine, menuImageSource, ocidName } from './images';
import type { MenuImage } from './images';
import { OcidClient } from './ocid-client';
import { isRegistered, register, registryHost } from './registries';
import { DashboardState, short } from './state';
import type { SetupState, WebviewMessage } from './types';

let state: DashboardState | undefined;
let panel: api.WebviewPanel | undefined;
let statusBar: api.StatusBarItem | undefined;
/** Releases shipped by followed peers since the dashboard was last opened. */
let newFromFollowed = 0;

/** Run podman as a visible task in Podman Desktop's task widget. */
async function podman(args: string[], title: string): Promise<void> {
  await api.window.withProgress({ location: api.ProgressLocation.TASK_WIDGET, title }, async () => {
    await api.process.exec('podman', args);
  });
}

/** Whether the user-level drop-in registers the current host. Refreshed by
 *  every setup poll; drives the podmanRun TLS-bypass decision. */
let registeredNow = false;

/** Run `podman pull/push` against the ocid registry. With the user-level
 *  registries.conf drop-in in place, rootless podman needs no flags; the
 *  bypass retry covers rootful podman and podman machines (which don't read
 *  the user drop-in) as well as unregistered setups. */
async function podmanRun(args: string[], title: string): Promise<void> {
  if (registeredNow) {
    try {
      await podman(args, title);
      return;
    } catch {
      // fall through to the TLS bypass
    }
  }
  await podman([args[0], '--tls-verify=false', ...args.slice(1)], title);
}

/** Prefer podman's stderr when a run fails; fall back to the error message. */
function runErrMsg(e: unknown): string {
  if (e !== null && typeof e === 'object' && 'stderr' in e) {
    const stderr = String((e as { stderr?: string }).stderr ?? '').trim();
    if (stderr) return stderr;
  }
  return e instanceof Error ? e.message : String(e);
}

// --- setup integrations (host side) ------------------------------------------

/** Where GUI-launched Podman Desktops often miss brew/cargo installs. */
const OCID_DIRS = [
  '/usr/local/bin',
  path.join(homedir(), '.cargo', 'bin'),
  path.join(homedir(), '.local', 'bin'),
  '/opt/homebrew/bin',
  '/home/linuxbrew/.linuxbrew/bin',
];

async function findOcid(): Promise<string | undefined> {
  try {
    const found = (await api.process.exec('sh', ['-c', 'command -v ocid'])).stdout.trim();
    if (found) return found;
  } catch {
    // not on PATH; check well-known locations below
  }
  for (const dir of OCID_DIRS) {
    const candidate = path.join(dir, 'ocid');
    try {
      await access(candidate, constants.X_OK);
      return candidate;
    } catch {
      // keep looking
    }
  }
  return undefined;
}

/** Cached daemon-binary lookup; a negative result is retried at most every
 *  30s (the poll loop calls this every 2s). */
let ocidPath: string | undefined;
let ocidLookupAt = 0;

async function ocidLookup(): Promise<string | undefined> {
  if (ocidPath) return ocidPath;
  if (Date.now() - ocidLookupAt < 30_000) return undefined;
  ocidLookupAt = Date.now();
  ocidPath = await findOcid();
  return ocidPath;
}

export async function activate(extensionContext: api.ExtensionContext): Promise<void> {
  const url = api.configuration.getConfiguration('ocid').get<string>('registryUrl');
  const base = url ?? 'http://127.0.0.1:5050';
  const host = registryHost(base);
  const client = new OcidClient(base);

  panel = api.window.createWebviewPanel('ocid', 'ocid', {
    localResourceRoots: [api.Uri.joinPath(extensionContext.extensionUri, 'media')],
  });
  panel.webview.html = await webviewHtml(extensionContext, panel);

  /** Setup state for the dashboard card; also keeps `registeredNow` fresh. */
  const setup = async (): Promise<SetupState> => {
    const linux = process.platform === 'linux';
    registeredNow = linux && (await isRegistered(host).catch(() => false));
    return {
      platform: linux ? 'linux' : 'other',
      registryHost: host,
      registered: registeredNow,
      ocidPath: await ocidLookup(),
    };
  };

  /** Write the user-level registries.conf drop-in (Linux; see registries.ts). */
  const registerRegistry = async (): Promise<void> => {
    if (process.platform !== 'linux') {
      throw new Error(`automatic registration is not available on ${process.platform} yet`);
    }
    await register(host);
  };

  /** Start the daemon detached; logs go to the extension's storage dir. */
  const startDaemon = async (): Promise<void> => {
    const bin = await ocidLookup();
    if (!bin) throw new Error('no ocid binary found — install it first (see the setup card)');
    const logDir = path.join(extensionContext.storagePath, 'daemon');
    await mkdir(logDir, { recursive: true });
    const out = await open(path.join(logDir, 'ocid.log'), 'a');
    try {
      const child = spawn(bin, [], { detached: true, stdio: ['ignore', out.fd, out.fd] });
      child.unref();
    } finally {
      await out.close();
    }
  };

  state = new DashboardState({
    client,
    webview: panel.webview,
    confirm: (message, ok) =>
      api.window
        .showInformationMessage(message, 'Cancel', ok)
        .then(choice => choice === ok),
    notify: (message, error) =>
      error ? api.window.showErrorMessage(message) : api.window.showInformationMessage(message),
    setup,
    registerRegistry,
    startDaemon,
    onFollowedRelease: (publisher, name, tag) => {
      newFromFollowed++;
      const reference = `${host}/${publisher}/${name}:${tag}`;
      void api.window
        .showInformationMessage(`${short(publisher)} released ${name}:${tag}`, 'Pull')
        .then(choice => {
          if (choice !== 'Pull') return;
          // Plain podman pull into the local engine; podmanRun adds the
          // TLS bypass when the registry is not (or cannot be) registered.
          podmanRun(['pull', reference], `ocid: pulling ${name}:${tag}`)
            .then(() => api.window.showInformationMessage(`Pulled ${name}:${tag} from ocid peers`))
            .catch(e => api.window.showErrorMessage(`Pull failed: ${runErrMsg(e)}`));
        });
    },
  });

  const receive = panel.webview.onDidReceiveMessage((e: unknown) => {
    if (e !== null && typeof e === 'object' && 'type' in e) {
      void state?.handleMessage(e as WebviewMessage);
    }
  });

  state.start();

  // Status bar: daemon up/down + peers, opens the dashboard on click.
  statusBar = api.window.createStatusBarItem();
  statusBar.text = 'ocid';
  statusBar.tooltip = 'Open the ocid dashboard';
  statusBar.command = 'ocid.openDashboard';
  statusBar.show();
  const updateBar = setInterval(() => {
    const s = state?.current();
    if (!statusBar || !s) return;
    if (!s.daemon || !s.status) {
      statusBar.text = 'ocid: down';
      statusBar.tooltip = 'ocid daemon not reachable — open the dashboard to start it';
    } else {
      statusBar.text = `ocid: ${s.status.neighbors.length} peer(s), ${s.status.releases} release(s)`;
      statusBar.tooltip =
        newFromFollowed > 0
          ? `${newFromFollowed} new release(s) from followed peers — click to open the dashboard`
          : `${s.status.did} (v${s.status.version}) — click to open the dashboard`;
    }
  }, 2_000);

  const seenDashboard = () => {
    newFromFollowed = 0;
  };
  const viewState = panel.onDidChangeViewState(e => {
    if (e.webviewPanel.active || e.webviewPanel.visible) seenDashboard();
  });
  const openDashboard = api.commands.registerCommand('ocid.openDashboard', () => {
    seenDashboard();
    panel?.reveal();
  });

  // "Push image to ocid peers" on the Images page. The daemon signs the
  // pushed release and announces it on gossip, so peers following this
  // node replicate it automatically — no further calls needed here.
  // NOTE: the argument is PD's Images-page UI object (name/tag/engineName),
  // not the api.ImageInfo the registration type suggests — see images.ts.
  const pushImage = api.commands.registerCommand('ocid.image.push', async (image: MenuImage) => {
    if (!image) {
      api.window.showErrorMessage('Run this from an image in the Images page.');
      return;
    }
    const source = menuImageSource(image);
    if (!source) {
      api.window.showErrorMessage('The image has no tag to push.');
      return;
    }
    if (!isPodmanEngine(image)) {
      api.window.showErrorMessage(
        `ocid push supports podman engines (got ${image.engineName ?? image.engineType ?? 'unknown'}).`,
      );
      return;
    }
    const name = ocidName(source);
    const target = `${host}/${name}`;
    try {
      await podmanRun(['push', source, target], `ocid: pushing ${name}`);
      api.window.showInformationMessage(`Pushed ${source} as ${target} — announced to peers.`);
    } catch (e) {
      api.window.showErrorMessage(
        `Push failed (is the ocid daemon running on ${host}?): ${runErrMsg(e)}`,
      );
    }
  });

  extensionContext.subscriptions.push(
    panel,
    receive,
    viewState,
    { dispose: () => clearInterval(updateBar) },
    openDashboard,
    pushImage,
    statusBar,
  );
}

export function deactivate(): void {
  state?.dispose();
  state = undefined;
  panel = undefined;
  statusBar = undefined;
}

/** index.html from the built frontend, with asset links rewritten to
 *  webview-safe URIs (the media dir is the vite outDir of `frontend/`). */
async function webviewHtml(
  extensionContext: api.ExtensionContext,
  view: api.WebviewPanel,
): Promise<string> {
  const media = api.Uri.joinPath(extensionContext.extensionUri, 'media');
  let html = await readFile(`${media.fsPath}/index.html`, 'utf8');
  const rewrite = (attr: string, matched: string): void => {
    const value = RegExp(`${attr}="(.*?)"`).exec(matched)?.[1];
    if (!value) return;
    const uri = view.webview.asWebviewUri(api.Uri.joinPath(media, value));
    html = html.replace(value, uri.toString());
  };
  html.match(/<script[^>]* src=".*?"[^>]*>/g)?.forEach(tag => rewrite('src', tag));
  html.match(/<link[^>]* href=".*?"[^>]*>/g)?.forEach(tag => rewrite('href', tag));
  return html;
}
