#!/usr/bin/env bash
# C9: this repository's .pre-commit-hooks.yaml, run by both hook managers.
#
# Usage: tests/hooks/try-repo.sh <dir holding the evnx binary>
#
# `evnx-scan-installed` (language: system) must block a staged secret under
# pre-commit and under prek. A hook-file change that breaks either manager
# then fails CI, not someone's commit. The `language: rust` hooks are not run
# here: each would build evnx from source on every run.
set -euo pipefail

bin_dir=$(cd "$1" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
export PATH="$bin_dir:$PATH"
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com
export GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com

fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
cd "$fixture"
git init -q
# Assembled here, so no whole key sits in this file.
printf 'AWS_ACCESS_KEY_ID=AKIA%s\n' 4OZRMFJ3VREALKEY > a.env
git add a.env

fail=0
for manager in pre-commit prek; do
    set +e
    "$manager" try-repo "$repo" evnx-scan-installed --files a.env >out.txt 2>&1
    code=$?
    set -e
    if [ "$code" -ne 1 ] || ! grep -q "aws-access-key\|AWS" out.txt; then
        echo "FAIL: $manager try-repo exited $code, want 1 with a finding:" >&2
        cat out.txt >&2
        fail=1
    else
        echo "ok: $manager blocks a staged secret"
    fi
done
exit "$fail"
