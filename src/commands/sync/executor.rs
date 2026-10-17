//! Execution logic for sync operations: forward, reverse, previews, and file writes.

use anyhow::{Context, Result};
use colored::*;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use indexmap::IndexMap;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::{NamingPolicy, SyncDirection};
use crate::core::{envfile, Parser};
use crate::docs;
use crate::utils::ui;

use super::models::{PlaceholderConfig, SyncAction, SyncPreview, VarChange};
use super::placeholder::{generate_placeholder, is_generated_value};
use super::security::{check_env_permissions, log_security_event, validate_var_name};

/// Write the env file sync manages: atomic, new files 0600, an existing file
/// keeps its mode, a symlink is written through (DEC-S6, DEC-S7, core::safe_write).
///
/// ⚠️ This used to force 0600 on every file whose name started with `.env` and
/// did not contain "example" — including a `.env` the user had set to 0640 —
/// and the template came out 0600 only because the temp file did (I-123).
fn write_env(path: &Path, content: &str) -> Result<()> {
    let written = crate::core::safe_write::write_private(path, content.as_bytes())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    crate::core::safe_write::report(&written);
    Ok(())
}

/// Write the template: as [`write_env`], but a new file is 0644 — it is meant
/// to be committed and read by the team.
///
/// ⚠️ Renaming a temp file over `.env.example` replaced a symlink with a plain
/// file, breaking monorepo and stow setups that link the template (I-122).
fn write_template(path: &Path, content: &str) -> Result<()> {
    let written = crate::core::safe_write::write_shared(path, content.as_bytes())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    crate::core::safe_write::report(&written);
    Ok(())
}

/// The two files a sync run operates on.
///
/// ⚠️ These were 37 hardcoded `".env"` / `".env.example"` literals spread across
/// the module, which is why `sync` was the only command with **no** file flags
/// at all — `.env.production` was simply unreachable, and there was nowhere to
/// put a path even if you wanted one.
#[derive(Debug, Clone)]
pub(super) struct SyncPaths {
    pub env: PathBuf,
    pub example: PathBuf,
}

impl SyncPaths {
    // `pub(super)` rather than private: the JSON `--check` path in `mod.rs`
    // names both files in its report, and duplicating two one-line formatters
    // to keep them private would be the wrong trade.
    pub(super) fn env_str(&self) -> String {
        self.env.display().to_string()
    }
    pub(super) fn example_str(&self) -> String {
        self.example.display().to_string()
    }
}

/// Internal context struct to reduce parameter passing
pub(super) struct SyncCtx {
    pub paths: SyncPaths,
    pub direction: SyncDirection,
    pub placeholder: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub force: bool,
    pub template_config: Option<PathBuf>,
    pub naming_policy: NamingPolicy,
    /// The project's declared contract, when it has one.
    ///
    /// ⚠️ Used by the reverse direction only. Forward sync makes the template
    /// match `.env`, and a spec has no values to contribute to that.
    pub spec: crate::core::spec::Spec,
}

/// Main execution entry point (called from mod.rs).
///
/// Returns whether the two files are **out of step** — `true` when variables are
/// missing from the target, `false` when nothing needs doing. `--check` turns
/// that into an exit code; every other mode ignores it.
pub fn execute(ctx: SyncCtx) -> Result<bool> {
    if ctx.verbose {
        println!(
            "{}",
            format!(
                "Running sync in verbose mode [direction={:?}, dry_run={}, force={}]",
                ctx.direction, ctx.dry_run, ctx.force
            )
            .dimmed()
        );
    }

    let config = ctx
        .template_config
        .as_ref()
        .map(PlaceholderConfig::from_path)
        .transpose()?
        .unwrap_or_default();

    // ⚠️ The title is the command, not a sentence about it. `sync` was the one
    // command whose header did not start with `evnx …`, so in a CI log among
    // other tools' output it read as a stray line rather than as evnx speaking.
    ui::print_header(
        "evnx sync",
        Some(&format!(
            "{} ↔ {}  ({})",
            ctx.paths.env_str(),
            ctx.paths.example_str(),
            ctx.direction
        )),
    );

    check_env_permissions(&ctx.paths.env_str())?;

    let result = match ctx.direction {
        SyncDirection::Forward => sync_forward(
            &ctx.paths,
            ctx.placeholder,
            ctx.verbose,
            ctx.dry_run,
            ctx.force,
            &config,
            ctx.naming_policy,
        ),
        SyncDirection::Reverse => sync_reverse(
            &ctx.paths,
            ctx.placeholder,
            ctx.verbose,
            ctx.dry_run,
            ctx.force,
            &config,
            ctx.naming_policy,
            &ctx.spec,
        ),
    };
    // Only print hint on success — noise-free error output
    if result.is_ok() {
        ui::print_docs_hint(&docs::SYNC);
    }

    result
}

// ─────────────────────────────────────────────────────────────
// Forward Sync: .env → .env.example
// ─────────────────────────────────────────────────────────────

/// Refuse the sync if any `allow_actual` key holds something that looks like a
/// credential.
///
/// # Why this exists at all
///
/// `allow_actual` writes a **real value** into `.env.example`, a file whose whole
/// purpose is to be committed. That is the right thing for the configuration half
/// of a `.env` — `APP_NAME`, `LOG_LEVEL`, `ENVIRONMENT` — where the real value is
/// the useful example and where `sync --reverse` then hands a teammate working
/// defaults. It is a disaster for the other half, and one mistyped entry is the
/// whole difference.
///
/// So the field is honoured, and the mistake is made unrepresentable instead of
/// discouraged. A tool whose reason to exist is keeping secrets out of places
/// they should not be cannot ship a documented setting whose misuse commits a
/// live credential.
///
/// # Why it runs here
///
/// Before the first write and before the preview, so the answer is all-or-nothing
/// and names **every** offending key at once. Failing per key as the template is
/// generated would leave a half-written file and make the operator run the
/// command once per mistake.
///
/// # What counts as a credential
///
/// The same detectors `evnx scan` uses, so the two commands cannot disagree about
/// one value.
///
/// ⚠️ That was not enough when this was written: the most likely dangerous entry
/// here is a `DATABASE_URL`, and **no built-in rule caught a connection string's
/// password**, so this carried its own check. D15 added the rule, so the check was
/// removed rather than left as a second implementation of one idea — two of those
/// drifting apart is the shape of half the defects in this codebase.
pub fn check_allow_actual(
    config: &PlaceholderConfig,
    env_vars: &IndexMap<String, String>,
) -> Result<()> {
    if config.allow_actual.is_empty() {
        return Ok(());
    }

    // ⚠️ `DetectorRegistry::new()` alone is NOT the same set `evnx scan` uses.
    //
    // It registers the two heuristics and nothing else; `RuleDetector`, which
    // holds the built-in provider patterns *and* any `[[scan.patterns]]`, is added
    // by `with_patterns`. Without it this guard was relying on the entropy
    // fallback and on names containing `secret`/`token`/`key` — enough for
    // `sk_live_…` under any name, because that is high-entropy, but **not** enough
    // for a connection string, whose entropy over the whole URL is low and whose
    // name (`DATABASE_URL`) contains no sensitive word.
    //
    // Found by adding the connection-string rule and watching this guard fail to
    // pick it up.
    let refused = credential_keys(config.allow_actual.iter(), env_vars, "allow_actual")?;

    if refused.is_empty() {
        return Ok(());
    }

    let mut message = String::from(
        "`allow_actual` would copy a real credential into .env.example, which is a \
         committed file. Nothing was written.\n",
    );
    for (key, why) in &refused {
        message.push_str(&format!("\n  {key} — {why}"));
    }
    message.push_str(
        "\n\n  `allow_actual` is for configuration whose real value is the useful \
         example — APP_NAME, LOG_LEVEL, ENVIRONMENT.\n  \
         Remove these keys from it and they will be written as placeholders.",
    );
    anyhow::bail!(message)
}

/// The keys among `keys` whose value in `env_vars` holds a credential, each with
/// the reason, by the detectors `evnx scan` uses. One check for every path that
/// writes **actual values** into `.env.example`: `allow_actual`, and the two
/// interactive "actual values" choices (I-113, DEC-S1). A key absent from
/// `env_vars` is skipped.
pub(crate) fn credential_keys<'a>(
    keys: impl IntoIterator<Item = &'a String>,
    env_vars: &IndexMap<String, String>,
    source: &str,
) -> Result<Vec<(String, String)>> {
    // ⚠️ `DetectorRegistry::new()` alone is NOT the set `evnx scan` uses; see
    // `check_allow_actual`. `with_patterns` adds the provider rules.
    let patterns =
        crate::commands::scan::PatternSet::compile(&crate::utils::patterns::builtin_rules())
            .map_err(|e| anyhow::anyhow!("the built-in secret patterns failed to compile: {e}"))?;
    let registry = crate::commands::scan::DetectorRegistry::new().with_patterns(patterns);
    let mut refused = Vec::new();
    for key in keys {
        let Some(value) = env_vars.get(key) else {
            continue;
        };
        // The strongest verdict, not the first: `scan_kv` runs every detector
        // and more than one can answer for a single value.
        if let Some(detection) = registry
            .scan_kv(key, value, source)
            .into_iter()
            .max_by_key(|d| d.confidence)
        {
            refused.push((key.clone(), detection.pattern));
        }
    }
    Ok(refused)
}

/// Say which keys were kept out of `.env.example`, and log the ones written
/// with real values (DEC-S1: the audit line always goes to stderr).
fn report_actual_values(written: &[String], refused: &[(String, String)]) {
    if !refused.is_empty() {
        ui::warning(format!(
            "{} kept out of .env.example: {} a credential, written as {}",
            refused.len(),
            if refused.len() == 1 {
                "it holds"
            } else {
                "they hold"
            },
            if refused.len() == 1 {
                "a placeholder"
            } else {
                "placeholders"
            }
        ));
        for (key, why) in refused {
            ui::warning_detail(format!("  • {key} — {why}"));
        }
    }
    log_security_event(written, true);
}

fn sync_forward(
    paths: &SyncPaths,
    use_placeholders: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    config: &PlaceholderConfig,
    naming_policy: NamingPolicy,
) -> Result<bool> {
    let parser = Parser::default();

    if !paths.env.exists() {
        ui::error("File not found: .env");
        ui::print_box(
            "Getting started",
            "It looks like this project hasn't been initialized yet.\n\n\
             To create .env and .env.example files, run:\n\n\
             $ evnx init\n\n\
             Or if you have an existing .env file, place it in this directory \
             and try again.",
        );
        ui::print_next_steps(&[
            "Run 'evnx init' to set up a new project",
            "Or copy your existing .env file to this directory",
            "Then run 'evnx sync' again",
        ]);
        anyhow::bail!(".env file not found - run 'evnx init' to get started");
    }

    let env_file = parser
        .parse_file(paths.env_str())
        .context("Failed to parse .env")?;

    // Before anything is written, and before any of it is printed.
    check_allow_actual(config, &env_file.vars)?;
    // ⚠️ "absent" and "will not parse" are different answers.
    //
    // This was a single `Err(_) =>` arm, so a `.env.example` that existed but was
    // corrupt — half-written, bad encoding, an unreadable mode — was announced as
    // ".env.example not found" and then **overwritten** by a plain `evnx sync`.
    // The one file in the pair that is meant to be committed, replaced without a
    // word because it could not be read.
    let example_file = if paths.example.exists() {
        parser
            .parse_file(paths.example_str())
            .with_context(|| format!("Failed to parse {}", paths.example_str()))?
    } else {
        ui::info(format!(
            "{} not found, creating from {}",
            paths.example_str(),
            paths.env_str()
        ));
        return handle_new_example_file(
            paths,
            &env_file.vars,
            use_placeholders,
            force,
            dry_run,
            config,
        );
    };

    // ⚠️ Ordered by the source file, not by hash. `HashSet::difference` yields
    // hash order and Rust seeds its hasher randomly per process, so the same
    // `.env` produced a different ordering on every run — and `sync` *writes*
    // this order into `.env.example`, so two developers syncing the same file
    // got two different diffs against each other.
    let example_keys: HashSet<_> = example_file.vars.keys().collect();
    let missing: Vec<_> = env_file
        .vars
        .keys()
        .filter(|key| !example_keys.contains(key))
        .collect();

    if missing.is_empty() {
        // ⚠️ "Up to date" is true but incurious when the source is empty and
        // the template is not. That is the shape of issue #15: a blank `.env`
        // beside a populated `.env.example` is far more likely a mistake — a
        // truncated file, the wrong directory, a failed secret fetch — than a
        // steady state. sync has nothing to copy, so it correctly changes
        // nothing; it should still say why it did nothing.
        if env_file.vars.is_empty() && !example_file.vars.is_empty() {
            ui::warning(format!(
                "{} has no variables, while {} has {}. Nothing was copied.",
                paths.env_str(),
                paths.example_str(),
                example_file.vars.len(),
            ));
            ui::warning_detail(format!(
                "{} If that is unexpected, check you are in the right directory. \
To fill {} from the template instead, run `evnx sync --direction reverse`.",
                ui::glyph::INFO.dimmed(),
                paths.env_str(),
            ));
            return Ok(false);
        }
        ui::success(".env.example is up to date");
        return Ok(false);
    }

    // Validate naming conventions
    let mut warnings = Vec::new();
    for key in &missing {
        if let Err(msg) = validate_var_name(key, naming_policy) {
            // ⚠️ An `Err`, not `process::exit(1)` (I-116, DEC-S13). A refusal
            // is "could not run", which is 2; under `--check`, 1 means drift,
            // so a bad name read as an out-of-date template.
            if naming_policy == NamingPolicy::Error {
                anyhow::bail!("{msg}");
            }
            warnings.push(msg);
        }
    }

    ui::print_box(
        "Variables to Add",
        &format!(
            "Found {} variable(s) in .env missing from .env.example:\n{}",
            missing.len(),
            missing
                .iter()
                .map(|k| format!("  • {}", key_highlight(k)))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );

    for warning in &warnings {
        ui::warning(warning);
    }

    // ⚠️ `--dry-run` implies non-interactive.
    //
    // A preview that stops to ask "Add these to .env.example?" cannot run anywhere
    // without a terminal, and `sync --dry-run` was therefore dying in CI with
    // `IO error: not a terminal` before printing anything. `--force` was the only
    // way through, which is why every CI example in the docs carries the pair —
    // a combination that reads as a contradiction ("force" plus "change nothing").
    //
    // Previewing the recommended default (placeholders) is the honest answer to
    // "what would this do", since that is option 0 of the prompt being skipped.
    if force || dry_run {
        if force && !dry_run {
            ui::info("Force mode: using placeholder values and skipping prompts");
        }
        let preview = add_with_placeholders_preview(&missing, &env_file.vars, config)?;
        if dry_run {
            print_preview(&SyncPreview {
                target_file: paths.example_str(),
                action: SyncAction::Add,
                variables: preview,
                warnings: vec![],
            });
            return Ok(true);
        }
        add_with_placeholders(paths, &missing, &env_file.vars, config)?;
        ui::success(format!("Added {} variables to .env.example", missing.len()));
        return Ok(true);
    }

    // Interactive mode
    let choices = vec![
        "Yes, add with placeholder values (recommended)",
        "Yes, add with actual values ! SECURITY RISK",
        "Let me choose individually",
        "No, skip",
    ];

    let selection = Select::new()
        .with_prompt("Add these to .env.example?")
        .items(&choices)
        .default(0)
        .interact()?;

    match selection {
        0 => {
            let preview = add_with_placeholders_preview(&missing, &env_file.vars, config)?;
            if dry_run {
                print_preview(&SyncPreview {
                    target_file: paths.example_str(),
                    action: SyncAction::Add,
                    variables: preview,
                    warnings: vec![],
                });
                return Ok(true);
            }
            add_with_placeholders(paths, &missing, &env_file.vars, config)?;
        }
        1 => {
            ui::print_box(
                "! SECURITY WARNING",
                "Adding actual values to .env.example may expose secrets if \
                 this file is committed to version control.\n\n\
                 Only proceed if you are certain .env.example is gitignored \
                 or contains no sensitive data.",
            );

            let confirm = Confirm::new()
                .with_prompt("Are you absolutely sure you want to add ACTUAL VALUES?")
                .default(false)
                .interact()?;

            if !confirm {
                ui::info("Aborted: values not added");
                return Ok(true);
            }

            let _ = verbose;
            let refused = credential_keys(missing.iter().copied(), &env_file.vars, "sync")?;
            let preview = add_with_actual_values_preview(&missing, &env_file.vars, config)?;
            if dry_run {
                print_preview(&SyncPreview {
                    target_file: paths.example_str(),
                    action: SyncAction::Add,
                    variables: preview,
                    warnings: vec!["! Actual values would be written (security risk)".into()],
                });
                return Ok(true);
            }
            add_with_actual_values(paths, &missing, &env_file.vars, config)?;
            let written: Vec<String> = missing
                .iter()
                .filter(|k| !refused.iter().any(|(r, _)| r == **k))
                .map(|k| (*k).clone())
                .collect();
            report_actual_values(&written, &refused);
        }
        2 => {
            let preview = add_interactively_preview(&missing, &env_file.vars, config)?;
            if dry_run {
                print_preview(&SyncPreview {
                    target_file: paths.example_str(),
                    action: SyncAction::Add,
                    variables: preview,
                    warnings: vec![],
                });
                return Ok(true);
            }
            add_interactively(paths, &missing, &env_file.vars, config)?;
        }
        3 => {
            ui::info("No changes made");
            return Ok(true);
        }
        _ => unreachable!(),
    }

    if !dry_run
        && Confirm::new()
            .with_prompt("Add a comment explaining these variables?")
            .default(true)
            .interact()?
    {
        let comment: String = Input::new()
            .with_prompt("Comment")
            .default("Additional configuration".to_string())
            .interact_text()?;

        let mut content = fs::read_to_string(&paths.example)?;
        content.push_str(&format!("\n# {}\n", comment));
        write_template(&paths.example, &content)?;
    }

    ui::success(format!(
        "Updated .env.example (+{} variables)",
        missing.len()
    ));

    ui::print_next_steps(&[
        "Review .env.example to ensure no secrets were added",
        "Commit .env.example to Git (never commit .env)",
        "Share changes with your team",
    ]);

    Ok(true)
}

/// Create `.env.example` when it does not exist yet.
///
/// ⚠️ **Placeholders are the default here, and that is a v0.5.0 change.**
///
/// This path used to copy `.env` **verbatim** unless `--placeholder` was passed
/// — real secrets and all — then print "Created .env.example", then advise
/// committing it. The placeholder-versus-actual prompt, whose second option is
/// labelled a security risk, only guarded the *other* path: adding variables to
/// an example file that already existed.
///
/// So the guard covered every case except the first `evnx sync` in a new
/// project, which is the one a getting-started guide tells you to run, and the
/// one where `.env` is most likely to hold the values you just pasted in.
/// `evnx scan` does not read `.env.example`, so nothing downstream caught it
/// either.
///
/// The choice is still offered when someone is at a terminal to make it. Without
/// one, it resolves to placeholders and says so, rather than silently picking
/// the unsafe branch.
fn handle_new_example_file(
    paths: &SyncPaths,
    vars: &IndexMap<String, String>,
    use_placeholders: bool,
    force: bool,
    dry_run: bool,
    config: &PlaceholderConfig,
) -> Result<bool> {
    let use_placeholders = if use_placeholders || force {
        true
    } else if crate::utils::term::interactive() {
        let choices = [
            "Placeholder values (recommended — safe to commit)",
            "Actual values copied from .env  ! SECURITY RISK",
        ];
        Select::new()
            .with_prompt(format!("Create {} with", paths.example_str()))
            .items(&choices)
            .default(0)
            .interact()?
            == 0
    } else {
        ui::info(format!(
            "no terminal: creating {} with placeholder values",
            paths.example_str()
        ));
        true
    };

    // DEC-S1: actual values, except a key holding a credential (I-113).
    let refused = if use_placeholders {
        Vec::new()
    } else {
        credential_keys(vars.keys(), vars, "sync")?
    };
    let is_refused = |k: &str| refused.iter().any(|(r, _)| r == k);

    let preview = if use_placeholders {
        convert_to_example_preview(vars, config)?
    } else {
        vars.iter()
            .map(|(k, v)| VarChange {
                key: k.clone(),
                old_value: None,
                new_value: if is_refused(k) {
                    generate_placeholder(k, Some(v), config)
                } else {
                    v.clone()
                },
                is_placeholder: is_refused(k),
            })
            .collect()
    };

    if dry_run {
        print_preview(&SyncPreview {
            target_file: paths.example_str(),
            action: SyncAction::Add,
            variables: preview,
            warnings: vec!["New file would be created".into()],
        });
        return Ok(true);
    }

    if use_placeholders {
        convert_to_example(paths, vars, config)?;
    } else {
        // The `.env` text as it is — comments and order kept — with each
        // credential's line rewritten to its placeholder.
        let text = fs::read_to_string(&paths.env)?;
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let body = line.trim_start();
            let body = body.strip_prefix("export ").unwrap_or(body);
            let key = body.split('=').next().unwrap_or("").trim();
            if body.contains('=') && is_refused(key) {
                let eol = if line.ends_with("\r\n") {
                    "\r\n"
                } else if line.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "{key}={}{eol}",
                    generate_placeholder(key, vars.get(key), config)
                ));
            } else {
                out.push_str(line);
            }
        }
        write_template(&paths.example, &out)?;
        let written: Vec<String> = vars.keys().filter(|k| !is_refused(k)).cloned().collect();
        report_actual_values(&written, &refused);
    }

    ui::success(format!("Created {}", paths.example_str()));
    if use_placeholders {
        ui::print_next_steps(&[
            "Review .env.example to ensure the placeholders are appropriate",
            "Commit .env.example to Git (never commit .env)",
        ]);
    } else {
        // ⚠️ Do not tell someone to commit a file that holds their real values.
        ui::warning(format!(
            "{} holds the ACTUAL values from .env, not placeholders",
            paths.example_str()
        ));
        ui::print_next_steps(&[
            "Replace the values before committing — this file is not safe to commit as written",
            "Or re-create it with `evnx sync --placeholder`",
        ]);
    }
    Ok(true)
}

// ─────────────────────────────────────────────────────────────
// Reverse Sync: .env.example → .env
// ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn sync_reverse(
    paths: &SyncPaths,
    _use_placeholders: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    config: &PlaceholderConfig,
    naming_policy: NamingPolicy,
    spec: &crate::core::spec::Spec,
) -> Result<bool> {
    if verbose {
        eprintln!("[DEBUG] Running reverse sync");
    }
    let parser = Parser::default();

    if !paths.example.exists() {
        ui::error("File not found: .env.example");
        ui::print_box(
            "Getting started",
            "The template file .env.example is missing.\n\n\
             To create it, either:\n\n\
             1. Run 'evnx init' to generate from a blueprint, OR\n\
             2. Create .env.example manually with your variable names\n\n\
             Example .env.example:\n\
             DATABASE_URL=your_database_url\n\
             API_KEY=your_api_key",
        );
        ui::print_next_steps(&[
            "Run 'evnx init' to generate .env.example from a template",
            "Or create .env.example manually with your variable names",
            "Then run 'evnx sync --direction reverse' again",
        ]);
        anyhow::bail!(".env.example not found - run 'evnx init' or create it manually");
    }

    let example_file = parser
        .parse_file(paths.example_str())
        .with_context(|| format!("Failed to parse {}", paths.example_str()))?;

    // Same distinction as in `sync_forward` — see the note there.
    let env_file = if paths.env.exists() {
        parser
            .parse_file(paths.env_str())
            .context("Failed to parse .env")?
    } else {
        ui::info(format!(
            "{} not found, creating from {}",
            paths.env_str(),
            paths.example_str()
        ));
        return handle_new_env_file(paths, &example_file.vars, dry_run, config);
    };

    // ⚠️ The template is not the only thing that declares a variable.
    //
    // Before this, `evnx validate` would report a spec-declared variable as
    // missing while `evnx sync --direction reverse` said ".env is up to date" —
    // two commands contradicting each other about the same variable in the same
    // project. A contract that `validate` enforces and `sync` cannot satisfy is
    // half a feature.
    //
    // Only variables that are **required** and **apply to this file** are added.
    // An optional one being absent is permitted by the contract, so inventing a
    // placeholder for it would be work the project did not ask for.
    let env_path_owned = paths.env_str();
    let env_label = crate::core::env_name::name_of(&env_path_owned);
    let mut declared: Vec<String> = example_file.vars.keys().cloned().collect();
    for (name, var) in spec {
        if var.is_required() && var.applies_to(env_label) && !declared.contains(name) {
            declared.push(name.clone());
        }
    }

    let env_keys: HashSet<&String> = env_file.vars.keys().collect();
    let missing: Vec<&String> = declared.iter().filter(|k| !env_keys.contains(*k)).collect();

    if missing.is_empty() {
        ui::success(".env is up to date");
        return Ok(false);
    }

    for key in &missing {
        if let Err(msg) = validate_var_name(key, naming_policy) {
            // ⚠️ An `Err`, not `process::exit(1)` (I-116, DEC-S13). A refusal
            // is "could not run", which is 2; under `--check`, 1 means drift,
            // so a bad name read as an out-of-date template.
            if naming_policy == NamingPolicy::Error {
                anyhow::bail!("{msg}");
            }
            ui::warning(&msg);
        }
    }

    ui::print_box(
        "Missing Variables",
        &format!(
            "Found {} variable(s) in .env.example missing from .env:\n{}",
            missing.len(),
            missing
                .iter()
                .map(|k| format!("  • {}", key_highlight(k)))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );

    // `--dry-run` implies non-interactive — see the note in `sync_forward`.
    if force || dry_run {
        if force && !dry_run {
            ui::info("Force mode: adding the template's values (placeholders where it has none), skipping prompts");
        }
        let preview = add_from_example_preview(&missing, &example_file.vars, true, config)?;
        if dry_run {
            print_preview(&SyncPreview {
                target_file: paths.env_str(),
                action: SyncAction::Add,
                variables: preview,
                warnings: vec!["Remember to replace placeholders with real values".into()],
            });
            return Ok(true);
        }
        add_from_example(paths, &missing, &example_file.vars, true, config)?;
        ui::success(format!("Added {} variables to .env", missing.len()));
        return Ok(true);
    }

    let choices = vec![
        "Yes, with the template's values (placeholders where it has none)",
        "Yes, prompt me for real values",
        "Let me choose individually",
        "No, skip",
    ];

    let selection = Select::new()
        .with_prompt("Add to .env?")
        .items(&choices)
        .default(0)
        .interact()?;

    let use_placeholders = match selection {
        0 => true,
        1 => false,
        2 => {
            let preview =
                add_from_example_interactive_preview(&missing, &example_file.vars, config)?;
            if dry_run {
                print_preview(&SyncPreview {
                    target_file: paths.env_str(),
                    action: SyncAction::Add,
                    variables: preview,
                    warnings: vec![],
                });
                return Ok(true);
            }
            add_from_example_interactive(paths, &missing, &example_file.vars, config)?;
            ui::success("Added selected variables to .env");
            return Ok(true);
        }
        3 => {
            ui::info("No changes made");
            return Ok(true);
        }
        _ => unreachable!(),
    };

    let preview = add_from_example_preview(&missing, &example_file.vars, use_placeholders, config)?;
    if dry_run {
        print_preview(&SyncPreview {
            target_file: paths.env_str(),
            action: SyncAction::Add,
            variables: preview,
            warnings: if use_placeholders {
                vec!["Remember to replace placeholders with real values".into()]
            } else {
                vec![]
            },
        });
        return Ok(true);
    }

    add_from_example(
        paths,
        &missing,
        &example_file.vars,
        use_placeholders,
        config,
    )?;
    ui::success(format!("Added {} variables to .env", missing.len()));

    if use_placeholders {
        ui::warning("Remember to replace placeholder values with real credentials!");
    }

    Ok(true)
}

fn handle_new_env_file(
    paths: &SyncPaths,
    example_vars: &IndexMap<String, String>,
    dry_run: bool,
    config: &PlaceholderConfig,
) -> Result<bool> {
    if dry_run {
        let preview: Vec<VarChange> = example_vars
            .iter()
            .map(|(k, v)| VarChange {
                key: k.clone(),
                old_value: None,
                new_value: v.clone(),
                is_placeholder: is_generated_value(v, config),
            })
            .collect();
        print_preview(&SyncPreview {
            target_file: paths.env_str(),
            action: SyncAction::Add,
            variables: preview,
            warnings: vec!["Replace placeholder values with real credentials!".into()],
        });
        return Ok(true);
    }

    write_env(&paths.env, &fs::read_to_string(&paths.example)?)?;
    ui::success("Created .env");
    ui::warning("Replace placeholder values with real credentials!");
    ui::print_next_steps(&[
        "Edit .env and replace placeholder values with real credentials",
        "Run 'chmod 600 .env' to secure the file",
        "Never commit .env to version control",
    ]);
    Ok(true)
}

// ─────────────────────────────────────────────────────────────
// Add Functions (Forward Sync Helpers)
// ─────────────────────────────────────────────────────────────

fn add_with_placeholders(
    paths: &SyncPaths,
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let mut content = fs::read_to_string(&paths.example).context("Failed to read .env.example")?;
    content.push_str(&format!("\n# Synced from {}\n", paths.env_str()));

    for key in keys {
        let placeholder = generate_placeholder(key, values.get(*key), config);
        content.push_str(&envfile::line(key, &placeholder));
        content.push('\n');
    }

    write_template(&paths.example, &content)?;
    Ok(())
}

fn add_with_placeholders_preview(
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let placeholder = generate_placeholder(key, values.get(*key), config);
            VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: placeholder,
                is_placeholder: true,
            }
        })
        .collect())
}

/// Append `keys` with their real values — except a key holding a credential,
/// which gets its placeholder instead (I-113, DEC-S1).
fn add_with_actual_values(
    paths: &SyncPaths,
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let refused = credential_keys(keys.iter().copied(), values, "sync")?;
    let mut content = fs::read_to_string(&paths.example)?;
    content.push_str(&format!(
        "\n# Synced from {} [! ACTUAL VALUES]\n",
        paths.env_str()
    ));

    for key in keys {
        if refused.iter().any(|(r, _)| r == *key) {
            let placeholder = generate_placeholder(key, values.get(*key), config);
            content.push_str(&envfile::line(key, &placeholder));
            content.push('\n');
        } else if let Some(value) = values.get(*key) {
            content.push_str(&envfile::line(key, value));
            content.push('\n');
        }
    }

    write_template(&paths.example, &content)?;
    Ok(())
}

fn add_with_actual_values_preview(
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    let refused = credential_keys(keys.iter().copied(), values, "sync")?;
    Ok(keys
        .iter()
        .filter_map(|key| {
            values.get(*key).map(|v| {
                if refused.iter().any(|(r, _)| r == *key) {
                    VarChange {
                        key: (*key).clone(),
                        old_value: None,
                        new_value: generate_placeholder(key, Some(v), config),
                        is_placeholder: true,
                    }
                } else {
                    VarChange {
                        key: (*key).clone(),
                        old_value: None,
                        new_value: v.clone(),
                        is_placeholder: false,
                    }
                }
            })
        })
        .collect())
}

fn add_interactively(
    paths: &SyncPaths,
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let selected = MultiSelect::new()
        .with_prompt("Select variables to add")
        .items(keys)
        .interact()?;

    let mut content = fs::read_to_string(&paths.example)?;
    content.push_str(&format!("\n# Synced from {}\n", paths.env_str()));

    for &idx in &selected {
        let key = keys[idx];
        let placeholder = generate_placeholder(key, values.get(key), config);
        content.push_str(&envfile::line(key, &placeholder));
        content.push('\n');
    }

    write_template(&paths.example, &content)?;
    Ok(())
}

fn add_interactively_preview(
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let placeholder = generate_placeholder(key, values.get(*key), config);
            VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: placeholder,
                is_placeholder: true,
            }
        })
        .collect())
}

fn convert_to_example(
    paths: &SyncPaths,
    vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let mut content = String::new();
    content.push_str(&format!("# Generated from {}\n", paths.env_str()));
    content.push_str("# Replace all placeholder values with real credentials\n\n");

    for (key, value) in vars {
        let placeholder = generate_placeholder(key, Some(value), config);
        content.push_str(&envfile::line(key, &placeholder));
        content.push('\n');
    }

    write_template(&paths.example, &content)?;
    Ok(())
}

fn convert_to_example_preview(
    vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(vars
        .iter()
        .map(|(k, v)| {
            let placeholder = generate_placeholder(k, Some(v), config);
            VarChange {
                key: k.clone(),
                old_value: None,
                new_value: placeholder,
                is_placeholder: true,
            }
        })
        .collect())
}

// ─────────────────────────────────────────────────────────────
// Reverse Sync Add Functions
// ─────────────────────────────────────────────────────────────

/// What reverse writes into `.env` for `key` (I-115, DEC-S2), and whether it
/// was generated.
///
/// The template's own value when it has one: it is the project's committed
/// answer for a new teammate, and it is already public. It used to be thrown
/// away and regenerated from the name — `REDIS_URL=redis://localhost:6379`
/// came back as `…/0`, `API_PORT=8080` as `8000`. Only an empty or
/// placeholder-looking value is generated, through the project's
/// `--template-config` (which reverse used to ignore for a built-in default).
fn reverse_value(
    key: &str,
    template: Option<&String>,
    config: &PlaceholderConfig,
) -> (String, bool) {
    use crate::core::classify::{is_placeholder, Strictness};
    match template {
        Some(v) if !v.trim().is_empty() && !is_placeholder(key, v, Strictness::Lenient) => {
            (v.clone(), false)
        }
        _ => (generate_placeholder(key, template, config), true),
    }
}

fn add_from_example(
    paths: &SyncPaths,
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    use_placeholders: bool,
    config: &PlaceholderConfig,
) -> Result<()> {
    let mut content = fs::read_to_string(&paths.env).unwrap_or_default();
    // ⚠️ Not "Synced from .env.example". Since the contract became a source,
    // a variable reaching `.env` here may have been declared in `[vars]` and
    // never appear in the template at all — naming the template would put a
    // false statement in a file the user reads.
    content.push_str("\n# Added by evnx sync\n");

    for key in keys {
        let value = if use_placeholders {
            reverse_value(key, example_vars.get(*key), config).0
        } else {
            let default = example_vars.get(*key).map(|s| s.as_str()).unwrap_or("");
            Input::new()
                .with_prompt(format!("Value for {}", key))
                .default(default.to_string())
                .interact_text()?
        };
        content.push_str(&envfile::line(key, &value));
        content.push('\n');
    }

    write_env(&paths.env, &content)?;
    Ok(())
}

fn add_from_example_preview(
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    use_placeholders: bool,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let (new_val, generated) = if use_placeholders {
                reverse_value(key, example_vars.get(*key), config)
            } else {
                (example_vars.get(*key).cloned().unwrap_or_default(), false)
            };
            VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: new_val,
                is_placeholder: generated,
            }
        })
        .collect())
}

fn add_from_example_interactive(
    paths: &SyncPaths,
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let selected = MultiSelect::new()
        .with_prompt("Select variables to add")
        .items(keys)
        .interact()?;

    let mut content = fs::read_to_string(&paths.env).unwrap_or_default();
    content.push_str(&format!("\n# Synced from {}\n", paths.example_str()));

    for &idx in &selected {
        let key = keys[idx];
        let (value, _) = reverse_value(key, example_vars.get(key), config);
        content.push_str(&envfile::line(key, &value));
        content.push('\n');
    }

    write_env(&paths.env, &content)?;
    Ok(())
}

fn add_from_example_interactive_preview(
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let (value, generated) = reverse_value(key, example_vars.get(*key), config);
            VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: value,
                is_placeholder: generated,
            }
        })
        .collect())
}

// ─────────────────────────────────────────────────────────────
// Preview Output Helpers
// ─────────────────────────────────────────────────────────────

fn print_preview(preview: &SyncPreview) {
    ui::print_preview_header();

    println!(
        "{}\n",
        format!(
            "Target: {} (action: {:?})",
            preview.target_file, preview.action
        )
        .dimmed()
    );

    if preview.variables.is_empty() {
        println!("{}", "  No changes".dimmed());
    } else {
        for var in &preview.variables {
            // ⚠️ Both arms must be one column wide. This was "🔒" against "!",
            // which is 2 columns against 1 — so every placeholder row pushed its
            // key one column right of every non-placeholder row, in the same list.
            let marker = if var.is_placeholder {
                ui::glyph::INFO
            } else {
                ui::glyph::WARN
            };
            println!(
                "  {} {} = {}",
                marker,
                key_highlight(&var.key),
                value_preview(&var.new_value, var.is_placeholder)
            );
        }
    }

    if !preview.warnings.is_empty() {
        println!();
        for warning in &preview.warnings {
            ui::warning(warning);
        }
    }

    println!("\n{}", "(dry-run: no files were modified)".dimmed());
}

fn key_highlight(key: &str) -> ColoredString {
    key.cyan()
}

fn value_preview(value: &str, is_placeholder: bool) -> ColoredString {
    if is_placeholder {
        value.dimmed()
    } else {
        value.yellow()
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
    fn test_atomic_write_sets_permissions() {
        let temp_dir = TempDir::new().unwrap();
        let env_path = temp_dir.path().join(".env");

        write_env(&env_path, "TEST=value\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&env_path).unwrap().permissions().mode();
            assert_eq!(perms & 0o777, 0o600, "File should have 0o600 permissions");
        }
    }

    #[test]
    fn test_atomic_write_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir.path().join("subdir").join(".env");

        // Should not panic even if subdir doesn't exist
        let result = write_env(&nested_path, "TEST=value\n");
        // This will fail because parent doesn't exist, but shouldn't panic
        assert!(result.is_err());
    }

    #[test]
    fn test_atomic_write_ensures_env_file_permissions() {
        let temp_dir = TempDir::new().unwrap();
        let env_path = temp_dir.path().join(".env");

        write_env(&env_path, "TEST=value\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&env_path).unwrap().permissions().mode();
            // This is the guarantee: .env files MUST have 0o600
            assert_eq!(
                perms & 0o777,
                0o600,
                ".env files must have restrictive permissions"
            );
        }
    }

    /// I-123, DEC-S7: a new template is 0644, and an existing file keeps the
    /// mode its owner gave it.
    #[test]
    #[cfg(unix)]
    fn t_i123_modes_new_template_0644_existing_kept() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new().unwrap();
        let example = d.path().join(".env.example");
        write_template(&example, "A=\n").unwrap();
        assert_eq!(
            fs::metadata(&example).unwrap().permissions().mode() & 0o777,
            0o644
        );

        let env = d.path().join(".env");
        fs::write(&env, "A=1\n").unwrap();
        fs::set_permissions(&env, fs::Permissions::from_mode(0o640)).unwrap();
        write_env(&env, "A=2\n").unwrap();
        assert_eq!(
            fs::metadata(&env).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    // Preview helper tests
    #[test]
    fn test_key_highlight_returns_colored() {
        // Just ensure it doesn't panic and returns a ColoredString
        let result = key_highlight("TEST_KEY");
        assert!(!format!("{}", result).is_empty());
    }

    #[test]
    fn test_value_preview_placeholder_vs_actual() {
        let placeholder = value_preview("YOUR_VALUE_HERE", true);
        let actual = value_preview("secret123", false);

        // Both should render, but with different styling (can't easily test colors in unit test)
        assert!(!format!("{}", placeholder).is_empty());
        assert!(!format!("{}", actual).is_empty());
    }

    #[test]
    fn test_add_with_placeholders_preview() {
        let mut values = IndexMap::new();
        values.insert("API_KEY".to_string(), "sk_live_123".to_string());
        values.insert("PORT".to_string(), "3000".to_string());

        // ✅ FIXED: Create Vec<&String> properly
        let keys_vec: Vec<String> = vec!["API_KEY".to_string(), "PORT".to_string()];
        let keys: Vec<&String> = keys_vec.iter().collect();

        let config = PlaceholderConfig::default();
        let preview = add_with_placeholders_preview(&keys, &values, &config).unwrap();

        assert_eq!(preview.len(), 2);
        assert!(preview.iter().all(|v| v.is_placeholder));
        assert!(preview
            .iter()
            .any(|v| v.key == "API_KEY" && v.new_value == "YOUR_KEY_HERE"));
        assert!(preview
            .iter()
            .any(|v| v.key == "PORT" && v.new_value == "8000"));
    }

    #[test]
    fn test_add_with_actual_values_preview() {
        let mut values = IndexMap::new();
        values.insert("API_KEY".to_string(), "sk_live_123".to_string());

        // ✅ FIXED: Create Vec<&String> properly
        let keys_vec: Vec<String> = vec!["API_KEY".to_string()];
        let keys: Vec<&String> = keys_vec.iter().collect();

        let preview =
            add_with_actual_values_preview(&keys, &values, &PlaceholderConfig::default()).unwrap();

        // DEC-S1 (I-113): a credential is shown as the placeholder it will be
        // written as, never as its real value.
        assert_eq!(preview.len(), 1);
        assert!(preview[0].is_placeholder);
        assert_ne!(preview[0].new_value, "sk_live_123");
    }

    // NEW: Integration-style tests with temp files
    #[test]
    fn test_convert_to_example_preview() {
        let mut vars = IndexMap::new();
        // ✅ FIXED: Use "postgresql://" to match the built-in rule
        vars.insert(
            "DB_URL".to_string(),
            "postgresql://user:pass@localhost/db".to_string(),
        );
        vars.insert("API_KEY".to_string(), "secret".to_string());

        let config = PlaceholderConfig::default();
        let preview = convert_to_example_preview(&vars, &config).unwrap();

        assert_eq!(preview.len(), 2);
        assert!(preview.iter().all(|v| v.is_placeholder));

        // ✅ FIXED: Check for the exact expected placeholder
        assert!(preview.iter().any(|v| v.key == "DB_URL"
            && v.new_value == "postgresql://user:password@localhost:5432/dbname"));
    }

    #[test]
    fn test_add_from_example_preview_with_placeholders() {
        let mut example_vars = IndexMap::new();
        example_vars.insert("NEW_VAR".to_string(), "placeholder_value".to_string());

        // ✅ FIXED: Create Vec<&String> properly
        let keys_vec: Vec<String> = vec!["NEW_VAR".to_string()];
        let keys: Vec<&String> = keys_vec.iter().collect();

        let preview =
            add_from_example_preview(&keys, &example_vars, true, &PlaceholderConfig::default())
                .unwrap();

        assert_eq!(preview.len(), 1);
        assert!(preview[0].is_placeholder);
        assert_eq!(preview[0].new_value, "YOUR_VALUE_HERE");
    }

    #[test]
    fn test_add_from_example_preview_without_placeholders() {
        let mut example_vars = IndexMap::new();
        example_vars.insert("NEW_VAR".to_string(), "example_default".to_string());

        // ✅ FIXED: Create Vec<&String> properly
        let keys_vec: Vec<String> = vec!["NEW_VAR".to_string()];
        let keys: Vec<&String> = keys_vec.iter().collect();

        let preview =
            add_from_example_preview(&keys, &example_vars, false, &PlaceholderConfig::default())
                .unwrap();

        assert_eq!(preview.len(), 1);
        assert!(!preview[0].is_placeholder);
        assert_eq!(preview[0].new_value, "example_default");
    }
}
