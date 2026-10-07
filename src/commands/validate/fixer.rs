//! Auto-fix logic and file operations for validation
//!
//! This module handles:
//! - Suggesting fixes for common issues
//! - Applying fixes to in-memory env vars
//! - Writing fixed content back to .env files

// use std::collections::HashMap;
use indexmap::IndexMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::types::{FixApplied, IssueType};

// ─────────────────────────────────────────────────────────────
// Fix Action Types
// ─────────────────────────────────────────────────────────────

/// Whether `--fix` may replace a weak secret that holds a **real value**.
///
/// ⛔ A `bool` parameter here would read `suggest_fix(k, v, &ty, false)` at the
/// call site, where `false` says nothing about what it permits. This is the one
/// decision in the module that destroys data when it goes the wrong way, so it
/// is named at every call site rather than positional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeakSecretPolicy {
    /// Placeholders only — a value nobody has filled in yet. The default.
    PlaceholdersOnly,
    /// `--rotate-weak-secrets`: replace a real credential too, because the
    /// person running it said to.
    Rotate,
}

#[derive(Debug, Clone)]
pub enum FixAction {
    GenerateSecret,
    ReplacePlaceholder(String),
    FixBoolean(String),
    AddMissing(String),
    Skip,
}

impl FixAction {
    /// Would applying this actually change anything for the better?
    ///
    /// Drives `Issue::auto_fixable`, so the report never offers `--fix` for
    /// something `--fix` will decline to touch.
    pub fn is_actionable(&self) -> bool {
        !matches!(self, FixAction::Skip)
    }
}

// ─────────────────────────────────────────────────────────────
// Fix Suggestion Logic
// ─────────────────────────────────────────────────────────────

pub fn suggest_fix(
    key: &str,
    value: &str,
    issue_type: &IssueType,
    weak_secrets: WeakSecretPolicy,
) -> FixAction {
    match issue_type {
        IssueType::PlaceholderValue => {
            // ⚠️ The same rule the weak-secret check uses, so `DB_PASSWORD`
            // and `API_TOKEN` are treated like `SECRET_KEY` rather than falling
            // through to "evnx cannot invent this".
            if super::checks::is_secret_shaped(key) {
                FixAction::GenerateSecret
            } else if key.contains("URL") {
                FixAction::ReplacePlaceholder("https://example.com".to_string())
            } else if key.contains("EMAIL") || key.contains("MAIL") {
                FixAction::ReplacePlaceholder("user@example.com".to_string())
            } else if key.contains("PORT") {
                FixAction::ReplacePlaceholder("8080".to_string())
            } else {
                // ⚠️ Not `your_value_here`. For a secret, a URL, an email or a
                // port there is a right answer evnx can supply; for `DB_NAME`
                // there is not, and substituting one placeholder for a *less
                // informative* one is a no-op that reports itself as a repair:
                //
                //   • DB_NAME: "your_db_name_value" → your_value_here
                //   ✗  DB_NAME looks like a placeholder
                //
                // It also discards the only hint the line carried about what
                // the variable is for.
                FixAction::Skip
            }
        }
        IssueType::BooleanTrap => {
            let fixed = if value.eq_ignore_ascii_case("true") {
                "true"
            } else {
                "false"
            };
            FixAction::FixBoolean(fixed.to_string())
        }
        // ⛔ **Only when the value is a placeholder.**
        //
        // Until 2026-10-07 this was an unconditional `GenerateSecret`, so
        // `--fix` replaced *working credentials* with random hex:
        //
        // ```text
        // DATABASE_PASSWORD="p@ss word#1"   →   DATABASE_PASSWORD=ad0da713…c508
        //   • DATABASE_PASSWORD: "p@ss word#1" → ad0da713…
        //   ✓ All checks passed
        // ```
        //
        // The check behind it is a *heuristic about length and wording*. It
        // cannot tell "this is a placeholder nobody has filled in" from "this
        // is the real password, and it is short" — and the second is the
        // common case, because a short password is exactly what trips it.
        //
        // Generating over a placeholder is a repair. Generating over a value
        // is destruction of the one thing on the machine that cannot be
        // recomputed, performed by a command whose name says *validate*.
        //
        // The finding still fires; it simply stops claiming to be fixable, and
        // keeps the advice it already carried ("Run: openssl rand -hex 32"),
        // which the person can act on with the old value still in front of
        // them.
        IssueType::WeakSecret => {
            if super::checks::is_placeholder(value) || weak_secrets == WeakSecretPolicy::Rotate {
                FixAction::GenerateSecret
            } else {
                FixAction::Skip
            }
        }
        IssueType::MissingVariable => {
            let default = if super::checks::is_secret_shaped(key) {
                "CHANGE_ME_SECURE_32_CHARS_MIN"
            } else {
                "your_value_here"
            };
            FixAction::AddMissing(default.to_string())
        }
        _ => FixAction::Skip,
    }
}

// ─────────────────────────────────────────────────────────────
// Secure Secret Generation
// ─────────────────────────────────────────────────────────────

/// Generate a secret: 32 bytes from the operating system's CSPRNG, as 64 hex
/// characters.
///
/// ⚠️ **Until 2026-09-24 this was the clock.** The body read:
///
/// ```text
/// let seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
/// format!("{:064x}", seed ^ 0x5DEECE66D)
/// ```
///
/// with a comment saying "Simple time-based entropy for demo; replace with
/// crypto RNG in production". It was not replaced. A `u128` of nanoseconds
/// occupies about 61 bits, so 48 of the 64 hex characters were always `0`, and
/// three runs seconds apart differed only in their last few:
///
/// ```text
/// 000000000000000000000000000000000000000000000000 18d81d26b81a8f18
/// 000000000000000000000000000000000000000000000000 18d81d26b6d438e2
/// 000000000000000000000000000000000000000000000000 18d81d26b760501b
/// ```
///
/// That is roughly 25 bits of *time*, not randomness. Anyone who knew the day a
/// project ran `evnx validate --fix` could search the whole space in seconds —
/// and evnx printed "Generated secure secret" over it.
///
/// `getrandom` reads the OS CSPRNG directly: `getrandom` on Linux, `BCryptGen…`
/// on Windows, `getentropy` on macOS.
///
/// # Panics
///
/// If the OS cannot supply randomness. That is not a condition to paper over
/// with a fallback: a silent downgrade to a weaker source is exactly the bug
/// this replaced, and a secret nobody can generate is better than one everybody
/// can guess.
pub fn generate_secure_secret() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .expect("the operating system could not supply randomness for a new secret");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ─────────────────────────────────────────────────────────────
// Apply Fix to In-Memory HashMap
// ─────────────────────────────────────────────────────────────

/// Is repeating the value evnx is about to replace harmless?
///
/// Two conditions, and the key matters as much as the value: `DEBUG=True` → `true`
/// is exactly the kind of change a report should show, and `SECRET_KEY=changeme`
/// names a placeholder rather than a credential. What must not be echoed is a
/// credential-shaped variable that held something real.
fn old_value_is_safe_to_show(key: &str, value: &str) -> bool {
    !super::checks::is_secret_shaped(key) || super::checks::is_placeholder(value)
}

pub fn apply_fix(
    key: &str,
    value: &str,
    action: &FixAction,
    env_vars: &mut IndexMap<String, String>,
) -> Option<FixApplied> {
    match action {
        FixAction::GenerateSecret => {
            let new_val = generate_secure_secret();
            env_vars.insert(key.to_string(), new_val.clone());
            let show = old_value_is_safe_to_show(key, value);
            Some(FixApplied {
                variable: key.to_string(),
                action: "Generated secure secret".to_string(),
                old_value: show.then(|| value.to_string()),
                old_value_withheld: !show,
                new_value: new_val,
            })
        }
        FixAction::ReplacePlaceholder(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            let show = old_value_is_safe_to_show(key, value);
            Some(FixApplied {
                variable: key.to_string(),
                action: "Replaced placeholder".to_string(),
                old_value: show.then(|| value.to_string()),
                old_value_withheld: !show,
                new_value: new_val.clone(),
            })
        }
        FixAction::FixBoolean(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            let show = old_value_is_safe_to_show(key, value);
            Some(FixApplied {
                variable: key.to_string(),
                action: "Fixed boolean format".to_string(),
                old_value: show.then(|| value.to_string()),
                old_value_withheld: !show,
                new_value: new_val.clone(),
            })
        }
        FixAction::AddMissing(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Added missing variable".to_string(),
                // There was no previous value, which is a different thing from
                // declining to show one.
                old_value: None,
                old_value_withheld: false,
                new_value: new_val.clone(),
            })
        }
        FixAction::Skip => None,
    }
}

// ─────────────────────────────────────────────────────────────
// File I/O: Write Fixed Content
// ─────────────────────────────────────────────────────────────

/// Rewrite **only** the lines whose key was actually fixed, and leave every
/// other byte of the file alone.
///
/// Returns the path of the backup it wrote first.
///
/// ⛔ **This function used to rewrite every line in the file**, by looking each
/// key up in `env_vars` — which is the *whole parsed map*, not the fixes. So a
/// single repaired variable round-tripped the entire file through the parser
/// and back out, and everything the parser normalises away was normalised away
/// for good:
///
/// ```text
/// QUOTED="has spaces and #hash"   →   QUOTED=has spaces and #hash
/// NOTE=value # trailing comment   →   NOTE=value
/// export PATH_LIKE=x              →   PATH_LIKE=x
/// ```
///
/// The first line is the serious one. It is not a cosmetic change: the value
/// cannot be read back. `sh` fails on it outright —
///
/// ```text
/// $ . ./.env
/// ./.env: line 1: spaces: command not found
/// $ echo "[$QUOTED]"
/// []
/// ```
///
/// — and a dotenv parser truncates it at ` #`. evnx reported `✓ All checks
/// passed` over that.
///
/// Two rules, and both are structural rather than careful:
///
/// 1. **A line whose key is not in `changed` is copied verbatim.** Not
///    re-serialised, not re-quoted, not trimmed. The only lines this function
///    may alter are the ones it was asked to alter.
/// 2. **A value this function writes is quoted when it would not survive being
///    read back.** The bug above was a write that could not be re-read; the
///    guard belongs at the write.
pub fn write_fixed_file(
    env_path: &str,
    env_vars: &IndexMap<String, String>,
    original_content: &str,
    changed: &BTreeSet<String>,
) -> Result<String> {
    let mut output = String::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    for line in original_content.lines() {
        // ⚠️ `key_of` returns None for blanks, comments and malformed lines, so
        // all three fall into the verbatim branch without a special case each.
        match key_of(line) {
            Some(key) if changed.contains(key) => {
                seen.insert(key);
                let value = match env_vars.get(key) {
                    Some(v) => v,
                    // Asked to change a key we hold no value for. Copy the line
                    // rather than invent one.
                    None => {
                        output.push_str(line);
                        output.push('\n');
                        continue;
                    }
                };
                // Preserve indentation and an `export ` prefix: they are part of
                // how the file is written, and rewriting a value is not a
                // licence to restyle the line.
                let indent_len = line.len() - line.trim_start().len();
                output.push_str(&line[..indent_len]);
                if line.trim_start().starts_with("export ") {
                    output.push_str("export ");
                }
                output.push_str(key);
                output.push('=');
                output.push_str(&render_value(key, value)?);
                output.push('\n');
            }
            Some(key) => {
                seen.insert(key);
                output.push_str(line);
                output.push('\n');
            }
            None => {
                output.push_str(line);
                output.push('\n');
            }
        }
    }

    // Append variables that were added rather than repaired.
    //
    // ⚠️ The membership test is the line scan above, not
    // `original_content.contains("KEY=")`. That substring matched `KEY=` inside
    // `MY_KEY=`, inside `# KEY=…` in a comment, and inside any *value* that
    // happened to contain it — so a genuinely missing variable could be
    // silently dropped on the floor by a comment mentioning it.
    for key in changed {
        if seen.contains(key.as_str()) {
            continue;
        }
        if let Some(value) = env_vars.get(key) {
            output.push_str(key);
            output.push('=');
            output.push_str(&render_value(key, value)?);
            output.push('\n');
        }
    }

    let backup = write_backup(env_path, original_content)?;
    fs::write(env_path, output).with_context(|| format!("Failed to write fixes to {env_path}"))?;
    Ok(backup)
}

/// The variable a line assigns, or `None` if it assigns nothing.
///
/// Blank lines, comments and lines with no `=` all return `None`, which is what
/// puts them in the verbatim branch.
fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (key, _) = trimmed.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some(key)
}

/// A value, quoted if it would not survive being read back unquoted.
///
/// ⓘ Every value evnx generates today — 64 hex characters, `true`, `false`,
/// `8080`, `https://example.com`, `user@example.com`, `your_value_here` — is
/// safe bare, so in practice this quotes nothing. It exists so that a fix
/// action added later cannot reintroduce the bug this file documents, and it
/// is covered by its own tests rather than by the fixes that happen to exist.
fn render_value(key: &str, value: &str) -> Result<String> {
    if value.contains('\n') || value.contains('\r') {
        // There is no representation for this in the format, and writing the
        // first line only would be the same class of silent truncation.
        anyhow::bail!("refusing to write {key}: the value contains a line break");
    }
    let needs_quoting = value.is_empty()
        || value.starts_with(char::is_whitespace)
        || value.ends_with(char::is_whitespace)
        || value.chars().any(|c| {
            matches!(
                c,
                ' ' | '\t' | '#' | '"' | '\'' | '`' | '$' | '\\' | '=' | '&'
            )
        });
    if !needs_quoting {
        return Ok(value.to_string());
    }
    // ⚠️ `\` and `"` only. `$` is deliberately left as-is inside the quotes:
    // dotenv implementations disagree about whether `\$` means a dollar sign or
    // a backslash followed by one, and guessing wrong would corrupt the value
    // in the same way as not quoting it. evnx never emits a `$`, so this is a
    // boundary that is documented rather than crossed.
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!("\"{escaped}\""))
}

/// Copy the file aside before overwriting it, and never clobber an earlier copy.
///
/// ⚠️ `--fix` edits a credentials file in place. Until 2026-10-07 it did so with
/// no undo at all: the run that replaced a working `DATABASE_PASSWORD` with
/// random hex left nothing to recover it from.
///
/// `.env.bak`, then `.env.bak.2`, `.env.bak.3` … rather than `sed -i.bak`'s
/// overwrite, because two `--fix` runs would otherwise leave a "backup" of the
/// already-damaged file and no copy of the original.
fn write_backup(env_path: &str, original_content: &str) -> Result<String> {
    let mut candidate = format!("{env_path}.bak");
    let mut n = 2;
    while Path::new(&candidate).exists() {
        candidate = format!("{env_path}.bak.{n}");
        n += 1;
        if n > 100 {
            anyhow::bail!(
                "refusing to write fixes: {env_path}.bak through {env_path}.bak.100 all exist — \
                 remove some before running --fix again"
            );
        }
    }
    fs::write(&candidate, original_content)
        .with_context(|| format!("Failed to write a backup to {candidate}"))?;

    // The backup holds the same credentials the original did, so it gets the
    // same permissions rather than the process umask's.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(env_path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        let _ = fs::set_permissions(&candidate, fs::Permissions::from_mode(mode));
    }

    Ok(candidate)
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─────────────────────────────────────────────────────────
    // S1 — `--fix` must not destroy what it was pointed at
    //
    // Each of these fails against the code as it stood on 2026-10-07.
    // ─────────────────────────────────────────────────────────

    fn fix(original: &str, changed: &[&str], vars: &[(&str, &str)]) -> String {
        let dir = std::env::temp_dir().join(format!(
            "evnx-fixer-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(&path, original).unwrap();

        let map: IndexMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let set: BTreeSet<String> = changed.iter().map(|k| k.to_string()).collect();

        let backup =
            write_fixed_file(path.to_str().unwrap(), &map, original, &set).expect("write failed");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            original,
            "the backup is not the original"
        );
        let out = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
        out
    }

    /// ⛔ The one that mattered. A quoted value containing a space and a `#`
    /// was rewritten unquoted, which `sh` cannot parse and a dotenv parser
    /// truncates at ` #` — while evnx printed "All checks passed".
    #[test]
    fn an_untouched_line_is_copied_byte_for_byte() {
        let original = "# a comment\n\
                        QUOTED=\"has spaces and #hash\"\n\
                        export EXPORTED=plain\n\
                        NOTE=value # trailing comment\n\
                        \n\
                        DEBUG=True\n";
        let out = fix(
            original,
            &["DEBUG"],
            &[
                ("QUOTED", "has spaces and #hash"),
                ("EXPORTED", "plain"),
                ("NOTE", "value"),
                ("DEBUG", "true"),
            ],
        );
        assert_eq!(
            out,
            "# a comment\n\
             QUOTED=\"has spaces and #hash\"\n\
             export EXPORTED=plain\n\
             NOTE=value # trailing comment\n\
             \n\
             DEBUG=true\n",
            "a line that was not fixed was rewritten"
        );
    }

    /// `export ` and leading whitespace are how the file is written, and
    /// repairing a value is not a licence to restyle the line.
    #[test]
    fn rewriting_a_line_keeps_its_export_prefix_and_indent() {
        let out = fix("  export DEBUG=True\n", &["DEBUG"], &[("DEBUG", "true")]);
        assert_eq!(out, "  export DEBUG=true\n");
    }

    /// ⛔ The membership test was `original.contains("KEY=")`, so a comment
    /// mentioning the variable convinced the appender it was already present
    /// and a genuinely missing variable was dropped.
    #[test]
    fn a_missing_variable_is_appended_even_if_a_comment_names_it() {
        let out = fix(
            "# set NEW_TOKEN= before deploying\nA=1\n",
            &["NEW_TOKEN"],
            &[("A", "1"), ("NEW_TOKEN", "your_value_here")],
        );
        assert!(
            out.ends_with("NEW_TOKEN=your_value_here\n"),
            "the variable was not appended: {out:?}"
        );
        assert!(out.starts_with("# set NEW_TOKEN= before deploying\n"));
    }

    /// A substring of a longer key must not be mistaken for the key.
    #[test]
    fn a_longer_key_containing_the_fixed_one_is_untouched() {
        let out = fix(
            "MY_DEBUG=True\nDEBUG=True\n",
            &["DEBUG"],
            &[("MY_DEBUG", "True"), ("DEBUG", "true")],
        );
        assert_eq!(out, "MY_DEBUG=True\nDEBUG=true\n");
    }

    /// Whatever this function writes, it must be able to read back.
    #[test]
    fn a_written_value_survives_a_round_trip() {
        for value in [
            "has spaces",
            "has #hash",
            "has\"quote",
            "has\\backslash",
            "  leading and trailing  ",
            "",
            "plain",
            "a=b",
        ] {
            let rendered = render_value("K", value).unwrap();
            let line = format!("K={rendered}");
            let parsed = crate::core::Parser::new(crate::core::ParserConfig::default())
                .parse_content(&line)
                .expect("evnx could not parse what evnx wrote");
            assert_eq!(
                parsed.get("K").map(String::as_str),
                Some(value),
                "round trip changed the value: wrote {line:?}"
            );
        }
    }

    /// There is no representation for a line break in this format, and writing
    /// the first line only is the same silent truncation by another route.
    #[test]
    fn a_value_with_a_line_break_is_refused_not_truncated() {
        assert!(render_value("K", "one\ntwo").is_err());
    }

    /// ⛔ `--fix` replaced working credentials with random hex, printed the old
    /// one, and reported success.
    #[test]
    fn a_real_credential_is_never_regenerated() {
        for value in ["p@ss word#1", "hunter2", "sk_live_short"] {
            assert!(
                matches!(
                    suggest_fix(
                        "DATABASE_PASSWORD",
                        value,
                        &IssueType::WeakSecret,
                        WeakSecretPolicy::PlaceholdersOnly,
                    ),
                    FixAction::Skip
                ),
                "--fix offered to overwrite a real value: {value:?}"
            );
        }
    }

    /// A placeholder is not a credential, so generating over one is the repair
    /// the flag is for.
    #[test]
    fn a_placeholder_secret_is_still_generated() {
        for value in ["changeme", "your_secret_here", "", "<replace me>"] {
            assert!(
                matches!(
                    suggest_fix(
                        "DATABASE_PASSWORD",
                        value,
                        &IssueType::WeakSecret,
                        WeakSecretPolicy::PlaceholdersOnly,
                    ),
                    FixAction::GenerateSecret
                ),
                "--fix declined a placeholder: {value:?}"
            );
        }
    }

    /// ⛔ The replaced credential must not be echoed — not to stdout, and not
    /// into `--format json`, which serialises this struct verbatim.
    #[test]
    fn a_replaced_credential_is_not_repeated_back() {
        let mut env = IndexMap::new();
        let fix = apply_fix(
            "DATABASE_PASSWORD",
            "p@ss word#1",
            &FixAction::GenerateSecret,
            &mut env,
        )
        .unwrap();
        assert_eq!(fix.old_value, None, "the old credential was carried along");
        assert!(fix.old_value_withheld, "the redaction was not recorded");

        let json = serde_json::to_string(&fix).unwrap();
        assert!(
            !json.contains("p@ss word#1"),
            "the old credential reached --format json: {json}"
        );
    }

    /// The complement: a value that is not a credential is shown, because
    /// `"True" → true` is the whole point of the report.
    #[test]
    fn a_harmless_old_value_is_still_shown() {
        let mut env = IndexMap::new();
        let boolean = apply_fix(
            "DEBUG",
            "True",
            &FixAction::FixBoolean("true".to_string()),
            &mut env,
        )
        .unwrap();
        assert_eq!(boolean.old_value.as_deref(), Some("True"));
        assert!(!boolean.old_value_withheld);

        // A placeholder names no credential, so it is shown even on a
        // secret-shaped key.
        let placeholder =
            apply_fix("API_KEY", "changeme", &FixAction::GenerateSecret, &mut env).unwrap();
        assert_eq!(placeholder.old_value.as_deref(), Some("changeme"));
        assert!(!placeholder.old_value_withheld);
    }

    /// Running twice must not leave a "backup" of the already-changed file and
    /// no copy of the original.
    #[test]
    fn a_second_run_does_not_clobber_the_first_backup() {
        let dir = std::env::temp_dir().join(format!("evnx-fixer-bak-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        let p = path.to_str().unwrap();

        let first = "A=one\n";
        fs::write(&path, first).unwrap();
        let mut map = IndexMap::new();
        map.insert("A".to_string(), "two".to_string());
        let b1 =
            write_fixed_file(p, &map, first, &["A".to_string()].into_iter().collect()).unwrap();

        let second = fs::read_to_string(&path).unwrap();
        map.insert("A".to_string(), "three".to_string());
        let b2 =
            write_fixed_file(p, &map, &second, &["A".to_string()].into_iter().collect()).unwrap();

        assert_ne!(b1, b2, "the second run overwrote the first backup");
        assert_eq!(fs::read_to_string(&b1).unwrap(), "A=one\n");
        assert_eq!(fs::read_to_string(&b2).unwrap(), "A=two\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_suggest_fix_placeholder_secret() {
        let action = suggest_fix(
            "API_KEY",
            "changeme",
            &IssueType::PlaceholderValue,
            WeakSecretPolicy::PlaceholdersOnly,
        );
        assert!(matches!(action, FixAction::GenerateSecret));
    }

    #[test]
    fn test_suggest_fix_boolean() {
        let action = suggest_fix(
            "DEBUG",
            "True",
            &IssueType::BooleanTrap,
            WeakSecretPolicy::PlaceholdersOnly,
        );
        assert!(matches!(action, FixAction::FixBoolean(s) if s == "true"));
    }

    #[test]
    fn test_apply_fix_generates_secret() {
        let mut env = IndexMap::new();
        let action = FixAction::GenerateSecret;
        let result = apply_fix("SECRET_KEY", "weak", &action, &mut env);

        assert!(result.is_some());
        let fix = result.unwrap();
        assert_eq!(fix.variable, "SECRET_KEY");
        assert_eq!(fix.new_value.len(), 64); // 32 bytes = 64 hex chars
        assert_eq!(env.get("SECRET_KEY"), Some(&fix.new_value));
    }

    /// ⚠️ The test above passed for two years against a generator that returned
    /// the clock. `len() == 64` was true of
    /// `000000000000000000000000000000000000000000000000 18d81d26b81a8f18`
    /// as well, so the assertion checked the *shape* and never the property the
    /// function exists for.
    ///
    /// These check the property. A time-based generator fails all three.
    #[test]
    fn a_generated_secret_is_actually_random() {
        let secrets: Vec<String> = (0..16).map(|_| generate_secure_secret()).collect();

        // 1. Distinct. A clock read twice in the same nanosecond repeats.
        let unique: std::collections::HashSet<&String> = secrets.iter().collect();
        assert_eq!(unique.len(), secrets.len(), "generated secrets repeated");

        // 2. No shared prefix. This is what actually caught the old generator:
        //    consecutive timestamps agreed on their first ~58 characters.
        let first = &secrets[0];
        for other in &secrets[1..] {
            let shared = first
                .chars()
                .zip(other.chars())
                .take_while(|(a, b)| a == b)
                .count();
            assert!(
                shared < 8,
                "two secrets share {shared} leading characters — this is a counter, not randomness:\n  {first}\n  {other}"
            );
        }

        // 3. Not dominated by one character. The old value was 75% zeros.
        for secret in &secrets {
            let zeros = secret.chars().filter(|c| *c == '0').count();
            assert!(
                zeros < 32,
                "{zeros} of 64 characters are '0' — the high bytes are not being filled: {secret}"
            );
            assert!(
                secret.chars().all(|c| c.is_ascii_hexdigit()),
                "not hex: {secret}"
            );
        }
    }

    #[test]
    fn test_apply_fix_boolean() {
        let mut env = IndexMap::new();
        let action = FixAction::FixBoolean("false".to_string());
        let result = apply_fix("DEBUG", "True", &action, &mut env);

        assert!(result.is_some());
        assert_eq!(env.get("DEBUG"), Some(&"false".to_string()));
    }

    #[test]
    fn test_generate_secret_length() {
        let secret = generate_secure_secret();
        assert_eq!(secret.len(), 64); // 32 bytes in hex
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
