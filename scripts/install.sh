#!/usr/bin/env bash
# evnx (formely dotenv-space-cli) installer
# Usage: curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash

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
info "Fetching latest release..."
# `|| true` so a network failure leaves LATEST empty and falls through to the
# fallback below, rather than aborting here under `set -e` with no message.
LATEST=$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
    "https://github.com/$REPO/releases/latest" 2>/dev/null | sed 's#.*/tag/##') || true

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
    *) error "Could not resolve the latest release (got: '${LATEST:-empty}')" ;;
esac

info "Latest version: $LATEST"

# Download URLs
URL="https://github.com/$REPO/releases/download/$LATEST/${BINARY_NAME}-${TARGET}.tar.gz"
CHECKSUM_URL="https://github.com/$REPO/releases/download/$LATEST/${BINARY_NAME}-${TARGET}.tar.gz.sha256"

# Create temp directory
TMP_DIR=$(mktemp -d)
trap "rm -rf $TMP_DIR" EXIT

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

# Determine install directory
if [ -w "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
    SUDO=""
elif [ -w "$HOME/.local/bin" ]; then
    INSTALL_DIR="$HOME/.local/bin"
    SUDO=""
    mkdir -p "$INSTALL_DIR"
else
    INSTALL_DIR="/usr/local/bin"
    SUDO="sudo"
    warn "Need sudo permission to install to $INSTALL_DIR"
fi

# Install
info "Installing to $INSTALL_DIR..."
$SUDO mv "${BINARY_NAME}-${TARGET}" "$INSTALL_DIR/$BINARY_NAME"
$SUDO chmod +x "$INSTALL_DIR/$BINARY_NAME"

# Verify installation
if command -v evnx >/dev/null 2>&1; then
    VERSION=$(evnx --version | awk '{print $2}')
    echo
    info "✓ Installation successful!"
    info "Installed version: $VERSION"
    echo
    echo "Quick start:"
    echo "  evnx init          # Create .env.example"
    echo "  evnx validate      # Check for issues"
    echo "  evnx scan          # Detect secrets"
    echo "  evnx --help        # See all commands"
    echo
else
    error "Installation failed - binary not found in PATH"
fi

# Check if install dir is in PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]] && [ "$INSTALL_DIR" = "$HOME/.local/bin" ]; then
    warn "$INSTALL_DIR is not in your PATH"
    echo "Add to your shell profile (.bashrc, .zshrc, etc.):"
    echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
fi