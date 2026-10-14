#!/usr/bin/env bash
# Behaviour lock: run a command's demo twice and diff, or compare two binaries.
#
#   scripts/behaviour-lock.sh <cmd> [evnx]                 run twice with one binary; must be identical
#   scripts/behaviour-lock.sh <cmd> <old-evnx> <new-evnx>  run with each; print every differing line
#
# <cmd> is one of the demos in tests/demos/ (sync, template, convert, diff, doctor,
# migrate, spec, scan, validate, init, backup) or `all`. [evnx] defaults to
# target/debug/evnx. Outputs are kept in a temp folder named at the end.
#
# `cargo test --test <cmd>_runner` compares each scene with tests/golden/demos/;
# this script is for the other half of the rule: before a change, show the run
# is repeatable; after it, list what moved so each line can be classified as
# intended (a decision id), new behaviour, or a regression.
#
# The masking matches tests/support/demo.rs closely enough for a diff between two
# runs on one machine; it is not used to produce golden files.
set -u
# Same umask as tests/support/demo.rs: scenes print file modes.
umask 022
here="$(cd "$(dirname "$0")/.." && pwd)"
cmd="${1:?usage: behaviour-lock.sh <cmd>|all [evnx] [new-evnx]}"
old="${2:-$here/target/debug/evnx}"
new="${3:-}"
out="$(mktemp -d)"

# Demos cd into each scene, so a relative binary path would fail in every run
# alike and diff as identical. Resolve it, and refuse one that cannot run.
abs() { [ -n "$1" ] || return 0; [ -x "$1" ] || { echo "not an executable: $1" >&2; exit 2; }; echo "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; }
old="$(abs "$old")" || exit 2
new="$(abs "$new")" || exit 2

mask() {
  sed -E "s#$1#<tmp>#g; s#/tmp/[A-Za-z0-9._/-]+#<tmp>#g; s/\(scratch: [^)]*\)//; \
s/[0-9]{4}-[0-9]{2}-[0-9]{2}([T ][0-9]{2}:[0-9]{2}(:[0-9]{2})?)?(\.[0-9]+)?/<date>/g; \
s/\b[0-9a-f]{64}\b/<random-hex64>/g; s/\b[0-9]{8}[_-][0-9]{6}\b/<stamp>/g"
}

run_demo() { # <cmd> <evnx> <file>
  local root; root="$(mktemp -d)"
  mkdir -p "$root/scenes" "$root/tmp" "$root/home"
  ( cd "$root" && env -i PATH="$PATH" EVNX="$2" DEMO_ROOT="$root/scenes" TMPDIR="$root/tmp" \
      HOME="$root/home" NO_COLOR=1 LC_ALL=C.UTF-8 GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null \
      GIT_AUTHOR_NAME=demo GIT_AUTHOR_EMAIL=demo@example.com \
      GIT_COMMITTER_NAME=demo GIT_COMMITTER_EMAIL=demo@example.com \
      bash "$here/tests/demos/run-$1-demo.sh" 2>&1 ) \
    | sed "s#$2#evnx#g" | mask "$root" > "$3"
  rm -rf "$root"
}

if [ "$cmd" = all ]; then
  set -- $(cd "$here/tests/demos" && ls run-*-demo.sh | sed -E 's/^run-(.*)-demo\.sh$/\1/')
else
  [ -f "$here/tests/demos/run-$cmd-demo.sh" ] || { echo "no demo for '$cmd' in tests/demos/" >&2; exit 2; }
  set -- "$cmd"
fi

status=0
for c in "$@"; do
  if [ -z "$new" ]; then
    run_demo "$c" "$old" "$out/$c.1.txt"; run_demo "$c" "$old" "$out/$c.2.txt"
    a="$out/$c.1.txt"; b="$out/$c.2.txt"; what="run 1 vs run 2"
  else
    run_demo "$c" "$old" "$out/$c.old.txt"; run_demo "$c" "$new" "$out/$c.new.txt"
    a="$out/$c.old.txt"; b="$out/$c.new.txt"; what="old vs new"
  fi
  n=$(diff "$a" "$b" | grep -c '^[<>]')
  printf '%-9s %5s lines  %3s MISMATCH  %s: %s differing lines\n' \
    "$c" "$(wc -l < "$b")" "$(grep -c MISMATCH "$b")" "$what" "$n"
  if [ "$n" -ne 0 ]; then status=1; diff "$a" "$b" | sed 's/^/    /'; fi
  # With one binary every check must hold. A binary built without the `backup`
  # or `migrate` feature fails the same way twice and would otherwise diff clean.
  if [ -z "$new" ] && grep -q MISMATCH "$b"; then
    status=1; echo "    MISMATCH lines (built with --all-features?):"; grep MISMATCH "$b" | sed 's/^/    /' | head -5
  fi
done
echo "outputs: $out"
exit $status
