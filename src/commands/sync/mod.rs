//! Sync command: keep `.env` and `.env.example` in sync.
//!
//! This module provides the public API for the sync command.
//! Internal implementation is organized into submodules for maintainability.

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::cli::{NamingPolicy, SyncDirection};
use crate::core::verdict::{self, Verdict};
use crate::utils::ui;

mod executor;
mod models;
mod security;

// Re-export public types for external use (if needed)
// `PlaceholderConfig` lives in `core::placeholder` since 0.10.12 (DEC-S19);
// re-exported so `evnx::commands::sync::PlaceholderConfig` keeps working.
pub use crate::core::placeholder::PlaceholderConfig;
pub use models::{SyncAction, SyncPreview, VarChange};

/// In sync — nothing to do.
pub const EXIT_IN_SYNC: i32 = verdict::EXIT_OK;
/// `--check` only: the files are out of step.
pub const EXIT_OUT_OF_SYNC: i32 = verdict::EXIT_FINDINGS;
/// `--check` only: the command could not reach a verdict.
pub const EXIT_ERROR: i32 = verdict::EXIT_ERROR;

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
// 8 flags. `SyncArgs` in `cli.rs` already holds exactly this set, so the tidier
// signature is to pass that struct — but it would rewrite every call site for no
// behavioural gain, so it is queued for the v0.5.0 command review instead.
// `validate::run` carries the same allow for the same reason.
#[allow(clippy::too_many_arguments)]
pub fn run(
    env: String,
    example: String,
    // ⚠️ `Option` since 0.10.12 (DEC-S17): whether a direction was *given*
    // decides how far `--check` looks. `None` is forward, looking both ways.
    direction: Option<SyncDirection>,
    placeholder: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    check: bool,
    format: String,
    template_config: Option<PathBuf>,
    naming_policy: NamingPolicy,
    spec: crate::core::spec::Spec,
    skip_without_env: bool,
) -> Result<Verdict> {
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

    // DEC-S9: kept so scripts and the old hook entry keep working.
    if placeholder {
        ui::warning("--placeholder is deprecated: it does what --force does (placeholder values, no prompt). Use --force.");
    }

    let ctx = executor::SyncCtx {
        paths,
        direction,
        two_way,
        verbose,
        // `--check` is a dry run that reports its verdict through the exit code.
        dry_run: dry_run || check,
        check,
        force: force || placeholder,
        naming_policy,
        spec,
    };

    // ⚠️ `--format json` is accepted only with `--check` (clap enforces it).
    // It reads the same drift the pretty output prints (I-117): it was a
    // second implementation that left out the spec, the naming policy and the
    // `allow_actual` guard, and gave a missing `.env` in reverse a different
    // exit code. An `Err` is exit 2 with the "No verdict" line, from `main`.
    let out_of_step = if format == "json" {
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

    /// Pins the public signature so a change to it has to be deliberate.
    ///
    /// ⚠️ Updated 2026-10-20 (R-sync, DEC-S17): `direction` is an `Option`,
    /// because whether a direction was given decides how far `--check` looks.
    ///
    /// ⚠️ Updated 2026-10-17 (U8d): `skip_without_env` appended, for the
    /// pre-commit hook on a clone with no `.env` (DEC-U8-5).
    ///
    /// ⚠️ Updated again for the spec: `[vars]` is now a source for the reverse
    /// direction, so `run` takes it. This pin is the reason that change had to
    /// be deliberate rather than discovered by a library consumer.
    ///
    /// ⚠️ Updated 2026-09-21 again: `env` and `example` lead the list. `sync` was
    /// the only command that could not be pointed at a file, so the two paths it
    /// operates on are now arguments rather than 37 literals in the executor.
    ///
    /// ⚠️ Updated 2026-09-21 twice. First `check: bool` was inserted after
    /// `force`; then `format: String` after `check`. `evnx` is published as a
    /// library as well as a binary, so each is a breaking change for a direct
    /// caller — acceptable in 0.x, and the whole point of this pin is that it
    /// cannot happen by accident. Six `bool`s in a row is also why the argument
    /// list should become `SyncArgs`; see the note on `run`.
    // ⚠️ `allow`, not a type alias. The complex type *is* the test: writing the
    // signature out in full, one commented argument per line, is what makes a
    // reordering visible in the diff. Hiding it behind an alias would move the
    // thing being pinned somewhere the reviewer does not look.
    #[allow(clippy::type_complexity)]
    #[test]
    fn test_run_signature_compiles() {
        let _func: fn(
            String,                            // env
            String,                            // example
            Option<crate::cli::SyncDirection>, // DEC-S17: given or not
            bool,                              // placeholder
            bool,                              // verbose
            bool,                              // dry_run
            bool,                              // force
            bool,                              // check
            String,                            // format
            Option<std::path::PathBuf>,
            crate::cli::NamingPolicy,
            crate::core::spec::Spec,
            bool, // skip_without_env (U8d, DEC-U8-5)
        ) -> anyhow::Result<Verdict> = run;
    }
}

// ─────────────────────────────────────────────────────────────
// Machine-readable --check
// ─────────────────────────────────────────────────────────────

/// What `--check --format json` reports.
///
/// The shape answers the question a CI gate asks — *what* drifted, not just that
/// something did.
#[derive(serde::Serialize)]
struct CheckReport {
    in_sync: bool,
    direction: &'static str,
    env: String,
    example: String,
    /// Keys the target is missing, in the source's order (the template's
    /// first, then names only the spec declares, in reverse).
    missing: Vec<String>,
    /// Without `--direction` (two-way, DEC-S3, DEC-S17): names only the
    /// template has. Absent from a one-way report, so its presence means it
    /// was checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    template_only: Option<Vec<String>>,
    summary: CheckSummary,
    /// `"spec"` when there was no `.env` and the template was checked against
    /// the required `[vars]` instead (DEC-S4). Absent otherwise, so the report
    /// for the usual comparison is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    compared_with: Option<&'static str>,
}

#[derive(serde::Serialize)]
struct CheckSummary {
    missing: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    template_only: Option<usize>,
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
    let drift = &loaded.drift;
    let template_only = (ctx.two_way && ctx.direction == SyncDirection::Forward)
        .then(|| drift.template_only.clone());
    let report = CheckReport {
        in_sync: !drift.out_of_step(),
        direction: match ctx.direction {
            SyncDirection::Forward => "forward",
            SyncDirection::Reverse => "reverse",
        },
        env: ctx.paths.env_str(),
        example: ctx.paths.example_str(),
        missing: drift.missing.iter().map(|(k, _)| k.clone()).collect(),
        summary: CheckSummary {
            missing: drift.missing.len(),
            template_only: template_only.as_ref().map(Vec::len),
        },
        template_only,
        compared_with: None,
    };

    // stdout, and nothing else on it — the property the golden tests pin.
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(drift.out_of_step())
}

// ─────────────────────────────────────────────────────────────
// No `.env`: CI and a fresh clone (DEC-S4, DEC-U8-5)
// ─────────────────────────────────────────────────────────────

/// The skip's one line, on stderr: a skipped check must not read as a pass.
fn say_skipped(paths: &executor::SyncPaths) {
    eprintln!(
        "· {} not found and no required [vars] to check {} against — nothing to check, skipped",
        paths.env_str(),
        paths.example_str()
    );
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

    if json {
        let report = CheckReport {
            in_sync: missing.is_empty(),
            direction: "forward",
            env: env_path.clone(),
            example: paths.example_str(),
            summary: CheckSummary {
                missing: missing.len(),
                template_only: None,
            },
            missing: missing.clone(),
            template_only: None,
            compared_with: Some("spec"),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
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
            "{env_path} not found: checking {} against the spec ([vars] in .evnx.toml)",
            paths.example_str()
        ));
        if missing.is_empty() {
            ui::success(format!(
                "{} declares every required variable ({required})",
                paths.example_str()
            ));
            ui::print_docs_hint(&crate::docs::SYNC);
        } else {
            ui::error(format!(
                "{} is missing {} required variable(s):",
                paths.example_str(),
                missing.len()
            ));
            for name in &missing {
                println!("  • {name}");
            }
            eprintln!();
            eprintln!(
                "✗ Out of sync. Add them to {} and commit the result.",
                paths.example_str()
            );
        }
    }

    Ok(if missing.is_empty() {
        Verdict::Clean
    } else {
        Verdict::Findings
    })
}
