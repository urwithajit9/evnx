# Changelog

All notable changes to this project are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [0.3.0] - 2026-03-17

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