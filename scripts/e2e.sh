#!/usr/bin/env bash
# End-to-end tests for ocid: two daemons on this host, driven by podman/curl/ocictl.
#
#   just e2e                 build binaries in podman, then run this
#   scripts/e2e.sh           run against ./bin/{ocid,ocictl}
#   OCID_BIN=/path scripts/e2e.sh
#
# Requirements on the host: bash, podman, curl, jq. `oras` enables the
# referrers test (skipped when missing). Nodes use ports 15050/15051/15052
# and a temporary OCID_HOME; everything is cleaned up on exit.
#
# Release timestamps are second-resolution, so consecutive pushes of the same
# image are spaced >1s apart (`tick`) to make `latest`/`last:N` deterministic.

set -euo pipefail

BIN=${OCID_BIN:-"$(cd "$(dirname "$0")/.." && pwd)/bin"}
OCID="$BIN/ocid"
CTL="$BIN/ocictl"
PORT_A=${PORT_A:-15050}
PORT_B=${PORT_B:-15051}
DNS_PORT=${DNS_PORT:-15953}
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
    [[ -n "${PID_C:-}" ]] && kill "$PID_C" 2>/dev/null || true
    [[ -n "${DNSPID:-}" ]] && kill "$DNSPID" 2>/dev/null || true
    sleep 0.5
    # shellcheck disable=SC2046
    podman rmi -f $(podman images --format '{{.Repository}}:{{.Tag}}' | grep -E "^$REG_A/|^$REG_B/|^${REG_C:-unset}/" || true) >/dev/null 2>&1 || true
    if (( FAIL > 0 )); then
        echo "--- node A log ---"; tail -n 30 "$WORK/a.log" || true
        echo "--- node B log ---"; tail -n 30 "$WORK/b.log" || true
        [[ -f "$WORK/c.log" ]] && { echo "--- node C log ---"; tail -n 30 "$WORK/c.log" || true; }
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
# The DNS section below serves zones from a dnsmasq on loopback; point A's
# resolver at it (default is the system resolver, which knows nothing of
# the test zones). Appended after init so the file always has the key.
echo "dns_nameserver = \"127.0.0.1:$DNS_PORT\"" >> "$HOME_A/config.toml"
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
# Listen on the SSE event stream while publishing (ocitop's data source).
curl -sN --max-time 60 "http://$REG_A/_ocid/events" >"$WORK/events.log" &
EVENTS_PID=$!
sleep 0.5
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
# B fetches from A here; listen on its stream for the transfer-progress events.
curl -sN --max-time 60 "http://$REG_B/_ocid/events" >"$WORK/events-b.log" &
EVENTS_B_PID=$!
sleep 0.5
podman pull -q --tls-verify=false "$REG_B/$A/alpine:1" >/dev/null && ok "podman pull via B succeeded" || fail "podman pull via B"
sleep 0.5
kill "$EVENTS_B_PID" 2>/dev/null || true
wait "$EVENTS_B_PID" 2>/dev/null || true
evb=$(cat "$WORK/events-b.log")
assert_contains "pull_progress events during on-demand fetch" "$evb" '"type":"pull_progress"'
assert_not_contains "no fetch_failed during on-demand fetch" "$evb" '"type":"fetch_failed"'
assert_contains "release_saved event on the fetching node" "$evb" '"type":"release_saved"'
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

log "event stream (SSE)"
kill "$EVENTS_PID" 2>/dev/null || true
wait "$EVENTS_PID" 2>/dev/null || true
ev=$(cat "$WORK/events.log")
assert_contains "SSE frames use data: lines" "$ev" "data: {"
assert_contains "release_saved event for the push" "$ev" '"type":"release_saved"'
assert_contains "outbound gossip event" "$ev" '"type":"gossip"'
assert_contains "http_request events while a listener is attached" "$ev" '"type":"http_request"'
assert_not_contains "event stream does not echo itself" "$ev" '/_ocid/events'
assert_not_contains "poll GETs are hidden from the event stream" "$ev" '"path":"/_ocid/releases"'
assert_contains "SSE content-type" "$(curl -s -D - -o /dev/null --max-time 1 "http://$REG_A/_ocid/events" || true)" "text/event-stream"

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
# 6. DNS publisher names (dnsmasq on loopback)
# ---------------------------------------------------------------------------

if command -v dnsmasq >/dev/null; then
    log "DNS publisher names (dnsmasq zone on 127.0.0.1:$DNS_PORT)"
    ZONE="images.ocid.test"
    ZONE2="stale.ocid.test"

    # Dotted self-names must keep working next to DNS resolution: with the
    # resolver unreachable, `my.app` falls through to A's own namespace.
    push_a "$SRC_IMAGE" "my.app"
    podman pull -q --tls-verify=false "$REG_A/my.app" >/dev/null \
        && ok "dotted self-name falls through when DNS is unreachable" || fail "dotted self-name (no server)"

    # B publishes under a human name: sign the _ocid record with B's key.
    REC_B=$(ctl_b dns-record "$ZONE" | awk -F'"' '/IN TXT/{print $2}')
    assert_contains "dns-record prints a signed record for B" "$REC_B" "v=ocid1 k=$B ts="
    dnsmasq --no-daemon --listen-address=127.0.0.1 --port="$DNS_PORT" --no-hosts --no-resolv \
        --txt-record="_ocid.$ZONE,$REC_B" >"$WORK/dnsmasq.log" 2>&1 &
    DNSPID=$!
    # Wait until the port actually answers before the first lookup.
    require "dnsmasq listening on 127.0.0.1:$DNS_PORT" 10 \
        bash -c "exec 3<>/dev/udp/127.0.0.1/$DNS_PORT" 2>/dev/null \
        || fail "dnsmasq did not start (see $WORK/dnsmasq.log)"
    sleep 0.5

    # Pull from A via B's DNS name: A rewrites images.ocid.test -> B's key
    # and fetches the release from B on demand.
    podman push -q --tls-verify=false "$SRC_IMAGE" "$REG_B/app:dns" >/dev/null
    podman pull -q --tls-verify=false "$REG_A/$ZONE/app:dns" >/dev/null \
        && ok "podman pull via DNS publisher name" || fail "pull via DNS name"

    # The control API resolves the same names as the registry: ocictl pull
    # takes a bare domain reference and a full registry reference pasted
    # wholesale (host prefix stripped daemon-side).
    ctl_a pull "$ZONE/app:dns" >/dev/null \
        && ok "ocictl pull via DNS publisher name" || fail "ocictl pull via DNS name"
    ctl_a pull "$REG_A/$ZONE/app:dns" >/dev/null \
        && ok "ocictl pull with pasted registry location" || fail "ocictl pull with host prefix"

    out=$(curl -s "http://$REG_A/_ocid/dns/resolve?zone=$ZONE")
    assert_eq "resolve endpoint: the zone maps to B" "$B" "$(echo "$out" | jq -r .publisher)"
    assert_eq "resolve endpoint: state pinned" "pinned" "$(echo "$out" | jq -r .state)"

    # Policy in domain form: follow + seed by name. Do this while the zone
    # is still pinned to B's key: the daemon resolves the domain (pin
    # matches) and records the mapping, and policy.toml keeps the
    # human-readable domain form.
    ctl_a follow "$ZONE" >/dev/null \
        && ok "follow by domain" || fail "follow by domain"
    ctl_a seed "$ZONE/app:dns" >/dev/null \
        && ok "seed by domain reference" || fail "seed by domain reference"
    assert_contains "policy.toml keeps the domain form" \
        "$(cat "$HOME_A/policy.toml")" "$ZONE"
    assert_contains "policy.toml records the resolved key" \
        "$(cat "$HOME_A/policy.toml")" "$B"
    ctl_a policy | grep -q "$ZONE" \
        && ok "ocictl policy lists the domain rule" || fail "policy lists domain rule"

    # Restart A so the hijack leg below cannot be served from the in-memory
    # lookup cache (the pin store is on disk and must survive).
    kill "$PID_A"; wait "$PID_A" 2>/dev/null || true
    OCID_HOME="$HOME_A" RUST_LOG=ocid=debug,warn "$OCID" --no-relay --listen "$REG_A" >>"$WORK/a.log" 2>&1 &
    PID_A=$!
    require "node A restarted" 15 curl -sf "http://$REG_A/v2/"
    # The restart drops the positive cache; the first resolve re-queries DNS
    # and must land on the pinned key (not the hijacker's).
    out=$(curl -s "http://$REG_A/_ocid/dns/resolve?zone=$ZONE")
    assert_eq "pin survives restart: still B, not the hijacker" "$B" "$(echo "$out" | jq -r .publisher)"

    # A different key claiming a pinned zone is a hijack and must be loud;
    # a record older than the freshness window is refused despite a valid
    # signature.
    HOME_F="$WORK/faker"; mkdir -p "$HOME_F"
    OCID_HOME="$HOME_F" "$CTL" init >/dev/null
    F=$(OCID_HOME="$HOME_F" "$CTL" whoami | awk '/^id/{print $2}')
    REC_F=$(OCID_HOME="$HOME_F" "$CTL" dns-record "$ZONE" | awk -F'"' '/IN TXT/{print $2}')
    HOME_S="$WORK/staler"; mkdir -p "$HOME_S"
    OCID_HOME="$HOME_S" "$CTL" init >/dev/null
    OLD_TS=$(( $(date +%s) - 32*24*3600 ))
    REC_S=$(OCID_HOME="$HOME_S" "$CTL" dns-record --ts "$OLD_TS" "$ZONE2" | awk -F'"' '/IN TXT/{print $2}')
    kill "$DNSPID"; wait "$DNSPID" 2>/dev/null || true
    dnsmasq --no-daemon --listen-address=127.0.0.1 --port="$DNS_PORT" --no-hosts --no-resolv \
        --txt-record="_ocid.$ZONE,$REC_F" --txt-record="_ocid.$ZONE2,$REC_S" >"$WORK/dnsmasq.log" 2>&1 &
    DNSPID=$!
    require "dnsmasq (hijack leg) listening" 10 \
        bash -c "exec 3<>/dev/udp/127.0.0.1/$DNS_PORT" 2>/dev/null \
        || fail "dnsmasq did not restart (see $WORK/dnsmasq.log)"
    sleep 0.5

    out=$(curl -s "http://$REG_A/v2/$ZONE/app/manifests/dns")
    assert_contains "pinned zone hijack is rejected" "$out" "refusing to switch keys"
    if podman pull -q --tls-verify=false "$REG_A/$ZONE/app:dns" >/dev/null 2>&1; then
        fail "pull via a hijacked zone must fail"
    else
        ok "pull via a hijacked zone fails"
    fi
    out=$(curl -s "http://$REG_A/v2/$ZONE2/app/manifests/dns")
    assert_contains "stale record is rejected" "$out" "is stale:"

    # Rotation: unpin, re-resolve — the zone then maps to the new key, and
    # the pin survives another daemon restart.
    out=$(curl -sf -X POST "http://$REG_A/_ocid/dns/unpin" -H 'content-type: application/json' -d "{\"zone\":\"$ZONE\"}")
    assert_contains "unpin reports removal" "$out" '"removed":true'
    out=$(curl -s "http://$REG_A/_ocid/dns/resolve?zone=$ZONE")
    assert_eq "re-resolve pins the new key" "$F" "$(echo "$out" | jq -r .publisher)"
    assert_eq "fresh pin state" "new" "$(echo "$out" | jq -r .state)"
    kill "$PID_A"; wait "$PID_A" 2>/dev/null || true
    OCID_HOME="$HOME_A" RUST_LOG=ocid=debug,warn "$OCID" --no-relay --listen "$REG_A" >>"$WORK/a.log" 2>&1 &
    PID_A=$!
    require "node A restarted again" 15 curl -sf "http://$REG_A/v2/"
    out=$(curl -s "http://$REG_A/_ocid/dns/resolve?zone=$ZONE")
    assert_eq "pin survives daemon restarts" "$F" "$(echo "$out" | jq -r .publisher)"
    assert_eq "state pinned after restart" "pinned" "$(echo "$out" | jq -r .state)"

    # The CLI surface: resolve reports the pinned state, dns-unpin drops it.
    out=$(ctl_a resolve "$ZONE")
    assert_contains "ocictl resolve shows the pinned publisher" "$out" "$F"
    assert_contains "ocictl resolve shows pinned state" "$out" "pinned"
    out=$(ctl_a dns-unpin "$ZONE")
    assert_contains "ocictl dns-unpin reports removal" "$out" "unpinned"
    out=$(ctl_a resolve "$ZONE")
    assert_eq "re-resolve after CLI unpin re-pins the same key" "$F" \
        "$(echo "$out" | awk '/^publisher/{print $2}')"

    # Clean up the domain-form policy rules so later sections see a
    # pristine policy.
    ctl_a unfollow "$ZONE" >/dev/null
    ctl_a unseed "$ZONE/app:dns" >/dev/null
else
    skip "dnsmasq not installed; DNS publisher-name tests skipped"
fi

# ---------------------------------------------------------------------------
# 7. referrers (oras)
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
# 8. TLS registry (config tls = "auto")
# ---------------------------------------------------------------------------

log "TLS registry"
PORT_C=${PORT_C:-15052}
REG_C="127.0.0.1:$PORT_C"
HOME_C="$WORK/c"
mkdir -p "$HOME_C"
OCID_HOME="$HOME_C" RUST_LOG=ocid=info,warn "$OCID" --no-relay --tls --listen "$REG_C" >"$WORK/c.log" 2>&1 &
PID_C=$!
CA="$HOME_C/tls/ca.crt"
require "node C up (https, CA-verified)" 15 curl -sf --cacert "$CA" "https://$REG_C/v2/"
assert_contains "TLS material generated" "$(ls "$HOME_C/tls")" "ca.crt"
assert_contains "config persisted tls for C" 'tls = "auto"' "$(grep ^tls "$HOME_C/config.toml")"
assert_contains "ocictl speaks https to the TLS daemon" "$(OCID_HOME="$HOME_C" "$CTL" status)" "https://$REG_C"
if curl -sf "http://$REG_C/v2/" >/dev/null 2>&1; then
    fail "plain http is rejected by the TLS registry"
else
    ok "plain http is rejected by the TLS registry"
fi

log "TLS registry: verified push and pull"
CERTS="$WORK/certs"
mkdir -p "$CERTS"
cp "$CA" "$CERTS/ca.crt"
# podman's --cert-dir is a *flat* directory (containers/image DockerCertPath):
# every *.crt in it is treated as a root CA.
podman push -q --cert-dir "$CERTS" "$SRC_IMAGE" "$REG_C/alpine:9" >/dev/null \
    && ok "podman push over verified TLS" || fail "podman push over verified TLS"
podman pull -q --cert-dir "$CERTS" "$REG_C/alpine:9" >/dev/null \
    && ok "podman pull over verified TLS" || fail "podman pull over verified TLS"
if podman pull -q "$REG_C/alpine:9" >/dev/null 2>&1; then
    fail "pull without the CA fails against the TLS registry"
else
    ok "pull without the CA fails against the TLS registry"
fi

# ---------------------------------------------------------------------------
# 9. ocictl offline
# ---------------------------------------------------------------------------

log "ocictl works without a daemon"
kill "$PID_B"; wait "$PID_B" 2>/dev/null || true; PID_B=
sleep 0.5
assert_contains "ls offline reads the index" "$(ctl_b ls)" "daemon not running"
assert_contains "status reports daemon down" "$(ctl_b status 2>&1 || true)" "is not running"

log "done"
