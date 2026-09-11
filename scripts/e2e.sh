#!/usr/bin/env bash
# End-to-end tests for ocid: two daemons on this host, driven by podman/curl/ocictl.
#
#   just e2e                 build binaries in podman, then run this
#   scripts/e2e.sh           run against ./bin/{ocid,ocictl}
#   OCID_BIN=/path scripts/e2e.sh
#
# Requirements on the host: bash, podman, curl, jq. `oras` enables the
# referrers test (skipped when missing). Nodes use ports 15050/15051 and a
# temporary OCID_HOME; everything is cleaned up on exit.
#
# Release timestamps are second-resolution, so consecutive pushes of the same
# image are spaced >1s apart (`tick`) to make `latest`/`last:N` deterministic.

set -euo pipefail

BIN=${OCID_BIN:-"$(cd "$(dirname "$0")/.." && pwd)/bin"}
OCID="$BIN/ocid"
CTL="$BIN/ocictl"
PORT_A=${PORT_A:-15050}
PORT_B=${PORT_B:-15051}
REG_A="127.0.0.1:$PORT_A"
REG_B="127.0.0.1:$PORT_B"
SRC_IMAGE=${SRC_IMAGE:-docker.io/library/alpine:latest}
SRC_IMAGE2=${SRC_IMAGE2:-docker.io/library/busybox:latest}
WORK=$(mktemp -d /tmp/ocid-e2e.XXXXXX)
HOME_A="$WORK/a"
HOME_B="$WORK/b"
PASS=0
FAIL=0

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

log()  { printf '\033[1;34m==> %s\033[0m\n' "$*"; }
ok()   { PASS=$((PASS + 1)); printf '  \033[32mok\033[0m   %s\n' "$*"; }
fail() { FAIL=$((FAIL + 1)); printf '  \033[31mFAIL\033[0m %s\n' "$*"; }
skip() { printf '  \033[33mskip\033[0m %s\n' "$*"; }

# assert_eq "description" expected actual
assert_eq() {
    if [[ "$2" == "$3" ]]; then ok "$1"; else fail "$1: expected '$2', got '$3'"; fi
}
# assert_contains "description" haystack needle
assert_contains() {
    if [[ "$2" == *"$3"* ]]; then ok "$1"; else fail "$1: '$3' not found in: $2"; fi
}
assert_not_contains() {
    if [[ "$2" != *"$3"* ]]; then ok "$1"; else fail "$1: unexpected '$3' in: $2"; fi
}

# wait_for "description" timeout_secs command...   (retries until exit 0)
# Records ok/FAIL and always returns 0 so the run continues.
wait_for() {
    local desc=$1 timeout=$2; shift 2
    local end=$((SECONDS + timeout))
    while ! "$@" >/dev/null 2>&1; do
        if (( SECONDS >= end )); then fail "$desc (timeout ${timeout}s)"; return 0; fi
        sleep 0.5
    done
    ok "$desc"
}
# require: like wait_for but aborts the run (used for node startup).
require() {
    local desc=$1 timeout=$2; shift 2
    local end=$((SECONDS + timeout))
    while ! "$@" >/dev/null 2>&1; do
        if (( SECONDS >= end )); then fail "$desc (timeout ${timeout}s)"; exit 1; fi
        sleep 0.5
    done
    ok "$desc"
}

ctl_a() { OCID_HOME="$HOME_A" "$CTL" "$@"; }
ctl_b() { OCID_HOME="$HOME_B" "$CTL" "$@"; }
push_a() { podman push -q --tls-verify=false "$1" "$REG_A/$2" >/dev/null; }
tick() { sleep 1.2; }

# b_tags <image-name> -> space separated sorted tag list held by B
b_tags() { ctl_b ls 2>/dev/null | awk -v img="$1" 'NR>1 && $2==img {print $3}' | sort | tr '\n' ' ' | sed 's/ $//'; }
has_tags() { [[ "$(b_tags "$1")" == "$2" ]]; }

cleanup() {
    log "cleanup"
    [[ -n "${PID_A:-}" ]] && kill "$PID_A" 2>/dev/null || true
    [[ -n "${PID_B:-}" ]] && kill "$PID_B" 2>/dev/null || true
    sleep 0.5
    # shellcheck disable=SC2046
    podman rmi -f $(podman images --format '{{.Repository}}:{{.Tag}}' | grep -E "^$REG_A/|^$REG_B/" || true) >/dev/null 2>&1 || true
    if (( FAIL > 0 )); then
        echo "--- node A log ---"; tail -n 30 "$WORK/a.log" || true
        echo "--- node B log ---"; tail -n 30 "$WORK/b.log" || true
    fi
    rm -rf "$WORK"
    echo
    echo "passed: $PASS  failed: $FAIL"
    (( FAIL == 0 ))
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# preflight
# ---------------------------------------------------------------------------

for tool in podman curl jq; do
    command -v "$tool" >/dev/null || { echo "missing $tool"; exit 2; }
done
[[ -x "$OCID" && -x "$CTL" ]] || { echo "binaries not found in $BIN (run 'just bin')"; exit 2; }
log "pulling source images"
podman pull -q "$SRC_IMAGE" >/dev/null
podman pull -q "$SRC_IMAGE2" >/dev/null

# ---------------------------------------------------------------------------
# 1. start two nodes
# ---------------------------------------------------------------------------

log "starting node A ($REG_A) and node B ($REG_B)"
mkdir -p "$HOME_A" "$HOME_B"
# init first so we can shorten the blob GC tick (daemon clamps it to >= 5s)
ctl_a init >/dev/null
sed -i 's/^blob_gc_interval_secs = .*/blob_gc_interval_secs = 5/' "$HOME_A/config.toml"
OCID_HOME="$HOME_A" RUST_LOG=ocid=debug,warn "$OCID" --no-relay --listen "$REG_A" >"$WORK/a.log" 2>&1 &
PID_A=$!
require "node A up" 15 curl -sf "http://$REG_A/v2/"
TICKET_A=$(ctl_a ticket)
A=$(ctl_a whoami | awk '/^id/{print $2}')
A_DID=$(ctl_a whoami | awk '/^did/{print $2}')

OCID_HOME="$HOME_B" RUST_LOG=ocid=debug,warn "$OCID" --no-relay --listen "$REG_B" --peer "$TICKET_A" >"$WORK/b.log" 2>&1 &
PID_B=$!
require "node B up" 15 curl -sf "http://$REG_B/v2/"
B=$(ctl_b whoami | awk '/^id/{print $2}')
require "B sees A as neighbor" 20 bash -c "OCID_HOME='$HOME_B' '$CTL' peers | grep -q '$A'"

assert_eq "config persisted listen for B" "listen = \"$REG_B\"" "$(grep ^listen "$HOME_B/config.toml")"
assert_contains "did:key form" "$A_DID" "did:key:z6Mk"

# ---------------------------------------------------------------------------
# 2. publish on A
# ---------------------------------------------------------------------------

log "publish alpine:1 on A"
push_a "$SRC_IMAGE" alpine:1
assert_contains "A lists alpine:1" "$(ctl_a ls)" "alpine"
REL="$HOME_A/index/releases/$A/alpine/1.json"
[[ -f "$REL" ]] && ok "release record written" || fail "release record missing"
assert_eq "release publisher is A" "$A" "$(jq -r .payload.publisher "$REL")"
sig=$(jq -r .signature "$REL"); assert_eq "release signature present (64-byte hex)" 128 "${#sig}"
DIGEST=$(jq -r .payload.manifest.digest "$REL")

log "push into another publisher's namespace is refused"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://$REG_A/v2/$B/x/blobs/uploads/")
assert_eq "POST upload into $B/ -> 403" 403 "$code"

# ---------------------------------------------------------------------------
# 3. on-demand pull on B (no policy): registry fetches from peers
# ---------------------------------------------------------------------------

log "podman pull on B of an image it never saw"
podman pull -q --tls-verify=false "$REG_B/$A/alpine:1" >/dev/null && ok "podman pull via B succeeded" || fail "podman pull via B"
assert_contains "B now holds alpine:1 (cache, no policy)" "$(ctl_b ls)" "alpine"
assert_contains "ls shows '-' policy for cached image" "$(ctl_b ls | awk 'NR>1 && $2=="alpine"')" " - "
out=$(podman run --rm "$REG_B/$A/alpine:1" echo hello-from-p2p)
assert_eq "container runs from replicated image" "hello-from-p2p" "$out"

# ---------------------------------------------------------------------------
# 4. registry features: range, delete, metrics
# ---------------------------------------------------------------------------

log "range requests"
LAYER=$(curl -s "http://$REG_A/v2/alpine/manifests/1" | jq -r '.layers[0].digest')
hdr=$(curl -s -D - -o /dev/null -H 'Range: bytes=100-199' "http://$REG_A/v2/alpine/blobs/$LAYER")
assert_contains "206 partial content" "$hdr" "206"
assert_contains "content-range header" "$hdr" "bytes 100-199/"
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Range: bytes=99999999999-' "http://$REG_A/v2/alpine/blobs/$LAYER")
assert_eq "unsatisfiable range -> 416" 416 "$code"

log "delete semantics"
code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE "http://$REG_A/v2/alpine/blobs/$LAYER")
assert_eq "DELETE blob -> 405" 405 "$code"
push_a "$SRC_IMAGE2" busybox:1
files_before=$(find "$HOME_A/blobs" -type f | wc -l)
code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE "http://$REG_A/v2/busybox/manifests/1")
assert_eq "DELETE manifest by tag -> 202" 202 "$code"
assert_not_contains "busybox gone from A" "$(ctl_a ls)" "busybox"
wait_for "busybox layer reclaimed by blob GC" 30 bash -c "[ \$(find '$HOME_A/blobs' -type f | wc -l) -lt $files_before ]"

log "metrics"
m=$(curl -s "http://$REG_A/metrics")
assert_contains "openmetrics EOF" "$m" "# EOF"
assert_contains "ocid_releases_published_total" "$m" "ocid_releases_published_total 2"
assert_contains "labelled http counter" "$m" 'ocid_http_requests_total{method="PUT",route="manifests",status="201"}'
assert_contains "iroh metrics included" "$m" "iroh_"
assert_contains "openmetrics content-type" "$(curl -s -D - -o /dev/null "http://$REG_A/metrics")" "application/openmetrics-text"

# ---------------------------------------------------------------------------
# 5. policy windows: latest / last:N / pins
# ---------------------------------------------------------------------------

log "policy: follow (default latest)"
ctl_b rm "$A/alpine:1" >/dev/null
tick; push_a "$SRC_IMAGE" alpine:2
tick; push_a "$SRC_IMAGE" alpine:3
assert_contains "B is only on its own gossip topic" "$(curl -s "http://$REG_B/metrics")" "ocid_gossip_topics 1"
ctl_b follow "$A" >/dev/null
wait_for "B holds only the latest tag (3)" 20 has_tags alpine "3"
assert_contains "ls shows policy 'latest'" "$(ctl_b ls | awk 'NR>1 && $2=="alpine"')" "latest"
assert_contains "B joined A's gossip topic" "$(curl -s "http://$REG_B/metrics")" "ocid_gossip_topics 2"

log "policy: seed --last 2 widens the window"
ctl_b seed "$A/alpine" --last 2 >/dev/null
wait_for "B holds tags 2 and 3" 20 has_tags alpine "2 3"

log "policy: new release on A rolls the window (prunes 2)"
tick; push_a "$SRC_IMAGE" alpine:4
wait_for "B holds tags 3 and 4" 20 has_tags alpine "3 4"
assert_contains "prune logged" "$(cat "$WORK/b.log")" "pruned"

log "policy: pin fetches and protects a release outside the window"
ctl_b pin "$A/alpine:1" >/dev/null
wait_for "B holds 1 (pinned), 3, 4" 20 has_tags alpine "1 3 4"
tick; push_a "$SRC_IMAGE" alpine:5
wait_for "B holds 1 (pinned), 4, 5" 20 has_tags alpine "1 4 5"
assert_contains "ls marks the pin" "$(ctl_b ls | awk 'NR>1 && $3=="1"')" "pin"
assert_eq "gc dry-run removes nothing that policy keeps" "would remove 0 release(s), 0 blob(s), 0 B freed" "$(ctl_b gc --dry-run | tail -1)"

log "policy: narrowing the policy prunes immediately; unruled leftovers become cache"
ctl_b unpin "$A/alpine:1" >/dev/null
wait_for "unpin drops 1 -> B holds 4, 5" 10 has_tags alpine "4 5"
ctl_b unseed "$A/alpine" >/dev/null
wait_for "unseed falls back to follow latest -> B holds 5" 10 has_tags alpine "5"
ctl_b unfollow "$A" >/dev/null
sleep 0.5
assert_eq "unfollow keeps 5 as cache" "5" "$(b_tags alpine)"
assert_contains "B left A's gossip topic" "$(curl -s "http://$REG_B/metrics")" "ocid_gossip_topics 1"
assert_contains "ls shows '-' policy for cache" "$(ctl_b ls | awk 'NR>1 && $2=="alpine"')" " - "
assert_eq "gc dry-run respects the grace period" "would remove 0 release(s), 0 blob(s), 0 B freed" "$(ctl_b gc --dry-run | tail -1)"
out=$(ctl_b gc --force)
assert_contains "gc --force removes the cached release" "$out" "removed 1 release(s), 3 blob(s)"
assert_not_contains "B empty after gc" "$(ctl_b ls)" "alpine"

log "aliases"
ctl_b track "$A" --as alice >/dev/null
podman pull -q --tls-verify=false "$REG_B/alice/alpine:5" >/dev/null && ok "pull via publisher alias" || fail "pull via alias"
ctl_b untrack alice >/dev/null

# ---------------------------------------------------------------------------
# 6. referrers (oras)
# ---------------------------------------------------------------------------

if command -v oras >/dev/null; then
    log "referrers: oras attach on A, discover on B"
    ctl_b follow "$A" >/dev/null
    wait_for "B follows A (has alpine:5)" 20 has_tags alpine "5"
    echo '{"e2e":"sbom"}' >"$WORK/sbom.json"
    # oras rejects file paths outside the working directory
    (cd "$WORK" && oras attach --plain-http --artifact-type application/vnd.e2e.sbom+json "$REG_A/alpine:5" sbom.json:application/json >/dev/null 2>&1) \
        && ok "oras attach" || fail "oras attach"
    wait_for "B replicated the referrer" 20 bash -c "oras discover --plain-http --format json '$REG_B/$A/alpine:5' 2>/dev/null | grep -q vnd.e2e.sbom"
    subj=$(curl -s "http://$REG_A/v2/alpine/referrers/$DIGEST" | jq -r '.manifests[0].artifactType')
    assert_eq "referrers API on A" "application/vnd.e2e.sbom+json" "$subj"
    ctl_b unfollow "$A" >/dev/null
else
    skip "oras not installed; referrers test skipped"
fi

# ---------------------------------------------------------------------------
# 7. ocictl offline
# ---------------------------------------------------------------------------

log "ocictl works without a daemon"
kill "$PID_B"; wait "$PID_B" 2>/dev/null || true; PID_B=
sleep 0.5
assert_contains "ls offline reads the index" "$(ctl_b ls)" "daemon not running"
assert_contains "status reports daemon down" "$(ctl_b status 2>&1 || true)" "is not running"

log "done"
