// Normalize what users paste into the dashboard pull field: a bare
// `<publisher>/<name>[:<tag>]` reference, a full registry reference
// (`127.0.0.1:5050/<publisher>/<name>:<tag>` — the host prefix is
// stripped daemon-side), or a whole `podman pull …` / `docker pull …`
// command as copied from the releases table.

export function normalizePullRef(input: string): string {
  return input
    .trim()
    .replace(/^(?:podman|docker)\s+pull\s+/i, '')
    .trim();
}
