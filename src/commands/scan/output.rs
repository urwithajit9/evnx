//! Output formatters for scan results.
//!
//! This module provides multiple output formats:
//! - **Pretty**: Human-readable terminal output with colors and icons
//! - **JSON**: Machine-readable JSON for tooling integration
//! - **SARIF**: Standard format for GitHub Code Scanning and static analysis
//!
//! # Example
//!
//! ```no_run
//! # use evnx::commands::scan::OutputFormat;
//! # use evnx::commands::scan::ScanResults;
//! # use evnx::commands::scan::render;
//! let results = ScanResults::new(10);
//! render(&results, OutputFormat::Pretty, &[]).unwrap();
//! ```

use super::models::{Confidence, ScanResults};
use crate::utils::string::pluralize;
use crate::utils::ui;
use crate::utils::ui::glyph;

/// Column the confidence level is right-aligned against.
const WIDTH: usize = 44;
use anyhow::Result;
use colored::*;
use std::path::PathBuf;
use std::str::FromStr;

/// Output format for scan results.
///
/// Determines how results are presented to the user or downstream tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable terminal output with colors
    Pretty,
    /// JSON format for machine parsing
    Json,
    /// SARIF format for GitHub Code Scanning
    Sarif,
    /// GitHub Actions workflow commands — inline annotations on the PR diff
    Github,
}

// impl OutputFormat {
//     /// Parse output format from string argument.
//     ///
//     /// # Arguments
//     ///
//     /// * `s` - Format string (case-insensitive)
//     ///
//     /// # Returns
//     ///
//     /// Matching OutputFormat variant, defaults to Pretty for unknown values.
//     ///
//     /// # Example
//     ///
//     /// ```
//     /// # use evnx::commands::scan::OutputFormat;
//     /// assert_eq!(OutputFormat::from_str("json"), OutputFormat::Json);
//     /// assert_eq!(OutputFormat::from_str("SARIF"), OutputFormat::Sarif);
//     /// assert_eq!(OutputFormat::from_str("unknown"), OutputFormat::Pretty);
//     /// ```
//     pub fn from_str(s: &str) -> Self {
//         match s.to_lowercase().as_str() {
//             "json" => Self::Json,
//             "sarif" => Self::Sarif,
//             _ => Self::Pretty,
//         }
//     }
// }
impl OutputFormat {
    /// Accepted spellings, for error messages and `--help`.
    pub const NAMES: &'static [&'static str] = &["pretty", "json", "sarif", "github"];
}

impl FromStr for OutputFormat {
    type Err = anyhow::Error;

    /// ⚠️ An unrecognised format is an **error**.
    ///
    /// This used to be `_ => Ok(Self::Pretty)`. A typo, or a format that was only
    /// ever documented, therefore printed human-readable output and exited 0 —
    /// `evnx scan --format github` did exactly that for as long as the flag was
    /// documented and unimplemented, so a CI step that looked configured was
    /// checking nothing. Failing loudly is the only safe default for a secret
    /// scanner's output contract.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::core::args::one_of(
            "format",
            s,
            &[
                ("pretty", Self::Pretty),
                ("text", Self::Pretty),
                ("json", Self::Json),
                ("sarif", Self::Sarif),
                // `evnx validate` spells this `github-actions`; accept both
                // everywhere rather than making people remember which is which.
                ("github", Self::Github),
                ("github-actions", Self::Github),
            ],
            Self::NAMES,
        )
    }
}

/// Render scan results in the specified format.
///
/// # Arguments
///
/// * `results` - Scan results to render
/// * `format` - Output format to use
/// * `files` - List of scanned files (used in pretty output)
///
/// # Returns
///
/// Ok(()) on success, Err if output fails (e.g., JSON serialization error).
///
/// # Example
///
/// ```no_run
/// # use evnx::commands::scan::OutputFormat;
/// # use evnx::commands::scan::ScanResults;
/// # use evnx::commands::scan::output::render;
/// let results = ScanResults::new(10);
/// render(&results, OutputFormat::Json, &[]).unwrap();
/// ```
pub fn render(results: &ScanResults, format: OutputFormat, files: &[PathBuf]) -> Result<()> {
    match format {
        OutputFormat::Pretty => render_pretty(results, files),
        OutputFormat::Json => render_json(results),
        OutputFormat::Sarif => render_sarif(results),
        OutputFormat::Github => render_github(results),
    }
}

/// Render results in human-readable pretty format.
///
/// Includes colored output, icons, and remediation guidance.
///
/// # Arguments
///
/// * `results` - Scan results to render
/// * `files` - List of scanned files for summary
fn render_pretty(results: &ScanResults, files: &[PathBuf]) -> Result<()> {
    // `-q` (I-060): no progress line and, on a clean run, no all-clear. Findings,
    // unreadable files and the summary under findings still print (G1).
    let quiet = crate::utils::ui::is_quiet();
    if !files.is_empty() && !quiet {
        let shown = files
            .iter()
            .take(3)
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let rest = files.len().saturating_sub(3);
        let suffix = if rest > 0 {
            format!(" and {}", pluralize(rest, "other file", "other files"))
        } else {
            String::new()
        };
        println!("  scanning {}{}\n", shown.dimmed(), suffix.dimmed());
    }

    // ⛔ Before the all-clear, because an unread file makes the all-clear false.
    //
    // `✓ No secrets detected` over a file the scanner could not open is the
    // worst output this command can produce: it is the answer someone gates a
    // deploy on. Printed first so it cannot be read as a footnote to a pass.
    render_unreadable(results);

    if results.secrets_found == 0 {
        if results.unreadable.is_empty() {
            if quiet {
                return Ok(());
            }
            // ⚠️ Not "No secrets detected" when an ignored file held some: they
            // were detected, and the line below says where (I-285).
            if results.ignored_by_git.is_empty() {
                println!("  {}  No secrets detected", glyph::OK.green());
            } else {
                println!(
                    "  {}  No secrets in files git would commit",
                    glyph::OK.green()
                );
            }
        } else {
            // Not a tick. Nothing was found, and that is not the same as clean.
            println!(
                "  {}  No secrets detected {}",
                glyph::WARN.yellow(),
                "in the files that could be read".dimmed()
            );
        }
        println!("\n  {}", files_line(results).dimmed());
        print_ignored_by_git(results);
        return Ok(());
    }

    // ⚠️ Every finding starts with its glyph at the same column, and every field
    // under it is indented past that glyph. The remediation note used to be
    // printed by `ui::warning`, which writes at column 0 — so a line that belongs
    // to one finding appeared to belong to none.
    for finding in &results.findings {
        let (mark, level) = match finding.confidence {
            Confidence::High => (glyph::FAIL.red(), "high".red()),
            Confidence::Medium => (glyph::WARN.yellow(), "medium".yellow()),
            Confidence::Low => (glyph::INFO.dimmed(), "low".dimmed()),
        };

        // Confidence right-aligned against a fixed column, so the levels form a
        // column of their own however long the pattern name is.
        let name = finding.pattern.bold().to_string();
        // `.max(2)` so a pattern name longer than the column still gets a gap
        // before its level, rather than running straight into it.
        let pad = WIDTH.saturating_sub(finding.pattern.chars().count()).max(2);
        println!("  {mark}  {name}{:pad$}{level}", "", pad = pad);

        println!(
            "     {}  {}",
            finding.location.dimmed(),
            finding.value_preview
        );

        // ⚠️ Three different reasons a finding is here, and they need three
        // different sentences. Saying "matches a live key format" for all of
        // them is a claim about the *value* that two of the three never made.
        //
        // Reported (F18, 2026-09-24): `NEXTAUTH_SECRET=dev-not-a-real-secret`
        // printed "matches a live key format, not a placeholder" — about a
        // value that is literally the word placeholder. Nothing had read it;
        // the variable's *name* is what matched.
        if finding.pattern == super::detector::DECLARED_SECRET {
            println!(
                "     {}  {}",
                glyph::INFO.dimmed(),
                "declared `secret = true` in .evnx.toml".dimmed()
            );
        } else if !finding.judged_value {
            println!(
                "     {}  {}",
                glyph::INFO.dimmed(),
                "flagged by variable name — the value was not checked".dimmed()
            );
        } else if finding.confidence == Confidence::High {
            let why = finding
                .note
                .as_deref()
                .unwrap_or("matches a live key format, not a placeholder");
            println!("     {}  {}", glyph::WARN.yellow(), why.dimmed());
        }
        // ⚠️ Printed before the revoke link on purpose. For a variable the
        // bundler publishes, revoking is not the first thing to do and is not
        // sufficient on its own — the value is already in every build that has
        // shipped, so the name has to change too.
        if let Some(prefix) = &finding.public_prefix {
            let what = crate::utils::patterns::public_prefix(prefix)
                .map(|(_, tool)| tool)
                .unwrap_or("your bundler");
            println!(
                "     {}  {}",
                glyph::WARN.yellow(),
                format!(
                    "{prefix} is inlined into the client bundle by {what} — anyone \
                     who loads your site can read this"
                )
                .yellow()
            );
            if let Some(var) = &finding.variable {
                let server_side = var
                    .strip_prefix(prefix)
                    .filter(|rest| !rest.is_empty())
                    .unwrap_or(var.as_str());
                // ⚠️ Hedged when nothing recognised the value's format. Analytics
                // and error-reporting keys — a PostHog key, a Sentry DSN — are
                // *designed* to ship in the bundle, and evnx cannot tell one from
                // an unrecognised 32-character token. Telling someone to move a
                // key server-side when it is meant to be public would be as wrong
                // as staying silent about one that is not.
                if finding.action_url.is_some() {
                    println!(
                        "     {}  rename to {} and read it server-side",
                        glyph::ARROW.dimmed(),
                        server_side.cyan()
                    );
                } else {
                    println!(
                        "     {}  if this is not meant to be public, rename it to {} \
                         and read it server-side",
                        glyph::ARROW.dimmed(),
                        server_side.cyan()
                    );
                }
            }
        }
        if let Some(url) = &finding.action_url {
            println!("     {}  revoke at {}", glyph::ARROW.dimmed(), url.cyan());
        }
        println!();
    }

    // One line, and only the counts that are non-zero — "0 medium-confidence
    // secrets" is noise, and "1 secrets" was simply wrong.
    let mut parts = vec![pluralize(results.secrets_found, "finding", "findings")];
    if results.high_confidence > 0 {
        parts.push(format!("{} high", results.high_confidence));
    }
    if results.medium_confidence > 0 {
        parts.push(format!("{} medium", results.medium_confidence));
    }
    if results.low_confidence > 0 {
        parts.push(format!("{} low", results.low_confidence));
    }
    println!("  {}", parts.join("  ·  ").bold());
    print_ignored_by_git(results);

    if results.has_critical_findings() {
        println!();

        // ⚠️ The steps below used to be unconditional, and for a variable behind
        // a build-time public prefix two of the three were wrong. Scrubbing git
        // history and force-pushing contains nothing when the value has been
        // compiled into every bundle already served — it is work that produces a
        // feeling of containment and no containment. Someone following it
        // literally would rewrite history and believe they were done.
        let public: Vec<_> = results
            .findings
            .iter()
            .filter(|f| f.public_prefix.is_some())
            .collect();

        // ⚠️ And the *second* version of these steps was wrong in the other
        // direction, which is why this branches again.
        //
        // "Revoke and reissue those keys" was unconditional, so a Supabase anon
        // key behind `NEXT_PUBLIC_` — a credential designed to ship in the
        // browser and protected by row-level security, not by secrecy — was met
        // with "treat them as public, not merely leaked". It *is* public. The
        // per-finding line already hedges for exactly this case; the footer
        // asserted over the top of it, one line later.
        //
        // The discriminator is the same one the per-finding hedge uses: a named
        // provider format carries an `action_url`, shape alone does not.
        let any_named_provider = public.iter().any(|f| f.action_url.is_some());

        // ⚠️ History advice names the files it is about (I-286). It said
        // `git filter-repo --path .env` whatever file held the key, so a key in
        // `src/main.py` came with a command that rewrote the wrong file.
        let committed_public = paths_where(public.iter().copied(), may_be_committed);
        let then_history = (!committed_public.is_empty()).then(|| {
            format!(
                "Then, for anything that was ALSO committed: {}, and force push after \
                 coordinating with everyone who has a clone",
                filter_repo(&committed_public)
            )
        });

        let steps: Vec<String> = if !public.is_empty() && any_named_provider {
            let mut steps = vec![
                "Rename the public-prefixed variables above and read them server-side \
                 — while the name carries the prefix, every build republishes the value"
                    .to_string(),
                "Revoke and reissue those keys — they have been readable by every \
                 visitor to your site, so treat them as public, not merely leaked"
                    .to_string(),
            ];
            steps.extend(then_history);
            steps
        } else if !public.is_empty() {
            // Nothing matched a known provider format, so evnx cannot tell a
            // deliberately publishable key from an unrecognised secret. Say that,
            // rather than prescribing a rotation that may be pointless work.
            let mut steps = vec![
                "Check whether these are meant to be public — analytics and \
                 error-reporting keys are designed to ship in the bundle, and \
                 evnx cannot tell those from a secret it does not recognise"
                    .to_string(),
                "If one is NOT meant to be public, rename it to drop the prefix and \
                 read it server-side, then revoke and reissue it — it has been \
                 readable by every visitor to your site"
                    .to_string(),
            ];
            steps.extend(then_history);
            steps
        } else {
            git_steps(results)
        };
        let steps: Vec<&str> = steps.iter().map(String::as_str).collect();
        ui::print_next_steps(&steps);
    }

    Ok(())
}

/// The one line about walked files git ignores (I-285): which, how many
/// findings they held, and that those are not counted. Not under `-q`, which
/// keeps findings only.
fn print_ignored_by_git(results: &ScanResults) {
    if results.ignored_by_git.is_empty() || crate::utils::ui::is_quiet() {
        return;
    }
    let held: usize = results.ignored_by_git.iter().map(|f| f.findings).sum();
    let paths: Vec<&str> = results
        .ignored_by_git
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    let (it, is) = if paths.len() == 1 {
        ("it is", if held == 1 { "is" } else { "are" })
    } else {
        ("they are", if held == 1 { "is" } else { "are" })
    };
    println!(
        "  {}  {}",
        glyph::INFO.dimmed(),
        format!(
            "git ignores {} — {} there {is} not counted, because {it} not committed \
             (--include-ignored lists them)",
            name_files(&paths),
            pluralize(held, "finding", "findings"),
        )
        .dimmed()
    );
}

/// `a`, `a and b`, `a, b and c`, `a, b, c and 2 other files`.
fn name_files(paths: &[&str]) -> String {
    match paths {
        [] => String::new(),
        [one] => (*one).to_string(),
        [init @ .., last] if paths.len() <= 3 => format!("{} and {last}", init.join(", ")),
        _ => format!(
            "{} and {}",
            paths[..3].join(", "),
            pluralize(paths.len() - 3, "other file", "other files")
        ),
    }
}

/// Whether the file a finding is in may be in git history: tracked, or
/// nothing is known (outside a repository the old advice stands).
fn may_be_committed(f: &super::models::Finding) -> bool {
    matches!(f.git, None | Some(super::models::GitState::Tracked))
}

/// The distinct files among `findings` that `keep` selects, in order, as
/// `git filter-repo` names them: from the repository root when git said so,
/// else as shown without a leading `./`.
fn paths_where<'a>(
    findings: impl Iterator<Item = &'a super::models::Finding>,
    keep: fn(&super::models::Finding) -> bool,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in findings.filter(|f| keep(f)) {
        let path = match &f.repo_path {
            Some(p) => p.clone(),
            None => {
                let (file, _) = split_location(&f.location);
                file.strip_prefix("./").unwrap_or(file).to_string()
            }
        };
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// The distinct files among `findings` in `state`, as the report shows them.
fn shown_in(results: &ScanResults, state: super::models::GitState) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in results.findings.iter().filter(|f| f.git == Some(state)) {
        let (file, _) = split_location(&f.location);
        if !out.iter().any(|p| p == file) {
            out.push(file.to_string());
        }
    }
    out
}

/// `git filter-repo --path a --path b --invert-paths`, for up to three files.
fn filter_repo(paths: &[String]) -> String {
    if paths.len() <= 3 {
        let args: String = paths.iter().map(|p| format!("--path {p} ")).collect();
        format!("git filter-repo {args}--invert-paths")
    } else {
        format!(
            "git filter-repo --path <file> --invert-paths, with one --path for each of the {} files above",
            paths.len()
        )
    }
}

/// The steps for findings with no public prefix, by what git knows (I-286).
///
/// * May be in history (tracked, or no repository): revoke, rewrite the
///   history of exactly those files, force push — the old advice.
/// * Not ignored and not tracked: nothing is committed yet, so keep it so.
/// * Ignored and not tracked (a file named on the command line, or
///   `--include-ignored`): nothing was committed.
fn git_steps(results: &ScanResults) -> Vec<String> {
    use super::models::GitState;
    let history = paths_where(results.findings.iter(), may_be_committed);
    let uncommitted = shown_in(results, GitState::Unignored);
    let ignored = shown_in(results, GitState::Ignored);
    let names = |v: &[String]| name_files(&v.iter().map(String::as_str).collect::<Vec<_>>());

    let mut steps = Vec::new();
    if !history.is_empty() {
        steps.push("Revoke or rotate the keys above — assume they are compromised".to_string());
        steps.push(format!(
            "Remove them from history: {}",
            filter_repo(&history)
        ));
        steps.push("Force push, after coordinating with everyone who has a clone".to_string());
    }
    if !uncommitted.is_empty() {
        // ⚠️ Two remedies: an env file is gitignored; a source file is not —
        // its value moves out of it. "Add src/billing.py to .gitignore" would
        // stop the source being committed, not the secret.
        let (env, source): (Vec<String>, Vec<String>) = uncommitted
            .iter()
            .cloned()
            .partition(|p| super::runner::named_like_dotenv(std::path::Path::new(p)));
        let them = |v: &[String]| if v.len() == 1 { "it" } else { "them" };
        if !env.is_empty() {
            steps.push(format!(
                "Keep them out of git: git would commit {} — add {} to .gitignore",
                names(&env),
                them(&env)
            ));
        }
        if !source.is_empty() {
            steps.push(format!(
                "Move the values out of {} before you commit — into a file git ignores, \
                 such as .env, read at run time",
                names(&source)
            ));
        }
        if history.is_empty() {
            steps.push("If one was pushed or shared before, revoke or rotate it".to_string());
        }
    }
    if !ignored.is_empty() {
        steps.push(format!(
            "git ignores {} and does not track {}, so nothing was committed",
            names(&ignored),
            if ignored.len() == 1 { "it" } else { "them" }
        ));
        if history.is_empty() && uncommitted.is_empty() {
            steps.push(
                "Rotate a key only if the file was shared another way — a copy, an \
                 archive, a container image"
                    .to_string(),
            );
        }
    }
    steps
}

/// Render results in JSON format.
///
/// Suitable for machine parsing, CI/CD integration, and custom tooling.
///
/// # Arguments
///
/// * `results` - Scan results to serialize
fn render_json(results: &ScanResults) -> Result<()> {
    // `flatten` keeps every existing top-level key exactly where it was — adding
    // `summary` is purely additive, so anything already parsing `secrets_found`
    // or `high_confidence` keeps working.
    #[derive(serde::Serialize)]
    struct JsonReport<'a> {
        #[serde(flatten)]
        results: &'a ScanResults,
        summary: super::models::Summary,
    }

    let json = serde_json::to_string_pretty(&JsonReport {
        results,
        summary: results.summary(),
    })?;
    println!("{}", json);
    Ok(())
}

/// Split a finding's `location` into (file, line).
///
/// `location` is built as `path:line` or `path:line (VAR)`. Splitting from the
/// **right** matters: a Windows path like `C:\\src\\.env:8` contains a colon that is
/// not the separator, and splitting from the left would report the file as `C`.
fn split_location(location: &str) -> (&str, usize) {
    let head = location.split(" (").next().unwrap_or(location);
    match head.rsplit_once(':') {
        Some((file, line)) => (file, line.parse().unwrap_or(1)),
        None => (head, 1),
    }
}

/// Escape a value for a GitHub Actions workflow command.
///
/// GitHub's own spec: `%`, CR and LF must be encoded everywhere, and inside a
/// *property* value `:` and `,` must be encoded too — they are the property
/// delimiters, so a raw one silently truncates the annotation.
fn gh_escape(value: &str, is_property: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '%' => out.push_str("%25"),
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            ':' if is_property => out.push_str("%3A"),
            ',' if is_property => out.push_str("%2C"),
            _ => out.push(c),
        }
    }
    out
}

/// Render findings as GitHub Actions annotations.
///
/// ```text
/// ::error file=.env,line=8,title=Secret detected (high)::AWS_ACCESS_KEY_ID matches AWS Access Key
/// ```
///
/// GitHub renders these on the pull request diff, so a reviewer sees the flagged
/// line without opening the workflow log.
///
/// ⚠️ **The value never appears in the annotation** — only the variable name and
/// the pattern that matched. Annotations are shown on the PR and kept in the
/// workflow log, both of which are readable by more people than the branch is.
/// A secret scanner that publishes the secret in order to report it would be
/// doing the leaking itself, even in truncated form.
fn render_github(results: &ScanResults) -> Result<()> {
    for f in &results.findings {
        let level = match f.confidence {
            Confidence::High => "error",
            Confidence::Medium => "warning",
            Confidence::Low => "notice",
        };
        let (file, line) = split_location(&f.location);

        // ⚠️ The same split as the terminal output (F18). "matches" and
        // "Rotate this credential" are claims about the *value*, and a
        // name-based finding never read one — so a NEXTAUTH_SECRET holding the
        // literal text `dev-not-a-real-secret` told CI to rotate a live key.
        // The wording has to soften here too, or the audience most likely to
        // automate on it is the one audience that cannot tell the two apart.
        let what = match (f.variable.as_deref(), f.judged_value) {
            (Some(v), true) => format!("{v} matches {}", f.pattern),
            (Some(v), false) => format!("{v} is named like a secret ({})", f.pattern),
            (None, _) => format!("{} detected", f.pattern),
        };

        let mut message = if f.judged_value {
            match &f.action_url {
                Some(url) => format!("{what}. Rotate it at {url}"),
                None => format!("{what}. Rotate this credential at its source"),
            }
        } else {
            format!(
                "{what}. The value was not checked against a known key format \
                 — confirm before rotating"
            )
        };

        // ⚠️ The annotation is what a reviewer actually reads on the pull
        // request, so the fact that changes the required action belongs in it.
        // "Rotate it" alone is incomplete advice for a variable the bundler
        // publishes: the name has to change too, or the next build republishes
        // the replacement.
        if let Some(prefix) = &f.public_prefix {
            message.push_str(&format!(
                " — and rename it: {prefix} is inlined into the client bundle, \
                 so rotating alone republishes the new value"
            ));
        }

        println!(
            "::{level} file={},line={},title={}::{}",
            gh_escape(file, true),
            line,
            gh_escape(&format!("Secret detected ({})", f.confidence), true),
            gh_escape(&message, false),
        );
    }
    Ok(())
}

/// The stable identifier for the *kind* of finding, not the instance.
///
/// ⚠️ This used to be the whole display name, so a finding from the key-name
/// heuristic produced
/// `secret/sensitive-config-key:-acme_internal_token` — **the variable name
/// inside the rule id**. Every new variable was therefore a new rule to GitHub,
/// which means the rules list grows without bound, a dismissed alert never stays
/// dismissed, and two runs of the same scan share no rules at all.
///
/// The display name carries the instance after a colon; the class is what comes
/// before it.
fn rule_id(pattern: &str) -> String {
    let class = pattern.split(':').next().unwrap_or(pattern).trim();
    format!(
        "secret/{}",
        class
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .replace("--", "-")
    )
}

/// Render results in SARIF format for GitHub Code Scanning.
///
/// SARIF (Static Analysis Results Interchange Format) is a standard
/// format for reporting static analysis results. This output can be
/// uploaded to GitHub Security tab or other SARIF-compatible platforms.
///
/// # Arguments
///
/// * `results` - Scan results to convert
///
/// # References
///
/// - [SARIF Specification](https://docs.oasis-open.org/sarif/sarif/v2.1.0/)
/// - [GitHub Code Scanning](https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/sarif-support-for-code-scanning)
fn render_sarif(results: &ScanResults) -> Result<()> {
    // ⚠️ `rules[]` was absent entirely. Without it GitHub has no name, no
    // description and no help link for an alert — the Security tab shows a bare
    // id. Collected in first-seen order so the document is reproducible.
    let mut rules: Vec<serde_json::Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for f in &results.findings {
        let id = rule_id(&f.pattern);
        if seen.contains(&id) {
            continue;
        }
        seen.push(id.clone());
        let class = f.pattern.split(':').next().unwrap_or(&f.pattern).trim();
        rules.push(serde_json::json!({
            "id": id,
            "name": class.replace(' ', ""),
            "shortDescription": { "text": class },
            "fullDescription": {
                "text": format!("evnx scan matched {class} in a file that may be committed.")
            },
            "helpUri": "https://docs.evnx.dev/cli/commands/scan",
            "defaultConfiguration": {
                "level": match f.confidence {
                    Confidence::High => "error",
                    Confidence::Medium => "warning",
                    Confidence::Low => "note",
                }
            }
        }));
    }

    let sarif_results: Vec<serde_json::Value> = results
        .findings
        .iter()
        .map(|f| {
            let level = match f.confidence {
                Confidence::High => "error",
                Confidence::Medium => "warning",
                Confidence::Low => "note",
            };

            let (file, line) = split_location(&f.location);
            let id = rule_id(&f.pattern);

            // ⚠️ Without a fingerprint GitHub tracks an alert by its line
            // number, so inserting a line above a secret closes the old alert
            // and opens a new one — losing any dismissal or comment on it.
            // Deliberately readable rather than hashed: it is the identity of
            // the finding, and someone debugging why two alerts did not merge
            // should be able to see why.
            let fingerprint = match &f.variable {
                Some(v) => format!("{file}::{v}::{id}"),
                None => format!("{file}::{}::{id}", f.value_preview),
            };

            let mut result = serde_json::json!({
                "ruleId": id,
                "level": level,
                "message": { "text": match &f.variable {
                    Some(v) => format!("{} detected in {v}", f.pattern.split(':').next().unwrap_or(&f.pattern).trim()),
                    None => format!("{} detected", f.pattern),
                }},
                "partialFingerprints": { "evnxSecretV1": fingerprint },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": file },
                        "region": { "startLine": line }
                    }
                }]
            });

            // Where to revoke it, when the pattern knows.
            if let Some(url) = &f.action_url {
                result["hostedViewerUri"] = serde_json::json!(url);
            }
            result
        })
        .collect();

    let sarif = serde_json::json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "evnx scan",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://docs.evnx.dev/cli/commands/scan",
                    "rules": rules
                }
            },
            "results": sarif_results
        }]
    });

    println!("{}", serde_json::to_string_pretty(&sarif)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::models::{Confidence, Finding};
    use super::*;

    #[test]
    fn test_output_format_from_str() {
        for (input, want) in [
            ("json", OutputFormat::Json),
            ("JSON", OutputFormat::Json),
            ("sarif", OutputFormat::Sarif),
            ("pretty", OutputFormat::Pretty),
            ("github", OutputFormat::Github),
            ("github-actions", OutputFormat::Github),
            ("  Github  ", OutputFormat::Github),
        ] {
            assert_eq!(input.parse::<OutputFormat>().unwrap(), want, "{input}");
        }

        // ⚠️ Previously this asserted `unknown` silently became Pretty.
        let err = "unknown".parse::<OutputFormat>().unwrap_err().to_string();
        assert!(err.contains("unknown format"), "{err}");
        assert!(
            err.contains("sarif"),
            "the error must list what is valid: {err}"
        );
    }

    #[test]
    fn test_render_json_structure() {
        let mut results = ScanResults::new(5);
        results.add_finding(Finding::new(
            "Test Key",
            Confidence::High,
            "test_...123",
            "test.rs:10",
            None,
            None,
        ));

        // Just verify it doesn't panic and produces valid JSON
        let result = render_json(&results);
        assert!(result.is_ok());
    }

    #[test]
    fn test_render_sarif_structure() {
        let mut results = ScanResults::new(5);
        results.add_finding(Finding::new(
            "AWS Key",
            Confidence::High,
            "AKIA...XYZ",
            ".env:15",
            Some("AWS_KEY".to_string()),
            Some("https://aws.amazon.com".to_string()),
        ));

        let result = render_sarif(&results);
        assert!(result.is_ok());
    }
}

#[cfg(test)]
mod github_tests {
    use super::*;

    #[test]
    fn location_splits_from_the_right() {
        assert_eq!(split_location(".env:8"), (".env", 8));
        assert_eq!(split_location(".env:8 (API_KEY)"), (".env", 8));
        assert_eq!(
            split_location("config/.env.staging:3"),
            ("config/.env.staging", 3)
        );
        // A drive letter is not the separator.
        assert_eq!(split_location(r"C:\src\.env:12"), (r"C:\src\.env", 12));
        // No line number at all.
        assert_eq!(split_location(".env"), (".env", 1));
    }

    #[test]
    fn property_values_escape_the_delimiters() {
        // `:` and `,` terminate a property, so a raw one truncates the annotation.
        assert_eq!(gh_escape("a,b", true), "a%2Cb");
        assert_eq!(gh_escape("a:b", true), "a%3Ab");
        // In the message body they are ordinary characters.
        assert_eq!(gh_escape("a,b:c", false), "a,b:c");
    }

    #[test]
    fn newlines_and_percent_escape_everywhere() {
        for is_property in [true, false] {
            assert_eq!(gh_escape("100%", is_property), "100%25");
            assert_eq!(gh_escape("a\nb", is_property), "a%0Ab");
            assert_eq!(gh_escape("a\r\nb", is_property), "a%0D%0Ab");
        }
    }
}

/// The files this scan selected and could not read.
///
/// ⚠️ Named rather than counted. "1 file could not be read" sends someone
/// looking; the path and the reason tell them what to do, and the reason is
/// usually "it is UTF-16", which is a one-line fix in their editor.
fn render_unreadable(results: &ScanResults) {
    if results.unreadable.is_empty() {
        return;
    }
    println!(
        "  {}  {}",
        glyph::FAIL.red(),
        pluralize(
            results.unreadable.len(),
            "file could not be read, so it was NOT scanned",
            "files could not be read, so they were NOT scanned"
        )
        .bold()
    );
    for f in &results.unreadable {
        println!("     {}  {}", f.path.dimmed(), f.reason.dimmed());
    }
    println!(
        "     {}  {}",
        glyph::ARROW.dimmed(),
        "re-save as UTF-8 and scan again — a file that was not read is not clean".dimmed()
    );
    println!();
}

/// `N files scanned`, plus the ones that were not.
fn files_line(results: &ScanResults) -> String {
    let scanned = pluralize(results.files_scanned, "file scanned", "files scanned");
    if results.unreadable.is_empty() {
        scanned
    } else {
        format!("{scanned}  ·  {} unreadable", results.unreadable.len())
    }
}
