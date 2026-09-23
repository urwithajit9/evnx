# evnx

[![CI](https://github.com/urwithajit9/evnx/workflows/CI/badge.svg)](https://github.com/urwithajit9/evnx/actions)
[![Release](https://img.shields.io/github/v/release/urwithajit9/evnx)](https://github.com/urwithajit9/evnx/releases)
[![crates.io](https://img.shields.io/crates/v/evnx.svg)](https://crates.io/crates/evnx)
[![PyPI](https://img.shields.io/pypi/v/evnx.svg)](https://pypi.org/project/evnx/)
[![npm](https://img.shields.io/npm/v/@evnx/cli.svg)](https://www.npmjs.com/package/@evnx/cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![GitHub Marketplace](https://img.shields.io/badge/Marketplace-evnx--action-blue?logo=github)](https://github.com/marketplace/actions/evnx-env-security-validation)

A CLI tool for managing `.env` files — validation, secret scanning, format conversion, and migration to cloud secret managers.

[Website](https://www.evnx.dev) | [Getting Started](./docs/GETTING_STARTED.md) | [Changelog](./CHANGELOG.md)

---

## Why evnx?

Accidentally committing secrets to version control is one of the most common and costly developer mistakes. evnx is a local-first tool that catches misconfigurations, detects credential leaks, and converts environment files to the format each deployment target expects — before anything reaches CI or production.

---
## Testing & playground

→ [urwithajit9/evnx-test](https://github.com/urwithajit9/evnx-test) —
try evnx in your browser via GitHub Actions, no installation required.

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

#### Scoop (user-local, no admin required)

```powershell
scoop bucket add evnx https://github.com/urwithajit9/scoop-evnx
scoop install evnx
```

#### Winget (system-wide)

```powershell
winget install urwithajit9.evnx
```

#### Cargo (Windows)

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
evnx add                          # interactive
evnx add service postgresql       # a known service, non-interactively
evnx add framework nextjs
```

---

### `evnx spec`

Declares what each variable **is** — something `.env.example` cannot express. Written
into `[vars]` in `.evnx.toml`, and read by `validate`, `scan` and `sync`.

```bash
evnx spec init                    # infer the contract from files you already have
```

```toml
[vars]
PORT = { format = "port", description = "HTTP listen port" }

[vars.TENANT_A]
secret = true                     # scan reports it; no heuristic could
environments = ["production"]     # required in production only
```

`secret = true` reports a credential whose name gives nothing away. `secret = false`
retracts a guess made from a variable's **name** — but never a match on its **value**,
so a variable holding `sk_live_…` is still reported however it is declared.

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
evnx scan                              # scan current directory, recursively
evnx scan src/ config/                 # specific paths (positional, repeatable)
evnx scan --severity high              # only what evnx is confident about
evnx scan --pattern 'ACME-[A-Z0-9]{32}'  # your own secret format
evnx scan --format sarif               # SARIF for the GitHub Security tab
evnx scan --exit-zero                  # warn but do not fail CI
```

Detects: AWS Access Keys, Stripe keys (live and test), GitHub tokens, OpenAI and Anthropic API keys, RSA/EC/OpenSSH private keys, high-entropy strings, and generic API key patterns — plus any format you declare with `--pattern` or `[[scan.patterns]]`.

**Exit codes are `0` clean, `1` secrets found, `2` the scan could not be completed.** `2` is not a louder `1`: "I found nothing" and "I could not look" are different answers. `--exit-zero` suppresses `1` only.

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

## Working with multiple environments

`--env-name` resolves `.env.<name>` across `validate`, `scan`, `diff`, `sync`, `convert`,
`backup` and `template`:

```bash
evnx validate --env-name production            # operates on .env.production
evnx diff --env-name production --against staging
```

A name whose file does not exist is an error listing the ones that do — it never quietly
falls back to `.env`. Set a project default with `[defaults] env_name` in `.evnx.toml`.

Comparing two real environments is the useful diff; the default compares against the
template, where every filled-in value differs by construction.

---

## Cloud sync _(requires `--features cloud`)_

Push your `.env` to the cloud and pull it on any machine or in any pipeline — with
the server **mathematically unable** to read it.

Encryption and decryption happen on your machine. The server stores ciphertext and
holds no key that can open it: not with full database access, not with a court
order, not after a breach.

```bash
evnx auth register                       # create an account
evnx auth login                          # sign in on this machine
evnx vault create app --env production   # a vault holds one .env, versioned
evnx cloud link app/production           # bind this directory to it
evnx cloud push                          # encrypt locally, upload ciphertext
evnx cloud pull                          # download, decrypt, write .env
```

### Installing with cloud support

**The prebuilt binaries already include the cloud commands.** npm, PyPI, Homebrew,
Scoop, winget and the GitHub Release are all built with `--all-features`, so if you
installed evnx any of those ways, `evnx cloud --help` already works.

`cargo install` is the one exception, because it compiles from source with
`default = []`:

```bash
cargo install evnx --features cloud
```

> ⚠️ **If you already installed evnx another way, check which binary you are running.**
> A package-manager install often sits earlier in `PATH` than `~/.cargo/bin`, so
> `cargo install` appears to succeed while the old binary still answers — and
> `evnx vault list` reports *"unrecognized subcommand"*.
>
> ```bash
> which -a evnx     # if ~/.cargo/bin/evnx is not first, that is why
> ```

### What the server can and cannot see

| Sent to the server | Never sent |
|---|---|
| Ciphertext — AES-256-GCM, encrypted before upload | Your `.env` values |
| Your **key names** (`DATABASE_URL`, `STRIPE_KEY`) so a listing can show what a vault holds | Your master password |
| An SRP-6a verifier, which proves knowledge of the password without revealing it | Your master key, or any vault key in usable form |

Key names travelling in the clear is a deliberate trade for a usable listing. If a
name is itself sensitive, do not make it a name.

**There is no password reset.** The server holds only ciphertext, so nobody —
including us — can recover an account whose master password is lost. Enable a
second factor and keep your recovery codes:

```bash
evnx auth totp enable
```

### Versioning and rollback

Every push is a new version. Nothing is overwritten.

```bash
evnx cloud history                  # what changed, when, and who pushed it
evnx cloud pull --version 3         # restore an earlier version
```

If someone else pushed since you last pulled, your push is refused with a conflict
rather than silently overwriting them. Pull, re-apply, push again — the version
number is authenticated into the ciphertext, so the same bytes cannot simply be
re-sent.

### Running a command without writing a file

```bash
evnx cloud run --vault app/production -- ./deploy.sh
evnx cloud run --include 'NEXT_PUBLIC_*' -- npm run build
```

Decrypts in memory and hands the variables to the child process. Nothing on disk,
nothing in shell history, and **nothing in `ps`** — values travel in the environment
block, never the argument vector. The child's exit code comes back unchanged, so a
pipeline can branch on it.

`--include` / `--exclude` narrow what the child sees; a filter that matches nothing is
an error rather than a silent run with zero secrets.

⚠️ This removes the plaintext *file*, not the master password. The vault key is wrapped
under your master key, so decryption needs it wherever the command runs. In CI, feed it
with `--password-stdin`.

### Sharing with a team

```bash
evnx vault share app/production --with teammate@example.com --role developer
evnx vault members app/production
evnx vault role app/production --user teammate@example.com --role viewer
evnx vault revoke app/production --user teammate@example.com
```

The vault key is wrapped for the recipient with a **hybrid X25519 + ML-KEM-768** scheme,
so a share is not opened by a future quantum computer harvesting today's traffic. There is
no X25519-only path; sharing with an account that has no ML-KEM key on file is refused
rather than downgraded.

Revocation re-keys the vault, so a removed member cannot open versions pushed after they
left.

⚠️ The recipient's public keys come from the server. Out-of-band fingerprint verification
is not built, so "the server cannot read your secrets" becomes "…cannot read them
*passively*" the moment you share.

### CI/CD

Mint a token scoped to one vault, read-only:

```bash
evnx auth token create ci --scope read --vault app/production --expires-in-days 90
```

Then in the pipeline:

```bash
export EVNX_TOKEN=evnx_tok_...
echo "$EVNX_PASSWORD" | evnx cloud pull --vault <VAULT_ID> --password-stdin
```

Two things to understand:

- **The token authenticates; it does not decrypt.** The master password is still
  required, and it is the more sensitive of the two. What the token buys is blast
  radius: scoped to one vault and `read`, it reaches that vault and nothing else.
- **Use the vault id, not `name/environment`.** A vault-scoped token is refused
  permission to list vaults — by design — so it cannot resolve a name. Get the id
  from `evnx vault list --verbose`.

Pipe the password rather than exporting it where you can: an environment variable
is visible in process listings and tends to end up in logs.

### Self-hosting

`evnx cloud` talks to `https://api.evnx.dev` by default. Point it anywhere:

```bash
evnx cloud status --server https://evnx.internal.example
```

or set `EVNX_SERVER`, or `server = "..."` in `~/.config/evnx/config.toml`. The
server is [open source](https://github.com/urwithajit9/evnx-server) and ships a
Docker stack. Plain `http://` is refused for anything but a loopback address,
because tokens travel in request headers.

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

## pre-commit / prek Integration

Use evnx as automatic git hooks via [pre-commit](https://pre-commit.com)
or [prek](https://prek.j178.dev) — no manual evnx install needed.
The binary is compiled and cached automatically on first run.

Add to `.pre-commit-config.yaml`:

```yaml
default_install_hook_types: [pre-commit, pre-push]

repos:
  - repo: https://github.com/urwithajit9/evnx
    rev: v0.3.6
    hooks:
      - id: evnx-scan        # blocks commit if secrets found
      - id: evnx-validate    # blocks commit if .env misconfigured
      - id: evnx-diff        # warns on .env/.env.example drift
      - id: evnx-scan-push   # strict scan on push
```

Then install:
```bash
pre-commit install   # or: prek install
```

That's it. On your next commit, hooks will auto-compile and run.

---

## Configuration

Store defaults in `.evnx.toml` at the project root:

```toml
[defaults]
env_name = "production"        # which .env.<name> commands operate on
example  = ".env.template"     # the template path, if not .env.example

[scan]
severity            = "high"        # "high" | "medium" | "low"
exclude             = ["fixtures"]  # paths this project never scans
ignore_placeholders = true

# Your own secret formats, beyond the built-in ones. Repeat per rule.
[[scan.patterns]]
name  = "Acme API key"
regex = "ACME-[A-Z0-9]{32}"

[validate]
strict           = true
validate_formats = true
ignore           = ["boolean_trap"]

[sync]
naming_policy = "warn"         # "warn" | "error" | "ignore"

[diff]
ignore_keys = ["BUILD_ID"]

[backup]
keep = 3

[cloud]
vault = "my-app/production"    # written by `evnx cloud link`

# What each variable *is* — read by validate, scan and sync.
[vars]
PORT       = { format = "port", description = "HTTP listen port" }
STRIPE_KEY = { secret = true, environments = ["production"] }
```

A setting belongs here if it is true of the **project** and would otherwise be repeated
on every invocation. It does not if it expresses what *this* invocation should do — which
is why `[scan] severity` is configurable and `--exit-zero` is not.

Precedence is **flag > config > default**. Lists combine rather than replace, so
`--exclude dist` scans with `dist` *and* the project's excludes. Settings that can weaken
scanning are announced on every run. Unknown keys warn and never fail, so a file written
for a later evnx keeps working on an earlier one.

Full reference: <https://www.evnx.dev/guides/reference/configuration-file>

⚠️ **On v0.4.x and earlier this file did nothing.** The loader existed and nothing called
it, so a project with `[validate] strict = true` validated non-strictly and said nothing.
Keys named in older copies of this README — `env_file`, `auto_fix`, `exclude_patterns`,
`[convert]`, `[aliases]` — were never read by anything and do not exist.

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

**Multiple environments are supported as of v0.5.0.** `evnx scan` reads every `.env`
variant it walks past, and `--env-name production` resolves to `.env.production` across
`validate`, `scan`, `diff`, `sync`, `convert`, `backup` and `template`.

⚠️ Before v0.5.0, `evnx scan` silently skipped `.env.production`, `.env.local`,
`.env.staging` and every other dotted variant, and reported "no secrets detected" without
opening them. If you relied on `evnx scan` as a gate on an earlier version, it never saw
the file most likely to hold production credentials.

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