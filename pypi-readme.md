# evnx

> ⚡ A blazing-fast environment variable manager and inspector — built in Rust.

[![Crates.io](https://img.shields.io/crates/v/evnx.svg)](https://crates.io/crates/evnx)
[![PyPI version](https://img.shields.io/pypi/v/evnx.svg)](https://pypi.org/project/evnx/)
[![PyPI - Python Version](https://img.shields.io/pypi/pyversions/evnx)](https://pypi.org/project/evnx/)
[![PyPI Downloads](https://img.shields.io/pypi/dm/evnx)](https://pypi.org/project/evnx/)
[![CI](https://github.com/urwithajit9/evnx/actions/workflows/ci.yml/badge.svg)](https://github.com/urwithajit9/evnx/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## What is evnx?

`evnx` is a developer-focused CLI tool for managing, inspecting, and validating environment variables across projects and shells.

<!-- TODO: Add a short GIF or screenshot here — it dramatically increases installs -->
<!-- ![evnx demo](docs/demo.gif) -->

**Key features:**

- 🔍 Inspect, search, and diff environment variables across `.env` files and the live shell environment
- ✅ Validate required variables are present before running a service
- 🚀 Zero runtime dependencies — single static binary
- 🦀 Written in Rust — fast startup, low memory footprint
- 🖥️ Works on Linux, macOS, and Windows

---

## Installation

### Via pip (Python ecosystem — recommended for teams using Python)

Works with Python 3.8+. Installs a pre-built native binary — **no Rust toolchain needed**.

```bash
pip install evnx
```

For CLI tools, `pipx` gives you an isolated environment (recommended):

```bash
pipx install evnx
```

Verify:

```bash
evnx --version
evnx --help
```

---

### Via cargo (Rust ecosystem)

```bash
cargo install evnx
```

---

### Via curl script (Linux / macOS — no runtime required)

```bash
curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash
```

---

### Pre-built binaries

Download from the [GitHub Releases page](https://github.com/urwithajit9/evnx/releases/latest):

| Platform      | Architecture | Download |
|---------------|--------------|----------|
| Linux         | x86_64       | `evnx-x86_64-unknown-linux-gnu.tar.gz` |
| Linux         | ARM64        | `evnx-aarch64-unknown-linux-gnu.tar.gz` |
| macOS         | Intel        | `evnx-x86_64-apple-darwin.tar.gz` |
| macOS         | Apple Silicon | `evnx-aarch64-apple-darwin.tar.gz` |
| Windows       | x86_64       | `evnx-x86_64-pc-windows-msvc.zip` |

Verify the checksum after download:

```bash
sha256sum -c evnx-x86_64-unknown-linux-gnu.tar.gz.sha256
```

---

## Quick Start

```bash
# List all environment variables in the current shell
evnx list

# Search for a variable by name
evnx get DATABASE_URL

# Load a .env file and show what variables it would set
evnx inspect .env

# Check that required variables are set (useful in CI/CD)
evnx check DATABASE_URL SECRET_KEY PORT

# Diff two .env files
evnx diff .env .env.production

# Export variables from a .env file to the current shell
eval $(evnx export .env)
```

---

## Usage

```
evnx [COMMAND] [OPTIONS]

COMMANDS:
  list        List all environment variables in the current session
  get         Get the value of a specific variable
  inspect     Parse and display a .env file
  check       Assert that required variables are present (exits 1 if not)
  diff        Compare two .env files
  export      Emit shell-compatible export statements from a .env file
  help        Print help for a command

OPTIONS:
  -h, --help       Print help
  -V, --version    Print version
  -q, --quiet      Suppress output (useful for scripts)
  --color <WHEN>   Control color output: auto, always, never [default: auto]
```

---

## Why a Rust binary distributed via PyPI?

Python developers typically reach for `pip install` to get CLI tools. By publishing `evnx` to PyPI alongside Cargo and the curl script:

- **No Rust knowledge required** — `pip install evnx` just works
- **Works in CI/CD** — any environment with Python (which is nearly all of them) can install it in one line
- **No dynamic linking issues** — wheels ship a statically-linked binary
- **Version pinned** — `pip install evnx==0.2.1` is repeatable

---

## Development

### Prerequisites

- Rust stable (`rustup update stable`)
- Python 3.8+ (for building/testing the PyPI package)

### Build from source

```bash
git clone https://github.com/urwithajit9/evnx.git
cd evnx

# Rust binary
cargo build --release
./target/release/evnx --help

# Build the pip wheel locally (requires maturin)
pip install maturin
maturin develop --release
evnx --help
```

### Run tests

```bash
cargo test --all-features
cargo clippy -- -D warnings
cargo fmt --check
```

---

## Contributing

Contributions are welcome! Please open an issue first for significant changes.

1. Fork the repository
2. Create a feature branch: `git checkout -b feat/my-feature`
3. Commit your changes: `git commit -m 'feat: add my feature'`
4. Push and open a Pull Request

Please follow [Conventional Commits](https://www.conventionalcommits.org/) for commit messages.

---

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for a full history of changes.

---

## License

MIT © [urwithajit9](https://github.com/urwithajit9)