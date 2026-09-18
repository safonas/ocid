// ocid Podman Desktop extension — dashboard for the local ocid daemon.
//
// The extension is a thin view over the daemon's /_ocid/ control API (the
// same endpoints ocictl uses): status, releases, peers, live SSE events,
// and the policy-mutation endpoints. All network I/O happens here in the
// extension backend; the webview is a pure view (the daemon has no CORS
// headers and the webview must not depend on streaming fetches).

import * as api from '@podman-desktop/api';
import { readFile } from 'node:fs/promises';
import { OcidClient } from './ocid-client';
import { DashboardState } from './state';
import type { WebviewMessage } from './types';

let state: DashboardState | undefined;
let panel: api.WebviewPanel | undefined;
let statusBar: api.StatusBarItem | undefined;

export async function activate(extensionContext: api.ExtensionContext): Promise<void> {
  const url = api.configuration.getConfiguration('ocid').get<string>('registryUrl');
  const client = new OcidClient(url ?? 'http://127.0.0.1:5050');

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
      statusBar.tooltip = `${s.status.did} (v${s.status.version}) — click to open the dashboard`;
    }
  }, 2_000);

  const openDashboard = api.commands.registerCommand('ocid.openDashboard', () => {
    panel?.reveal();
  });

  extensionContext.subscriptions.push(
    panel,
    receive,
    { dispose: () => clearInterval(updateBar) },
    openDashboard,
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
