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
    # From v0.15.0 the archive also carries completions/ (WP-16).
    local members="evnx-x86_64-unknown-linux-gnu"
    if [ "$1" = "0.15.0" ]; then
        mkdir -p "$dir/build/completions"
        echo "# bash completions for evnx" > "$dir/build/completions/evnx.bash"
        echo "#compdef evnx" > "$dir/build/completions/_evnx"
        echo "# fish completions for evnx" > "$dir/build/completions/evnx.fish"
        members="$members completions"
    fi
    # shellcheck disable=SC2086  # a short list of names without spaces
    tar -czf "$dir/$ASSET" -C "$dir/build" $members
    (cd "$dir" && if command -v sha256sum >/dev/null; then sha256sum "$ASSET"; else shasum -a 256 "$ASSET"; fi > "$ASSET.sha256")
}
release 0.10.18
release 0.10.15
release 0.11.0
release 0.15.0

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
# Always a different first digit: `s/^./0/` changed nothing when the hash of the
# freshly built fake archive already began with 0 (1 run in 16 flaked).
sed 's/^0/1/;t;s/^./0/' "$WORK/good.sha256" > "$RELEASES/v0.10.18/$ASSET.sha256"
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

# WP-16: not on PATH prints "use now" and "keep" lines, and edits nothing
# without consent. HOME is the case's own, so a profile written here is checked
# in the case folder.
fresh modify-off
install "" SHELL=/bin/zsh
if [ $CODE -eq 0 ] && has "Use it now" && has "Keep it" \
    && has 'export PATH="$HOME/.local/bin:$PATH"' && has "$HOME_/.zshrc" \
    && has "Nothing was changed" && [ ! -e "$HOME_/.zshrc" ]; then
    ok "modify_path_off_prints_two_lines_edits_nothing"
else bad "modify_path_off_prints_two_lines_edits_nothing"; fi

fresh modify-on
install "" SHELL=/bin/zsh EVNX_MODIFY_PATH=1
if [ $CODE -eq 0 ] && [ "$(grep -c 'export PATH="$HOME/.local/bin:$PATH"' "$HOME_/.zshrc")" = 1 ] \
    && has "Use it now" && has "added to $HOME_/.zshrc" && ! has "Nothing was changed"; then
    ok "modify_path_on_appends_once"
else bad "modify_path_on_appends_once"; fi
# A second run finds the exact line and writes nothing more.
install "" SHELL=/bin/zsh EVNX_MODIFY_PATH=1
if [ $CODE -eq 0 ] && [ "$(grep -c 'export PATH=' "$HOME_/.zshrc")" = 1 ] && has "already has that line"; then
    ok "modify_path_second_run_no_duplicate"
else bad "modify_path_second_run_no_duplicate"; fi

# Content already in the profile is kept; the line goes at the end.
fresh modify-keeps
printf 'alias ll="ls -l"\n' > "$HOME_/.zshrc"
install "" SHELL=/bin/zsh EVNX_MODIFY_PATH=1
if [ "$(head -1 "$HOME_/.zshrc")" = 'alias ll="ls -l"' ] && grep -q 'export PATH=' "$HOME_/.zshrc"; then
    ok "modify_path_keeps_the_existing_profile"
else bad "modify_path_keeps_the_existing_profile"; fi

fresh modify-bash
install "" SHELL=/bin/bash EVNX_MODIFY_PATH=1
bash_ok=false
[ -f "$HOME_/.bashrc" ] && grep -q 'export PATH="$HOME/.local/bin:$PATH"' "$HOME_/.bashrc" && [ ! -e "$HOME_/.zshrc" ] && bash_ok=true
fresh modify-fish
install "" SHELL=/usr/bin/fish EVNX_MODIFY_PATH=1
if $bash_ok && [ -f "$HOME_/.config/fish/config.fish" ] \
    && grep -q 'set -gx PATH "$HOME/.local/bin" $PATH' "$HOME_/.config/fish/config.fish" \
    && ! grep -q 'export PATH' "$HOME_/.config/fish/config.fish"; then
    ok "modify_path_zsh_bash_fish_targets"
else bad "modify_path_zsh_bash_fish_targets"; fi

# An unknown shell is told what to do and nothing is written, even with the variable.
fresh modify-other
install "" SHELL=/bin/tcsh EVNX_MODIFY_PATH=1
if [ $CODE -eq 0 ] && has "Use it now" && has "nothing was offered" && [ -z "$(ls -A "$HOME_" | grep -v '^\.local$\|^\.config$')" ]; then
    ok "modify_path_unknown_shell_writes_nothing"
else bad "modify_path_unknown_shell_writes_nothing"; fi

# A terminal answer: y edits, anything else does not. `_EVNX_TTY` stands in for /dev/tty.
fresh tty-yes
echo y > "$CASE/answer"
install "" SHELL=/bin/zsh _EVNX_TTY="$CASE/answer"
yes_ok=false
[ -f "$HOME_/.zshrc" ] && grep -q 'export PATH=' "$HOME_/.zshrc" && yes_ok=true
fresh tty-no
echo n > "$CASE/answer"
install "" SHELL=/bin/zsh _EVNX_TTY="$CASE/answer"
if $yes_ok && [ ! -e "$HOME_/.zshrc" ] && has "Nothing was changed"; then
    ok "modify_path_terminal_answer_decides"
else bad "modify_path_terminal_answer_decides"; fi

# EVNX_MODIFY_PATH=0 never asks, even with a terminal answer on offer.
fresh modify-zero
echo y > "$CASE/answer"
install "" SHELL=/bin/zsh EVNX_MODIFY_PATH=0 _EVNX_TTY="$CASE/answer"
if [ ! -e "$HOME_/.zshrc" ]; then ok "modify_path_zero_means_no"; else bad "modify_path_zero_means_no"; fi

# On PATH already: no PATH lines at all.
fresh on-path
install "$HOME_/.local/bin:" SHELL=/bin/zsh EVNX_MODIFY_PATH=1
if [ $CODE -eq 0 ] && ! has "Use it now" && ! has "Keep it" && [ ! -e "$HOME_/.zshrc" ]; then
    ok "modify_path_on_path_says_nothing"
else bad "modify_path_on_path_says_nothing"; fi

# CMP-3: completions are copied only into folders that already exist.
fresh comp-some
mkdir -p "$HOME_/.local/share/bash-completion/completions"
install "$HOME_/.local/bin:" EVNX_VERSION=0.15.0
if [ $CODE -eq 0 ] && [ -f "$HOME_/.local/share/bash-completion/completions/evnx" ] \
    && [ ! -e "$HOME_/.local/share/zsh" ] && [ ! -e "$HOME_/.config/fish" ] \
    && has "Installed bash completions" && ! has "zsh completions"; then
    ok "completions_copied_only_into_existing_dirs"
else bad "completions_copied_only_into_existing_dirs"; fi

fresh comp-all
mkdir -p "$HOME_/.local/share/bash-completion/completions" "$HOME_/.local/share/zsh/site-functions" "$HOME_/.config/fish/completions"
install "$HOME_/.local/bin:" EVNX_VERSION=0.15.0
if [ $CODE -eq 0 ] && [ -f "$HOME_/.local/share/zsh/site-functions/_evnx" ] \
    && [ -f "$HOME_/.config/fish/completions/evnx.fish" ] && [ -x "$HOME_/.local/bin/evnx" ]; then
    ok "completions_all_three_shells_when_their_folders_exist"
else bad "completions_all_three_shells_when_their_folders_exist"; fi

# An archive from before 0.15.0 has no completions/: the install is unaffected.
fresh comp-old
mkdir -p "$HOME_/.local/share/bash-completion/completions"
install "$HOME_/.local/bin:" EVNX_VERSION=0.11.0
if [ $CODE -eq 0 ] && [ -x "$HOME_/.local/bin/evnx" ] \
    && [ ! -e "$HOME_/.local/share/bash-completion/completions/evnx" ] && ! has "completions"; then
    ok "completions_absent_from_an_old_archive_is_fine"
else bad "completions_absent_from_an_old_archive_is_fine"; fi

# The attestation check still runs on the new archive layout.
fresh att-new
: > "$WORK/gh.log"
install "$HOME_/.local/bin:" EVNX_VERSION=0.15.0 _EVNX_GH="$STUBS/gh" STUB_GH=ok
if [ $CODE -eq 0 ] && has "Provenance verified" && grep -q -- "--source-ref refs/tags/v0.15.0" "$WORK/gh.log"; then
    ok "attestation_still_runs_when_gh_present"
else bad "attestation_still_runs_when_gh_present"; fi

echo
echo "== $pass ok, $fail failed =="
[ "$fail" -eq 0 ]
