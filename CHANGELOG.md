# Changelog

All notable changes to evnx are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Backup Command

### Added

- **`--key-file <PATH>`** flag on `evnx backup` — reads a file and uses its
  contents as the encryption password, enabling fully non-interactive CI/CD
  pipelines without storing a password in environment variables. UTF-8 content
  is trimmed of surrounding whitespace; binary content is Base64-encoded before
  being fed into Argon2id. A warning is emitted if both `--key-file` and a
  future `--password` flag are supplied simultaneously.

- **`--keep <N>`** flag on `evnx backup` (default: `3`) — rotates existing
  backup files before each write so the last N backups are preserved alongside
  the new one. Rotation renames in reverse order (`.backup.{N-1}` → `.backup.N`
  first, then down to `.backup` → `.backup.1`) to prevent mid-chain clobbering.
  Files at the overflow position are warned about but never deleted. Set `--keep 0`
  to disable rotation and overwrite silently.

- **`--verify`** flag on `evnx backup` — immediately re-decrypts the backup file
  after writing and compares the recovered content byte-for-byte against the
  original source. Exits with code 6 on mismatch and leaves the file on disk for
  manual inspection. Costs one additional Argon2id round (~1 s).

- **`BackupError::VerifyFailed`** — new typed error variant, exit code 6. Covers
  both re-decryption failures and content-mismatch failures from `--verify`.

- **Exit codes table** in `evnx backup --help` (`docs::BACKUP.after_help`):

  | Code | Meaning |
  |------|---------|
  | 0 | Success |
  | 1 | Generic error (IO, unexpected failure) |
  | 2 | Source file not found or not a regular file |
  | 3 | Password confirmation did not match |
  | 4 | Encryption failed |
  | 5 | Failed to write backup file |
  | 6 | Post-write integrity check failed (`--verify`) |

### Changed

- **`backup.rs` refactored into `backup/` module** — monolithic file split into
  three focused files following the same structure as `restore/`:
  - `mod.rs` — CLI adapter: header, password prompts, key-file resolution,
    orchestration. No pure logic.
  - `core.rs` — Pure logic: `BackupOptions`, `backup_inner`, `rotate_backups`,
    `verify_backup`, `encrypt_content`, `decrypt_content`, `BackupMetadata`.
    Fully testable without a TTY.
  - `error.rs` — `BackupError` enum with exit codes, `Display`, and
    `std::error::Error`.

- **Password memory safety** — password string is now wrapped in a
  `ZeroizeOnDrop` RAII guard inside `backup_inner` immediately on entry,
  guaranteeing zeroization on every exit path including `?`-propagated errors
  and panics. Previously the password was zeroized manually after
  `encrypt_content` returned, leaving a window if encryption panicked.

- **Argon2id spinner** — the static `println!("Encrypting…")` line has been
  replaced with `ui::spinner()` + `finish_and_clear()`, matching the restore
  command. Suppressed automatically when `--verbose` is active to avoid
  interleaved output.

- **Consistent header** — the hand-rolled `┌─ Create encrypted backup ───┐`
  block has been replaced with `ui::print_header("evnx backup", …)`, matching
  every other subcommand.

- **Verbose diagnostics** — `--verbose` now emits a `ui::verbose_stderr` line
  at every pipeline stage (source path, bytes read, password acceptance,
  rotation, encryption, write, verify) instead of a single dimmed line at
  startup.

- **Success summary** — post-write output now uses `ui::print_key_value` with
  Source / Backup / Size / Verified fields instead of bare `println!` calls.

- **Next-steps block** — the "⚠️ Important:" section now uses a private
  `print_next_steps()` helper (mirroring `restore/core.rs`) instead of
  inline `println!` bullets.

- **`run()` signature** extended with `key_file: Option<String>`, `keep: u32`,
  `verify: bool`. `cli.rs` `Commands::Backup` variant updated with matching
  `--key-file`, `--keep`, `--verify` arguments. `main.rs` dispatch arm updated
  to destructure and forward all new fields.

- **`BackupError` wired to `main.rs`** — the dispatch arm now downcasts to
  `BackupError` and calls `exit_code()`, matching the restore dispatch pattern.
  Previously all backup failures mapped to exit code 1.

### Fixed

- **Off-by-one in `rotate_backups`** — loop range was `(1..keep)` which stopped
  one position short, silently destroying the oldest backup in the chain instead
  of shifting it. Corrected to `(1..=keep)`.

- **`encrypt_content` re-export visibility** — changed from
  `#[cfg(feature = "backup")]` to `#[cfg(all(feature = "backup", test))]` to
  eliminate the unused-import warning in production builds. The symbol is only
  consumed by `restore/core.rs` integration tests.

- **Unused `colored::Colorize` import** — removed from `run()`'s feature block;
  all colour output in that scope flows through `ui::` helpers which import the
  trait internally.

### Security

- Password zeroization is now unconditional on all exit paths via `ZeroizeOnDrop`
  (see Changed above). The previous implementation had a narrow panic window
  between password acceptance and manual `zeroize()` after encryption.


### Added Restore Command

- **`evnx restore --inspect`** — decrypt a backup and list variable key
  names without writing any files. Values are never displayed. Useful for
  confirming backup contents before a full restore.

- **Non-interactive password input for `evnx restore`** — two new options
  for CI/CD environments where interactive prompts are not possible:
  - `--password-file <path>` — read the decryption password from a file.
  - `EVNX_PASSWORD` environment variable — read and immediately unset.
  Both options are less secure than the interactive prompt and print a
  warning to stderr. The interactive prompt remains the default.

- **Argon2id progress spinner** — a live spinner is shown during the
  key-derivation step of `evnx restore`, which is deliberately slow (~1 s).
  Suppressed automatically in `--verbose` mode.

- **Structured exit codes for `evnx restore`** — shell scripts can now
  branch on specific failure modes:

  | Code | Meaning                                          |
  |------|--------------------------------------------------|
  | 0    | Success                                          |
  | 1    | Generic error (IO, encoding, etc.)               |
  | 2    | Wrong password or corrupt backup                 |
  | 3    | Backup file not found                            |
  | 4    | Restore cancelled by user                        |
  | 5    | Restored to `.restored` fallback (bad content)   |
  | 6    | evnx-cloud restore not yet available             |

### Changed

- `evnx restore` internal architecture split into focused modules
  (`core`, `source`, `error`) — pure logic is now independently testable
  without a terminal. No user-facing behaviour changes.

- `evnx restore --verbose` now emits a diagnostic line at every pipeline
  stage (source, output path, KDF start, variable count, schema version)
  instead of a single line at the start.

- Password is now zeroized via a RAII guard on every exit path including
  `?`-propagated errors and panics. Decrypted content and ciphertext blob
  are also explicitly zeroized after use.

### Fixed

- `evnx restore <directory>` now reports a clear "not a regular file"
  error instead of a confusing IO message.

- Metadata block (schema version, original file, created-at, variable
  count) was duplicated between `--dry-run` and the normal path — now
  rendered by a single `print_metadata()` helper.

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