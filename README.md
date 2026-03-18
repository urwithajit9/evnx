# evnx

[![CI](https://github.com/urwithajit9/evnx/workflows/CI/badge.svg)](https://github.com/urwithajit9/evnx/actions)
[![Release](https://img.shields.io/github/v/release/urwithajit9/evnx)](https://github.com/urwithajit9/evnx/releases)
[![crates.io](https://img.shields.io/crates/v/evnx.svg)](https://crates.io/crates/evnx)
[![PyPI](https://img.shields.io/pypi/v/evnx.svg)](https://pypi.org/project/evnx/)
[![npm](https://img.shields.io/npm/v/@evnx/cli.svg)](https://www.npmjs.com/package/@evnx/cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A CLI tool for managing `.env` files — validation, secret scanning, format conversion, and migration to cloud secret managers.

[Website](https://www.evnx.dev) | [Getting Started](./docs/GETTING_STARTED.md) | [Changelog](./CHANGELOG.md)

---

## Why evnx?

Accidentally committing secrets to version control is one of the most common and costly developer mistakes. evnx is a local-first tool that catches misconfigurations, detects credential leaks, and converts environment files to the format each deployment target expects — before anything reaches CI or production.

---

## Installation

### Linux / macOS

```bash
curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash
```

### Homebrew (macOS and Linux)

```bash
brew install urwithajit9/evnx/evnx
```

### npm

```bash
npm install -g @evnx/cli
```

### pipx (recommended for Python environments)
```bash
pipx install evnx
```

pipx installs CLI tools into isolated environments and wires them to your
system PATH automatically. It is the correct tool for installing Python-
distributed CLI binaries like evnx.

**Don't have pipx?**

**macOS**
```bash
brew install pipx
pipx ensurepath
```

**Ubuntu / Debian (Python 3.11+)**
```bash
sudo apt install pipx
pipx ensurepath
```
On older Ubuntu (20.04 and below) where `pipx` is not in apt:
```bash
pip install --user pipx
python -m pipx ensurepath
```
Note: `pip install evnx` will fail on Ubuntu 22.04+ with an "externally managed
environment" error (PEP 668). This is intentional — Ubuntu protects the system
Python. Use pipx instead.

**Windows**
```powershell
python -m pip install --user pipx
python -m pipx ensurepath
```
After running `ensurepath`, close and reopen your terminal (a full logout/login
may be required for PATH changes to take effect), then:
```powershell
pipx install evnx
```

After installing pipx on any platform, restart your terminal and run:
```bash
pipx install evnx
evnx --version
```

### Cargo

```bash
cargo install evnx
# with all optional features
cargo install evnx --all-features
```

### Windows

Install [Rust](https://rustup.rs/) first, then:

```powershell
cargo install evnx
evnx --version
```

### Verify

```bash
evnx --version
evnx --help
```

---

## Commands

### `evnx init`

Interactive project setup. Creates `.env` and `.env.example` files for your project through a guided TUI.

```
evnx init
```

Running `evnx init` launches an interactive menu with three modes:

```
How do you want to start?
  Blank      — create empty .env files
  Blueprint  — use a pre-configured stack (Python, Node.js, Rust, Go, PHP, and more)
  Architect  — build a custom stack by selecting services interactively
```

There are no flags required. The interactive flow handles stack and service selection inside the TUI.

---

### `evnx add`

Add variables to an existing `.env` file interactively. Supports custom input, service blueprints, and variable templates.

```bash
evnx add
```

---

### `evnx validate`

Validates your `.env` file for common misconfigurations before deployment.

```bash
evnx validate                            # pretty output
evnx validate --strict                   # exit non-zero on warnings
evnx validate --format json              # machine-readable output
evnx validate --format github-actions    # inline GitHub annotations
```

Detects: missing required variables, placeholder values (`YOUR_KEY_HERE`, `CHANGE_ME`), the boolean string trap (`DEBUG="False"` is truthy in most runtimes), weak secret keys, localhost in production, and suspicious port numbers.

---

### `evnx scan`

Scans files for accidentally committed credentials using pattern matching and entropy analysis.

```bash
evnx scan                         # scan current directory
evnx scan --path src/             # specific path
evnx scan --format sarif          # SARIF output for GitHub Security tab
evnx scan --exit-zero             # warn but do not fail CI
```

Detects: AWS Access Keys, Stripe keys (live and test), GitHub tokens, OpenAI and Anthropic API keys, RSA/EC/OpenSSH private keys, high-entropy strings, and generic API key patterns.

---

### `evnx diff`

Compares `.env` and `.env.example` and shows what is missing, extra, or mismatched.

```bash
evnx diff                     # compare .env vs .env.example
evnx diff --show-values       # include actual values
evnx diff --reverse           # swap comparison direction
evnx diff --format json       # JSON output
```

---

### `evnx convert`

Converts your `.env` file to 14+ output formats for various deployment targets.

```bash
evnx convert --to json
evnx convert --to yaml
evnx convert --to shell
evnx convert --to docker-compose
evnx convert --to kubernetes
evnx convert --to terraform
evnx convert --to github-actions
evnx convert --to aws-secrets
evnx convert --to gcp-secrets
evnx convert --to azure-keyvault
evnx convert --to heroku
evnx convert --to vercel
evnx convert --to railway
evnx convert --to doppler
```

Advanced filtering and transformation:

```bash
evnx convert --to json \
  --output secrets.json \
  --include "AWS_*" \
  --exclude "*_LOCAL" \
  --prefix "APP_" \
  --transform uppercase \
  --base64
```

Pipe directly to AWS Secrets Manager:

```bash
evnx convert --to aws-secrets | \
  aws secretsmanager create-secret \
    --name prod/myapp/config \
    --secret-string file:///dev/stdin
```

---

### `evnx sync`

Keeps `.env` and `.env.example` aligned, in either direction.

```bash
# Forward: .env → .env.example (document what you have)
evnx sync --direction forward --placeholder

# Reverse: .env.example → .env (generate env from template)
evnx sync --direction reverse
```

---

### `evnx migrate` _(requires `--features migrate`)_

Migrates secrets directly to cloud secret managers.

```bash
# GitHub Actions secrets
evnx migrate --from env-file --to github-actions \
  --repo owner/repo --github-token $GITHUB_TOKEN

# AWS Secrets Manager
evnx migrate --to aws-secrets-manager --secret-name prod/myapp/config

# Doppler (with dry run)
evnx migrate --to doppler --dry-run
```

---

### `evnx doctor`

Runs a health check on your environment configuration setup.

```bash
evnx doctor                          # check current directory
evnx doctor --path /path/to/project
```

Checks: `.env` exists and has secure permissions, `.env` is in `.gitignore`, `.env.example` is tracked by Git, and project structure detection.

---

### `evnx template`

Generates configuration files from templates using `.env` variable substitution.

```bash
evnx template \
  --input config.template.yml \
  --output config.yml \
  --env .env
```

Supported inline filters:

```yaml
database:
  host: {{DB_HOST}}
  port: {{DB_PORT|int}}
  ssl:  {{DB_SSL|bool}}
  name: {{DB_NAME|upper}}
```

---

### `evnx backup` / `evnx restore` _(requires `--features backup`)_

Creates and restores AES-256-GCM encrypted backups using Argon2 key derivation.

```bash
evnx backup .env --output .env.backup
evnx restore .env.backup --output .env
```

---

## CI/CD Integration

### GitHub Actions

```yaml
name: Validate environment

on: [push, pull_request]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install evnx
        run: |
          curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash

      - name: Validate configuration
        run: evnx validate --strict --format github-actions

      - name: Scan for secrets
        run: evnx scan --format sarif > scan-results.sarif

      - name: Upload SARIF
        uses: github/codeql-action/upload-sarif@v3
        if: always()
        with:
          sarif_file: scan-results.sarif
```

### GitLab CI

```yaml
validate-env:
  stage: validate
  image: alpine:latest
  before_script:
    - apk add --no-cache curl bash
    - curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash
  script:
    - evnx validate --strict --format json
    - evnx scan --format sarif > scan.sarif
  artifacts:
    reports:
      sast: scan.sarif
```

### Pre-commit hook

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/urwithajit9/evnx
    rev: v0.3.5   
    hooks:
      - id: evnx-scan         # Blocks commit if secrets found
      - id: evnx-validate     # Blocks commit if validation fails
      - id: evnx-diff         # Warns on .env/.env.example drift
      - id: evnx-doctor       # Warns if .env is not gitignored
```

---

## Configuration

Store defaults in `.evnx.toml` at the project root:

```toml
[defaults]
env_file = ".env"
example_file = ".env.example"
verbose = false

[validate]
strict = true
auto_fix = false
format = "pretty"

[scan]
ignore_placeholders = true
exclude_patterns = ["*.example", "*.sample", "*.template"]
format = "pretty"

[convert]
default_format = "json"
base64 = false

[aliases]
gh = "github-actions"
k8s = "kubernetes"
tf = "terraform"
```

---

## Known Limitations

**Array and multiline values** — evnx follows the strict `.env` spec where values are simple strings. The following will not parse correctly:

```bash
# Not supported
CORS_ALLOWED=["https://example.com", "https://admin.example.com"]
CONFIG={"key": "value"}
DATABASE_HOSTS="""
host1.example.com
host2.example.com
"""
```

Use comma-separated strings and parse them in application code. A `--lenient` flag for extended syntax is under consideration — see [open issues](https://github.com/urwithajit9/evnx/issues).

**Windows** — file permissions checking is limited (no Unix permission model). Terminal color support requires PowerShell or Windows Terminal on older systems.

---

## Development

```bash
git clone https://github.com/urwithajit9/evnx.git
cd evnx

cargo build                          # core features only
cargo build --all-features
cargo test
cargo clippy --all-features -- -D warnings
cargo fmt
```

Feature flags:

```toml
[features]
default = []
migrate = ["reqwest", "base64", "indicatif"]
backup  = ["aes-gcm", "argon2", "rand"]
full    = ["migrate", "backup"]
```

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Contributions are welcome in: additional format converters, secret pattern improvements, Windows enhancements, extended `.env` format support, and integration examples.

---

## License

MIT — see [LICENSE](LICENSE).

---

## Credits

Built by [Ajit Kumar](https://github.com/urwithajit9).

Related projects: [python-dotenv](https://github.com/theskumar/python-dotenv), [dotenvy](https://github.com/allan2/dotenvy), [direnv](https://direnv.net/), [git-secrets](https://github.com/awslabs/git-secrets).

---

[Website](https://www.evnx.dev) | [Issues](https://github.com/urwithajit9/evnx/issues) | [Discussions](https://github.com/urwithajit9/evnx/discussions) | [Email](mailto:support@evnx.dev)