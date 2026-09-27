#!/usr/bin/env bash
#
# verify-claims.sh — check documented claims about evnx by running it.
#
# Every claim below is a proposition with a stable ID. The script runs the
# commands that settle it and prints CONFIRMED, REFUTED or SKIP. A REFUTED
# claim means the documented fact is now wrong and something needs updating —
# either the code or the document.
#
# ── Why this exists ─────────────────────────────────────────────────────────
#
# This replaces an 11-step checklist that lived in a review bundle's markdown
# and asserted nothing. Every step printed a value for a human to interpret,
# and three of them read an exit code whose cause they had guessed wrong:
#
#     printf 'PEM="line1\nline2"\n' > .env.ml
#     evnx validate --env .env.ml; echo "exit=$?"     # → exit=2
#
# The comment above it said "expect a parse error", `2` looked like agreement,
# and the review recorded "multi-line values are unsupported". The real cause
# was a missing `.env.example`; validate never parsed the file. That false fact
# became a feature request to build multi-line support, which has always worked.
#
# So the rule here is: **assert on the message, not only the code.** evnx
# deliberately separates 1 (a finding) from 2 (could not run), and a script
# verifying evnx is the last place that distinction should collapse. Claim C18
# keeps a negative control that reproduces the original trap, so the script
# proves it can tell the two causes of exit 2 apart.
#
# Three further rules the old checklist broke:
#
#   * **Never read `$?` through a pipe.** `evnx … | grep x; echo $?` reports
#     grep's status. Everything here goes through `run`, which captures output
#     and status with no pipeline.
#   * **One temp directory per claim.** The old version shared one, so a later
#     step overwrote the `.evnx.toml` an earlier step had generated.
#   * **Never build into the default target directory.** Its last step ran
#     `cargo build` — no `--all-features` — which overwrites `target/debug/evnx`,
#     the very binary under test, silently turning it into a default-features
#     build. Anyone re-running the earlier steps afterwards got different
#     answers. This script never invokes cargo; it takes binaries as input.
#
# ── Usage ───────────────────────────────────────────────────────────────────
#
#   scripts/verify-claims.sh                     # all claims
#   scripts/verify-claims.sh C18 N1              # only these
#   scripts/verify-claims.sh --list              # claim IDs and propositions
#
#   EVNX=path/to/evnx        binary under test. Default: target/debug/evnx
#                            Build it with: cargo build --all-features
#   EVNX_NOFEAT=path/to/evnx a default-features build, for claim I66. Build with
#                            `cargo build --target-dir /tmp/nofeat` — note the
#                            separate target dir, so $EVNX survives.
#   --require-all            treat SKIP as failure (for CI, where the build is
#                            known and nothing should be unrunnable)
#
# ── Exit status ─────────────────────────────────────────────────────────────
#
#   0  every claim confirmed (skips reported but tolerated)
#   1  at least one claim refuted — a documented fact has changed
#   2  the script could not run: binary missing, or --require-all with a skip
#
# That mirrors evnx's own contract on purpose. A verification script that exits
# 0 when it could not run is the bug it exists to catch.

# NOT `set -e`: evnx exits 1 and 2 by design and those are results, not errors.
set -uo pipefail

ORIG_DIR=$(pwd)
REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    C_OK=$'\033[32m'; C_BAD=$'\033[31m'; C_WARN=$'\033[33m'
    C_DIM=$'\033[2m'; C_BOLD=$'\033[1m'; C_OFF=$'\033[0m'
else
    C_OK=; C_BAD=; C_WARN=; C_DIM=; C_BOLD=; C_OFF=
fi

REQUIRE_ALL=0
WANTED=()
for arg in "$@"; do
    case "$arg" in
        --require-all) REQUIRE_ALL=1 ;;
        --list)        LIST_ONLY=1 ;;
        -h|--help)     sed -n '3,70p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*)            echo "unknown option: $arg" >&2; exit 2 ;;
        *)             WANTED+=("$arg") ;;
    esac
done

# ── Claim registry ──────────────────────────────────────────────────────────
# ID, kind, proposition. `gap` claims assert that something is MISSING, so a
# REFUTED gap is good news — the feature landed. Stating the kind in the output
# is deliberate: reading a verdict backwards is what produced the false fact
# this script was written to prevent.
CLAIM_IDS=(N1 C11 C12 F1 C18 PTB4 C19 C14 K8S C6 I66)
claim_kind() {
    case "$1" in
        I66)    echo gap ;;
        *)      echo behaviour ;;
    esac
}
claim_text() {
    case "$1" in
    N1)   echo "--format accepts both 'github' and 'github-actions' on validate and scan, but each command's error lists only its own spelling" ;;
    C11)  echo "init has a non-interactive surface: --list-components, --detect, --with, --from-source, --blueprint, --yes" ;;
    C12)  echo "spec init infers a contract from .env and writes .evnx.toml with required/secret per variable" ;;
    F1)   echo "a secret behind a build-time public prefix is high confidence, names what inlines it, and the remedy leads with renaming rather than rewriting git history" ;;
    C18)  echo "multi-line values are supported: a two-line quoted value validates clean and round-trips through convert --to json" ;;
    PTB4) echo "a Symfony-style layout works through .evnx.toml [defaults]: .env.local is the env, .env is the example" ;;
    C19)  echo "the 0/1/2 contract holds: a missing env file is 2 with a message naming that file; an unknown convert target is 2" ;;
    C14)  echo "diff --env-name X --against Y compares two environment files and names both" ;;
    K8S)  echo "convert --to kubernetes emits stringData with raw values; --base64 emits data with base64 values" ;;
    C6)   echo "cloud run exists and documents that values travel in the environment, never the argument vector" ;;
    I66)  echo "a default-features build lists gated commands in --help, but invoking one gives clap's bare error with no feature hint" ;;
    esac
}

if [ "${LIST_ONLY:-0}" = 1 ]; then
    for id in "${CLAIM_IDS[@]}"; do
        printf '%-6s %-10s %s\n' "$id" "[$(claim_kind "$id")]" "$(claim_text "$id")"
    done
    exit 0
fi

# ── Binary resolution ───────────────────────────────────────────────────────
EVNX=${EVNX:-$REPO_ROOT/target/debug/evnx}
if [ ! -x "$EVNX" ]; then
    printf '%scannot run%s: no binary at %s\n' "$C_BAD" "$C_OFF" "$EVNX" >&2
    printf 'build one with:  cargo build --all-features\n' >&2
    printf 'or point at one: EVNX=/path/to/evnx %s\n' "$0" >&2
    exit 2
fi
EVNX=$(cd "$(dirname "$EVNX")" && pwd)/$(basename "$EVNX")   # absolute: we cd per claim
NOFEAT=${EVNX_NOFEAT:-}
if [ -n "$NOFEAT" ] && [ -x "$NOFEAT" ]; then
    NOFEAT=$(cd "$(dirname "$NOFEAT")" && pwd)/$(basename "$NOFEAT")
fi

# Which gated commands this binary carries. Used to skip rather than fail: a
# default-features build legitimately cannot answer C6.
HELP_TOP=$("$EVNX" --help 2>&1)
has_cmd() { grep -qE "^[[:space:]]+$1[[:space:]]" <<<"$HELP_TOP"; }

# ── Harness ─────────────────────────────────────────────────────────────────
n_confirmed=0; n_refuted=0; n_skipped=0
REFUTED_IDS=(); SKIPPED_IDS=()
TMPDIRS=()
CLAIM_ID=; CLAIM_FAILURES=()

cleanup() {
    cd "$ORIG_DIR" || true
    for d in "${TMPDIRS[@]:-}"; do [ -n "$d" ] && rm -rf "$d"; done
}
trap cleanup EXIT

# run <cmd...> — capture output and status with no pipeline, so $RC is the
# command's own. OUT holds stdout and stderr together.
run() { OUT=$("$@" 2>&1); RC=$?; }

# Assertions. Each records a failure rather than aborting, so one claim reports
# everything wrong with it in a single pass.
note_fail() { CLAIM_FAILURES+=("$1"); }

want_rc() {
    [ "$RC" = "$1" ] || note_fail "expected exit $1, got $RC"
}
want_rc_in() {   # want_rc_in 0 1  — for commands where either is correct
    local r
    for r in "$@"; do [ "$RC" = "$r" ] && return 0; done
    note_fail "expected exit one of [$*], got $RC"
}
want_out() {
    case "$OUT" in *"$1"*) ;; *) note_fail "output missing: $1" ;; esac
}
want_not_out() {
    case "$OUT" in *"$1"*) note_fail "output should not contain: $1" ;; esac
}
# grep's own status is what we want here, so a redirect (not a pipe) is correct.
want_line() {
    grep -qE "$1" <<<"$OUT" || note_fail "no line matching /$1/"
}
want_no_line() {
    grep -qE "$1" <<<"$OUT" && note_fail "unexpected line matching /$1/"
    return 0
}
want_file() {
    [ -f "$1" ] || note_fail "expected file not written: $1"
}
want_file_has() {
    [ -f "$1" ] || { note_fail "expected file not written: $1"; return 0; }
    grep -qF -- "$2" "$1" || note_fail "$1 missing: $2"
}

wanted() {
    [ ${#WANTED[@]} -eq 0 ] && return 0
    local w
    for w in "${WANTED[@]}"; do [ "$w" = "$1" ] && return 0; done
    return 1
}

# begin <id> — fresh temp dir, reset failures. Returns 1 if the claim is filtered out.
begin() {
    wanted "$1" || return 1
    CLAIM_ID=$1
    CLAIM_FAILURES=()
    local d
    d=$(mktemp -d) || { printf '%smktemp failed%s\n' "$C_BAD" "$C_OFF" >&2; exit 2; }
    TMPDIRS+=("$d")
    cd "$d" || exit 2
    printf '\n%s%-5s%s %s[%s]%s %s\n' \
        "$C_BOLD" "$CLAIM_ID" "$C_OFF" "$C_DIM" "$(claim_kind "$CLAIM_ID")" "$C_OFF" \
        "$(claim_text "$CLAIM_ID")"
    return 0
}

# settle — print the verdict from whatever assertions recorded.
settle() {
    if [ ${#CLAIM_FAILURES[@]} -eq 0 ]; then
        printf '      %sCONFIRMED%s\n' "$C_OK" "$C_OFF"
        n_confirmed=$((n_confirmed + 1))
    else
        printf '      %sREFUTED%s\n' "$C_BAD" "$C_OFF"
        local f
        for f in "${CLAIM_FAILURES[@]}"; do printf '        · %s\n' "$f"; done
        if [ "$(claim_kind "$CLAIM_ID")" = gap ]; then
            printf '        %s(a refuted gap means the feature landed — update the fact base)%s\n' "$C_DIM" "$C_OFF"
        fi
        n_refuted=$((n_refuted + 1))
        REFUTED_IDS+=("$CLAIM_ID")
    fi
    cd "$ORIG_DIR" || exit 2
}

skip_claim() {
    printf '      %sSKIP%s  %s\n' "$C_WARN" "$C_OFF" "$1"
    n_skipped=$((n_skipped + 1))
    SKIPPED_IDS+=("$CLAIM_ID")
    cd "$ORIG_DIR" || exit 2
}

printf '%sverifying claims about%s %s\n' "$C_DIM" "$C_OFF" "$EVNX"
printf '%s%s%s\n' "$C_DIM" "$("$EVNX" --version 2>&1 | head -1)" "$C_OFF"

# ── N1 · --format spellings ─────────────────────────────────────────────────
# The original step ran `validate --help | grep -iA4 -- '--format'` and left a
# human to compare two help texts. What is actually true is narrower and more
# interesting: both spellings work on both commands, and each command's error
# advertises only one of them. The two exact-list assertions are the point — if
# either list changes, this claim must be re-read.
if begin N1; then
    printf 'A=1\n' > .env
    printf 'A=\nMISSING_VAR=\n' > .env.example

    run "$EVNX" validate --format github-actions
    want_rc 1                      # 1 = MISSING_VAR found, which is the point
    want_out '::error'
    run "$EVNX" validate --format github
    want_rc 1
    want_out '::error'             # the undocumented spelling is accepted

    run "$EVNX" scan --format github
    want_rc_in 0 1
    want_not_out 'unknown format'
    run "$EVNX" scan --format github-actions
    want_rc_in 0 1
    want_not_out 'unknown format'  # likewise, in the other direction

    run "$EVNX" validate --format bogus
    want_rc 2
    want_out 'expected one of: pretty, json, github-actions'
    run "$EVNX" scan --format bogus
    want_rc 2
    want_out 'expected one of: pretty, json, sarif, github'
    settle
fi

# ── C11 · init's non-interactive surface ────────────────────────────────────
# The original grepped five flags in one alternation, so finding any one of
# them looked like finding all five. Checked individually here.
if begin C11; then
    run "$EVNX" init --list-components
    want_rc 0
    want_out 'FRAMEWORK'
    want_out 'nextjs'

    for flag in --detect --with --from-source --blueprint --yes; do
        run "$EVNX" init --help
        want_rc 0
        want_out "$flag"
    done
    settle
fi

# ── C12 · spec init ─────────────────────────────────────────────────────────
if begin C12; then
    printf 'API_TOKEN=abc123\n' > .env
    run "$EVNX" spec init
    want_rc 0
    want_out 'wrote 1 variable'
    want_file .evnx.toml
    want_file_has .evnx.toml '[vars.API_TOKEN]'
    want_file_has .evnx.toml 'required = true'
    want_file_has .evnx.toml 'secret = true'
    settle
fi

# ── F1 · no public-prefix warning ───────────────────────────────────────────
# GAP claim. The original step also ran `validate --env-name test` and printed
# its exit code — which was 2 from a missing `.env.example`, so it said nothing
# about prefixes at all. That call is dropped rather than fixed: validate has no
# part in this claim. scan is what would carry such a warning.
#
# Verified against source when this was written: `VITE_`/`NEXT_PUBLIC_` appear
# only in help prose and a placeholder example, never in a detector.
if begin F1; then
    key="sk_test_$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 24)"
    printf 'VITE_STRIPE_SECRET_KEY=%s\n' "$key" > .env.test

    run "$EVNX" scan .env.test --no-color
    want_rc 1
    want_out 'Stripe'              # the provider is still named, so you know where to revoke
    want_out 'inlined into the client bundle'
    want_out 'Vite'                # …and what does the inlining
    want_out 'rename to STRIPE_SECRET_KEY'

    # ⚠️ The assertion that matters. `git filter-repo` may still be offered —
    # a public-prefixed key is often committed too — but it must not come
    # before the rename, because on its own it contains nothing: the value is
    # already in every bundle that shipped.
    steps=$OUT
    case "$steps" in
      *'Rename the public-prefixed'*) ;;
      *) note_fail 'the next steps must lead with renaming' ;;
    esac
    if grep -q 'filter-repo' <<<"$steps"; then
        first=$(grep -n 'Rename the public-prefixed' <<<"$steps" | head -1 | cut -d: -f1)
        hist=$(grep -n 'filter-repo' <<<"$steps" | head -1 | cut -d: -f1)
        [ -n "$first" ] && [ "$first" -lt "$hist" ] \
            || note_fail 'history rewriting is offered before the rename'
    fi
    settle
fi

# ── C18 · multi-line values ─────────────────────────────────────────────────
# The claim the old checklist got backwards, and the reason this file exists.
if begin C18; then
    printf 'PEM="line1\nline2"\n' > .env.ml
    printf 'PEM=\n' > .env.example

    # Negative control FIRST: with no example file, validate exits 2 for a
    # reason that has nothing to do with parsing. The old step stopped here and
    # read the 2 as a parse error. Asserting the message is what tells them
    # apart — and if this ever stops naming .env.example, the control has
    # rotted and the rest of the claim is no longer trustworthy.
    mkdir -p control && cp .env.ml control/
    ( cd control || exit 2
      out=$("$EVNX" validate --env .env.ml 2>&1); rc=$?
      [ "$rc" = 2 ] || { echo "CONTROL: expected exit 2, got $rc"; exit 1; }
      case "$out" in
        *'.env.example does not exist'*) exit 0 ;;
        *) echo "CONTROL: exit 2 for an unexpected reason: $(head -3 <<<"$out")"; exit 1 ;;
      esac )
    [ $? -eq 0 ] || note_fail "negative control failed: cannot distinguish the two causes of exit 2"

    # The real test: same file, example present.
    run "$EVNX" validate --env .env.ml --example .env.example
    want_rc 0
    want_out 'All checks passed'

    # And the value survives a round trip with its newline intact.
    run "$EVNX" convert --to json --env .env.ml
    want_rc 0
    want_out 'line1\nline2'        # single quotes: a literal backslash-n in the JSON
    settle
fi

# ── PTB4 · Symfony-style layout via [defaults] ──────────────────────────────
# All three commands exit 1 here and that is the CORRECT answer: .env.local
# really is missing APP_ENV. The old step ran them bare, so a reader checking
# `$?` would have concluded the layout was broken. The header is the evidence —
# it names the two files [defaults] selected.
if begin PTB4; then
    printf 'APP_ENV=dev\n' > .env
    printf 'APP_SECRET=x\n' > .env.local
    printf '[defaults]\nenv_name = "local"\nexample = ".env"\n' > .evnx.toml

    run "$EVNX" diff
    want_rc 1
    want_out '.env.local'
    want_out 'APP_ENV'

    run "$EVNX" validate
    want_rc 1
    want_out 'APP_ENV'
    settle
fi

# ── C19 · the 0/1/2 contract ────────────────────────────────────────────────
# `.env.example` is created first on purpose. Without it, `validate --env
# ./missing.env` still exits 2 — but for the example file, not the named one —
# so the original step confirmed the contract while testing something else.
if begin C19; then
    printf 'A=\n' > .env.example

    run "$EVNX" validate --env ./missing.env
    want_rc 2
    want_out './missing.env does not exist'   # names the file we asked about

    printf 'A=1\n' > .env
    run "$EVNX" convert --to bogus
    want_rc 2
    want_out 'Unknown format: bogus'
    settle
fi

# ── C14 · cross-environment diff ────────────────────────────────────────────
if begin C14; then
    printf 'A=1\nB=2\n' > .env.production
    printf 'A=1\n' > .env.staging

    run "$EVNX" diff --env-name production --against staging
    want_rc 1                      # 1 = a difference was found, as intended
    want_out '.env.production'
    want_out '.env.staging'
    want_out 'B'
    settle
fi

# ── K8S · kubernetes output ─────────────────────────────────────────────────
# Worth asserting rather than eyeballing: `stringData` holds plaintext and
# `data` holds base64. Emitting base64 under `stringData` would make Kubernetes
# decode it a second time, and `head` on the old step showed only the first
# lines — the key/value pairs are at the bottom.
if begin K8S; then
    printf 'A=1\n' > .env

    run "$EVNX" convert --to kubernetes
    want_rc 0
    want_line '^[[:space:]]*stringData:'
    want_out 'A: 1'

    run "$EVNX" convert --to kubernetes --base64
    want_rc 0
    want_line '^[[:space:]]*data:'
    want_no_line '^[[:space:]]*stringData:'
    want_out 'A: MQ=='             # base64("1")
    settle
fi

# ── C6 · cloud run surface ──────────────────────────────────────────────────
if begin C6; then
    if ! has_cmd cloud; then
        skip_claim "this binary has no 'cloud' subcommand (build with --features cloud)"
    else
        run "$EVNX" cloud run --help
        want_rc 0
        want_out 'argument vector'
        want_out '--'
        settle
    fi
fi

# ── I66 · gated commands in a default-features build ────────────────────────
# GAP claim, and a split verdict: `--help` documents the gated commands well,
# but invoking one falls through to clap's generic error with no pointer to
# that footer. Needs a second binary — which is why this script never builds
# one itself; see the header.
if begin I66; then
    if [ -z "$NOFEAT" ] || [ ! -x "$NOFEAT" ]; then
        skip_claim "set EVNX_NOFEAT to a default-features build (cargo build --target-dir /tmp/nofeat)"
    else
        run "$NOFEAT" --help
        want_rc 0
        want_out 'Not built into this binary'
        want_out 'cargo install evnx --features backup'

        run "$NOFEAT" backup
        want_rc 2
        want_out 'unrecognized subcommand'
        want_not_out '--features'   # the gap: the error never mentions the footer
        settle
    fi
fi

# ── Summary ─────────────────────────────────────────────────────────────────
printf '\n%s%s%s\n' "$C_DIM" '────────────────────────────────────────────' "$C_OFF"
printf '%s%d confirmed%s' "$C_OK" "$n_confirmed" "$C_OFF"
[ "$n_refuted" -gt 0 ] && printf '  ·  %s%d refuted%s (%s)' "$C_BAD" "$n_refuted" "$C_OFF" "${REFUTED_IDS[*]}"
[ "$n_skipped" -gt 0 ] && printf '  ·  %s%d skipped%s (%s)' "$C_WARN" "$n_skipped" "$C_OFF" "${SKIPPED_IDS[*]}"
printf '\n'

if [ "$n_confirmed" -eq 0 ] && [ "$n_refuted" -eq 0 ]; then
    printf '%sno claims ran — nothing was verified, which is not a clean result%s\n' "$C_BAD" "$C_OFF"
    exit 2
fi
if [ "$REQUIRE_ALL" = 1 ] && [ "$n_skipped" -gt 0 ]; then
    printf '%s--require-all: a skipped claim is a claim nobody checked%s\n' "$C_BAD" "$C_OFF"
    exit 2
fi
[ "$n_refuted" -gt 0 ] && exit 1
exit 0
