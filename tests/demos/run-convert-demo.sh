#!/usr/bin/env bash
# In-repo copy for tests/convert_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/convert/tutorial/run-convert-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test convert_runner`, or scripts/behaviour-lock.sh convert to compare two binaries.
# evnx convert demo runner (C0-C11). Non-interactive. Safe: each scenario runs in its own temp folder.
# Usage: EVNX=/path/to/evnx ./run-convert-demo.sh [C5 ...]
# "KNOWN" lines mark behaviour logged as an issue in the review; they show 0.10.0 as it is today.
# Nothing here calls a cloud tool: the pipe scenario uses a stub `aws` that only prints its arguments.
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
  printf 'DATABASE_URL=postgres://prod:pw@prod-db:5432/app\nPORT=80\n' > .env.production
}

C0() { scene C0; run "$EVNX" --version; run "$EVNX" convert --help; run "$EVNX" convert --to json; expect 2 "no .env: could not run"; echo "KNOWN I-082: --help carries developer commit notes ('clap strips the markdown fence...'); decided: removed"; }
C1() { scene C1; fixture; run "$EVNX" convert --to json; expect 0 "json"; run "$EVNX" convert --to yaml; expect 0 "yaml"; run "$EVNX" convert --to shell; expect 0 "shell: single-quoted, runs nothing"; run "$EVNX" convert --to docker-compose; expect 0 "compose environment block"; }
C2() { scene C2; fixture; for a in k8s tf yml gh-actions compose; do run "$EVNX" convert --to $a | head -3 >/dev/null; done; run "$EVNX" convert --to k8s; expect 0 "alias k8s = kubernetes"; run "$EVNX" convert --to JSON; expect 0 "case does not matter"; run "$EVNX" convert --to bogus; expect 2 "unknown format lists the valid ones"; }
C3() { scene C3; fixture; run "$EVNX" convert --to kubernetes; expect 0 "stringData: literal values, Kubernetes encodes on apply"; run "$EVNX" convert --to kubernetes --base64; expect 0 "data: values are base64"; echo "KNOWN I-084: Secret name is always app-secrets; decided: --name/--namespace, required"; }
# U2-EDIT (2026-10-14): C4 flipped on purpose: new output 600, existing output needs --force (I-081, DEC-G5).
C4() { scene C4; fixture; run "$EVNX" convert --to json -o out.json; expect 0 "writes the file"; echo "mode of out.json: $(stat -c %a out.json)  (I-081 fixed: a new output is 600)"; run "$EVNX" convert --to yaml -o out.json; expect 2 "I-081 fixed (DEC-G5): an existing output is refused without --force"; run "$EVNX" convert --to yaml -o out.json --force; expect 0 "--force replaces it and says Replaced"; head -2 out.json; run "$EVNX" convert --to json -o nodir/out.json; expect 2 "missing directory is an error, no mkdir"; }
C5() { scene C5; fixture; run "$EVNX" convert --to json --include "P*"; expect 0 "prefix glob: PORT, PRICE"; run "$EVNX" convert --to json --include "*_URL" --exclude "DATABASE_*"; expect 0 "exclude applies after include: nothing left"; run "$EVNX" convert --to json --include "API_*" --prefix "APP_" --transform lowercase; expect 0 "prefix before casing: app_api_secret"; run "$EVNX" convert --to json --include "NOPE_*"; expect 0 "KNOWN I-089: zero match prints {} and exits 0; decided: exit 1, --allow-empty"; run "$EVNX" convert --to json --include "PORT,DEBUG"; echo "KNOWN I-090: comma list matches nothing; decided: repeatable --include"; run "$EVNX" convert --to json --include "P?RT"; echo "KNOWN I-090: ? is literal today; decided: core::glob"; : > empty; run "$EVNX" convert --env empty --to json; expect 0 "KNOWN I-089: empty .env prints {} and exits 0"; }
# U8-EDIT (2026-10-16): C6 flipped on purpose (I-088, DEC-U8-3).
C6() { scene C6; printf 'Api=1\nAPI=2\n' > .env; run "$EVNX" convert --to json --transform uppercase; expect 2 "FIXED I-088: Api and API collide; refused, both named, nothing written (DEC-U8-3)"; printf 'DB_URL=1\ndb_url=2\n' > .env; run "$EVNX" convert --to json --transform camelCase; expect 2 "FIXED I-088: both become dbUrl; refused, both named (a prefix alone cannot collide: it is applied to every key)"; fixture; run "$EVNX" convert --to json --transform Title; expect 0 "KNOWN I-090: unknown mode ignored; decided: exit 2 listing modes"; for t in uppercase lowercase camelCase snake_case; do run "$EVNX" convert --to json --include "DATABASE_URL" --transform $t; done; }
C7() { scene C7; fixture; run "$EVNX" convert --to gcp-secrets; echo "KNOWN I-084/I-099: fixed project my-project; gcloud secrets create fails for an existing secret; decided: --project required, create-or-add-version"; run "$EVNX" convert --to azure-keyvault; echo "KNOWN I-084/I-086: fixed vault; secret on the command line (visible in ps while it runs); decided: stdin/file if az supports it"; run "$EVNX" convert --to heroku; echo "KNOWN I-084: --app YOUR_APP_NAME placeholder"; }
C8() { scene C8; fixture; run "$EVNX" convert --to github-actions; echo "KNOWN I-085: prose for a web form; decided: keep, label clearly"; run "$EVNX" convert --to vercel; echo "KNOWN I-092: type is always plain; decided: classifier, after verification V-03"; run "$EVNX" convert --to terraform; echo "KNOWN I-093: names lowercased; decided: --transform decides, documented"; run bash -c '"$0" convert --to aws-secrets > a.json; "$0" convert --to json > j.json; cmp a.json j.json && echo "aws-secrets is byte-identical to json"; "$0" convert --to railway > r.json; cmp r.json j.json && echo "railway is byte-identical to json"' "$EVNX"; echo "KNOWN I-087: decided: keep, fix menu labels, railway experimental"; }
C9() { scene C9; fixture
  mkdir -p bin; printf '#!/bin/sh\nprintf "stub aws got:\\n"; for a; do printf "  ARG<%%s>\\n" "$a"; done; echo "  stdin:"; cat /dev/stdin | head -3\n' > bin/aws; chmod +x bin/aws
  run bash -c 'PATH="$PWD/bin:$PATH"; "$0" convert --to aws-secrets 2>/dev/null | aws secretsmanager create-secret --name prod/myapp/config --secret-string file:///dev/stdin' "$EVNX"; expect 0 "pipe mechanics only: stdout is the JSON and nothing else (a stub, NOT the real aws: acceptance by aws is unverified V-01)"
  for f in json yaml shell kubernetes terraform docker-compose gcp-secrets; do n=$("$EVNX" convert --to $f 2>/dev/null | grep -c 'Docs:'); echo "  $f: lines with the Docs footer on stdout = $n"; done
  run bash -c '"$0" convert --to json | head -1' "$EVNX"; expect 0 "broken pipe: no panic message (exit 101 is the failure; I-063 class)"; }
C10() { scene C10; fixture; run "$EVNX" convert --env-name production --to json; expect 0 "named environment"; run "$EVNX" convert --env-name prod --to json; expect 2 "unknown name lists the environments"; run "$EVNX" convert --env-name production --env .env --to json; expect 2 "--env-name conflicts with --env"; run "$EVNX" convert --env nope.env --to json; expect 2 "KNOWN I-091: missing file error is nested ('Convert command failed: Failed to validate input file: ...'); decided: one line"; }
C11() { scene C11; printf 'PEM="-----BEGIN KEY-----\nabc\n-----END KEY-----"\nPLAIN=ok\n' > .env; run "$EVNX" convert --to json; expect 0 "multi-line value as \\n in JSON"; run bash -c '"$0" convert --to json | python3 -c "import sys,json; print(repr(json.load(sys.stdin)[\"PEM\"]))"' "$EVNX"; run "$EVNX" convert --to yaml; run "$EVNX" convert --to shell; run "$EVNX" convert; expect 2 "KNOWN I-083: no terminal and no --to; error is doubled and says 'terminal'; decided: one line, 'pass --to'"; }

if [ $# -eq 0 ]; then set -- C0 C1 C2 C3 C4 C5 C6 C7 C8 C9 C10 C11; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
