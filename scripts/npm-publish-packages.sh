#!/usr/bin/env bash
#
# npm-publish-packages.sh — publish npm packages so that a re-run finishes the
# job instead of failing on what an earlier attempt already uploaded (I-256).
#
# ── Why this exists ─────────────────────────────────────────────────────────
#
# v0.10.6's publish uploaded all five platform packages, then timed out waiting
# for one that npm showed 8.5 minutes later. The re-run could not get past it:
# publishing linux-x64 again failed while npm was still processing the first
# upload, with an error the loop's skip pattern did not match, and the wrapper
# step had no skip at all. The loop decided from `npm publish`'s error text;
# this decides from the registry.
#
# ── What it does, per package folder ────────────────────────────────────────
#
#   1. The version is already in the registry: skip it, without `npm publish`.
#   2. Otherwise `npm publish --access public --provenance`.
#      - It succeeds: published (`npm-await.sh` confirms it later).
#      - It fails with an authentication signature — ENEEDAUTH, E401,
#        401 Unauthorized, or `404 Not Found - PUT`, which is how npm answers an
#        unauthorised publish to an existing package (the expired token of
#        v0.4.0 read like a missing package): stop at once, say so. Never wait
#        on or mask an auth failure.
#      - It fails any other way: wait for the version to appear, at most
#        NPM_PUBLISH_SETTLE seconds and never past NPM_AWAIT_DEADLINE. If it
#        appears, an earlier attempt's upload has finished: count it. If not,
#        fail with npm's output.
#
# ── Usage ───────────────────────────────────────────────────────────────────
#
#   npm-publish-packages.sh <package-dir>...
#
# Environment:
#   NPM_BIN             npm executable (default `npm`), as for npm-await.sh
#   NPM_AWAIT_INTERVAL  seconds between polls (default 10)
#   NPM_AWAIT_DEADLINE  epoch seconds shared with npm-await.sh; no wait runs past it
#   NPM_PUBLISH_SETTLE  longest wait for a failed publish to appear (default 600)
#
# Exit status:
#   0  every package is in the registry, or was published by this run
#   1  one could not be published (named, with npm's output)
#   2  called wrongly (no folder, or a folder without package.json)

set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
await="$here/npm-await.sh"
NPM_BIN=${NPM_BIN:-npm}
SETTLE=${NPM_PUBLISH_SETTLE:-600}

[ $# -ge 1 ] || { echo "usage: ${0##*/} <package-dir>..." >&2; exit 2; }
for dir in "$@"; do
    [ -f "$dir/package.json" ] || { echo "error: no package.json in $dir" >&2; exit 2; }
done

for dir in "$@"; do
    name=$(node -p "require(process.argv[1]).name" "$(cd "$dir" && pwd)/package.json")
    version=$(node -p "require(process.argv[1]).version" "$(cd "$dir" && pwd)/package.json")
    spec="$name@$version"

    if "$await" exists "$name" "$version"; then
        echo "✓ $spec is already published, skipping"
        continue
    fi

    echo "Publishing $spec..."
    if out=$(cd "$dir" && "$NPM_BIN" publish --access public --provenance 2>&1); then
        printf '%s\n' "$out"
        echo "✓ $spec published"
        continue
    fi
    printf '%s\n' "$out"

    if grep -qiE 'ENEEDAUTH|E401|401 Unauthorized|404 Not Found - PUT' <<<"$out"; then
        echo "::error::$spec: npm refused the credentials — check NPM_TOKEN (npm reports an unauthorised publish as 404)"
        exit 1
    fi

    # Not an auth failure: most likely an earlier attempt's upload that npm is
    # still processing. The registry has the answer.
    deadline=$(( $(date +%s) + SETTLE ))
    if [ -n "${NPM_AWAIT_DEADLINE:-}" ] && [ "$NPM_AWAIT_DEADLINE" -lt "$deadline" ]; then
        deadline=$NPM_AWAIT_DEADLINE
    fi
    echo "  npm publish failed; waiting for $spec to appear (an earlier upload may still be processing)"
    if NPM_AWAIT_DEADLINE=$deadline "$await" version "$name" "$version"; then
        echo "✓ $spec is in the registry — published by an earlier attempt"
    else
        echo "::error::$spec failed to publish and did not appear — npm said:"
        printf '%s\n' "$out" | sed 's/^/    /'
        exit 1
    fi
done
