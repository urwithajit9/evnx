# 📘 User Guide: `evnx validate` Command

> **Validate environment configuration against best practices, security standards, and your `.env.example` template.**

---

## 🎯 Quick Start

```bash
# Basic validation (pretty output)
evnx validate

# Auto-fix common issues
evnx validate --fix

# Strict mode + JSON output for CI
evnx validate --strict --format json

# Validate production env with format checks
evnx validate --pattern .env.production --validate-formats

# Ignore specific warnings
evnx validate --ignore boolean_trap,localhost_in_docker
```

---

## 📋 What Does `validate` Do?

The `validate` command compares your `.env` file against a `.env.example` template and performs comprehensive checks:

| Check | Severity | Description |
|-------|----------|-------------|
| 🔍 Missing Variables | `error` | Variables in `.env.example` not present in `.env` |
| 🔍 Extra Variables | `warning` | Variables in `.env` not defined in `.env.example` (strict mode) |
| 🔍 Placeholder Values | `error` | Values like `changeme`, `your_key_here`, `<placeholder>` |
| 🔍 Boolean String Trap | `warning` | `DEBUG=True` (string) vs `DEBUG=true` (boolean) |
| 🔍 Weak SECRET_KEY | `error` | Keys shorter than 32 chars or containing weak patterns |
| 🔍 localhost in Docker | `warning` | `localhost` URLs when Docker files are detected |
| 🔍 Format Validation | `warning/error` | Invalid URLs, ports, or emails (with `--validate-formats`) |

---

## ⚙️ Command Options

```bash
evnx validate [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--env <PATH>` | String | `.env` | Path to your environment file |
| `--example <PATH>` | String | `.env.example` | Path to your example/template file |
| `--strict` | Flag | `false` | Warn about extra variables not in `.env.example` |
| `--fix` | Flag | `false` | Auto-fix common issues (placeholders, booleans, weak secrets) |
| `--format <FORMAT>` | String | `pretty` | Output format: `pretty`, `json`, `github-actions` |
| `--exit-zero` | Flag | `false` | Always exit with code 0 (useful for CI) |
| `--ignore <TYPES>` | List | `[]` | Comma-separated issue types to suppress |
| `--validate-formats` | Flag | `false` | Enable URL/port/email format validation |
| `--pattern <PATTERN>` | String | `None` | Use `.env.*` files like `.env.production`, `.env.local` |
| `-v, --verbose` | Flag | `false` | Enable verbose output (global flag) |

---

## 🎨 Output Formats

### Pretty (Default) — For Humans

```
┌─ evnx validate ────────────────────────────────────────────┐
│ Check environment configuration                           │
└───────────────────────────────────────────────────────────┘

📋 Preview:

⚠️ Issues Found:
  1. 🚨 Missing required variable: API_KEY
     → Add API_KEY=<value> to .env
     💡 Auto-fixable with --fix
     📍 .env:?

  2. ⚠️ DEBUG is set to "False" (string, not boolean)
     → Use false or 0 for proper boolean handling
     💡 Auto-fixable with --fix
     📍 .env:?

┌─ Summary ─────────────────────────────────────────────────┐
│ Errors: 1  |  Warnings: 1  |  Fixed: 0                   │
└──────────────────────────────────────────────────────────┘

Next steps:
  1. Review and fix the errors above
  2. Run with --fix to auto-correct common issues
  3. Use --ignore issue_type to suppress specific warnings
```

### JSON — For Machines & Tooling

```bash
evnx validate --format json | jq '.summary'
```

```json
{
  "status": "failed",
  "required_present": 8,
  "required_total": 10,
  "issues": [
    {
      "severity": "error",
      "type": "missing_variable",
      "variable": "API_KEY",
      "message": "Missing required variable: API_KEY",
      "location": ".env:?",
      "suggestion": "Add API_KEY=<value> to .env",
      "auto_fixable": true
    }
  ],
  "fixed": [],
  "summary": {
    "errors": 1,
    "warnings": 1,
    "style": 0,
    "fixed_count": 0
  }
}
```

### GitHub Actions — For CI Annotations

```bash
evnx validate --format github-actions
```

```
::error file=.env,line=1::Missing required variable: API_KEY
::warning file=.env,line=1::DEBUG is set to "False" (string, not boolean)
::notice file=.env,line=1::Fixed: SECRET_KEY → Generated secure secret
```

> These annotations appear inline in GitHub PR checks and commit views.

---

## 🚀 Use Cases & Examples

### 🔹 Local Development: Quick Check

```bash
# Just validate your .env
evnx validate

# With verbose output for debugging
evnx validate -v
```

**When to use**: Before running your app locally to catch config issues early.

---

### 🔹 Auto-Fix Common Issues

```bash
# Fix placeholders, booleans, and weak secrets automatically
evnx validate --fix

# Review changes first (git diff)
git diff .env
```

**What gets fixed**:
| Issue | Auto-Fix Action |
|-------|----------------|
| `API_KEY=changeme` | → `API_KEY=<64-char-hex-secret>` |
| `DEBUG=False` | → `DEBUG=false` |
| Missing `NEW_VAR` | → `NEW_VAR=your_value_here` |
| `SECRET_KEY=weak` | → `SECRET_KEY=<secure-random-key>` |

> ⚠️ **Always review `--fix` changes before committing!**

---

### 🔹 CI/CD Pipeline Integration

#### GitHub Actions

```yaml
# .github/workflows/validate.yml
name: Validate Environment

on: [push, pull_request]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install evnx
        run: cargo install evnx

      - name: Validate .env.example
        run: evnx validate --format github-actions --strict
        continue-on-error: true  # Allow warnings, fail on errors

      - name: Validate production config
        run: evnx validate --pattern .env.production --validate-formats --format json
        env:
          # Provide secrets for validation if needed
          CI: true
```

#### GitLab CI

```yaml
# .gitlab-ci.yml
validate:env:
  stage: test
  script:
    - evnx validate --format json --strict --exit-zero > validation-report.json
  artifacts:
    reports:
      dotenv: validation-report.json
```

#### Generic CI (Jenkins, CircleCI, etc.)

```bash
# Fail on errors, ignore warnings
evnx validate --strict --format json

# Parse result in script
if [ "$(jq -r '.summary.errors' validation.json)" -gt 0 ]; then
    echo "❌ Validation failed with errors"
    exit 1
fi
```

---

### 🔹 Multi-Environment Validation

```bash
# Validate development config
evnx validate --pattern .env.local

# Validate staging config with format checks
evnx validate --pattern .env.staging --validate-formats

# Validate production (strictest)
evnx validate \
  --pattern .env.production \
  --strict \
  --validate-formats \
  --format json
```

**Pro tip**: Add these as npm scripts or Makefile targets:

```makefile
# Makefile
validate-dev:
	evnx validate --pattern .env.local

validate-prod:
	evnx validate --pattern .env.production --strict --validate-formats

validate-all: validate-dev validate-prod
```

---

### 🔹 Suppress Non-Critical Warnings

```bash
# Ignore localhost warnings in local dev
evnx validate --ignore localhost_in_docker

# Ignore multiple issue types
evnx validate --ignore boolean_trap,extra_variable,localhost_in_docker

# Combine with other flags
evnx validate --strict --ignore extra_variable --format json
```

**Common ignore scenarios**:
| Scenario | Suggested `--ignore` |
|----------|---------------------|
| Local development with Docker | `localhost_in_docker` |
| Legacy code with string booleans | `boolean_trap` |
| Developer-specific env vars | `extra_variable` |
| Placeholder docs in example | `placeholder_value` |

---

### 🔹 Pre-Commit Hook Integration

```bash
# .git/hooks/pre-commit (or use husky, lint-staged, etc.)
#!/bin/bash
set -e

echo "🔍 Validating environment configuration..."

# Run validation, allow warnings but fail on errors
if ! evnx validate --format json --exit-zero | jq -e '.summary.errors == 0' > /dev/null; then
    echo "❌ Environment validation failed!"
    echo "💡 Run 'evnx validate' to see details"
    echo "💡 Or run 'evnx validate --fix' to auto-correct common issues"
    exit 1
fi

echo "✅ Environment validation passed"
```

Make it executable:
```bash
chmod +x .git/hooks/pre-commit
```

---

### 🔹 Security Audit Mode

```bash
# Strict validation focused on security issues
evnx validate \
  --strict \
  --validate-formats \
  --format json | jq '.issues | map(select(.severity == "error"))'

# Check specifically for weak secrets
evnx validate --format json | jq '.issues[] | select(.type == "weak_secret")'
```

**Security checks performed**:
- ✅ SECRET_KEY length ≥ 32 characters
- ✅ No weak patterns (`password`, `secret`, `1234`, `changeme`)
- ✅ No placeholder values in sensitive variables
- ✅ AWS/Stripe keys not set to example values

---

## 🧩 Issue Types Reference

| Issue Type | Severity | Auto-Fixable | Description |
|------------|----------|--------------|-------------|
| `missing_variable` | `error` | ✅ | Variable in `.env.example` not in `.env` |
| `extra_variable` | `warning` | ❌ | Variable in `.env` not in `.env.example` (strict) |
| `placeholder_value` | `error` | ✅ | Value looks like a placeholder (`changeme`, etc.) |
| `boolean_trap` | `warning` | ✅ | Boolean set as string `"True"` instead of `true` |
| `weak_secret` | `error` | ✅ | SECRET_KEY is too short or predictable |
| `localhost_in_docker` | `warning` | ❌ | localhost URL detected with Docker files present |
| `invalid_url` | `warning` | ❌ | URL-format variable doesn't match `https?://...` |
| `invalid_port` | `error` | ❌ | Port variable not in range 1-65535 |
| `invalid_email` | `warning` | ❌ | Email variable doesn't match standard format |

---

## 🔧 Advanced Configuration

### Custom `.env.example` Location

```bash
# Use a schema file in a different directory
evnx validate --example config/templates/.env.schema

# Validate against a remote template (after downloading)
curl -O https://example.com/.env.example
evnx validate --example .env.example
```

### Custom Output Handling

```bash
# Save JSON report to file
evnx validate --format json > validation-report.json

# Extract just errors for alerting
evnx validate --format json | jq -r '.issues[] | select(.severity == "error") | .message'

# Count issues for metrics
evnx validate --format json | jq '.summary | {errors, warnings}'
```

### Combining with Other Tools

```bash
# Validate before running app
evnx validate && cargo run

# Validate and generate docs
evnx validate --format json | jq -r '.issues[] | "- \(.variable): \(.message)"' > ENV_ISSUES.md

# Validate in Docker build
# Dockerfile
RUN evnx validate --format github-actions --exit-zero || echo "⚠️ Validation warnings (non-blocking)"
```

---

## 🚨 Troubleshooting

### "Validation failed but I don't see errors"

```bash
# Check output format
evnx validate --format pretty  # Human-readable

# Enable verbose mode
evnx validate -v

# Check exit code explicitly
evnx validate; echo "Exit code: $?"
```

### "Auto-fix didn't change my file"

```bash
# --fix only applies to auto-fixable issues
evnx validate --format json | jq '.issues[] | select(.auto_fixable == false)'

# Review which issues were fixed
evnx validate --fix --format json | jq '.fixed'

# Check file permissions
ls -la .env
```

### "JSON output has UI characters"

This indicates a bug — UI headers should only appear in `pretty` format.

```bash
# Report the issue with reproduction steps
evnx validate --format json 2>&1 | cat -A  # Show hidden chars
```

### "Port validation rejects valid port 0"

Port 0 is reserved by the OS for ephemeral assignment and is not valid for application configuration.

```bash
# Use a valid port (1-65535)
MY_PORT=8080  # ✅ Valid
MY_PORT=0     # ❌ Invalid (reserved)
```

---

## 🎯 Best Practices

### ✅ Do

```bash
# Commit .env.example, never .env
echo ".env" >> .gitignore

# Validate in CI before deployment
evnx validate --strict --format github-actions

# Use --fix for onboarding new developers
evnx validate --fix  # Then review changes

# Validate format for production configs
evnx validate --pattern .env.production --validate-formats
```

### ❌ Don't

```bash
# Don't commit .env with real secrets
git add .env  # ❌ Never do this

# Don't ignore all warnings in CI
evnx validate --ignore missing_variable,placeholder_value,weak_secret  # ❌ Defeats the purpose

# Don't use --exit-zero to hide failures
evnx validate --exit-zero && deploy  # ❌ Might deploy broken config
```

### 📁 Recommended Project Structure

```
my-project/
├── .env.example          # Template with all required vars (committed)
├── .env                  # Local secrets (gitignored)
├── .env.local            # Developer overrides (gitignored)
├── .env.production       # Production config template (committed, no secrets)
├── .env.staging          # Staging config template (committed)
├── .gitignore
│   └── .env*             # Ignore all local env files
└── .github/workflows/
    └── validate.yml      # CI validation
```

---

## 🔄 Migration Guide: Upgrading to New Validate

If you're upgrading from an older version:

### Breaking Changes
| Old Behavior | New Behavior | Migration |
|-------------|--------------|-----------|
| Boolean message mentioned Python | Language-agnostic message | Update test assertions |
| UI headers in all output formats | UI only in `pretty` format | No action needed (improvement) |
| `--fix` was unimplemented | `--fix` now works for common issues | Try `--fix` for faster onboarding |

### New Features to Explore
```bash
# Try auto-fix
evnx validate --fix

# Validate formats
evnx validate --validate-formats

# Suppress specific warnings
evnx validate --ignore boolean_trap

# Use environment patterns
evnx validate --pattern .env.production
```

---

## 📞 Getting Help

```bash
# Full help
evnx validate --help

# Verbose debugging
evnx validate -v

# Report issues
# https://github.com/your-org/evnx/issues
```

---

## 🧪 Quick Reference Card

```bash
# ─────────────────────────────────────
# Local Development
# ─────────────────────────────────────
evnx validate                          # Quick check
evnx validate --fix                    # Auto-fix issues
evnx validate --ignore localhost_in_docker  # Skip Docker warnings

# ─────────────────────────────────────
# CI/CD Integration
# ─────────────────────────────────────
evnx validate --format github-actions  # GitHub annotations
evnx validate --format json            # Machine-readable
evnx validate --strict --exit-zero     # Warnings OK, errors fail

# ─────────────────────────────────────
# Multi-Environment
# ─────────────────────────────────────
evnx validate --pattern .env.local         # Dev
evnx validate --pattern .env.staging       # Staging
evnx validate --pattern .env.production --validate-formats  # Prod

# ─────────────────────────────────────
# Security & Auditing
# ─────────────────────────────────────
evnx validate --strict --format json | jq '.issues[] | select(.severity=="error")'
evnx validate | grep -i secret    # Quick secret check

# ─────────────────────────────────────
# Output & Integration
# ─────────────────────────────────────
evnx validate --format json > report.json
evnx validate --format json | jq '.summary'
evnx validate && cargo run        # Validate before running
```

---

> 💡 **Pro Tip**: Add `evnx validate` to your `pre-commit` hook and CI pipeline to catch environment issues before they reach production!

---

*Last updated: March 2026 | evnx v0.2.1*