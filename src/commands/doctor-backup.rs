/// Doctor command - diagnose environment setup issues
use anyhow::Result;
use colored::*;
use std::path::Path;

pub fn run(_path: String, _verbose: bool) -> Result<()> {
    println!(
        "\n{}",
        "┌─ Diagnosing environment setup ──────────────────────┐".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    let mut issues = 0;
    let mut warnings = 0;

    // Check .env file
    println!("{}", "Checking .env file...".bold());
    if Path::new(".env").exists() {
        println!("  {} File exists at .env", "✓".green());

        // Check permissions (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(".env")?;
            let mode = metadata.permissions().mode();
            if mode & 0o044 != 0 {
                println!(
                    "  {} File has {:o} permissions (should be 600 for security)",
                    "⚠️".yellow(),
                    mode & 0o777
                );
                warnings += 1;
            } else {
                println!("  {} File has secure permissions (600)", "✓".green());
            }
        }

        // Check if in .gitignore
        if Path::new(".gitignore").exists() {
            let gitignore = std::fs::read_to_string(".gitignore")?;
            if gitignore.contains(".env") {
                println!("  {} File is in .gitignore", "✓".green());
            } else {
                println!("  {} File is NOT in .gitignore (should be)", "✗".red());
                issues += 1;
            }
        }
    } else {
        println!("  {} File does not exist", "✗".red());
        issues += 1;
    }

    // Check .env.example
    println!("\n{}", "Checking .env.example...".bold());
    if Path::new(".env.example").exists() {
        println!("  {} File exists", "✓".green());

        // Check if tracked in Git
        if std::process::Command::new("git")
            .args(["ls-files", "--error-unmatch", ".env.example"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            println!("  {} File is tracked in Git", "✓".green());
        } else {
            println!("  {} File is NOT tracked in Git (should be)", "✗".red());
            issues += 1;
        }
    }

    // Check project structure
    println!("\n{}", "Checking project structure...".bold());
    if Path::new("requirements.txt").exists() {
        println!(
            "  {} Detected Python project (requirements.txt)",
            "✓".green()
        );

        // Check for python-dotenv
        let req = std::fs::read_to_string("requirements.txt")?;
        if req.contains("python-dotenv") {
            println!("  {} python-dotenv is installed", "✓".green());
        } else {
            println!("  {} python-dotenv not in requirements.txt", "⚠️".yellow());
            warnings += 1;
        }
    } else if Path::new("package.json").exists() {
        println!("  {} Detected Node.js project (package.json)", "✓".green());
    } else if Path::new("Cargo.toml").exists() {
        println!("  {} Detected Rust project (Cargo.toml)", "✓".green());
    }

    // Check Docker
    if Path::new("docker-compose.yml").exists() || Path::new("Dockerfile").exists() {
        println!("  {} Docker files detected", "ℹ️".cyan());
    }

    // Summary
    println!("\n{}", "Summary:".bold());
    if issues == 0 && warnings == 0 {
        println!("  {} 0 issues found", "✓".green());
        println!("\nOverall health: {} Excellent", "✓".green());
    } else {
        if issues > 0 {
            println!("  🚨 {} critical issues", issues);
        }
        if warnings > 0 {
            println!("  ⚠️  {} warnings", warnings);
        }
        println!("\nOverall health: {} Needs attention", "⚠️".yellow());
    }

    if issues > 0 {
        println!("\n{}", "Recommendations:".bold());
        println!("  1. Add .env to .gitignore if not present");
        println!("  2. Track .env.example in Git");
        println!("  3. Set secure permissions: chmod 600 .env");
    }

    Ok(())
}
