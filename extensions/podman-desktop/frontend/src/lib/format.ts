// Shared display helpers for the webview (pure functions, no state).

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KiB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MiB`;
  return `${(n / 1024 ** 3).toFixed(1)} GiB`;
}

/** Deterministic color for a publisher key, stable across the dashboard. */
export function publisherColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return `hsl(${h % 360} 55% 55%)`;
}

export function shortId(id: string): string {
  return `${id.slice(0, 8)}…`;
}

/** Daemon DTOs carry epoch seconds; `nowMs` is epoch milliseconds. */
export function timeAgo(tsSecs: number, nowMs: number): string {
  const s = Math.max(0, Math.floor(nowMs / 1000) - tsSecs);
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

/** Extension-stamped event times are epoch milliseconds. */
export function clock(tsMs: number): string {
  return new Date(tsMs).toLocaleTimeString(undefined, { hour12: false });
}
