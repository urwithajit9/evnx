# Contributing to evnx

Thank you for considering contributing to evnx! This document provides guidelines and instructions for contributing.

## 🌟 Ways to Contribute

- 🐛 Report bugs and issues
- 💡 Suggest new features or improvements
- 📝 Improve documentation
- 🔧 Submit pull requests
- 🌍 Help with translations (future)
- 💬 Answer questions in discussions
- ⭐ Star the repository and spread the word

## 📋 Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Changes](#making-changes)
- [Submitting Changes](#submitting-changes)
- [Coding Standards](#coding-standards)
- [Testing Guidelines](#testing-guidelines)
- [Documentation](#documentation)
- [Community](#community)

---

## 📜 Code of Conduct

### Our Pledge

We are committed to providing a welcoming and inclusive environment for everyone, regardless of:
- Experience level
- Gender identity and expression
- Sexual orientation
- Disability
- Personal appearance
- Body size
- Race
- Ethnicity
- Age
- Religion
- Nationality

### Expected Behavior

- ✅ Be respectful and considerate
- ✅ Welcome newcomers and help them learn
- ✅ Accept constructive criticism gracefully
- ✅ Focus on what's best for the community
- ✅ Show empathy towards other community members

### Unacceptable Behavior

- ❌ Harassment, trolling, or insulting comments
- ❌ Personal or political attacks
- ❌ Publishing others' private information
- ❌ Any conduct inappropriate in a professional setting

**Enforcement:** Violations may result in temporary or permanent ban from the project.

---

## 🚀 Getting Started

### Prerequisites

Before contributing, ensure you have:

- **Rust toolchain** (stable, 1.70+)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

- **Git**
  ```bash
  git --version
  ```

- **A GitHub account**

### First-Time Contributors

Looking for where to start? Check out:
- Issues labeled [`good first issue`](https://github.com/urwithajit9/evnx/labels/good%20first%20issue)
- Issues labeled [`help wanted`](https://github.com/urwithajit9/evnx/labels/help%20wanted)
- [Documentation improvements](https://github.com/urwithajit9/evnx/labels/documentation)

**Don't be shy!** Everyone was a beginner once. We're here to help.

---

## 🛠️ Development Setup

### 1. Fork and Clone

```bash
# Fork the repository on GitHub, then:
git clone https://github.com/YOUR_USERNAME/evnx.git
cd evnx

# Add upstream remote
git remote add upstream https://github.com/urwithajit9/evnx.git
```

### 2. Install Dependencies

```bash
# All dependencies are in Cargo.toml
# Just build to fetch them
cargo build
```

### 3. Verify Setup

```bash
# Run tests
cargo test

# Run clippy (linter)
cargo clippy --all-features -- -D warnings

# Format code
cargo fmt

# Build with all features
cargo build --all-features

# Run the CLI
cargo run -- --help
```

### 4. Create a Branch

```bash
# Create a new branch for your work
git checkout -b feature/your-feature-name

# Or for bug fixes
git checkout -b fix/issue-123
```

**Branch naming conventions:**
- `feature/` - New features
- `fix/` - Bug fixes
- `docs/` - Documentation changes
- `refactor/` - Code refactoring
- `test/` - Test improvements
- `chore/` - Maintenance tasks

---

## 🔨 Making Changes

### Code Changes

1. **Follow Rust conventions**
   - Use `cargo fmt` for formatting
   - Pass `cargo clippy` without warnings
   - Write idiomatic Rust

2. **Add tests**
   - Unit tests for individual functions
   - Integration tests for CLI commands
   - Document test purpose with comments

3. **Update documentation**
   - Update README.md if adding features
   - Add/update doc comments (`///`)
   - Update docs/ files if needed

### Documentation Changes

- Use clear, concise language
- Include examples where helpful
- Check spelling and grammar
- Test any code snippets
- Update table of contents if needed

### Example Changes

**Good commit:**
```rust
/// Validates that a variable name follows naming conventions.
///
/// # Arguments
/// * `name` - The variable name to validate
///
/// # Returns
/// * `Ok(())` if valid
/// * `Err(ValidationError)` if invalid
///
/// # Examples
/// ```
/// assert!(validate_var_name("DATABASE_URL").is_ok());
/// assert!(validate_var_name("123invalid").is_err());
/// ```
pub fn validate_var_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        return Err(ValidationError::EmptyName);
    }

    if !name.chars().next().unwrap().is_alphabetic() {
        return Err(ValidationError::InvalidStart);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_var_name() {
        assert!(validate_var_name("VALID_NAME").is_ok());
        assert!(validate_var_name("123invalid").is_err());
        assert!(validate_var_name("").is_err());
    }
}
```

---

## 📤 Submitting Changes

### Before Submitting

Run the full test suite:

```bash
# Format code
cargo fmt

# Run linter
cargo clippy --all-features -- -D warnings

# Run all tests
cargo test --all-features

# Test individual features
cargo test --no-default-features
cargo test --features migrate
cargo test --features backup

# Build release to ensure it compiles
cargo build --release --all-features
```

### Commit Guidelines

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types:**
- `feat:` New feature
- `fix:` Bug fix
- `docs:` Documentation changes
- `style:` Code style changes (formatting, etc.)
- `refactor:` Code refactoring
- `test:` Adding or updating tests
- `chore:` Maintenance tasks

**Examples:**
```
feat(scan): add support for Anthropic API key detection

- Added pattern for Anthropic API keys (sk-ant-...)
- Added tests for new pattern
- Updated documentation

Closes #123
```

```
fix(validate): handle empty .env files gracefully

Previously crashed with panic on empty files.
Now returns appropriate error message.

Fixes #456
```

### Pull Request Process

1. **Update your branch**
   ```bash
   git fetch upstream
   git rebase upstream/main
   ```

2. **Push to your fork**
   ```bash
   git push origin feature/your-feature-name
   ```

3. **Create Pull Request**
   - Go to your fork on GitHub
   - Click "New Pull Request"
   - Select your branch
   - Fill out the PR template

4. **PR Description Template**
   ```markdown
   ## Description
   Brief description of changes

   ## Type of Change
   - [ ] Bug fix
   - [ ] New feature
   - [ ] Breaking change
   - [ ] Documentation update

   ## Testing
   - [ ] Unit tests added/updated
   - [ ] Integration tests added/updated
   - [ ] Manual testing performed

   ## Checklist
   - [ ] Code follows project style
   - [ ] Self-review completed
   - [ ] Comments added where needed
   - [ ] Documentation updated
   - [ ] Tests pass locally
   - [ ] No new warnings from clippy

   ## Related Issues
   Closes #123
   ```

5. **Address Review Comments**
   - Respond to all comments
   - Make requested changes
   - Push updates to the same branch
   - Re-request review

---

## 📏 Coding Standards

### Rust Style Guide

Follow the [Rust Style Guide](https://doc.rust-lang.org/1.0.0/style/):

```rust
// Good
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

// Bad - inconsistent naming, unclear logic
fn calcEntropy(d: &str) -> f64 {
    let b=d.as_bytes();let mut c=HashMap::new();
    for &x in b{*c.entry(x).or_insert(0)+=1;}
    // ... (rest of implementation)
}
```

### Error Handling

Use `Result` and proper error types:

```rust
// Good
pub fn read_env_file(path: &Path) -> Result<String, EnvError> {
    fs::read_to_string(path)
        .map_err(|e| EnvError::FileRead {
            path: path.to_path_buf(),
            source: e,
        })
}

// Bad - using unwrap
pub fn read_env_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap()  // ❌ Can panic!
}
```

### Documentation

Use doc comments for public items:

```rust
/// Scans a directory for accidentally committed secrets.
///
/// # Arguments
/// * `path` - Directory to scan
/// * `options` - Scan configuration options
///
/// # Returns
/// * `Ok(ScanResults)` - Found secrets and statistics
/// * `Err(ScanError)` - I/O errors or configuration issues
///
/// # Examples
/// ```
/// use evnx::scan::{scan_directory, ScanOptions};
///
/// let options = ScanOptions::default();
/// let results = scan_directory("./src", options)?;
/// println!("Found {} secrets", results.count);
/// ```
pub fn scan_directory(path: &Path, options: ScanOptions) -> Result<ScanResults, ScanError> {
    // Implementation
}
```

### Testing

Write comprehensive tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_env_file() {
        let content = "DATABASE_URL=postgres://localhost\nAPI_KEY=test123";
        let result = parse_env(content);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[test]
    fn test_empty_env_file() {
        let content = "";
        let result = parse_env(content);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_malformed_env_file() {
        let content = "INVALID LINE WITHOUT EQUALS";
        let result = parse_env(content);
        assert!(result.is_err());
    }
}
```

---

## 🧪 Testing Guidelines

### Running Tests

```bash
# All tests
cargo test --all-features

# Specific test
cargo test test_validate_var_name

# With output
cargo test -- --nocapture

# Integration tests only
cargo test --test integration_tests
```

### Test Coverage

We aim for:
- **80%+ code coverage** for core functionality
- **100% coverage** for security-critical code (parsing, scanning, validation)
- Integration tests for all CLI commands

### Writing Tests

1. **Unit tests** - Test individual functions
2. **Integration tests** - Test CLI commands end-to-end
3. **Property tests** - Test with random inputs (using proptest)

**Example integration test:**
```rust
#[test]
fn test_validate_command() {
    let temp_dir = tempdir().unwrap();
    let env_path = temp_dir.path().join(".env");

    fs::write(&env_path, "DATABASE_URL=postgres://localhost").unwrap();

    let output = Command::cargo_bin("evnx")
        .unwrap()
        .arg("validate")
        .arg("--strict")
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
}
```

---

## 📚 Documentation

### What to Document

- **Public APIs** - All public functions, structs, enums
- **Examples** - Show how to use the feature
- **Edge cases** - Document unexpected behavior
- **Errors** - What errors can occur and why

### Documentation Files

- `README.md` - Overview and quick start
- `docs/GETTING_STARTED.md` - Detailed walkthrough
- `docs/USE_CASES.md` - Real-world examples
- `docs/CICD_GUIDE.md` - CI/CD integration
- `ARCHITECTURE.md` - System design
- `CHANGELOG.md` - Version history

### Updating Documentation

When adding features:
1. Update relevant docs/ files
2. Add examples to README.md
3. Update CHANGELOG.md (unreleased section)
4. Add doc comments to code

---

## 🤝 Community

### Getting Help

- 💬 [GitHub Discussions](https://github.com/urwithajit9/evnx/discussions) - Ask questions
- 🐛 [Issue Tracker](https://github.com/urwithajit9/evnx/issues) - Report bugs
- 📧 Email: support@dotenv.space

### Communication Channels

- **GitHub Discussions** - Questions, ideas, show & tell
- **Issue Tracker** - Bug reports, feature requests
- **Pull Requests** - Code review and discussion

### Response Times

We try to:
- Respond to issues within 48 hours
- Review PRs within 1 week
- Cut releases monthly (or as needed for security)

---

## 🎯 Development Priorities

### High Priority
- Security improvements (secret detection patterns)
- Performance optimization
- Windows support improvements
- Documentation

### Medium Priority
- New format converters
- Additional secret patterns
- CLI UX improvements
- Integration examples

### Low Priority
- Translations (i18n)
- GUI wrapper
- Web service version
- Plugins/extensions

---

## 🏆 Recognition

Contributors are recognized in:
- `CONTRIBUTORS.md` file
- Release notes
- GitHub contributors graph

Significant contributions may be highlighted in:
- Blog posts
- Social media
- Project website

---

## 📝 License

By contributing, you agree that your contributions will be licensed under the MIT License.

---

## ❓ Questions?

Don't hesitate to ask! There are no stupid questions.

- Open a [Discussion](https://github.com/urwithajit9/evnx/discussions)
- Comment on an existing issue
- Email: support@dotenv.space

**Thank you for making evnx better!** 🙏

---

<div align="center">

**Made with 🦀 Rust and ❤️ by contributors like you**

[Website](https://dotenv.space) • [Documentation](./docs/GETTING_STARTED.md) • [GitHub](https://github.com/urwithajit9/evnx)

</div>