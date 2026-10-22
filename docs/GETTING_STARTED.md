# Getting Started with evnx

evnx is a CLI for managing `.env` files — validation, secret scanning, format conversion, and migration to cloud secret managers. It also syncs them end-to-end encrypted. This guide covers installation through your first complete workflow.

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
- [Step 9 — Render a config file from a template](#step-9--render-a-config-file-from-a-template)
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

Example output after selecting the T3 Turbo blueprint (evnx 0.10.1):

```
Preview:
28 variables
  • framework ×4
  • service ×18
  • infra ×6
  Categories: API, Application, Authentication, CI/CD, Cloud Storage, Database, Deployment Platform, Payments
Generate .env files with these variables? yes
✓ Created .env from template
✓ Created .gitignore with .env*, !.env.example, !.env.sample, !.env.template
✓ Created .env.example with 28 variables
```

Next.js stacks also get `NEXT_PUBLIC_` copies of service values that are meant for the
browser (here the Stripe publishable key); Vite, SvelteKit, Nuxt and Laravel stacks get
their own prefixes. In the menus, Blank is pre-selected, yes/no questions take their answer
on Enter, and Esc cancels without writing anything.

The `×` counts are variables by where they come from (framework, services, infrastructure), not numbers of components.

### Architect mode

Lets you compose a custom stack by selecting a language, framework, and individual services interactively (databases, auth providers, storage, payment processors, and more). Use this when no blueprint matches your exact setup.

### Non-interactive usage

For scripting or CI, name what you want; nothing is guessed. `--yes` alone writes **blank** files (it has meant Blank since v0.5.0, not "the first blueprint"):

```bash
evnx init --yes                                  # blank .env.example, .env and .gitignore rules
evnx init --yes --blueprint t3_modern            # a named stack
evnx init --yes --with nextjs,postgresql,stripe  # name the components directly
evnx init --list-components                      # every name --with accepts
evnx init --detect                               # accept what package.json, compose files etc. declare
evnx init --from-source --yes                    # add the variables the code reads
evnx init --yes --path ./packages/api            # write into another directory
```

Re-running is safe: `.env` is never overwritten, and an existing `.env.example` is replaced only with `--force` (under `--yes` without `--force`, init stops with exit 2). Without a terminal, plain `evnx init` cannot show its menu, so pass one of the flags above.

After `init`, edit `.env` and replace placeholder values with your real credentials. Never commit `.env` — it is added to `.gitignore` automatically.

---

## Step 2 — Add variables

Once your project is initialised, use `evnx add` to add the variables of a new service, framework or whole stack without touching the files directly:

```bash
evnx add service stripe
```

It shows a preview and asks before writing; `--yes` skips the question. `evnx add framework <id>` and `evnx add blueprint <id>` work the same way, and `evnx add custom` asks for your own variables one at a time. `.env.example` and `.env` are updated together, so they stay in sync as your project grows — the value you give goes only into `.env`; `.env.example`, which you commit, gets a placeholder such as `YOUR_KEY_HERE`.

---

## Step 3 — Run a health check

Before anything else, run `doctor` to check whether your environment setup is correct:

```bash
evnx doctor
```

Sample output, for a project `evnx init` set up whose `.env.example` is not committed yet:

```
  evnx doctor  Diagnosing environment setup

  ✓  Validate .env file existence, security, and syntax

  !  Check .env.example exists and is Git-tracked
     .env.example is not tracked in Git — run: git add .env.example

  ✓  Detect project type and validate dependencies

  ·  Check for Docker configuration files
     No Docker configuration detected

  !  Check file permissions for sensitive files
     .env: permissions 644 (recommended: 600)

  2 warnings  ·  2 checks passed
```

`doctor` checks every env file — `.env`, `.env.<name>` and plaintext copies such as
`.env.bak`: that git ignores it and does not track it, that it is valid dotenv, and that
no other user can read it. It also checks that `.env.example` exists and is tracked by
git. It does not compare the two files' keys: `evnx sync --check` does that (Step 8), and
so does `evnx diff`. Errors exit 1; warnings exit 0 unless you pass `--strict`.

Run with `--fix` to repair the `.gitignore` entries and file permissions. Each repair is
printed, and checked again afterwards:

```bash
evnx doctor --fix
# ✓ Fixed: .env mode 644 → 600
```

---

## Step 4 — Scan for secrets

`evnx scan` detects accidentally committed credentials using pattern matching and entropy analysis:

```bash
evnx scan
```

Sample output, in a git repository whose `.gitignore` covers `.env`, with a key also
pasted into a source file:

```
  evnx scan  Checking for exposed secrets

  scanning ./.env, ./.gitignore, ./src/billing.py

  ✗  Stripe Secret Key (LIVE)                    high
     ./src/billing.py:1  sk_l…hijk
     !  matches a live key format, not a placeholder
     →  revoke at https://dashboard.stripe.com/apikeys

  1 finding  ·  1 high
  ·  git ignores ./.env — 3 findings there are not counted, because it is not committed (--include-ignored lists them)


Next steps:
  1. Move the values out of ./src/billing.py before you commit — into a file git ignores, such as .env, read at run time
  2. If one was pushed or shared before, revoke or rotate it
```

Values are never printed whole (`sk_l…hijk`). **In a git repository a folder scan counts
what git would commit**: `.env` is where secrets belong, so its findings are one line and
do not fail the run. A tracked file is reported with advice to rewrite its history; name a
file (`evnx scan .env`) or add `--include-ignored` to see every finding.

Detected: AWS access keys (long-term and STS), GitHub tokens (classic and fine-grained),
Slack tokens, Google API keys, JSON Web Tokens, Stripe live and test keys, OpenAI and
Anthropic API keys, private keys (RSA, EC, OpenSSH, PKCS#8), passwords in connection
strings, values in variables named like secrets, and high-entropy strings.
Placeholders such as `sk_live_xxxx…` or `user:password@localhost` are not findings.

**Exit codes:** `0` clean, `1` secrets found or a file that could not be read, `2` the
scan could not be completed — a path that does not exist, a `--pattern` that will not
compile, nothing left to scan.

⚠️ **`2` is not a louder `1`.** "I found nothing" and "I could not look" are different
answers, and collapsing them is what let `evnx scan ./typo` report success on older
versions. `--exit-zero` suppresses `1` only; trouble still exits `2`, because the flag
means "do not fail my build over findings", not "never tell me the scan was impossible".

A folder scan reads **every** file, so a key in `config/id_rsa.pem` or `app.swift` is
found. Binary files and files over 1 MiB (`--max-file-size`) are skipped, and the scan
says how many (`-v` lists them). To read only some file types, opt in with
`--extensions env,yml,json` — the run then says it is limited.

Scan specific paths — positional and repeatable:

```bash
evnx scan src/ config/
```

Output as SARIF (for GitHub Security tab):

```bash
evnx scan --format sarif > scan-results.sarif
```

Output as JSON:

```bash
evnx scan --format json | jq '.findings[].location'
```

---

## Step 5 — Validate your configuration

Scanning finds secrets. Validation finds misconfiguration — wrong types, placeholder values, weak secrets, and environment inconsistencies:

```bash
evnx validate --strict
```

Sample output:

```
  evnx validate  Check environment configuration

  !  Extra variable not in .env.example: OLD_FLAG
     .env
     →  Add OLD_FLAG to .env.example or remove from .env

  ✗  SENDGRID_API_KEY looks like a placeholder
     .env
     ·  fixable with --fix

  !  DEBUG is set to "True" (string, not boolean)
     .env
     →  Use true or 1 for proper boolean handling
     ·  fixable with --fix

  ✗  JWT_SECRET is too weak or predictable
     .env
     →  Rotate JWT_SECRET where it is issued, then replace it here. …

  2 errors  ·  2 warnings
```

Errors exit 1; warnings alone exit 0. `--strict` adds the extra-variable check
(`OLD_FLAG` above); `--fail-on warning` makes warnings fail the run as well.
`--fix` repairs what it can — and lists what it could only add as a placeholder
under "Needs your input" — and `--fix --dry-run` shows the plan without writing.

Other output formats:

```bash
evnx validate --format json             # machine-readable
evnx validate --format github-actions   # inline PR annotations
```

### Declare the contract

`.env.example` cannot say that a variable is optional, a port, or a secret. `[vars]` in
`.evnx.toml` can, and `evnx spec init` writes a first one from the files you already have:

```bash
evnx spec init            # infer from .env.example and .env, write .evnx.toml
cat .evnx.toml            # every field is a guess: read it, set required = false where it applies
evnx validate             # now checks against the contract
git add .evnx.toml        # commit it; the spec is the contract
```

A `format` is written only when your own value passes it, so this `validate` passes.
Later, `evnx spec init --merge` adds only the names `[vars]` does not declare yet.

---

## Step 6 — Compare files

`evnx diff` shows which keys `.env` lacks and which ones `.env.example` does not
declare yet:

```bash
evnx diff
```

Sample output (with `.env.example` declaring `OLD_API_KEY`, and `.env` holding two
keys the template lacks):

```
  evnx diff  .env ↔ .env.example

Missing from .env (present in .env.example):
  + OLD_API_KEY

Extra in .env (not in .env.example):
  - NEW_FEATURE_FLAG
  - REDIS_URL

  1 missing from .env  ·  2 not in .env.example
  values not compared against a template (--values to compare them)
```

`+` is a key to add to `.env`, `-` a key the template does not declare. Against a
template only key names are compared: every filled-in value differs from its
placeholder by design. Between two real environments, values are compared too:

```bash
evnx diff --env-name production --against staging
```

Exit codes: `0` nothing `--fail-on` counts, `1` differences found, `2` could not run.

Options:

```bash
evnx diff --fail-on missing             # exit 1 only when .env lacks a template key
evnx diff --values                      # compare values against the template too
evnx diff --show-values                 # print values; secrets stay masked (postgres://admin:***@db)
evnx diff --reverse                     # swap the comparison direction
evnx diff --ignore-keys BUILD_ID        # leave keys out (exact case; adds to [diff] ignore_keys)
evnx diff --format json                 # JSON for scripting (schema_version 1)
evnx diff --format patch --show-values  # values are <withheld> without --show-values
```

A CI gate on missing keys only:

```bash
evnx diff --fail-on missing
# or, from the JSON:
evnx diff --format json | jq -e '.summary.missing == 0'
```

The JSON has `missing` and `extra` (key names), `different` (one object per changed
key, with `value_state`: `withheld`, `masked` or `shown`), `ignored`, `compared`
(`keys` or `values`) and `summary` counts.

---

## Step 7 — Convert to another format

`evnx convert` transforms your `.env` into the format a deployment target expects:

```bash
evnx convert --to kubernetes --name myapp-env --output secret.yaml
```

The file holds your secrets, so it is created readable by you only (mode 0600).
If `secret.yaml` already exists, evnx stops and changes nothing; add `--force`
to replace it. In a git repository evnx also checks that `.gitignore` covers it.

Available targets (`evnx convert --help` has the details):

```bash
evnx convert --to json
evnx convert --to yaml
evnx convert --to shell
evnx convert --to docker-compose
evnx convert --to kubernetes --name myapp-env
evnx convert --to terraform
evnx convert --to github-actions          # text to paste into the GitHub web form
evnx convert --to aws-secrets             # JSON for create-secret --secret-string
evnx convert --to gcp-secrets --project my-gcp-project
evnx convert --to azure-keyvault --vault-name my-vault
evnx convert --to heroku --heroku-app my-app
evnx convert --to vercel                  # experimental
evnx convert --to railway                 # experimental
evnx convert --to doppler                 # experimental
```

Choose and rename variables:

```bash
# Only the database settings
evnx convert --to json --include "DB_*,DATABASE_URL"

# Namespace every name
evnx convert --to json --prefix "MYAPP_" --transform uppercase
```

A filter that leaves nothing is an error unless `--allow-empty`, and two names
that become one in the output are an error unless `--allow-collisions`.

Pipe directly to AWS Secrets Manager (Linux and macOS; stdout is the JSON alone):

```bash
evnx convert --to aws-secrets | \
  aws secretsmanager create-secret \
    --name prod/myapp/config \
    --secret-string file:///dev/stdin
```

### Or move them into a secret manager

`evnx migrate` prints the commands that put your `.env` into a secret manager —
AWS, GCP, Azure Key Vault, Heroku, Vercel, Doppler, Infisical, Railway — for you to
read and run; evnx itself contacts none of them. GitHub Actions is the exception:
evnx uploads there directly (`--repo`, token from `GITHUB_TOKEN`).

```bash
evnx migrate --to heroku --heroku-app my-app --dry-run   # what would be sent
evnx migrate --to heroku --heroku-app my-app             # the command, to run yourself
evnx migrate --env-name production --to gcp-secret-manager --project my-gcp-project \
  --print-only > upload.sh                                # only the commands, as a script
```

The [migrate guide](https://docs.evnx.dev/cli/commands/migrate) lists every destination.

---

## Step 8 — Keep files in sync

`evnx sync` keeps `.env` and `.env.example` aligned as your project evolves. It compares
variable names and only adds; it never removes a variable or changes a value.

```bash
# Forward (the default): add new names from .env to .env.example, as placeholder values
evnx sync

# Reverse: fill .env from .env.example — the template's values, placeholders where it has none
evnx sync --direction reverse

# Your own check, both ways: exit 1 if either file has a name the other lacks
evnx sync --check

# In CI and pre-commit hooks, one way: exit 1 only when .env has a name the template lacks.
# 0 in sync, 1 drift, 2 no verdict. With no .env, the template is checked against the
# required [vars] in .evnx.toml.
evnx sync --check --direction forward
```

Generate `.env.example` from your existing `.env` — the forward direction creates it if it
is missing. Values become placeholders chosen from each name and value, never the real ones:

```bash
evnx sync

cat .env.example
# Generated from .env
# Replace all placeholder values with real credentials

APP_NAME=YOUR_VALUE_HERE
DATABASE_URL=postgresql://user:password@localhost:5432/dbname
AWS_ACCESS_KEY_ID=YOUR_KEY_HERE
PORT=8000
```

In a terminal, sync asks first (placeholders, or real values for keys that are not
credentials); without one, or with `CI` set, it uses placeholders and says so. Commit
`.env.example` to your repository. It tells teammates which variables are required without
exposing values.

---

## Step 9 — Render a config file from a template

`evnx template` fills a config file (YAML, TOML, JSON, a compose file) with the values in `.env`:

```yaml
# config.template.yml
database:
  host: {{DB_HOST}}
  port: {{DB_PORT|int}}
  ssl:  {{DB_SSL|bool}}
```

```bash
# Check that it renders, writing nothing; --strict fails on a variable with no value
evnx template --input config.template.yml --output config.yml --dry-run --strict

# Render it, and keep it out of git
evnx template --input config.template.yml --output config.yml --gitignore
```

The rendered file holds real values, so it is created readable by you only (0600). In a
deploy step, `--output -` prints it to stdout instead, and `--mkdir` creates a missing
output directory. An unknown filter is an error and nothing is written; the filters are
listed in `evnx template --help`.

---

## Set up the pre-commit hook

The pre-commit hook is the most important step for ongoing protection. It runs `evnx scan` on the files being committed, blocking a commit that contains secrets.

⚠️ Scan the **staged files**, never the working tree: the tree holds your real,
gitignored `.env`, so a hook that scans it blocks every commit.

### Using pre-commit

Add to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: local
    hooks:
      - id: evnx-scan
        name: Scan for secrets
        entry: evnx scan
        language: system
        pass_filenames: true    # the staged files — never the gitignored .env

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
# Scan what is being committed. The working tree holds your gitignored .env with
# real keys; scanning it would block every commit.
files=()
while IFS= read -r -d '' f; do files+=("$f"); done \
  < <(git diff --cached --name-only -z --diff-filter=ACM)
[ ${#files[@]} -eq 0 ] && exit 0
evnx scan "${files[@]}"
case $? in
  0) exit 0 ;;
  1) echo "evnx: secrets in the staged files — commit blocked."; exit 1 ;;
  *) echo "evnx: the scan could not complete — commit blocked."; exit 1 ;;
esac
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
 && evnx scan
```

---

## Configuration file

Store project-level defaults in `.evnx.toml` at the project root:

```toml
[defaults]
env_name = "production"        # which .env.<name> commands operate on
example  = ".env.template"     # the template path, if not .env.example

[scan]
severity            = "high"        # "high" | "medium" | "low"
exclude             = ["fixtures"]
ignore_placeholders = true

[validate]
strict           = true
validate_formats = true
fail_on          = "error"           # "error" | "warning": the lowest severity that exits 1

[sync]
naming_policy = "warn"

# What each variable *is* — read by validate, scan and sync. `evnx spec init` writes it.
[vars.PORT]
format      = "port"
description = "HTTP listen port"

[vars.STRIPE_KEY]
secret       = true
environments = ["production"]
```

Precedence is **flag > config > default**, and lists combine rather than replace. Only
`.evnx.toml`, with the leading dot, is read; a bare `evnx.toml` stops every command with the
rename to make.

⚠️ **On v0.4.x and earlier this file did nothing** — the loader existed and nothing called
it. Keys named in older copies of this page (`env_file`, `auto_fix`, `exclude_patterns`,
`[convert]`, `[aliases]`) were never read by anything and do not exist.

Full reference: [docs.evnx.dev/cli/reference/configuration-file](https://docs.evnx.dev/cli/reference/configuration-file)

---

## Next steps

| Topic | Link |
|---|---|
| All command guides | [docs.evnx.dev/cli](https://docs.evnx.dev/cli) |
| evnx init in depth | [docs.evnx.dev/cli/commands/init](https://docs.evnx.dev/cli/commands/init) |
| evnx add in depth | [docs.evnx.dev/cli/commands/add](https://docs.evnx.dev/cli/commands/add) |
| Prevent secret leaks | [docs.evnx.dev/cli/use-cases/prevent-secret-leaks](https://docs.evnx.dev/cli/use-cases/prevent-secret-leaks) |
| GitHub Actions integration | [docs.evnx.dev/cli/integrations/github-actions](https://docs.evnx.dev/cli/integrations/github-actions) |
| Migrate to AWS Secrets Manager | [docs.evnx.dev/cli/use-cases/use-cases-aws](https://docs.evnx.dev/cli/use-cases/use-cases-aws) |
| Team collaboration with sync | [docs.evnx.dev/cli/use-cases/team-collaboration](https://docs.evnx.dev/cli/use-cases/team-collaboration) |
| .evnx.toml reference | [docs.evnx.dev/cli/reference/configuration-file](https://docs.evnx.dev/cli/reference/configuration-file) |
| Security model | [docs.evnx.dev/cli/reference/concepts-security-model](https://docs.evnx.dev/cli/reference/concepts-security-model) |
| Changelog | [CHANGELOG.md](../CHANGELOG.md) |

---

[Website](https://www.evnx.dev) | [GitHub](https://github.com/urwithajit9/evnx) | [Issues](https://github.com/urwithajit9/evnx/issues)