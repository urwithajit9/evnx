#!/usr/bin/env bash
# Release gate: a binary carries every optional feature (I-200, DEC-SF8).
#
#   scripts/check-build-features.sh <evnx> [feature ...]
#
# Reads the compiled feature list from `evnx commands --json` rather than
# grepping `--help` for command names. The grep checked four commands (auth,
# vault, cloud, org) and so never noticed a binary without `migrate` or
# `backup`; it also depended on how clap lays out `--help`. Every prebuilt
# binary is built with `--all-features`, so the default is all four.
#
# Exit 0: every feature is there. 1: some are missing (named). 2: no answer —
# the binary did not run, printed no JSON, or jq is not installed.
set -euo pipefail

bin="${1:?usage: check-build-features.sh <evnx> [feature ...]}"
shift
want=("$@")
[ ${#want[@]} -gt 0 ] || want=(net migrate backup cloud)

command -v jq >/dev/null || { echo "error: jq is needed to read evnx commands --json" >&2; exit 2; }
json="$("$bin" commands --json)" || { echo "error: '$bin commands --json' failed" >&2; exit 2; }
have="$(jq -r '.features[]' <<<"$json" 2>/dev/null)" \
  || { echo "error: '$bin commands --json' printed no feature list" >&2; exit 2; }
version="$(jq -r '.evnx_version' <<<"$json")"

missing=()
for f in "${want[@]}"; do
  grep -qx "$f" <<<"$have" || missing+=("$f")
done

if [ ${#missing[@]} -ne 0 ]; then
  echo "evnx $version is missing: ${missing[*]} (built with: $(echo $have))" >&2
  exit 1
fi
echo "✓ evnx $version carries: ${want[*]}"
