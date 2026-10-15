#!/usr/bin/env bash
# In-repo copy for tests/diff_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/diff/tutorial/run-diff-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test diff_runner`, or scripts/behaviour-lock.sh diff to compare two binaries.
# evnx diff demo runner (D0-D11). Non-interactive. Safe: each scenario runs in its own temp folder.
# Usage: EVNX=/path/to/evnx ./run-diff-demo.sh [D5 ...]
# "KNOWN" lines mark behaviour logged as an issue in the review; they show 0.10.0 as it is today.
# All values are synthetic.
set -u
EVNX="${EVNX:-evnx}"
DEMO_ROOT="${DEMO_ROOT:-$(mktemp -d)}"
export NO_COLOR=1
pass=0; fail=0
run() { echo; echo "\$ ${*/#$EVNX/evnx}"; "$@" > "$DEMO_ROOT/.out" 2>"$DEMO_ROOT/.err" </dev/null; local rc=$?; cat "$DEMO_ROOT/.out"; [ -s "$DEMO_ROOT/.err" ] && { echo "[stderr]"; cat "$DEMO_ROOT/.err"; }; echo "[exit $rc]"; LAST=$rc; }
expect() { if [ "$LAST" -eq "$1" ]; then pass=$((pass+1)); echo "  ok: $2 (exit $1)"; else fail=$((fail+1)); echo "  MISMATCH: $2 expected exit $1, got $LAST"; fi; }
scene() { echo; echo "================ $1 ================"; D="$DEMO_ROOT/$1"; rm -rf "$D"; mkdir -p "$D"; cd "$D" || exit 1; }
fixture() {
  cat > .env.example <<'X'
DATABASE_URL=postgres://user:pass@localhost:5432/app
API_SECRET=generate-me
API_PORT=8080
REDIS_URL=redis://localhost:6379
FEATURE_X=false
X
  cat > .env <<'X'
DATABASE_URL=postgres://admin:s3cr3tPassw0rd@db.internal:5432/app
API_SECRET=real-secret-value-123456
API_PORT=9090
EXTRA_DEBUG=1
X
  printf 'DATABASE_URL=postgres://stg:pw@stg-db:5432/app\nAPI_PORT=8080\nSTAGING_ONLY=1\n' > .env.staging
  printf 'DATABASE_URL=postgres://prod:pw@prod-db:5432/app\nAPI_PORT=80\nSENTRY_DSN=https://abc@sentry.io/1\n' > .env.production
}

D0() { scene D0; run "$EVNX" --version; run "$EVNX" diff --help; run "$EVNX" diff; expect 2 "no .env: could not run, exit 2"; }
D1() { scene D1; printf 'A=1\nB=2\n' > .env.example; printf 'B=2\nA=1\n' > .env; run "$EVNX" diff; expect 0 "same keys and values in another order are identical"; }
D2() { scene D2; fixture; run "$EVNX" diff; expect 1 "any difference exits 1 (missing, extra, different values)"; }
# U3-EDIT (2026-10-15): D3 note flipped on purpose (I-076).
D3() { scene D3; fixture; run "$EVNX" diff --show-values; expect 1 "values shown; API_SECRET is masked by its name"; echo "I-076 fixed (DEC-G9): the password inside DATABASE_URL is masked, the URL keeps its structure"; }
D4() { scene D4; fixture; run "$EVNX" diff --reverse; expect 1 "sides swapped: what the template would need to learn from .env"; }
D5() { scene D5; fixture; run "$EVNX" diff --env-name production --against staging; expect 1 "two real environments"; run "$EVNX" diff --env-name production --against staging --ignore-keys SENTRY_DSN,STAGING_ONLY,DATABASE_URL,API_PORT; expect 0 "environment-specific keys ignored: identical"; run "$EVNX" diff --env-name prod; expect 2 "unknown name lists the environments, no silent fallback"; run "$EVNX" diff --env-name staging --env .env; expect 2 "--env-name conflicts with --env"; run "$EVNX" diff --against staging --example .env.production; expect 2 "--against conflicts with --example"; }
# U4-EDIT (2026-10-15): D6 expects exit 2 for an unknown --format (I-074).
D6() { scene D6; fixture; run "$EVNX" diff --format json; run "$EVNX" diff --format json --with-stats; run bash -c '"$0" diff --format json 2>/dev/null | jq -e ".summary.missing == 0" >/dev/null' "$EVNX"; expect 1 "CI gate recipe: fails because keys are missing"; run "$EVNX" diff --format xml; expect 2 "FIXED I-074: an unknown format is refused with the valid values"; echo "KNOWN I-076b: null values with values_redacted false; decided: value_state"; }
# U3-EDIT (2026-10-15): D7 note flipped on purpose (I-071); I-072 stays known.
D7() { scene D7; fixture; run "$EVNX" diff --format patch; run bash -c '"$0" diff --format patch > changes.patch 2>/dev/null; patch -p1 --dry-run < changes.patch' "$EVNX"; echo "I-071 fixed (DEC-G7): values are <withheld> unless --show-values"; echo "KNOWN I-072: patch(1) rejects the output and headings always say .env; decided: dotenv fragment with real file names"; }
# U3-EDIT (2026-10-15): D8 gains the I-071 note.
D8() { scene D8; fixture; cp .env .env.before; printf 'y\ny\ny\ny\n' > answers; run bash -c '"$0" diff --format patch --interactive < answers' "$EVNX"; cmp .env .env.before && echo "  .env is byte-identical after answering y to every prompt"; ls -A | tr '\n' ' '; echo; echo "I-071 fixed (DEC-G7): the prompts withhold values unless --show-values"; echo "KNOWN I-073: 'Applied changes' but nothing is written and no changes.patch exists; decided: write the file with .env.bak"; }
D9() { scene D9; fixture; run "$EVNX" diff --ignore-keys extra_debug; expect 1 "KNOWN I-077: ignore-keys is case-sensitive and silent about names that match nothing"; printf '[diff]\nignore_keys = ["EXTRA_DEBUG"]\n' > .evnx.toml; run "$EVNX" diff --ignore-keys REDIS_URL; expect 1 "config and flag add up (union); the config line is printed"; }
D10() { scene D10; printf 'A="x"\nB=y\nC=\nexport D=1\nE=1\nE=2\nF="multi\nline"\nG=a b\n# comment\n' > left; printf 'A=x\nB="y"\nC=1\nD=1\nE=2\nF="multi\nline"\nG="a b"\nH=\n' > right; run "$EVNX" diff --env left --example right --show-values; expect 1 "quotes, export, multiline and duplicates compare as parsed values; C is empty vs 1; H is missing"; }
D11() { scene D11; fixture; run "$EVNX" diff -q; echo "KNOWN I-079: -q changes nothing; decided: hides header and footer"; run "$EVNX" diff -v; echo "(verbose adds one stderr line today)"; rm .env; run "$EVNX" diff; expect 2 "KNOWN D-39: this is what the pre-commit hook evnx-diff does in a fresh clone"; }

if [ $# -eq 0 ]; then set -- D0 D1 D2 D3 D4 D5 D6 D7 D8 D9 D10 D11; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
