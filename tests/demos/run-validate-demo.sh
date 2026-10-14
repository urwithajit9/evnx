#!/usr/bin/env bash
# In-repo copy for tests/validate_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/validate/tutorial/run-validate-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test validate_runner`, or scripts/behaviour-lock.sh validate to compare two binaries.
# evnx validate demo runner (V0-V10). Non-interactive. Safe: each scenario runs in its own temp folder.
# Usage: EVNX=/path/to/evnx ./run-validate-demo.sh [V5 ...]
# "KNOWN" lines mark behaviour logged as an issue in the review; they show 0.10.0 as it is today.
set -u
EVNX="${EVNX:-evnx}"
DEMO_ROOT="${DEMO_ROOT:-$(mktemp -d)}"
export NO_COLOR=1
pass=0; fail=0
run() { echo; echo "\$ ${*/#$EVNX/evnx}"; "$@" > "$DEMO_ROOT/.out" 2>&1 </dev/null; local rc=$?; cat "$DEMO_ROOT/.out"; echo "[exit $rc]"; LAST=$rc; }
expect() { if [ "$LAST" -eq "$1" ]; then pass=$((pass+1)); echo "  ok: $2 (exit $1)"; else fail=$((fail+1)); echo "  MISMATCH: $2 expected exit $1, got $LAST"; fi; }
scene() { echo; echo "================ $1 ================"; D="$DEMO_ROOT/$1"; rm -rf "$D"; mkdir -p "$D"; cd "$D" || exit 1; }
bad_fixture() {
  cat > .env.example <<'X'
DATABASE_URL=postgres://user:pass@localhost:5432/app
SECRET_KEY=generate-me
API_PORT=8080
ADMIN_EMAIL=admin@example.com
DEBUG=false
STRIPE_KEY=sk_test_xxx
OPTIONAL_X=
A=1
X
  cat > .env <<'X'
DATABASE_URL=not-a-url
SECRET_KEY=changeme
API_PORT=99999
ADMIN_EMAIL=nobody
DEBUG=False
EXTRA_VAR=1
A=1
A=2
X
}

V0() { scene V0; run "$EVNX" --version; run "$EVNX" validate; expect 2 "no .env.example: could not run, exit 2 (not 1)"; }
V1() { scene V1; printf 'A=1\nB=hello\n' > .env.example; printf 'A=1\nB=world\n' > .env; run "$EVNX" validate; expect 0 "clean project exits 0"; }
V2() { scene V2; bad_fixture; run "$EVNX" validate; expect 1 "findings exit 1; extras and formats are silent by default"; }
V3() { scene V3; bad_fixture; run "$EVNX" validate --strict --validate-formats; expect 1 "strict adds extras; formats adds url/port/email"; }
V4() { scene V4; bad_fixture; run "$EVNX" validate --ignore placeholder_value,weak_secret,missing_variable; expect 0 "ignored types disappear; warnings alone exit 0"; run "$EVNX" validate --exit-zero; expect 0 "--exit-zero: report but exit 0"; run "$EVNX" validate --ignore bogus_type; echo "KNOWN I-038: unknown type accepted silently; decided: exit 2 and list valid types"; }
V5() { scene V5; bad_fixture; run "$EVNX" validate --format json; run "$EVNX" validate --format github-actions; run "$EVNX" validate --format xml; expect 2 "bad format value is an error"; echo "KNOWN I-034: required_present counts any variable, not the declared ones (see JSON summary above)"; }
V6() { scene V6; bad_fixture; run "$EVNX" validate --fix; run ls -A; run cat .env; echo "(SECRET_KEY placeholder replaced; non-placeholder values such as DATABASE_URL are never touched; .env.bak kept)"; run "$EVNX" validate --rotate-weak-secrets; expect 2 "--rotate-weak-secrets requires --fix"; echo "Gap S1: no --fix --dry-run yet (decided)"; }
V7() { scene V7; printf 'A=\nB=\n' > .env.example; printf 'A=1\nB=2\n' > .env.staging; printf 'A=1\nB=2\n' > .env.production; run "$EVNX" validate --env-name staging; expect 0 "resolves .env.staging"; run "$EVNX" validate --env-name prod; expect 2 "no silent fallback; lists existing names"; run "$EVNX" validate --env-name staging --env .env.production; expect 2 "--env-name conflicts with --env"; }
V8() { scene V8; printf 'A=1\nB=2\nPORT=80\n' > .env.example; printf 'A=1\nPORT=abc\n' > .env
  cat > .evnx.toml <<'X'
[vars.B]
required = false
[vars.PORT]
format = "port"
[vars.A]
environments = ["prodution"]
X
  run "$EVNX" validate; echo "B is optional (required=false) so no missing error; PORT format_mismatch expected"; echo "KNOWN I-042: typo 'prodution' in environments is silently accepted; decided: warn"; }
V9() { scene V9; printf 'DB_HOST=db\n' > .env.example; printf 'DB_HOST=localhost\n' > .env; run "$EVNX" validate; echo "(no Docker file: localhost is fine)"; printf 'FROM alpine\n' > Dockerfile; run "$EVNX" validate; echo "(Dockerfile in cwd: localhost_in_docker warning)"; mkdir -p sub; cp .env .env.example sub/; cd sub; run "$EVNX" validate; echo "KNOWN I-046: run from sub/ the project's Dockerfile one level up is not seen, so no warning; decided: env file folder, then project root"; }
# U1-EDIT (2026-10-14): V10 reader is `true` after 0.3 s instead of `head -c 10`: with head the result depended on timing (T-068); I-028 was fixed in 0.10.1.
V10() { scene V10; git init -q .; printf 'A=1\n' > .env.example; printf 'A=1\n' > .env; run "$EVNX" validate; expect 0 "KNOWN D-27: .env is not in .gitignore and validate says nothing (docs claim a check)"; bad_fixture; run bash -c '{ sleep 0.3; "$0" validate; } | true; exit ${PIPESTATUS[0]}' "$EVNX"; expect 141 "I-028 fixed in 0.10.1: a closed pipe ends validate quietly, exit 141"; }

if [ $# -eq 0 ]; then set -- V0 V1 V2 V3 V4 V5 V6 V7 V8 V9 V10; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
