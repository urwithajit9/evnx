#!/usr/bin/env bash
# In-repo copy for tests/init_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/init/tutorial/run-init-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test init_runner`, or scripts/behaviour-lock.sh init to compare two binaries.
# evnx init demo runner for evnx 0.10.1. Non-interactive scenarios (S1-S10, S12-S18).
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
S2() { scene S2; run "$EVNX" init --yes; run "$EVNX" init --yes; expect 2 "second init refuses"; echo "KEEP=1" > .env; run "$EVNX" init --yes --force; expect 0 "--force replaces .env.example"; run cat .env
  check ".env still holds KEEP=1 (never overwritten)" grep -qx 'KEEP=1' .env; }
S3() { scene S3; run "$EVNX" init --list-components; expect 0 "list components"
  check "51 components" bash -c "[ \$('$EVNX' init --list-components | grep -cE '^    [a-z_0-9]+ ') -eq 51 ]"
  check "aliases are listed" bash -c "'$EVNX' init --list-components | grep -q 'PostgreSQL (also: postgres, pg)'"; }
S4() { scene S4; run "$EVNX" init --with vite,supabase,clerk,vercel --yes; expect 0 "pulse stack"; run cat .env.example
  check "17 variables" bash -c "[ \$(grep -cE '^[A-Z][A-Z0-9_]*=' .env.example) -eq 17 ]"
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
S14() { scene S14; echo "Agent-style run: no terminal, flags only, machine-checkable exit codes"; run "$EVNX" init --with nextjs,postgresql,stripe --yes </dev/null; expect 0 "agent init"; run "$EVNX" init --with nextjs --yes </dev/null; expect 2 "agent re-run is refused, not silently overwritten"; }
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
  check "exit 141 on a closed pipe" bash -c "rm -rf p && mkdir p && cd p && { sleep 0.3; '$EVNX' init --with nextjs --yes; } | true; [ \${PIPESTATUS[0]} -eq 141 ]"
  check ".gitignore exists whenever .env does" bash -c "cd p && [ ! -f .env ] || grep -qx '.env\\*' .gitignore"; }
S18() { scene S18; echo ".env from a template you already have"; mkdir src; printf 'const a = process.env.NEW_ONE;\n' > src/a.ts; printf 'HAND=1\n' > .env.example
  run "$EVNX" init --from-source --yes; expect 0 "from-source beside a template"
  check ".env is a copy of the template" cmp -s .env .env.example
  check ".gitignore written" grep -qx '.env\*' .gitignore; }

if [ $# -eq 0 ]; then set -- S0 S1 S2 S3 S4 S5 S6 S7 S8 S9 S10 S11 S12 S13 S14 S15 S16 S17 S18; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
