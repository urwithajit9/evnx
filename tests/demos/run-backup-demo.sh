#!/usr/bin/env bash
# In-repo copy for tests/backup_runner.rs (unit U0, 2026-10-14). Source: the review package,
# local/backup-restore/tutorial/run-backup-demo.sh. Changes from the source are marked U0-EDIT. Run through
# `cargo test --test backup_runner`, or scripts/behaviour-lock.sh backup to compare two binaries.
# evnx backup / restore demo runner (BR0-BR12). Non-interactive (no terminal), safe: each scene runs in its own temp folder.
# Usage: EVNX=/path/to/evnx ./run-backup-demo.sh [BR5 ...]
# "KNOWN" lines mark behaviour logged as an issue in the review (21-backup-restore.md); they show 0.10.0 as it is today.
# Nothing here touches the network. All values are synthetic. Passphrases are demo values, not secrets.
set -u
EVNX="${EVNX:-evnx}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEMO_ROOT="${DEMO_ROOT:-$(mktemp -d)}"
export NO_COLOR=1
unset EVNX_PASSWORD
pass=0; fail=0
run() { echo; echo "\$ ${*/#$EVNX/evnx}"; "$@" > "$DEMO_ROOT/.out" 2>"$DEMO_ROOT/.err" </dev/null; local rc=$?; cat "$DEMO_ROOT/.out"; [ -s "$DEMO_ROOT/.err" ] && { echo "[stderr]"; cat "$DEMO_ROOT/.err"; }; echo "[exit $rc]"; LAST=$rc; }
expect() { if [ "$LAST" -eq "$1" ]; then pass=$((pass+1)); echo "  ok: $2 (exit $1)"; else fail=$((fail+1)); echo "  MISMATCH: $2 expected exit $1, got $LAST"; fi; }
check() { if eval "$1"; then pass=$((pass+1)); echo "  ok: $2"; else fail=$((fail+1)); echo "  MISMATCH: $2"; fi; }
scene() { echo; echo "================ $1 ================"; D="$DEMO_ROOT/$1"; rm -rf "$D"; mkdir -p "$D"; cd "$D" || exit 1; }
fixture() {
  # Mostly ordinary lines on purpose: the PEM value's continuation lines must stay under 20% of the file,
  # or 0.10.0 sends the restore to .restored (I-145, shown separately in BR6).
  cat > .env <<'X'
DB_URL=postgres://user:pw@db.internal/app
API_SECRET=sk_live_demo_not_a_real_key_000
NAME="My App # prod"
PORT=8080
HOST=0.0.0.0
LOG_LEVEL=info
REGION=eu-west-1
CACHE_TTL=300
FEATURE_X=on
WORKERS=4
PEM="-----BEGIN KEY-----
MIIEdemoLineOne
-----END KEY-----"
X
  printf 'demo-passphrase-123\n' > pw
}
modes() { stat -c '%a %n' "$@"; }

BR0() { scene BR0; fixture; run "$EVNX" --version; run "$EVNX" restore; expect 2 "restore needs a backup argument (clap)"; run "$EVNX" backup .env --env-name production --key-file pw; expect 2 "[ENV] and --env-name conflict"; run "$EVNX" backup --env .env --key-file pw; expect 2 "D-99/D-107: --env does not exist (docs examples use it)"; }
# U1-EDIT (2026-10-14): BR1 looks for whole cleartext strings, in the file and base64-decoded. It grepped
# for "PEM", which random base64 contains about once in 400 runs; CI hit it (T-071).
BR1() { scene BR1; fixture; run "$EVNX" backup --key-file pw --verify; expect 0 "backup with --verify"; run modes .env.backup; mv .env env.orig; run "$EVNX" restore .env.backup --key-file pw; expect 0 "restore with no .env present"; check 'cmp -s .env env.orig' "restored file is byte-identical (multi-line PEM, 'My App # prod' survive)"; run modes .env; check '[ "$(stat -c %a .env.backup)" = 600 ]' "backup mode 0600"; check '! grep -q "API_SECRET=\|sk_live_demo_not_a_real_key\|BEGIN KEY" .env.backup && ! base64 -d .env.backup 2>/dev/null | grep -aq "API_SECRET=\|sk_live_demo_not_a_real_key\|BEGIN KEY"' "the backup file holds no key name or value in clear"; }
BR2() { scene BR2; fixture; head -c 40 /dev/urandom > bin.key; run "$EVNX" backup --key-file bin.key --output b1.bak; expect 0 "binary key file"; run "$EVNX" restore b1.bak --key-file bin.key --inspect; expect 0 "same file under restore's spelling opens it (F7)"; run env EVNX_PASSWORD=env-passphrase-1 "$EVNX" backup --output b2.bak; expect 0 "EVNX_PASSWORD, warning printed"; run env EVNX_PASSWORD= "$EVNX" backup --output b3.bak; expect 2 "empty EVNX_PASSWORD refused"; : > empty.key; run "$EVNX" backup --key-file empty.key --output b4.bak; expect 2 "empty key file refused"; run env EVNX_PASSWORD=env-passphrase-1 "$EVNX" backup --key-file pw --output b5.bak; expect 0 "KNOWN I-155: file and env both set, the file wins with no notice"; run env EVNX_PASSWORD=env-passphrase-1 "$EVNX" restore b5.bak --inspect; expect 2 "the env secret does not open it (the file was used)"; echo a > short.key; run "$EVNX" backup --key-file short.key --output b6.bak; expect 0 "KNOWN I-147: a one-character secret is accepted; decided: floor 8 for new backups"; run "$EVNX" backup --output b7.bak; expect 2 "KNOWN I-148/D-101: no terminal, no secret: 'not a terminal'; decided: a line naming --key-file and EVNX_PASSWORD"; }
BR3() { scene BR3; fixture; for i in 1 2 3 4 5; do printf "N=$i\n" > .env; "$EVNX" backup --key-file pw >/dev/null 2>&1; done; for f in .env.backup .env.backup.1 .env.backup.2 .env.backup.3; do "$EVNX" restore "$f" --key-file pw --output "o.$f" >/dev/null 2>&1 </dev/null; echo "$f -> $(cat "o.$f" 2>/dev/null)"; done; ls -1 .env.backup*; check '[ "$(cat o..env.backup)" = N=5 ] && [ "$(cat o..env.backup.3)" = N=2 ]' "newest first, up to .3"; run "$EVNX" backup --key-file pw --keep 1; expect 0 "--keep 1 replaces .1 and warns"; run "$EVNX" backup --key-file pw --keep 0; expect 0 "--keep 0: no rotation"; mkdir t && cd t && cp ../pw . && printf 'A=1\n' > .env && printf '[backup]\nkeep = 1\n' > .evnx.toml && for i in 1 2 3; do "$EVNX" backup --key-file pw >/dev/null 2>&1; done; echo "toml keep=1:"; ls -1 .env.backup*; printf '[backup]\nkeep = "x"\n' > .evnx.toml; run "$EVNX" backup --key-file pw; expect 2 "bad config value is an error"; }
# U2-EDIT (2026-10-15): BR4 prints "absent" itself instead of stat's error, whose wording differs between
# GNU and uutils coreutils (CI failed, T-072).
# U2-EDIT (2026-10-15): BR4 flipped on purpose: directory output refused without renaming, symlink written through (I-146 part, I-151, I-152).
BR4() { scene BR4; fixture; mkdir odir; run "$EVNX" backup --key-file pw --output odir; expect 5 "I-146 partly fixed (U2): an existing directory is refused and nothing is renamed; DEC-B3 wants exit 2 (open)"; run bash -c 'for f in odir odir.1; do if [ -e "$f" ]; then stat -c "%F %n" "$f"; else echo "$f: absent"; fi; done'; cp .env src.env; run "$EVNX" backup src.env --key-file pw --output src.env; expect 0 "KNOWN I-146: output equals source: the .env is now ciphertext"; run bash -c 'ls src.env*; grep -c "^DB_URL=" src.env || echo "src.env no longer holds DB_URL= (it is ciphertext); the plaintext is only in src.env.1"'; run "$EVNX" backup --key-file pw --output nodir/x.bak; expect 5 "I-152 fixed (U2): a missing directory is named plainly"; ln -s real.bak lnk.bak; run "$EVNX" backup --key-file pw --output lnk.bak; expect 0 "I-151 fixed (DEC-B7): a symlink output is written through with a notice"; }
# U5-EDIT (2026-10-15): BR5 and BR6 notes flipped on purpose (I-145).
BR5() { scene BR5; fixture; printf 'export A=1\nPEM="-----BEGIN\nMIIE\n-----END"\nB=2\n' > ex.env; run "$EVNX" backup ex.env --key-file pw --output ex.bak; expect 0 "FIXED I-145: read as a .env, no warning"; run "$EVNX" restore ex.bak --key-file pw --inspect; expect 0 "inspect uses the real parser: three names, no values"; run "$EVNX" backup --key-file pw --output b.bak >/dev/null 2>&1; run "$EVNX" restore b.bak --key-file pw --dry-run; expect 0 "dry-run: nothing written"; "$EVNX" restore b.bak --key-file pw --output o.env --dry-run >/dev/null 2>&1 </dev/null; check '[ ! -e o.env ] && [ ! -e o.env.restored ]' "no file created by dry-run"; run "$EVNX" restore b.bak --key-file pw --inspect --dry-run; expect 0 "both flags: inspect wins"; check '! "$EVNX" restore b.bak --key-file pw --inspect 2>&1 </dev/null | grep -q "sk_live\|MIIEdemo\|postgres://"' "no value reaches the output"; }
BR6() { scene BR6; fixture; t() { printf "$1" > lk.env; "$EVNX" backup lk.env --key-file pw --output lk.bak >/dev/null 2>&1; rm -f lk.out lk.out.restored; "$EVNX" restore lk.bak --key-file pw --output lk.out >/dev/null 2>&1 </dev/null; echo "  [$2] exit $? $(ls lk.out* | tr '\n' ' ')"; }; echo "restore of valid-looking inputs (named path vs .restored):"; t 'A=1\n' "plain"; t '' "empty"; t '# only comments\n' "comments"; t 'export A=1\n' "FIXED I-145 export"; t 'my-key=1\nA=2\n' "dashed key: the parser rejects it too, stays .restored (correct)"; t 'a.b=1\n' "dotted key: the parser rejects it too, stays .restored (correct)"; t 'A=1\nB=2\nC=3\nD=4\nCERT="-----BEGIN\nl1\nl2\nl3\n-----END"\n' "FIXED I-145 PEM-heavy"; t 'just words\n' "prose (correct)"; printf 'just words\nmore words\n' > prose.txt; "$EVNX" backup prose.txt --key-file pw >/dev/null 2>&1; run "$EVNX" restore prose.txt.backup --key-file pw --output r.env; expect 5 "fallback: .restored, exit 5"; echo "KNOWN I-154: the line above says Restored 0 variable(s) with a tick next to exit 5"; }
BR7() { scene BR7; fixture; "$EVNX" backup --key-file pw >/dev/null 2>&1; run "$EVNX" restore .env.backup --key-file pw; expect 2 "KNOWN I-148: existing .env, no terminal: 'not a terminal', 'No verdict'; decided: --yes (alias --force) and a line naming it"; run "$EVNX" restore .env.backup --key-file pw --output .env.new; expect 0 "today's workaround: a new path, then mv"; check 'cmp -s .env .env.new' "identical content"; }
BR8() { scene BR8; fixture; "$EVNX" backup --key-file pw >/dev/null 2>&1; echo wrongwrong > bad; run "$EVNX" restore .env.backup --key-file bad --inspect; expect 2 "wrong password"; run "$EVNX" restore nope.backup --key-file pw; expect 3 "backup not found"; mkdir adir; run "$EVNX" restore adir --key-file pw; expect 3 "directory: not a regular file"; head -c 100 .env.backup > trunc.bak; run "$EVNX" restore trunc.bak --key-file pw --inspect; expect 2 "KNOWN I-149: truncated file says 'Wrong password or corrupt'"; echo 'garbage!!' > garb.bak; run "$EVNX" restore garb.bak --key-file pw --inspect; expect 2 "KNOWN I-149: not an evnx backup says the same"; python3 - <<'PY'
import base64
s=open('.env.backup').read().strip(); b=bytearray(base64.b64decode(s)); b[0]=2; open('ver2.bak','w').write(base64.b64encode(bytes(b)).decode())
b=bytearray(base64.b64decode(s)); b[60]^=1; open('flip.bak','w').write(base64.b64encode(bytes(b)).decode())
PY
run "$EVNX" restore ver2.bak --key-file pw --inspect; expect 2 "KNOWN I-149: a newer format (version byte 2) looks like a wrong password; decided: say 'upgrade'"; run "$EVNX" restore flip.bak --key-file pw --inspect; expect 2 "a flipped byte: tamper (stays one message with wrong password)"; }
# U2-EDIT (2026-10-15): BR9 flipped on purpose: restore writes through a symlink (I-151, DEC-B7).
BR9() { scene BR9; fixture; "$EVNX" backup --key-file pw >/dev/null 2>&1; run "$EVNX" restore cloud://myproject --key-file pw; expect 6 "KNOWN D-112: cloud:// exits 6 'not yet available'; decided (DEC-B4): removed, hint 'use evnx cloud pull'"; ln -s realtarget lnk.env; run "$EVNX" restore .env.backup --key-file pw --output lnk.env; expect 0 "I-151 fixed (DEC-B7): restore writes through the symlink with a notice"; }
# U0-EDIT (2026-10-14): fixtures live in tests/golden/backup/ in the repo.
BR10() { scene BR10; cp "$HERE/../golden/backup/golden-0.10.0.backup" golden.bak; cp "$HERE/../golden/backup/golden.key" golden.key; run "$EVNX" restore golden.bak --key-file golden.key --output golden.out; expect 0 "FORMAT LOCK: the backup made by 0.10.0 opens in this version"; check 'cmp -s golden.out "$HERE/../golden/backup/golden.env"' "and gives back the recorded content byte for byte"; }
# U0-EDIT (2026-10-14): BR11 -q check rewritten: 0.10.1 made -q hide the banner, so "-q changes nothing" no longer holds; the residual is still locked.
# U1-EDIT (2026-10-14): BR11 -q checks flipped on purpose: -q backup is silent on success (I-153, DEC-G4).
BR11() { scene BR11; fixture; run "$EVNX" -q backup --key-file pw; expect 0 "I-153 fixed (DEC-G4): -q backup prints nothing on success"; "$EVNX" -q backup --key-file pw > q.out 2>&1; "$EVNX" backup --key-file pw > n.out 2>&1; check '[ ! -s q.out ] && [ -s n.out ]' "I-153 fixed (DEC-G4): -q output is empty, without -q the summary prints"; "$EVNX" backup -v --key-file pw --output v.bak > v.out 2>&1 </dev/null; check '! grep -q "sk_live\|postgres://\|MIIEdemo\|demo-passphrase" v.out' "-v prints no secret value or passphrase"; }
BR12() { scene BR12; fixture; "$EVNX" backup --key-file pw >/dev/null 2>&1; for m in backup restore; do :; done; "$EVNX" restore .env.backup --key-file pw --output o.env -v > rv.out 2>&1 </dev/null; check '! grep -q "sk_live\|postgres://\|MIIEdemo\|demo-passphrase" rv.out' "restore -v prints no secret value or passphrase"; check '[ "$(stat -c %a o.env)" = 600 ]' "restored file mode 0600"; check 'cmp -s .env o.env' "round trip with the fixture"; }

if [ $# -eq 0 ]; then set -- BR0 BR1 BR2 BR3 BR4 BR5 BR6 BR7 BR8 BR9 BR10 BR11 BR12; fi
for s in "$@"; do "$s"; done
echo; echo "================ summary: $pass ok, $fail mismatch  (scratch: $DEMO_ROOT) ================"
[ "$fail" -eq 0 ]
