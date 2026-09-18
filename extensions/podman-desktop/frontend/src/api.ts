// Webview side of the extension <-> webview protocol: receives state
// snapshots, sends actions. `acquirePodmanDesktopApi()` is injected into the
// webview by Podman Desktop.

import type { Action, StateSnapshot, WebviewOut } from '../../src/types';

interface PodmanDesktopWebviewApi {
  postMessage(message: unknown): void;
}

declare function acquirePodmanDesktopApi(): PodmanDesktopWebviewApi;

const podmanDesktop = acquirePodmanDesktopApi();

let listener: ((state: StateSnapshot) => void) | undefined;

export function onState(fn: (state: StateSnapshot) => void): void {
  listener = fn;
}

export function sendAction(action: Action): void {
  podmanDesktop.postMessage({ type: 'action', action });
}

export function sendReady(): void {
  podmanDesktop.postMessage({ type: 'ready' });
}

export function listen(): void {
  window.addEventListener('message', (event: MessageEvent) => {
    const data = event.data as WebviewOut | undefined;
    if (data?.type === 'state') {
      listener?.(data.state);
    }
  });
  sendReady();
}
