use crate::core::gitignore::{append_entry, check_ignored, GitignoreStatus};
use anyhow::{bail, Context, Result};
use colored::*;
use std::fs;
use std::path::Path;

/// Entries `init` adds to `.gitignore`, in this order — the negations must
/// follow the pattern they carve out of.
///
/// ⚠️ This used to be `.env`, `.env.local`, `.env.*.local` — the Next.js
/// convention, which assumes `.env.production` holds *non-secret defaults* and
/// so leaves it committable. For a secrets tool that assumption is backwards.
/// Verified before the change: a fresh `evnx init`, then `git check-ignore`:
///
/// ```text
/// .env             ignored
/// .env.local       ignored
/// .env.production  NOT IGNORED   ← and `evnx doctor` reported ✓
/// .env.staging     NOT IGNORED
/// .env.test        NOT IGNORED
/// ```
///
/// Ignoring `.env*` wholesale and exempting the template family is both safer
/// and more durable: it covers `.env.prod`, `.env.dev` and whatever else a
/// project invents, rather than only the names evnx happens to know.
use crate::core::gitignore::ENV_ENTRIES as GITIGNORE_ENTRIES;

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
    // Public signature kept: this is reachable as library API. The outcome is
    // what `init` itself needs, through `write_env_files_with_outcome`.
    write_env_files_with_outcome(output_path, example_content, template_content, opts).map(|_| ())
}

/// What a write did, so the caller can say so truthfully.
///
/// ⚠️ All three used to be `Ok(())`, and every caller printed "✓ Created
/// .env.example" and "Next steps: Edit .env" afterwards — after an abort, and
/// after the user had chosen to keep their own template, when no `.env` existed
/// (I-013, N-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// `.env.example` was written (and `.env`, if it was missing).
    Written,
    /// The user kept their existing `.env.example`; only `.gitignore` was updated.
    KeptExisting,
    /// The user cancelled; nothing was written.
    Aborted,
}

/// Stop before a confirmation prompt that cannot be shown.
///
/// ⚠️ Without a terminal dialoguer fails with "IO error: not a terminal" (twice),
/// which says nothing about what to do. `--with` and `--blueprint` reach a
/// confirmation unless `--yes` is given, so a script that forgot `--yes` got
/// exactly that (N-08, the same family as I-014). Exit code stays 2.
pub fn require_terminal(example: &str) -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        bail!(
            "there is no terminal to confirm with.\n\n\
             Add --yes to write without asking:\n  {example}"
        );
    }
    Ok(())
}

/// Print the one line every cancel path shares, and say so in the type.
pub fn aborted() -> Outcome {
    println!("{}", "Aborted. Nothing was written.".yellow());
    Outcome::Aborted
}

/// [`write_env_files`], returning what happened.
///
/// # Order: protect first, write second, talk last
///
/// ⚠️ This used to write `.env`, then **print** "Created .env from template",
/// then update `.gitignore`. When stdout was a closed pipe (`evnx init … | head`)
/// that print panicked, and `.gitignore` was never written: a `.env` was left on
/// disk that git would commit. Measured on 0.10.0 with `| head -n 3`: 4 to 33%
/// of runs, depending on timing (I-008).
///
/// Now `.gitignore` is updated before any env file exists, and nothing is
/// printed until every write has finished. The lines printed, and their order,
/// are the same as before. A failed print can only lose a message, never a rule.
pub(crate) fn write_env_files_with_outcome(
    output_path: &Path,
    example_content: &str,
    template_content: &str,
    opts: WriteOptions,
) -> Result<Outcome> {
    fs::create_dir_all(output_path)
        .with_context(|| format!("Failed to create directory: {}", output_path.display()))?;

    let example_path = output_path.join(".env.example");

    // Questions before writes: a refusal or a cancel leaves nothing behind.
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
            .interact_opt()
            .context("could not ask whether to overwrite .env.example")?;

        match overwrite {
            None => return Ok(aborted()),
            Some(false) => {
                let gitignore_line = update_gitignore(output_path)?;
                println!(
                    "{}",
                    "Kept the existing .env.example. Nothing was generated.".yellow()
                );
                if let Some(line) = gitignore_line {
                    println!("{line}");
                }
                return Ok(Outcome::KeptExisting);
            }
            Some(true) => {}
        }
    }

    // 1. Protect.
    let gitignore_line = update_gitignore(output_path)?;

    // 2. Write. ⚠️ `trim_start`, not `trim`. The trailing newline is not
    // decoration: without it the next line appended to this file fuses onto the
    // final comment —
    //
    //   # Generated by evnx v0.5.0 on 2026-09-24 02:08APPENDED_VAR=x
    //
    // — and dotenv reads the whole thing as a comment, so the variable silently
    // disappears. `evnx add` is immune because its writer trims and re-adds the
    // newline; a hand edit or another tool is not.
    fs::write(
        &example_path,
        format!("{}\n", example_content.trim_start().trim_end()),
    )
    .with_context(|| format!("Failed to write: {}", example_path.display()))?;

    // `.env` holds real values, so it is never replaced — only created.
    let env_path = output_path.join(".env");
    let created_env = !env_path.exists();
    if created_env {
        fs::write(&env_path, template_content)
            .with_context(|| format!("Failed to write: {}", env_path.display()))?;
    }

    // 3. Talk, in the order these lines have always appeared.
    if created_env {
        println!("{} Created .env from template", "✓".green());
    }
    if let Some(line) = gitignore_line {
        println!("{line}");
    }

    Ok(Outcome::Written)
}

/// Ensure `.gitignore` actually ignores the env files. Returns the line to print,
/// if anything was added; printing is the caller's job, after its writes.
///
/// ⚠️ This used to test `content.contains(".env\n")`, which a **comment** satisfies.
/// A `.gitignore` reading `# Remember: never commit .env` was therefore treated as
/// already protecting `.env`: nothing was written, nothing was printed, and
/// `git check-ignore .env` reported the file unignored — while `init`'s closing
/// banner told the user never to commit it. `check_ignored` compares whole lines
/// and skips comments, which is the distinction that was missing.
fn update_gitignore(output_path: &Path) -> Result<Option<String>> {
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

    if added.is_empty() {
        return Ok(None);
    }
    let what = added.join(", ");
    Ok(Some(if existed {
        format!("{} Added {} to .gitignore", "✓".green(), what)
    } else {
        format!("{} Created .gitignore with {}", "✓".green(), what)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn opts(yes: bool, force: bool) -> WriteOptions {
        WriteOptions { yes, force }
    }

    /// I-008: `.gitignore` is written before any env file, so a failure (or a
    /// panic on a closed stdout) partway through can never leave `.env`
    /// unprotected. Forced here by making `.env.example` impossible to write.
    #[test]
    fn gitignore_is_written_before_any_env_file() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".env.example")).unwrap(); // a directory

        let err = write_env_files(dir.path(), "A=1", "A=\n", opts(true, true))
            .expect_err("writing over a directory must fail");
        assert!(err.to_string().contains(".env.example"), "{err}");

        let gitignore = fs::read_to_string(dir.path().join(".gitignore"))
            .expect(".gitignore must already exist when the env write fails");
        assert!(
            gitignore.lines().any(|l| l.trim() == ".env*"),
            "{gitignore}"
        );
        assert!(!dir.path().join(".env").exists());
    }

    #[test]
    fn the_outcome_says_what_happened() {
        let dir = TempDir::new().unwrap();
        let outcome =
            write_env_files_with_outcome(dir.path(), "A=1", "A=\n", opts(true, false)).unwrap();
        assert_eq!(outcome, Outcome::Written);
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

        // ⚠️ Changed 2026-09-24. This asserted `"GENERATED=1"` — no trailing
        // newline — and so pinned the bug it was meant to guard against: the
        // file `init` wrote could not be appended to, because the next line
        // fused onto its last one.
        assert_eq!(fs::read_to_string(&example).unwrap(), "GENERATED=1\n");
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
                .any(|l| l.trim() == ".env*" && !l.trim_start().starts_with('#')),
            "`.env*` must be present as a real rule, not only inside a comment:\n{content}"
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
        assert_eq!(
            first.lines().filter(|l| l.trim() == ".env*").count(),
            1,
            "the rule must appear exactly once:\n{first}"
        );
    }
}
