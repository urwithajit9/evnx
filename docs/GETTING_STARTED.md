# Getting Started with evnx

evnx is a CLI for managing `.env` files — validation, secret scanning, format conversion, and migration to cloud secret managers. This guide covers installation through your first complete workflow.

Full documentation and guides are available at [evnx.dev](https://www.evnx.dev).

---

## Table of contents

- [Installation](#installation)
- [Verify your install](#verify-your-install)
- [Your first workflow](#your-first-workflow)
- [Step 1 — Initialize your project](#step-1--initialize-your-project)
- [Step 2 — Add variables](#step-2--add-variables)
- [Step 3 — Run a health check](#step-3--run-a-health-check)
- [Step 4 — Scan for secrets](#step-4--scan-for-secrets)
- [Step 5 — Validate your configuration](#step-5--validate-your-configuration)
- [Step 6 — Compare files](#step-6--compare-files)
- [Step 7 — Convert to another format](#step-7--convert-to-another-format)
- [Step 8 — Keep files in sync](#step-8--keep-files-in-sync)
- [Set up the pre-commit hook](#set-up-the-pre-commit-hook)
- [CI/CD integration](#cicd-integration)
- [Configuration file](#configuration-file)
- [Next steps](#next-steps)

---

## Installation

### Linux / macOS

```bash
curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash
```

The script detects your OS and architecture, downloads the correct binary from GitHub Releases, installs it to `~/.local/bin`, and verifies the checksum.

### npm

```bash
npm install -g @evnx/cli
```

### pipx (recommended for Python environments)

```bash
pipx install evnx
```

`pip install evnx` also works but places the binary inside the active virtualenv's `bin/` directory. Use `pipx` to make `evnx` available system-wide without managing a virtualenv manually.

### Cargo

```bash
cargo install evnx
# with all optional features (cloud migration, encrypted backups)
cargo install evnx --all-features
```

### Windows

Install [Rust](https://rustup.rs/) first, then:

```powershell
cargo install evnx
```

---

## Verify your install

```bash
evnx --version
# evnx 0.3.0

evnx --help
```

---

## Your first workflow

The following steps walk through the core evnx workflow on a new project. Each step is independent — skip to any section you need.

---

## Step 1 — Initialize your project

`evnx init` creates `.env` and `.env.example` for your project through a guided interactive TUI. Run it with no arguments:

```bash
cd my-project
evnx init
```

You will be presented with three modes:

```
How do you want to start?
  Blank      — create empty .env files
  Blueprint  — use a pre-configured stack
  Architect  — build a custom stack interactively
```

### Blank mode

Creates minimal empty `.env` and `.env.example` files. Use this when you want a clean starting point with no pre-filled variables.

### Blueprint mode

Selects from a catalog of pre-configured stacks — Next.js, FastAPI, Django, Laravel, T3 Turbo, Rust, and more. Each blueprint generates `.env.example` with the variables that stack typically needs, along with sensible placeholders.

Example output after selecting the T3 Turbo blueprint:

```
Preview:
  NEXTAUTH_URL=http://localhost:3000
  NEXTAUTH_SECRET=CHANGE_ME
  DATABASE_URL=postgresql://localhost:5432/app
  CLERK_SECRET_KEY=CHANGE_ME
  ... 11 more variables

Created .env.example with 15 variables
Added .env to .gitignore
```

### Architect mode

Lets you compose a custom stack by selecting a language, framework, and individual services interactively (databases, auth providers, storage, payment processors, and more). Use this when no blueprint matches your exact setup.

### Non-interactive usage

For scripting or CI environments, use `--yes` to skip prompts and accept the first available blueprint:

```bash
evnx init --yes
evnx init --yes --path ./packages/api
```

After `init`, edit `.env` and replace placeholder values with your real credentials. Never commit `.env` — it is added to `.gitignore` automatically.

---

## Step 2 — Add variables

Once your project is initialised, use `evnx add` to add new variables interactively without touching the file directly:

```bash
evnx add
```

The interactive flow lets you add a custom variable, pick from a service blueprint (e.g. add Stripe variables to an existing project), or select from a variable template. This keeps `.env` and `.env.example` in sync as your project grows.

---

## Step 3 — Run a health check

Before anything else, run `doctor` to check whether your environment setup is correct:

```bash
evnx doctor
```

Sample output:

```
[DOCTOR] Running environment health check...
[OK]      File permissions look good (600)
[OK]      .env is in .gitignore
[WARNING] No .env.example found
[WARNING] .env.example is out of sync with .env — 2 variables differ
[INFO]    2 warnings, 0 errors
[TIP]     Run 'evnx doctor --fix' to auto-fix gitignore issues
```

`doctor` checks: whether `.env` is tracked by git, whether `.gitignore` covers `.env`, whether `.env.example` exists and is in sync, and file permissions on `.env`.

Run with `--fix` to auto-resolve gitignore and permissions issues:

```bash
evnx doctor --fix
```

---

## Step 4 — Scan for secrets

`evnx scan` detects accidentally committed credentials using pattern matching and entropy analysis:

```bash
evnx scan
```

Sample output:

```
[SCAN] Scanning .env...

[ERROR]   AWS_ACCESS_KEY_ID — matches pattern: aws_access_key_id
          Value: AKIAIOSFODNN7EXAMPLE
          Risk: HIGH — AWS access keys can be used to access your AWS account

[ERROR]   AWS_SECRET_ACCESS_KEY — high entropy string detected
          Risk: HIGH

[WARNING] STRIPE_SECRET_KEY — matches pattern: stripe_live_key
          Risk: MEDIUM — live Stripe key detected (looks like a placeholder)

[SUMMARY] 2 errors, 1 warning
```

Detected patterns include: AWS access keys, Stripe live and test keys, GitHub personal access tokens, OpenAI and Anthropic API keys, RSA/EC/OpenSSH private keys, high-entropy strings, and generic API key patterns.

**Exit codes:** `evnx scan` exits with code `1` if errors are found — use this to block CI pipelines. Use `--exit-zero` to always exit `0` for advisory-only checks.

Scan a specific directory:

```bash
evnx scan --path src/
```

Output as SARIF (for GitHub Security tab):

```bash
evnx scan --format sarif > scan-results.sarif
```

Output as JSON:

```bash
evnx scan --format json | jq '.findings[].key'
```

---

## Step 5 — Validate your configuration

Scanning finds secrets. Validation finds misconfiguration — wrong types, placeholder values, weak secrets, and environment inconsistencies:

```bash
evnx validate --strict
```

Sample output:

```
[VALIDATE] Checking .env...

[WARNING] DEBUG=true — boolean string trap
          Python reads this as the string "true", not bool True
          Tip: Use DEBUG=1 or DEBUG=True depending on your framework

[WARNING] JWT_SECRET=secret — weak secret detected
          Value is too short (6 chars). Minimum recommended: 32 chars.

[WARNING] SENDGRID_API_KEY=your-sendgrid-key — placeholder value detected

[WARNING] DATABASE_URL contains localhost — production environment detected
          but localhost database URL found

[SUMMARY] 0 errors, 4 warnings
```

`--strict` promotes warnings to errors and exits non-zero. Without it, warnings are advisory only.

Other output formats:

```bash
evnx validate --format json             # machine-readable
evnx validate --format github-actions   # inline PR annotations
```

---

## Step 6 — Compare files

`evnx diff` shows what is missing, extra, or mismatched between `.env` and `.env.example`:

```bash
evnx diff
```

Sample output:

```
[DIFF] Comparing .env and .env.example

+ NEW_FEATURE_FLAG  (in .env only — missing from .env.example)
+ REDIS_URL         (in .env only — missing from .env.example)
- OLD_API_KEY       (in .env.example only — no longer in .env)
```

Options:

```bash
evnx diff --show-values     # include actual values in output
evnx diff --reverse         # swap the comparison direction
evnx diff --format json     # JSON output for scripting
```

---

## Step 7 — Convert to another format

`evnx convert` transforms your `.env` into the format a deployment target expects:

```bash
evnx convert --to kubernetes --output secret.yaml
```

Available targets:

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

Advanced filtering:

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

## Step 8 — Keep files in sync

`evnx sync` keeps `.env` and `.env.example` aligned as your project evolves:

```bash
# Forward: propagate new keys from .env into .env.example (values redacted)
evnx sync --direction forward --placeholder

# Reverse: generate a fresh .env from .env.example (useful for new teammates or CI)
evnx sync --direction reverse
```

Generate `.env.example` from your existing `.env`:

```bash
evnx sync --generate-example

cat .env.example
# APP_NAME=
# DATABASE_URL=
# AWS_ACCESS_KEY_ID=
# ...
```

Commit `.env.example` to your repository. It tells teammates which variables are required without exposing values.

---

## Set up the pre-commit hook

The pre-commit hook is the most important step for ongoing protection. It runs `evnx scan` automatically before every `git commit`, blocking commits that contain secrets.

### Using pre-commit

Add to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: local
    hooks:
      - id: evnx-scan
        name: Scan for secrets
        entry: evnx scan --exit-code
        language: system
        files: '\.env'
        pass_filenames: false

      - id: evnx-validate
        name: Validate .env
        entry: evnx validate --strict
        language: system
        pass_filenames: false
```

Then install:

```bash
pre-commit install
```

### Manual git hook

```bash
cat > .git/hooks/pre-commit << 'EOF'
#!/bin/bash
evnx scan --exit-code
if [ $? -ne 0 ]; then
  echo "evnx: secrets detected — commit blocked."
  exit 1
fi
EOF
chmod +x .git/hooks/pre-commit
```

The `.pre-commit-config.yaml` approach is better for teams — the configuration is committed to the repo, so every developer gets the hook automatically after running `pre-commit install`.

---

## CI/CD integration

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

      - name: Upload SARIF to GitHub Security
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

### Docker

```dockerfile
FROM rust:slim AS build
RUN cargo install evnx

COPY .env .env
RUN evnx validate --strict \
 && evnx scan --exit-code
```

---

## Configuration file

Store project-level defaults in `.evnx.toml` at the project root:

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

Full reference: [evnx.dev/guides/reference/configuration-file](https://www.evnx.dev/guides/reference/configuration-file)

---

## Next steps

| Topic | Link |
|---|---|
| All command guides | [evnx.dev/guides](https://www.evnx.dev/guides) |
| evnx init in depth | [evnx.dev/guides/commands/init](https://www.evnx.dev/guides/commands/init) |
| evnx add in depth | [evnx.dev/guides/commands/add](https://www.evnx.dev/guides/commands/add) |
| Prevent secret leaks | [evnx.dev/guides/use-cases/prevent-secret-leaks](https://www.evnx.dev/guides/use-cases/prevent-secret-leaks) |
| GitHub Actions integration | [evnx.dev/guides/integrations/github-actions](https://www.evnx.dev/guides/integrations/github-actions) |
| Migrate to AWS Secrets Manager | [evnx.dev/guides/use-cases/use-cases-aws](https://www.evnx.dev/guides/use-cases/use-cases-aws) |
| Team collaboration with sync | [evnx.dev/guides/use-cases/team-collaboration](https://www.evnx.dev/guides/use-cases/team-collaboration) |
| .evnx.toml reference | [evnx.dev/guides/reference/configuration-file](https://www.evnx.dev/guides/reference/configuration-file) |
| Security model | [evnx.dev/guides/reference/concepts-security-model](https://www.evnx.dev/guides/reference/concepts-security-model) |
| Changelog | [CHANGELOG.md](../CHANGELOG.md) |

---

[Website](https://www.evnx.dev) | [GitHub](https://github.com/urwithajit9/evnx) | [Issues](https://github.com/urwithajit9/evnx/issues)