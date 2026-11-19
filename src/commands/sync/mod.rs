//! Sync command: keep `.env` and `.env.example` in sync.
//!
//! This module provides the public API for the sync command.
//! Internal implementation is organized into submodules for maintainability.

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::cli::{NamingPolicy, SyncDirection};
use crate::core::output::{Mode, Out};
use crate::core::verdict::Verdict;
use crate::utils::ui;

mod executor;
mod models;
mod security;

// Re-export public types for external use (if needed)
// `PlaceholderConfig` lives in `core::placeholder` since 0.10.12 (DEC-S19);
// re-exported so `evnx::commands::sync::PlaceholderConfig` keeps working.
pub use crate::core::placeholder::PlaceholderConfig;
pub use executor::VarChange;
pub use models::{PreviewRow, SyncAction, SyncPreview};

/// What `evnx sync` was asked for.
pub struct SyncOptions {
    pub env: String,
    pub example: String,
    // ⚠️ `Option` since 0.10.12 (DEC-S17): whether a direction was *given*
    // decides how far `--check` looks. `None` is forward, looking both ways.
    pub direction: Option<SyncDirection>,
    pub placeholder: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub force: bool,
    pub check: bool,
    pub format: String,
    pub template_config: Option<PathBuf>,
    pub naming_policy: NamingPolicy,
    pub spec: crate::core::spec::Spec,
    pub skip_without_env: bool,
}

/// Public entry point called from main.rs.
///
/// # `--check` and its three exit codes
///
/// | code | meaning |
/// |------|---------|
/// | `0`  | in sync |
/// | `1`  | **out of sync** — the assertion failed |
/// | `2`  | **error** — missing file, parse failure, bad config |
///
/// The three states exist because two of them were previously indistinguishable.
/// `--check` returned `1` for "your template is stale" *and* for "`.env` is
/// missing" *and* for "the template will not parse", so a CI gate could fail the
/// build without being able to say which had happened — and
/// `evnx sync --check || true`, the documented advisory idiom, swallowed genuine
/// breakage along with the signal it meant to ignore.
///
/// `0 / 1 / 2` is the POSIX shape, and the reason `diff` and `grep` both use it:
///
/// ```text
/// diff   same → 0    different → 1    trouble → 2
/// grep   match → 0   no match  → 1    trouble → 2
/// ```
///
/// ⚠️ **Drift is reported through the exit code only under `--check`.** Plain
/// `sync` exits `0` whatever it found, and `--dry-run` previews and exits `0`.
/// A failure is `2` with the "No verdict" line in every mode (DEC-S8, U4).
///
/// # How far `--check` looks (DEC-S3, DEC-S17)
///
/// Without `--direction` it is two-way: a name only the template has is drift
/// too — the check a developer runs on their own pair. A named direction makes
/// it one-way (only names the target lacks), which is how CI and the shipped
/// pre-commit hook ask, so a teammate whose `.env` has not caught up with the
/// template can still commit.
///
/// # Why `--dry-run` is kept
///
/// `--check` and `--dry-run` print the same preview and differ in the exit code
/// and the verdict lines, so the case for dropping one is real. Two things keep it:
/// `--dry-run` has shipped since before v0.4.0 and appears throughout the guides;
/// and `evnx sync --check || true` cannot distinguish `1` from `2`, so a user who
/// only wants the preview needs a flag that never reports state through the exit
/// code at all.
pub fn run(options: SyncOptions) -> Result<Verdict> {
    let SyncOptions {
        env,
        example,
        direction,
        placeholder,
        verbose,
        dry_run,
        force,
        check,
        format,
        template_config,
        naming_policy,
        spec,
        skip_without_env,
    } = options;
    let two_way = direction.is_none();
    let direction = direction.unwrap_or_default();
    let paths = executor::SyncPaths {
        env: PathBuf::from(env),
        example: PathBuf::from(example),
    };

    // DEC-U8-5: the skip exists for a forward run on a clone with no `.env`.
    // Reverse *creates* `.env`; a skip there would hide that it never did.
    if skip_without_env && direction == SyncDirection::Reverse {
        anyhow::bail!(
            "--skip-without-env applies to --direction forward only: reverse creates {} from {}",
            paths.env_str(),
            paths.example_str()
        );
    }

    // Before every branch, so a bad `--template-config` is exit 2 with or
    // without a `.env` — the check below returned before it was read (I-279).
    let config = executor::load_config(template_config.as_ref())?;

    // ⚠️ No `.env` is the normal state of CI and of a fresh clone, so it is
    // answered here rather than as "file not found" (DEC-S4, I-128).
    if direction == SyncDirection::Forward && !paths.env.exists() {
        if check {
            return check_without_env(&paths, &spec, skip_without_env, format == "json");
        }
        if skip_without_env {
            say_skipped(&paths);
            return Ok(Verdict::Clean);
        }
    }

    // `placeholder` is not a command-line flag any more (removed in 0.15.0,
    // with `-f/--force`: `--yes` is the one spelling); the argument stays so
    // that a direct caller of this library function keeps compiling.
    let ctx = executor::SyncCtx {
        paths,
        direction,
        two_way,
        verbose,
        // `--check` is a dry run that reports its verdict through the exit code.
        dry_run: dry_run || check,
        check,
        force: force || placeholder,
        json: format == "json",
        naming_policy,
        spec,
    };

    // ⚠️ `--format json` works in every mode (SYNC-2). It reads the same drift
    // the pretty output prints (I-117): it was a second implementation that
    // left out the spec, the naming policy and the `allow_actual` guard, and
    // gave a missing `.env` in reverse a different exit code. An `Err` is exit
    // 2 with the "No verdict" line, from `main`.
    let out_of_step = if format == "json" && !check {
        return run_as_json(&ctx, &config);
    } else if format == "json" {
        check_as_json(&ctx, &config)?
    } else {
        executor::execute(&ctx, &config)?
    };
    Ok(if check && out_of_step {
        Verdict::Findings
    } else {
        Verdict::Clean
    })
}

// ─────────────────────────────────────────────────────────────
// Tests (Only if you want to test public API here)
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // SyncDirection is defined in cli.rs, so test it there instead
    // But if you want to verify the run() function signature compiles:

    /// The public entry point takes a named struct: a new or reordered option
    /// is a compile error here, and every field is named at the call site.
    #[test]
    fn test_run_options_compile() {
        let _options = SyncOptions {
            env: String::new(),
            example: String::new(),
            direction: None, // DEC-S17: given or not
            placeholder: false,
            verbose: false,
            dry_run: false,
            force: false,
            check: false,
            format: String::new(),
            template_config: None,
            naming_policy: crate::cli::NamingPolicy::Warn,
            spec: crate::core::spec::Spec::default(),
            skip_without_env: false, // U8d, DEC-U8-5
        };
        let _func: fn(SyncOptions) -> anyhow::Result<Verdict> = run;
    }
}

// ─────────────────────────────────────────────────────────────
// Machine-readable --check
// ─────────────────────────────────────────────────────────────

/// What `--check --format json` reports.
///
/// The shape answers the question a CI gate asks — *what* drifted, not just that
/// something did.
#[derive(Debug, serde::Serialize)]
pub(super) struct SyncReport {
    in_sync: bool,
    direction: &'static str,
    env: String,
    example: String,
    /// Keys the target is missing, sorted: the order the docs promise and a
    /// CI diff needs (RM-11; 0.10.12 to 0.10.18 printed source order).
    missing: Vec<String>,
    /// Without `--direction` (two-way, DEC-S3, DEC-S17): names only the
    /// template has. Absent from a one-way report, so its presence means it
    /// was checked. Sorted, like `missing`.
    #[serde(skip_serializing_if = "Option::is_none")]
    template_only: Option<Vec<String>>,
    summary: CheckSummary,
    /// A run that writes, or with `--dry-run` would write: the names added to
    /// the target, sorted. Absent under `--check`, which writes nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    added: Option<Vec<String>>,
    /// `"spec"` when there was no `.env` and the template was checked against
    /// the required `[vars]` instead (DEC-S4). Absent otherwise, so the report
    /// for the usual comparison is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    compared_with: Option<&'static str>,
    /// What the text needs and the JSON does not carry.
    #[serde(skip)]
    text: executor::SyncText,
}

#[derive(Debug, serde::Serialize)]
struct CheckSummary {
    missing: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    template_only: Option<usize>,
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// Print the drift as JSON. Returns whether the files are out of step.
///
/// ⚠️ The drift comes from [`executor::load`], the computation the pretty
/// output uses, after the same refusals: an unreadable file, a credential in
/// `allow_actual`, a name `--naming-policy error` refuses. Each is exit 2 with
/// nothing on stdout.
fn check_as_json(
    ctx: &executor::SyncCtx,
    config: &crate::core::placeholder::PlaceholderConfig,
) -> Result<bool> {
    let loaded = executor::load(ctx, config, false)?;
    let report = report_of(ctx, &loaded.drift);
    // stdout, and nothing else on it — the property the golden tests pin.
    Out::new(Mode::Json).emit(&report)?;
    Ok(loaded.drift.out_of_step())
}

/// `--format json` without `--check` (SYNC-2): the `--check` report, plus
/// `added`. `in_sync`, `missing` and `template_only` describe the files as they
/// were found, before any write; `added` is what was written, or with
/// `--dry-run` what would be.
///
/// ⚠️ stdout holds the document and nothing else: the status lines are
/// suppressed, and a run that would stop to ask is refused (as `init --json`
/// is) rather than asking into the middle of it. The document is printed only
/// after the write succeeded, so an `Err` leaves stdout empty.
fn run_as_json(
    ctx: &executor::SyncCtx,
    config: &crate::core::placeholder::PlaceholderConfig,
) -> Result<Verdict> {
    if !ctx.dry_run && !ctx.force && executor::no_prompt_reason().is_none() {
        return Err(crate::core::verdict::usage(
            "--format json needs --yes or --dry-run: it never asks.",
        ));
    }
    ui::set_quiet(true);
    let loaded = executor::load(ctx, config, false)?;
    let mut report = report_of(ctx, &loaded.drift);
    report.added = Some(report.missing.clone());
    if !ctx.dry_run {
        executor::execute(ctx, config)?;
    }
    Out::new(Mode::Json).emit(&report)?;
    Ok(Verdict::Clean)
}

fn report_of(ctx: &executor::SyncCtx, drift: &executor::Drift) -> SyncReport {
    let template_only = (ctx.two_way && ctx.direction == SyncDirection::Forward)
        .then(|| sorted(drift.template_only.clone()));
    SyncReport {
        in_sync: !drift.out_of_step(),
        direction: match ctx.direction {
            SyncDirection::Forward => "forward",
            SyncDirection::Reverse => "reverse",
        },
        env: ctx.paths.env_str(),
        example: ctx.paths.example_str(),
        missing: sorted(drift.missing.iter().map(|(k, _)| k.clone()).collect()),
        summary: CheckSummary {
            missing: drift.missing.len(),
            template_only: template_only.as_ref().map(Vec::len),
        },
        template_only,
        added: None,
        compared_with: None,
        text: executor::SyncText {
            paths: ctx.paths.clone(),
            direction: ctx.direction,
            missing: drift.missing.len(),
            template_only: drift.template_only.len(),
            required: 0,
            names: Vec::new(),
        },
    }
}

// ─────────────────────────────────────────────────────────────
// No `.env`: CI and a fresh clone (DEC-S4, DEC-U8-5)
// ─────────────────────────────────────────────────────────────

/// The skip's one line, on stderr: a skipped check must not read as a pass.
fn say_skipped(paths: &executor::SyncPaths) {
    crate::core::verdict::skipped(&format!(
        "{} not found and no required [vars] to check {} against",
        paths.env_str(),
        paths.example_str()
    ));
}

/// Forward `--check` with no `.env`.
///
/// DEC-S4: the template is checked against the spec — every required
/// `[vars]` name that applies to this file must be declared in it. Without a
/// spec there is nothing to compare: exit 2, or the announced skip when the
/// caller asked for one (a pre-commit hook, DEC-U8-5).
///
/// ⚠️ Before this, CI — which has no `.env` — got "file not found", exit 2,
/// on every run, so `sync --check` could not be used as the gate it is
/// documented as.
fn check_without_env(
    paths: &executor::SyncPaths,
    spec: &crate::core::spec::Spec,
    skip_without_env: bool,
    json: bool,
) -> Result<Verdict> {
    use crate::core::Parser;

    let env_path = paths.env_str();
    let label = crate::core::env_name::name_of(&env_path);
    let required = spec
        .values()
        .filter(|v| v.is_required() && v.applies_to(label))
        .count();

    // FLD-6: `.env` is missing but `.env.prod` is here — the likely meaning.
    if label.is_none() && !skip_without_env {
        let dir = paths.env.parent().filter(|d| !d.as_os_str().is_empty());
        let dir = dir.unwrap_or(std::path::Path::new("."));
        if crate::core::resolve::other_env_files(dir, None).is_some() {
            anyhow::bail!(
                "{}",
                crate::core::resolve::not_found(dir, &env_path, "sync --check")
            );
        }
    }

    if required == 0 {
        if skip_without_env {
            say_skipped(paths);
            return Ok(Verdict::Clean);
        }
        anyhow::bail!(
            "nothing to compare: {env} does not exist, and .evnx.toml declares no required \
             [vars] to check {example} against.\n\n\
             Create {env} (`{create}`), declare the contract with \
             `evnx spec init`, or — in a pre-commit hook, where a missing {env} is expected — \
             pass --skip-without-env.",
            env = env_path,
            example = paths.example_str(),
            create = executor::fix_command(paths, SyncDirection::Reverse)
        );
    }

    // A missing template declares nothing, so every required name is missing.
    let example_vars = if paths.example.exists() {
        Parser::default()
            .parse_file(&paths.example)
            .with_context(|| format!("Failed to parse {}", paths.example_str()))?
            .vars
    } else {
        Default::default()
    };
    let missing: Vec<String> =
        crate::core::spec::missing_required(spec, |k| example_vars.contains_key(k), label)
            .into_iter()
            .map(String::from)
            .collect();

    let report = SyncReport {
        in_sync: missing.is_empty(),
        direction: "forward",
        env: env_path.clone(),
        example: paths.example_str(),
        summary: CheckSummary {
            missing: missing.len(),
            template_only: None,
        },
        missing: sorted(missing.clone()),
        template_only: None,
        added: None,
        compared_with: Some("spec"),
        text: executor::SyncText {
            paths: paths.clone(),
            direction: SyncDirection::Forward,
            missing: missing.len(),
            template_only: 0,
            required,
            names: missing.clone(),
        },
    };
    if json {
        Out::new(Mode::Json).emit(&report)?;
    } else {
        ui::print_header(
            "evnx sync",
            Some(&format!(
                "{} ↔ {}  (forward)",
                env_path,
                paths.example_str()
            )),
        );
        ui::info(format!(
            "{env_path} not found: checking {} against the contract ([vars] in .evnx.toml)",
            paths.example_str()
        ));
        Out::new(Mode::Text).emit(&report)?;
    }

    Ok(if missing.is_empty() {
        Verdict::Clean
    } else {
        Verdict::Findings
    })
}
