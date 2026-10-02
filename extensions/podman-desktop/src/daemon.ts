// The bundled daemon as a podman container: quadlet-supervised where a user
// systemd session exists, `podman run --restart=always` fallback otherwise.
// The PATH-daemon fallback lives in extension.ts.

import * as api from '@podman-desktop/api';
import { access, mkdir, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';

import { configHome } from './registries.ts';

export const DAEMON_IMAGE = 'localhost/ocid-daemon:ext';
export const CONTAINER_NAME = 'ocid';
export const REGISTRY_PORT = 5050;

/** OCID_HOME for the pod daemon; trust.ts reads its CA from here. */
export function daemonHome(): string {
  return path.join(homedir(), '.local', 'share', 'ocid');
}

async function podman(args: string[]): Promise<api.RunResult> {
  return api.process.exec('podman', args);
}

async function imageExists(): Promise<boolean> {
  try {
    await podman(['image', 'exists', DAEMON_IMAGE]);
    return true;
  } catch {
    return false;
  }
}

export async function isDaemonRunning(): Promise<boolean> {
  try {
    const out = await podman([
      'container',
      'inspect',
      '--format',
      '{{.State.Running}}',
      CONTAINER_NAME,
    ]);
    return out.stdout.trim() === 'true';
  } catch {
    return false;
  }
}

/** Where the bundled image tarball would be (in the extension artifact). */
export function tarballPath(extensionRoot: string): string {
  return path.join(extensionRoot, 'bin', 'ocid-daemon.tar');
}

export async function hasBundledDaemon(extensionRoot: string): Promise<boolean> {
  try {
    await access(tarballPath(extensionRoot));
    return true;
  } catch {
    return false;
  }
}

function quadletPath(): string {
  return path.join(configHome(), 'containers', 'systemd', 'ocid.container');
}

/** Host network keeps the iroh endpoint fully p2p (QUIC + mDNS); the
 *  registry itself stays on loopback. */
const RUN_ARGS = [
  '--network',
  'host',
  '--userns',
  'keep-id:uid=1000,gid=1000',
  '-v',
  `${daemonHome()}:/data:Z`,
  DAEMON_IMAGE,
  '--tls',
  '--listen',
  `127.0.0.1:${REGISTRY_PORT}`,
] as const;

function quadletUnit(): string {
  return [
    '# Managed by the ocid Podman Desktop extension.',
    '[Unit]',
    'Description=ocid daemon (bundled by the Podman Desktop extension)',
    '',
    '[Container]',
    `Image=${DAEMON_IMAGE}`,
    `ContainerName=${CONTAINER_NAME}`,
    'Exec=--tls --listen 127.0.0.1:' + REGISTRY_PORT,
    'Network=host',
    'UserNS=keep-id:uid=1000,gid=1000',
    'Volume=%h/.local/share/ocid:/data:Z',
    '',
    '[Service]',
    'Restart=always',
    '',
    '[Install]',
    'WantedBy=default.target',
    '',
  ].join('\n');
}

/** Start via systemd user unit; false when there is no user systemd session. */
async function startViaQuadlet(): Promise<boolean> {
  try {
    const unit = quadletPath();
    await mkdir(path.dirname(unit), { recursive: true });
    await writeFile(unit, quadletUnit());
    await api.process.exec('systemctl', ['--user', 'daemon-reload']);
    await api.process.exec('systemctl', ['--user', 'start', 'ocid.service']);
    // systemctl start returns before the container is up.
    for (let i = 0; i < 20 && !(await isDaemonRunning()); i++) {
      await new Promise(resolve => setTimeout(resolve, 500));
    }
    return await isDaemonRunning();
  } catch {
    return false;
  }
}

/** Load the bundled image (idempotent) and start the pod — OCID_HOME under
 *  daemonHome(), so the CA lands where trust.ts reads it. */
export async function startDaemonPod(extensionRoot: string): Promise<void> {
  if (!(await imageExists())) {
    await podman(['load', '-i', tarballPath(extensionRoot)]);
  }
  if (await isDaemonRunning()) return;
  await mkdir(daemonHome(), { recursive: true });
  // clear a stopped container from a previous run
  await podman(['rm', '-f', CONTAINER_NAME]).catch(() => undefined);
  if (await startViaQuadlet()) return;
  await podman(['run', '-d', '--name', CONTAINER_NAME, '--restart', 'always', ...RUN_ARGS]);
}

export async function stopDaemonPod(): Promise<void> {
  await api.process.exec('systemctl', ['--user', 'stop', 'ocid.service']).catch(() => undefined);
  await podman(['rm', '-f', CONTAINER_NAME]).catch(() => undefined);
  await rm(quadletPath(), { force: true }).catch(() => undefined);
}
