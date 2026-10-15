#!/usr/bin/env bash
# In-repo copy for tests/sync_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/sync/tutorial/run-sync-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test sync_runner`, or scripts/behaviour-lock.sh sync to compare two binaries.
# evnx sync demo runner (S0-S13). Non-interactive (no terminal), safe: each scene runs in its own temp folder.
# Usage: EVNX=/path/to/evnx ./run-sync-demo.sh [S5 ...]
# "KNOWN" lines mark behaviour logged as an issue in the review; they show 0.10.0 as it is today.
# Nothing here touches the network. All values are synthetic. Interactive prompts are NOT run here (see the runbook).
set -u
EVNX="${EVNX:-evnx}"
DEMO_ROOT="${DEMO_ROOT:-$(mktemp -d)}"
export NO_COLOR=1
pass=0; fail=0
run() { echo; echo "\$ ${*/#$EVNX/evnx}"; "$@" > "$DEMO_ROOT/.out" 2>"$DEMO_ROOT/.err" </dev/null; local rc=$?; cat "$DEMO_ROOT/.out"; [ -s "$DEMO_ROOT/.err" ] && { echo "[stderr]"; cat "$DEMO_ROOT/.err"; }; echo "[exit $rc]"; LAST=$rc; }
expect() { if [ "$LAST" -eq "$1" ]; then pass=$((pass+1)); echo "  ok: $2 (exit $1)"; else fail=$((fail+1)); echo "  MISMATCH: $2 expected exit $1, got $LAST"; fi; }
scene() { echo; echo "================ $1 ================"; D="$DEMO_ROOT/$1"; rm -rf "$D"; mkdir -p "$D"; cd "$D" || exit 1; }
fixture() {
  cat > .env <<'X'
DATABASE_URL=postgres://admin:s3cr3t@db.internal:5432/app
API_SECRET=abc123def456
API_PORT=9090
NEW_FEATURE=true
stripe_key=sk_live_abcdefghijklmnopqrstuvwx
X
  cat > .env.example <<'X'
DATABASE_URL=postgres://user:pass@localhost:5432/app
API_SECRET=
API_PORT=8080
REDIS_URL=redis://localhost:6379
X
  chmod 644 .env .env.example
}
show() { echo "--- $1:"; cat "$1"; }

S0() { scene S0; fixture; run "$EVNX" --version; run "$EVNX" sync; expect 2 "drift but no terminal: 'not a terminal' error (KNOWN I-124; decided: auto-placeholder)"; run "$EVNX" sync --env nope.env; expect 2 "missing .env: exit 2 (KNOWN I-125: message names .env, not nope.env)"; }
S1() { scene S1; fixture; mv .env.example old.example; run "$EVNX" sync -f; expect 0 "no template: created with placeholders (no terminal / -f)"; show .env.example; run "$EVNX" sync --check; expect 0 "second run: in sync"; run "$EVNX" sync -f; expect 0 "idempotent: nothing to add"; }
S2() { scene S2; fixture; run "$EVNX" sync -n; expect 0 "-n previews, always exit 0"; run "$EVNX" sync --check; expect 1 "--check: out of sync"; run "$EVNX" sync -f; expect 0 "-f appends placeholders"; show .env.example; run "$EVNX" sync --check; expect 0 "now in sync"; }
S3() { scene S3; fixture; run "$EVNX" sync --check --format json; expect 1 "json report: missing keys, sorted"; run "$EVNX" sync --check --format json --direction reverse; expect 1 "reverse: keys the .env lacks"; run "$EVNX" sync --format json; expect 2 "json needs --check (clap error)"; printf 'A=1\n' > t.env; printf 'A=1\nONLY_IN_TEMPLATE=x\n' > t.example; run "$EVNX" sync --check --env t.env --example t.example; expect 0 "KNOWN I-119: a key only in the template is never reported; decided: two-way, exit 1"; }
S4() { scene S4; fixture; run "$EVNX" sync -f --direction reverse; expect 0 "reverse adds template-only keys"; show .env; echo "KNOWN I-115: REDIS_URL became redis://localhost:6379/0, not the template's own value; decided: copy the template value"; rm .env; run "$EVNX" sync -f --direction reverse; expect 0 "no .env: template copied as it is"; show .env; echo "KNOWN I-129: a template header such as '# Generated from .env' would be copied into the new .env"; }
S5() { scene S5; printf 'SUPPORT_EMAIL=help@acme.io\nMONKEY_COUNT=3\nBYPASS_CACHE=1\nGHOST_MODE=on\nDEBUG=false\nDB_URL=postgres://u:p@h/db\nMONGO_URL=mongodb://u:p@h/db\nAPI_PORT=80\n' > .env; run "$EVNX" sync -f; expect 0 "template created from .env"; show .env.example; echo "KNOWN I-120: SUPPORT_EMAIL=8000, MONKEY_COUNT=YOUR_KEY_HERE, BYPASS_CACHE=YOUR_PASSWORD_HERE, GHOST_MODE=localhost, DEBUG=true, postgres:// and mongodb:// = YOUR_URL_HERE; decided: whole-token matching, scheme list"; }
S6() { scene S6; fixture; echo '{"patterns":{"^STRIPE_":"<stripe>","_KEY$":"<k>"},"default":"<set-me>","allow_actual":["API_PORT","NEW_FEATURE"]}' > tc.json; run "$EVNX" sync -n --template-config tc.json; expect 0 "custom patterns, default and allow_actual in the preview"; echo '{"allow_actual":["DATABASE_URL","API_SECRET"]}' > tc2.json; run "$EVNX" sync -n --template-config tc2.json; expect 2 "allow_actual with credentials: whole run refused, every key named"; echo '{"patterns":{"(":"x"}}' > tc3.json; run "$EVNX" sync -n --template-config tc3.json; expect 2 "bad regex refused at load"; echo '{"allow_actaul":["NEW_FEATURE"]}' > tc4.json; run "$EVNX" sync -n --template-config tc4.json; expect 0 "KNOWN I-121: misspelt key accepted and ignored (NEW_FEATURE stays YOUR_VALUE_HERE); decided: error, exit 2"; printf '[sync]\nnaming_policy = "bogus"\n' > .evnx.toml; run "$EVNX" sync -n; expect 0 "KNOWN I-121: invalid [sync] naming_policy ignored; decided: error"; }
S7() { scene S7; fixture; run "$EVNX" sync -n; expect 0 "default warn: stripe_key warned, still synced"; run "$EVNX" sync -n --naming-policy error; expect 1 "policy error in plain mode (decided: exit 2)"; run "$EVNX" sync --check --naming-policy error; expect 1 "KNOWN I-116: under --check this is exit 1, which reads as 'out of sync'; decided: exit 2"; run "$EVNX" sync -n --naming-policy ignore; expect 0 "ignore: silent"; }
S8() { scene S8; printf 'APP_NAME="My App # prod"\nLOG_LEVEL=info\n' > .env; echo '{"allow_actual":["APP_NAME","LOG_LEVEL"]}' > tc.json; run "$EVNX" sync -f --template-config tc.json; expect 0 "allow_actual writes the real values"; show .env.example; run bash -c '"$0" convert --to json --env .env.example | grep APP_NAME' "$EVNX"; expect 0 "KNOWN I-114: written unquoted, re-read as 'My App'; decided: shared writer quotes"; }
# U2-EDIT (2026-10-15): S9 notes flipped on purpose (I-122, I-123).
S9() { scene S9; fixture; mkdir shared; printf 'KEEP=1\n' > shared/real.example; rm .env.example; ln -s shared/real.example .env.example; run "$EVNX" sync -f; expect 0 "template is a symlink"; run bash -c 'test -L .env.example && echo "still a symlink" || echo "REPLACED by a regular file"; cat shared/real.example'; echo "I-122 fixed (DEC-S6): the link stays and its target is written"; rm -f .env.example; fixture; run "$EVNX" sync -f; run bash -c 'stat -c "%n mode %a (was 644)" .env.example .env'; echo "I-123 fixed (DEC-S7): existing modes are kept; a new template is 644, a new .env 600"; }
S10() { scene S10; printf 'A=1\nAPI_KEY=zzz\n' > .env.example; run "$EVNX" sync --direction forward --placeholder; expect 2 "the shipped pre-commit hook on a fresh clone (no .env)"; printf 'A=1\nAPI_KEY=zzz\nB=2\n' > .env; run "$EVNX" sync --direction forward --placeholder; expect 2 "the shipped hook with drift and no terminal"; echo "KNOWN I-128: both cases block the commit; decided: hook runs 'evnx sync --check' (works without .env via spec) plus optional 'sync --force' hook"; }
S11() { scene S11; fixture; : > empty.env; run "$EVNX" sync --check --env empty.env; expect 0 "KNOWN I-127: empty source, full template: warning but exit 0; decided: exit 2 under --check"; run "$EVNX" sync --check --direction reverse --env nope.env; expect 1 "reverse, .env missing, pretty: out of sync (it would create it)"; run "$EVNX" sync --check --format json --direction reverse --env nope.env; expect 2 "KNOWN I-117: same inputs, json says exit 2; decided: one drift computation"; }
S12() { scene S12; fixture; cp .env .env.production; run "$EVNX" sync --env-name production --check; expect 1 "--env-name picks .env.production"; run "$EVNX" sync --env-name typo --check; expect 2 "unknown environment: error listing the existing ones"; run "$EVNX" sync --env-name production -f; expect 0 "header names the files in play"; run bash -c 'tail -n 3 .env.example'; }
S13() { scene S13; fixture; run "$EVNX" sync -d reverse -n; expect 2 "D-73: the docs list -d; the binary rejects it (decided: fix the docs)"; run "$EVNX" sync -p -n; expect 2 "D-73: the docs list -p; rejected"; run env CI=1 "$EVNX" sync; expect 2 "D-74: the docs say CI=1 acts like --force; it does not (decided: honour CI=1)"; }

if [ $# -eq 0 ]; then set -- S0 S1 S2 S3 S4 S5 S6 S7 S8 S9 S10 S11 S12 S13; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
