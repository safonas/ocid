// The object Podman Desktop actually passes to `dashboard/image` menu
// commands is the Images-page UI model (name/tag/engineName — see PD's
// renderer ImageInfoUI), not the api.ImageInfo (RepoTags/engineType) the
// command signature suggests. Handle both shapes.

/** Structural type covering both the documented api.ImageInfo and the
 *  ImageInfoUI the Images page really sends. */
export interface MenuImage {
  id?: string;
  name?: string;
  tag?: string;
  RepoTags?: string[];
  engineType?: string;
  engineName?: string;
}

/** Full `repo:tag` source reference for podman push, or undefined for
 *  untagged images (name/tag are '<none>' / ''). */
export function menuImageSource(image: MenuImage | undefined): string | undefined {
  if (!image) return undefined;
  const repoTag = image.RepoTags?.find(t => t && !t.includes('<none>'));
  if (repoTag) return repoTag;
  if (image.name && image.name !== '<none>' && image.tag && image.tag !== '<none>') {
    return `${image.name}:${image.tag}`;
  }
  return undefined;
}

/** True when the image lives in a podman engine — the only engine the
 *  host podman CLI (and therefore our push) can reach. */
export function isPodmanEngine(image: MenuImage | undefined): boolean {
  if (!image) return false;
  const engine = image.engineType ?? image.engineName ?? '';
  return engine.toLowerCase().includes('podman');
}

/** docker.io/library/alpine:latest -> alpine:latest; quay.io/org/app -> org/app. */
export function ocidName(repoTag: string): string {
  let rest = repoTag;
  const slash = rest.indexOf('/');
  const first = slash === -1 ? '' : rest.slice(0, slash);
  if (slash !== -1 && (first.includes('.') || first.includes(':') || first === 'localhost')) {
    rest = rest.slice(slash + 1);
    if (rest.startsWith('library/')) rest = rest.slice('library/'.length);
  }
  return rest;
}

/** A leading path segment that names a publisher's namespace on the ocid
 * registry: a 64-hex publisher id, or a DNS publisher name (dotted,
 * hostname charset — the daemon resolves those forms to another publisher
 * and denies pushes into any namespace but our own). Mirrors ocid-core's
 * is_domain_name, including the all-numeric-label exclusion. */
function isPublisherSegment(seg: string): boolean {
  if (/^[0-9a-f]{64}$/.test(seg)) return true;
  if (!/^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$/.test(seg) || !seg.includes('.')) return false;
  return !seg.split('.').some(l => /^[0-9]+$/.test(l));
}

/** Drop a leading `<publisher>/` namespace segment, if any. A single
 * remaining segment is an own-namespace image (implicit self) and keeps
 * its name. */
function stripPublisherNamespace(name: string): string {
  const slash = name.indexOf('/');
  if (slash === -1) return name;
  const first = name.slice(0, slash);
  const rest = name.slice(slash + 1);
  return rest && isPublisherSegment(first) ? rest : name;
}

/** Resolve digest-pinned references to a pushable `name:tag` — the digest
 * becomes a deterministic tag when the name carries no real one. */
function withDerivedTag(named: string): string {
  const at = named.indexOf('@');
  if (at === -1) return named;
  const name = named.slice(0, at);
  if (name.includes(':')) return name;
  const hex = named.slice(at + 1).replace(/^sha256:/, '');
  return `${name}:sha256-${hex.slice(0, 12)}`;
}

/** Destination `name:tag` on the ocid registry for a source reference.
 *
 * Images previously pulled from this ocid registry carry their source
 * publisher's namespace (`<host>/<hex|domain>/name:tag`) — re-pushing
 * must land in OUR namespace (the daemon denies pushes into another
 * publisher's), so the publisher segment is stripped when the source
 * host matches the ocid registry. Digest-pinned sources
 * (`name@sha256:…`) carry no tag, and pushing to a digest-pinned
 * destination is rejected by podman unless the digests happen to match —
 * so a deterministic tag is derived from the digest (`sha256-<12 hex>`).
 * `name:tag@sha256:…` keeps its real tag. */
export function ocidTarget(source: string, registryHost?: string): string {
  if (registryHost && source.startsWith(`${registryHost}/`)) {
    const rest = source.slice(registryHost.length + 1);
    return withDerivedTag(stripPublisherNamespace(rest));
  }
  const at = source.indexOf('@');
  if (at === -1) return ocidName(source);
  return withDerivedTag(ocidName(source.slice(0, at)) + source.slice(at));
}
