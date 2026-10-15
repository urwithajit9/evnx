#!/usr/bin/env bash
# In-repo copy for tests/migrate_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/migrate/tutorial/run-migrate-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test migrate_runner`, or scripts/behaviour-lock.sh migrate to compare two binaries.
# evnx migrate demo runner (M0-M12). Non-interactive. Safe: each scenario runs in its own temp folder.
# Usage: EVNX=/path/to/evnx ./run-migrate-demo.sh [M5 ...]
# "KNOWN" lines mark behaviour logged as an issue in the review; they show 0.10.0 as it is today.
# Nothing here contacts a cloud service. Printed commands are run only through STUB tools
# (fake aws, az, vercel, heroku on PATH) that print their arguments. The GitHub upload is not run.
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
  cat > .env <<'X'
APP_NAME=demo app
DATABASE_URL=postgres://admin:s3cr3t@db.internal:5432/app
API_SECRET=abc123
DEBUG=true
PORT=8080
GREETING="hello \"world\""
EMPTY=
PRICE=$5
X
}
stubs() { # fake tools that print their arguments (and stdin for vercel)
  mkdir -p bin
  for t in aws az heroku; do printf '#!/bin/sh\nprintf "stub %s args:"; for a; do printf " <%%s>" "$a"; done; echo\n' $t > bin/$t; chmod +x bin/$t; done
  printf '#!/bin/sh\nprintf "stub vercel args:"; for a; do printf " <%%s>" "$a"; done; echo; echo "  stdin: $(cat)"\n' > bin/vercel; chmod +x bin/vercel
}
cmds() { grep -E "^(aws|az|vercel|heroku|echo|  [A-Z_]+=|  --|\\$)" "$1"; }

M0() { scene M0; run "$EVNX" --version; run "$EVNX" migrate --help; run "$EVNX" migrate; expect 2 "no --to and no terminal: refuses, lists the destinations"; run "$EVNX" migrate --to bogus --dry-run; expect 2 "unknown destination"; }
M1() { scene M1; fixture; for d in "heroku --heroku-app demo-app" "aws-secrets-manager --secret-name prod/app" "gcp-secret-manager --project p" "azure-keyvault --vault-name demo-vault" "vercel --vercel-project p" "doppler --project p --doppler-config dev" "infisical --project p --infisical-env dev" "railway --railway-project p" "github-actions --repo o/r"; do run "$EVNX" migrate --to $d --dry-run; expect 0 "dry run: $d"; done; }
M2() { scene M2; fixture; for d in aws-secrets-manager azure-keyvault heroku github-actions; do run "$EVNX" migrate --to $d --dry-run; expect 2 "dry run still needs the name it would print: $d"; done; }
M3() { scene M3; fixture; run "$EVNX" migrate --to heroku --heroku-app demo-app; expect 0 "heroku: one bulk command, single-quoted"; run "$EVNX" migrate --to doppler --project p --doppler-config dev; expect 2 "KNOWN I-110: doppler needs the tool installed to PRINT commands on a real run (not on a dry run); decided: opt-in --check"; run "$EVNX" migrate --to railway --railway-project p; expect 2 "KNOWN I-110: same for railway (and infisical)"; }
M4() { scene M4; fixture; run "$EVNX" migrate --to heroku --heroku-app demo-app --include "API_*,PORT" --add-prefix PROD_; expect 0 "comma list, then prefix"; run "$EVNX" migrate --to heroku --heroku-app demo-app --include "NOPE_*"; expect 2 "a filter that matches nothing is an error (convert prints {})"; : > empty.env; run "$EVNX" migrate --source-file empty.env --to heroku --heroku-app demo-app; expect 2 "an empty source is an error"; printf 'API_X=1\nX=2\n' > col.env; run "$EVNX" migrate --source-file col.env --to heroku --heroku-app demo-app --strip-prefix API_; expect 0 "KNOWN I-104: API_X and X both become X; one value lost; message says only '1 of 2'; decided: error + --allow-collisions"; }
M5() { scene M5; fixture; run "$EVNX" migrate --to heroku --heroku-app demo-app --skip-existing; expect 2 "refused: evnx cannot see what already exists for a printing destination"; run "$EVNX" migrate --to heroku --heroku-app demo-app --overwrite; expect 2 "refused for the same reason"; }
M6() { scene M6; stubs; printf "A=it's\nB=\$5\nC=\$(echo INJECTED)\nD=two words\n" > .env; "$EVNX" migrate --to heroku --heroku-app demo-app > out.txt 2>/dev/null </dev/null; sed -n '/^heroku config:set/,/^$/p' out.txt > run.sh; run bash -c 'PATH="$PWD/bin:$PATH" sh run.sh'; expect 0 "heroku (and doppler, infisical, railway) quote correctly: every value arrives exactly as written, nothing is executed"; }
# U6-EDIT (2026-10-15): M7, M8, M9 flipped on purpose (I-101, I-094, I-102); dummy names
# `a` and `v` are now `demo-app` and `demo-vault`, which Heroku and Azure accept (A-17).
M7() { scene M7; stubs; printf 'PRICE=$5\nHOME_X=$HOME\nCMD=$(echo INJECTED)\n' > .env; for d in "vercel --vercel-project p" "azure-keyvault --vault-name demo-vault"; do "$EVNX" migrate --to $d > out.txt 2>/dev/null </dev/null; grep -E '^(echo|printf|az)' out.txt > run.sh; echo; echo "--- printed for $d:"; cat run.sh; run bash -c 'HOME=/home/demo PATH="$PWD/bin:$PATH" sh run.sh'; done; echo "FIXED I-101: every value reaches the tool exactly as written (single quotes, printf); nothing expands or runs"; }
M8() { scene M8; stubs; printf "A=it's\n" > .env; "$EVNX" migrate --to aws-secrets-manager --secret-name s > out.txt 2>/dev/null </dev/null; sed -n '/^aws secretsmanager create-secret/,/^}/p' out.txt > run.sh; echo "--- printed create-secret command:"; cat run.sh; run bash -c 'PATH="$PWD/bin:$PATH" sh run.sh 2>&1; echo "sh exit: $?"'; echo "FIXED I-094: the apostrophe is written '\\'' and the command runs"; }
M9() { scene M9; printf 'A=1\n' > .env; for pair in "heroku --heroku-app" "vercel --vercel-project" "azure-keyvault --vault-name"; do set -- $pair; run bash -c '"$0" migrate --to "$1" "$2" "x;echo_PWNED" </dev/null' "$EVNX" "$1" "$2"; expect 2 "FIXED I-102: refused, naming the flag: $1"; done; }
# U6-EDIT (2026-10-15): M10 flipped on purpose (I-103).
M10() { scene M10; run bash -c 'env -i PATH=/usr/bin:/bin HOME=/x API_TOKEN=tok-demo OTHER=1 "$(command -v "$0")" migrate --from environment --to heroku --heroku-app demo-app --dry-run' "$EVNX"; expect 2 "FIXED I-103: refused without --include"; run bash -c 'env -i PATH=/usr/bin:/bin HOME=/x API_TOKEN=tok-demo OTHER=1 "$(command -v "$0")" migrate --from environment --include OTHER --to heroku --heroku-app demo-app --dry-run' "$EVNX"; expect 0 "with --include only OTHER is sent, and the names are listed"; }
M11() { scene M11; fixture; "$EVNX" migrate --to heroku --heroku-app demo-app > run.sh 2>/dev/null </dev/null; echo "first 6 lines of run.sh:"; sed -n 1,6p run.sh; run sh -n run.sh; expect 2 "the saved file is not a script"; echo "KNOWN I-111: the banner and status lines are on stdout, so the saved file is not a script; decided: --print-only"; }
M12() { scene M12; fixture; run "$EVNX" migrate --env-name production --to heroku --heroku-app demo-app; expect 2 "KNOWN S-M1: no --env-name; the clap error points at an unrelated flag; decided: add --env-name"; run "$EVNX" migrate --to heroku --heroku-app demo-app -q; expect 0 "KNOWN I-105: -q changes nothing; decided: hides banner and next steps"; run "$EVNX" migrate --source-file nope.env --to heroku --heroku-app demo-app; expect 2 "missing file: one clear line with a hint"; }

if [ $# -eq 0 ]; then set -- M0 M1 M2 M3 M4 M5 M6 M7 M8 M9 M10 M11 M12; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
