#!/usr/bin/env bash
# In-repo copy for tests/scan_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/scan/tutorial/run-scan-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test scan_runner`, or scripts/behaviour-lock.sh scan to compare two binaries.
# evnx scan demo runner (S0-S11). Non-interactive. Safe: each scenario runs in its own temp folder.
# Usage: EVNX=/path/to/evnx ./run-scan-demo.sh [S5 ...]
# "KNOWN" lines mark behaviour logged as an issue in the review; they show 0.10.0 as it is today.
# All secrets below are synthetic. None is a working credential.
set -u
EVNX="${EVNX:-evnx}"
DEMO_ROOT="${DEMO_ROOT:-$(mktemp -d)}"
export NO_COLOR=1
pass=0; fail=0
run() { echo; echo "\$ ${*/#$EVNX/evnx}"; "$@" > "$DEMO_ROOT/.out" 2>&1 </dev/null; local rc=$?; cat "$DEMO_ROOT/.out"; echo "[exit $rc]"; LAST=$rc; }
expect() { if [ "$LAST" -eq "$1" ]; then pass=$((pass+1)); echo "  ok: $2 (exit $1)"; else fail=$((fail+1)); echo "  MISMATCH: $2 expected exit $1, got $LAST"; fi; }
scene() { echo; echo "================ $1 ================"; D="$DEMO_ROOT/$1"; rm -rf "$D"; mkdir -p "$D"; cd "$D" || exit 1; }
leaky_env() {
  cat > .env <<'X'
AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY
STRIPE_SECRET_KEY=sk_live_51H1234567890abcdefghijk
GITHUB_TOKEN=ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8
DATABASE_URL=postgres://admin:s3cr3tPassw0rd@db.internal:5432/app
APP_NAME=demo
DEBUG=true
X
}

S0() { scene S0; run "$EVNX" --version; run "$EVNX" scan --help; run "$EVNX" scan .; expect 2 "empty folder: nothing was scanned is exit 2, not a clean pass"; }
S1() { scene S1; printf 'APP_NAME=demo\nDEBUG=true\nPORT=8080\n' > .env; run "$EVNX" scan .; expect 0 "clean project exits 0"; }
S2() { scene S2; leaky_env; run "$EVNX" scan .; expect 1 "findings exit 1"; }
S3() { scene S3; leaky_env; run "$EVNX" scan . --severity high; expect 1 "only high findings"; run "$EVNX" scan . --severity all; expect 1 "all = low and above"; run "$EVNX" scan . --severity bogus; expect 2 "bad severity is a usage error"; }
# U0-EDIT (2026-10-14): S4 reader is `true` after 0.3 s instead of `head -c 10`; with head, the order of
# stdout and the Docs line on stderr depended on timing (T-070). I-063 was fixed in 0.10.1.
S4() { scene S4; leaky_env; run "$EVNX" scan . --format json; run "$EVNX" scan . --format github; run "$EVNX" scan . --format sarif --exit-zero; run bash -c '{ sleep 0.3; "$0" scan . --format sarif; } | true; exit ${PIPESTATUS[0]}' "$EVNX"; expect 141 "I-063 fixed in 0.10.1: a closed pipe ends scan quietly, exit 141"; }
# R-SCAN-EDIT (2026-10-21): S5 flipped — I-050 fixed; sk_live_xxxx is a stand-in, not a finding.
S5() { scene S5; leaky_env; printf 'STRIPE_KEY=sk_live_xxxxxxxxxxxxxxxxxxxxxxxx\nAPI_KEY=your-api-key-here\n' > .env.example; run "$EVNX" scan .; echo ".env.example is excluded by default"; run "$EVNX" scan . --exclude nothing-matches; expect 1 "custom --exclude replaces the default"; run "$EVNX" scan .env.example; expect 0 "named file is scanned (1 file scanned); FIXED I-050: sk_live_xxxx is a placeholder, not a finding"; run "$EVNX" scan .env.example --ignore-placeholders; echo "FIXED I-050 (R-scan): a run of x after sk_live_ is a stand-in with or without --ignore-placeholders"; }
S6() { scene S6; printf 'VENDOR_REF=acme_live_9f8e7d6c5b4a39281706\nNAME=x\n' > .env; run "$EVNX" scan .; expect 0 "value of an unknown provider under a neutral name is not detected (name heuristic cannot help)"; run "$EVNX" scan . --pattern 'acme_live_[0-9a-f]{20}'; expect 1 "--pattern adds a custom regex (Custom pattern 1)"; run "$EVNX" scan . --pattern '(['; expect 2 "bad regex is an error"; }
# R-SCAN-EDIT (2026-10-21): S7 I-060 and S10 I-049, I-058 flipped.
S7() { scene S7; leaky_env; cat > .evnx.toml <<'X'
[scan]
severity = "high"
exclude = ["vendor"]
ignore_placeholders = true
disable = ["Stripe Secret Key (LIVE)"]
[vars]
STRIPE_SECRET_KEY = { secret = false }
X
  run "$EVNX" scan .; expect 1 "config merges: weakening is announced on screen"; run "$EVNX" scan . -q; echo "FIXED I-060/I-049 (R-scan): -q is findings only, and keeps the line saying the config weakens the scan"; run "$EVNX" scan . --severity low; echo "flag beats config"; }
# R-SCAN-EDIT (2026-10-21): S8 I-054 flipped — `scan . .` reads each file once.
# U8-EDIT (2026-10-16): S8 flipped — I-047 fixed; the walk reads every file (DEC-U8-2).
S8() { scene S8; mkdir -p config src; cp /dev/null config/id_rsa.pem; printf -- '-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu\n-----END RSA PRIVATE KEY-----\n' > config/id_rsa.pem; printf 'k = "AKIA4OZRMFJ3VREALKEY"\n' > src/main.py; run "$EVNX" scan .; expect 1 "walk finds src/main.py and config/id_rsa.pem"; echo "FIXED I-047 (U8b, DEC-U8-2): the walk reads every file, so config/id_rsa.pem is listed above"; run "$EVNX" scan config/id_rsa.pem; expect 1 "named file is scanned"; run "$EVNX" scan . .; echo "FIXED I-054 (R-scan): the same path twice is scanned once"; }
S9() { scene S9; run "$EVNX" scan does-not-exist; expect 2 "missing path is exit 2"; printf 'AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n' | iconv -t UTF-16LE > utf16.env; run "$EVNX" scan utf16.env; expect 1 "UTF-16 file is decoded and scanned"; printf 'A=\377\376\001binary\n' > bad.env; run "$EVNX" scan bad.env; echo "(unreadable or odd input fails closed; see exit above)"; }
S10() { scene S10; leaky_env; run "$EVNX" scan . --exit-zero; expect 0 "--exit-zero reports but exits 0"; run "$EVNX" scan . --no-color -q; echo "FIXED I-049 (0.10.3 + R-scan): -q drops the banner and the next steps; findings only"; run "$EVNX" scan . -v; echo "FIXED I-058 (R-scan): -v counts the 13 built-in rules and the custom patterns apart"; }
S11() { scene S11; cat > .env <<'X'
NEXT_PUBLIC_STRIPE_SECRET=sk_live_51H1234567890abcdefghijk
# OLD_KEY=AKIA4OZRMFJ3VREALKEY
SIGNING_KEY=abc
SIGNING_KEY=
X
  run "$EVNX" scan .; expect 1 "client-bundle prefix escalation; a commented-out key is still a leaked key"; }

# S12 (R-scan, 2026-10-21): DEC-SC1 / I-285, I-286. In a git repository a walk counts what git would
# commit; a gitignored, untracked .env is summarised in one line. Named, --include-ignored or committed: reported.
S12() { scene S12; git init -q .; printf '.env*\n!.env.example\n' > .gitignore; leaky_env; cp .env .env.bak; printf 'APP_NAME=\n' > .env.example; run "$EVNX" scan; expect 0 "FIXED I-285: a gitignored, untracked .env is summarised, not counted"; run "$EVNX" scan .env; expect 1 "a named file is reported in full; I-286: nothing was committed, so no history advice"; run "$EVNX" scan --include-ignored --severity high; expect 1 "--include-ignored counts them"; git add -f .env && git -c user.name=demo -c user.email=demo@example.com commit -qm env; run "$EVNX" scan --severity high; expect 1 "a committed .env is reported with history advice naming it"; }
if [ $# -eq 0 ]; then set -- S0 S1 S2 S3 S4 S5 S6 S7 S8 S9 S10 S11 S12; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
