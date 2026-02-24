# dotenv-space Architecture

## Table of Contents
1. [System Overview](#system-overview)
2. [Core Design Principles](#core-design-principles)
3. [Module Architecture](#module-architecture)
4. [Data Flow](#data-flow)
5. [Key Design Patterns](#key-design-patterns)
6. [Extension Points](#extension-points)
7. [Performance Considerations](#performance-considerations)
8. [Security Model](#security-model)

---

## System Overview

dotenv-space is a command-line tool for managing environment variables. It follows a modular architecture with clear separation of concerns.

```
┌─────────────────────────────────────────────────────────┐
│                     CLI Interface                        │
│                    (clap + main.rs)                      │
└────────────────────┬────────────────────────────────────┘
                     │
         ┌───────────┴───────────┐
         │                       │
    ┌────▼────┐            ┌─────▼─────┐
    │Commands │            │   Core    │
    │ Module  │────────────│  Module   │
    └────┬────┘            └─────┬─────┘
         │                       │
         │    ┌──────────────────┴─────┬────────────┐
         │    │                        │            │
    ┌────▼────▼───┐          ┌────────▼──┐    ┌────▼────┐
    │   Formats   │          │  Utils    │    │Templates│
    │   Module    │          │  Module   │    │ Module  │
    └─────────────┘          └───────────┘    └─────────┘
```

### Component Responsibilities

**CLI Interface:**
- Argument parsing (clap)
- Command routing
- Global flags handling
- Help text generation

**Commands Module:**
- Business logic for each command
- User interaction (dialoguer)
- Output formatting
- Error handling

**Core Module:**
- .env file parsing
- Format conversion infrastructure
- Validation logic
- Encryption/decryption (backup/restore)

**Formats Module:**
- Format-specific converters
- Export to various platforms
- Import from various sources

**Utils Module:**
- Secret pattern detection
- Entropy calculation
- Git operations
- File system utilities

**Templates Module:**
- Embedded templates for init
- Stack-specific configurations
- Service configurations

---

## Core Design Principles

### 1. Client-Side First

**Principle:** All operations happen locally. No network calls except explicit migrations.

**Implementation:**
```rust
// Good: Local operation
pub fn validate(env: &Path, example: &Path) -> Result<()> {
    let parser = Parser::default();
    let env_vars = parser.parse_file(env)?;
    let example_vars = parser.parse_file(example)?;
    // Compare locally
}

// Only when explicitly requested
pub fn migrate_to_github(vars: &HashMap, token: &str) -> Result<()> {
    // Network call happens here, but user knows
}
```

### 2. Fail Fast, Fail Clear

**Principle:** Errors should be immediate and actionable.

**Implementation:**
```rust
// Bad
Err(anyhow!("Parse error"))

// Good
Err(ParseError::InvalidFormat {
    line: 5,
    message: "Expected KEY=value, found 'KEY value'".to_string(),
})
```

### 3. Zero Configuration

**Principle:** Smart defaults, works out of the box.

**Implementation:**
```rust
// Default parser just works
let parser = Parser::default();

// But configurable when needed
let mut config = ParserConfig::default();
config.strict = true;
let parser = Parser::new(config);
```

### 4. Composable Operations

**Principle:** Commands can be chained or used independently.

**Implementation:**
```bash
# Each command is independent
dotenv-space validate
dotenv-space scan
dotenv-space convert --to json

# But they work together
dotenv-space validate && dotenv-space scan && dotenv-space convert --to json > output.json
```

### 5. Idempotent by Default

**Principle:** Running the same command twice produces the same result.

**Implementation:**
```rust
// init command checks before overwriting
if path.exists() && !force {
    return Err(anyhow!("File already exists, use --force to overwrite"));
}
```

---

## Module Architecture

### Core Module

**File:** `src/core/mod.rs`

```
core/
├── parser.rs       - .env file parser (600 lines)
├── converter.rs    - Format conversion infrastructure (200 lines)
├── validator.rs    - Validation logic (future)
├── scanner.rs      - Secret scanning (future)
└── encryptor.rs    - Encryption for backup (future)
```

**Key Types:**

```rust
// Parser
pub struct Parser {
    config: ParserConfig,
}

pub struct ParserConfig {
    pub allow_expansion: bool,
    pub strict: bool,
    pub max_expansion_depth: usize,
}

pub struct EnvFile {
    pub vars: HashMap<String, String>,
    pub source: String,
}

// Converter
pub trait Converter {
    fn convert(&self, vars: &HashMap<String, String>, options: &ConvertOptions) -> Result<String>;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
}

pub struct ConvertOptions {
    pub include_pattern: Option<String>,
    pub exclude_pattern: Option<String>,
    pub base64: bool,
    pub prefix: Option<String>,
    pub transform: Option<KeyTransform>,
}
```

### Commands Module

**File:** `src/commands/mod.rs`

```
commands/
├── init.rs         - Project initialization (300 lines)
├── validate.rs     - Environment validation (400 lines)
├── scan.rs         - Secret detection (500 lines)
├── diff.rs         - File comparison (200 lines)
├── convert.rs      - Format conversion (150 lines)
├── migrate.rs      - Migration to cloud (400 lines)
├── sync.rs         - Sync .env ↔ .example (350 lines)
├── template.rs     - Template generation (300 lines)
├── backup.rs       - Encrypted backup (200 lines)
├── restore.rs      - Restore from backup (150 lines)
└── doctor.rs       - Health check (300 lines)
```

**Command Pattern:**

```rust
pub fn run(
    // Command-specific arguments
    arg1: String,
    arg2: bool,
    // Standard flags
    verbose: bool,
) -> Result<()> {
    // 1. Validate inputs
    // 2. Parse files
    // 3. Execute logic
    // 4. Format output
    // 5. Handle errors
}
```

### Formats Module

**File:** `src/formats/mod.rs`

```
formats/
├── json.rs         - JSON output
├── yaml.rs         - YAML output
├── aws.rs          - AWS Secrets Manager
├── gcp.rs          - GCP Secret Manager
├── github.rs       - GitHub Actions
├── docker.rs       - Docker Compose
├── kubernetes.rs   - Kubernetes Secrets
├── terraform.rs    - Terraform .tfvars
└── shell.rs        - Shell export scripts
```

**Converter Pattern:**

```rust
pub struct JsonConverter;

impl Converter for JsonConverter {
    fn convert(&self, vars: &HashMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let filtered = options.filter_vars(vars);
        let transformed = /* transform keys/values */;
        let json = serde_json::to_string_pretty(&transformed)?;
        Ok(json)
    }
    
    fn name(&self) -> &str { "json" }
    fn description(&self) -> &str { "Generic JSON key-value format" }
}
```

### Utils Module

**File:** `src/utils/mod.rs`

```
utils/
├── patterns.rs     - Secret detection patterns (300 lines)
├── git.rs          - Git operations (future)
├── ui.rs           - Terminal UI helpers (future)
└── fs.rs           - File system utilities (future)
```

---

## Data Flow

### 1. Parse Flow

```
User Input (.env file)
    ↓
Parser::parse_file()
    ↓
Read file → Parse lines → Extract key=value
    ↓
Handle quotes → Escape sequences → Variable expansion
    ↓
HashMap<String, String>
```

**Code:**
```rust
fn parse_file(&self, path: &Path) -> Result<EnvFile> {
    let content = fs::read_to_string(path)?;
    let vars = self.parse_content(&content)?;
    Ok(EnvFile { vars, source: path.to_string() })
}
```

### 2. Validation Flow

```
.env + .env.example
    ↓
Parse both files
    ↓
Compare keys (missing, extra, different)
    ↓
Check values (placeholders, weak keys, traps)
    ↓
Issues[]
    ↓
Format output (pretty, JSON, GitHub Actions)
```

### 3. Conversion Flow

```
.env file
    ↓
Parse → HashMap
    ↓
Filter (include/exclude patterns)
    ↓
Transform (keys and values)
    ↓
Converter::convert()
    ↓
Format-specific output (JSON, YAML, etc.)
```

### 4. Migration Flow

```
.env file
    ↓
Parse → HashMap
    ↓
Convert to target format
    ↓
Authenticate with target service
    ↓
Upload via API
    ↓
Verify + Report
```

---

## Key Design Patterns

### 1. Trait-Based Conversion

**Pattern:** Strategy pattern via traits

```rust
pub trait Converter {
    fn convert(&self, vars: &HashMap<String, String>, options: &ConvertOptions) -> Result<String>;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
}

// Usage
let converter: Box<dyn Converter> = match format {
    "json" => Box::new(JsonConverter),
    "yaml" => Box::new(YamlConverter),
    _ => return Err(anyhow!("Unknown format")),
};

let output = converter.convert(&vars, &options)?;
```

**Why:** Easy to add new formats without modifying existing code.

### 2. Error Context Chain

**Pattern:** Context-aware errors

```rust
pub fn validate(env: &str, example: &str) -> Result<()> {
    let env_file = parser.parse_file(env)
        .context("Failed to parse .env")?;
        
    let example_file = parser.parse_file(example)
        .context("Failed to parse .env.example")?;
    
    // ...
}
```

**Why:** Users get clear error messages showing what failed and why.

### 3. Builder Pattern for Configuration

**Pattern:** Fluent configuration

```rust
let mut config = ParserConfig::default();
config.strict = true;
config.allow_expansion = false;
config.max_expansion_depth = 5;

let parser = Parser::new(config);
```

**Why:** Optional configuration with sensible defaults.

### 4. Command Pattern

**Pattern:** Each command is a module with run() function

```rust
pub mod validate {
    pub fn run(env: String, example: String, verbose: bool) -> Result<()> {
        // Implementation
    }
}

// In main.rs
match cli.command {
    Commands::Validate { env, example, verbose } => {
        commands::validate::run(env, example, verbose)?;
    }
}
```

**Why:** Clear separation, testable, composable.

### 5. Repository Pattern (Future)

**Pattern:** Abstract data access

```rust
pub trait SecretStore {
    fn get(&self, key: &str) -> Result<String>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn list(&self) -> Result<Vec<String>>;
}

pub struct GitHubSecretsStore { /* ... */ }
impl SecretStore for GitHubSecretsStore { /* ... */ }

pub struct AWSSecretsStore { /* ... */ }
impl SecretStore for AWSSecretsStore { /* ... */ }
```

**Why:** Easy to add new secret managers.

---

## Extension Points

### Adding a New Format Converter

**1. Create the converter:**
```rust
// src/formats/my_format.rs
pub struct MyFormatConverter;

impl Converter for MyFormatConverter {
    fn convert(&self, vars: &HashMap<String, String>, options: &ConvertOptions) -> Result<String> {
        // Implementation
    }
    
    fn name(&self) -> &str { "my-format" }
    fn description(&self) -> &str { "My custom format" }
}
```

**2. Register it:**
```rust
// src/formats/mod.rs
pub mod my_format;
pub use my_format::MyFormatConverter;

// src/commands/convert.rs
let converter: Box<dyn Converter> = match format.as_str() {
    // ... existing formats ...
    "my-format" => Box::new(MyFormatConverter),
    _ => return Err(anyhow!("Unknown format")),
};
```

**3. Add tests:**
```rust
#[test]
fn test_my_format_converter() {
    let mut vars = HashMap::new();
    vars.insert("KEY".to_string(), "value".to_string());
    
    let converter = MyFormatConverter;
    let result = converter.convert(&vars, &ConvertOptions::default()).unwrap();
    
    assert!(result.contains("expected output"));
}
```

### Adding a New Secret Pattern

**1. Add the pattern:**
```rust
// src/utils/patterns.rs
lazy_static! {
    pub static ref MY_SECRET: Regex = Regex::new(r"my_pattern_here").unwrap();
}
```

**2. Add detection logic:**
```rust
pub fn detect_secret(value: &str, key: &str) -> Option<(String, Confidence, Option<String>)> {
    // ... existing patterns ...
    
    if MY_SECRET.is_match(value) {
        return Some((
            "My Secret Type".to_string(),
            Confidence::High,
            Some("https://revoke-url.com".to_string()),
        ));
    }
    
    None
}
```

### Adding a New Command

**1. Create command module:**
```rust
// src/commands/my_command.rs
pub fn run(arg: String, verbose: bool) -> Result<()> {
    // Implementation
}
```

**2. Add to CLI:**
```rust
// src/main.rs
#[derive(Subcommand)]
enum Commands {
    // ... existing commands ...
    
    MyCommand {
        #[arg(short, long)]
        arg: String,
    },
}

match cli.command {
    // ... existing matches ...
    Commands::MyCommand { arg } => {
        commands::my_command::run(arg, cli.verbose)?;
    }
}
```

---

## Performance Considerations

### 1. Lazy Evaluation

**Pattern:** Only parse when needed

```rust
// Don't parse if just checking file existence
if !path.exists() {
    return Err(anyhow!("File not found"));
}

// Only parse if needed
let vars = parser.parse_file(path)?;
```

### 2. Streaming for Large Files

**Future optimization:**
```rust
// Current: Load entire file
let content = fs::read_to_string(path)?;

// Future: Stream for huge files
let file = File::open(path)?;
let reader = BufReader::new(file);
for line in reader.lines() {
    // Process line by line
}
```

### 3. Parallel Processing

**Future optimization:**
```rust
use rayon::prelude::*;

files.par_iter()
    .map(|file| scan_file(file))
    .collect()
```

### 4. Caching

**Future optimization:**
```rust
struct ParserCache {
    cache: Arc<Mutex<HashMap<PathBuf, EnvFile>>>,
}

impl ParserCache {
    fn get_or_parse(&self, path: &Path) -> Result<EnvFile> {
        if let Some(cached) = self.cache.lock().unwrap().get(path) {
            return Ok(cached.clone());
        }
        
        let parsed = self.parser.parse_file(path)?;
        self.cache.lock().unwrap().insert(path.to_owned(), parsed.clone());
        Ok(parsed)
    }
}
```

---

## Security Model

### 1. Secrets Never Leave the Machine

**Principle:** All secret detection happens locally.

```rust
// Good: Local scanning
pub fn scan(path: &Path) -> Result<Vec<Finding>> {
    let content = fs::read_to_string(path)?;
    let findings = detect_secrets(&content);
    Ok(findings)
}

// Bad: Never send secrets to external service
// pub fn scan_remote(path: &Path) -> Result<Vec<Finding>> {
//     let content = fs::read_to_string(path)?;
//     let response = reqwest::post("https://api.example.com/scan")
//         .body(content) // ❌ Sending secrets
//         .send()?;
// }
```

### 2. Encryption for Backups

**Pattern:** AES-256-GCM with Argon2 key derivation

```rust
pub fn encrypt(plaintext: &[u8], password: &str) -> Result<Vec<u8>> {
    // Derive key from password
    let salt = generate_salt();
    let key = derive_key(password, &salt)?;
    
    // Encrypt with AES-256-GCM
    let nonce = generate_nonce();
    let cipher = Aes256Gcm::new(&key);
    let ciphertext = cipher.encrypt(&nonce, plaintext)?;
    
    // Return: salt || nonce || ciphertext
    Ok([salt, nonce, ciphertext].concat())
}
```

### 3. Secure File Permissions

**Pattern:** Warn on insecure permissions

```rust
pub fn check_permissions(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)?;
    let permissions = metadata.permissions();
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = permissions.mode();
        
        if mode & 0o044 != 0 {
            eprintln!("⚠️  Warning: {} has insecure permissions", path.display());
            eprintln!("   Recommended: chmod 600 {}", path.display());
        }
    }
    
    Ok(())
}
```

### 4. No Logging of Secrets

**Pattern:** Redact secrets in logs

```rust
pub fn log_operation(operation: &str, key: &str, value: &str) {
    let redacted = if is_secret(key) {
        "***REDACTED***"
    } else {
        value
    };
    
    info!("{}: {} = {}", operation, key, redacted);
}
```

---

## Testing Architecture

### Unit Tests

**Location:** Same file as implementation

```rust
// src/core/parser.rs
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_basic() {
        let parser = Parser::default();
        let vars = parser.parse_content("KEY=value").unwrap();
        assert_eq!(vars.get("KEY"), Some(&"value".to_string()));
    }
}
```

### Integration Tests

**Location:** `tests/integration_tests.rs`

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_validate_command() {
    Command::cargo_bin("dotenv-space")
        .unwrap()
        .args(&["validate", "--format", "json"])
        .assert()
        .success();
}
```

### Benchmarks

**Location:** `benches/cli_benchmarks.rs`

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_parser(c: &mut Criterion) {
    c.bench_function("parse_medium", |b| {
        let parser = Parser::default();
        b.iter(|| parser.parse_content(MEDIUM_ENV));
    });
}

criterion_group!(benches, bench_parser);
criterion_main!(benches);
```

---

## Future Architecture Improvements

### 1. Plugin System

**Vision:** Allow third-party format converters

```rust
pub trait Plugin {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn converters(&self) -> Vec<Box<dyn Converter>>;
}

// Load plugins from ~/.dotenv-space/plugins/
```

### 2. Configuration System

**Vision:** User preferences

```rust
// .dotenv-space.toml
[defaults]
validate_strict = true
scan_ignore_placeholders = true

[formats]
default = "json"

[aliases]
gh = "github-actions"
k8s = "kubernetes"
```

### 3. Interactive TUI

**Vision:** Terminal UI for complex operations

```rust
use tui::{backend::CrosstermBackend, Terminal};

pub fn interactive_diff() -> Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    
    // Show side-by-side diff with accept/reject
}
```

### 4. Language Server Protocol

**Vision:** IDE integration

```rust
// LSP server for .env files
// - Auto-completion for known keys
// - Inline validation warnings
// - Quick fixes for common issues
```

---

## Summary

dotenv-space follows clean architecture principles:
- **Separation of concerns** - Clear module boundaries
- **Dependency inversion** - Core depends on abstractions (traits)
- **Open/closed principle** - Easy to extend (new formats), hard to break (existing code)
- **Single responsibility** - Each module has one job
- **Testability** - Pure functions, dependency injection

The architecture supports:
- Easy addition of new formats
- Easy addition of new commands
- Performance optimization (future)
- Security-first design
- Comprehensive testing

**The foundation is solid for long-term growth.**
