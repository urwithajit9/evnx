# evnx CLI

[![CI](https://github.com/urwithajit9/evnx/workflows/CI/badge.svg)](https://github.com/urwithajit9/evnx/actions)
[![Release](https://img.shields.io/github/v/release/urwithajit9/evnx)](https://github.com/urwithajit9/evnx/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A comprehensive CLI tool for managing `.env` files — validation, secret scanning, format conversion, and migration to cloud secret managers.

**📚 [Documentation](./docs/GETTING_STARTED.md)** | **🌐 [Website](https://dotenv.space)**

## Why evnx?

I built this after accidentally pushing AWS credentials to GitHub in a test file during an Airflow refactor (20 DAGs, 300+ Scrapy spiders). The key was revoked immediately, other services went down, and I had to explain the incident to my development head. That conversation was more painful than any billing alert.

Three years later, I'm still paranoid about secrets management. This tool is the safety net I wish I'd had.

## ✨ Features

All features are **production-ready** and working in v0.1.0!

### Core Commands (Always Available)

- ✅ **`init`** - Interactive project setup with templates for Python, Node.js, Rust, Go, PHP
- ✅ **`validate`** - Comprehensive validation (checks for placeholders, weak secrets, misconfigurations)
- ✅ **`scan`** - Secret detection using pattern matching and entropy analysis
- ✅ **`diff`** - Compare `.env` and `.env.example`, show missing/extra variables
- ✅ **`convert`** - Transform to 14+ formats (JSON, YAML, Docker, Kubernetes, AWS, GCP, Azure, GitHub Actions, and more)
- ✅ **`sync`** - Keep `.env` and `.env.example` in sync (bidirectional)

### Extended Commands (With Features)

- ✅ **`migrate`** - Direct migration to secret managers (GitHub Actions, AWS Secrets Manager, Doppler, Infisical)
- ✅ **`doctor`** - Diagnose common setup issues
- ✅ **`template`** - Generate config files from templates with variable substitution
- ✅ **`backup`** - Create AES-256-GCM encrypted backups
- ✅ **`restore`** - Restore from encrypted backups

**Build with all features:**
```bash
cargo build --features full
# or
cargo build --all-features
```

## 🚀 Quick Start

### Installation

#### macOS / Linux
```bash
curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/install.sh | bash
```

#### Windows
**Prerequisites:** Install [Rust](https://rustup.rs/) first.

```powershell
# Clone the repository
git clone https://github.com/urwithajit9/evnx.git
cd evnx

# Build and install with core features
cargo install --path .

# Or build with all features
cargo install --path . --features full

# Verify installation
evnx --version
```

**Note:** Add `%USERPROFILE%\.cargo\bin` to your PATH if not already done during Rust installation.

**Tested on:** Windows 10/11 with PowerShell 5.1+

#### From crates.io
```bash
# Install with core features only
cargo install evnx

# Install with all features
cargo install evnx --features full
```

#### Verify Installation
```bash
evnx --version
evnx --help
```

### Basic Usage

```bash
# 1. Initialize a new project
evnx init

# 2. Validate your configuration
evnx validate --strict

# 3. Scan for accidentally committed secrets
evnx scan

# 4. Compare files
evnx diff --show-values

# 5. Convert to different formats
evnx convert --to json > config.json
evnx convert --to github-actions
evnx convert --to kubernetes > secret.yaml

# 6. Keep files in sync
evnx sync --direction forward
```

## 📖 Documentation

- **[Getting Started Guide](./docs/GETTING_STARTED.md)** - Complete walkthrough with examples
- **[Use Cases](./docs/USE_CASES.md)** - Real-world scenarios
- **[CI/CD Integration](./docs/CICD_GUIDE.md)** - GitLab, GitHub Actions, Jenkins
- **[Architecture](./ARCHITECTURE.md)** - System design and internals
- **[Contributing](./CONTRIBUTING.md)** - How to contribute

## 🎯 Command Overview

### `evnx init`

**Interactive project setup** - Generates `.env.example` with sensible defaults.

```bash
evnx init                                # Interactive mode
evnx init --stack python --yes           # Quick setup
evnx init --stack nodejs --services postgres,redis
```

**Supported stacks:** Python, Node.js, Rust, Go, PHP
**Supported services:** PostgreSQL, Redis, MongoDB, MySQL, RabbitMQ, Elasticsearch, AWS S3, Stripe, SendGrid, OpenAI, and more

---

### `evnx validate`

**Comprehensive validation** - Catches misconfigurations before deployment.

```bash
evnx validate                            # Pretty output
evnx validate --strict                   # Fail on warnings
evnx validate --format json              # JSON output
evnx validate --format github-actions    # GitHub annotations
```

**Detects:**
- ❌ Missing required variables
- ❌ Placeholder values (`YOUR_KEY_HERE`, `CHANGE_ME`)
- ❌ Boolean string trap (`DEBUG="False"` is truthy!)
- ❌ Weak `SECRET_KEY` (too short, common patterns)
- ❌ `localhost` in production
- ❌ Suspicious port numbers

---

### `evnx scan`

**Secret detection** - Find accidentally committed credentials.

```bash
evnx scan                                # Scan current directory
evnx scan --path src/                    # Specific directory
evnx scan --format sarif                 # SARIF for GitHub Security
evnx scan --exit-zero                    # Don't fail CI
```

**Detects 8+ secret types:**
- AWS Access Keys (`AKIA...`)
- Stripe API Keys (live & test)
- GitHub Personal Access Tokens
- OpenAI API Keys
- Anthropic API Keys
- Private Keys (RSA, EC, OpenSSH)
- High-entropy strings (potential secrets)
- Generic API keys

**SARIF output** integrates with GitHub Security tab!

---

### `evnx diff`

**File comparison** - See what's different between environments.

```bash
evnx diff                                # Compare .env and .env.example
evnx diff --show-values                  # Show actual values
evnx diff --reverse                      # Swap comparison
evnx diff --format json                  # JSON output
```

---

### `evnx convert`

**Format conversion** - Transform to 14+ output formats.

```bash
evnx convert --to json                   # Generic JSON
evnx convert --to yaml                   # Generic YAML
evnx convert --to shell                  # Shell export script
evnx convert --to docker-compose         # Docker Compose format
evnx convert --to kubernetes             # Kubernetes Secret YAML
evnx convert --to terraform              # Terraform .tfvars
evnx convert --to github-actions         # GitHub Actions format
evnx convert --to aws-secrets            # AWS Secrets Manager
evnx convert --to gcp-secrets            # GCP Secret Manager
evnx convert --to azure-keyvault         # Azure Key Vault
evnx convert --to heroku                 # Heroku Config Vars
evnx convert --to vercel                 # Vercel Environment Variables
evnx convert --to railway               # Railway JSON
evnx convert --to doppler                # Doppler format
```

**Advanced options:**
```bash
evnx convert --to json \
  --output secrets.json \              # Write to file
  --include "AWS_*" \                  # Filter variables
  --exclude "*_LOCAL" \                # Exclude patterns
  --prefix "APP_" \                    # Add prefix
  --transform uppercase \              # Transform keys
  --base64                             # Base64-encode values
```

**Real-world example - Deploy to AWS:**
```bash
evnx convert --to aws-secrets | \
  aws secretsmanager create-secret \
    --name prod/myapp/config \
    --secret-string file:///dev/stdin
```

---

### `evnx sync`

**Bidirectional sync** - Keep `.env` and `.env.example` aligned.

```bash
# Forward: .env → .env.example (document what you have)
evnx sync --direction forward --placeholder

# Reverse: .env.example → .env (generate from template)
evnx sync --direction reverse
```

**Use cases:**
- Generate `.env` from `.env.example` in CI/CD
- Update `.env.example` when adding new variables
- Maintain documentation

---

### `evnx migrate` *(Requires `--features migrate`)*

**Cloud migration** - Move secrets directly to secret managers.

```bash
# GitHub Actions Secrets
evnx migrate \
  --from env-file \
  --to github-actions \
  --repo owner/repo \
  --github-token $GITHUB_TOKEN

# AWS Secrets Manager
evnx migrate \
  --to aws-secrets-manager \
  --secret-name prod/myapp/config

# Doppler
evnx migrate \
  --to doppler \
  --dry-run  # Preview changes first
```

**Features:**
- ✅ Conflict detection (skip or overwrite)
- ✅ Dry-run mode
- ✅ Progress tracking
- ✅ Encrypted uploads (GitHub uses libsodium)

---

### `evnx doctor`

**Health check** - Diagnose common issues.

```bash
evnx doctor                              # Check current directory
evnx doctor --path /path/to/project
```

**Checks:**
- ✅ `.env` exists and has secure permissions
- ✅ `.env` is in `.gitignore`
- ✅ `.env.example` exists and is tracked by Git
- ✅ Project structure detection (Python, Node.js, Rust, Docker)

---

### `evnx template`

**Template generation** - Dynamic config file creation.

```bash
evnx template \
  --input config.template.yml \
  --output config.yml \
  --env .env
```

**Supports filters:**
```yaml
# config.template.yml
database:
  host: {{DB_HOST}}
  port: {{DB_PORT|int}}
  ssl: {{DB_SSL|bool}}
  name: {{DB_NAME|upper}}
```

---

### `evnx backup/restore` *(Requires `--features backup`)*

**Encrypted backups** - AES-256-GCM encryption with Argon2 key derivation.

```bash
# Create backup
evnx backup .env --output .env.backup

# Restore
evnx restore .env.backup --output .env
```

**Security:**
- AES-256-GCM encryption
- Argon2 password hashing
- No secrets in plaintext

---

## ⚠️ Known Issues

### Array/List Value Parsing

evnx currently **does not support** array-like or list-like values in `.env` files. This affects Django and other frameworks that use python-dotenv's extended syntax.

**Will fail:**
```bash
# Arrays with brackets
CORS_ALLOWED=["https://example.com", "https://admin.example.com"]

# Multiline values
DATABASE_HOSTS="""
host1.example.com
host2.example.com
"""

# JSON values
CONFIG={"key": "value", "nested": {"data": 123}}
```

**Workaround:**
```bash
# Use comma-separated strings instead
CORS_ALLOWED=https://example.com,https://admin.example.com

# Or use base64-encoded JSON
CONFIG_JSON=eyJrZXkiOiJ2YWx1ZSJ9

# Parse in your application code
# Python example:
import os
import json
cors_allowed = os.getenv("CORS_ALLOWED", "").split(",")
config = json.loads(base64.b64decode(os.getenv("CONFIG_JSON")))
```

**Why this limitation?**
evnx follows the strict `.env` format specification which defines values as simple strings. Django's python-dotenv uses extended parsing that's not compatible with the standard format used by most other tools.

**Tracking:** We're considering adding a `--lenient` or `--extended` flag for compatibility. Follow [Issue #XX](https://github.com/urwithajit9/evnx/issues) for updates.

**Affects:**
- Django projects using complex ALLOWED_HOSTS or CORS settings
- Projects with JSON/YAML embedded in .env values
- Multiline string values

**Does NOT affect:**
```bash
# These work fine
DATABASE_URL=postgres://localhost/db
API_KEYS=key1,key2,key3           # Simple comma-separated
ALLOWED_HOSTS=example.com admin.example.com  # Space-separated
DEBUG=True
PORT=3000
```

### Windows-Specific Issues

- File permissions checking is limited on Windows (no Unix permissions)
- Path handling uses backslashes (handled internally)
- Some terminal color codes may not display correctly in older CMD (use PowerShell or Windows Terminal)

**These are tracked and will be improved in future releases.**

---

## 🔧 CI/CD Integration

### GitHub Actions

```yaml
name: Validate Environment

on: [push, pull_request]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install evnx
        run: |
          curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/install.sh | bash

      - name: Validate configuration
        run: evnx validate --strict --format github-actions

      - name: Scan for secrets
        run: evnx scan --format sarif > scan-results.sarif

      - name: Upload SARIF to GitHub Security
        uses: github/codeql-action/upload-sarif@v2
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
    - curl -sSL https://install.dotenv.space | bash
  script:
    - evnx validate --strict --format json
    - evnx scan --format sarif > scan.sarif
  artifacts:
    reports:
      sast: scan.sarif
```

### Pre-commit Hook

```yaml
# .pre-commit-config.yaml
repos:
  - repo: local
    hooks:
      - id: dotenv-validate
        name: Validate .env files
        entry: evnx validate --strict
        language: system
        pass_filenames: false

      - id: dotenv-scan
        name: Scan for secrets
        entry: evnx scan --exit-zero
        language: system
        pass_filenames: false
```

---

## ⚙️ Configuration

Store preferences in `.evnx.toml`:

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

## 🏗️ Development

```bash
# Clone repository
git clone https://github.com/urwithajit9/evnx.git
cd evnx

# Build (core features only)
cargo build

# Build with all features
cargo build --all-features

# Run tests
cargo test

# Run with features
cargo run --features migrate -- migrate --help
cargo run --features backup -- backup --help
cargo run --all-features -- --help

# Lint and format
cargo clippy --all-features -- -D warnings
cargo fmt
```

### Feature Flags

```toml
# Cargo.toml features
[features]
default = []
migrate = ["reqwest", "base64", "indicatif"]
backup = ["aes-gcm", "argon2", "rand"]
full = ["migrate", "backup"]
```

**Why feature flags?**
- Smaller binary size for basic usage
- Optional dependencies (reqwest, crypto libraries)
- Faster compilation during development

---

## 🤝 Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md).

**Areas where help is appreciated:**
- Additional format converters
- Secret pattern improvements
- Windows support enhancements
- Extended `.env` format support (arrays, multiline values)
- Documentation improvements
- Integration examples
- Translation (i18n)

---

## 📜 License

MIT License - see [LICENSE](LICENSE)

---

## 🙏 Credits

Built by [Ajit Kumar](https://github.com/urwithajit9) after learning the hard way about secrets management.

**Inspired by:**
- Countless developers who've accidentally committed secrets
- The pain of production incidents caused by misconfiguration
- The desire for better developer tooling

**Related Projects:**
- [python-dotenv](https://github.com/theskumar/python-dotenv) - Python implementation
- [dotenvy](https://github.com/allan2/dotenvy) - Rust dotenv parser
- [direnv](https://direnv.net/) - Environment switcher
- [git-secrets](https://github.com/awslabs/git-secrets) - AWS secret scanning

---

## 🆘 Support

- 🐛 [Report a bug](https://github.com/urwithajit9/evnx/issues/new?template=bug_report.md)
- 💡 [Request a feature](https://github.com/urwithajit9/evnx/issues/new?template=feature_request.md)
- 💬 [Start a discussion](https://github.com/urwithajit9/evnx/discussions)
- 📧 [Email](mailto:support@dotenv.space)

---

## ⭐ Show Your Support

If this tool saved you from a secrets incident or made your life easier, please:

- ⭐ [Star the repository](https://github.com/urwithajit9/evnx)
- 🐦 [Tweet about it](https://twitter.com/intent/tweet?text=Check%20out%20evnx%20-%20a%20comprehensive%20CLI%20for%20managing%20.env%20files!&url=https://github.com/urwithajit9/evnx)
- 📝 [Write a blog post](https://github.com/urwithajit9/evnx/discussions)
- 💬 Tell your teammates

**Your support helps improve the tool for everyone!**

---

<div align="center">

**Made with 🦀 Rust and ❤️ by developers who've been there**

[Website](https://dotenv.space) • [Documentation](./docs/GETTING_STARTED.md) • [GitHub](https://github.com/urwithajit9/evnx)

</div>