# Getting Started with dotenv-space

**Version:** 0.1.0  
**Last Updated:** February 2026  

Complete guide to using dotenv-space for `.env` file management, validation, and secret scanning.

## Table of Contents

1. [Installation](#installation)
2. [Quick Start](#quick-start)
3. [Core Commands](#core-commands)
4. [Common Use Cases](#common-use-cases)
5. [CI/CD Integration](#cicd-integration)
6. [Configuration](#configuration)
7. [Best Practices](#best-practices)
8. [Troubleshooting](#troubleshooting)

---

## Installation

### Prerequisites

- Linux, macOS, or WSL2 (Windows support coming soon)
- Rust 1.70+ (for building from source)

### Method 1: Install Script (Recommended)

```bash
curl -sSL https://raw.githubusercontent.com/urwithajit9/dotenv-space/main/install.sh | bash
```

Installs to `/usr/local/bin/dotenv-space`.

### Method 2: Cargo Install

```bash
# Core features only
cargo install dotenv-space

# With all features
cargo install dotenv-space --features full
```

### Method 3: Build from Source

```bash
git clone https://github.com/urwithajit9/dotenv-space.git
cd dotenv-space
cargo build --release --all-features
sudo cp target/release/dotenv-space /usr/local/bin/
```

### Verify Installation

```bash
dotenv-space --version
# Output: dotenv-space 0.1.0

dotenv-space --help
# Shows list of commands
```

---

## Quick Start

### 5-Minute Tutorial

```bash
# 1. Create a new project directory
mkdir my-app && cd my-app

# 2. Initialize with dotenv-space (interactive)
dotenv-space init

# Answer prompts:
# - Stack: python
# - Services: postgres, redis
# - Confirm: yes

# This creates:
# - .env.example (template)
# - .env (copy with placeholder values)
# - .gitignore (adds .env)

# 3. Edit .env with real values
nano .env
# Replace placeholders with actual credentials

# 4. Validate your configuration
dotenv-space validate

# Output:
# ✓ Loaded 8 variables from .env
# ✓ All required variables present
# ✓ No issues found

# 5. Scan for accidentally committed secrets
dotenv-space scan

# Output:
# ✓ Scanned 12 files
# ✓ No secrets detected

# 6. Compare files to see what's different
dotenv-space diff

# Output:
# Missing in .env:
#   - REDIS_URL
# Extra in .env:
#   - DEBUG_MODE

# 7. Convert to different format
dotenv-space convert --to json > config.json

# Done! You now have:
# - Validated configuration
# - Scanned for secrets
# - Multiple format exports
```

---

## Core Commands

### 1. `init` - Project Setup

**Purpose:** Generate `.env.example` with intelligent defaults for your stack.

#### Interactive Mode

```bash
dotenv-space init
```

**Prompts:**
1. Select your stack (Python/Node.js/Rust/Go/PHP)
2. Select services (PostgreSQL/Redis/MongoDB/etc.)
3. Confirm generation

#### Non-Interactive Mode

```bash
# Python with PostgreSQL and Redis
dotenv-space init --stack python --services postgres,redis --yes

# Node.js with MongoDB
dotenv-space init --stack nodejs --services mongodb --yes

# Custom path
dotenv-space init --path backend/.env --yes
```

#### What Gets Generated

**Example for Python + PostgreSQL:**

`.env.example`:
```bash
# Python Application Configuration
PYTHONPATH=.
DEBUG=False

# PostgreSQL Database
DATABASE_URL=postgresql://user:password@localhost:5432/dbname
SQL_ENGINE=django.db.backends.postgresql
SQL_DATABASE=mydb
SQL_USER=dbuser
SQL_PASSWORD=changeme
SQL_HOST=localhost
SQL_PORT=5432

# Security
SECRET_KEY=your-secret-key-here-minimum-50-characters

# Optional: Redis Cache
# REDIS_URL=redis://localhost:6379/0
```

`.gitignore` (appended):
```bash
# Environment variables
.env
.env.local
```

---

### 2. `validate` - Configuration Validation

**Purpose:** Check `.env` against `.env.example`, catch common mistakes.

#### Basic Validation

```bash
dotenv-space validate
```

**Output (Success):**
```
┌─ Validation Results ─────────────────────────────────┐
│                                                       │
│ ✓ Status: PASSED                                     │
│ ✓ Required variables: 8/8 present                    │
│ ✓ No issues found                                    │
│                                                       │
└───────────────────────────────────────────────────────┘
```

**Output (With Issues):**
```
┌─ Validation Results ─────────────────────────────────┐
│                                                       │
│ ✗ Status: FAILED                                     │
│ ✗ Required variables: 6/8 present                    │
│ ⚠ Issues found: 3                                    │
│                                                       │
└───────────────────────────────────────────────────────┘

❌ Missing Required Variables:
  - DATABASE_URL (line 12 in .env.example)
  - SECRET_KEY (line 18 in .env.example)

⚠️  Warnings:
  - DEBUG="False" (line 3 in .env)
    String "False" is truthy in Python!
    Suggestion: Use DEBUG=0 or DEBUG=false

ℹ️  Summary:
  Errors: 2
  Warnings: 1
  Passed: 6/8 variables
```

#### Strict Mode

```bash
dotenv-space validate --strict
```

Fails on **warnings** too, not just errors. Recommended for CI/CD.

#### Output Formats

**JSON (for parsing):**
```bash
dotenv-space validate --format json
```

```json
{
  "status": "failed",
  "required_present": 6,
  "required_total": 8,
  "issues": [
    {
      "severity": "error",
      "type": "missing_required",
      "variable": "DATABASE_URL",
      "message": "Required variable is missing",
      "location": {
        "file": ".env.example",
        "line": 12
      },
      "suggestion": "Add DATABASE_URL to your .env file"
    }
  ]
}
```

**GitHub Actions (annotations):**
```bash
dotenv-space validate --format github-actions
```

Creates annotations in GitHub Actions UI:
```
::error file=.env,line=1::Missing required variable: DATABASE_URL
::warning file=.env,line=3::DEBUG="False" is truthy in Python
```

#### What Validation Checks

✅ **Missing Required Variables**
```bash
# .env.example has:
DATABASE_URL=...

# .env doesn't have it:
# ❌ ERROR: Missing DATABASE_URL
```

✅ **Placeholder Values**
```bash
# .env has:
API_KEY=YOUR_KEY_HERE
# ❌ ERROR: Placeholder value detected
```

Common placeholders detected:
- `YOUR_KEY_HERE`
- `CHANGE_ME`
- `REPLACE_THIS`
- `EXAMPLE`
- `<insert-...>`
- `TODO`

✅ **Boolean String Trap**
```bash
# Python/Node.js:
DEBUG="False"  # ⚠️  WARNING: String is truthy!
# Should be:
DEBUG=0        # ✅ or false, False (no quotes)
```

✅ **Weak SECRET_KEY**
```bash
SECRET_KEY=123456  # ❌ ERROR: Too short (min 50 chars)
SECRET_KEY=aaaaaaaaaaaaaaaaaaaaaa  # ⚠️  WARNING: Low entropy
```

✅ **localhost in Docker**
```bash
DATABASE_URL=postgresql://localhost:5432/db
# ⚠️  WARNING: localhost won't work in Docker
# Suggestion: Use service name or host.docker.internal
```

✅ **Port Numbers**
```bash
PORT=80   # ⚠️  WARNING: Privileged port, needs root
PORT=99999  # ❌ ERROR: Invalid port (max 65535)
```

---

### 3. `scan` - Secret Detection

**Purpose:** Find accidentally committed credentials in your codebase.

#### Basic Scan

```bash
dotenv-space scan
```

Scans current directory recursively.

**Output (No Secrets):**
```
Scanning directory: .
✓ Scanned 42 files
✓ No secrets detected

Files scanned:
  - .env.example ✓
  - src/*.py ✓
  - tests/*.py ✓
```

**Output (Secrets Found):**
```
⚠️  Found 3 potential secrets:

HIGH CONFIDENCE:
├─ .env:12 - AWS Access Key
│  AKIA4OZRMFJ3EXAMPLE123
│  Pattern: AWS Access Key (AKIA...)
│  ⚠️  Revoke at: https://aws.amazon.com/security/

├─ config.py:45 - Stripe API Key
│  sk_live_51H...
│  Pattern: Stripe Live Key
│  ⚠️  Revoke at: https://dashboard.stripe.com/apikeys

MEDIUM CONFIDENCE:
└─ settings.py:12 - High-entropy string
   a8f3k2j9dks3j2kd9s3jdk29s3jdk2s9
   Entropy: 4.8 bits/char (threshold: 4.5)
   Might be a secret, please verify

Summary:
  High confidence: 2
  Medium confidence: 1
  Files scanned: 42
  
⚠️  Action required: Review and revoke exposed secrets!
```

#### Advanced Scanning

**Scan specific directory:**
```bash
dotenv-space scan --path src/
```

**Exclude files:**
```bash
dotenv-space scan --exclude "*.example" --exclude "*.sample"
```

**Scan for specific patterns:**
```bash
dotenv-space scan --pattern aws --pattern stripe
```

**Ignore obvious placeholders:**
```bash
dotenv-space scan --ignore-placeholders
```

Skips values like:
- `your-key-here`
- `change-me`
- `example-value`

**SARIF output (for GitHub Security):**
```bash
dotenv-space scan --format sarif > scan-results.sarif
```

Upload to GitHub:
```yaml
# .github/workflows/security.yml
- name: Upload SARIF
  uses: github/codeql-action/upload-sarif@v2
  with:
    sarif_file: scan-results.sarif
```

#### What Scan Detects

| Pattern | Example | Confidence |
|---------|---------|------------|
| AWS Access Key | `AKIA4OZRMFJ3EXAMPLE123` | High |
| AWS Secret Key | 40-char base64 string | Medium |
| Stripe API Key | `sk_live_...` or `sk_test_...` | High |
| GitHub Token | `ghp_...`, `gho_...`, `ghs_...` | High |
| OpenAI API Key | `sk-...` (48 chars) | High |
| Anthropic API Key | `sk-ant-api...` | High |
| Private Key | `-----BEGIN PRIVATE KEY-----` | High |
| Generic API Key | `api_key=...` (32+ chars) | Medium |
| High Entropy | Random-looking strings | Low |

---

### 4. `diff` - File Comparison

**Purpose:** See differences between `.env` and `.env.example`.

#### Basic Diff

```bash
dotenv-space diff
```

**Output:**
```
┌─ Comparing .env vs .env.example ─────────────────────┐

Missing in .env (required):
  ├─ DATABASE_URL
  ├─ REDIS_URL
  └─ SECRET_KEY

Extra in .env (not in example):
  ├─ DEBUG_MODE
  └─ LOCAL_SETTING

Different values:
  PORT: 8000 → 3000

Summary:
  Missing: 3
  Extra: 2
  Different: 1
  
└───────────────────────────────────────────────────────┘
```

#### Show Actual Values

```bash
dotenv-space diff --show-values
```

```
Missing in .env:
  DATABASE_URL = postgresql://localhost:5432/db

Extra in .env:
  DEBUG_MODE = true
  LOCAL_SETTING = /tmp/data

Different values:
  PORT: 8000 → 3000
```

⚠️  **Security:** By default, values are hidden to prevent accidental exposure.

#### Reverse Comparison

```bash
dotenv-space diff --reverse
```

Compares `.env.example` vs `.env` (swap direction).

#### JSON Output

```bash
dotenv-space diff --format json
```

```json
{
  "missing": ["DATABASE_URL", "REDIS_URL"],
  "extra": ["DEBUG_MODE"],
  "different": [
    {
      "key": "PORT",
      "env_value": "8000",
      "example_value": "3000"
    }
  ]
}
```

---

### 5. `convert` - Format Conversion

**Purpose:** Transform `.env` to 14+ different formats.

#### Basic Conversion

```bash
# JSON
dotenv-space convert --to json

# YAML
dotenv-space convert --to yaml

# Shell script
dotenv-space convert --to shell
```

#### Save to File

```bash
dotenv-space convert --to json --output config.json
```

#### Filter Variables

**Include specific variables:**
```bash
dotenv-space convert --to json --include "AWS_*"
```

Only exports variables starting with `AWS_`.

**Exclude variables:**
```bash
dotenv-space convert --to json --exclude "*_LOCAL"
```

Skips variables ending with `_LOCAL`.

#### Transform Keys

```bash
# Uppercase
dotenv-space convert --to json --transform uppercase

# Lowercase
dotenv-space convert --to json --transform lowercase

# camelCase
dotenv-space convert --to json --transform camelCase

# snake_case
dotenv-space convert --to json --transform snake_case
```

**Example:**
```bash
# Input: database_url=...
# uppercase: DATABASE_URL
# lowercase: database_url
# camelCase: databaseUrl
# snake_case: database_url
```

#### Add Prefix

```bash
dotenv-space convert --to json --prefix "APP_"
```

```json
{
  "APP_DATABASE_URL": "...",
  "APP_SECRET_KEY": "...",
  "APP_PORT": "8000"
}
```

#### Base64 Encode

```bash
dotenv-space convert --to kubernetes --base64
```

Useful for Kubernetes Secrets (must be base64-encoded).

#### All Formats

**Generic formats:**
- `json` - Simple JSON object
- `yaml` - YAML format
- `shell` - Shell export script

**Cloud providers:**
- `aws-secrets` - AWS Secrets Manager (CLI commands)
- `gcp-secrets` - GCP Secret Manager (gcloud commands)
- `azure-keyvault` - Azure Key Vault (az commands)

**CI/CD:**
- `github-actions` - GitHub Actions secrets format
- `gitlab-ci` - GitLab CI variables

**Containers:**
- `docker-compose` - Docker Compose env_file format
- `kubernetes` - Kubernetes Secret YAML

**Infrastructure:**
- `terraform` - Terraform .tfvars file

**Secret managers:**
- `doppler` - Doppler secrets JSON
- `heroku` - Heroku config vars (heroku commands)
- `vercel` - Vercel environment variables JSON
- `railway` - Railway JSON format

---

### 6. `sync` - Bidirectional Sync

**Purpose:** Keep `.env` and `.env.example` in sync.

#### Forward Sync (.env → .env.example)

**Use case:** You added variables to `.env`, now document them in `.env.example`.

```bash
dotenv-space sync --direction forward --placeholder
```

**What it does:**
1. Reads all variables from `.env`
2. Adds missing ones to `.env.example`
3. Uses placeholder values (not real secrets!)

**Example:**
```bash
# .env has:
DATABASE_URL=postgresql://prod-db:5432/app
NEW_FEATURE_FLAG=enabled

# After sync, .env.example has:
DATABASE_URL=postgresql://user:password@localhost:5432/dbname
NEW_FEATURE_FLAG=your-value-here
```

#### Reverse Sync (.env.example → .env)

**Use case:** Generate `.env` from template in CI/CD.

```bash
dotenv-space sync --direction reverse
```

**What it does:**
1. Reads template from `.env.example`
2. Creates/updates `.env`
3. Fills values from environment variables

**Example CI/CD usage:**
```bash
# GitLab CI variables:
export DATABASE_URL="postgresql://ci-db:5432/test"
export SECRET_KEY="ci-secret-key"

# Generate .env for this run
dotenv-space sync --direction reverse

# Now .env has:
# DATABASE_URL=postgresql://ci-db:5432/test
# SECRET_KEY=ci-secret-key
```

---

## Common Use Cases

### Use Case 1: New Python Project Setup

```bash
# 1. Initialize project
mkdir my-django-app && cd my-django-app
dotenv-space init --stack python --services postgres,redis

# 2. Edit .env with real values
nano .env

# 3. Validate before first run
dotenv-space validate --strict

# 4. Add to git
git add .env.example .gitignore
git commit -m "Add environment configuration"
# Note: .env is NOT committed (in .gitignore)

# 5. Other developers clone and run:
dotenv-space validate  # Shows what's missing
# Then they fill in their own .env
```

---

### Use Case 2: Pre-commit Secret Scanning

**Goal:** Prevent accidental secret commits.

**Setup:**
```bash
# Install pre-commit
pip install pre-commit

# Create .pre-commit-config.yaml
cat > .pre-commit-config.yaml << 'EOF'
repos:
  - repo: local
    hooks:
      - id: dotenv-scan
        name: Scan for secrets
        entry: dotenv-space scan --exit-zero
        language: system
        pass_filenames: false
        stages: [commit]
EOF

# Install hook
pre-commit install
```

**Now every commit:**
```bash
git add .
git commit -m "Update config"

# Pre-commit runs:
# Scanning for secrets...
# ⚠️  Found AWS Access Key in .env
# Commit blocked!
```

---

### Use Case 3: CI/CD Validation

**GitHub Actions:**

```yaml
# .github/workflows/validate.yml
name: Validate Environment

on: [push, pull_request]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install dotenv-space
        run: |
          curl -sSL https://install.dotenv.space | bash
          echo "$HOME/.local/bin" >> $GITHUB_PATH
      
      - name: Validate
        run: dotenv-space validate --strict --format github-actions
      
      - name: Scan for secrets
        run: dotenv-space scan --format sarif > scan.sarif
      
      - name: Upload SARIF
        uses: github/codeql-action/upload-sarif@v2
        if: always()
        with:
          sarif_file: scan.sarif
```

**Result:**
- ✅ Validation errors appear as annotations
- ✅ Secrets appear in Security tab
- ✅ PR blocked if validation fails

---

### Use Case 4: Docker Deployment

**Problem:** Need to pass environment variables to Docker container.

**Solution 1: Docker Compose format**

```bash
dotenv-space convert --to docker-compose > .env.docker

docker-compose --env-file .env.docker up
```

**Solution 2: Kubernetes Secret**

```bash
dotenv-space convert --to kubernetes --base64 > secret.yaml

kubectl apply -f secret.yaml
```

**Solution 3: AWS ECS Task Definition**

```bash
dotenv-space convert --to json | \
  aws ecs register-task-definition \
    --family my-app \
    --container-definitions file:///dev/stdin
```

---

### Use Case 5: Multi-Environment Management

**Setup:**
```bash
my-app/
├── .env.development
├── .env.staging
├── .env.production
└── .env.example
```

**Validate all environments:**
```bash
for env in development staging production; do
  echo "Validating $env..."
  dotenv-space validate \
    --env .env.$env \
    --example .env.example \
    --strict
done
```

**Convert for deployment:**
```bash
# Staging
dotenv-space convert \
  --env .env.staging \
  --to aws-secrets \
  --output staging-secrets.sh

# Production
dotenv-space convert \
  --env .env.production \
  --to aws-secrets \
  --output prod-secrets.sh
```

---

## CI/CD Integration

### GitHub Actions

**Complete workflow:**

```yaml
name: Environment Validation

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main]

jobs:
  validate-env:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install dotenv-space
        run: |
          curl -sSL https://raw.githubusercontent.com/urwithajit9/dotenv-space/main/install.sh | bash
          echo "$HOME/.local/bin" >> $GITHUB_PATH
      
      - name: Validate configuration
        run: |
          dotenv-space validate \
            --strict \
            --format github-actions
      
      - name: Scan for secrets
        run: |
          dotenv-space scan \
            --format sarif \
            --ignore-placeholders > scan-results.sarif
      
      - name: Upload SARIF to GitHub Security
        uses: github/codeql-action/upload-sarif@v2
        if: always()
        with:
          sarif_file: scan-results.sarif
      
      - name: Compare with example
        run: |
          dotenv-space diff --format json > diff-report.json
      
      - name: Upload diff report
        uses: actions/upload-artifact@v3
        with:
          name: env-diff-report
          path: diff-report.json
```

---

### GitLab CI

```yaml
# .gitlab-ci.yml
stages:
  - validate
  - test
  - deploy

validate-env:
  stage: validate
  image: alpine:latest
  before_script:
    - apk add --no-cache curl bash
    - curl -sSL https://install.dotenv.space | bash
  script:
    - dotenv-space validate --strict --format json
    - dotenv-space scan --format sarif > gl-sast-report.json
  artifacts:
    reports:
      sast: gl-sast-report.json
  only:
    - merge_requests
    - main

test:
  stage: test
  before_script:
    - dotenv-space sync --direction reverse
    - dotenv-space validate --strict
  script:
    - npm test

deploy-staging:
  stage: deploy
  before_script:
    - dotenv-space convert --to aws-secrets > setup-secrets.sh
  script:
    - bash setup-secrets.sh
    - ./deploy.sh staging
  only:
    - develop
```

---

### Jenkins

```groovy
pipeline {
    agent any
    
    stages {
        stage('Install dotenv-space') {
            steps {
                sh 'curl -sSL https://install.dotenv.space | bash'
            }
        }
        
        stage('Validate') {
            steps {
                sh 'dotenv-space validate --strict'
            }
        }
        
        stage('Scan') {
            steps {
                sh 'dotenv-space scan --format json > scan-results.json'
                archiveArtifacts artifacts: 'scan-results.json'
            }
        }
        
        stage('Deploy') {
            when {
                branch 'main'
            }
            steps {
                sh '''
                    dotenv-space convert --to aws-secrets | \
                    aws secretsmanager create-secret \
                      --name prod/myapp/config \
                      --secret-string file:///dev/stdin
                '''
            }
        }
    }
}
```

---

## Configuration

### Config File: `.dotenv-space.toml`

Place in project root or home directory:

```toml
[defaults]
env_file = ".env"
example_file = ".env.example"
verbose = false

[validate]
strict = true
auto_fix = false
format = "pretty"

[scan]
ignore_placeholders = true
exclude_patterns = ["*.example", "*.sample", "*.template", "test_*.env"]
format = "pretty"

[convert]
default_format = "json"
base64 = false
# prefix = "APP_"
# transform = "uppercase"

[aliases]
# Shortcuts for convert command
gh = "github-actions"
k8s = "kubernetes"
tf = "terraform"
aws = "aws-secrets"
```

**Using aliases:**
```bash
dotenv-space convert --to gh    # Same as --to github-actions
dotenv-space convert --to k8s   # Same as --to kubernetes
```

---

## Best Practices

### 1. Never Commit `.env`

**✅ Do:**
```bash
# .gitignore
.env
.env.local
.env.*.local
```

**❌ Don't:**
```bash
git add .env  # NEVER!
```

### 2. Always Commit `.env.example`

```bash
git add .env.example
git commit -m "Update environment template"
```

### 3. Use Strict Validation in CI

```yaml
# GitHub Actions
- run: dotenv-space validate --strict --format github-actions
```

### 4. Scan Before Every Commit

```bash
# .pre-commit-config.yaml
- id: dotenv-scan
  entry: dotenv-space scan --exit-zero
```

### 5. Use Descriptive Comments

```bash
# .env.example

# Database connection string
# Format: postgresql://user:password@host:port/database
# Required: Yes
DATABASE_URL=postgresql://localhost:5432/mydb

# Secret key for session signing
# Generate: python -c 'import secrets; print(secrets.token_hex(32))'
# Required: Yes
# Minimum length: 32 characters
SECRET_KEY=your-secret-key-here
```

### 6. Rotate Secrets Regularly

```bash
# Check secret age
dotenv-space scan --format json | jq '.findings[] | select(.age_days > 90)'

# Generate new secrets
python -c 'import secrets; print(secrets.token_urlsafe(32))'
```

### 7. Use Secret Managers in Production

```bash
# Don't use .env files in production
# Migrate to secret manager:

dotenv-space convert --to aws-secrets | \
  aws secretsmanager create-secret \
    --name prod/myapp/config \
    --secret-string file:///dev/stdin

# Or use migrate command (with --features migrate):
dotenv-space migrate \
  --to aws-secrets-manager \
  --secret-name prod/myapp/config
```

---

## Troubleshooting

### Problem: "Command not found: dotenv-space"

**Solution:**
```bash
# Add to PATH
export PATH="$HOME/.local/bin:$PATH"
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc

# Or reinstall
curl -sSL https://install.dotenv.space | bash
```

### Problem: "Missing required variable" but it exists

**Cause:** Variable has different name or whitespace.

**Solution:**
```bash
# Show exact names
dotenv-space diff --show-values

# Common issues:
DATABASE_URL  # ✅ Correct
DATABASE URL  # ❌ Space in name
DATABASE_URL= # ❌ Trailing space
```

### Problem: Validation passes but app crashes

**Cause:** Variable exists but has wrong format/value.

**Solution:**
Add format validation to `.env.example`:
```bash
# Add comments describing expected format
# DATABASE_URL format: postgresql://user:pass@host:port/db
DATABASE_URL=postgresql://localhost:5432/mydb

# PORT must be integer 1-65535
PORT=8000
```

Then use `dotenv-space validate --strict`.

### Problem: "Permission denied" when running

**Solution:**
```bash
chmod +x ~/.local/bin/dotenv-space
```

### Problem: Too many warnings

**Solution:**
Use config file to customize:

```toml
# .dotenv-space.toml
[scan]
ignore_placeholders = true
exclude_patterns = ["*.example", "*.test"]
```

---

## Next Steps

- **[Use Cases](./USE_CASES.md)** - Real-world scenarios
- **[CI/CD Guide](./CICD_GUIDE.md)** - Detailed CI/CD integration
- **[Architecture](../ARCHITECTURE.md)** - How it works internally
- **[Contributing](../CONTRIBUTING.md)** - Help improve dotenv-space

---

## Get Help

- 🐛 [Report a bug](https://github.com/urwithajit9/dotenv-space/issues)
- 💡 [Request a feature](https://github.com/urwithajit9/dotenv-space/issues)
- 💬 [Ask a question](https://github.com/urwithajit9/dotenv-space/discussions)

---

**Happy environment managing! 🚀**