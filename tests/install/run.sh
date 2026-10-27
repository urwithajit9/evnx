#!/usr/bin/env bash
# Tests for scripts/install.sh (RM-20, UP-11). No network: `curl`, `uname`,
# `ldd` and `sudo` are stubs on a temp PATH, and the "release" is a fake
# archive whose binary prints `evnx <version>`.
#
#   bash tests/install/run.sh [path/to/install.sh]
#
# ⚠️ Every case runs with its own HOME and an unwritable stand-in for
# /usr/local/bin (`_EVNX_SYSTEM_DIR`), so nothing outside the temp folder is
# written. A call to `sudo` fails the case.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SCRIPT="${1:-$ROOT/scripts/install.sh}"
WORK=$(mktemp -d)
trap 'chmod -R u+w "$WORK" 2>/dev/null; rm -rf "$WORK"' EXIT

pass=0
fail=0
ok() { pass=$((pass + 1)); echo "  ok: $1"; }
bad() { fail=$((fail + 1)); echo "  FAIL: $1"; echo "$OUT" | sed 's/^/      | /'; }

# ── Fake releases ────────────────────────────────────────────────────────────
ASSET="evnx-x86_64-unknown-linux-gnu.tar.gz"
RELEASES="$WORK/releases"
release() { # version
    local dir="$RELEASES/v$1"
    mkdir -p "$dir/build"
    printf '#!/bin/sh\necho "evnx %s"\n' "$1" > "$dir/build/evnx-x86_64-unknown-linux-gnu"
    chmod +x "$dir/build/evnx-x86_64-unknown-linux-gnu"
    tar -czf "$dir/$ASSET" -C "$dir/build" evnx-x86_64-unknown-linux-gnu
    (cd "$dir" && if command -v sha256sum >/dev/null; then sha256sum "$ASSET"; else shasum -a 256 "$ASSET"; fi > "$ASSET.sha256")
}
release 0.10.18
release 0.10.15
release 0.11.0

# ── Stubs ────────────────────────────────────────────────────────────────────
STUBS="$WORK/stubs"
mkdir -p "$STUBS"
cat > "$STUBS/curl" <<EOF
#!/usr/bin/env bash
# The latest-release redirect, or a download from \$RELEASES by URL.
echo "curl \$*" >> "$WORK/curl.log"
url="" out=""
while [ \$# -gt 0 ]; do
    case "\$1" in
        -o) out="\$2"; shift 2 ;;
        -w) shift 2 ;;
        http*) url="\$1"; shift ;;
        *) shift ;;
    esac
done
case "\$url" in
    */releases/latest) printf 'https://github.com/urwithajit9/evnx/releases/tag/v%s' "\${STUB_LATEST:-0.10.18}" ;;
    */releases/download/*)
        rel=\${url#*/releases/download/}
        src="$RELEASES/\$rel"
        [ -f "\$src" ] || exit 22
        cp "\$src" "\$out" ;;
    *) exit 22 ;;
esac
EOF
printf '#!/bin/sh\ncase "$1" in -s) echo Linux ;; -m) echo x86_64 ;; *) /usr/bin/uname "$@" ;; esac\n' > "$STUBS/uname"
printf '#!/bin/sh\necho "ldd (GNU libc) 2.39"\n' > "$STUBS/ldd"
printf '#!/bin/sh\necho "sudo called: $*" >&2\nexit 99\n' > "$STUBS/sudo"
# gh (RM-83): STUB_GH=ok | fail | noauth | old. Logs its arguments.
cat > "$STUBS/gh" <<EOF
#!/bin/sh
echo "gh \$*" >> "$WORK/gh.log"
case "\$1 \$2 \$3" in
    "attestation verify --help") [ "\$STUB_GH" = old ] && exit 1; exit 0 ;;
    "auth status "*) [ "\$STUB_GH" = noauth ] && exit 1; exit 0 ;;
    "attestation verify "*)
        [ "\$STUB_GH" = ok ] && { echo "Verification succeeded!"; exit 0; }
        echo "no attestations found for subject"; exit 1 ;;
esac
exit 2
EOF
chmod +x "$STUBS"/*
# By default a case sees no gh at all: GitHub's runners ship a real one.
NOGH="$STUBS/no-such-gh"

# ── One case ─────────────────────────────────────────────────────────────────
# Sets CASE (its folder), HOME_ (its home) and SYS (unwritable system dir).
fresh() {
    CASE="$WORK/case-$1"
    HOME_="$CASE/home"
    SYS="$CASE/usr-local-bin"
    mkdir -p "$HOME_" "$SYS"
    chmod 555 "$SYS"
}

# install [extra PATH before the defaults] [VAR=value ...]
install() {
    local front="$1"; shift
    OUT=$(cd "$CASE" && env -i HOME="$HOME_" PATH="$front$STUBS:/usr/bin:/bin" \
        _EVNX_SYSTEM_DIR="$SYS" _EVNX_GH="$NOGH" "$@" bash "$SCRIPT" 2>&1)
    CODE=$?
}

has() { case "$OUT" in *"$1"*) return 0 ;; esac; return 1; }

echo "== install.sh =="

# e, a: a fresh account installs into ~/.local/bin, created, with no sudo.
fresh e
install "$HOME_/.local/bin:"
if [ $CODE -eq 0 ] && [ -x "$HOME_/.local/bin/evnx" ] && ! has "sudo called"; then
    ok "e: a fresh account gets ~/.local/bin, created, no sudo"
else bad "e: fresh account"; fi
has "Installed evnx 0.10.18 to $HOME_/.local/bin/evnx" && ok "a: reports the version of the file it wrote" || bad "a: report"

# d: not on PATH is exit 0 with the export line.
fresh d
install ""
if [ $CODE -eq 0 ] && has 'export PATH="$HOME/.local/bin:$PATH"' && ! has "Installation failed"; then
    ok "d: not on PATH is exit 0 with the export line"
else bad "d: not on PATH"; fi

# a, b: an older npm copy ahead on PATH is named, with its removal command.
fresh b
mkdir -p "$CASE/npm/lib/node_modules/@evnx/cli/bin" "$CASE/npm/bin"
printf '#!/bin/sh\necho "evnx 0.10.0"\n' > "$CASE/npm/lib/node_modules/@evnx/cli/bin/evnx"
chmod +x "$CASE/npm/lib/node_modules/@evnx/cli/bin/evnx"
ln -s ../lib/node_modules/@evnx/cli/bin/evnx "$CASE/npm/bin/evnx"
install "$CASE/npm/bin:$HOME_/.local/bin:"
if [ $CODE -eq 0 ] && has "Installed evnx 0.10.18" && ! has "Installed evnx 0.10.0"; then
    ok "a: the shadowing copy's version is not reported as installed"
else bad "a: shadowed report"; fi
if has "$CASE/npm/bin/evnx (version 0.10.0)" && has "npm uninstall -g @evnx/cli"; then
    ok "b: the copy that wins on PATH is named, with npm's removal command"
else bad "b: shadow warning"; fi

# b, d: the new folder is not on PATH and another copy is. That copy may be
# the only working one, so it is named but never offered for removal (the
# live run of #233 told the owner to `rm ~/.local/bin/evnx`).
fresh bd
mkdir -p "$CASE/old/bin"
printf '#!/bin/sh\necho "evnx 0.10.18"\n' > "$CASE/old/bin/evnx"
chmod +x "$CASE/old/bin/evnx"
install "$CASE/old/bin:" EVNX_INSTALL_DIR="$CASE/new/bin"
if [ $CODE -eq 0 ] && has "still runs $CASE/old/bin/evnx (version 0.10.18)" \
    && has "export PATH=\"$CASE/new/bin:\$PATH\"" && ! has "Remove it" && ! has "rm $CASE"; then
    ok "b, d: not on PATH: the copy that runs is named, never offered for removal"
else bad "b, d: not on PATH with another copy"; fi

# c: a Homebrew link is never replaced.
fresh c-brew
mkdir -p "$CASE/brew/Cellar/evnx/0.10.0/bin" "$CASE/brew/bin"
printf '#!/bin/sh\necho "evnx 0.10.0"\n' > "$CASE/brew/Cellar/evnx/0.10.0/bin/evnx"
chmod +x "$CASE/brew/Cellar/evnx/0.10.0/bin/evnx"
ln -s ../Cellar/evnx/0.10.0/bin/evnx "$CASE/brew/bin/evnx"
install "" EVNX_INSTALL_DIR="$CASE/brew/bin"
if [ $CODE -eq 1 ] && has "brew upgrade evnx" && [ -L "$CASE/brew/bin/evnx" ] \
    && [ "$("$CASE/brew/bin/evnx")" = "evnx 0.10.0" ]; then
    ok "c: a Homebrew link is left alone; brew upgrade is named"
else bad "c: Homebrew"; fi

# c: a pip-installed copy is never replaced.
fresh c-pip
mkdir -p "$CASE/py/bin" "$CASE/py/lib/python3.12/site-packages/evnx-0.10.0.dist-info"
printf '#!/bin/sh\necho "evnx 0.10.0"\n' > "$CASE/py/bin/evnx"
chmod +x "$CASE/py/bin/evnx"
install "" EVNX_INSTALL_DIR="$CASE/py/bin"
if [ $CODE -eq 1 ] && has "pip install --upgrade evnx" && [ "$("$CASE/py/bin/evnx")" = "evnx 0.10.0" ]; then
    ok "c: a pip-owned copy is left alone; pip is named"
else bad "c: pip"; fi

# f: EVNX_VERSION pins a release, with no lookup of the latest.
fresh f-version
: > "$WORK/curl.log"
install "$HOME_/.local/bin:" EVNX_VERSION=0.10.15
if [ $CODE -eq 0 ] && has "Installed evnx 0.10.15" && ! grep -q "releases/latest" "$WORK/curl.log"; then
    ok "f: EVNX_VERSION=0.10.15 installs 0.10.15 without asking for the latest"
else bad "f: EVNX_VERSION"; fi

# f: EVNX_INSTALL_DIR chooses the folder, created if missing.
fresh f-dir
install "$CASE/tools/bin:" EVNX_INSTALL_DIR="$CASE/tools/bin"
if [ $CODE -eq 0 ] && [ -x "$CASE/tools/bin/evnx" ] && [ ! -e "$HOME_/.local/bin/evnx" ]; then
    ok "f: EVNX_INSTALL_DIR chooses the folder"
else bad "f: EVNX_INSTALL_DIR"; fi

# A writable system folder is still the first choice.
fresh system
chmod 755 "$SYS"
install "$SYS:"
if [ $CODE -eq 0 ] && [ -x "$SYS/evnx" ] && [ ! -e "$HOME_/.local/bin" ]; then
    ok "a writable /usr/local/bin is still preferred"
else bad "system dir"; fi

# Unchanged: a checksum mismatch installs nothing.
fresh checksum
cp "$RELEASES/v0.10.18/$ASSET.sha256" "$WORK/good.sha256"
sed 's/^./0/' "$WORK/good.sha256" > "$RELEASES/v0.10.18/$ASSET.sha256"
install "$HOME_/.local/bin:"
cp "$WORK/good.sha256" "$RELEASES/v0.10.18/$ASSET.sha256"
if [ $CODE -eq 1 ] && has "CHECKSUM MISMATCH" && [ ! -e "$HOME_/.local/bin/evnx" ]; then
    ok "a checksum mismatch installs nothing"
else bad "checksum"; fi

# RM-83: provenance is checked when gh can, from v0.11.0 on.
fresh p-ok
: > "$WORK/gh.log"
install "$HOME_/.local/bin:" EVNX_VERSION=0.11.0 _EVNX_GH="$STUBS/gh" STUB_GH=ok
if [ $CODE -eq 0 ] && has "Provenance verified" && [ -x "$HOME_/.local/bin/evnx" ] \
    && grep -q -- "--signer-workflow urwithajit9/evnx/.github/workflows/release.yml" "$WORK/gh.log" \
    && grep -q -- "--source-ref refs/tags/v0.11.0" "$WORK/gh.log"; then
    ok "RM-83: a verified archive installs; the check names release.yml and the tag"
else bad "RM-83: verified"; fi

fresh p-fail
install "$HOME_/.local/bin:" EVNX_VERSION=0.11.0 _EVNX_GH="$STUBS/gh" STUB_GH=fail
if [ $CODE -eq 1 ] && has "PROVENANCE CHECK FAILED" && has "no attestations found" \
    && [ ! -e "$HOME_/.local/bin/evnx" ]; then
    ok "RM-83: a failed provenance check installs nothing"
else bad "RM-83: failed check"; fi

fresh p-nogh
install "$HOME_/.local/bin:" EVNX_VERSION=0.11.0
if [ $CODE -eq 0 ] && has "Provenance not checked: the GitHub CLI (gh) is not installed" \
    && has "gh attestation verify" && [ -x "$HOME_/.local/bin/evnx" ]; then
    ok "RM-83: no gh is one line, with the command to check by hand"
else bad "RM-83: no gh"; fi

fresh p-noauth
install "$HOME_/.local/bin:" EVNX_VERSION=0.11.0 _EVNX_GH="$STUBS/gh" STUB_GH=noauth
if [ $CODE -eq 0 ] && has "gh is not signed in" && ! has "PROVENANCE CHECK FAILED" \
    && [ -x "$HOME_/.local/bin/evnx" ]; then
    ok "RM-83: gh not signed in is 'not checked', not a failure"
else bad "RM-83: gh not signed in"; fi

fresh p-old
install "$HOME_/.local/bin:" EVNX_VERSION=0.11.0 _EVNX_GH="$STUBS/gh" STUB_GH=old
if [ $CODE -eq 0 ] && has "no 'attestation' command" && [ -x "$HOME_/.local/bin/evnx" ]; then
    ok "RM-83: a gh without 'attestation' is 'not checked'"
else bad "RM-83: old gh"; fi

fresh p-pre011
: > "$WORK/gh.log"
install "$HOME_/.local/bin:" EVNX_VERSION=0.10.15 _EVNX_GH="$STUBS/gh" STUB_GH=fail
if [ $CODE -eq 0 ] && has "releases before v0.11.0 carry no attestation" \
    && [ ! -s "$WORK/gh.log" ] && [ -x "$HOME_/.local/bin/evnx" ]; then
    ok "RM-83: a release before v0.11.0 is never checked, so never refused"
else bad "RM-83: pre-0.11.0"; fi

echo
echo "== $pass ok, $fail failed =="
[ "$fail" -eq 0 ]
