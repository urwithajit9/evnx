use anyhow::{Context, Result};
use colored::*;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use std::fs;
use std::path::Path;

/// Interactive project setup — generates .env.example
pub fn run(
    stack: Option<String>,
    services: Option<String>,
    path: String,
    yes: bool,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("{}", "Running init in verbose mode".dimmed());
    }

    println!(
        "\n{}",
        "┌─ evnx init ─────────────────────────────────┐".cyan()
    );
    println!(
        "{}",
        "│ Let's set up environment variables for your project │".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    // Determine stack (interactive or from flag)
    let selected_stack = if let Some(s) = stack {
        s
    } else if yes {
        "python".to_string()
    } else {
        let stacks = vec![
            "Python (Django/FastAPI)",
            "Node.js (Next.js/Express)",
            "Rust",
            "Go",
            "PHP (Laravel)",
            "Other",
        ];
        let selection = Select::new()
            .with_prompt("What's your primary stack?")
            .items(&stacks)
            .default(0)
            .interact()?;

        match selection {
            0 => "python",
            1 => "nodejs",
            2 => "rust",
            3 => "go",
            4 => "php",
            _ => "other",
        }
        .to_string()
    };

    // Determine services (interactive or from flag)
    let selected_services = if let Some(s) = services {
        s.split(',').map(|s| s.trim().to_string()).collect()
    } else if yes {
        vec!["postgresql".to_string(), "redis".to_string()]
    } else {
        let services = vec![
            "PostgreSQL",
            "Redis",
            "MongoDB",
            "AWS S3",
            "Stripe",
            "Twilio",
            "SendGrid",
            "Sentry",
            "OpenAI",
        ];
        let selections = MultiSelect::new()
            .with_prompt("Which services will you use? (Space to select, Enter to confirm)")
            .items(&services)
            .interact()?;

        selections
            .iter()
            .map(|&i| services[i].to_lowercase().replace(" ", "_"))
            .collect()
    };

    // Determine output path
    let output_path = if yes {
        path.clone()
    } else {
        Input::new()
            .with_prompt("Where should I create .env.example?")
            .default(path)
            .interact_text()?
    };

    // Generate .env.example content
    let env_example_content = generate_env_example(&selected_stack, &selected_services);
    let env_example_path = Path::new(&output_path).join(".env.example");

    // Check if file already exists
    if env_example_path.exists() && !yes {
        let overwrite = Confirm::new()
            .with_prompt(format!(
                "{} already exists. Overwrite?",
                env_example_path.display()
            ))
            .default(false)
            .interact()?;

        if !overwrite {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // Write .env.example
    fs::write(&env_example_path, env_example_content.trim())
        .context("Failed to write .env.example")?;

    let var_count = env_example_content
        .lines()
        .filter(|l| l.contains('='))
        .count();
    println!(
        "{} Created .env.example with {} variables",
        "✓".green(),
        var_count
    );

    // Create .env from template
    let env_path = Path::new(&output_path).join(".env");
    if !env_path.exists() {
        let mut env_content =
            "# TODO: Replace all placeholder values with real credentials\n\n".to_string();
        env_content.push_str(&env_example_content);
        fs::write(&env_path, env_content).context("Failed to write .env")?;
        println!(
            "{} Created .env from template (fill in real values)",
            "✓".green()
        );
    }

    // Update .gitignore
    let gitignore_path = Path::new(&output_path).join(".gitignore");
    if gitignore_path.exists() {
        let gitignore_content = fs::read_to_string(&gitignore_path)?;
        if !gitignore_content.contains(".env") {
            let mut updated = gitignore_content;
            updated.push_str("\n# Environment files\n.env\n.env.local\n.env.*.local\n");
            fs::write(&gitignore_path, updated)?;
            println!("{} Added .env to .gitignore", "✓".green());
        }
    } else {
        fs::write(
            &gitignore_path,
            "# Environment files\n.env\n.env.local\n.env.*.local\n",
        )?;
        println!("{} Created .gitignore", "✓".green());
    }

    // Print next steps
    println!("\n{}", "Next steps:".bold());
    println!("  1. Edit .env and replace placeholder values");
    println!("  2. Never commit .env to Git");
    println!("  3. Run 'evnx validate' to check for issues");

    Ok(())
}

fn generate_env_example(stack: &str, services: &[String]) -> String {
    let mut content = String::new();

    // Stack-specific variables
    match stack {
        "python" => {
            content.push_str("# Django/FastAPI\n");
            content.push_str("SECRET_KEY=generate-with-openssl-rand-hex-32\n");
            content.push_str("DEBUG=True\n");
            content.push_str("ALLOWED_HOSTS=localhost,127.0.0.1\n\n");
        }
        "nodejs" => {
            content.push_str("# Node.js\n");
            content.push_str("NODE_ENV=development\n");
            content.push_str("PORT=3000\n\n");
        }
        "rust" => {
            content.push_str("# Rust\n");
            content.push_str("APP__ENVIRONMENT=development\n");
            content.push_str("APP__PORT=8080\n\n");
        }
        _ => {
            content.push_str("# Application\n");
            content.push_str("APP_ENV=development\n\n");
        }
    }

    // Service-specific variables
    for service in services {
        match service.as_str() {
            "postgresql" => {
                content.push_str("# Database\n");
                content
                    .push_str("DATABASE_URL=postgresql://user:password@localhost:5432/dbname\n\n");
            }
            "redis" => {
                content.push_str("# Cache\n");
                content.push_str("REDIS_URL=redis://localhost:6379/0\n\n");
            }
            "mongodb" => {
                content.push_str("# Database\n");
                content.push_str("MONGODB_URI=mongodb://localhost:27017/mydb\n\n");
            }
            "aws_s3" => {
                content.push_str("# AWS S3\n");
                content.push_str("AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n");
                content
                    .push_str("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\n");
                content.push_str("AWS_STORAGE_BUCKET_NAME=your-bucket-name\n");
                content.push_str("AWS_REGION=us-east-1\n\n");
            }
            "stripe" => {
                content.push_str("# Stripe\n");
                content.push_str("STRIPE_SECRET_KEY=sk_test_YOUR_KEY_HERE\n");
                content.push_str("STRIPE_PUBLISHABLE_KEY=pk_test_YOUR_KEY_HERE\n");
                content.push_str("STRIPE_WEBHOOK_SECRET=whsec_YOUR_WEBHOOK_SECRET\n\n");
            }
            "sendgrid" => {
                content.push_str("# SendGrid\n");
                content.push_str("SENDGRID_API_KEY=SG.YOUR_API_KEY_HERE\n\n");
            }
            "sentry" => {
                content.push_str("# Sentry\n");
                content.push_str("SENTRY_DSN=https://YOUR_SENTRY_DSN@sentry.io/PROJECT_ID\n\n");
            }
            "openai" => {
                content.push_str("# OpenAI\n");
                content.push_str("OPENAI_API_KEY=sk-proj-YOUR_KEY_HERE\n\n");
            }
            _ => {}
        }
    }

    // Add generation footer
    content.push_str(&format!(
        "# Generated by evnx v{} on {}\n",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().format("%Y-%m-%d")
    ));

    content
}
