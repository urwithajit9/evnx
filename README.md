# evnx

[![CI](https://github.com/urwithajit9/evnx/workflows/CI/badge.svg)](https://github.com/urwithajit9/evnx/actions)
[![Release](https://img.shields.io/github/v/release/urwithajit9/evnx)](https://github.com/urwithajit9/evnx/releases)
[![crates.io](https://img.shields.io/crates/v/evnx.svg)](https://crates.io/crates/evnx)
[![PyPI](https://img.shields.io/pypi/v/evnx.svg)](https://pypi.org/project/evnx/)
[![npm](https://img.shields.io/npm/v/@evnx/cli.svg)](https://www.npmjs.com/package/@evnx/cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![GitHub Marketplace](https://img.shields.io/badge/Marketplace-evnx--action-blue?logo=github)](https://github.com/marketplace/actions/evnx-env-security-validation)

A CLI tool for managing `.env` files — validation, secret scanning, format conversion, and
migration to cloud secret managers. It also syncs them end-to-end encrypted, so the server
stores only ciphertext it cannot read — with a limit worth knowing before you rely
on it, in [Cloud sync](#cloud-sync-requires---features-cloud).

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
evnx commands      # the features this build has, and every command
```

`--version` is the same whether or not a feature is compiled in; `evnx commands`
says `built with: net, migrate, backup, cloud` for a full build.

---

## Commands

Commands exit **0** when they ran and found nothing, **1** when they ran and the
answer is no (secrets found, drift, a failed upload), and **2** when they could
not run — "No verdict: evnx did not finish". `backup` and `restore` are the
exception: they keep the specific codes listed in their section below (`restore`
exits 3 when the backup file is missing) and never exit 1. `-q` prints nothing on success; findings and errors
still print. ⚠️ Not yet in the cloud commands: `org`, `vault`, `cloud push`/`pull`/
`export`/`audit` and `auth`'s token, session and device commands still print their
progress lines under `-q` (`auth status`, `cloud status` and `cloud unlink` are quiet).

### `evnx init`

Interactive project setup. Creates `.env`, the project's template (`.env.example`) and the `.evnx.toml` contract through a guided TUI.

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

No flags are required: the interactive flow handles stack and service selection. For scripts and CI, every choice also has a flag:

| Flag | Does |
|---|---|
| `--yes`, `-y` | no prompts; blank files unless `--blueprint` or `--with` is given; in a fresh clone, keeps the template and creates `.env` from it |
| `--blueprint <id>` | a named stack (an unknown id lists the ten valid ones) |
| `--with <a,b,c>` | name the components directly |
| `--list-components` | print every name `--with` accepts |
| `--detect` | accept what `package.json`, compose files and other manifests declare |
| `--from-source` | add the variables the code reads; never removes |
| `--force` | replace an existing template (`.env` is never overwritten); not with `--from-source` |
| `--path <dir>`, `-p` | write somewhere other than `.` |
| `--dry-run` | say what would be written (template, `.env`, `.gitignore`, `.evnx.toml`); write nothing |
| `--json` | the result as one JSON document, the fields `evnx add --json` prints (with `--yes` or `--dry-run`) |

Short names work too (`postgres`, `pg`, `mongo`, `next`, `k8s` and others; see `--list-components`), and browser frameworks get prefixed copies of public values (`VITE_SUPABASE_URL` next to `SUPABASE_URL`); a `NEXT_PUBLIC_` name is written only on Next.js.

What init writes, in a project that already has files:

- **The template the project uses**: `[defaults] example` in `.evnx.toml`, else `.env.example`, `.env.sample` or `.env.template`, else a new `.env.example`. It is replaced only with `--force`.
- **`.env`**, created `0600`, or, when it exists, given the names the new template has and it lacks; a value already there is never touched.
- **`.evnx.toml`**: created with one `[vars]` entry per variable (in the folder `--path` names, also below the repository root), or the new names recorded into the one in force, as `evnx add` records them. Commit it with the template.
- Other env files (`.env.production`, `.env.local` …) are left alone and listed, with the command that checks one; `[defaults] env_name` is named when `.env` is not the file it points at.

With `CI` set, init asks nothing: a run that would show the menu stops with exit 2 and names the flags. Full reference: [docs.evnx.dev/cli/commands/init](https://docs.evnx.dev/cli/commands/init).

---

### `evnx add`

Add the variables a service, framework or whole stack needs to `.env.example` and `.env`, without retyping them. Same catalogue and section headings as `init`. Names already there are skipped, and a value you have set is never replaced. Each subcommand shows a preview and asks first; `--yes` skips the question (scripts and CI need it).

```bash
evnx add service postgresql       # any catalogue id: a service, an infra target or a framework (`add infra` is an alias)
evnx add framework nextjs         # --language is optional
evnx add blueprint t3_modern      # a whole stack
evnx add custom API_URL=https://api.example.com DEBUG=false --yes   # your own: values go to .env only, .env.example gets placeholders
evnx add custom                   # ... or one prompt at a time
evnx add --list                   # every id
evnx add service redis --dry-run  # what would be written; --json for scripts
```

`--path` names the folder that holds `.env.example` and `.env`. When the project keeps a
contract, the new names are recorded in the nearest `.evnx.toml` at or above that folder,
which may be the repository root rather than the folder itself.

---

### `evnx spec`

Declares what each variable **is** — something `.env.example` cannot express. Written
into `[vars]` in the project's `.evnx.toml`, and read by `validate`, `scan` and `sync`;
`evnx init` writes a first one, and `evnx init` and `evnx add` record the variables they
introduce there, never rewriting an entry.

```bash
evnx spec init                            # infer the contract from .env.example and .env
evnx spec init --env-name staging         # sample values from .env.staging
evnx spec init --with django,postgresql   # or start from the component catalogue
evnx spec init --stdout                   # print it instead (only TOML goes to stdout)
evnx spec init --merge                    # later: add only the names [vars] does not declare yet
```

Every field is a guess from your files — read it before you commit it. A `format` is
written only when the sample value passes it, so the first `validate` afterwards passes;
any it left out are named. Names only in `.env` follow the template's, marked
`# only in .env` with `required = false`. `--env`, `--env-name`, `--example` and
`[defaults]` work as they do for `validate`; a file named by a flag must exist.

It writes the project's one `.evnx.toml` — the one in force at or above this folder, else
a new one at the git root — and refuses a second `[vars]` unless `--merge`, which keeps
every existing entry as written. Nothing is ever written that TOML would refuse to read.
Exit 0 when the contract was written or printed, 2 when nothing was written.

```toml
[vars.PORT]
format      = "port"              # url, int, port, bool, email; anything else is a regex
description = "HTTP listen port"

[vars.TENANT_A]
secret       = true               # scan reports it; no heuristic could
environments = ["production"]     # required in production only — not in a plain .env either
```

One `[vars.NAME]` table per variable is the documented form. The inline
`PORT = { format = "port" }` form under `[vars]` is read too, but `spec init --merge` and
`evnx add` cannot extend it. A key evnx does not know (`requird = false`) is a warning on
every command and an error in `validate`. `bool` accepts `true yes on 1 y` and
`false no off 0 n`, in any case; a regex is anchored only if you anchor it.

`secret = true` reports a credential whose name gives nothing away. `secret = false`
retracts a guess made from a variable's **name** — but never a match on its **value**,
so a variable holding `sk_live_…` is still reported however it is declared.

---

### `evnx validate`

Validates your `.env` file for common misconfigurations before deployment.

```bash
evnx validate                            # pretty output; exit 1 on errors
evnx validate --strict                   # also report variables nothing declares
evnx validate --fail-on warning          # warnings exit 1 too
evnx validate --fix --dry-run            # what --fix would change; writes nothing
evnx validate --format json              # machine-readable output
evnx validate --format github-actions    # inline GitHub annotations
```

Detects: missing required variables, placeholder values (`YOUR_KEY_HERE`, `CHANGE_ME`), the boolean string trap (`DEBUG="False"` is truthy in most runtimes), weak secret keys, duplicate keys, `localhost` in a project with Docker files, an env file `.gitignore` does not cover (a warning) or git already tracks (an error), and, with `--validate-formats`, malformed URLs, ports and emails. Warnings alone exit 0 unless `--fail-on warning`.

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
evnx scan --extensions env,yml,json    # opt-in: read only these files in folders (the run says so)
```

A folder scan reads **every** file. Binary files and files over 1 MiB
(`--max-file-size`) are skipped, and the scan says how many (`-v` lists them). A
file you name is always read.

**In a git repository a folder scan counts what git would commit.** A filled-in
`.env` that `.gitignore` covers is where secrets belong: its findings are summarised
in one line and do not fail the run. A tracked file, or one the next `git add .`
would take, is reported. Name the file (`evnx scan .env`) or add `--include-ignored`
to see every finding.

Detects: AWS access keys (long-term and STS), GitHub tokens (classic and
fine-grained), Slack tokens, Google API keys, JSON Web Tokens, Stripe keys (live and
test), OpenAI and Anthropic API keys, private keys (RSA, EC, OpenSSH, PKCS#8),
passwords in connection strings, values in variables named like secrets, and
high-entropy strings — plus any format you declare with `--pattern` or
`[[scan.patterns]]`. `evnx scan --help` lists the rule names `[scan] disable` takes.

**Exit codes are `0` clean, `1` secrets found, `2` the scan could not be completed.** `2` is not a louder `1`: "I found nothing" and "I could not look" are different answers. `--exit-zero` suppresses `1` only.

---

### `evnx diff`

Compares two env files. Against the template (`.env.example`) it compares **key names
only** — which keys `.env` lacks and which ones the template does not declare — because
every filled-in value differs from its placeholder. Between two environments
(`--against`) it compares values too.

```bash
evnx diff                     # keys missing from .env, or not in .env.example
evnx diff --fail-on missing   # exit 1 only when .env lacks a key the template has
evnx diff --env-name production --against staging   # two environments: keys and values
evnx diff --values            # compare values against the template too
evnx diff --show-values       # print values (secrets stay masked, e.g. postgres://admin:***@db)
evnx diff --reverse           # swap comparison direction
evnx diff --ignore-keys BUILD_ID   # leave keys out (exact case; adds to [diff] ignore_keys)
evnx diff --format json       # JSON output (schema_version 1)
evnx diff --format patch --show-values   # values are <withheld> without --show-values
```

Exit codes: `0` nothing `--fail-on` counts (default: missing, extra, different), `1`
differences found, `2` could not run. ⚠️ `--format patch --show-values` and
`--interactive --show-values` print real secrets; without `--show-values` they print
`<withheld>`. `--interactive` writes nothing yet.

---

### `evnx convert`

Converts your `.env` file to the format one of 14 deployment targets expects.
`--help` lists them; `k8s`, `tf`, `yml`, `gh-actions` and `compose` are aliases.

```bash
evnx convert --to json
evnx convert --to yaml
evnx convert --to shell
evnx convert --to docker-compose
evnx convert --to kubernetes --name myapp-env [--namespace prod]
evnx convert --to terraform
evnx convert --to github-actions        # text to paste into the GitHub web form
evnx convert --to aws-secrets           # JSON for create-secret --secret-string
evnx convert --to gcp-secrets --project my-gcp-project
evnx convert --to azure-keyvault --vault-name my-vault
evnx convert --to heroku --heroku-app my-app
evnx convert --to vercel                # experimental
evnx convert --to railway               # experimental
evnx convert --to doppler               # experimental
```

The formats that write a place into their output need it named: `--project`,
`--vault-name`, `--heroku-app`, `--name`. There is no placeholder to edit by
hand. The `gcp-secrets` script creates each secret, or adds a new version when
it already exists. `doppler`, `vercel` and `railway` are experimental: their
shapes have not been checked against those services' importers, and evnx says
so on stderr. For Doppler, `doppler secrets upload` documents the flat JSON
that `--to json` prints.

Choose and rename variables:

```bash
# Only the database settings, for a per-service Kubernetes Secret
evnx convert --to kubernetes --name db-env --include "DB_*,DATABASE_URL" -o db-secret.yaml

# Namespace every name for a shared platform
evnx convert --to json --prefix "MYAPP_" --transform uppercase
```

`--include` and `--exclude` take globs (`*` for any run, `?` for one
character), matched case-sensitively; repeat the flag or separate patterns
with commas, and exclude wins. A filter that leaves nothing, or an empty
`.env`, is an error (exit 2) unless `--allow-empty`. So is a pair of names that
become one name in the output — `Api` and `API` under `--transform uppercase`,
or `A_B` and `a_b` for gcp-secrets, terraform, Azure Key Vault or GitHub —
unless `--allow-collisions` keeps the last value.

`--output` creates the file readable by you only (0600) and refuses to replace
an existing one unless `--force` is given. It never writes over the `.env` it
reads. In a git repository it checks `.gitignore` for the file, as
`evnx template` does: it asks with a terminal, warns without one, and
`--gitignore` / `--no-gitignore` decide without asking.

Pipe directly to AWS Secrets Manager:

```bash
evnx convert --to aws-secrets | \
  aws secretsmanager create-secret \
    --name prod/myapp/config \
    --secret-string file:///dev/stdin
```

stdout carries the JSON and nothing else (notices go to stderr), so the
secret goes straight down the pipe. `file:///dev/stdin` works on Linux and
macOS, not on Windows. evnx does not run `aws` itself.

---

### `evnx sync`

Keeps `.env` and `.env.example` aligned by variable **name**. It only ever adds: it never
removes a variable or changes a value (comparing values between two environments is
`evnx diff`).

```bash
# Forward (the default): add .env's new names to .env.example, as placeholder values
evnx sync

# Reverse: add the template's names to .env — its values, placeholders where it has none
evnx sync --direction reverse

# Preview: write nothing, ask nothing, exit 0
evnx sync --dry-run

# Your own check, both ways: names the template lacks, and names only the template has
evnx sync --check

# CI and pre-commit: one way — exit 1 only when .env has a name the template lacks.
# With no .env, the template is checked against the required [vars] in .evnx.toml.
evnx sync --check --direction forward
```

| Flag | What it does |
|---|---|
| `--direction forward\|reverse` | Which file to fill (default `forward`). With `--check`, naming one makes the check one-way |
| `-n`, `--dry-run` | Preview; writes nothing, never prompts, exits 0 |
| `--check` | Exit 0 in sync, 1 out of sync (the message names the command that fixes each side), 2 no verdict. Implies `--dry-run` |
| `--format json` | With `--check`: `missing`, plus `template_only` when no `--direction` is given |
| `-f`, `--force` | Skip the prompts and add placeholder values. With no terminal, or `CI` set, sync does this anyway and says so |
| `--env`, `--env-name`, `--example` | The pair to compare (default `.env` and `.env.example`) |
| `--template-config FILE` | JSON placeholder rules: `patterns`, `default`, `allow_actual` (real values for configuration such as `LOG_LEVEL`; a credential is refused). An unknown key is an error |
| `--naming-policy warn\|error\|ignore` | Names that are not `UPPER_SNAKE_CASE`; `error` stops with exit 2 |
| `--skip-without-env` | For a pre-commit hook: with no `.env` and no spec, skip (exit 0) and say so |
| `--placeholder` | Deprecated; does what `--force` does |

---

### `evnx migrate` _(requires `--features migrate`)_

Moves a `.env` into a secret manager. For **eight** destinations evnx **prints the
commands** for you to read and run, and contacts nothing; only **`github-actions`
uploads**, over the GitHub API.

| Destination | What evnx prints or does | Needs |
|---|---|---|
| `aws-secrets-manager` | `create-secret` and `update-secret`, all keys as one JSON secret | `--secret-name` |
| `gcp-secret-manager` | a script that creates each secret or adds a version | `--project` |
| `azure-keyvault` | `az keyvault secret set` per key (`_` becomes `-`) | `--vault-name` |
| `heroku` | one `heroku config:set` (one restart) | `--heroku-app` |
| `vercel` | `vercel env add … --force` per key, value on stdin | `--vercel-project` or a linked directory |
| `doppler`, `infisical` | `secrets set` per key | `--project`, optional config / env |
| `railway` | one `railway variable set` | `--railway-project` with `--railway-environment`, or a linked directory |
| `github-actions` | uploads each secret, encrypted for the repository | `--repo`, `GITHUB_TOKEN` |

```bash
# Print the commands, read them, run them
evnx migrate --to aws-secrets-manager --secret-name prod/myapp/config

# Only the commands on stdout: a script to review, then run
evnx migrate --to gcp-secret-manager --project my-gcp-project --print-only > upload.sh

# Another environment: reads .env.production ([defaults] env_name works too)
evnx migrate --env-name production --to heroku --heroku-app my-app

# From the process environment: --include is required, and evnx lists what it sends
evnx migrate --from environment --include 'APP_*,DATABASE_URL' --to heroku --heroku-app my-app

# Preview what would happen: nothing printed to run, nothing uploaded
evnx migrate --to doppler --project myapp --dry-run

# GitHub Actions from CI. The token comes from GITHUB_TOKEN; --github-token works
# but puts it in shell history. --yes skips the question; existing secrets still
# need --overwrite or --skip-existing. Exit 1 if any upload failed.
evnx migrate --to github-actions --repo owner/repo --yes --skip-existing
```

`--include` / `--exclude` take comma-separated globs; `--strip-prefix` and `--add-prefix`
rename. Two names that end up as one — by renaming, or because Azure Key Vault and GitHub
ignore case — are refused (exit 2) unless `--allow-collisions` keeps the last value.
`--skip-existing` and `--overwrite` only apply to `github-actions`; the destinations evnx
prints commands for are refused them, since evnx cannot see what is already there.
Exit codes: 0 done, 1 some GitHub uploads failed, 2 could not run.

---

### `evnx doctor`

Runs a health check on your environment configuration setup.

```bash
evnx doctor                          # check current directory
evnx doctor path/to/project          # or --path path/to/project
evnx doctor --fix                    # repair .gitignore and file modes, then check again
evnx doctor --strict --format json   # a CI gate: warnings fail too
evnx doctor --skip-without-env       # a pre-commit hook: a fresh clone with no .env passes
```

Checks, for every env file (`.env` and `.env.<name>`, and plaintext copies such as
`.env.bak`; not `.envrc`): git ignores it and does not track it, it is valid dotenv, and
no other user can read it (mode 600). Also: the template exists and is tracked, the
project type, and Docker files. The template is `[defaults] example` in `.evnx.toml` if
set, else the first of `.env.example`, `.env.sample`, `.env.template`. A tracked env
file, an unignored one and a syntax error are errors; a readable file and a missing
template are warnings.

`--fix` adds the `.env*` entries `evnx init` writes to `.gitignore` and sets each
readable env file to 600 (a symlinked one only when its target is inside the project).
It checks again afterwards and says "not fixed" when the problem remains. It never runs
git: a tracked file needs `git rm --cached`, which doctor prints.

`EVNX_AUTO_FIX=1` does what `--fix` does, and says so on stderr; `EVNX_OUTPUT_JSON=1`
does what `--format json` does. The JSON report has `checks[]` (`name`, `description`,
`severity`, `details`, `fixable`, `fixed`, `ran`) and `summary`.

Exit codes: 0 healthy (warnings pass unless `--strict`), 1 problems found, 2 could not
run — a bad path or flag, a folder with nothing to diagnose, or a check that could not
finish (`ran: false`).

---

### `evnx template`

Renders a config file from a template, filling in values from `.env`.

```bash
evnx template \
  --input config.template.yml \
  --output config.yml \
  --env .env
```

The output usually holds real values, so a new file is created readable by you
only (0600; `--mode 0640` to choose another). An existing output is replaced and
keeps its mode. `--output -` prints to stdout instead of writing a file. The
template and the env file are never overwritten.

| Option | What it does |
|---|---|
| `--input FILE`, `--output FILE` | the template, and where to write (`-` for stdout) |
| `--env FILE`, `--env-name NAME` | the values: `.env` by default, `.env.NAME` with `--env-name` |
| `--strict` | refuse to write when a placeholder has no value (the default warns, writes it through and exits 0) |
| `--dry-run` | render and say what would be written; writes nothing, prints no value, leaves `.gitignore` alone |
| `--mkdir` | create the output's missing directories (0700) |
| `--mode MODE` | the output's file mode, new or existing |
| `--gitignore` / `--no-gitignore` | add the output to `.gitignore` without asking / skip the check. Neither: ask in a terminal, warn in CI |

References are `{{VAR}}`, `${VAR}` and `$VAR`. Filters, one per placeholder:

```yaml
database:
  host: {{DB_HOST}}
  port: {{DB_PORT|int}}
  ssl:  {{DB_SSL|bool}}
  name: {{DB_NAME|upper}}
```

| Filter | Result |
|---|---|
| `upper`, `lower` | the value in upper or lower case |
| `title` | Title Case: `hello wORLD` → `Hello World` |
| `capitalize` | first character only: `hello wORLD` → `Hello wORLD` |
| `bool` | `true`/`yes`/`on`/`1`/`y` → `true`, `false`/`no`/`off`/`0`/`n` → `false` (any case); anything else is an error |
| `int` | a whole number; anything else is an error |
| `json` | a quoted, escaped JSON string |
| `default:VALUE` | `VALUE` when the variable is empty or not defined |

An unknown filter, or two chained (`{{X|upper|json}}`), is an error naming its
line, and nothing is written. Exit codes: 0 rendered, 2 nothing written.

---

### `evnx backup` / `evnx restore` _(requires `--features backup`)_

`backup` encrypts a `.env` into one file (AES-256-GCM, key from Argon2id over a
passphrase you choose, mode 0600); `restore` decrypts it. No account, no network.

```bash
evnx backup                                  # .env → .env.backup, asks for a passphrase twice
evnx backup --env-name production --verify   # .env.production → .env.production.backup, then re-read it
evnx restore .env.backup --inspect           # variable names only, never values
evnx restore .env.backup --dry-run           # decrypt and check, write nothing
evnx restore .env.backup --output .env       # asks before replacing an existing .env
```

**The passphrase** comes from `--key-file` (also spelled `--password-file`), then
`EVNX_PASSWORD`, then a prompt. A new backup needs at least 8 characters from any
of them; `restore` takes any length, so an older backup made with a shorter one
still opens. A key file is trimmed of surrounding whitespace, or Base64-encoded
when it is binary. When both a key file and `EVNX_PASSWORD` are set, the file
wins and a warning says so.

**Rotation:** each `backup` shifts the previous ones to `.1`, `.2` … up to
`--keep` (default 3, or `[backup] keep` in `.evnx.toml`); the one at the last
place is replaced, and older files are left alone. `--keep 0` overwrites. The
output may not be the source file, or a directory.

**Replacing a file:** `restore` never replaces an existing output silently. With
a terminal it asks; in a script pass `--yes` (alias `--force`). Either way the
replaced file is kept as `<output>.bak`. An output that already holds exactly the
backup's content is left alone. Content that is not a `.env` evnx can read goes
to `<output>.restored`, exit 5. In a git repository the restored file gets the
same `.gitignore` check as `template` (`--gitignore`, `--no-gitignore`).

In CI:

```bash
EVNX_PASSWORD="$BACKUP_PASSPHRASE" evnx backup --output "backups/env-$(date +%F).backup" --keep 0 --verify
evnx restore backups/env-2026-10-20.backup --key-file /run/secrets/backup-key --yes
```

| Exit | `backup` | `restore` |
|---|---|---|
| 0 | written (and verified) | restored, inspected, checked, or already up to date |
| 2 | could not run: source, key file or passphrase, output path, no terminal | could not run, or could not decrypt (wrong password, truncated, a newer format) |
| 3 | passphrase confirmation did not match | backup file not found |
| 4 | encryption failed | cancelled at the overwrite question |
| 5 | could not write the backup | content written to `<output>.restored` |
| 6 | `--verify` failed | — |

**A backup is not a vault.** They solve different problems and do not read each
other's files:

| | `backup` / `restore` | `cloud push` / `cloud pull` |
|---|---|---|
| Where the ciphertext lives | a file you keep | the evnx server, as versions |
| Secret | a passphrase or key file you choose | your account's master password |
| History | `--keep N` numbered files | server versions: `--version N`, `history`, `diff` |
| Non-interactive | `--key-file`, `EVNX_PASSWORD` | `--password-stdin` |
| Replacing a file | `--yes` (alias `--force`), keeps `.bak` | `--force` |
| Works with no account or network | yes | no |
| Everything back at once | the file is the backup | `cloud export` |

For a vault, use `evnx cloud pull`; `restore cloud://…` is not supported.

---

### `evnx commands` _(alias `surface`)_

Which build you have: the version, the features it was compiled with, and every
command, grouped. The answer to "why is `evnx vault` not found?". `--json` emits
the whole command tree (schema 1) for tooling.

```bash
evnx commands
evnx commands --json | jq -e '.features | index("cloud")'   # exit 1 without cloud
```

---

### `evnx completions`

Prints a completion script for `bash`, `zsh`, `fish`, `powershell` or `elvish`.
`evnx completions --help` shows where each shell reads it.

```bash
evnx completions bash > ~/.local/share/bash-completion/completions/evnx
evnx completions fish > ~/.config/fish/completions/evnx.fish
```

---

### `evnx update`

How to upgrade, for the channel that installed this binary (cargo, Homebrew,
Scoop, winget, npm, pip or the install script). It contacts nothing unless you
pass `--check`, and never replaces its own binary.

```bash
evnx update                    # the upgrade command for this install
evnx update --check            # exit 1 when a newer release exists
evnx update --check --json     # the same answer for CI
```

---

## Working with multiple environments

`--env-name` resolves `.env.<name>` across `validate`, `diff`, `sync`, `convert`,
`migrate`, `backup` and `template`. `scan` has no `--env-name`: it reads every `.env`
variant a folder walk finds, or the file you name.

```bash
evnx validate --env-name production            # operates on .env.production
evnx diff --env-name production --against staging
```

A name whose file does not exist is an error listing the ones that do — it never quietly
falls back to `.env`. Set a project default with `[defaults] env_name` in `.evnx.toml`.

Between two real environments `diff` compares values too; against the template it compares
key names only, because every filled-in value differs from its placeholder.

---

## Cloud sync _(requires `--features cloud`)_

Push your `.env` to the cloud and pull it on any machine or in any pipeline.
Encryption and decryption happen on your machine; the server stores ciphertext and
holds no key that can open it.

**What that guarantees:** what is already stored stays closed. A database dump, a
court order against stored data, a passive observer — none of them yield
plaintext, because the key never leaves your machine.

⛔ **What it does not guarantee: safety from a server that has been taken over.**
The client decides how to unwrap a vault key from fields the *server* sends, so an
attacker with database write access can present a key they chose and read what you
push **next** — including for a vault you never shared. Fixing it needs signed
wraps and locally pinned identities, which are not built.

⚠️ **Cloud sync is therefore beta.** Do not put secrets there that you could not
rotate. The rest of evnx — validate, scan, diff, sync, convert, and the commands
`migrate` prints — runs entirely offline and is not affected. (`migrate --to
github-actions` is the exception: it calls the GitHub API, and talks to nothing else.)

ⓘ This README previously said "not after a breach". That was wrong, and the
correction is tracked in [`SECURITY.md`](./SECURITY.md#what-the-threat-model-actually-claims).

```bash
evnx auth register                       # create an account
evnx auth login                          # sign in on this machine
evnx vault create app --env production   # a vault holds one .env, versioned
evnx cloud link app/production           # bind this directory to it
evnx cloud push                          # encrypt locally, upload ciphertext
evnx cloud pull                          # download, decrypt, write .env
evnx cloud status                        # server, sign-in, this directory's binding
evnx cloud unlink                        # forget the binding
```

A vault name is lower-case letters, digits and hyphens. `-q` silences what a
cloud command says it did; errors, warnings and anything you asked for — a
table, a new token, `pull --file -` — still print.

### Installing with cloud support

**Most prebuilt binaries already include the cloud commands.** npm, Homebrew,
Scoop, winget and the GitHub Release are all built with `--all-features`, so if you
installed evnx any of those ways, `evnx cloud --help` already works.

> ⓘ **PyPI carries the cloud commands from 0.10.0 onward.** Every wheel from
> 0.4.0 **up to and including 0.9.0** was built with `--features full`, and
> `cloud` is deliberately not in `full` — so `pip install evnx` gave a binary
> with no `auth`, `vault`, `cloud` or `org` at all, for five releases.
>
> Fixed in 0.10.0 and verified by installing the published wheel and running it
> rather than by reading the workflow: `evnx commands` reports
> `built with: net, migrate, backup, cloud`.
>
> ⚠️ **If you are on 0.9.0 or earlier, `pip install --upgrade evnx`.** The
> version string was identical whether or not the feature was compiled in, which
> is exactly why this went unnoticed for five releases.

`cargo install` is the other exception, by design, because it compiles from source
with `default = []`. Build every feature, as the prebuilt binaries are (this is
also the command `evnx update` prints for a cargo install):

```bash
cargo install evnx --all-features
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
evnx cloud history                                  # what changed, when, and who pushed it
evnx cloud pull --version 3                         # restore an earlier version
evnx cloud diff                                     # what differs between two versions
evnx cloud export --output ../backup --all-versions # every version to disk, decrypted
evnx cloud delete-version 2 --vault app/production  # remove one version for good
```

`cloud export` writes the latest version unless `--all-versions` is given, into
`./evnx-export/` unless `--output` (`-o`) names a directory. Inside a git work
tree it refuses the default directory unless `--force`, and writes a
`.gitignore` there either way: these are plaintext secrets. `cloud
delete-version` never removes the latest version, and needs a sign-in rather than
an API token.

### Which file push and pull use

`push` and `pull` use `.env.<environment>` when that file exists —
`.env.production` for an `app/production` vault — and `.env` otherwise;
`--file` or `--env-name` names one outright. Each says which file it used.

Pushing the **default** `.env` into a **production** vault asks first; with no
terminal to ask on (CI), it is refused unless `--yes`. A file named with `--file`
or `--env-name` is not asked about. Addressed by id, a vault's environment is
not known for choosing the file (the file is `.env`); a signed-in session still
looks it up to ask, a vault-scoped token cannot and is not asked.

`pull` writing a file inside a git repository checks that git ignores it: it asks
on a terminal, warns without one, adds it with `--gitignore`, and skips the check
with `--no-gitignore`. Over a file that differs it asks first, or `--force`.

⚠️ `cloud diff` compares **key names**, not values. The server holds ciphertext
and a list of names, so it can tell you a variable appeared, disappeared or
changed — never what it changed to. It says "same keys, different contents"
rather than claiming nothing changed.

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

### Who changed what, and being told when

```bash
evnx cloud audit --vault app/production   # who pushed, pulled, shared, revoked
evnx cloud watch --vault app/production   # notify when someone else pushes
```

`cloud watch` skips your own pushes; `--include-mine` shows them too. A failed
poll is a blip; from the third in a row it says so on stderr, and after ten in a
row it stops with exit 2 and what to check.

⛔ `cloud watch` **notifies and never pulls.** Pulling automatically would
overwrite local edits without being asked, and it would mean holding the vault
key in memory for as long as the watch runs. It tells you; you decide.

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
evnx vault rekey app/production      # rotate the key, nobody removed
evnx vault leave app/production      # leave a vault shared with you
evnx vault list --verbose            # every vault you can reach, with ids
evnx vault delete app/production     # the owner only; no undelete
```

| | viewer | developer | admin | owner |
|---|---|---|---|---|
| pull, run, export, history, diff, watch, audit, members | ✓ | ✓ | ✓ | ✓ |
| push | — | ✓ | ✓ | ✓ |
| share | — | — | as viewer or developer | ✓ |
| change a role | — | — | below their own, for someone they outrank | ✓ (not their own) |
| revoke | — | — | someone they outrank | ✓ (not themselves) |
| rekey, delete a version | — | — | ✓ | ✓ |
| delete the vault | — | — | — | ✓ |
| leave | ✓ | ✓ | ✓ | — (delete it instead) |

Sharing with someone who is already a member is refused before the password:
another role is a role change (`vault role`), the same role changes nothing.
Sharing with yourself, or with the owner, is refused.

`revoke` re-keys by default: every version is re-encrypted and re-wrapped for
whoever stays. `--no-rekey` only removes them, and they keep the key they had —
use it only when you re-key separately. Leaving does not re-key either; someone
who stays can run `vault rekey`.

The vault key is wrapped for the recipient with a **hybrid X25519 + ML-KEM-768** scheme,
so a share is not opened by a future quantum computer harvesting today's traffic. There is
no X25519-only path; sharing with an account that has no ML-KEM key on file is refused
rather than downgraded.

Revocation re-keys the vault, so a removed member cannot open versions pushed after they
left.

⚠️ The recipient's public keys come from the server, and out-of-band fingerprint
verification is not built — so a hostile server can substitute its own and read
what you share.

⛔ This is **not** limited to sharing. Nothing local records which vaults you own,
so the client cannot tell a genuine share from a fabricated one; the same
substitution works on a vault you created and never shared. See
[`SECURITY.md`](./SECURITY.md#what-the-threat-model-actually-claims).

### Organisations and plans

An organisation owns a **plan** and a set of **seats**. Whoever holds a seat gets
that plan's limits — more vaults, more versions, more API tokens.

```bash
evnx org create "Acme Corp" --slug acme
evnx org invite colleague@example.com --role member
evnx org assign colleague@example.com      # their limits become the org's
evnx org billing                           # what the plan is, and until when
```

The rest of `evnx org`:

```bash
evnx org list                               # the organisations you are in
evnx org members                            # who is in it, and who holds a seat
evnx org invites                            # open invitations
evnx org uninvite <invite-id>               # withdraw one
evnx org accept <token>                     # join, from an invitation
evnx org seats                              # seat count (--set N / --unlimited before billing)
evnx org release colleague@example.com      # give a seat back
evnx org role colleague@example.com --set admin
evnx org remove colleague@example.com
evnx org delete                             # the owner only
evnx org leave                              # anyone but the owner
```

`--org <slug>` picks the organisation when you belong to more than one.

> ⛔ **An organisation does not give anyone access to a vault.** It is a plan and a
> directory, and that is the whole feature. The server cannot wrap a vault key —
> that is the guarantee, not a gap in it — so no membership, role or seat can
> produce access. Sharing is `evnx vault share`, by someone who holds the key.

A seat can be held in exactly one organisation at a time; directory membership is
unconstrained, so a contractor at two companies is two entries and one seat.

Releasing a seat drops that person back to their own plan, which can leave them
**over** the new limit. Nothing is deleted and nothing becomes unreadable — a
vault over the limit stays readable and exportable, and the next *push* to it is
what gets refused.

`evnx org billing` is read-only. Plans, seat counts and payment methods are
changed at [app.evnx.dev/billing](https://app.evnx.dev/billing/), because a card
form does not belong in a terminal.

⚠️ Organisation commands need a real sign-in — an `evnx_tok_` API token is
refused, because inviting people and assigning seats changes what the account is
billed.

### Your account (`evnx auth`)

Nineteen commands in five groups. Each one's `--help` says more, and the full
guide is at [docs.evnx.dev/cli/commands/auth](https://docs.evnx.dev/cli/commands/auth).

| Command | What it does |
|---|---|
| **Account** | |
| `evnx auth register` | Create an evnx cloud account |
| `evnx auth login` | Sign in on this machine. CI uses an API token instead ([below](#cicd)) |
| `evnx auth logout` | Sign out. The stored login is removed even if the server cannot be reached |
| `evnx auth status` | Your account as the server sees it: verified, 2FA, plan and usage |
| **API tokens**, for CI/CD | |
| `evnx auth token create <name>` | Mint a token. Shown once, then only revocable |
| `evnx auth token list` | List the account's live tokens |
| `evnx auth token revoke <name-or-id>` | Revoke a token |
| **Two-factor** | |
| `evnx auth totp enable` | Turn on two-factor authentication, and get ten recovery codes |
| `evnx auth totp disable` | Turn it off. Needs a current code, not just a session |
| `evnx auth totp recovery-codes` | Issue fresh recovery codes, invalidating the old ones |
| **Sessions and devices** | |
| `evnx auth sessions list` | List active sessions |
| `evnx auth sessions revoke <id>` | Revoke one session |
| `evnx auth sessions revoke-others` | Revoke every session except this one |
| `evnx auth devices list` | Networks and browsers this account has signed in from |
| `evnx auth devices disavow <device-id>` | Report that a device was not you |
| **Master password and data** | |
| `evnx auth rotate-master-password` | Change your master password |
| `evnx auth undo-password-change` | Undo a change, by proving the previous password |
| `evnx auth download-data` | Everything the server holds about your account, as JSON |
| `evnx auth delete-account` | Delete your account and everything only you can reach |

#### Where the CLI keeps your login

One folder, which depends on the platform:

| Platform | Folder |
|---|---|
| Linux | `~/.config/evnx` (or `$XDG_CONFIG_HOME/evnx`) |
| macOS | `~/Library/Application Support/evnx` |
| Windows | `%APPDATA%\evnx` |

`EVNX_CONFIG_DIR` replaces the folder entirely, which is also how to keep a work
and a personal account apart on one machine.

| File | Holds | Written by |
|---|---|---|
| `config.toml` | `server = "..."` and other preferences | you, by hand |
| `credentials.json` | your email and the session's access and refresh tokens | only the CLI, at mode `0600` in a folder kept at `0700` (Linux and macOS) |

Neither file holds your master password or any key that opens a vault: those
are never written to disk, so every command that decrypts asks for the password.

| Variable | Effect |
|---|---|
| `EVNX_SERVER` | The server for this run. `--server` beats it; it beats `config.toml`; the default is `https://api.evnx.dev` |
| `EVNX_TOKEN` | An API token, used instead of the stored login ([CI/CD](#cicd)) |
| `EVNX_CONFIG_DIR` | The folder above |

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
  from `evnx vault list --verbose`. By id, a vault-scoped token works with
  `cloud pull`, `run`, `export`, `history`, `diff`, `watch`, `audit` and `vault
  members`, and with `cloud push` when it is `read_write`. Sharing, roles,
  revoking, re-keying, leaving, deleting a vault or a version need a signed-in
  person: a token is refused before anything is sent.

Pipe the password rather than exporting it where you can: an environment variable
is visible in process listings and tends to end up in logs.

### Self-hosting

`evnx cloud` talks to `https://api.evnx.dev` by default. Point it anywhere:

```bash
evnx cloud status --server https://evnx.internal.example
```

or set `EVNX_SERVER`, or `server = "..."` in `config.toml` in the CLI's folder
([where that is](#where-the-cli-keeps-your-login)). The
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
    # ⚠️ Pin 0.10.0 or newer. Before it, these hooks ran `evnx scan` with no
    # arguments and `always_run: true`, so every commit scanned the whole working
    # tree — including the real, gitignored `.env` a developer necessarily has.
    # That found secrets, exited 1, and blocked the commit. Every commit.
    rev: v0.10.0
    hooks:
      - id: evnx-scan        # blocks commit if secrets found
      - id: evnx-validate    # blocks commit if .env misconfigured
      - id: evnx-diff        # blocks commit if .env lacks a key .env.example declares (skips with no .env)
      - id: evnx-sync        # blocks commit if .env.example lacks a variable from .env (skips with no .env)
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
fail_on          = "error"           # "error" | "warning": the lowest severity that exits 1

[sync]
naming_policy = "warn"         # "warn" | "error" | "ignore"; any other value is an error
                               # placeholder rules are a JSON file: --template-config

[diff]
ignore_keys = ["BUILD_ID"]

[backup]
keep = 3                       # backups kept by `evnx backup` rotation; --keep wins

[cloud]
vault = "my-app/production"    # written by `evnx cloud link`

# What each variable *is* — read by validate, scan and sync. `evnx spec init` writes it.
[vars.PORT]
format      = "port"
description = "HTTP listen port"

[vars.STRIPE_KEY]
secret       = true
environments = ["production"]
```

A setting belongs here if it is true of the **project** and would otherwise be repeated
on every invocation. It does not if it expresses what *this* invocation should do — which
is why `[scan] severity` is configurable and `--exit-zero` is not.

Precedence is **flag > config > default**. Lists combine rather than replace, so
`--exclude dist` scans with `dist` *and* the project's excludes. Settings that can weaken
scanning are announced on every run. Unknown keys warn and never fail, so a file written
for a later evnx keeps working on an earlier one — except under `[vars.NAME]`, where
`validate` counts one as an error: a typo there changes the contract.

Only `.evnx.toml`, with the leading dot, is read. A bare `evnx.toml` stops every command
with the rename to make; beside a `.evnx.toml` it is a warning.

Full reference: <https://docs.evnx.dev/cli/reference/configuration-file>

⚠️ **On v0.4.x and earlier this file did nothing.** The loader existed and nothing called
it, so a project with `[validate] strict = true` validated non-strictly and said nothing.
Keys named in older copies of this README — `env_file`, `auto_fix`, `exclude_patterns`,
`[convert]`, `[aliases]` — were never read by anything and do not exist.

---

## Known Limitations

⚠️ **Corrected 2026-09-27.** This section said multiline and array values "will not
parse correctly" and gave three examples. Two of the three parse fine, and
multiline values have been supported all along — the parser accepts a quoted value
that spans lines and `convert` round-trips the newlines intact. The wrong claim
was repeated into several planning documents and nearly produced work to build a
feature that already existed.

**Multiline values work.** A value whose opening quote is not closed on the same
line continues onto the next, with either quote style:

```bash
PRIVATE_KEY="-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDGxYz1
-----END PRIVATE KEY-----"
NEXT_VAR=parsing continues normally after it
```

`evnx convert --to json` gives `"-----BEGIN PRIVATE KEY-----\nMIIEvQ…\n-----END PRIVATE KEY-----"`,
and `evnx scan` reports it as a private key. Single quotes span lines too, without
unescaping.

**Values stay strings.** evnx follows the strict `.env` spec, so JSON and arrays are
preserved exactly but never interpreted:

```bash
CORS_ALLOWED=["https://example.com", "https://admin.example.com"]   # parses — as a string
CONFIG={"key": "value"}                                             # parses — as a string
```

Both are preserved character-for-character. Parse them in your application code,
as you would with any other `.env` loader.

**Escape sequences work the same whether or not the value spans lines.** In a
double-quoted value `\n`, `\t`, `\\`, `\"` and `\'` are interpreted; single-quoted
and backtick values are literal. So an escaped quote is fine in either shape:

```bash
ONE_LINE="he said \"hello\""
SPANNING="he said \"hello\"
and left"
```

⚠️ **This changed — see the CHANGELOG for the version.** The second form used to be
a parse error, and a double-quoted value that spanned lines kept its backslashes
instead of resolving them, so `"x\ty"` gave a tab on one line but a literal
backslash and `t` across two. If you relied on backslashes surviving verbatim
inside a multiline double-quoted value, switch it to single quotes, which are
literal by definition.

**One thing genuinely does not work** — triple-quoted / heredoc style, which is
not `.env` syntax in any dialect evnx targets:

```bash
# not supported
DATABASE_HOSTS="""
host1.example.com
"""
```

A `--lenient` flag for extended syntax is under consideration — see [open issues](https://github.com/urwithajit9/evnx/issues).

**Multiple environments are supported as of v0.5.0.** `evnx scan` reads every `.env`
variant it walks past, and `--env-name production` resolves to `.env.production` across
`validate`, `diff`, `sync`, `convert`, `backup` and `template`.

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
net     = ["dep:reqwest"]                    # update --check
migrate = ["net", "dep:crypto_box"]          # migrate (GitHub upload uses both)
backup  = ["dep:aes-gcm", "dep:argon2"]
cloud   = ["net", "dep:evnx-crypto", …]      # auth, vault, cloud, org
full    = ["migrate", "backup"]
```

The prebuilt binaries are built with `--all-features`.

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

[Website](https://www.evnx.dev) | [Issues](https://github.com/urwithajit9/evnx/issues) | [Discussions](https://github.com/urwithajit9/evnx/discussions) | [Support](mailto:support@evnx.dev) | [Security](SECURITY.md)