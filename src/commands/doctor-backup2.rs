//! Doctor command - diagnose environment setup issues
//!
//! Validates common configuration patterns:
//! - `.env` file existence, permissions, and gitignore status
//! - `.env.example` tracking in Git
//! - Project type detection (Python, Node.js, Rust, Go, etc.)
//! - Docker configuration presence
//! - Basic `.env` syntax validation
//!
//! # Usage
//! ```bash
//! evnx doctor                    # Check current directory
//! evnx doctor --path ./my-app   # Check specific project
//! evnx doctor --verbose         # Show detailed diagnostics
//! ```

use anyhow::{Context, Result};
use colored::*;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run the doctor diagnostic checks
///
/// # Arguments
/// * `path` - Project directory to analyze (defaults to "." from CLI)
/// * `verbose` - Enable detailed output when true
///
/// # Returns
/// * `Ok(())` on success, `Err` on IO or parsing failures
pub fn run(path: String, verbose: bool) -> Result<()> {
    let project_root = PathBuf::from(&path);

    // Print header
    println!(
        "\n{}",
        "┌─ Diagnosing environment setup ──────────────────────┐".cyan()
    );
    if verbose {
        println!("{}  Project path: {}", "│".cyan(), project_root.display());
    }
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    let mut issues = 0usize;
    let mut warnings = 0usize;

    // ─────────────────────────────────────────────────────────
    // Check .env file
    // ─────────────────────────────────────────────────────────
    println!("{}", "Checking .env file...".bold());
    let env_path = project_root.join(".env");

    if env_path.exists() {
        println!("  {} File exists at .env", "✓".green());

        // Check permissions (Unix only, with graceful Windows fallback)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            match fs::metadata(&env_path) {
                Ok(metadata) => {
                    let mode = metadata.permissions().mode() & 0o777;
                    let is_secure = mode == 0o600 || mode == 0o400;

                    if is_secure {
                        if verbose {
                            println!("  {} File has secure permissions ({:o})", "✓".green(), mode);
                        }
                    } else {
                        println!(
                            "  {} File has {:o} permissions (recommended: 600)",
                            "⚠️".yellow(),
                            mode
                        );
                        warnings += 1;
                    }
                }
                Err(e) if verbose => {
                    println!("  {} Could not read permissions: {}", "ℹ️".cyan(), e);
                }
                Err(_) => {}
            }
        }

        #[cfg(not(unix))]
        if verbose {
            println!("  {} Permission checks skipped (Windows)", "ℹ️".cyan());
        }

        // Check if in .gitignore (using git CLI fallback to string search)
        let gitignored = is_gitignored(&project_root, ".env")
            .unwrap_or_else(|_| fallback_gitignore_check(&project_root, ".env"));

        if gitignored {
            if verbose {
                println!("  {} File is properly ignored by Git", "✓".green());
            }
        } else {
            println!("  {} File is NOT in .gitignore (security risk)", "✗".red());
            issues += 1;
        }

        // Validate .env syntax (basic KEY=VALUE parsing)
        match validate_env_syntax(&env_path) {
            Ok(_) if verbose => {
                println!("  {} .env syntax is valid", "✓".green());
            }
            Ok(_) => {}
            Err(e) => {
                println!("  {} Syntax issue: {}", "⚠️".yellow(), e);
                warnings += 1;
            }
        }
    } else {
        println!("  {} File does not exist", "✗".red());
        if verbose {
            println!("    Hint: Create from .env.example or run `evnx init`");
        }
        issues += 1;
    }

    // ─────────────────────────────────────────────────────────
    // Check .env.example
    // ─────────────────────────────────────────────────────────
    println!("\n{}", "Checking .env.example...".bold());
    let example_path = project_root.join(".env.example");

    if example_path.exists() {
        println!("  {} File exists", "✓".green());

        // Check if tracked in Git (with graceful fallback)
        let is_tracked = Command::new("git")
            .args([
                "-C",
                project_root.to_string_lossy().as_ref(),
                "ls-files",
                "--error-unmatch",
                ".env.example",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if is_tracked {
            if verbose {
                println!("  {} File is tracked in Git", "✓".green());
            }
        } else {
            println!("  {} File is NOT tracked in Git (recommended)", "✗".red());
            if verbose {
                println!("    Hint: git add .env.example && git commit -m 'Add env template'");
            }
            warnings += 1;
        }
    } else {
        println!("  {} File does not exist", "⚠️".yellow());
        if verbose {
            println!("    Hint: Create a template with `evnx init` or manually");
        }
        warnings += 1;
    }

    // ─────────────────────────────────────────────────────────
    // Check project structure (expanded detection)
    // ─────────────────────────────────────────────────────────
    println!("\n{}", "Checking project structure...".bold());

    let detected = detect_project_type(&project_root);

    // Use if-else chain for string value matching (not enum variants)
    if let Some(project_type) = detected.as_deref() {
        if project_type.starts_with("Python") {
            println!(
                "  {} Detected Python project ({})",
                "✓".green(),
                project_type
            );
            if project_type.contains("requirements.txt") {
                check_requirements_txt(&project_root, verbose, &mut warnings);
            } else {
                check_python_deps(&project_root, verbose, &mut warnings);
            }
        } else if project_type == "Node.js" {
            println!("  {} Detected Node.js project (package.json)", "✓".green());
            if verbose {
                check_node_deps(&project_root, &mut warnings);
            }
        } else if project_type == "Rust" {
            println!("  {} Detected Rust project (Cargo.toml)", "✓".green());
            if verbose {
                check_rust_deps(&project_root, &mut warnings);
            }
        } else if project_type == "Go" {
            println!("  {} Detected Go project (go.mod)", "✓".green());
        } else if project_type == "PHP" {
            println!("  {} Detected PHP project (composer.json)", "✓".green());
        } else {
            // Fallback for any other detected type
            println!("  {} Detected {} project", "✓".green(), project_type);
        }
    } else {
        println!("  {} No recognized project configuration", "ℹ️".cyan());
        if verbose {
            println!("    Supported: requirements.txt, pyproject.toml, Pipfile, poetry.lock,");
            println!("               package.json, Cargo.toml, go.mod, composer.json");
        }
    }

    // ─────────────────────────────────────────────────────────
    // Check Docker configuration
    // ─────────────────────────────────────────────────────────
    let docker_files = [
        "docker-compose.yml",
        "docker-compose.yaml",
        "Dockerfile",
        "Containerfile",
        ".dockerignore",
    ];

    let found_files: Vec<&str> = docker_files
        .iter()
        .copied() // Convert &&str → &str so join() works
        .filter(|f| project_root.join(f).exists())
        .collect();

    if !found_files.is_empty() {
        println!("\n  {} Docker configuration detected", "ℹ️".cyan());
        if verbose {
            println!("    Found: {}", found_files.join(", "));
        }
    }

    // ─────────────────────────────────────────────────────────
    // Summary
    // ─────────────────────────────────────────────────────────
    println!("\n{}", "Summary:".bold());

    if issues == 0 && warnings == 0 {
        println!("  {} 0 issues found", "✓".green());
        if verbose {
            println!("  {} All checks passed", "✓".green());
        }
        println!("\nOverall health: {} Excellent", "✓".green());
    } else {
        if issues > 0 {
            println!(
                "  🚨 {} critical issue{}",
                issues,
                if issues > 1 { "s" } else { "" }
            );
        }
        if warnings > 0 {
            println!(
                "  ⚠️  {} warning{}",
                warnings,
                if warnings > 1 { "s" } else { "" }
            );
        }
        println!("\nOverall health: {} Needs attention", "⚠️".yellow());
    }

    // Recommendations (only if issues exist)
    if issues > 0 || (verbose && warnings > 0) {
        println!("\n{}", "Recommendations:".bold());
        let mut rec_num = 1;

        if !is_gitignored(&project_root, ".env").unwrap_or(false) {
            println!(
                "  {}. Add .env to .gitignore: echo '.env' >> .gitignore",
                rec_num
            );
            rec_num += 1;
        }
        if !example_path.exists()
            || !Command::new("git")
                .args([
                    "-C",
                    project_root.to_string_lossy().as_ref(),
                    "ls-files",
                    "--error-unmatch",
                    ".env.example",
                ])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        {
            println!(
                "  {}. Track .env.example in Git: git add .env.example",
                rec_num
            );
            rec_num += 1;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::metadata(&env_path) {
                let mode = meta.permissions().mode() & 0o777;
                if mode != 0o600 && mode != 0o400 {
                    println!("  {}. Secure .env permissions: chmod 600 .env", rec_num);
                    rec_num += 1;
                }
            }
        }

        if rec_num == 1 {
            println!("  • Review warnings above for optional improvements");
        }
    }

    // Exit with error code for CI/CD integration (issues = failure)
    if issues > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

/// Check if a file is ignored by Git (using git CLI)
fn is_gitignored(project_root: &Path, filename: &str) -> Result<bool> {
    let output = Command::new("git")
        .args([
            "-C",
            project_root.to_string_lossy().as_ref(),
            "check-ignore",
            filename,
        ])
        .output()
        .context("Failed to execute git check-ignore")?;

    Ok(output.status.success())
}

/// Fallback: simple string search in .gitignore (less accurate)
fn fallback_gitignore_check(project_root: &Path, filename: &str) -> bool {
    let gitignore_path = project_root.join(".gitignore");
    if !gitignore_path.exists() {
        return false;
    }

    fs::read_to_string(&gitignore_path)
        .ok()
        .map(|content| {
            content
                .lines()
                .map(str::trim)
                .any(|line| line == filename || line.starts_with(&format!("/{}", filename)))
        })
        .unwrap_or(false)
}

/// Validate basic .env syntax: KEY=VALUE format
///
/// Accepts: comments (#), empty lines, and KEY=VALUE pairs
/// Rejects: lines without = (except comments/empty), invalid key names
fn validate_env_syntax(path: &Path) -> Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    // Regex for valid .env line: empty, comment, or KEY=VALUE
    // KEY must start with letter/underscore, contain only alnum/underscore
    let line_re = Regex::new(r"^(?:\s*$|\s*#.*|\s*[A-Za-z_][A-Za-z0-9_]*\s*=.*)$")
        .context("Invalid regex pattern")?;

    for (idx, line) in content.lines().enumerate() {
        if !line_re.is_match(line) {
            let snippet = line.chars().take(50).collect::<String>();
            return Err(anyhow::anyhow!(
                "Line {}: invalid syntax '{}{}'",
                idx + 1,
                snippet,
                if line.len() > 50 { "..." } else { "" }
            ));
        }
    }
    Ok(())
}

/// Detect project type based on configuration files
fn detect_project_type(project_root: &Path) -> Option<String> {
    // Python ecosystem (check in priority order)
    if project_root.join("pyproject.toml").exists() {
        // Heuristic: check for poetry vs generic
        if let Ok(content) = fs::read_to_string(project_root.join("pyproject.toml")) {
            if content.contains("[tool.poetry]") {
                return Some("Python (Poetry)".to_string());
            }
        }
        return Some("Python (pyproject.toml)".to_string());
    }
    if project_root.join("poetry.lock").exists() {
        return Some("Python (Poetry)".to_string());
    }
    if project_root.join("Pipfile").exists() {
        return Some("Python (Pipenv)".to_string());
    }
    if project_root.join("requirements.txt").exists() {
        return Some("Python (requirements.txt)".to_string());
    }

    // Other languages
    if project_root.join("package.json").exists() {
        return Some("Node.js".to_string());
    }
    if project_root.join("Cargo.toml").exists() {
        return Some("Rust".to_string());
    }
    if project_root.join("go.mod").exists() {
        return Some("Go".to_string());
    }
    if project_root.join("composer.json").exists() {
        return Some("PHP".to_string());
    }

    None
}

/// Check Python dependencies for dotenv support
fn check_requirements_txt(project_root: &Path, verbose: bool, warnings: &mut usize) {
    let req_path = project_root.join("requirements.txt");
    if let Ok(content) = fs::read_to_string(&req_path) {
        let has_dotenv = content
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .any(|line| {
                // Match python-dotenv, dotenv, or python_dotenv (exact package name)
                line.starts_with("python-dotenv")
                    || line.starts_with("dotenv") && !line.contains("django-dotenv")
            });

        if has_dotenv {
            if verbose {
                println!("  {} python-dotenv dependency found", "✓".green());
            }
        } else {
            println!("  {} python-dotenv not in requirements.txt", "⚠️".yellow());
            *warnings += 1;
        }
    }
}

/// Check Python pyproject.toml for dotenv support
fn check_python_deps(project_root: &Path, verbose: bool, warnings: &mut usize) {
    let pyproject_path = project_root.join("pyproject.toml");
    if let Ok(content) = fs::read_to_string(&pyproject_path) {
        // Simple heuristic checks
        let has_dotenv = content.contains("python-dotenv")
            || content.contains("pydantic-settings")  // Modern alternative
            || content.contains("dynaconf");

        if has_dotenv {
            if verbose {
                println!("  {} Environment variable handling configured", "✓".green());
            }
        } else if verbose {
            println!(
                "  {} Consider adding python-dotenv or pydantic-settings",
                "ℹ️".cyan()
            );
        }
    }
}

/// Check Node.js dependencies (optional, verbose only)
fn check_node_deps(project_root: &Path, warnings: &mut usize) {
    let pkg_path = project_root.join("package.json");
    if let Ok(content) = fs::read_to_string(&pkg_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            let deps = json
                .get("dependencies")
                .or_else(|| json.get("devDependencies"));
            if let Some(deps) = deps.and_then(|d| d.as_object()) {
                if !deps.contains_key("dotenv") && !deps.contains_key("dotenv-cli") {
                    // Only warn in verbose mode to avoid noise
                    eprintln!("  ℹ️  Consider adding 'dotenv' package for Node.js");
                }
            }
        }
    }
}

/// Check Rust dependencies (optional, verbose only)
fn check_rust_deps(project_root: &Path, warnings: &mut usize) {
    let cargo_path = project_root.join("Cargo.toml");
    if let Ok(content) = fs::read_to_string(&cargo_path) {
        if !content.contains("dotenvy") && !content.contains("dotenv") {
            eprintln!("  ℹ️  Consider adding 'dotenvy' crate for Rust");
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_validate_env_syntax_valid() {
        let dir = TempDir::new().unwrap();
        let env_path = dir.path().join(".env");

        fs::write(
            &env_path,
            r#"
# Database config
DB_HOST=localhost
DB_PORT=5432

# API keys (placeholders)
API_KEY=your_key_here
EMPTY_VAR=
"#,
        )
        .unwrap();

        assert!(validate_env_syntax(&env_path).is_ok());
    }

    #[test]
    fn test_validate_env_syntax_invalid() {
        let dir = TempDir::new().unwrap();
        let env_path = dir.path().join(".env");

        // Invalid: line without = that's not a comment or empty
        fs::write(&env_path, "INVALID LINE WITHOUT EQUALS\n").unwrap();

        let result = validate_env_syntax(&env_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Line 1"));
    }

    #[test]
    fn test_detect_project_type_python() {
        let dir = TempDir::new().unwrap();

        // Test requirements.txt detection
        fs::write(dir.path().join("requirements.txt"), "flask\n").unwrap();
        assert_eq!(
            detect_project_type(dir.path()),
            Some("Python (requirements.txt)".to_string())
        );

        // Test pyproject.toml detection (generic)
        fs::remove_file(dir.path().join("requirements.txt")).ok();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"test\"\n",
        )
        .unwrap();
        assert_eq!(
            detect_project_type(dir.path()),
            Some("Python (pyproject.toml)".to_string())
        );
    }

    #[test]
    fn test_fallback_gitignore_check() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "# Env files\n.env\n*.log\n").unwrap();

        assert!(fallback_gitignore_check(dir.path(), ".env"));
        assert!(!fallback_gitignore_check(dir.path(), ".env.local"));
    }
}
