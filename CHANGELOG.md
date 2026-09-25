# Changelog

All notable changes to evnx are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.5.0] - 2026-09-25

The command-review release. Every command was run against a real build and
compared with its published guide; where the two disagreed, one of them was
wrong and got fixed. 31 pull requests (#29–#59).

**Full write-up: [docs/releases/v0.5.0.md](docs/releases/v0.5.0.md).** Entries
below are one line each; the detail, including how each was found, is there.

### Security

⚠️ **`evnx validate --fix` generated predictable secrets and called them
secure.** `generate_secure_secret` returned `SystemTime::now().as_nanos()`
XORed with a constant, under a comment reading *"replace with crypto RNG in
production"*. 48 of the 64 hex characters were always `0`, and three runs
seconds apart differed only in their last few — roughly 25 bits, of *time*
rather than randomness, searchable by anyone who knew the day. Now 32 bytes
from the OS CSPRNG via `getrandom`.

**If `evnx validate --fix` ever generated a `SECRET_KEY` for you, rotate it.**

⚠️ **Five commands reported success while doing nothing, or the wrong thing.**
These are the reason to upgrade rather than wait.

- **`evnx migrate --to github` never encrypted anything.** It base64-encoded the
  plaintext and posted it as `encrypted_value`. Now a real libsodium sealed box.
  **Rotate any secret pushed to GitHub by an earlier version.**
- **`evnx sync` wrote real values into `.env.example`** when creating one, then
  advised committing it. Placeholders are now the default.
- **`evnx scan` never read `.env.production`** — or any dotted variant. A
  directory with `.env` and `.env.production` scanned only the first and reported
  a completed scan.
- **`evnx scan <unwalkable path>` printed "✓ No secrets detected"** and exited 0.
- **`evnx migrate` with no `--to` guessed.** With no terminal it silently picked
  the **first** destination in the list and exited 0, printing a plan for
  somewhere the user had never chosen.

### Fixed — from the DevRel review of 2026-09-24

Every command re-checked against every guide, each claim verified independently
before acting. Write-up and verification in `evnx-devrel-review/`.

- **`evnx init` wrote a `.env.example` that could not be appended to.** No
  trailing newline, so the next line fused onto the final comment and the
  variable silently disappeared. `evnx add` was immune, which is why it
  survived — evnx's own documented next step hid it.
- **`evnx validate --fix` exited 1 after fixing everything.** It counted the
  issues found on entry and never recounted, so `validate --fix && deploy`
  never reached `deploy`, while the next plain `validate` exited 0.
- **`--validate-formats` rejected every non-HTTP URL.** `postgresql://`,
  `redis://` and `amqp://` were all "not a valid URL" — including the
  `DATABASE_URL` that `evnx init --with postgresql` had just written. There
  were two URL definitions in one binary; there is now one.
- **`doctor` called `export FOO=bar` invalid** while the parser accepted it,
  and reported only the first bad line. Both fixed — plus a third disagreement
  the new cross-check found on its first run: the parser refused a leading
  underscore, which POSIX allows.
- **Findings came out in a different order every run** — `HashSet::difference`
  with Rust's randomised hasher. `sync` had the same bug, and there it *writes*
  that order into `.env.example`, so two developers syncing one file produced
  two different files.
- **A key written twice vanished in silence.** Last-wins is kept; the silence
  is not. `validate` now names the lines and says which one takes effect.
- **SARIF put the variable name inside the `ruleId`**, so every new variable was
  a new rule to GitHub and no dismissal ever stuck. Adds `rules[]` with help
  links and `partialFingerprints`, so an alert survives a line moving above it.
- **`completions` answered an unusable argument with 1**, bypassing the
  centralised exit-code mapping. Now 2, like everything else.
- **`convert --help` was backwards about kubernetes** — it respects `--base64`,
  emitting `stringData:` without it.
- **`cloud status` promised a token renewal it had no way to verify**,
  contradicting `evnx auth status` on the same machine.
- **`init --detect` conflicted with `--yes`**, which it already implies.
- **Fourteen flags had no help text at all**, including `--format` on `validate`
  and `scan` — the two flags that make evnx usable in CI, neither naming `json`,
  `sarif` or `github`. The prose already existed, accurately, in the guides.
- **`backup` and `restore` disagreed about how a passphrase arrives.** `restore`
  honoured `EVNX_PASSWORD`; `backup` ignored it, so a scheduled backup had no way
  to supply one except writing it to disk — while the restore it fed needed no
  file at all. `backup` now accepts it, and each command accepts the other's flag
  spelling (`--key-file` / `--password-file`).
- **One key file could produce two different passwords.** `backup --key-file`
  Base64-encodes a binary key file before Argon2id; `restore --password-file`
  read the same file as UTF-8 and stripped a trailing newline. A binary key file
  therefore wrote a backup that could not be opened with the file that wrote it,
  and the only symptom was a failed decryption. Both sides now use one reader.
- **`evnx scan` claimed a value matched a live key format when nothing had read
  the value.** Every high-confidence finding carried "matches a live key format,
  not a placeholder", including ones reached purely by the variable's *name* —
  so `NEXTAUTH_SECRET=dev-not-a-real-secret` was reported as a live key. That
  line is what tells someone to drop everything and rotate; attaching it to a
  name match teaches people to ignore it. Name-based findings now say so.
- **`migrate --dry-run` reported a migration that had not happened.** The summary
  said `✓ 0 command(s) generated` while printing the commands, then followed it
  with "verify secrets were set" and "redeploy your project". It now reports what
  it previewed and points at the run that would apply it.
- **`convert --help` printed its examples as unusable Markdown** — a fenced block
  in a doc comment, which clap renders verbatim, so the fence markers and the
  shell line-continuations landed inline on one collapsed line. The one place in
  the CLI where copying an example could not work.

### Fixed — validate's blind spots

⚠️ **`evnx validate` said nothing about the credentials `evnx init` had just
generated.** Two independent gaps, and together they meant a fresh
`evnx init --with nextjs,postgresql` followed by the documented
`cp .env.example .env` produced a `DB_PASSWORD` and a `NEXTAUTH_SECRET` still
holding placeholders — and `validate` passed it without a word. A forgotten
password reaching production is the single thing this command exists to prevent.

- **The placeholder list did not know evnx's own placeholders.** `init` writes
  `your_<name>_value`; the list held `your_key_here`, `your_secret_here` and
  `your_token_here` — the suffix is `_here`, not `_value`. Now any `your_*`.
- **The weak-secret check only ever looked at one variable.** It was
  `env_vars.get("SECRET_KEY")` — a single hardcoded lookup — so `JWT_SECRET=123`,
  `SESSION_SECRET=weak` and `DB_PASSWORD=dev` all validated clean. Now every
  credential-shaped name. The length floor is split by role: 32 characters for a
  framework signing key, 16 for everything else, because an AWS access key **ID**
  is 20 characters by specification and is not weak.
- **A generated secret could be called weak by the command that generated it.**
  `1234` and `abcd` are ordinary hex, so roughly one run in 500 produced a
  64-character secret containing one. A uniformly hex or base64 value is now
  exempt from the word list.
- **`validate --fix` reported a placeholder as a completed repair.** Adding a
  missing key can only insert `your_value_here`; the key exists, the value does
  not. It now says so rather than printing the fix directly above the finding
  that contradicts it.
- **`--fix` swapped one placeholder for a vaguer one and called it a fix.**
  Recognising `your_*` meant `--fix` began "repairing"
  `DB_NAME=your_db_name_value` into `DB_NAME=your_value_here` — discarding the
  only hint the line carried, and reporting it directly above the finding it had
  not resolved. evnx can invent a secret, a URL, an email and a port; it cannot
  invent a database name, and no longer offers to.
- **`--fix` repaired at most one weak secret per run, and only if it was called
  `SECRET_KEY`.** The fixer carried the same hardcoded lookup as the check.
  `DB_PASSWORD` and `API_TOKEN` are now treated as credentials too, so `--fix`
  generates real values for them instead of declining.

### Fixed — the generated `.env` is now readable by evnx

⚠️ **`evnx init` told you to run `evnx validate`, and doing so reported every
variable missing.** `init` wrote `.env` with every assignment commented out as
`# TODO: DB_HOST=localhost`, while `.env.example` held the same variables
active — so the parser saw nothing in the file `init` had just created:

```
3. Run 'evnx validate' to check configuration     <-- init's own next step

✗  Missing required variable: DATABASE_URL
✗  Missing required variable: DB_HOST
... all six                                        exit 1
```

It also made `init`'s first instruction — *"Edit .env and replace placeholder
values"* — untrue, since there were no values in `.env` to replace. `.env` and
`.env.example` now carry the same assignments, and `validate` reports what
actually needs attention: `DB_NAME looks like a placeholder`.

- **`evnx add` reported variables the parser could not see.** Same cause:
  `add service postgresql` printed "Added 6 variables", wrote six `# TODO:`
  comments, and `validate` — run immediately after — called all six missing. Two
  commands contradicting each other in consecutive steps.
- **`evnx sync` no longer just says "up to date" when the source is empty.** A
  blank `.env` beside a populated `.env.example` is more likely a truncated file
  or the wrong directory than a steady state, so it now says so and points at
  `--direction reverse`. Still exit `0` — a notice, not a failure.
- `evnx add` writing into an empty `.env` no longer opens the file with three
  blank lines.
- **A real provider key is no longer called weak.** The weak-word list was a
  plain substring match, so `abcd` occurring inside a 62-character SendGrid key
  reported *"too weak or predictable"* — with the advice *"Run: openssl rand
  -hex 32"*, which would replace a working credential with a random string. A
  weak word now has to make up at least a fifth of the value to count. This
  replaced a charset heuristic added in the same release, which was fragile in
  two ways found within a day of each other: it knew the separators `+/=-_` but
  not `.`, and it required a digit, so dotted tokens and digit-free keys both
  fell through.

### Fixed — reported issues

- **A file that is absent is no longer reported as a file that failed to parse**
  ([#12](https://github.com/urwithajit9/evnx/issues/12)). `evnx diff` with no
  `.env.example` said *"Failed to parse .env.example"*, sending you after a
  syntax error in a file that does not exist; `validate` printed the OS error
  twice on top of that. Both now name the file and say how to create it. A file
  that really is malformed still reports the line — which `diff` had been
  dropping, because it printed its error without the cause chain.
- **`evnx migrate` no longer reports a migration that did not happen**
  ([#13](https://github.com/urwithajit9/evnx/issues/13)). An empty source exited
  `0` with no docs link, and named the source *kind* (`env-file`) rather than the
  file — so it could not distinguish an empty file from the wrong directory.
  Both it and a filter that matches nothing are now errors, as they already were
  in `evnx cloud run`.
- **`--skip-existing` and `--overwrite` are refused where they cannot work.**
  Eight of nine destinations only print commands, so they cannot know what
  already exists at the destination; they accepted both flags in silence.
  `--to github-actions` is the one that uploads, and honours them — its dry run
  now also counts what it previewed instead of reporting zero.
- **`evnx add custom` can no longer write a file evnx cannot read**
  ([#10](https://github.com/urwithajit9/evnx/issues/10), item 4). The variable
  name came straight from the prompt unvalidated, so entering `NODE_VERSION=22`
  wrote `# TODO: NODE_VERSION=22=22`. The parser had always refused such a key;
  it was never asked. Entering a name with `=` now suggests the name you meant.
- **`scan`'s machine formats made the claim its terminal output had stopped
  making.** `--format json` returned identical fields for a name match and a
  value match, and `--format github` still said *"matches"* and *"Rotate this
  credential"* for a finding reached purely by the variable's name. JSON gains
  `matched_by: "value" | "name"` (additive — no existing consumer breaks) and the
  annotation now says the value went unchecked.
- Generated `.env` files no longer carry two leading spaces on their
  `# (required)` comments — the one ragged edge in otherwise column-aligned
  output.

⚠️ **Behaviour changes worth knowing before you upgrade CI:**

- `evnx validate` now flags unfilled `your_*` placeholders and weak secrets in
  any credential-shaped variable. A `.env` that passed before may now fail —
  which is the point, but it is a change.
- `evnx migrate` exits `2` instead of `0` when there is nothing to migrate.

### Added

- **`evnx spec`** — a `[vars]` contract saying what each variable *is*: required,
  secret, its format, which environments need it. Read by `validate`, `scan` and
  `sync`. `evnx spec init` infers it from the files you already have.
- **`evnx cloud run -- <cmd>`** — decrypt a vault into a subprocess. Nothing on
  disk, nothing in shell history, nothing in `ps`. `--include`/`--exclude` narrow
  what the child sees; the child's exit code comes back unchanged.
- **`evnx vault share`, `members`, `role`, `revoke`** — team access, wrapped with
  hybrid X25519 + ML-KEM-768. Revocation re-keys the vault.
- **`evnx scan --pattern` and `[[scan.patterns]]`** — your own secret formats,
  matched in one `RegexSet` pass so declaring rules stays close to free.
- **`.evnx.toml` is now read.** The loader existed and nothing called it.
  Precedence is flag > config > default; lists combine; settings that weaken
  scanning are announced on every run.
- **`--env-name production`** across `validate`, `scan`, `diff`, `sync`,
  `convert`, `backup` and `template`. `diff --env-name production --against
  staging` compares two real environments.
- **`evnx sync --check`** — a CI gate with 0/1/2 exit codes, and `--format json`.
- **`evnx doctor --fix`, `--path`, `--strict`** — the auto-fix had worked since
  doctor shipped, reachable only via `EVNX_AUTO_FIX=1`.
- **`evnx init --detect`, `--from-source`, `--with`, `--list-components`** —
  `--from-source` builds the template from what the code actually reads.
- `evnx scan --severity`, a `summary` object in `scan --format json`, and golden
  files pinning every machine-readable output.

### Fixed

- `evnx migrate` no longer claims `✓ N uploaded` for the eight destinations that
  only print commands. `--dry-run` runs headless.
- `evnx add` warns when the `.env` it wrote to is not covered by `.gitignore`.
- `evnx template` refuses unresolved placeholders under `--strict`; `{{ VAR }}`
  with spaces and `|default:` both work.
- `evnx init` and `doctor` cover every env file, not just `.env`.
- `evnx add blueprint <unknown>` lists the blueprints instead of saying "run
  `evnx init`" — an interactive prompt, and so unusable in the scripts where
  `--yes` is passed. `evnx init --blueprint <unknown>` already listed them.
- `evnx add` no longer writes an orphaned `  # (required)` line detached from
  the variable it describes, and the files it writes end with a newline.
- Benchmarks compile again; `install.sh` no longer depends on JSON formatting.
- CI lints `--all-targets`, so test code is checked too. It was not before, and
  an `Assert` whose result was dropped — asserting nothing — sat unnoticed.

### Changed

- **`evnx validate --pattern` was removed** — its implementation was a discarded
  argument. Use `--env-name`.
- **`evnx scan` exit codes are now 0 / 1 / 2.** `2` means the scan could not be
  completed, and is not a louder `1`. `--exit-zero` suppresses `1` only.
- ⚠️ **An error is now exit `2` in every command, not `1`.** `scan`, `diff`,
  `doctor` and `sync --check` already used `2` for "could not run, so there is
  no verdict". Everything else inherited Rust's default, which numbers any error
  `1` — the code `validate` uses for *invalid*, `diff` for *differences found*
  and `sync --check` for *drifted*. So `evnx validate --env ./missing.env`
  reported **validation failed** for a file it never opened, and a typo'd
  `--env-name` did the same in `validate`, `diff`, `sync`, `convert`, `template`
  and `backup`. A malformed `.evnx.toml` did it in every command at once.
  **Findings are unchanged:** a real validation error is still `1`, differences
  still `1`, a secret still `1`, success still `0`.
- `evnx doctor` gained `--format`; `EVNX_OUTPUT_JSON=1` still works and the flag
  wins when both are given.

---

## [0.4.0] - 2026-09-15

Zero-knowledge encrypted cloud sync. Push a `.env` to a server that is
**mathematically unable** to read it, and pull it on any machine or in any
pipeline.

Everything here is behind `--features cloud`, which is **off by default** and not
enabled in the prebuilt npm / PyPI / Homebrew / Scoop binaries. `cargo install evnx`
produces a byte-identical binary to 0.3.8 in behaviour — no existing command
changed.

### Added

- **`evnx auth`** — `register`, `login`, `logout`, `status`. Authentication is
  SRP-6a: the password never leaves the machine, and the server must prove it
  holds your verifier before any session is stored.
- **`evnx auth totp`** — `enable` (with a scannable QR), `disable`,
  `recovery-codes`. `disable` and `recovery-codes` require a current second
  factor rather than a live session, so a stolen session cannot strip 2FA from an
  account.
- **`evnx auth sessions`** — `list`, `revoke`, `revoke-others`. Revocation kills
  the refresh tokens and blocklists the session, so an outstanding access token
  stops working immediately rather than lingering for its remaining lifetime.
- **`evnx auth token`** — `create`, `list`, `revoke`. API tokens for CI, scopable
  to one vault and to read-only. Supplied to the CLI via `EVNX_TOKEN`.
- **`evnx vault`** — `create`, `list`, `delete`. A vault holds one `.env`,
  versioned. Its key is generated locally and wrapped under your master key
  before it leaves the machine.
- **`evnx cloud`** — `push`, `pull`, `history`, `link`, `unlink`, `status`.
  `push` encrypts the file's **raw bytes**, so comments, ordering, quoting and
  whitespace survive a round trip unchanged.
- **`evnx cloud link`** binds a directory to a vault via `[cloud] vault` in
  `.evnx.toml`, so `push` and `pull` need no `--vault`. Written with a
  format-preserving TOML editor: your comments and unrelated settings survive.
  Safe to commit — a vault name is not a secret, and sharing it means a
  teammate's pull lands in the same place.
- **`evnx cloud status --ping`** exits non-zero when the server is unreachable,
  so it works as a health check. Without `--ping` it reads local files only and
  works offline.
- `rust-version = "1.85"` is now declared. Verified by building on it.

### Security

- **Credentials are stored at mode 0600** in `~/.config/evnx/credentials.json`,
  and the permissions are re-checked on every read — a file found readable by
  anyone else is tightened, with a warning that the tokens should be treated as
  exposed.
- **Plain `http://` is refused** for anything but a loopback address. Access and
  refresh tokens travel in request headers, and a refresh token lives 30 days.
- **Nothing that can decrypt a vault is ever written to disk.** The master key,
  vault keys and the private key are derived in memory, used, and dropped. A
  stolen `credentials.json` buys an authenticated session, and the API answers it
  with ciphertext.
- **No `--password` flag exists**, so a master password cannot reach shell
  history. Commands prompt, or read `--password-stdin`.
- Clears five advisories that were live in the dependency tree, four of them in
  the TLS stack the cloud feature uses: `RUSTSEC-2026-0104`, `-0099`, `-0098`,
  `-0049` (rustls-webpki) and `RUSTSEC-2026-0285` (rustls). Removes `atty`, which
  is unmaintained and carried `RUSTSEC-2021-0145`, in favour of
  `std::io::IsTerminal`.
- A `cargo audit` job was added to CI. Its absence is why those advisories went
  unnoticed.

### Changed

- `evnx --features cloud` roughly doubles the binary, 3.6 MiB → 7.2 MiB, and adds
  ~118 crates. This is why `cloud` is not in `full` and not in the prebuilt
  binaries.

### Known limitations

- **Vault sharing is not included.** Shared vault keys are wrapped with X25519,
  which is not post-quantum safe; a hybrid ML-KEM wrap lands before sharing
  ships. Solo vaults are unaffected — they are wrapped with Argon2id and
  XChaCha20, which are symmetric throughout.
- **A previously installed evnx can shadow the cargo one.** A package-manager
  install often sits earlier in `PATH` than `~/.cargo/bin`, so
  `cargo install evnx --features cloud` appears to succeed while the old binary
  still answers. Diagnose with `which -a evnx`.
- **One `.env` file at a time**, unchanged from 0.3.x: `evnx scan` does not
  discover `.env.prod`, `.env.production`, `.env.local` or `.env.staging` when
  scanning a directory. `validate` and `diff` accept them via `--env`.

---

## [0.3.8] - 2026-03-30

### Added

- `evnx backup --key-file <PATH>` — reads encryption password from a file
  for fully non-interactive CI/CD pipelines. UTF-8 content is trimmed of
  surrounding whitespace; binary content is Base64-encoded before Argon2id.
- `evnx backup --keep <N>` (default: 3) — rotates existing backup files before
  each write, preserving the last N backups. Set `--keep 0` to disable rotation.
- `evnx backup --verify` — re-decrypts the backup immediately after writing and
  compares byte-for-byte against the source. Exits with code 6 on mismatch.
- `BackupError::VerifyFailed` — new typed error variant, exit code 6.
- Exit codes table in `evnx backup --help` covering codes 0–6.
- `evnx restore --inspect` — decrypt a backup and list variable key names
  without writing any files. Values are never displayed.
- `evnx restore --password-file <path>` and `EVNX_PASSWORD` environment
  variable for non-interactive restore in CI/CD environments.
- Argon2id progress spinner during the key-derivation step of `evnx restore`.
- Structured exit codes for `evnx restore` covering codes 0–6.

### Changed

- `backup.rs` refactored into `backup/` module — split into `mod.rs`,
  `core.rs`, and `error.rs` following the same structure as `restore/`.
- Password memory safety — password string now wrapped in a `ZeroizeOnDrop`
  RAII guard inside `backup_inner`, guaranteeing zeroization on all exit paths
  including `?`-propagated errors and panics.
- Argon2id spinner replaces static `println!("Encrypting…")` in backup,
  matching the restore command. Suppressed when `--verbose` is active.
- `evnx restore` internal architecture split into `core`, `source`, and
  `error` modules — pure logic is independently testable without a terminal.
- `evnx restore --verbose` now emits a diagnostic line at every pipeline stage
  instead of a single line at startup.
- Password, decrypted content, and ciphertext blob are all zeroized via RAII
  guard on every exit path including panics.
- Consistent header and success summary formatting across backup and restore
  using shared `ui::` helpers.

### Fixed

- Off-by-one in `rotate_backups` — loop range was `(1..keep)`, stopping one
  position short and silently destroying the oldest backup. Corrected to
  `(1..=keep)`.
- `evnx restore <directory>` now reports a clear "not a regular file" error
  instead of a confusing IO message.
- `encrypt_content` re-export visibility corrected to eliminate unused-import
  warning in production builds.

### Security

- Password zeroization is now unconditional on all exit paths via `ZeroizeOnDrop`.
  The previous implementation had a narrow panic window between password
  acceptance and manual `zeroize()` after encryption.

**Full Changelog**: https://github.com/urwithajit9/evnx/compare/v0.3.7...v0.3.8

---

## [0.3.7] - 2026-03-20

### Fixed

- Scoop and Winget publish jobs moved from standalone workflow files into
  `release.yml` as inline jobs. Standalone `release: published` workflows
  never triggered because GitHub suppresses that event when a release is
  created by `GITHUB_TOKEN` inside a workflow.
- Scoop manifest `architecture.url` was writing a literal `$version` string
  instead of the real version number due to incorrect heredoc escaping.
  Corrected to use `${VERSION}` in the architecture block and `\$version`
  only in the autoupdate block where Scoop expects its own template variable.

**Full Changelog**: https://github.com/urwithajit9/evnx/compare/v0.3.6...v0.3.7

---

## [0.3.6] - 2026-03-19

### Added

- Windows package manager support via two new publish channels:
  - Scoop (user-local, no admin required): `scoop bucket add evnx https://github.com/urwithajit9/scoop-evnx && scoop install evnx`
  - Winget (system-wide): `winget install urwithajit9.evnx`
  - Both channels auto-update on every `v*` tag release via GitHub Actions.
- `evnx pre-commit` subcommand for Git pre-commit hook integration. Supports
  validation, secret scanning, and format checks before commit. An example
  hook script is provided in `scripts/`.
- All CLI `--help` outputs now include a link to the corresponding docs page
  at `https://www.evnx.dev/guides/<command>`. Error messages also link to
  relevant docs pages for faster resolution.
- Windows installation guide: https://www.evnx.dev/guides/install/windows
- Pre-commit integration guide: https://www.evnx.dev/guides/pre-commit

### Fixed

- Scan command panic: resolved `thread 'main' panicked at 'index out of bounds'`
  when scanning empty or malformed `.env` files (#142).
- Secret detection false positives: common placeholder values such as
  `your_key_here`, `CHANGEME`, and `***` are no longer flagged.
- Windows path resolution: `.evnx.toml` config lookup now resolves correctly
  on Windows systems.
- SHA256 validation: fixed checksum verification for cross-platform tarball
  downloads.
- Release workflow: `.zip` and `.zip.sha256` artifacts were produced during the
  build but never copied into the GitHub Release assets. Both Scoop and Winget
  installer URLs would 404 on every run. The asset preparation step now copies
  all `*.zip*` files alongside `*.tar.gz*` files.
- Scoop manifest: `bin` field referenced `evnx.exe`, which does not exist
  inside the archive. Corrected to `evnx-x86_64-pc-windows-msvc.exe` with an
  alias mapping it to the `evnx` command.
- Scoop workflow commit step: bare `$VERSION` shell variable was undefined in
  the commit step context. Replaced with `${{ steps.version.outputs.VERSION }}`.
- Scoop workflow hash source: manifest was fetching the `.tar.gz.sha256` file
  for the Windows ZIP installer. Corrected to fetch `.zip.sha256`.

### Changed

- Release workflow now produces `.zip` and `.zip.sha256` alongside `.tar.gz`
  for the Windows target, required for Scoop and Winget compatibility.
- Binary size reduced by approximately 12% using `cargo build --release --strip`.
- Error messages improved with actionable suggestions and direct links to docs.
- Secret scanning patterns updated to detect current AWS, Azure, and GitHub
  token formats.
- Pre-commit hooks run in an isolated subprocess to prevent environment leakage.

### Packaging

| Channel  | Install command                                      | Auto-update |
|----------|------------------------------------------------------|-------------|
| Scoop    | `scoop install evnx`                                 | Yes         |
| Winget   | `winget install urwithajit9.evnx`                    | Yes         |
| Homebrew | `brew install urwithajit9/evnx/evnx`                 | Yes         |
| Cargo    | `cargo install evnx`                                 | Yes         |
| PyPI     | `pipx install evnx`                                  | Yes         |
| npm      | `npm install -g @evnx/cli`                           | Yes         |

**Full Changelog**: https://github.com/urwithajit9/evnx/compare/v0.3.5...v0.3.6

---

## [0.3.5] - 2026-03-16

### Fixed

- PyPI Linux wheel build matrix reduced to x86_64 only. aarch64 and armv7
  targets were removed because cross-compilation inside the manylinux Docker
  container fails when the migrate feature is enabled — reqwest pulls rustls
  which pulls ring, and ring's ARM assembly fails to compile in the
  cross-compilation environment. x86_64 builds natively and is unaffected.
  ARM Linux users can install via the curl script or cargo instead:
  `curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash`
  or `cargo install evnx --features full`.

## [0.3.4] - 2026-03-16

- fix: switch reqwest to native-tls, ring removed from dependency tree


## [0.3.3] - 2026-03-16

### Fixed

- PyPI aarch64 wheel build failure caused by ring crate assembly
  cross-compilation error. Added RING_PREGENERATE_ASM=1 env var to
  maturin linux build job and switched reqwest to rustls-tls-native-roots
  to avoid ring dependency during cross-compilation.
- npm smoke test timing increased to handle registry replication delay.

## [0.3.2] - 2026-03-16

- fix: add features=["full"] to pyproject.toml [tool.maturin]
  so PyPI wheel includes migrate, backup, restore commands

## [0.3.1] - 2026-03-16

Patch release fixing PyPI distribution and Homebrew automation. No changes to
the CLI itself — only release infrastructure and documentation.

### Fixed

- PyPI wheels were published without optional features (`migrate`, `backup`,
  `restore`). All maturin build jobs now include `--features full`, so
  `pipx install evnx` installs the full command set.
- `update-homebrew-tap` job in `release.yml` was incorrectly indented as a
  nested key inside the `release` job. It was never executing. Moved to the
  correct top-level position under `jobs:`.
- Homebrew Formula `install` block updated from `Dir["evnx-*"].first` glob to
  explicit per-platform binary names for reliable installs.

### Added

- Homebrew tap support: `brew install urwithajit9/evnx/evnx`.
  The `update-homebrew-tap` job in `release.yml` now automatically updates
  `urwithajit9/homebrew-evnx` with the correct version and SHA256 checksums
  on every release.
- Homebrew install instructions added to README and release notes template.

### Changed

- README installation section restructured with per-OS pipx setup instructions
  covering macOS, Ubuntu/Debian (including the PEP 668 explanation for 22.04+),
  older Ubuntu (20.04), and Windows.
- Release notes template updated to include the Homebrew install command.

---

## [0.3.0] - 2026-03-14

This release is a comprehensive refactor. The focus is on command consistency,
improved test coverage, and breaking changes to several commands that had
accumulated technical debt from the initial prototype. Users upgrading from
0.2.x should review the breaking changes below before updating.

### Breaking changes

- `evnx init` no longer accepts `--stack` or `--services` flags. The command
  is now fully interactive, using a TUI with three modes: Blank, Blueprint,
  and Architect. Run `evnx init` with no arguments to start.
- `evnx add` interactive flow has been revised. Previous flag-based usage is
  not guaranteed to be compatible.
- Several internal argument names and output formats were normalised for
  consistency across commands. Run `evnx <command> --help` after upgrading.

### Changed

- `evnx init` refactored to interactive TUI with Blank / Blueprint / Architect
  modes replacing the previous `--stack` and `--services` argument approach.
- All commands reviewed and refactored for argument consistency and improved
  error output.
- Test suite significantly expanded across all command paths.

### Fixed

- Shell syntax error in Windows binary extraction during npm publish workflow.
- Dead CHANGELOG link removed from GitHub release notes template.
- npm platform packages now include a stub README to reduce search noise on
  npmjs.com.
- npm install instructions corrected to use `@evnx/cli` package name
  (previously showed unscoped `evnx` which does not exist on npm).
- All documentation and install scripts updated from `dotenv.space` to
  `evnx.dev` domain.

### Added

- `CHANGELOG.md` added to repository.
- Auto-generated "What's Changed" section appended to GitHub releases via
  `generate_release_notes: true`.
- npm badge and PyPI badge added to README.
- `workflow_dispatch` added to `npm-publish.yml` for manual recovery without
  cutting a new release tag.

---

## [0.2.1] - 2026-03-07

### Changed

- Several commands refactored with improved internal structure.
- Test coverage improved across validation and scan paths.

### Fixed

- Various known bugs addressed.

### Added

- npm publish workflow (`npm-publish.yml`) — publishes `@evnx/cli` to npmjs.com
  on each tagged release.
- PyPI publish workflow (`python-publish.yml`) — publishes `evnx` to PyPI via
  maturin on each tagged release. Install with `pipx install evnx`.

---

## [0.2.0] - 2026-03-04

### Breaking changes

- Multiple initial commands revised with updated arguments and behaviour.
  Users upgrading from 0.1.0 should review `evnx --help` for each command.

### Added

- `evnx add` command for adding variables to `.env` interactively from custom
  input, service blueprints, or templates.
- 14+ format targets for `evnx convert`: JSON, YAML, Shell, Docker Compose,
  Kubernetes, Terraform, GitHub Actions, AWS Secrets Manager, GCP Secret
  Manager, Azure Key Vault, Heroku, Vercel, Railway, Doppler.

### Changed

- Secret pattern detection enhanced with improved entropy analysis.
- Error messages for validation failures made more actionable.
- CLI documentation expanded.

### Fixed

- Windows path handling corrected.
- False positives in GitHub token detection reduced.

### Performance

- Validation on large `.env` files approximately 3x faster.

---

## [0.1.0] - 2026-03-01

Initial public release.

### Added

- `evnx init` — stack and service based interactive project setup.
- `evnx validate` — basic validation engine (placeholders, weak secrets,
  misconfigurations).
- `evnx scan` — core secret scanning with pattern matching.
- `evnx diff` — comparison between `.env` and `.env.example`.
- `evnx convert` — basic format conversion.
- `evnx sync` — bidirectional sync between `.env` and `.env.example`.