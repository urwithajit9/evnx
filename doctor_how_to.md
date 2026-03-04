# 🩺 `evnx doctor` — How-to Guide

> **Diagnose and fix environment setup issues in your project**

The `doctor` command is your go-to tool for validating project configuration, security practices, and environment setup. It checks `.env` files, Git configuration, project structure, and more — with optional auto-fix and JSON output for CI/CD.

---

## 📋 Table of Contents

1. [Quick Start](#-quick-start)
2. [What It Checks](#-what-it-checks)
3. [Basic Usage](#-basic-usage)
4. [Advanced Features](#-advanced-features)
5. [CI/CD Integration](#-cicd-integration)
6. [Project-Type Examples](#-project-type-examples)
7. [Troubleshooting](#-troubleshooting)
8. [Best Practices](#-best-practices)
9. [Exit Codes Reference](#-exit-codes-reference)

---

## 🚀 Quick Start

```bash
# Run diagnostics on current directory
$ evnx doctor

┌─ evnx doctor ─────────────────────────────────────┐
│ Diagnosing environment setup                      │
└───────────────────────────────────────────────────┘

Checking .env file...
  ✓ File exists at .env
  ✗ File is NOT in .gitignore (security risk)
  ✓ .env syntax is valid

Checking .env.example...
  ✓ File exists
  ✓ File is tracked in Git

Checking project structure...
  ✓ Detected Python project (requirements.txt)
  ⚠️ python-dotenv not in requirements.txt

Summary:
  🚨 1 critical issue
  ⚠️  1 warning

Overall health: ⚠️ Needs attention

Recommendations:
  Run with EVNX_AUTO_FIX=1 to auto-correct issues
  Or manually address the following:
    • env_file (Validate .env file existence, security, and syntax)
```

---

## 🔍 What It Checks

| Check | Description | Severity if Failed |
|-------|-------------|-------------------|
| **`.env` exists** | Verifies `.env` file is present | ⚠️ Warning |
| **`.env` in `.gitignore`** | Ensures secrets aren't committed | 🚨 **Error** |
| **`.env` syntax** | Validates `KEY=VALUE` format | ⚠️ Warning |
| **File permissions** (Unix) | Checks for `600`/`400` on `.env` | ⚠️ Warning |
| **`.env.example` exists** | Template for team onboarding | ⚠️ Warning |
| **`.env.example` Git-tracked** | Ensures template is versioned | ⚠️ Warning |
| **Project type detection** | Identifies Python/Node/Rust/Go/PHP | ℹ️ Info |
| **Dependency validation** | Checks for dotenv packages | ⚠️ Warning (verbose) |
| **Docker config** | Detects Dockerfile/compose files | ℹ️ Info |

---

## 🎯 Basic Usage

### Check Current Directory
```bash
evnx doctor
```

### Check Specific Project
```bash
evnx doctor --path ./my-service
```

### Verbose Mode (Detailed Output)
```bash
evnx doctor --verbose
```
Shows:
- Exact file permissions (`644` vs `600`)
- Git tracking status details
- Dependency check results
- Docker files found

### Example Output (Verbose)
```
│ Project path: /home/user/my-app

Checking .env file...
  ✓ File exists at .env
  ⚠️ File has 644 permissions (recommended: 600)
  ✓ File is properly ignored by git
  ✓ .env syntax is valid

Checking .env.example...
  ✓ File exists
  ✓ File is tracked in Git
```

---

## ⚙️ Advanced Features

### 🔧 Auto-Fix Mode

Automatically remediate common issues:

```bash
# Enable via environment variable
EVNX_AUTO_FIX=1 evnx doctor --verbose

# What it fixes:
# • Adds `.env` to `.gitignore` if missing
# • Sets `.env` permissions to 600 (Unix only)
# • Reports manual steps for Git tracking
```

> ⚠️ **Note**: Auto-fix requires write permissions to project files. Git operations (like `git add`) still require manual confirmation.

### 📄 JSON Output (CI/CD Ready)

Machine-readable output for automation:

```bash
# Output as JSON
EVNX_OUTPUT_JSON=1 evnx doctor

# Parse with jq
EVNX_OUTPUT_JSON=1 evnx doctor | jq '.summary.errors'

# Save report
EVNX_OUTPUT_JSON=1 evnx doctor > health-report.json
```

#### JSON Schema
```json
{
  "project_path": "./my-app",
  "timestamp": "2024-01-15T10:30:00Z",
  "summary": {
    "total": 5,
    "errors": 1,
    "warnings": 2,
    "passed": 2,
    "info": 0,
    "fixable": 1
  },
  "checks": [
    {
      "name": "env_file",
      "description": "Validate .env file existence, security, and syntax",
      "severity": "error",
      "details": "❌ .env is NOT in .gitignore (security risk)",
      "fixable": true,
      "fixed": false
    }
  ]
}
```

### 🎛️ Combined Flags

```bash
# Verbose + JSON + Auto-fix (full diagnostic pipeline)
EVNX_OUTPUT_JSON=1 EVNX_AUTO_FIX=1 evnx doctor --verbose --path ./api

# Quiet mode (only errors/warnings)
evnx doctor 2>&1 | grep -E "✗|⚠️|🚨"
```

---

## 🔄 CI/CD Integration

### GitHub Actions Example
```yaml
# .github/workflows/env-check.yml
name: Environment Health Check

on: [push, pull_request]

jobs:
  doctor:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install evnx
        run: cargo install evnx

      - name: Run diagnostics
        run: |
          EVNX_OUTPUT_JSON=1 evnx doctor --path ./app > report.json

      - name: Upload report
        uses: actions/upload-artifact@v4
        with:
          name: doctor-report
          path: report.json

      - name: Fail on critical issues
        run: |
          ERRORS=$(jq '.summary.errors' report.json)
          if [ "$ERRORS" -gt 0 ]; then
            echo "🚨 $ERRORS critical issues found"
            exit 1
          fi
```

### GitLab CI Example
```yaml
# .gitlab-ci.yml
env-doctor:
  stage: test
  script:
    - cargo install evnx
    - EVNX_OUTPUT_JSON=1 evnx doctor --path ./service > doctor-report.json
  artifacts:
    paths: [doctor-report.json]
    reports:
      dotenv: doctor-report.json  # Custom reporter integration
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
```

### Exit Codes Reference
| Code | Meaning | Use Case |
|------|---------|----------|
| `0` | ✅ All checks passed or only warnings | Success in CI |
| `1` | 🚨 One or more critical errors | Fail pipeline on security issues |
| `2+` | ⚠️ Runtime errors (IO, Git failures) | Debug tooling issues |

> 💡 **Tip**: Use `|| true` to prevent pipeline failure if you only want to collect diagnostics:
> ```bash
> evnx doctor || true  # Continue even if errors found
> ```

---

## 🐍 Project-Type Examples

### Python (requirements.txt)
```bash
# Setup
echo "FLASK_APP=app.py" > .env
echo "FLASK_APP=" > .env.example
echo ".env" >> .gitignore
git add .env.example

# Check
evnx doctor
# ✓ All Python-specific checks pass
```

### Python (Poetry)
```bash
# Poetry projects use pyproject.toml
evnx doctor --verbose
# ✓ Detects Poetry config
# ℹ️ Suggests pydantic-settings if dotenv not found
```

### Node.js
```bash
# Ensure dotenv is in dependencies
npm install dotenv --save

# Check
evnx doctor --verbose
# ✓ Detects package.json
# ℹ️ Confirms 'dotenv' package presence
```

### Rust
```bash
# Add dotenvy to Cargo.toml
# [dependencies]
# dotenvy = "0.15"

evnx doctor
# ✓ Detects Cargo.toml
# ℹ️ Validates dotenvy/dotenv crate
```

### Multi-Service Repository
```bash
# Check each service independently
for dir in services/*/; do
  echo "Checking $dir..."
  evnx doctor --path "$dir" || echo "❌ Issues in $dir"
done
```

---

## 🛠️ Troubleshooting

### ❌ "File is NOT in .gitignore" but it is
```bash
# Check for exact match (no trailing spaces)
cat .gitignore | grep -n "^\.env$"

# Fix: Ensure no leading/trailing whitespace
echo ".env" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' >> .gitignore
```

### ❌ Permission check fails on Windows
```bash
# Expected: Permission checks are skipped on Windows
# If you see an error, ensure you're on latest evnx version

# Workaround: Manually verify file properties
# Right-click .env → Properties → Security tab
```

### ❌ Git commands fail ("not a git repository")
```bash
# Doctor gracefully degrades when Git is unavailable
# To suppress Git-related warnings:
git init  # Initialize repo if needed

# Or check outside Git:
evnx doctor --verbose 2>&1 | grep -v "git"
```

### ❌ JSON output is malformed
```bash
# Ensure no other output interferes
EVNX_OUTPUT_JSON=1 evnx doctor 2>/dev/null | jq .

# Check for debug prints in verbose mode
# Avoid --verbose when parsing JSON in scripts
```

### ❌ Auto-fix doesn't apply all changes
```bash
# Auto-fix limitations:
# • Git operations require manual confirmation (security)
# • Windows permission fixes not supported

# Manual follow-up:
git add .env.example && git commit -m "chore: add env template"
```

---

## 📚 Best Practices

### ✅ For Development
```bash
# 1. Run doctor after cloning a repo
git clone my-project && cd my-project
evnx doctor

# 2. Add to pre-commit hooks (optional)
# .pre-commit-config.yaml
- repo: local
  hooks:
    - id: env-doctor
      name: Environment Health Check
      entry: evnx doctor
      language: system
      pass_filenames: false
```

### ✅ For Production Deployments
```bash
# 1. Validate environment before deploy
if ! EVNX_OUTPUT_JSON=1 evnx doctor | jq -e '.summary.errors == 0'; then
  echo "🚨 Environment issues detected - aborting deploy"
  exit 1
fi

# 2. Log report for audit
EVNX_OUTPUT_JSON=1 evnx doctor >> deploy-log.json
```

### ✅ For Team Onboarding
```bash
# 1. Include in README
## Setup
1. Clone repo
2. Run `evnx doctor` to validate environment
3. Fix any reported issues
4. Run `evnx init` if .env.example exists

# 2. Share baseline report
evnx doctor --verbose > .evnx-baseline.txt
# Commit baseline for reference (not .env!)
```

### ✅ Security Checklist
- [ ] `.env` is in `.gitignore` (🚨 critical)
- [ ] `.env` permissions are `600` (Unix)
- [ ] `.env.example` contains no real secrets
- [ ] `.env.example` is tracked in Git
- [ ] No hardcoded secrets in source code

---

## 🧪 Testing Your Configuration

### Create a Test Project
```bash
mkdir test-env && cd test-env
git init

# Create minimal config
echo "TEST_VAR=value" > .env
echo "TEST_VAR=" > .env.example
echo ".env" > .gitignore
echo '{"name":"test"}' > package.json  # Node.js marker

# Run doctor
evnx doctor
# Expected: ✓ All checks pass
```

### Simulate Issues
```bash
# Remove .env from gitignore
sed -i '/\.env/d' .gitignore
evnx doctor
# Expected: ✗ File is NOT in .gitignore

# Fix with auto-fix
EVNX_AUTO_FIX=1 evnx doctor
# Expected: ✓ Added .env to .gitignore
```

---

## 📦 Extending Doctor (For Contributors)

### Add a New Check
```rust
// 1. Implement DiagnosticCheck trait
struct MyNewCheck;

impl DiagnosticCheck for MyNewCheck {
    fn name(&self) -> &'static str { "my_check" }
    fn description(&self) -> &'static str { "What this validates" }

    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult> {
        // Your validation logic
        Ok(CheckResult { /* ... */ })
    }
}

// 2. Register in get_all_checks()
fn get_all_checks() -> Vec<Box<dyn DiagnosticCheck>> {
    vec![
        // ... existing checks
        Box::new(MyNewCheck),
    ]
}
```

### Customize Output
```rust
// Use existing UI functions from utils/ui.rs
use crate::utils::ui;

ui::success("Custom success message");
ui::warning("Custom warning");
ui::error("Custom error");
ui::info("Custom info");
```

---

## 🆘 Getting Help

```bash
# View built-in help
evnx doctor --help

# Report issues
# https://github.com/urwithajit9/evnx/issues

# Check version
evnx --version
```

### Common Questions

**Q: Can I disable specific checks?**
A: Not yet — but you can filter output:
```bash
evnx doctor 2>&1 | grep -v "docker_config"
```

**Q: Does doctor modify my files?**
A: Only with `EVNX_AUTO_FIX=1`, and only for safe operations (gitignore, permissions). Git commands always require manual confirmation.

**Q: Can I use this in a monorepo?**
A: Yes! Run with `--path` for each service:
```bash
for service in services/*; do
  evnx doctor --path "$service"
done
```

**Q: How do I contribute a new project type?**
A: Add detection logic to `detect_project_type()` and validation to `ProjectStructureCheck`. See [CONTRIBUTING.md](https://github.com/urwithajit9/evnx/blob/main/CONTRIBUTING.md).

---

> 💡 **Pro Tip**: Run `evnx doctor` as part of your `pre-push` hook to catch environment issues before they reach CI:
> ```bash
> # .git/hooks/pre-push
> #!/bin/bash
> evnx doctor || {
>   echo "🚨 Fix environment issues before pushing"
>   exit 1
> }
> ```

---

*Documentation version: 0.2.0 | Last updated: March 2024*
*For the latest docs, visit: https://github.com/urwithajit9/evnx/blob/main/docs/doctor.md* 🩺✨