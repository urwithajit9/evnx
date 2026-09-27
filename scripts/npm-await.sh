#!/usr/bin/env bash
#
# npm-await.sh — wait for npm's registry to actually show what we published,
# and assert what it shows.
#
# ── Why this exists ─────────────────────────────────────────────────────────
#
# npm's publish pipeline is asynchronous. `npm publish` returning 0 means the
# tarball was accepted, not that the version is resolvable, and not that the
# `latest` dist-tag has moved. Three npm releases were believed published while
# nothing had been, and then v0.5.2 — which published correctly to all six
# packages — went red on a smoke test that gave up after 90 seconds.
#
# So the rule is: **read the registry back, and assert what it says.** A publish
# step's exit code is not evidence.
#
# Two things are asserted separately because they fail separately:
#
#   version   the exact version is resolvable
#   dist-tag  `latest` actually points at it
#
# The second is the one that used to be invisible. The smoke test installs
# `@evnx/cli@$VERSION` — pinned — so it passes even when `latest` still points at
# the previous release, which is what plain `npm install -g @evnx/cli` gives
# every user.
#
# ── Usage ───────────────────────────────────────────────────────────────────
#
#   npm-await.sh version  <pkg> <version> [timeout_seconds]
#   npm-await.sh dist-tag <pkg> <tag> <expected_version> [timeout_seconds]
#
#   npm-await.sh version  @evnx/cli 0.5.3
#   npm-await.sh dist-tag @evnx/cli latest 0.5.3 120
#
# Environment:
#   NPM_AWAIT_INTERVAL  seconds between polls (default 10)
#   NPM_AWAIT_TIMEOUT   default timeout when the argument is omitted (default 300)
#   NPM_BIN             npm executable (default `npm`) — the seam the tests use
#
# Exit status:
#   0  the registry shows what was expected
#   1  it did not, within the timeout — the message names what was seen instead
#   2  called wrongly (bad subcommand or missing argument)
#
# ⚠️ Every query passes `--prefer-online`. Without it npm may serve a cached
# 404 for the whole polling window, so the loop would expire on stale data while
# the version was live — a poll that cannot observe change is just a sleep.

set -uo pipefail

NPM_BIN=${NPM_BIN:-npm}
INTERVAL=${NPM_AWAIT_INTERVAL:-10}
DEFAULT_TIMEOUT=${NPM_AWAIT_TIMEOUT:-300}

die_usage() {
    printf 'usage: %s version  <pkg> <version> [timeout]\n' "${0##*/}" >&2
    printf '       %s dist-tag <pkg> <tag> <expected> [timeout]\n' "${0##*/}" >&2
    exit 2
}

# Read one field from the registry into the globals FIELD_VALUE and
# LAST_NPM_OUTPUT. Returns npm's own status.
#
# ⚠️ Sets globals rather than printing, and callers must NOT wrap it in `$()`.
# It did print, and the caller captured it with `got=$(npm_field …)` — which runs
# in a subshell, so LAST_NPM_OUTPUT never reached the parent and every diagnostic
# that reads it was dead code. Caught by testing the timeout path against a stub.
npm_field() {   # npm_field <spec> <field>
    local out rc
    out=$("$NPM_BIN" view --prefer-online "$1" "$2" 2>&1); rc=$?
    LAST_NPM_OUTPUT=$out
    FIELD_VALUE=
    [ "$rc" -ne 0 ] && return "$rc"
    # Last non-empty line, quotes and whitespace stripped. npm prints a bare
    # value for a single field, but majors have differed on quoting and on
    # echoing the spec first, so this does not assume a format.
    FIELD_VALUE=$(printf '%s\n' "$out" \
        | sed -e "s/^[[:space:]]*//" -e "s/[[:space:]]*$//" -e "s/^['\"]//" -e "s/['\"]\$//" \
        | grep -v '^$' \
        | tail -1)
    return 0
}

# Poll until `npm_field <spec> <field>` equals <want>.
poll_until() {   # poll_until <spec> <field> <want> <timeout> <label>
    local spec=$1 field=$2 want=$3 timeout=$4 label=$5
    local waited=0 got attempt=0

    printf '  awaiting %s\n' "$label"
    while :; do
        attempt=$((attempt + 1))
        npm_field "$spec" "$field"   # sets FIELD_VALUE and LAST_NPM_OUTPUT
        got=$FIELD_VALUE
        if [ "$got" = "$want" ]; then
            printf '  ✓ %s after %ss (attempt %s)\n' "$label" "$waited" "$attempt"
            return 0
        fi

        if [ "$waited" -ge "$timeout" ]; then
            printf '::error::%s — gave up after %ss\n' "$label" "$waited"
            if [ -n "$got" ]; then
                printf '  registry says %s, expected %s\n' "$got" "$want" >&2
            else
                printf '  the registry returned nothing for %s\n' "$spec" >&2
                # Distinguish "no such package" from "package exists, version
                # missing" — they mean very different things at release time.
                case "${LAST_NPM_OUTPUT:-}" in
                    *'No match found for version'*)
                        printf '  the package exists but this version was never published\n' >&2 ;;
                    *E404*)
                        printf '  the package itself is not in the registry — check the scope and the token\n' >&2 ;;
                esac
                printf '  last npm output:\n' >&2
                printf '%s\n' "${LAST_NPM_OUTPUT:-(none)}" | sed 's/^/    /' | head -8 >&2
            fi
            return 1
        fi

        # Only mention the intermediate state when there is one; a plain 404
        # while propagating is expected and would just be noise.
        if [ -n "$got" ]; then
            printf '    %ss: %s is %s, waiting for %s\n' "$waited" "$field" "$got" "$want"
        else
            printf '    %ss: not resolvable yet\n' "$waited"
        fi
        sleep "$INTERVAL"
        waited=$((waited + INTERVAL))
    done
}

[ $# -ge 1 ] || die_usage
sub=$1; shift

case "$sub" in
    version)
        [ $# -ge 2 ] || die_usage
        pkg=$1; want=$2; timeout=${3:-$DEFAULT_TIMEOUT}
        poll_until "$pkg@$want" version "$want" "$timeout" "$pkg@$want to be resolvable"
        ;;
    dist-tag)
        [ $# -ge 3 ] || die_usage
        pkg=$1; tag=$2; want=$3; timeout=${4:-$DEFAULT_TIMEOUT}
        poll_until "$pkg" "dist-tags.$tag" "$want" "$timeout" "$pkg dist-tag '$tag' to be $want"
        ;;
    *)
        die_usage
        ;;
esac
