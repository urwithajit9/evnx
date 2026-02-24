# dotenv-space CLI

[![CI](https://github.com/urwithajit9/dotenv-space/workflows/CI/badge.svg)](https://github.com/urwithajit9/dotenv-space/actions)
[![Release](https://img.shields.io/github/v/release/urwithajit9/dotenv-space)](https://github.com/urwithajit9/dotenv-space/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A comprehensive CLI tool for managing `.env` files — validation, secret scanning, format conversion, and migration to cloud secret managers.

**📚 [Documentation](./docs/GETTING_STARTED.md)** | **🌐 [Website](https://dotenv.space)**

## Why dotenv-space?

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
curl -sSL https://raw.githubusercontent.com/urwithajit9/dotenv-space/main/install.sh | bash
```

#### From source
```bash
# Install with core features only
cargo install dotenv-space

# Install with all features
cargo install dotenv-space --features full
```

#### Verify
```bash
dotenv-space --version
dotenv-space --help
```

### Basic Usage

```bash
# 1. Initialize a new project
dotenv-space init

# 2. Validate your configuration
dotenv-space validate --strict

# 3. Scan for accidentally committed secrets
dotenv-space scan

# 4. Compare files
dotenv-space diff --show-values

# 5. Convert to different formats
dotenv-space convert --to json > config.json
dotenv-space convert --to github-actions
dotenv-space convert --to kubernetes > secret.yaml

# 6. Keep files in sync
dotenv-space sync --direction forward
```

## 📖 Documentation

- **[Getting Started Guide](./docs/GETTING_STARTED.md)** - Complete walkthrough with examples
- **[Use Cases](./docs/USE_CASES.md)** - Real-world scenarios
- **[CI/CD Integration](./docs/CICD_GUIDE.md)** - GitLab, GitHub Actions, Jenkins
- **[Architecture](./ARCHITECTURE.md)** - System design and internals
- **[Contributing](./CONTRIBUTING.md)** - How to contribute

## 🎯 Command Overview

### `dotenv-space init`

**Interactive project setup** - Generates `.env.example` with sensible defaults.

```bash
dotenv-space init                                # Interactive mode
dotenv-space init --stack python --yes           # Quick setup
dotenv-space init --stack nodejs --services postgres,redis
```

**Supported stacks:** Python, Node.js, Rust, Go, PHP  
**Supported services:** PostgreSQL, Redis, MongoDB, MySQL, RabbitMQ, Elasticsearch, AWS S3, Stripe, SendGrid, OpenAI, and more

---

### `dotenv-space validate`

**Comprehensive validation** - Catches misconfigurations before deployment.

```bash
dotenv-space validate                            # Pretty output
dotenv-space validate --strict                   # Fail on warnings
dotenv-space validate --format json              # JSON output
dotenv-space validate --format github-actions    # GitHub annotations
```

**Detects:**
- ❌ Missing required variables
- ❌ Placeholder values (`YOUR_KEY_HERE`, `CHANGE_ME`)
- ❌ Boolean string trap (`DEBUG="False"` is truthy!)
- ❌ Weak `SECRET_KEY` (too short, common patterns)
- ❌ `localhost` in production
- ❌ Suspicious port numbers

---

### `dotenv-space scan`

**Secret detection** - Find accidentally committed credentials.

```bash
dotenv-space scan                                # Scan current directory
dotenv-space scan --path src/                    # Specific directory
dotenv-space scan --format sarif                 # SARIF for GitHub Security
dotenv-space scan --exit-zero                    # Don't fail CI
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

### `dotenv-space diff`

**File comparison** - See what's different between environments.

```bash
dotenv-space diff                                # Compare .env and .env.example
dotenv-space diff --show-values                  # Show actual values
dotenv-space diff --reverse                      # Swap comparison
dotenv-space diff --format json                  # JSON output
```

---

### `dotenv-space convert`

**Format conversion** - Transform to 14+ output formats.

```bash
dotenv-space convert --to json                   # Generic JSON
dotenv-space convert --to yaml                   # Generic YAML
dotenv-space convert --to shell                  # Shell export script
dotenv-space convert --to docker-compose         # Docker Compose format
dotenv-space convert --to kubernetes             # Kubernetes Secret YAML
dotenv-space convert --to terraform              # Terraform .tfvars
dotenv-space convert --to github-actions         # GitHub Actions format
dotenv-space convert --to aws-secrets            # AWS Secrets Manager
dotenv-space convert --to gcp-secrets            # GCP Secret Manager
dotenv-space convert --to azure-keyvault         # Azure Key Vault
dotenv-space convert --to heroku                 # Heroku Config Vars
dotenv-space convert --to vercel                 # Vercel Environment Variables
dotenv-space convert --to railway               # Railway JSON
dotenv-space convert --to doppler                # Doppler format
```

**Advanced options:**
```bash
dotenv-space convert --to json \
  --output secrets.json \              # Write to file
  --include "AWS_*" \                  # Filter variables
  --exclude "*_LOCAL" \                # Exclude patterns
  --prefix "APP_" \                    # Add prefix
  --transform uppercase \              # Transform keys
  --base64                             # Base64-encode values
```

**Real-world example - Deploy to AWS:**
```bash
dotenv-space convert --to aws-secrets | \
  aws secretsmanager create-secret \
    --name prod/myapp/config \
    --secret-string file:///dev/stdin
```

---

### `dotenv-space sync`

**Bidirectional sync** - Keep `.env` and `.env.example` aligned.

```bash
# Forward: .env → .env.example (document what you have)
dotenv-space sync --direction forward --placeholder

# Reverse: .env.example → .env (generate from template)
dotenv-space sync --direction reverse
```

**Use cases:**
- Generate `.env` from `.env.example` in CI/CD
- Update `.env.example` when adding new variables
- Maintain documentation

---

### `dotenv-space migrate` *(Requires `--features migrate`)*

**Cloud migration** - Move secrets directly to secret managers.

```bash
# GitHub Actions Secrets
dotenv-space migrate \
  --from env-file \
  --to github-actions \
  --repo owner/repo \
  --github-token $GITHUB_TOKEN

# AWS Secrets Manager
dotenv-space migrate \
  --to aws-secrets-manager \
  --secret-name prod/myapp/config

# Doppler
dotenv-space migrate \
  --to doppler \
  --dry-run  # Preview changes first
```

**Features:**
- ✅ Conflict detection (skip or overwrite)
- ✅ Dry-run mode
- ✅ Progress tracking
- ✅ Encrypted uploads (GitHub uses libsodium)

---

### `dotenv-space doctor`

**Health check** - Diagnose common issues.

```bash
dotenv-space doctor                              # Check current directory
dotenv-space doctor --path /path/to/project
```

**Checks:**
- ✅ `.env` exists and has secure permissions
- ✅ `.env` is in `.gitignore`
- ✅ `.env.example` exists and is tracked by Git
- ✅ Project structure detection (Python, Node.js, Rust, Docker)

---

### `dotenv-space template`

**Template generation** - Dynamic config file creation.

```bash
dotenv-space template \
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

### `dotenv-space backup/restore` *(Requires `--features backup`)*

**Encrypted backups** - AES-256-GCM encryption with Argon2 key derivation.

```bash
# Create backup
dotenv-space backup .env --output .env.backup

# Restore
dotenv-space restore .env.backup --output .env
```

**Security:**
- AES-256-GCM encryption
- Argon2 password hashing
- No secrets in plaintext

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
      
      - name: Install dotenv-space
        run: |
          curl -sSL https://raw.githubusercontent.com/urwithajit9/dotenv-space/main/install.sh | bash
      
      - name: Validate configuration
        run: dotenv-space validate --strict --format github-actions
      
      - name: Scan for secrets
        run: dotenv-space scan --format sarif > scan-results.sarif
      
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
    - dotenv-space validate --strict --format json
    - dotenv-space scan --format sarif > scan.sarif
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
        entry: dotenv-space validate --strict
        language: system
        pass_filenames: false
      
      - id: dotenv-scan
        name: Scan for secrets
        entry: dotenv-space scan --exit-zero
        language: system
        pass_filenames: false
```

---

## ⚙️ Configuration

Store preferences in `.dotenv-space.toml`:

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
git clone https://github.com/urwithajit9/dotenv-space.git
cd dotenv-space

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
- Windows support
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

- 🐛 [Report a bug](https://github.com/urwithajit9/dotenv-space/issues/new?template=bug_report.md)
- 💡 [Request a feature](https://github.com/urwithajit9/dotenv-space/issues/new?template=feature_request.md)
- 💬 [Start a discussion](https://github.com/urwithajit9/dotenv-space/discussions)
- 📧 [Email](mailto:support@dotenv.space)

---

## ⭐ Show Your Support

If this tool saved you from a secrets incident or made your life easier, please:

- ⭐ [Star the repository](https://github.com/urwithajit9/dotenv-space)
- 🐦 [Tweet about it](https://twitter.com/intent/tweet?text=Check%20out%20dotenv-space%20-%20a%20comprehensive%20CLI%20for%20managing%20.env%20files!&url=https://github.com/urwithajit9/dotenv-space)
- 📝 [Write a blog post](https://github.com/urwithajit9/dotenv-space/discussions)
- 💬 Tell your teammates

**Your support helps improve the tool for everyone!**

---

<div align="center">

**Made with 🦀 Rust and ❤️ by developers who've been there**

[Website](https://dotenv.space) • [Documentation](./docs/GETTING_STARTED.md) • [GitHub](https://github.com/urwithajit9/dotenv-space)

</div>