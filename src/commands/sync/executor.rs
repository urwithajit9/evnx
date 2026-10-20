//! Execution logic for sync operations: forward, reverse, previews, and file writes.

use anyhow::{Context, Result};
use colored::*;
use dialoguer::{Input, MultiSelect, Select};
use indexmap::IndexMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::{NamingPolicy, SyncDirection};
use crate::core::envfile::{self, only_in};
use crate::core::placeholder::{generate_placeholder, is_generated_value, PlaceholderConfig};
use crate::core::Parser;
use crate::docs;
use crate::utils::string::count;
use crate::utils::ui;

use super::models::{SyncAction, SyncPreview, VarChange};
use super::security::{log_security_event, validate_var_name};

/// Write the env file sync manages: atomic, new files 0600, an existing file
/// keeps its mode, a symlink is written through (DEC-S6, DEC-S7, core::safe_write).
///
/// ⚠️ This used to force 0600 on every file whose name started with `.env` and
/// did not contain "example" — including a `.env` the user had set to 0640 —
/// and the template came out 0600 only because the temp file did (I-123).
///
/// ⚠️ Only the symlink half of `safe_write::report`: a readable `.env` was
/// already reported before the run, and a second warning after the write said
/// the same thing in other words (I-276).
fn write_env(path: &Path, content: &str) -> Result<()> {
    let written = crate::core::safe_write::write_private(path, content.as_bytes())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    crate::core::safe_write::report_link(&written);
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
/// put a path even if you wanted one. Messages named the literals for longer
/// still: `--env nope.env` reported "File not found: .env" (I-125).
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
    /// No `--direction` was given (DEC-S17): forward, and `--check` also
    /// counts names only the template has (DEC-S3). A named direction asks
    /// one way, as CI and the pre-commit hook do.
    pub two_way: bool,
    pub verbose: bool,
    /// `--dry-run`, or `--check`, which implies it.
    pub dry_run: bool,
    pub check: bool,
    /// `--force`, or the deprecated `--placeholder` (DEC-S9).
    pub force: bool,
    pub naming_policy: NamingPolicy,
    /// The project's declared contract, when it has one.
    ///
    /// ⚠️ Used by the reverse direction only. Forward sync makes the template
    /// match `.env`, and a spec has no values to contribute to that.
    pub spec: crate::core::spec::Spec,
}

// ─────────────────────────────────────────────────────────────
// The drift: one computation for every output (I-117)
// ─────────────────────────────────────────────────────────────

/// Where a name the target lacks is declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    /// The source file (`.env` forward, the template in reverse).
    Source,
    /// Only `[vars]` in `.evnx.toml`, as a required name (reverse).
    Spec,
}

/// What differs between the two files, by name.
pub(super) struct Drift {
    /// Names the target lacks, in the source's order; in reverse, the
    /// template's first, then names only the spec declares (sorted).
    pub missing: Vec<(String, Origin)>,
    /// Two-way only (DEC-S17): names only the template has.
    pub template_only: Vec<String>,
}

impl Drift {
    /// Whether `--check` reports drift (exit 1).
    pub(super) fn out_of_step(&self) -> bool {
        !self.missing.is_empty() || !self.template_only.is_empty()
    }

    fn names(&self) -> Vec<&String> {
        self.missing.iter().map(|(k, _)| k).collect()
    }

    fn spec_names(&self) -> Vec<String> {
        self.missing
            .iter()
            .filter(|(_, o)| *o == Origin::Spec)
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// The two files as read, and the drift between them.
pub(super) struct Loaded {
    /// `None` when the file does not exist (reverse may create it).
    pub env: Option<IndexMap<String, String>>,
    /// `None` when the file does not exist (forward may create it).
    pub example: Option<IndexMap<String, String>>,
    pub drift: Drift,
}

/// The `--template-config` file, or the built-in rules. A file that does not
/// load — bad JSON, an unknown key (I-121), a pattern that does not compile —
/// is an error before anything is read or written.
///
/// ⚠️ Loaded once, by `mod.rs`, before every branch: with no `.env` the
/// `--check` path returned before it, so the same command passed in CI (no
/// `.env`) and failed locally (I-279).
pub(super) fn load_config(path: Option<&PathBuf>) -> Result<PlaceholderConfig> {
    Ok(path
        .map(PlaceholderConfig::from_path)
        .transpose()?
        .unwrap_or_default())
}

/// Read both files and work out what differs: the one computation behind the
/// preview, the write, the pretty `--check` and the JSON report (I-117).
///
/// Refused here, before anything is printed or written:
/// - a file that will not parse, or a missing source;
/// - forward: a credential in `allow_actual` (`check_allow_actual`);
/// - a name `--naming-policy error` refuses (I-116, DEC-S13);
/// - under `--check`: an empty `.env` beside a populated template (I-127,
///   DEC-S14) — far more likely a mistake than a steady state, so it is no
///   verdict rather than a pass.
///
/// `pretty` prints the getting-started box before a missing-source error.
pub(super) fn load(ctx: &SyncCtx, config: &PlaceholderConfig, pretty: bool) -> Result<Loaded> {
    let p = &ctx.paths;
    let parse = |path: &Path| -> Result<IndexMap<String, String>> {
        Ok(Parser::default()
            .parse_file(path)
            .with_context(|| format!("Failed to parse {}", path.display()))?
            .vars)
    };
    let empty = IndexMap::new();

    let loaded = match ctx.direction {
        SyncDirection::Forward => {
            if !p.env.exists() {
                if pretty {
                    env_not_found(p);
                }
                anyhow::bail!("{} not found - run 'evnx init' to get started", p.env_str());
            }
            let env = parse(&p.env)?;
            // Before anything is written, and before any of it is printed.
            check_allow_actual(config, &env, p)?;
            // ⚠️ "absent" and "will not parse" are different answers.
            //
            // This was a single `Err(_) =>` arm, so a `.env.example` that existed
            // but was corrupt — half-written, bad encoding, an unreadable mode —
            // was announced as ".env.example not found" and then **overwritten**
            // by a plain `evnx sync`. The one file in the pair that is meant to
            // be committed, replaced without a word because it could not be read.
            let example = p.example.exists().then(|| parse(&p.example)).transpose()?;
            let target = example.as_ref().unwrap_or(&empty);
            if ctx.check && env.is_empty() && !target.is_empty() {
                anyhow::bail!(
                    "{env} has no variables, while {example} has {n}: evnx could not verify \
                     the pair.\n  An empty {env} beside a populated template is far more \
                     likely a mistake — a truncated file, the wrong directory, a failed \
                     secret fetch — than a pair in step.",
                    env = p.env_str(),
                    example = p.example_str(),
                    n = target.len()
                );
            }
            Drift {
                // ⚠️ Ordered by the source file, not by hash: `sync` writes this
                // order into `.env.example`, and a per-process `HashSet` order
                // gave two developers two different diffs of the same file.
                missing: only_in(&env, target)
                    .into_iter()
                    .map(|k| (k, Origin::Source))
                    .collect(),
                template_only: if ctx.two_way {
                    only_in(target, &env)
                } else {
                    Vec::new()
                },
            }
            .into_loaded(Some(env), example)
        }
        SyncDirection::Reverse => {
            if !p.example.exists() {
                if pretty {
                    example_not_found(p);
                }
                anyhow::bail!(
                    "{} not found - run 'evnx init' or create it manually",
                    p.example_str()
                );
            }
            let example = parse(&p.example)?;
            let env = p.env.exists().then(|| parse(&p.env)).transpose()?;
            let target = env.as_ref().unwrap_or(&empty);
            let mut missing: Vec<(String, Origin)> = only_in(&example, target)
                .into_iter()
                .map(|k| (k, Origin::Source))
                .collect();
            // ⚠️ The template is not the only thing that declares a variable.
            //
            // Before this, `evnx validate` would report a spec-declared variable
            // as missing while `evnx sync --direction reverse` said ".env is up to
            // date" — two commands contradicting each other about the same
            // variable. Only variables that are **required** and **apply to this
            // file** are added: an optional one being absent is permitted by the
            // contract. A new `.env` gets them too (I-275).
            let env_path = p.env_str();
            let label = crate::core::env_name::name_of(&env_path);
            missing.extend(
                crate::core::spec::missing_required(
                    &ctx.spec,
                    |k| example.contains_key(k) || target.contains_key(k),
                    label,
                )
                .into_iter()
                .map(|k| (k.to_string(), Origin::Spec)),
            );
            Drift {
                missing,
                template_only: Vec::new(),
            }
            .into_loaded(env, Some(example))
        }
    };

    for (key, _) in &loaded.drift.missing {
        // ⚠️ An `Err`, not `process::exit(1)` (I-116, DEC-S13). A refusal
        // is "could not run", which is 2; under `--check`, 1 means drift,
        // so a bad name read as an out-of-date template.
        validate_var_name(key, ctx.naming_policy).map_err(|msg| anyhow::anyhow!(msg))?;
    }
    Ok(loaded)
}

impl Drift {
    fn into_loaded(
        self,
        env: Option<IndexMap<String, String>>,
        example: Option<IndexMap<String, String>>,
    ) -> Loaded {
        Loaded {
            env,
            example,
            drift: self,
        }
    }
}

/// Why sync will not ask: `None` when it may (DEC-S9, DEC-S18).
///
/// ⚠️ Without a terminal every prompt used to fail with "IO error: not a
/// terminal: not a terminal", naming neither `--force` nor `--dry-run`, and
/// `CI=1` — documented as "like `--force`" — was never read (I-124, D-74).
fn no_prompt_reason() -> Option<&'static str> {
    if crate::utils::term::ci() {
        Some("CI is set")
    } else if !crate::utils::term::can_prompt() {
        // stdin to answer on, and stderr to draw on (I-280).
        Some("no terminal")
    } else {
        None
    }
}

/// Main execution entry point (called from mod.rs).
///
/// Returns whether the two files are **out of step** — names missing from the
/// target, or (two-way) names only the template has. `--check` turns that
/// into an exit code; every other mode ignores it.
pub fn execute(ctx: &SyncCtx, config: &PlaceholderConfig) -> Result<bool> {
    if ctx.verbose {
        println!(
            "{}",
            format!(
                "Running sync in verbose mode [direction={:?}, two_way={}, dry_run={}, force={}]",
                ctx.direction, ctx.two_way, ctx.dry_run, ctx.force
            )
            .dimmed()
        );
    }

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

    crate::core::permissions::warn_if_readable(&ctx.paths.env);

    let drift = match ctx.direction {
        SyncDirection::Forward => sync_forward(ctx, config),
        SyncDirection::Reverse => sync_reverse(ctx, config),
    }?;

    // Only on success — noise-free error output.
    ui::print_docs_hint(&docs::SYNC);

    if ctx.check && drift.out_of_step() {
        print_verdict(ctx, &drift);
    }
    Ok(drift.out_of_step())
}

/// The `--check` verdict: one line per side that drifted, each naming the
/// command that fixes it (I-118).
///
/// ⚠️ It said "Run `evnx sync --direction X` and commit the result" whatever
/// the direction — but reverse writes `.env`, which is never committed.
fn print_verdict(ctx: &SyncCtx, drift: &Drift) {
    let (env, example) = (ctx.paths.env_str(), ctx.paths.example_str());
    eprintln!();
    eprintln!("✗ Out of sync.");
    let n = drift.missing.len();
    if n > 0 {
        match ctx.direction {
            SyncDirection::Forward => eprintln!(
                "  · {} in {env} {} missing from {example}: run `{}` and commit {example}.",
                count(n, "variable"),
                is_are(n),
                fix_command(&ctx.paths, SyncDirection::Forward)
            ),
            SyncDirection::Reverse => eprintln!(
                "  · {} {} missing from {env}: run `{}` to add {} to {env}.",
                count(n, "variable"),
                is_are(n),
                fix_command(&ctx.paths, SyncDirection::Reverse),
                it_them(n)
            ),
        }
    }
    let t = drift.template_only.len();
    if t > 0 {
        eprintln!(
            "  · {} in {example} {} not in {env}: remove {} from {example}, or add {} to {env} \
             with `{}`.",
            count(t, "variable"),
            is_are(t),
            it_them(t),
            it_them(t),
            fix_command(&ctx.paths, SyncDirection::Reverse)
        );
    }
}

/// The command that adds the missing side, with `--env` / `--example` when
/// they are not the defaults: `evnx sync --direction reverse` run as written
/// after `--env-name production` would fill `.env`, not `.env.production`.
pub(super) fn fix_command(paths: &SyncPaths, direction: SyncDirection) -> String {
    use crate::core::shell::word;
    let mut cmd = format!("evnx sync --direction {direction}");
    if paths.env != Path::new(".env") {
        cmd.push_str(&format!(" --env {}", word(&paths.env_str())));
    }
    if paths.example != Path::new(".env.example") {
        cmd.push_str(&format!(" --example {}", word(&paths.example_str())));
    }
    cmd
}

fn is_are(n: usize) -> &'static str {
    if n == 1 {
        "is"
    } else {
        "are"
    }
}

fn it_them(n: usize) -> &'static str {
    if n == 1 {
        "it"
    } else {
        "them"
    }
}

/// The names only the template has (two-way, DEC-S17). `sync` only adds, so
/// they are reported and left alone; under `--check` they are drift.
fn print_template_only(ctx: &SyncCtx, names: &[String]) {
    if names.is_empty() {
        return;
    }
    ui::print_box(
        &format!("Only in {}", ctx.paths.example_str()),
        &format!(
            "Found {} in {} that {} does not have:\n{}",
            count(names.len(), "variable"),
            ctx.paths.example_str(),
            ctx.paths.env_str(),
            names
                .iter()
                .map(|k| format!("  • {}", key_highlight(k)))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    if !ctx.check {
        ui::info(format!(
            "sync only adds: remove {} from {} by hand if {} stale, or add {} to {} with `{}`",
            it_them(names.len()),
            ctx.paths.example_str(),
            if names.len() == 1 {
                "it is"
            } else {
                "they are"
            },
            it_them(names.len()),
            ctx.paths.env_str(),
            fix_command(&ctx.paths, SyncDirection::Reverse)
        ));
    }
}

// ─────────────────────────────────────────────────────────────
// Forward Sync: .env → .env.example
// ─────────────────────────────────────────────────────────────

/// "File not found" for the source of a forward sync, with the way out.
fn env_not_found(p: &SyncPaths) {
    ui::error(format!("File not found: {}", p.env_str()));
    ui::print_box(
        "Getting started",
        &format!(
            "It looks like this project hasn't been initialized yet.\n\n\
             To create .env and .env.example files, run:\n\n\
             $ evnx init\n\n\
             Or if you have an existing {} file, place it in this directory \
             and try again.",
            p.env_str()
        ),
    );
    ui::print_next_steps(&[
        "Run 'evnx init' to set up a new project",
        &format!(
            "Or copy your existing {} file to this directory",
            p.env_str()
        ),
        "Then run 'evnx sync' again",
    ]);
}

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
/// command once per mistake. `--check --format json` runs it too (I-117).
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
pub(super) fn check_allow_actual(
    config: &PlaceholderConfig,
    env_vars: &IndexMap<String, String>,
    paths: &SyncPaths,
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

    let mut message = format!(
        "`allow_actual` would copy a real credential into {}, which is a \
         committed file. Nothing was written.\n",
        paths.example_str()
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

/// Say which keys were kept out of the template, and log the ones written
/// with real values (DEC-S1: the audit line always goes to stderr).
fn report_actual_values(paths: &SyncPaths, written: &[String], refused: &[(String, String)]) {
    if !refused.is_empty() {
        ui::warning(format!(
            "{} kept out of {}: {} a credential, written as {}",
            refused.len(),
            paths.example_str(),
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
    log_security_event(written, &paths.example_str());
}

/// Ask for the optional comment that heads the added block. Asked **before**
/// the write so it lands above the variables it describes — it was appended
/// after them, in a second write (I-126, O-4).
fn ask_comment() -> Result<Option<String>> {
    if !crate::utils::prompt::yes_no()
        .with_prompt("Add a comment explaining these variables?")
        .default(true)
        .interact()?
    {
        return Ok(None);
    }
    let comment: String = Input::new()
        .with_prompt("Comment")
        .default("Additional configuration".to_string())
        .interact_text()?;
    Ok(Some(comment))
}

fn sync_forward(ctx: &SyncCtx, config: &PlaceholderConfig) -> Result<Drift> {
    let paths = &ctx.paths;
    let Loaded {
        env,
        example,
        drift,
    } = load(ctx, config, true)?;
    let env_vars = env.unwrap_or_default();

    let Some(example_vars) = example else {
        // ⚠️ A preview says what *would* happen (I-281).
        ui::info(format!(
            "{} not found, {} from {}",
            paths.example_str(),
            if ctx.dry_run {
                "would create it"
            } else {
                "creating"
            },
            paths.env_str()
        ));
        handle_new_example_file(ctx, &env_vars, config)?;
        return Ok(drift);
    };

    let missing = drift.names();

    if missing.is_empty() {
        // ⚠️ "Up to date" is true but incurious when the source is empty and
        // the template is not. That is the shape of issue #15: a blank `.env`
        // beside a populated `.env.example` is far more likely a mistake — a
        // truncated file, the wrong directory, a failed secret fetch — than a
        // steady state. sync has nothing to copy, so it correctly changes
        // nothing; it should still say why it did nothing. (`--check` refuses
        // it in `load`, I-127.)
        if env_vars.is_empty() && !example_vars.is_empty() {
            ui::warning(format!(
                "{} has no variables, while {} has {}. Nothing was copied.",
                paths.env_str(),
                paths.example_str(),
                example_vars.len(),
            ));
            ui::warning_detail(format!(
                "{} If that is unexpected, check you are in the right directory. \
To fill {} from the template instead, run `{}`.",
                ui::glyph::INFO.dimmed(),
                paths.env_str(),
                fix_command(paths, SyncDirection::Reverse),
            ));
        } else if drift.template_only.is_empty() {
            ui::success(format!("{} is up to date", paths.example_str()));
        } else {
            print_template_only(ctx, &drift.template_only);
        }
        return Ok(drift);
    }

    ui::print_box(
        "Variables to Add",
        &format!(
            "Found {} in {} missing from {}:\n{}",
            count(missing.len(), "variable"),
            paths.env_str(),
            paths.example_str(),
            missing
                .iter()
                .map(|k| format!("  • {}", key_highlight(k)))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    print_template_only(ctx, &drift.template_only);

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
    // No terminal, or CI, answers the same way (DEC-S9).
    let no_prompt = no_prompt_reason();
    if ctx.force || ctx.dry_run || no_prompt.is_some() {
        if !ctx.dry_run {
            match (ctx.force, no_prompt) {
                (true, _) => ui::info("Force mode: using placeholder values and skipping prompts"),
                (false, Some(why)) => ui::info(format!("{why}: adding with placeholder values")),
                (false, None) => {}
            }
        }
        let preview = add_with_placeholders_preview(&missing, &env_vars, config)?;
        if ctx.dry_run {
            print_preview(&SyncPreview {
                target_file: target_label(&paths.example),
                action: SyncAction::Add,
                variables: preview,
                warnings: vec![],
            });
            return Ok(drift);
        }
        add_with_placeholders(paths, &missing, &env_vars, config, None)?;
        ui::success(format!(
            "Added {} to {}",
            count(missing.len(), "variable"),
            paths.example_str()
        ));
        return Ok(drift);
    }

    // Interactive mode
    let choices = vec![
        "Yes, add with placeholder values (recommended)",
        "Yes, add with actual values ! SECURITY RISK",
        "Let me choose individually",
        "No, skip",
    ];

    let selection = Select::new()
        .with_prompt(format!("Add these to {}?", paths.example_str()))
        .items(&choices)
        .default(0)
        .interact()?;

    let added = match selection {
        0 => {
            let comment = ask_comment()?;
            add_with_placeholders(paths, &missing, &env_vars, config, comment.as_deref())?;
            missing.len()
        }
        1 => {
            ui::print_box(
                "! SECURITY WARNING",
                &format!(
                    "Adding actual values to {example} may expose secrets if \
                     this file is committed to version control.\n\n\
                     Only proceed if you are certain {example} is gitignored \
                     or contains no sensitive data.",
                    example = paths.example_str()
                ),
            );

            let confirm = crate::utils::prompt::yes_no()
                .with_prompt("Are you absolutely sure you want to add ACTUAL VALUES?")
                .default(false)
                .interact()?;

            if !confirm {
                ui::info("Aborted: values not added");
                return Ok(drift);
            }

            let comment = ask_comment()?;
            add_with_actual_values(paths, &missing, &env_vars, config, comment.as_deref())?;
            let refused = credential_keys(missing.iter().copied(), &env_vars, "sync")?;
            let written: Vec<String> = missing
                .iter()
                .filter(|k| !refused.iter().any(|(r, _)| r == **k))
                .map(|k| (*k).clone())
                .collect();
            report_actual_values(paths, &written, &refused);
            missing.len()
        }
        2 => {
            let selected = MultiSelect::new()
                .with_prompt("Select variables to add")
                .items(&missing)
                .interact()?;
            if selected.is_empty() {
                ui::info("No changes made");
                return Ok(drift);
            }
            let chosen: Vec<&String> = selected.iter().map(|&i| missing[i]).collect();
            let comment = ask_comment()?;
            add_with_placeholders(paths, &chosen, &env_vars, config, comment.as_deref())?;
            chosen.len()
        }
        3 => {
            ui::info("No changes made");
            return Ok(drift);
        }
        _ => unreachable!(),
    };

    ui::success(format!(
        "Updated {} (+{})",
        paths.example_str(),
        count(added, "variable")
    ));

    let review = format!(
        "Review {} to ensure no secrets were added",
        paths.example_str()
    );
    let commit = format!(
        "Commit {} to Git (never commit {})",
        paths.example_str(),
        paths.env_str()
    );
    ui::print_next_steps(&[&review, &commit, "Share changes with your team"]);

    Ok(drift)
}

/// Create the template when it does not exist yet.
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
/// one, or with CI set, it resolves to placeholders and says so, rather than
/// silently picking the unsafe branch.
fn handle_new_example_file(
    ctx: &SyncCtx,
    vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let paths = &ctx.paths;
    // A preview never asks and never says it is creating anything (I-281).
    let use_placeholders = if ctx.force || ctx.dry_run {
        true
    } else if let Some(why) = no_prompt_reason() {
        ui::info(format!(
            "{why}: creating {} with placeholder values",
            paths.example_str()
        ));
        true
    } else {
        let choices = [
            "Placeholder values (recommended — safe to commit)".to_string(),
            format!(
                "Actual values copied from {}  ! SECURITY RISK",
                paths.env_str()
            ),
        ];
        Select::new()
            .with_prompt(format!("Create {} with", paths.example_str()))
            .items(&choices)
            .default(0)
            .interact()?
            == 0
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

    if ctx.dry_run {
        print_preview(&SyncPreview {
            target_file: target_label(&paths.example),
            action: SyncAction::Add,
            variables: preview,
            warnings: vec!["New file would be created".into()],
        });
        return Ok(());
    }

    if use_placeholders {
        convert_to_example(paths, vars, config)?;
    } else {
        // ⛔ Built from the parsed values, like every other write to the
        // template — never from `.env`'s text. That rewrote the text line by
        // line and missed what a line scan cannot see: the continuation lines
        // of a multi-line value (a PEM key's body) and `export<TAB>KEY=…`,
        // so a refused credential reached the committed file while the run
        // said it was kept out (I-277). Comments are not carried over; the
        // placeholder template never carried them either.
        let mut out = format!(
            "{}{}\n{}\n\n",
            GENERATED_HEADER[0],
            paths.env_str(),
            ACTUAL_VALUES_NOTE
        );
        for (key, value) in vars {
            let value = if is_refused(key) {
                generate_placeholder(key, Some(value), config)
            } else {
                value.clone()
            };
            out.push_str(&envfile::line(key, &value));
            out.push('\n');
        }
        write_template(&paths.example, &out)?;
        let written: Vec<String> = vars.keys().filter(|k| !is_refused(k)).cloned().collect();
        report_actual_values(paths, &written, &refused);
    }

    ui::success(format!("Created {}", paths.example_str()));
    if use_placeholders {
        let review = format!(
            "Review {} to ensure the placeholders are appropriate",
            paths.example_str()
        );
        let commit = format!(
            "Commit {} to Git (never commit {})",
            paths.example_str(),
            paths.env_str()
        );
        ui::print_next_steps(&[&review, &commit]);
    } else {
        // ⚠️ Do not tell someone to commit a file that holds their real values.
        ui::warning(format!(
            "{} holds the ACTUAL values from {}, not placeholders",
            paths.example_str(),
            paths.env_str()
        ));
        // ⚠️ `evnx sync --force` alone does nothing here: the template now
        // exists and has every name (I-278).
        let recreate = format!(
            "Or delete {} and run `evnx sync --force` to create it with placeholders",
            paths.example_str()
        );
        ui::print_next_steps(&[
            "Replace the values before committing — this file is not safe to commit as written",
            &recreate,
        ]);
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Reverse Sync: .env.example → .env
// ─────────────────────────────────────────────────────────────

/// "File not found" for the template a reverse sync reads.
fn example_not_found(p: &SyncPaths) {
    ui::error(format!("File not found: {}", p.example_str()));
    ui::print_box(
        "Getting started",
        &format!(
            "The template file {example} is missing.\n\n\
             To create it, either:\n\n\
             1. Run 'evnx init' to generate from a blueprint, OR\n\
             2. Create {example} manually with your variable names\n\n\
             Example {example}:\n\
             DATABASE_URL=your_database_url\n\
             API_KEY=your_api_key",
            example = p.example_str()
        ),
    );
    ui::print_next_steps(&[
        &format!(
            "Run 'evnx init' to generate {} from a template",
            p.example_str()
        ),
        &format!(
            "Or create {} manually with your variable names",
            p.example_str()
        ),
        "Then run 'evnx sync --direction reverse' again",
    ]);
}

fn sync_reverse(ctx: &SyncCtx, config: &PlaceholderConfig) -> Result<Drift> {
    if ctx.verbose {
        eprintln!("[DEBUG] Running reverse sync");
    }
    let paths = &ctx.paths;
    let Loaded {
        env,
        example,
        drift,
    } = load(ctx, config, true)?;
    let example_vars = example.unwrap_or_default();

    let Some(_) = env else {
        ui::info(format!(
            "{} not found, {} from {}",
            paths.env_str(),
            if ctx.dry_run {
                "would create it"
            } else {
                "creating"
            },
            paths.example_str()
        ));
        handle_new_env_file(ctx, &example_vars, &drift.spec_names(), config)?;
        return Ok(drift);
    };

    let missing = drift.names();
    if missing.is_empty() {
        ui::success(format!("{} is up to date", paths.env_str()));
        return Ok(drift);
    }

    // ⚠️ Each name says where it is declared. They were all listed as "in
    // .env.example", including names only `[vars]` requires (I-255).
    let spec_file = crate::core::config::PROJECT_FILE;
    ui::print_box(
        "Missing Variables",
        &format!(
            "Found {} missing from {}:\n{}",
            count(missing.len(), "variable"),
            paths.env_str(),
            drift
                .missing
                .iter()
                .map(|(k, origin)| match origin {
                    Origin::Source => format!("  • {}", key_highlight(k)),
                    Origin::Spec => {
                        format!(
                            "  • {}  (required by [vars] in {spec_file})",
                            key_highlight(k)
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );

    // `--dry-run` implies non-interactive — see the note in `sync_forward`.
    let no_prompt = no_prompt_reason();
    if ctx.force || ctx.dry_run || no_prompt.is_some() {
        if !ctx.dry_run {
            match (ctx.force, no_prompt) {
                (true, _) => ui::info(
                    "Force mode: adding the template's values (placeholders where it has none), skipping prompts",
                ),
                (false, Some(why)) => ui::info(format!(
                    "{why}: adding the template's values (placeholders where it has none)"
                )),
                (false, None) => {}
            }
        }
        let preview = add_from_example_preview(&missing, &example_vars, true, config)?;
        if ctx.dry_run {
            print_preview(&SyncPreview {
                target_file: target_label(&paths.env),
                action: SyncAction::Add,
                variables: preview,
                warnings: placeholder_reminder(&missing, &example_vars, config),
            });
            return Ok(drift);
        }
        add_from_example(paths, &missing, &example_vars, true, config)?;
        ui::success(format!(
            "Added {} to {}",
            count(missing.len(), "variable"),
            paths.env_str()
        ));
        return Ok(drift);
    }

    let choices = vec![
        "Yes, with the template's values (placeholders where it has none)",
        "Yes, prompt me for real values",
        "Let me choose individually",
        "No, skip",
    ];

    let selection = Select::new()
        .with_prompt(format!("Add to {}?", paths.env_str()))
        .items(&choices)
        .default(0)
        .interact()?;

    let use_placeholders = match selection {
        0 => true,
        1 => false,
        2 => {
            if add_from_example_interactive(paths, &missing, &example_vars, config)? {
                ui::success(format!("Added selected variables to {}", paths.env_str()));
            }
            return Ok(drift);
        }
        3 => {
            ui::info("No changes made");
            return Ok(drift);
        }
        _ => unreachable!(),
    };

    add_from_example(paths, &missing, &example_vars, use_placeholders, config)?;
    ui::success(format!(
        "Added {} to {}",
        count(missing.len(), "variable"),
        paths.env_str()
    ));

    if use_placeholders && !placeholder_reminder(&missing, &example_vars, config).is_empty() {
        ui::warning("Remember to replace placeholder values with real credentials!");
    }

    Ok(drift)
}

/// The reminder to replace placeholders, only when something added is one
/// (I-126: it was printed for a preview of real template values too).
fn placeholder_reminder(
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Vec<String> {
    if keys
        .iter()
        .any(|k| reverse_value(k, example_vars.get(*k), config).1)
    {
        vec!["Remember to replace placeholders with real values".into()]
    } else {
        Vec::new()
    }
}

/// The comment lines sync itself writes at the top of a template it
/// generates. True for that template; false for a `.env` copied from it.
const GENERATED_HEADER: &[&str] = &[
    "# Generated from ",
    "# Replace all placeholder values with real credentials",
];

/// The second header line of a template created with actual values (I-277).
const ACTUAL_VALUES_NOTE: &str = "# ! ACTUAL VALUES: replace them before committing this file";

/// The template's text for a new `.env`: sync's own generated header replaced
/// by one that is true of the new file (I-129, DEC-S20). Everything else —
/// comments, order, blank lines — is kept.
fn env_text_from_template(template: &str, example_label: &str, env_label: &str) -> String {
    // ⚠️ Replaced when it is sync's own header — "# Generated from <this
    // .env>", which is false of the `.env` being created (I-129), or the two
    // lines sync writes — and kept otherwise: a first line of the user's own,
    // "# Generated from terraform output", is not sync's to drop (I-283).
    let mut lines = template.split_inclusive('\n').peekable();
    let Some(first) = lines.peek().map(|l| l.trim_end().to_string()) else {
        return template.to_string();
    };
    let names_this_env = first == format!("{}{env_label}", GENERATED_HEADER[0]);
    let mut rest = lines.clone();
    rest.next();
    let sync_note = rest
        .peek()
        .map(|l| l.trim_end())
        .is_some_and(|l| l == GENERATED_HEADER[1] || l == ACTUAL_VALUES_NOTE);
    if !(names_this_env || first.starts_with(GENERATED_HEADER[0]) && sync_note) {
        return template.to_string();
    }
    lines.next();
    if sync_note {
        lines.next();
    }
    let mut out = format!("# Created from {example_label} by evnx sync\n");
    out.extend(lines);
    out
}

/// Create `.env` from the template (reverse, no `.env` yet).
///
/// DEC-S20: the template's text — comments and order kept, its own values —
/// with a generated header made true for the new file (I-129), and the
/// spec's required names it lacks appended (I-275), so the new file is the
/// one `--check` said it would be.
fn handle_new_env_file(
    ctx: &SyncCtx,
    example_vars: &IndexMap<String, String>,
    spec_names: &[String],
    config: &PlaceholderConfig,
) -> Result<()> {
    let paths = &ctx.paths;
    if ctx.dry_run {
        let mut preview: Vec<VarChange> = example_vars
            .iter()
            .map(|(k, v)| VarChange {
                key: k.clone(),
                old_value: None,
                new_value: v.clone(),
                is_placeholder: is_generated_value(v, config),
            })
            .collect();
        preview.extend(spec_names.iter().map(|k| {
            let (value, generated) = reverse_value(k, None, config);
            VarChange {
                key: k.clone(),
                old_value: None,
                new_value: value,
                is_placeholder: generated,
            }
        }));
        print_preview(&SyncPreview {
            target_file: target_label(&paths.env),
            action: SyncAction::Add,
            variables: preview,
            warnings: vec!["Replace placeholder values with real credentials!".into()],
        });
        return Ok(());
    }

    let template = fs::read_to_string(&paths.example)?;
    let mut content = env_text_from_template(&template, &paths.example_str(), &paths.env_str());
    if !spec_names.is_empty() {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!(
            "\n# Added by evnx sync: required by [vars] in {}\n",
            crate::core::config::PROJECT_FILE
        ));
        for key in spec_names {
            content.push_str(&envfile::line(key, &reverse_value(key, None, config).0));
            content.push('\n');
        }
    }
    write_env(&paths.env, &content)?;
    ui::success(format!("Created {}", paths.env_str()));
    ui::warning("Replace placeholder values with real credentials!");
    let edit = format!(
        "Edit {} and replace placeholder values with real credentials",
        paths.env_str()
    );
    let never = format!("Never commit {} to version control", paths.env_str());
    ui::print_next_steps(&[&edit, &never]);
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Add Functions (Forward Sync Helpers)
// ─────────────────────────────────────────────────────────────

/// The heading of a block sync appends: where it came from, then the user's
/// comment when there is one — above the variables it describes (I-126).
fn block_heading(source: &str, marker: &str, comment: Option<&str>) -> String {
    let mut heading = format!("\n# Synced from {source}{marker}\n");
    if let Some(c) = comment {
        heading.push_str(&format!("# {c}\n"));
    }
    heading
}

fn add_with_placeholders(
    paths: &SyncPaths,
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
    comment: Option<&str>,
) -> Result<()> {
    let mut content = fs::read_to_string(&paths.example)
        .with_context(|| format!("Failed to read {}", paths.example_str()))?;
    content.push_str(&block_heading(&paths.env_str(), "", comment));

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
    comment: Option<&str>,
) -> Result<()> {
    let refused = credential_keys(keys.iter().copied(), values, "sync")?;
    let mut content = fs::read_to_string(&paths.example)?;
    content.push_str(&block_heading(
        &paths.env_str(),
        " [! ACTUAL VALUES]",
        comment,
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

fn convert_to_example(
    paths: &SyncPaths,
    vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let mut content = String::new();
    content.push_str(&format!("{}{}\n", GENERATED_HEADER[0], paths.env_str()));
    content.push_str(&format!("{}\n\n", GENERATED_HEADER[1]));

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

/// Returns whether anything was chosen (and written).
fn add_from_example_interactive(
    paths: &SyncPaths,
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<bool> {
    let selected = MultiSelect::new()
        .with_prompt("Select variables to add")
        .items(keys)
        .interact()?;
    if selected.is_empty() {
        // As forward does: nothing chosen, nothing written — not an empty
        // heading (I-283).
        ui::info("No changes made");
        return Ok(false);
    }

    let mut content = fs::read_to_string(&paths.env).unwrap_or_default();
    // Same heading as the other reverse writes, for the same reason.
    content.push_str("\n# Added by evnx sync\n");

    for &idx in &selected {
        let key = keys[idx];
        let (value, _) = reverse_value(key, example_vars.get(key), config);
        content.push_str(&envfile::line(key, &value));
        content.push('\n');
    }

    write_env(&paths.env, &content)?;
    Ok(true)
}

// ─────────────────────────────────────────────────────────────
// Preview Output Helpers
// ─────────────────────────────────────────────────────────────

/// The file a write to `path` would change, as the preview names it: through
/// a symlink, the link and its target — worked out by `safe_write::preview`,
/// the rule the write itself follows (R-template).
fn target_label(path: &Path) -> String {
    match crate::core::safe_write::preview(path) {
        Ok(p) if p.through_link => format!("{} -> {}", path.display(), p.target.display()),
        _ => path.display().to_string(),
    }
}

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
