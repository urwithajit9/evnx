#!/usr/bin/env bash
# dotenv-space installer
# Usage: curl -sSL https://raw.githubusercontent.com/urwithajit9/dotenv-space-cli/main/scripts/install.sh | bash

set -e

REPO="urwithajit9/dotenv-space-cli"
BINARY_NAME="dotenv-space"

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

info "Installing dotenv-space..."

# Detect OS and Architecture
OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
    Linux)
        # Try to detect musl vs glibc
        if ldd --version 2>&1 | grep -q musl; then
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
info "Fetching latest release..."
LATEST=$(curl -fsSL https://api.github.com/repos/$REPO/releases/latest | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$LATEST" ]; then
    error "Failed to fetch latest version"
fi

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

# Verify checksum
if curl -fsSL "$CHECKSUM_URL" -o checksum.sha256 2>/dev/null; then
    info "Verifying checksum..."
    if command -v sha256sum >/dev/null 2>&1; then
        if ! sha256sum -c checksum.sha256; then
            warn "Checksum verification failed"
            read -p "Continue anyway? (y/N) " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                error "Installation aborted"
            fi
        else
            info "Checksum verified ✓"
        fi
    fi
fi

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
if command -v dotenv-space >/dev/null 2>&1; then
    VERSION=$(dotenv-space --version | awk '{print $2}')
    echo
    info "✓ Installation successful!"
    info "Installed version: $VERSION"
    echo
    echo "Quick start:"
    echo "  dotenv-space init          # Create .env.example"
    echo "  dotenv-space validate      # Check for issues"
    echo "  dotenv-space scan          # Detect secrets"
    echo "  dotenv-space --help        # See all commands"
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