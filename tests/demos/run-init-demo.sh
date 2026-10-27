#!/usr/bin/env bash
# In-repo copy for tests/init_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/init/tutorial/run-init-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test init_runner`, or scripts/behaviour-lock.sh init to compare two binaries.
# evnx init demo runner for evnx 0.10.1. Non-interactive scenarios (S1-S10, S12-S26; S19-S26 added by R-init).
# Usage:  EVNX=/path/to/evnx ./run-init-demo.sh [scenario ...]     e.g.  ./run-init-demo.sh S4 S16
# Each scenario starts in its own empty scratch folder under $DEMO_ROOT (default: a temp dir).
# Prints each command, its output and "[exit N]", then checks exit codes (expect) and
# content (check). Safe: it never touches your real projects.
# For evnx 0.10.0 use archive-0.10.0/run-init-demo.sh.
set -u
EVNX="${EVNX:-evnx}"
DEMO_ROOT="${DEMO_ROOT:-$(mktemp -d)}"
export NO_COLOR=1
pass=0; fail=0
run() { echo; echo "\$ ${*/#$EVNX/evnx}"; "$@" 2>&1; local rc=$?; echo "[exit $rc]"; LAST=$rc; }
expect() { # expect <exit code> <label>
  if [ "$LAST" -eq "$1" ]; then pass=$((pass+1)); echo "  ok: $2 (exit $1)"; else fail=$((fail+1)); echo "  MISMATCH: $2 expected exit $1, got $LAST"; fi; }
check() { # check <label> <command...>: passes when the command succeeds
  local label=$1; shift
  if "$@" >/dev/null 2>&1; then pass=$((pass+1)); echo "  ok: $label"; else fail=$((fail+1)); echo "  MISMATCH: $label"; fi; }
scene() { echo; echo "================ $1 ================"; D="$DEMO_ROOT/$1"; rm -rf "$D"; mkdir -p "$D"; cd "$D" || exit 1; }

S0() { scene S0; run "$EVNX" --version; run "$EVNX" init --help | head -n 30 >/dev/null; echo "(help truncated; see slide 'Flag reference')"; }
S1() { scene S1; run "$EVNX" init --yes; expect 0 "blank init"; run ls -A; run cat .env.example; run cat .gitignore
  check "says 'no variables yet', not 'empty'" bash -c "'$EVNX' init --yes --force | grep -q 'no variables yet'"; }
S2() { scene S2; run "$EVNX" init --yes; run "$EVNX" init --yes; expect 0 "FIXED DQ-31 (R-init): a second init --yes is already set up, not a refusal"; echo "KEEP=1" > .env; run "$EVNX" init --yes --force; expect 0 "--force replaces .env.example"; run cat .env
  check ".env still holds KEEP=1 (never overwritten)" grep -qx 'KEEP=1' .env; }
S3() { scene S3; run "$EVNX" init --list-components; expect 0 "list components"
  check "56 components (R-init, IN-14: astro, hono, neon, turnstile, fly)" bash -c "[ \$('$EVNX' init --list-components | grep -cE '^    [a-z_0-9]+ ') -eq 56 ]"
  check "aliases are listed" bash -c "'$EVNX' init --list-components | grep -q 'PostgreSQL (also: postgres, pg)'"; }
S4() { scene S4; run "$EVNX" init --with vite,supabase,clerk,vercel --yes; expect 0 "pulse stack"; run cat .env.example
  check "15 variables (R-init, I-002b: Clerk's and Vercel's NEXT_PUBLIC_ names are Next.js only)" bash -c "[ \$(grep -cE '^[A-Z][A-Z0-9_]*=' .env.example) -eq 15 ]"
  check "VITE_ copies present" bash -c "grep -q '^VITE_SUPABASE_URL=' .env.example && grep -q '^VITE_SUPABASE_ANON_KEY=' .env.example && grep -q '^VITE_CLERK_PUBLISHABLE_KEY=' .env.example"
  check "originals kept" bash -c "grep -q '^SUPABASE_URL=' .env.example && grep -q '^CLERK_PUBLISHABLE_KEY=' .env.example"; }
S5() { scene S5; run "$EVNX" init --blueprint t3_modern --yes; expect 0 "t3_modern"; check "28 variables" bash -c "[ \$(grep -cE '^[A-Z][A-Z0-9_]*=' .env.example) -eq 28 ]"; run "$EVNX" init --blueprint nope --yes; expect 2 "unknown blueprint lists ids"; }
S6() { scene S6; run "$EVNX" init --with vite,supabse --yes; expect 2 "typo is refused"; check "typo gets a suggestion" bash -c "'$EVNX' init --with vite,supabse --yes 2>&1 | grep -q 'Did you mean: supabase'"
  run "$EVNX" init --with postgre --yes; expect 2 "prefix hint works"
  run "$EVNX" init --blueprint mern_v2 --with stripe --yes; expect 2 "conflict"; run "$EVNX" init --detect --with stripe --yes; expect 2 "conflict"; }
S7() { scene S7; echo '{"dependencies":{"vite":"5","@clerk/clerk-react":"5","@supabase/supabase-js":"2","stripe":"14"}}' > package.json; run "$EVNX" init --detect --yes; expect 0 "detect"
  check "Clerk detected (@clerk/clerk-react)" grep -q '^CLERK_PUBLISHABLE_KEY=' .env.example; }
S8() { scene S8; mkdir src; printf 'export const a = import.meta.env.VITE_SUPABASE_URL;\nexport const b = process.env.TURNSTILE_SECRET;\nexport const c = "VITE_CLERK_PUBLISHABLE_KEY";\n' > src/config.ts
  run "$EVNX" init --from-source --yes; expect 0 "from-source on a fresh project"; run cat .env.example
  check "import.meta.env read" grep -q '^VITE_SUPABASE_URL=' .env.example
  printf 'OLD_UNUSED=1\n' > .env.example; run "$EVNX" init --from-source --yes; expect 0 "append to existing"; run cat .env.example
  check "existing line kept" grep -qx 'OLD_UNUSED=1' .env.example
  run "$EVNX" init --from-source --yes --force; expect 2 "--force with --from-source is refused"; }
S9() { scene S9; run "$EVNX" init --yes -p packages/api; expect 0 "path created (-p)"; run ls -A packages/api; }
S10() { scene S10; echo "SECRET=real-value" > .env; run "$EVNX" init --with vite --yes; expect 0 "init with existing .env"; run cat .env
  check ".env unchanged (never overwritten)" grep -qx 'SECRET=real-value' .env; }
S11() { echo; echo "================ S11 (interactive) ================"; echo "Not scripted. Run by hand: $EVNX init   then follow the runbook keys (Blank is pre-selected). Or use drive-pty.py."; }
S12() { scene S12; run bash -c "echo '1' | '$EVNX' init"; expect 2 "no terminal and no flags: stops"
  check "the hint names the flags" bash -c "echo 1 | '$EVNX' init 2>&1 | grep -q -- '--blueprint <id>'"
  run bash -c "'$EVNX' init --with stripe </dev/null"; expect 2 "--with without --yes and no terminal"
  check "...says to add --yes" bash -c "'$EVNX' init --with stripe </dev/null 2>&1 | grep -q 'evnx init --with stripe --yes'"; }
S13() { scene S13; run "$EVNX" -q init --yes; expect 0 "quiet init"
  check "-q prints nothing" bash -c "rm -f .env* .gitignore; [ -z \"\$('$EVNX' -q init --yes 2>&1)\" ]"; }
S14() { scene S14; echo "Agent-style run: no terminal, flags only, machine-checkable exit codes"; run "$EVNX" init --with nextjs,postgresql,stripe --yes </dev/null; expect 0 "agent init"; run "$EVNX" init --with nextjs --yes </dev/null; expect 2 "agent re-run is refused, not silently overwritten"
  check "FIXED I-302 (R-init): the refusal names evnx add, which keeps the template" bash -c "'$EVNX' init --with nextjs --yes </dev/null 2>&1 | grep -q 'evnx add blueprint <id> or evnx add service <id>'"; }
S15() { scene S15; echo "Aliases write exactly what the id writes"; run "$EVNX" init --with next,pg --yes; expect 0 "aliases next,pg"
  mkdir ids && (cd ids && "$EVNX" init --with nextjs,postgresql --yes >/dev/null 2>&1)
  check "same file as nextjs,postgresql" bash -c "diff <(grep -v '^# Generated' .env.example) <(grep -v '^# Generated' ids/.env.example)"
  run "$EVNX" init --blueprint t3modern --yes; expect 2 "mistyped blueprint"; check "blueprint typo hint" bash -c "'$EVNX' init --blueprint t3modern --yes 2>&1 | grep -q 'Did you mean: t3_modern'"; }
S16() { scene S16; echo "Browser copies: public values only"; run "$EVNX" init --with nextjs,supabase,stripe --yes; expect 0 "next + supabase + stripe"
  check "NEXT_PUBLIC_ copies" bash -c "grep -q '^NEXT_PUBLIC_SUPABASE_URL=' .env.example && grep -q '^NEXT_PUBLIC_STRIPE_PUBLIC_KEY=' .env.example"
  check "no secret copied" bash -c "! grep -qE '^NEXT_PUBLIC_(SUPABASE_SERVICE_ROLE_KEY|SUPABASE_JWT_SECRET|STRIPE_SECRET_KEY)=' .env.example"
  mkdir srv && (cd srv && "$EVNX" init --with express,postgresql --yes >/dev/null 2>&1)
  check "server framework: no copies" bash -c "! grep -qE '^(VITE|NEXT_PUBLIC|PUBLIC|NUXT_PUBLIC)_' srv/.env.example"; }
# U0-EDIT (2026-10-14): S17 reader is `true` after a 0.3 s delay instead of `head -n 1`, so the pipe is
# closed before evnx writes. With head, whether evnx hit the closed pipe depended on timing (T-067).
S17() { scene S17; echo "Piped into head: files complete, quiet exit 141"; run bash -c "{ sleep 0.3; '$EVNX' init --with nextjs --yes; } | true; exit \${PIPESTATUS[0]}"
  check "exit 141 on a closed pipe" bash -c "rm -rf p && mkdir p && cd p && git init -q && { sleep 0.3; '$EVNX' init --with nextjs --yes; } | true; [ \${PIPESTATUS[0]} -eq 141 ]"
  check ".gitignore exists whenever .env does" bash -c "cd p && [ ! -f .env ] || grep -qx '.env\\*' .gitignore"
  # p and q are repositories so the contract is theirs, not the scene folder's.
  check "RM-03: all four files, .evnx.toml included" bash -c "cd p && [ -f .env ] && [ -f .env.example ] && [ -f .gitignore ] && [ -f .evnx.toml ]"
  check "RM-03: the same for a blueprint" bash -c "rm -rf q && mkdir q && cd q && git init -q && { sleep 0.3; '$EVNX' init --blueprint t3_modern --yes; } | true; [ -f .env ] && [ -f .env.example ] && [ -f .gitignore ] && [ -f .evnx.toml ]"; }
S18() { scene S18; echo ".env from a template you already have"; mkdir src; printf 'const a = process.env.NEW_ONE;\n' > src/a.ts; printf 'HAND=1\n' > .env.example
  run "$EVNX" init --from-source --yes; expect 0 "from-source beside a template"
  check ".env is a copy of the template" cmp -s .env .env.example
  check ".gitignore written" grep -qx '.env\*' .gitignore; }

# R-init (0.10.16, 2026-10-22): S19-S26 lock the open issues and the DQ-26..DQ-33 decisions. Each KNOWN check
# holds on 0.10.15 and flips to FIXED with its fix (plan: local/init/plan/PLAN-INIT-0.10.16.md in the review package).
# logrun <file> <command...>: like run, and keeps the combined output in <file> for the checks after it.
logrun() { local f=$1; shift; echo; echo "\$ ${*/#$EVNX/evnx}"; "$@" > "$f" 2>&1; local rc=$?; cat "$f"; echo "[exit $rc]"; LAST=$rc; }
repo() { mkdir -p "$1" && git -C "$1" init -q; }
S19() { scene S19; echo "A blueprint id where a component is expected, and a prefix of one"
  logrun mern.out "$EVNX" init --blueprint mern --yes; expect 2 "unknown blueprint"
  check "FIXED I-235 (R-init): a prefix of mern_v2 is suggested, by the component matcher" bash -c "grep -i 'did you mean' mern.out | grep -q 'mern_v2'"
  logrun t3.out "$EVNX" init --with t3_modern --yes; expect 2 "a blueprint id given to --with"
  check "FIXED I-235 (R-init): a blueprint id in --with names --blueprint t3_modern" grep -q -- '--blueprint t3_modern' t3.out; }
S20() { scene S20; echo "--from-source: destructuring, split lines, other languages"; mkdir src
  printf 'const { API_KEY, DB_URL: db, PORT = 3000 } = process.env;\nconst split = process\n  .env.SPLIT_LINE;\n' > src/a.js
  printf 'import os\nv = os.getenv("PY_GETENV")\n' > src/b.py; printf 'key = ENV.fetch("RUBY_FETCH")\n' > src/c.rb
  printf '<?php $k = getenv("PHP_GETENV");\n' > src/d.php; printf 'const K: &str = env!("RUST_ENV_MACRO");\n' > src/e.rs
  run "$EVNX" init --from-source --yes; expect 0 "from-source"
  check "FIXED I-236 (R-init, DQ-22): destructured names are found, plain, renamed and defaulted" bash -c "[ \$(grep -cE '^(API_KEY|DB_URL|PORT)=' .env.example) -eq 3 ]"
  check "FIXED I-236 (R-init): access split across lines is found" grep -q '^SPLIT_LINE=' .env.example
  check "FIXED I-236 (R-init): os.getenv, ENV.fetch, getenv and env! are found" bash -c "[ \$(grep -cE '^(PY_GETENV|RUBY_FETCH|PHP_GETENV|RUST_ENV_MACRO)=' .env.example) -eq 4 ]"; }
S21() { scene S21; echo "Browser names on a Vite stack; compose.yaml"; run "$EVNX" init --with vite,clerk,vercel --yes; expect 0 "vite + clerk + vercel"
  check "FIXED I-002b (R-init, DQ-23): no NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY on a Vite stack; VITE_CLERK_PUBLISHABLE_KEY instead" bash -c "! grep -q '^NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY=' .env.example && grep -q '^VITE_CLERK_PUBLISHABLE_KEY=' .env.example"
  check "FIXED I-002b (R-init): no NEXT_PUBLIC_VERCEL_ENV on a Vite stack" bash -c "! grep -q '^NEXT_PUBLIC_VERCEL_ENV=' .env.example"
  mkdir compose; printf 'services:\n  db:\n    image: postgres:16\n' > compose/compose.yaml
  run "$EVNX" init --detect -p compose; expect 0 "detect beside compose.yaml"
  check "FIXED I-257 (R-init): compose.yaml is read (core::project::compose_files)" grep -q '^DATABASE_URL=' compose/.env.example; }
S22() { scene S22; echo "The contract: .evnx.toml"; repo new; run "$EVNX" init --with redis --yes -p new; expect 0 "init in a repo"
  check "FIXED I-242 (R-init, DQ-18): .evnx.toml is created with [vars]" grep -q '^\[vars.REDIS_URL\]' new/.evnx.toml
  repo vars; printf '[vars.EXISTING]\nrequired = true\n' > vars/.evnx.toml; run "$EVNX" init --with redis --yes -p vars; expect 0 "beside a contract"
  check "FIXED I-242 (DQ-17): new names are recorded into [vars]; the entry there is kept" bash -c "grep -q '^\[vars.REDIS_URL\]' vars/.evnx.toml && head -2 vars/.evnx.toml | grep -qx 'required = true'"
  repo novars; printf '[scan]\nseverity = "high"\n' > novars/.evnx.toml; run "$EVNX" init --with redis --yes -p novars; expect 0 "beside a config without [vars]"
  check "FIXED DQ-27 (R-init): [vars.NAME] tables are appended, [scan] kept" bash -c "grep -q '^\[vars.REDIS_URL\]' novars/.evnx.toml && head -1 novars/.evnx.toml | grep -qx '\[scan\]'"
  repo mono; mkdir -p mono/apps/web; run "$EVNX" init --with nextjs --yes -p mono/apps/web; expect 0 "a package below the git root"
  check "FIXED DQ-26 (R-init): the package gets its own .evnx.toml, the root none" bash -c "[ -e mono/apps/web/.evnx.toml ] && [ ! -e mono/.evnx.toml ]"; }
S23() { scene S23; echo "The project's own template"; repo sample; printf 'FROM_SAMPLE=x\n' > sample/.env.sample
  run "$EVNX" init --with postgresql --yes -p sample; expect 2 "FIXED I-295 (R-init, DQ-28): .env.sample is the template; a stack over it is refused, as over .env.example"
  check "FIXED I-295: no second template" test ! -e sample/.env.example
  run "$EVNX" init --with postgresql --yes --force -p sample; expect 0 "--force replaces .env.sample"
  check "the stack is in .env.sample" grep -q '^DATABASE_URL=' sample/.env.sample
  repo cfg; printf '[defaults]\nexample = ".env.template"\n' > cfg/.evnx.toml
  run "$EVNX" init --with redis --yes -p cfg; expect 0 "[defaults] example names .env.template, not there yet"
  check "FIXED I-295: [defaults] example is the file written; no .env.example" bash -c "grep -q '^REDIS_URL=' cfg/.env.template && [ ! -e cfg/.env.example ]"; }
S24() { scene S24; echo "Other env files"; repo named; printf '[defaults]\nenv_name = "local"\n' > named/.evnx.toml
  logrun named.out "$EVNX" init --yes -p named; expect 0 "[defaults] env_name = local"
  check "RM-17: init writes .env.local, the file [defaults] env_name names, and no .env" bash -c "[ -f named/.env.local ] && [ ! -f named/.env ]"
  repo multi; for n in production staging test; do printf "APP_ENV=%s\n" "$n" > "multi/.env.$n"; done
  logrun multi.out "$EVNX" init --with stripe --yes -p multi; expect 0 ".env.production, .env.staging, .env.test present"
  check "FIXED I-296: the env files left unchanged are named, with how to check one" bash -c "grep -q 'env.production' multi.out && grep -q 'validate --env-name production' multi.out"
  repo rc; printf 'export A=1\n' > rc/.envrc
  logrun rc.out "$EVNX" init --yes -p rc; expect 0 "beside a direnv .envrc"
  check ".envrc is ignored by .env* (stays, DQ-33)" git -C rc check-ignore -q .envrc
  check "FIXED I-174 (R-init, DQ-33): one line says so, and how to commit it" grep -q '!.envrc' rc.out; }
S25() { scene S25; echo "An existing .env; a fresh clone"; repo old; printf 'DATABASE_URL=postgres://real@db/app\nSECRET_KEY=s3cr3t\n' > old/.env
  run "$EVNX" init --with nextjs,postgresql --yes -p old; expect 0 "beside a real .env"
  check ".env values kept" bash -c "grep -qx 'SECRET_KEY=s3cr3t' old/.env && [ \$(grep -c '^DATABASE_URL=' old/.env) -eq 1 ]"
  check "FIXED I-297 (R-init, DQ-29): .env has every name the template has" bash -c 'for n in $(grep -oE "^[A-Z][A-Z0-9_]*" old/.env.example); do grep -q "^$n=" old/.env || exit 1; done'
  repo clone; printf 'A=\nB=\n' > clone/.env.example
  run "$EVNX" init --yes -p clone; expect 0 "FIXED DQ-31 (R-init): init --yes on a fresh clone keeps the template and creates .env"
  check "FIXED DQ-31: .env is a copy of the template" cmp -s clone/.env clone/.env.example; }
S26() { scene S26; echo "The target's .evnx.toml, run from another folder"; repo broken; printf '[vars\nbroken' > broken/.evnx.toml
  run "$EVNX" init --yes -p broken; expect 2 "FIXED I-298 (R-init): a malformed .evnx.toml at the target stops init"
  check "FIXED I-298: nothing written beside it" bash -c "[ ! -e broken/.env.example ] && [ ! -e broken/.env ] && [ ! -e broken/.gitignore ]"
  repo bare; printf '[vars.A]\nrequired = true\n' > bare/evnx.toml
  run "$EVNX" init --yes -p bare; expect 2 "FIXED I-298: a bare evnx.toml at the target stops init, naming the rename"; }
if [ $# -eq 0 ]; then set -- S0 S1 S2 S3 S4 S5 S6 S7 S8 S9 S10 S11 S12 S13 S14 S15 S16 S17 S18 S19 S20 S21 S22 S23 S24 S25 S26; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
