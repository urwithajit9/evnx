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
# R-VALIDATE-EDIT (2026-10-18): check() added (same as the spec demo's), for the flipped KNOWN lines below.
check() { if eval "$1"; then pass=$((pass+1)); echo "  ok: $2"; else fail=$((fail+1)); echo "  MISMATCH: $2"; fi; }
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
# R-VALIDATE-EDIT (2026-10-18): V2 check added — I-035 (one finding per cause).
V2() { scene V2; bad_fixture; run "$EVNX" validate; expect 1 "findings exit 1; extras and formats are silent by default"; check '[ "$(grep -c "SECRET_KEY" "$DEMO_ROOT/.out")" -eq 1 ]' "FIXED I-035: SECRET_KEY=changeme is one finding (the placeholder), not also 'too weak'"; }
V3() { scene V3; bad_fixture; run "$EVNX" validate --strict --validate-formats; expect 1 "strict adds extras; formats adds url/port/email"; }
# R-VALIDATE-EDIT (2026-10-18): V4 flipped — I-038 (exit 2, the valid types listed); S2 --fail-on added.
V4() { scene V4; bad_fixture; run "$EVNX" validate --ignore placeholder_value,weak_secret,missing_variable; expect 0 "ignored types disappear; warnings alone exit 0"; run "$EVNX" validate --ignore placeholder_value,weak_secret,missing_variable --fail-on warning; expect 1 "NEW S2: --fail-on warning makes the warnings fail the run"; run "$EVNX" validate --exit-zero; expect 0 "--exit-zero: report but exit 0"; run "$EVNX" validate --ignore bogus_type; expect 2 "FIXED I-038: an unknown type is refused and the valid types are listed"; }
# R-VALIDATE-EDIT (2026-10-18): V5 flipped — I-034 (declared names present: 6 of 8).
V5() { scene V5; bad_fixture; run "$EVNX" validate --format json; check 'grep -q "\"required_present\": 6," "$DEMO_ROOT/.out" && grep -q "\"required_total\": 8," "$DEMO_ROOT/.out"' "FIXED I-034: required_present counts the declared variables that are there (6 of 8)"; run "$EVNX" validate --format github-actions; run "$EVNX" validate --format xml; expect 2 "bad format value is an error"; }
# R-VALIDATE-EDIT (2026-10-18): V6 flipped — I-037 (fixed vs needs your input), S1 (--fix --dry-run), I-260 (no invented provider key).
V6() { scene V6; bad_fixture; run "$EVNX" validate --fix; check 'grep -q "^Fixed (2):" "$DEMO_ROOT/.out" && grep -q "^Needs your input (2):" "$DEMO_ROOT/.out" && ! grep -q "STRIPE_KEY looks like a placeholder" "$DEMO_ROOT/.out"' "FIXED I-037: the placeholders --fix had to add are listed under 'Needs your input', not counted as fixed or repeated as errors"; run ls -A; run cat .env; echo "(SECRET_KEY placeholder replaced; non-placeholder values such as DATABASE_URL are never touched; .env.bak kept)"; run "$EVNX" validate --rotate-weak-secrets; expect 2 "--rotate-weak-secrets requires --fix"; bad_fixture; cp .env before.env; run "$EVNX" validate --fix --dry-run; expect 1 "FIXED S1: --fix --dry-run shows what it would do; the verdict is the file as it stands"; check 'cmp -s .env before.env && [ ! -e .env.bak.2 ]' "FIXED S1: nothing written, no second backup"; printf 'STRIPE_KEY=\n' > .env.example; printf 'STRIPE_KEY=changeme\n' > .env; run "$EVNX" validate --fix; expect 1 "FIXED I-260 (DEC-VA4): --fix never invents a key a provider issues; the placeholder is still reported"; check 'grep -qx "STRIPE_KEY=changeme" .env' "FIXED I-260: STRIPE_KEY is left as it was"; }
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
  # R-VALIDATE-EDIT (2026-10-18): V8 flipped — I-042 (a warning, exit unchanged).
  run "$EVNX" validate; echo "B is optional (required=false) so no missing error; PORT format_mismatch expected"; check 'grep -q "did you mean '"'"'production'"'"'" "$DEMO_ROOT/.out"' "FIXED I-042: typo 'prodution' in environments is a warning naming 'production'"; }
# R-VALIDATE-EDIT (2026-10-18): V9 flipped — I-046 (the env file's folder, then each folder above it).
V9() { scene V9; printf 'DB_HOST=db\n' > .env.example; printf 'DB_HOST=localhost\n' > .env; run "$EVNX" validate; echo "(no Docker file: localhost is fine)"; printf 'FROM alpine\n' > Dockerfile; run "$EVNX" validate; echo "(Dockerfile in cwd: localhost_in_docker warning)"; mkdir -p sub; cp .env .env.example sub/; cd sub; run "$EVNX" validate; check 'grep -q "DB_HOST uses localhost" "$DEMO_ROOT/.out"' "FIXED I-046: run from sub/, the project's Dockerfile one level up is seen"; }
# U1-EDIT (2026-10-14): V10 reader is `true` after 0.3 s instead of `head -c 10`: with head the result depended on timing (T-068); I-028 was fixed in 0.10.1.
# R-VALIDATE-EDIT (2026-10-18): V10 flipped — D-27 (DEC-VA1: a warning, exit 0).
V10() { scene V10; git init -q .; printf 'A=1\n' > .env.example; printf 'A=1\n' > .env; run "$EVNX" validate; expect 0 "FIXED D-27 (DEC-VA1): .env not covered by .gitignore is a warning; warnings alone exit 0"; check 'grep -q "not covered by .gitignore" "$DEMO_ROOT/.out"' "FIXED D-27: validate says so"; bad_fixture; run bash -c '{ sleep 0.3; "$0" validate; } | true; exit ${PIPESTATUS[0]}' "$EVNX"; expect 141 "I-028 fixed in 0.10.1: a closed pipe ends validate quietly, exit 141"; }

if [ $# -eq 0 ]; then set -- V0 V1 V2 V3 V4 V5 V6 V7 V8 V9 V10; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
