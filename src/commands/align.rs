//! `evnx align`: give `.env` files the layout of their template — its
//! sections, comments and key order — keeping every value they hold.
//!
//! The layout rules, and the comment rule, are in [`crate::core::layout`]. This
//! file reads the files, refuses what must not be aligned, reports, and writes.
//!
//! ⛔ **Names, lines and statuses only.** Nothing printed here — the report,
//! `--format json`, or an error — holds a value, a comment or a source line.
//!
//! # Order of work
//!
//! 0. Which template each file follows is decided before this module runs, by
//!    [`crate::core::resolve`], as for every other command: `--example`, the
//!    environment's own `.env.<name>.example`, `[defaults] example`, then the
//!    project's `.env.example`. Files can follow different templates.
//! 1. Each template is read; a problem in one stops the run.
//! 2. Every target is checked against every template and the other targets:
//!    the same file under two names (a symlink or a hard link) is refused.
//! 3. Every target is read and aligned in memory. A problem in **any** target
//!    stops the run before **any** file is written.
//! 4. `--check` and `--dry-run` report and stop.
//!    (A backup is private, recorded and git-ignored by `safe_write::backup`,
//!    as every command's is since WP-7, so `align` no longer refuses a
//!    `.gitignore` that does not cover `.env.bak`: the backup adds `*.bak`.)
//! 5. The files that change are written through [`safe_write::replace_all`],
//!    as `remove` writes them: each kept as `.bak` (unless `--no-backup`), all
//!    staged, then each committed only if it still holds what was read. If one
//!    fails, those already replaced are put back. A file already aligned is not
//!    opened for writing and gets no backup.
//!
//! ⚠️ Each replacement is atomic; the set is not one transaction. Another
//! process can see the first file aligned before the last one is.

use anyhow::{bail, Result};
use serde::Serialize;
use std::path::Path;

use crate::core::layout::{LayoutError, Plan, Reference, EXTRAS_HEADING};
use crate::core::verdict::Verdict;
use crate::core::{resolve, safe_write};
use crate::utils::path::label;
use crate::utils::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Write the files that change.
    Write,
    /// Report what would change; write nothing.
    DryRun,
    /// Report, write nothing, and exit 1 when a file is out of alignment.
    Check,
}

/// A file to align and the template it follows, as [`crate::core::resolve`]
/// paired them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub env: String,
    pub template: String,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub targets: Vec<Target>,
    pub mode: Mode,
    pub json: bool,
    /// Keep no `.bak` copy of a file before replacing it.
    pub no_backup: bool,
}

/// A target that has been read and aligned in memory.
struct Prepared {
    path: String,
    template: String,
    original: Vec<u8>,
    plan: Plan,
}

/// Run `evnx align`. `Ok` is the verdict; `Err` is "no verdict" (exit 2).
pub fn run(opts: Options) -> Result<Verdict> {
    run_with(opts, &mut |_, _| {})
}

/// [`run`], with a call just before each file is replaced, so tests can change
/// a file under the run or make a write fail.
pub fn run_with(mut opts: Options, before_commit: &mut dyn FnMut(usize, &Path)) -> Result<Verdict> {
    if opts.targets.is_empty() {
        bail!("no file to align: pass --env (or --env-name)");
    }

    // The one "not found" message every command prints, with the environments
    // that do exist, when the only file asked for is not there.
    if let [only] = opts.targets.as_slice() {
        let path = Path::new(&only.env);
        if std::fs::symlink_metadata(path).is_err() {
            let dir = match path.parent() {
                Some(d) if !d.as_os_str().is_empty() => d,
                _ => Path::new("."),
            };
            bail!(resolve::not_found(dir, &label(&only.env), "align"));
        }
    }

    // One spelling per template file (`.env.example` and `./.env.example`, or a
    // link to it), the first one given, so it is read and reported once.
    let mut seen: Vec<String> = Vec::new();
    for t in &mut opts.targets {
        match seen
            .iter()
            .find(|s| safe_write::same_file(Path::new(s.as_str()), Path::new(&t.template)))
        {
            Some(first) => t.template = first.clone(),
            None => seen.push(t.template.clone()),
        }
    }

    // Each template once, in the order the targets name them.
    let mut templates: Vec<(&str, Reference)> = Vec::new();
    for t in &opts.targets {
        if !templates.iter().any(|(p, _)| *p == t.template) {
            templates.push((&t.template, read_template(&t.template)?));
        }
    }
    let names: Vec<&str> = templates.iter().map(|(p, _)| *p).collect();
    refuse_aliases(&names, &opts.targets)?;

    // ── Preflight: every target, before anything is written ──────────────────
    let mut prepared = Vec::with_capacity(opts.targets.len());
    let mut problems: Vec<String> = Vec::new();
    for t in &opts.targets {
        let (_, reference) = templates
            .iter()
            .find(|(p, _)| *p == t.template)
            .expect("every target's template was read above");
        match prepare(reference, t) {
            Ok(p) => prepared.push(p),
            Err(lines) => problems.extend(lines),
        }
    }
    if !problems.is_empty() {
        bail!(
            "nothing was aligned and no file was written:\n{}",
            problems
                .iter()
                .map(|p| format!("  {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let drifted = prepared.iter().filter(|p| p.plan.changed).count();
    match opts.mode {
        Mode::Check | Mode::DryRun => {
            report(&opts, &prepared, &Written::default());
            let verdict = if opts.mode == Mode::Check && drifted > 0 {
                Verdict::Findings
            } else {
                Verdict::Clean
            };
            Ok(verdict)
        }
        Mode::Write => {
            let written = write_all(&prepared, opts.no_backup, before_commit)?;
            report(&opts, &prepared, &written);
            Ok(Verdict::Clean)
        }
    }
}

fn read_text(path: &str) -> std::result::Result<String, String> {
    let shown = label(path);
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("{shown} does not exist"))
        }
        Err(e) => return Err(format!("{shown}: could not be read ({e})")),
    };
    if !meta.is_file() {
        return Err(format!("{shown} is not a regular file"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{shown}: could not be read ({e})"))?;
    String::from_utf8(bytes).map_err(|_| format!("{shown}: {}", LayoutError::NotUtf8))
}

fn read_template(path: &str) -> Result<Reference> {
    let text = read_text(path).map_err(|e| anyhow::anyhow!("template {e}"))?;
    Reference::parse(&text).map_err(|errors| {
        anyhow::anyhow!(
            "the template {} cannot be used as a layout; no file was written:\n{}",
            label(path),
            indent(&label(path), &errors)
        )
    })
}

fn indent(file: &str, errors: &[LayoutError]) -> String {
    errors
        .iter()
        .map(|e| format!("  {file}: {e}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// No target is a template (its own or another's), and no two targets are the
/// same file: by path, through a symlink, and by hard link (same device and
/// inode).
fn refuse_aliases(templates: &[&str], targets: &[Target]) -> Result<()> {
    for (i, t) in targets.iter().enumerate() {
        for template in templates {
            if safe_write::same_file(Path::new(template), Path::new(&t.env)) {
                bail!(
                    "{} is the same file as the template {} (by path, symlink or hard link). \
                     Nothing was written.",
                    label(&t.env),
                    label(template)
                );
            }
        }
        for earlier in &targets[..i] {
            if safe_write::same_file(Path::new(&earlier.env), Path::new(&t.env)) {
                bail!(
                    "{} and {} are the same file (by path, symlink or hard link); \
                     name it once. Nothing was written.",
                    label(&earlier.env),
                    label(&t.env)
                );
            }
        }
    }
    Ok(())
}

fn prepare(reference: &Reference, target: &Target) -> std::result::Result<Prepared, Vec<String>> {
    let path = target.env.as_str();
    let text = read_text(path).map_err(|e| vec![e])?;
    let plan = reference.align(&text).map_err(|errors| {
        errors
            .iter()
            .map(|e| format!("{}: {e}", label(path)))
            .collect::<Vec<_>>()
    })?;
    Ok(Prepared {
        path: path.to_string(),
        template: target.template.clone(),
        original: text.into_bytes(),
        plan,
    })
}

/// What a write did: the files replaced, and each one's backup.
#[derive(Default)]
struct Written {
    paths: Vec<String>,
    backups: Vec<(String, safe_write::Backup)>,
}

impl Written {
    fn contains(&self, path: &str) -> bool {
        self.paths.iter().any(|p| p == path)
    }
    fn backup_of(&self, path: &str) -> Option<String> {
        self.backups
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| label(&b.path.to_string_lossy()))
    }
    /// `evnx undo .env`, for the file's backup.
    fn undo_of(&self, path: &str) -> Option<String> {
        self.backups
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| b.undo())
    }
}

/// Back up, stage and commit every change, through the write `remove` uses.
fn write_all(
    prepared: &[Prepared],
    no_backup: bool,
    before_commit: &mut dyn FnMut(usize, &Path),
) -> Result<Written> {
    let changing: Vec<&Prepared> = prepared.iter().filter(|p| p.plan.changed).collect();
    let labels: Vec<String> = changing.iter().map(|p| label(&p.path)).collect();
    let files: Vec<safe_write::Replacement<'_>> = changing
        .iter()
        .zip(&labels)
        .map(|(p, l)| safe_write::Replacement {
            path: Path::new(&p.path),
            label: l,
            original: &p.original,
            content: p.plan.output.as_bytes(),
            private: true,
            backup: !no_backup,
        })
        .collect();
    let backups = safe_write::replace_all(&files, before_commit)?;
    Ok(Written {
        paths: changing.iter().map(|p| p.path.clone()).collect(),
        backups: changing
            .iter()
            .zip(backups)
            .filter_map(|(p, b)| b.map(|b| (p.path.clone(), b)))
            .collect(),
    })
}

// ── Report ────────────────────────────────────────────────────────────────────

/// `--format json`. `schema_version` changes only when a field changes meaning.
#[derive(Serialize)]
struct JsonReport<'a> {
    schema_version: u32,
    mode: &'static str,
    /// Every target already matched its template before this run.
    aligned: bool,
    targets: Vec<JsonTarget<'a>>,
}

#[derive(Serialize)]
struct JsonTarget<'a> {
    file: String,
    /// The template this file follows.
    template: String,
    /// `aligned` (nothing to do), `written`, or `out-of-alignment` (check,
    /// dry run).
    status: &'static str,
    moved: Vec<JsonMove<'a>>,
    missing: Vec<JsonMissing<'a>>,
    extra: Vec<JsonMove<'a>>,
    target_only_comments: usize,
    /// The `.bak` kept before this file was replaced: write mode only.
    backup: Option<String>,
}

#[derive(Serialize)]
struct JsonMove<'a> {
    key: &'a str,
    from_line: usize,
    to_line: usize,
}

#[derive(Serialize)]
struct JsonMissing<'a> {
    key: &'a str,
    line: usize,
}

fn status(p: &Prepared, written: &Written) -> &'static str {
    if !p.plan.changed {
        "aligned"
    } else if written.contains(&p.path) {
        "written"
    } else {
        "out-of-alignment"
    }
}

fn report(opts: &Options, prepared: &[Prepared], written: &Written) {
    if opts.json {
        let r = JsonReport {
            schema_version: 1,
            mode: match opts.mode {
                Mode::Write => "write",
                Mode::DryRun => "dry-run",
                Mode::Check => "check",
            },
            aligned: prepared.iter().all(|p| !p.plan.changed),
            targets: prepared
                .iter()
                .map(|p| JsonTarget {
                    file: label(&p.path),
                    template: label(&p.template),
                    status: status(p, written),
                    moved: moves(&p.plan.moved),
                    missing: p
                        .plan
                        .missing
                        .iter()
                        .map(|a| JsonMissing {
                            key: &a.key,
                            line: a.line,
                        })
                        .collect(),
                    extra: moves(&p.plan.extra),
                    target_only_comments: p.plan.target_comments,
                    backup: written.backup_of(&p.path),
                })
                .collect(),
        };
        // Serialising names and numbers cannot fail.
        println!(
            "{}",
            serde_json::to_string_pretty(&r).expect("a report of names and numbers")
        );
        return;
    }

    let quiet = ui::is_quiet();
    // One template for every file: the summary names it. Several: each file's
    // line does.
    let mut templates: Vec<&str> = Vec::new();
    for p in prepared {
        if !templates.contains(&p.template.as_str()) {
            templates.push(&p.template);
        }
    }
    let one_template = templates.len() == 1;
    for p in prepared {
        let s = match (opts.mode, p.plan.changed) {
            (_, false) => "already aligned",
            (Mode::Write, true) => "aligned",
            (Mode::DryRun, true) => "would change",
            (Mode::Check, true) => "out of alignment",
        };
        // -q: what changed or drifted still prints; "already aligned" and a
        // successful write's details do not.
        let show = match opts.mode {
            Mode::Check => p.plan.changed || !quiet,
            Mode::DryRun => true,
            Mode::Write => !quiet,
        };
        if !show {
            continue;
        }
        // A file can be aligned and still lack keys: their placeholders are in
        // place. Said here, because --check exits 0 for it.
        let lacking = p.plan.missing.len();
        let to = if one_template {
            String::new()
        } else {
            format!("  (template {})", label(&p.template))
        };
        if !p.plan.changed && lacking > 0 {
            println!(
                "{}  {s}; {lacking} key{} with no value (# KEY= placeholder{}){to}",
                label(&p.path),
                plural(lacking),
                plural(lacking)
            );
        } else {
            println!("{}  {s}{to}", label(&p.path));
        }
        if p.plan.changed {
            details(&p.plan);
            if let Some(b) = written.backup_of(&p.path) {
                let undo = written.undo_of(&p.path).unwrap_or_default();
                println!("    backup  {b}   undo: {undo}");
            }
        }
    }

    let changed = prepared.iter().filter(|p| p.plan.changed).count();
    let with = if one_template {
        format!(" {}", label(templates[0]))
    } else {
        " their templates".to_string()
    };
    match opts.mode {
        Mode::Write if changed > 0 => ui::success(format!(
            "Aligned {changed} of {} file{} to{with}",
            prepared.len(),
            plural(prepared.len()),
        )),
        Mode::Check if changed > 0 => {
            eprintln!(
                "{changed} of {} file{} out of alignment with{with}.",
                prepared.len(),
                plural(prepared.len()),
            );
            // One command per template, naming every file, so it is right
            // whatever .evnx.toml says.
            for template in &templates {
                let envs: String = prepared
                    .iter()
                    .filter(|p| p.plan.changed && p.template == *template)
                    .map(|p| format!(" --env {}", crate::core::shell::word(&label(&p.path))))
                    .collect();
                if !envs.is_empty() {
                    eprintln!(
                        "Fix: evnx align --example {}{envs}",
                        crate::core::shell::word(&label(template)),
                    );
                }
            }
        }
        Mode::DryRun => ui::info("Dry run: no file was written."),
        _ => {}
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn moves(m: &[crate::core::layout::Moved]) -> Vec<JsonMove<'_>> {
    m.iter()
        .map(|m| JsonMove {
            key: &m.key,
            from_line: m.from_line,
            to_line: m.to_line,
        })
        .collect()
}

/// One row per key: where it was and where it goes. Never a value.
fn details(plan: &Plan) {
    for m in &plan.moved {
        println!("    moved  {}  line {} → {}", m.key, m.from_line, m.to_line);
    }
    for a in &plan.missing {
        println!(
            "    missing  {}  line {} (written as # {}=)",
            a.key, a.line, a.key
        );
    }
    for m in &plan.extra {
        println!(
            "    extra  {}  line {} → {}  (kept under \"{}\")",
            m.key,
            m.from_line,
            m.to_line,
            EXTRAS_HEADING.trim_start_matches("# ")
        );
    }
    if plan.target_comments > 0 {
        println!(
            "    kept   {} comment line{} found only in this file",
            plan.target_comments,
            plural(plan.target_comments)
        );
    }
    if plan.moved.is_empty() && plan.missing.is_empty() && plan.extra.is_empty() {
        println!("    layout  headings, comments or blank lines from the template");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE: &str = "A=\nB=\n";
    const FILES: &[&str] = &[".env.one", ".env.two", ".env.three"];

    fn project() -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        std::fs::write(d.path().join(".env.example"), REFERENCE).unwrap();
        for (i, f) in FILES.iter().enumerate() {
            std::fs::write(d.path().join(f), format!("B={i}\nA={i}\n")).unwrap();
        }
        d
    }

    fn options(d: &tempfile::TempDir) -> Options {
        let at = |f: &str| d.path().join(f).to_string_lossy().into_owned();
        Options {
            targets: FILES
                .iter()
                .map(|f| Target {
                    env: at(f),
                    template: at(".env.example"),
                })
                .collect(),
            mode: Mode::Write,
            json: false,
            no_backup: false,
        }
    }

    fn read(d: &tempfile::TempDir, f: &str) -> String {
        std::fs::read_to_string(d.path().join(f)).unwrap()
    }

    /// The third replacement fails: the first two are put back, every file
    /// holds what it held, and the backups taken first stay.
    #[test]
    fn a_failed_third_write_puts_the_first_two_back() {
        let d = project();
        let err = run_with(options(&d), &mut |i, path| {
            if i == 2 {
                std::fs::remove_file(path).unwrap();
                std::fs::create_dir(path).unwrap();
            }
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("Put back as they were: "), "{err}");
        assert!(
            err.contains(".env.one, ") && err.contains(".env.two."),
            "{err}"
        );
        assert_eq!(read(&d, ".env.one"), "B=0\nA=0\n");
        assert_eq!(read(&d, ".env.two"), "B=1\nA=1\n");
        assert_eq!(read(&d, ".env.one.bak"), "B=0\nA=0\n");
    }

    /// A file edited after it was read is not overwritten, and the file already
    /// replaced before it is put back.
    #[test]
    fn a_file_changed_under_the_run_is_left_alone_and_the_rest_put_back() {
        let d = project();
        let err = run_with(options(&d), &mut |i, path| {
            if i == 1 {
                std::fs::write(path, "B=edited\n").unwrap();
            }
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("changed while evnx was running"), "{err}");
        assert!(err.contains("Not reached: "), "{err}");
        assert_eq!(read(&d, ".env.one"), "B=0\nA=0\n");
        assert_eq!(read(&d, ".env.two"), "B=edited\n");
        assert_eq!(read(&d, ".env.three"), "B=2\nA=2\n");
    }

    #[test]
    fn a_good_run_aligns_every_file() {
        let d = project();
        run_with(options(&d), &mut |_, _| {}).unwrap();
        for (i, f) in FILES.iter().enumerate() {
            assert_eq!(read(&d, f), format!("A={i}\nB={i}\n"), "{f}");
        }
    }
}
