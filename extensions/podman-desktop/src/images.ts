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

/** Destination `name:tag` on the ocid registry for a source reference.
 *  Digest-pinned sources (`name@sha256:…`) carry no tag, and pushing to a
 *  digest-pinned destination is rejected by podman unless the digests
 *  happen to match — so a deterministic tag is derived from the digest
 *  (`sha256-<12 hex>`). `name:tag@sha256:…` keeps its real tag. */
export function ocidTarget(source: string): string {
  const at = source.indexOf('@');
  if (at === -1) return ocidName(source);
  const named = ocidName(source.slice(0, at));
  if (named.includes(':')) return named;
  const hex = source.slice(at + 1).replace(/^sha256:/, '');
  return `${named}:sha256-${hex.slice(0, 12)}`;
}
