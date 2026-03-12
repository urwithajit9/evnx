# Contributing to evnx

Thank you for your interest in contributing to **evnx**. Contributions from the community help improve the project, expand its capabilities, and make it more reliable for everyone.

This guide outlines the process for contributing code, documentation, and ideas.

---

# Ways to Contribute

There are many ways to contribute to the project:

* Report bugs or unexpected behavior
* Suggest new features or improvements
* Improve or expand documentation
* Submit pull requests
* Participate in discussions and answer questions
* Share the project within your community

Every contribution, large or small, is appreciated.

---

# Table of Contents

* Code of Conduct
* Getting Started
* Development Setup
* Making Changes
* Submitting Changes
* Coding Standards
* Testing Guidelines
* Documentation
* Community

---

# Code of Conduct

We are committed to fostering a welcoming and respectful community for contributors of all backgrounds and experience levels.

## Expected Behavior

Contributors are expected to:

* Be respectful and constructive in communication
* Welcome and support new contributors
* Accept feedback and code review professionally
* Focus discussions on improving the project

## Unacceptable Behavior

The following behaviors will not be tolerated:

* Harassment, personal attacks, or discriminatory language
* Publishing private information without consent
* Trolling or intentionally disruptive behavior
* Any conduct inappropriate for a professional environment

Project maintainers may take appropriate action, including removal from the community, in response to violations.

---

# Getting Started

## Prerequisites

Before contributing, ensure the following tools are installed:

### Rust Toolchain

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Recommended Rust version: **1.70 or newer**

### Git

```bash
git --version
```

### GitHub Account

You will need a GitHub account to fork the repository and submit pull requests.

---

## First-Time Contributors

If you are new to the project, consider starting with issues labeled:

* `good first issue`
* `help wanted`
* `documentation`

These tasks are designed to help new contributors get familiar with the codebase.

---

# Development Setup

## 1. Fork and Clone the Repository

Fork the repository on GitHub and then clone it locally:

```bash
git clone https://github.com/YOUR_USERNAME/evnx.git
cd evnx
```

Add the upstream repository:

```bash
git remote add upstream https://github.com/urwithajit9/evnx.git
```

---

## 2. Install Dependencies

All dependencies are defined in `Cargo.toml`.

```bash
cargo build
```

This command will automatically download and build required dependencies.

---

## 3. Verify the Environment

Run the following commands to ensure your environment is correctly configured:

```bash
cargo test
cargo clippy --all-features -- -D warnings
cargo fmt
cargo build --all-features
cargo run -- --help
```

---

## 4. Create a Feature Branch

Always create a branch before making changes.

```bash
git checkout -b feature/your-feature-name
```

Common branch prefixes:

| Prefix    | Purpose               |
| --------- | --------------------- |
| feature/  | New features          |
| fix/      | Bug fixes             |
| docs/     | Documentation updates |
| refactor/ | Code improvements     |
| test/     | Test additions        |
| chore/    | Maintenance tasks     |

---

# Making Changes

## Code Contributions

When contributing code:

* Follow standard Rust conventions
* Ensure `cargo fmt` formatting is applied
* Ensure `cargo clippy` passes without warnings
* Write clear and maintainable code
* Include tests where applicable

## Documentation Improvements

Documentation contributions are highly valued.

When editing documentation:

* Use clear and concise language
* Include examples where helpful
* Verify that code snippets compile or run correctly
* Check spelling and grammar

---

# Submitting Changes

Before submitting a pull request, run the full test suite.

```bash
cargo fmt
cargo clippy --all-features -- -D warnings
cargo test --all-features
cargo build --release --all-features
```

You may also test individual features:

```bash
cargo test --no-default-features
cargo test --features migrate
cargo test --features backup
```

---

# Commit Guidelines

This project follows the **Conventional Commits** specification.

Format:

```
<type>(scope): short description
```

Common commit types:

| Type     | Description           |
| -------- | --------------------- |
| feat     | New feature           |
| fix      | Bug fix               |
| docs     | Documentation updates |
| refactor | Code restructuring    |
| test     | Test improvements     |
| chore    | Maintenance tasks     |

Example:

```
feat(scan): add support for Anthropic API key detection

- Added detection pattern
- Added associated tests
- Updated documentation
```

---

# Pull Request Process

1. Update your branch with the latest changes:

```bash
git fetch upstream
git rebase upstream/main
```

2. Push your branch:

```bash
git push origin feature/your-feature-name
```

3. Open a Pull Request on GitHub.

4. Complete the pull request template describing your changes.

5. Address feedback from maintainers during review.

---

# Coding Standards

## Rust Style

Follow the official Rust style guidelines.

```rust
fn calculate_entropy(data: &str) -> f64 {
    let bytes = data.as_bytes();
    let mut counts = HashMap::new();

    for &byte in bytes {
        *counts.entry(byte).or_insert(0) += 1;
    }

    let len = bytes.len() as f64;

    counts.values()
        .map(|&count| {
            let p = count as f64 / len;
            -p * p.log2()
        })
        .sum()
}
```

Avoid patterns that may cause runtime panics such as unnecessary `unwrap()` usage.

---

## Error Handling

Prefer structured error handling using `Result`.

```rust
pub fn read_env_file(path: &Path) -> Result<String, EnvError> {
    fs::read_to_string(path)
        .map_err(|e| EnvError::FileRead {
            path: path.to_path_buf(),
            source: e,
        })
}
```

---

# Testing Guidelines

Testing is essential for maintaining reliability.

## Running Tests

```bash
cargo test --all-features
cargo test test_validate_var_name
cargo test -- --nocapture
cargo test --test integration_tests
```

## Coverage Goals

* 80%+ coverage for core modules
* 100% coverage for security-critical logic (parsing, scanning, validation)
* Integration tests for CLI commands

Types of tests used:

* Unit tests
* Integration tests
* Property tests (via `proptest`)

---

# Documentation

Good documentation is critical for adoption.

Documentation should cover:

* Public APIs
* Usage examples
* Error conditions
* Edge cases

Project documentation files include:

* `README.md`
* `docs/GETTING_STARTED.md`
* `docs/USE_CASES.md`
* `docs/CICD_GUIDE.md`
* `ARCHITECTURE.md`
* `CHANGELOG.md`

Whenever a new feature is added:

1. Update relevant documentation
2. Add usage examples
3. Update the changelog
4. Add doc comments to code

---

# Community

## Support and Discussion

Community communication happens primarily through GitHub.

* GitHub Discussions — questions and ideas
* Issue Tracker — bug reports and feature requests
* Pull Requests — code review and implementation discussions

Contact email:

[support@dotenv.space](mailto:support@dotenv.space)

---

## Response Expectations

Project maintainers aim to:

* Respond to issues within **48 hours**
* Review pull requests within **one week**
* Publish releases periodically or when necessary for security updates

---

# Project Priorities

### High Priority

* Security improvements
* Performance optimization
* Windows compatibility improvements
* Documentation improvements

### Medium Priority

* Additional secret detection patterns
* CLI usability improvements
* Integration examples

### Long-Term

* Internationalization
* Plugin ecosystem
* Optional web interface

---

# Contributor Recognition

Contributors are recognized through:

* `CONTRIBUTORS.md`
* Release notes
* GitHub contributors page

Significant contributions may also be highlighted in project announcements.

---

# License

By contributing to **evnx**, you agree that your contributions will be licensed under the **MIT License**.

---

# Questions

If you have questions about contributing:

* Open a discussion on GitHub
* Comment on an issue
* Contact: [support@dotenv.space](mailto:support@dotenv.space)

Thank you for helping improve **evnx**.

---

**evnx**
Secure environment management for modern development workflows.

Website: [https://dotenv.space](https://dotenv.space)
Repository: [https://github.com/urwithajit9/evnx](https://github.com/urwithajit9/evnx)


