#!/usr/bin/env bash
# evnx (formerly dotenv-space-cli) installer
# Usage: curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash
#
# Environment (UP-11f):
#   EVNX_VERSION=0.10.18        install this release instead of the latest
#   EVNX_INSTALL_DIR=/opt/bin   install into this folder instead of the default
#
# ⚠️ This file goes live on merge to `main`: dotenv.space/install.sh proxies it,
# cached for up to an hour. Review a change to it like a release.
# Tests: tests/install/run.sh (stubbed curl, uname, ldd; no network).

# ⚠️ `-o pipefail` is not free on this script and two pipelines had to be
# changed for it — see the musl check and the release lookup below. Both
# relied on a command in the pipeline failing while the pipeline succeeded,
# which is exactly what pipefail stops tolerating. Adding the flag without
# reading them would have broken Alpine installs.
set -euo pipefail

REPO="urwithajit9/evnx"
BINARY_NAME="evnx"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

info "Installing evnx..."

# Detect OS and Architecture
OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
    Linux)
        # Try to detect musl vs glibc
        # ⚠️ `|| true` is load-bearing under `set -o pipefail`. musl's ldd
        # prints its version to stderr and **exits 1** — that is why `2>&1` is
        # here at all. With pipefail and without this, the pipeline inherits
        # ldd's failure even though grep matched, musl goes undetected, and
        # Alpine users are handed the glibc binary, which does not run.
        if { ldd --version 2>&1 || true; } | grep -q musl; then
            OS="unknown-linux-musl"
            info "Detected musl-based Linux"
        else
            OS="unknown-linux-gnu"
            info "Detected GNU/Linux"
        fi
        ;;
    Darwin)
        OS="apple-darwin"
        info "Detected macOS"
        ;;
    *)
        error "Unsupported OS: $OS"
        ;;
esac

case "$ARCH" in
    x86_64)
        ARCH="x86_64"
        ;;
    arm64|aarch64)
        ARCH="aarch64"
        ;;
    *)
        error "Unsupported architecture: $ARCH"
        ;;
esac

TARGET="${ARCH}-${OS}"
info "Target: $TARGET"

# Get latest version
#
# Resolved from the /releases/latest redirect rather than the JSON API, for two
# reasons:
#
#   1. The API is rate-limited to 60 requests/hour per IP when unauthenticated,
#      which CI runners on shared egress hit routinely.
#   2. The previous parser broke outright. It ran
#        grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/'
#      which relies on the response being pretty-printed, one field per line.
#      GitHub now returns minified JSON, so grep matched the whole 27 KB body and
#      sed's greedy .* captured the LAST quoted string in the document —
#      producing LATEST="mentions_count" and a 404 on every download.
#
# The API call is kept as a fallback, with position-independent parsing that works
# on minified and pretty-printed JSON alike.
if [ -n "${EVNX_VERSION:-}" ]; then
    # A pinned release (UP-11f): no lookup. `v0.10.18` and `0.10.18` both work.
    LATEST="v${EVNX_VERSION#v}"
    info "Pinned by EVNX_VERSION"
else
    info "Fetching latest release..."
    # `|| true` so a network failure leaves LATEST empty and falls through to
    # the fallback below, rather than aborting here under `set -e` with no
    # message.
    LATEST=$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
        "https://github.com/$REPO/releases/latest" 2>/dev/null | sed 's#.*/tag/##') || true
fi

if [ -z "$LATEST" ] || [ "$LATEST" = "https://github.com/$REPO/releases" ]; then
    # ⓘ `grep | head -1` is left exactly as it was. The obvious pipefail worry
    # is that `head` closes the pipe and grep dies of SIGPIPE — measured
    # against a 655 KB grep output, ten times the pipe buffer, and it does not
    # happen. Swapping in `grep -m1` to "fix" it is actively wrong: this JSON
    # is one line, `-m1` stops after the first matching LINE, and `-o` then
    # still prints every match on it — the parsed tag became the LAST
    # tag_name in the body instead of the first.
    LATEST=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
        | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' \
        | head -1 \
        | sed -E 's/.*"([^"]+)"$/\1/') || true
fi

# A tag must look like a version. Without this the script happily builds a URL
# from whatever junk it parsed and fails later with a confusing 404.
case "$LATEST" in
    v[0-9]*) ;;
    *) error "Could not resolve the release to install (got: '${LATEST:-empty}')" ;;
esac

info "Latest version: $LATEST"

# Download URLs
URL="https://github.com/$REPO/releases/download/$LATEST/${BINARY_NAME}-${TARGET}.tar.gz"
CHECKSUM_URL="https://github.com/$REPO/releases/download/$LATEST/${BINARY_NAME}-${TARGET}.tar.gz.sha256"

# Create temp directory
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

cd "$TMP_DIR"

# Download binary
ARCHIVE_NAME="${BINARY_NAME}-${TARGET}.tar.gz"

info "Downloading from $URL"
if ! curl -fsSL "$URL" -o "$ARCHIVE_NAME"; then
    error "Failed to download binary"
fi

# ── Verify the checksum. Mandatory, and fatal when it fails ──────────────────
#
# ⚠️ This block used to be able to skip verification in three separate ways and
# say nothing about any of them:
#
#   1. A missing or 404ing checksum file: the whole `if` was false, and the
#      install continued unverified.
#   2. No `sha256sum` on the box — which is **every macOS machine**, where the
#      tool is `shasum -a 256`. There was no `else`, so macOS never verified a
#      download, ever.
#   3. On mismatch it asked "Continue anyway?" with `read -p`. ⛔ The documented
#      way to run this script is `curl -sSL … | bash`, so the script itself is
#      on stdin — `read` consumes the script's own remaining text rather than
#      waiting for a human. The prompt could not work in the path the README
#      publishes, and under `set -u` an unset $REPLY aborts on top of that.
#
# A checksum that is optional, unavailable on macOS, and overridable by a
# prompt nobody can answer is not a check. All three paths now stop the install.
if ! curl -fsSL "$CHECKSUM_URL" -o checksum.sha256; then
    error "Could not download the checksum for $ARCHIVE_NAME.
       Expected it at: $CHECKSUM_URL
       Refusing to install a binary that cannot be verified."
fi

if command -v sha256sum >/dev/null 2>&1; then
    SHA_VERIFY="sha256sum -c"
elif command -v shasum >/dev/null 2>&1; then
    SHA_VERIFY="shasum -a 256 -c"
else
    error "Neither sha256sum nor shasum is available, so the download cannot be
       verified. Install coreutils, or download the release manually from
       https://github.com/$REPO/releases/tag/$LATEST"
fi

info "Verifying checksum..."
if ! $SHA_VERIFY checksum.sha256; then
    error "CHECKSUM MISMATCH for $ARCHIVE_NAME.
       The download does not match what the release published. This is either
       a corrupted transfer or a tampered file — do not run it.
       Nothing was installed."
fi
info "Checksum verified ✓"

info "Extracting..."
tar -xzf "$ARCHIVE_NAME"

# ── Where to install ─────────────────────────────────────────────────────────
#
# EVNX_INSTALL_DIR if given; else /usr/local/bin when writable; else
# ~/.local/bin, created if missing (UP-11e, DEC-UP5).
#
# ⚠️ ~/.local/bin used to be chosen only when it already existed, so a fresh
# account fell through to `sudo` for /usr/local/bin. A user-local install
# needs no privilege. sudo is now used only for a folder named on purpose
# with EVNX_INSTALL_DIR, and says so.
#
# _EVNX_SYSTEM_DIR exists for tests/install/run.sh, so no test can write to the
# real /usr/local/bin. Not documented; not for users.
SYSTEM_DIR="${_EVNX_SYSTEM_DIR:-/usr/local/bin}"
LOCAL_DIR="$HOME/.local/bin"
SUDO=""
if [ -n "${EVNX_INSTALL_DIR:-}" ]; then
    INSTALL_DIR="${EVNX_INSTALL_DIR%/}"
    mkdir -p "$INSTALL_DIR" 2>/dev/null || true
    if [ ! -w "$INSTALL_DIR" ]; then
        SUDO="sudo"
        warn "$INSTALL_DIR is not writable (EVNX_INSTALL_DIR): using sudo"
        $SUDO mkdir -p "$INSTALL_DIR"
    fi
elif [ -w "$SYSTEM_DIR" ]; then
    INSTALL_DIR="$SYSTEM_DIR"
else
    INSTALL_DIR="$LOCAL_DIR"
    mkdir -p "$INSTALL_DIR" || error "Could not create $INSTALL_DIR.
       Choose another folder: EVNX_INSTALL_DIR=/path/to/bin"
fi
TARGET_FILE="$INSTALL_DIR/$BINARY_NAME"

# A pip record for the copy in bin folder $1: `pyvenv.cfg` one level up, or an
# `evnx-<version>.dist-info` in that prefix's site-packages or dist-packages.
# The same rule as `evnx update` (src/commands/update/channel.rs).
pip_owns() {
    local prefix record
    prefix=$(dirname "$1")
    [ -f "$prefix/pyvenv.cfg" ] && return 0
    for record in "$prefix"/lib/python*/site-packages/evnx-*.dist-info \
                  "$prefix"/lib/python*/dist-packages/evnx-*.dist-info; do
        [ -d "$record" ] && return 0
    done
    return 1
}

# $1 with every symlink followed. A loop, not `readlink -f`, which macOS
# before 12.3 does not have.
resolved() {
    local p="$1" link
    while [ -L "$p" ]; do
        link=$(readlink "$p")
        case "$link" in
            /*) p="$link" ;;
            *) p="$(dirname "$p")/$link" ;;
        esac
    done
    echo "$p"
}

# The command that removes the copy at $1, from where it lives (UP-11b).
removal_hint() {
    local real
    real=$(resolved "$1")
    case "$real" in
        */Cellar/*|*/homebrew/*|*/linuxbrew/*) echo "brew uninstall evnx" ;;
        */node_modules/*|*/npm/*|*/.npm-global/*) echo "npm uninstall -g @evnx/cli" ;;
        */pipx/*) echo "pipx uninstall evnx" ;;
        */.cargo/bin/*|*/cargo/bin/*) echo "cargo uninstall evnx" ;;
        *)
            if pip_owns "$(dirname "$1")"; then
                echo "pip uninstall evnx"
            else
                echo "rm $1"
            fi
            ;;
    esac
}

# ── Never replace a copy another package manager owns (UP-11c) ───────────────
#
# On Intel macOS /usr/local/bin is writable, and Homebrew's `evnx` there is a
# link into Cellar: replacing it left brew believing it still managed a file it
# no longer did. The same for npm's link and a pip install's script. That
# manager upgrades its own copy.
if [ -e "$TARGET_FILE" ] || [ -L "$TARGET_FILE" ]; then
    case "$(resolved "$TARGET_FILE")" in
        */Cellar/*) OWNER="Homebrew"; UPGRADE="brew upgrade evnx" ;;
        */node_modules/*) OWNER="npm"; UPGRADE="npm install -g @evnx/cli@latest" ;;
        *)
            if pip_owns "$INSTALL_DIR"; then
                OWNER="pip"; UPGRADE="pip install --upgrade evnx  (or: pipx upgrade evnx)"
            else
                OWNER=""
            fi
            ;;
    esac
    if [ -n "$OWNER" ]; then
        error "$TARGET_FILE belongs to $OWNER, so this script will not replace it.
       Upgrade it there:  $UPGRADE
       Or install beside it:  EVNX_INSTALL_DIR=\$HOME/.local/bin (then remove one copy).
       Nothing was installed."
    fi
fi

info "Installing to $INSTALL_DIR..."
$SUDO mv "${BINARY_NAME}-${TARGET}" "$TARGET_FILE"
$SUDO chmod +x "$TARGET_FILE"

# ── Report the copy that was installed (UP-11a) ──────────────────────────────
#
# ⚠️ Not `evnx --version`: that is whichever copy is first on PATH, and with an
# older npm or pip copy ahead of this folder it printed *that* version as
# "Installed version". The install had worked; the report described another
# file.
VERSION=$("$TARGET_FILE" --version | awk '{print $2}')
echo
info "✓ Installed evnx $VERSION to $TARGET_FILE"

# ── Which copy runs when you type `evnx` (UP-11b, UP-11d) ────────────────────
FIRST=$(command -v "$BINARY_NAME" 2>/dev/null || true)
ON_PATH=true
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) ON_PATH=false ;;
esac

OTHER=""
if [ -n "$FIRST" ] && ! [ "$FIRST" -ef "$TARGET_FILE" ]; then
    OTHER=$("$FIRST" --version 2>/dev/null | awk '{print $2}' || true)
    OTHER="$FIRST (version ${OTHER:-unknown})"
fi

if [ "$ON_PATH" = false ]; then
    # ⚠️ Exit 0: the install worked. This used to end "Installation failed",
    # and the hint below it was never reached (UP-11d).
    #
    # ⚠️ No removal hint here. The copy on PATH may be the only working one,
    # and `export PATH="<dir>:$PATH"` puts this folder first anyway. Advising
    # `rm` on it left the user with no `evnx` at all.
    if [ "$INSTALL_DIR" = "$LOCAL_DIR" ]; then
        # shellcheck disable=SC2016  # written for the profile, unexpanded
        LINE='export PATH="$HOME/.local/bin:$PATH"'
    else
        LINE="export PATH=\"$INSTALL_DIR:\$PATH\""
    fi
    if [ -n "$OTHER" ]; then
        NOW="so \`evnx\` still runs $OTHER.
       To use this copy, add"
    else
        NOW="so \`evnx\` is not found yet.
       Add"
    fi
    warn "$INSTALL_DIR is not on your PATH, $NOW this line to your shell profile
       (~/.bashrc, ~/.zshrc, …), then open a new terminal:
         $LINE"
elif [ -n "$OTHER" ]; then
    # Both on PATH and the other wins: that copy is the one to remove.
    warn "Another evnx comes first on your PATH, so typing \`evnx\` runs it, not this one:
       $OTHER
       Remove it:  $(removal_hint "$FIRST")
       Then check: type -a evnx"
fi

echo
echo "Quick start:"
echo "  evnx init          # Create .env.example"
echo "  evnx validate      # Check for issues"
echo "  evnx scan          # Detect secrets"
echo "  evnx --help        # See all commands"
echo
