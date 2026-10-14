#!/usr/bin/env bash
# In-repo copy for tests/template_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/template/tutorial/run-template-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test template_runner`, or scripts/behaviour-lock.sh template to compare two binaries.
# evnx template demo runner (TP0-TP11). Non-interactive (no terminal), safe: each scene runs in its own temp folder.
# Usage: EVNX=/path/to/evnx ./run-template-demo.sh [TP5 ...]
# "KNOWN" lines mark behaviour logged as an issue in the review; they show 0.10.0 as it is today.
# Nothing here touches the network. All values are synthetic. Needs git for the gitignore scenes.
set -u
EVNX="${EVNX:-evnx}"
DEMO_ROOT="${DEMO_ROOT:-$(mktemp -d)}"
export NO_COLOR=1 GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
pass=0; fail=0
run() { echo; echo "\$ ${*/#$EVNX/evnx}"; "$@" > "$DEMO_ROOT/.out" 2>"$DEMO_ROOT/.err" </dev/null; local rc=$?; cat "$DEMO_ROOT/.out"; [ -s "$DEMO_ROOT/.err" ] && { echo "[stderr]"; cat "$DEMO_ROOT/.err"; }; echo "[exit $rc]"; LAST=$rc; }
expect() { if [ "$LAST" -eq "$1" ]; then pass=$((pass+1)); echo "  ok: $2 (exit $1)"; else fail=$((fail+1)); echo "  MISMATCH: $2 expected exit $1, got $LAST"; fi; }
scene() { echo; echo "================ $1 ================"; D="$DEMO_ROOT/$1"; rm -rf "$D"; mkdir -p "$D"; cd "$D" || exit 1; }
fixture() {
  cat > .env <<'X'
DB_HOST=db.internal
DB_PORT=5432
DB_SSL=yes
DB_NAME=app
API_SECRET=sk_live_abcdefghijklmnopqrstuvwx
GREETING="say \"hi\""
EMPTY=
X
  cat > config.tpl.yml <<'X'
database:
  host: {{DB_HOST}}
  port: {{DB_PORT|int}}
  ssl: ${DB_SSL}
  name: {{DB_NAME|upper}}
  title: {{ DB_NAME | title }}
  flag: {{DB_SSL|bool}}
secret: $API_SECRET
msg: {{GREETING|json}}
opt: {{NOPE|default:fallback}}
empty: {{EMPTY|default:dflt}}
X
}
tpl() { printf '%s\n' "$1" > t.tpl; }
env_() { printf '%b' "$1" > t.env; }
render() { "$EVNX" template --input t.tpl --output t.out --env t.env --no-gitignore >/dev/null 2>t.err </dev/null; local rc=$?; local o=""; [ -f t.out ] && o=$(tr '\n' '~' < t.out); echo "  [$rc] $o $(grep -o 'Error: .*' t.err | head -1 | cut -c1-100)"; rm -f t.out; }
show() { echo "--- $1:"; cat "$1"; }

TP0() { scene TP0; fixture; run "$EVNX" --version; run "$EVNX" template; expect 2 "--input and --output are required (clap)"; run "$EVNX" template --input config.tpl.yml --output o --gitignore --no-gitignore; expect 2 "the two gitignore flags conflict"; }
TP1() { scene TP1; fixture; run "$EVNX" template --input config.tpl.yml --output out.yml --no-gitignore; expect 0 "all three styles, whitespace, int, upper, title, bool, json, default"; show out.yml; run bash -c 'stat -c "out.yml mode %a" out.yml'; echo "KNOWN I-130: the file holds a live-looking key and is created 644; decided: 0600 (and tighten an existing file with a notice)"; }
TP2() { scene TP2; fixture; echo "--- filter values (rendered; [exit] then output):"; env_ 'A=hello world\nB=on\nC=yes\nD=maybe\nE= 8000\nF=8000.0\nG=+5\nH=\n'; tpl 'title={{A|title}}'; render; tpl 'bool on={{B|bool}} yes={{C|bool}} maybe={{D|bool}}'; render; tpl 'int={{E|int}} plus={{G|int}}'; render; tpl 'bad={{F|int}}'; render; tpl 'empty={{H|int}}'; render; echo "KNOWN I-137: title only capitalises the first letter; I-138: on and maybe are silently false; decided: real title case, strict |bool"; }
TP3() { scene TP3; fixture; printf 'a: {{MISSING}}\nb: $ALSO\nc: ${NOPE2}\nd: $HOME\n' > u.tpl; run "$EVNX" template --input u.tpl --output u.out --no-gitignore; expect 0 "KNOWN I-142: unresolved references are written through, exit 0; decided: error by default"; run "$EVNX" template --input u.tpl --output u2.out --strict --no-gitignore; expect 2 "--strict refuses, names all four (including \$HOME: KNOWN I-135), writes nothing"; run bash -c 'test -e u2.out && echo "u2.out exists" || echo "u2.out not created"'; }
TP4() { scene TP4; fixture; printf 'a: {{DB_HOST|bogus}}\nb: ${DB_HOST:-dflt}\nc: {{DB_HOST|upper|json}}\nd: \\$DB_HOST\ne: $$DB_HOST\n' > e.tpl; run "$EVNX" template --input e.tpl --output e.out --no-gitignore; expect 0 "KNOWN I-136/I-135: unknown filter, bash default and chained filter are written through silently; no escape"; show e.out; echo "decided: unknown filter is an error, chaining works, backslash escape"; }
TP5() { scene TP5; fixture; printf 'E=\nB=bee\n' > t.env; echo 'a={{E|default:one}} b={{E|default:two}}' > t.tpl; run "$EVNX" template --input t.tpl --output t.out --env t.env --no-gitignore; expect 0 "KNOWN I-134: the second default reuses the first"; show t.out; printf 'A={{B|upper}}\nB=bee\n' > r.env; echo 'v={{A|json}}' > r.tpl; run "$EVNX" template --input r.tpl --output r.out --env r.env --no-gitignore; expect 0 "KNOWN I-133: the value of A is evaluated as a template by a later key's filter"; show r.out; }
TP6() { scene TP6; fixture; cp config.tpl.yml same.yml; run "$EVNX" template --input same.yml --output same.yml --no-gitignore; expect 0 "KNOWN I-131: input equal to output is accepted"; run bash -c 'head -3 same.yml; grep -c "{{" same.yml'; echo "the template now holds real values and no placeholders; decided: refuse, exit 2"; echo old > exist.yml; run "$EVNX" template --input config.tpl.yml --output exist.yml --no-gitignore; expect 0 "KNOWN I-132: an existing output is replaced without a word; decided: notice 'Replaced'"; }
TP7() { scene TP7; fixture; mkdir real; echo keep > real/target.yml; ln -s real/target.yml link.yml; run "$EVNX" template --input config.tpl.yml --output link.yml --no-gitignore; expect 0 "KNOWN I-140: the symlink target is overwritten silently"; run bash -c 'test -L link.yml && echo "link kept"; head -2 real/target.yml'; run "$EVNX" template --input config.tpl.yml --output - --no-gitignore; expect 0 "KNOWN S-T1: --output - writes a FILE named -"; run bash -c 'test -f ./- && echo "file named - exists"'; run "$EVNX" template --input config.tpl.yml --output nodir/x.yml --no-gitignore; expect 2 "missing output directory: clear error (decided: hint and --mkdir)"; }
TP8() { scene TP8; fixture; git init -q .; run "$EVNX" template --input config.tpl.yml --output out1.yml --gitignore; expect 0 "--gitignore adds the output"; show .gitignore; run "$EVNX" template --input config.tpl.yml --output out2.yml; expect 0 "default mode without a terminal: warning only"; printf 'out3.yml\n!out3.yml\n' >> .gitignore; run "$EVNX" template --input config.tpl.yml --output out3.yml --gitignore; expect 0 "KNOWN I-139: says already in .gitignore, but the negation means git will commit it"; run bash -c 'git check-ignore -q out3.yml && echo "git: ignored" || echo "git: NOT ignored"'; git add out4.yml 2>/dev/null; echo x > out4.yml; git add -f out4.yml; git -c user.email=a@b -c user.name=n commit -qm x; run "$EVNX" template --input config.tpl.yml --output out4.yml --gitignore; expect 0 "KNOWN O-8: already tracked: added to .gitignore, no word that it stays tracked"; run bash -c 'git ls-files out4.yml'; }
TP9() { scene TP9; fixture; cp .env .env.production; sed -i 's/db.internal/prod.db/' .env.production; run "$EVNX" template --input config.tpl.yml --output p.yml --env-name production --no-gitignore; expect 0 "--env-name production"; run bash -c 'grep host p.yml'; run "$EVNX" template --input config.tpl.yml --output p2.yml --env-name typo --no-gitignore; expect 2 "unknown environment lists the existing ones"; run "$EVNX" template --input nope.tpl --output p3.yml --no-gitignore; expect 2 "missing input"; run "$EVNX" template --input config.tpl.yml --output p4.yml --env nope.env --no-gitignore; expect 2 "missing env file"; }
TP10() { scene TP10; printf 'BASE=http://localhost\nURL=$BASE/api\nHOSTP=$BASE:8080\nSITE=$BASE.example.com\nPASS=a$BASE!\nOK=${BASE}/api\n' > t.env; echo 'url={{URL}} hostp={{HOSTP}} site={{SITE}} pass={{PASS}} ok={{OK}}' > t.tpl; run "$EVNX" template --input t.tpl --output t.out --env t.env --no-gitignore; expect 0 "KNOWN I-141: the parser drops the character after a bare \$NAME"; show t.out; run bash -c '"$0" convert --to json --env t.env | grep -E "URL|OK"' "$EVNX"; echo "decided: keep the terminator (every command)"; }
TP11() { scene TP11; fixture; run "$EVNX" template -i config.tpl.yml -o out.yml; expect 2 "D-88: the module documentation shows -i and -o; they do not exist"; }

if [ $# -eq 0 ]; then set -- TP0 TP1 TP2 TP3 TP4 TP5 TP6 TP7 TP8 TP9 TP10 TP11; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
