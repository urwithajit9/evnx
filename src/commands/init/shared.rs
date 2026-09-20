use crate::core::gitignore::{append_entry, check_ignored, GitignoreStatus};
use anyhow::{bail, Context, Result};
use colored::*;
use std::fs;
use std::path::Path;

/// Entries `init` adds to `.gitignore`.
const GITIGNORE_ENTRIES: &[&str] = &[".env", ".env.local", ".env.*.local"];

/// How to treat an existing `.env.example`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WriteOptions {
    /// Non-interactive: no prompts may be issued.
    pub yes: bool,
    /// Overwrite an existing `.env.example` without asking.
    pub force: bool,
}

/// Write `.env.example` and `.env`, then make sure `.gitignore` covers them.
///
/// # Why `.env.example` is guarded
///
/// `.env` has always been written only when absent, but `.env.example` was
/// written unconditionally — so `evnx init` silently replaced a committed,
/// hand-maintained template with generated content and reported success. That is
/// the one file in this pair that is *meant* to be in version control, which
/// makes it the one most likely to hold work worth keeping.
///
/// The rule now: an existing `.env.example` is overwritten only on an explicit
/// answer. Under `--yes` there is nobody to ask, so the command stops rather than
/// guessing — `--force` is how a script says it meant it.
pub fn write_env_files(
    output_path: &Path,
    example_content: &str,
    template_content: &str,
    opts: WriteOptions,
) -> Result<()> {
    fs::create_dir_all(output_path)
        .with_context(|| format!("Failed to create directory: {}", output_path.display()))?;

    let example_path = output_path.join(".env.example");

    if example_path.exists() && !opts.force {
        if opts.yes {
            bail!(
                "{} already exists.\n\n\
                 Refusing to overwrite it without being asked to — it is a tracked file \
                 and the contents may be hand-maintained.\n\
                 Re-run with --force to replace it, or remove it first.",
                example_path.display()
            );
        }

        let overwrite = dialoguer::Confirm::new()
            .with_prompt(format!("{} exists. Overwrite it?", example_path.display()))
            .default(false)
            .interact()
            .context("could not ask whether to overwrite .env.example")?;

        if !overwrite {
            println!("{}", "Kept the existing .env.example.".yellow());
            update_gitignore(output_path)?;
            return Ok(());
        }
    }

    fs::write(&example_path, example_content.trim())
        .with_context(|| format!("Failed to write: {}", example_path.display()))?;

    // `.env` holds real values, so it is never replaced — only created.
    let env_path = output_path.join(".env");
    if !env_path.exists() {
        fs::write(&env_path, template_content)
            .with_context(|| format!("Failed to write: {}", env_path.display()))?;
        println!("{} Created .env from template", "✓".green());
    }

    update_gitignore(output_path)?;

    Ok(())
}

/// Ensure `.gitignore` actually ignores the env files.
///
/// ⚠️ This used to test `content.contains(".env\n")`, which a **comment** satisfies.
/// A `.gitignore` reading `# Remember: never commit .env` was therefore treated as
/// already protecting `.env`: nothing was written, nothing was printed, and
/// `git check-ignore .env` reported the file unignored — while `init`'s closing
/// banner told the user never to commit it. `check_ignored` compares whole lines
/// and skips comments, which is the distinction that was missing.
fn update_gitignore(output_path: &Path) -> Result<()> {
    let gitignore_path = output_path.join(".gitignore");
    let existed = gitignore_path.exists();
    let mut added = Vec::new();

    for entry in GITIGNORE_ENTRIES {
        match check_ignored(&gitignore_path, entry)? {
            GitignoreStatus::AlreadyIgnored => {}
            GitignoreStatus::NotIgnored | GitignoreStatus::FileNotFound => {
                append_entry(&gitignore_path, entry)?;
                added.push(*entry);
            }
        }
    }

    if !added.is_empty() {
        let what = added.join(", ");
        if existed {
            println!("{} Added {} to .gitignore", "✓".green(), what);
        } else {
            println!("{} Created .gitignore with {}", "✓".green(), what);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn opts(yes: bool, force: bool) -> WriteOptions {
        WriteOptions { yes, force }
    }

    #[test]
    fn under_yes_an_existing_example_is_not_clobbered() {
        let dir = TempDir::new().unwrap();
        let example = dir.path().join(".env.example");
        fs::write(&example, "MY_CAREFULLY_WRITTEN_TEMPLATE=1\n").unwrap();

        let err = write_env_files(dir.path(), "GENERATED=1", "GENERATED=\n", opts(true, false))
            .expect_err("must refuse rather than overwrite");

        assert!(err.to_string().contains("--force"), "{err}");
        assert_eq!(
            fs::read_to_string(&example).unwrap(),
            "MY_CAREFULLY_WRITTEN_TEMPLATE=1\n",
            "the original template must survive"
        );
    }

    #[test]
    fn force_overwrites_the_example() {
        let dir = TempDir::new().unwrap();
        let example = dir.path().join(".env.example");
        fs::write(&example, "OLD=1\n").unwrap();

        write_env_files(dir.path(), "GENERATED=1", "GENERATED=\n", opts(true, true)).unwrap();

        assert_eq!(fs::read_to_string(&example).unwrap(), "GENERATED=1");
    }

    #[test]
    fn an_existing_env_is_never_replaced() {
        let dir = TempDir::new().unwrap();
        let env = dir.path().join(".env");
        fs::write(&env, "REAL_SECRET=keepme\n").unwrap();

        write_env_files(dir.path(), "GENERATED=1", "GENERATED=\n", opts(true, true)).unwrap();

        assert_eq!(fs::read_to_string(&env).unwrap(), "REAL_SECRET=keepme\n");
    }

    /// A `.gitignore` that only *mentions* `.env` in a comment protects nothing.
    #[test]
    fn a_commented_mention_does_not_count_as_ignored() {
        let dir = TempDir::new().unwrap();
        let gitignore = dir.path().join(".gitignore");
        fs::write(&gitignore, "# Remember: never commit .env\nnode_modules/\n").unwrap();

        write_env_files(dir.path(), "A=1", "A=\n", opts(true, false)).unwrap();

        let content = fs::read_to_string(&gitignore).unwrap();
        assert!(
            content
                .lines()
                .any(|l| l.trim() == ".env" && !l.trim_start().starts_with('#')),
            "`.env` must be present as a real rule, not only inside a comment:\n{content}"
        );
        assert!(
            content.contains("# Remember: never commit .env"),
            "the user's own comment must survive"
        );
    }

    #[test]
    fn gitignore_updates_are_idempotent() {
        let dir = TempDir::new().unwrap();
        write_env_files(dir.path(), "A=1", "A=\n", opts(true, false)).unwrap();
        let first = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        write_env_files(dir.path(), "A=1", "A=\n", opts(true, true)).unwrap();
        let second = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        assert_eq!(first, second, "a second run must not duplicate entries");
        assert_eq!(first.matches("\n.env\n").count().max(1), 1);
    }
}
