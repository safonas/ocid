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
// "push to ocid peers" entry on the Images page.

import * as api from '@podman-desktop/api';
import { readFile } from 'node:fs/promises';
import { OcidClient } from './ocid-client';
import { DashboardState, short } from './state';
import type { WebviewMessage } from './types';

let state: DashboardState | undefined;
let panel: api.WebviewPanel | undefined;
let statusBar: api.StatusBarItem | undefined;
/** Releases shipped by followed peers since the dashboard was last opened. */
let newFromFollowed = 0;

/** Registry host:port in the form podman sees (no scheme, no trailing slash). */
function registryHost(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url.replace(/^https?:\/\//, '').replace(/\/+$/, '');
  }
}

/** docker.io/library/alpine:latest -> alpine:latest; quay.io/org/app -> org/app. */
function ocidName(repoTag: string): string {
  let rest = repoTag;
  const slash = rest.indexOf('/');
  const first = slash === -1 ? '' : rest.slice(0, slash);
  if (slash !== -1 && (first.includes('.') || first.includes(':') || first === 'localhost')) {
    rest = rest.slice(slash + 1);
    if (rest.startsWith('library/')) rest = rest.slice('library/'.length);
  }
  return rest;
}

/** Run podman as a visible task in Podman Desktop's task widget. */
async function podman(args: string[], title: string): Promise<void> {
  await api.window.withProgress({ location: api.ProgressLocation.TASK_WIDGET, title }, async () => {
    await api.process.exec('podman', args);
  });
}

/** Prefer podman's stderr when a run fails; fall back to the error message. */
function runErrMsg(e: unknown): string {
  if (e !== null && typeof e === 'object' && 'stderr' in e) {
    const stderr = String((e as { stderr?: string }).stderr ?? '').trim();
    if (stderr) return stderr;
  }
  return e instanceof Error ? e.message : String(e);
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

  state = new DashboardState({
    client,
    webview: panel.webview,
    confirm: (message, ok) =>
      api.window
        .showInformationMessage(message, 'Cancel', ok)
        .then(choice => choice === ok),
    notify: (message, error) =>
      error ? api.window.showErrorMessage(message) : api.window.showInformationMessage(message),
    onFollowedRelease: (publisher, name, tag) => {
      newFromFollowed++;
      const reference = `${host}/${publisher}/${name}:${tag}`;
      void api.window
        .showInformationMessage(`${short(publisher)} released ${name}:${tag}`, 'Pull')
        .then(choice => {
          if (choice !== 'Pull') return;
          // Plain podman pull into the local engine. Until the phase-2
          // registries.conf onboarding lands, the local registry needs the
          // TLS bypass; hosts with the drop-in configured can drop the flag.
          podman(['pull', '--tls-verify=false', reference], `ocid: pulling ${name}:${tag}`)
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
      statusBar.tooltip = 'ocid daemon not reachable — start it with `ocid`';
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
  const pushImage = api.commands.registerCommand('ocid.image.push', async (image: api.ImageInfo) => {
    const source = image.RepoTags?.[0];
    if (!source) {
      api.window.showErrorMessage('The image has no tag to push.');
      return;
    }
    if (image.engineType !== 'podman') {
      api.window.showErrorMessage(`ocid push supports podman engines (got ${image.engineName}).`);
      return;
    }
    const name = ocidName(source);
    const target = `${host}/${name}`;
    try {
      await podman(['push', '--tls-verify=false', source, target], `ocid: pushing ${name}`);
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
