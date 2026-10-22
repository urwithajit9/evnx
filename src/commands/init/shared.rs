use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::config::{Config, Template};
use crate::schema::models::{VarCollection, VarMetadata, VarSource};
use crate::utils::string::pluralize;
use crate::utils::ui;

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

/// The folder init writes to, and what governs it (R-init).
///
/// ⚠️ Every path used to be `path.join(".env.example")`, whatever the project
/// named its template (I-295), and nothing read the target's `.evnx.toml`
/// (I-298). The template is now [`Template::of`] the target and its config,
/// the rule doctor follows, and the config is the one `main` loaded for the
/// target ([`target_config`]).
pub struct Target {
    pub root: PathBuf,
    pub config: Config,
    pub template: Template,
    pub verbose: bool,
}

impl Target {
    pub fn new(root: &Path, config: Config, verbose: bool) -> Target {
        let template = Template::of(root, &config);
        Target {
            root: root.to_path_buf(),
            config,
            template,
            verbose,
        }
    }

    pub fn template_path(&self) -> PathBuf {
        self.template.path(&self.root)
    }

    /// The template as a person names it: `.env.example`, `.env.sample`, …
    pub fn template_name(&self) -> &str {
        &self.template.name
    }
}

/// How to treat an existing template.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WriteOptions {
    /// Non-interactive: no prompts may be issued.
    pub yes: bool,
    /// Overwrite an existing template without asking.
    pub force: bool,
    /// `--dry-run`: say what would be written, write nothing (IN-16).
    pub dry_run: bool,
    /// `--json`: the result as one document on stdout (IN-16).
    pub json: bool,
}

impl WriteOptions {
    /// What `--yes` and `--force` say, and nothing else.
    pub fn new(yes: bool, force: bool) -> WriteOptions {
        WriteOptions {
            yes,
            force,
            dry_run: false,
            json: false,
        }
    }
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
    // Public signature kept: this is reachable as library API. Files only, as
    // before: the template the folder uses, `.env`, `.gitignore`; no contract.
    let target = Target::new(output_path, Config::default(), false);
    write_files(
        &target,
        &VarCollection::default(),
        example_content,
        template_content,
        opts,
    )
    .map(|_| ())
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
    /// `--dry-run`: the plan was printed; nothing was written.
    DryRun,
}

// Moved to `utils::prompt` in 0.10.2 so `add` shares it; re-exported so
// init's call sites read as before.
pub(crate) use crate::utils::prompt::yes_no;

/// Whether init may ask: stdin and stderr are terminals, and this is not a CI
/// job (DQ-30).
///
/// ⚠️ Stdin alone was the test, so `evnx init 2>log` at a terminal reached a
/// menu dialoguer could not draw — it draws on stderr — and failed with "IO
/// error: not a terminal: not a terminal" (I-299; sync's I-280). And under
/// `CI=1` a runner that happened to have a terminal showed the menu; CI now
/// gets the same refusal, naming the flags, as no terminal at all.
pub fn can_ask() -> bool {
    crate::utils::term::can_prompt() && !crate::utils::term::ci()
}

/// Stop before a confirmation that cannot be asked ([`can_ask`]), naming the
/// same command with `--yes`.
pub fn require_terminal(example: &str) -> anyhow::Result<()> {
    if !can_ask() {
        return Err(crate::utils::prompt::no_terminal(example));
    }
    Ok(())
}

/// The `.evnx.toml` governing the folder init writes to (I-298).
///
/// ⚠️ `main` loads the working folder's config, and with `-p` that is not the
/// target's: a malformed or bare `evnx.toml` beside the files init was about
/// to write was never read, and they were written. Loaded here before
/// anything is written; an error stops the run (exit 2, "did not run"). When
/// it is a different file from the one `main` announced, it is announced too,
/// with its warnings.
pub fn target_config(path: &Path, announced: Option<&Path>) -> Result<crate::core::config::Loaded> {
    let loaded = crate::core::config::load(path)?;
    if let Some(source) = &loaded.source {
        let same = announced.is_some_and(|a| crate::core::safe_write::same_file(a, source));
        if !same {
            let quiet = crate::utils::ui::is_quiet();
            crate::utils::ui::config_banner(source, &loaded.config.security_overrides(), quiet);
            for warning in &loaded.warnings {
                crate::utils::ui::config_warning(warning, quiet);
            }
        }
    }
    Ok(loaded)
}

/// Print the one line every cancel path shares, and say so in the type.
pub fn aborted() -> Outcome {
    println!("{}", "Aborted. Nothing was written.".yellow());
    Outcome::Aborted
}

/// Write the files, say what was written (`report`, the caller's own line),
/// then keep the contract and say what init left alone ([`finish`]).
pub(crate) fn write(
    target: &Target,
    vars: &VarCollection,
    example_content: &str,
    env_content: &str,
    opts: WriteOptions,
    report: &dyn Fn(),
) -> Result<Outcome> {
    // Worked out before any write, from the state the writes will see: printed
    // instead of writing (--dry-run), or as the result (--json). Neither asks,
    // so an existing template is refused here exactly as under --yes.
    let planned = if opts.dry_run || opts.json {
        if target.template_path().exists() && !opts.force {
            return Err(refusal(&target.template_path()));
        }
        Some(super::report::plan_write(target, vars)?)
    } else {
        None
    };
    if opts.dry_run {
        emit(planned.as_ref(), opts, &target.root);
        return Ok(Outcome::DryRun);
    }
    let outcome = write_files(target, vars, example_content, env_content, opts)?;
    match outcome {
        Outcome::Written => {
            report();
            finish(target, vars, vars)?;
        }
        Outcome::KeptExisting => {
            finish(target, &VarCollection::default(), &template_vars(target)?)?
        }
        Outcome::Aborted | Outcome::DryRun => {}
    }
    emit(planned.as_ref(), opts, &target.root);
    Ok(outcome)
}

/// Print a plan the way the flags ask: the dry-run listing, the JSON
/// document, or nothing.
pub(crate) fn emit(planned: Option<&super::report::Planned>, opts: WriteOptions, root: &Path) {
    let Some(p) = planned else { return };
    if opts.json {
        super::report::print_json(p, opts.dry_run, root);
    } else if opts.dry_run {
        super::report::print_dry_run(p);
    }
}

/// The error for a template that would be overwritten with nobody to ask.
fn refusal(example_path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "{} already exists.\n\n\
         Refusing to overwrite it without being asked to — it is a tracked file \
         and the contents may be hand-maintained.\n\
         Re-run with --force to replace it, or remove it first.",
        example_path.display()
    )
}

/// After the files: the `.evnx.toml` contract (IN-15), then the notices for
/// what init does not write (DQ-32, DQ-33).
///
/// `written` are the names this run introduced, recorded into a `[vars]` that
/// already exists (DQ-17); `whole` is everything the template now declares,
/// for a contract that is created or gains its first `[vars]` (DQ-18, DQ-27).
pub(crate) fn finish(
    target: &Target,
    written: &VarCollection,
    whole: &VarCollection,
) -> Result<()> {
    super::contract::record(target, written, whole);
    notices(target);
    Ok(())
}

/// The template's own variables, for a contract made from a template init
/// kept rather than wrote: every name required, its value the sample — what
/// `spec init` reads from a template.
pub(crate) fn template_vars(target: &Target) -> Result<VarCollection> {
    let mut vars = VarCollection::default();
    let Some(map) = crate::commands::add::shared::read_assignments(&target.template_path())? else {
        return Ok(vars);
    };
    for (name, value) in map {
        vars.vars.insert(
            name,
            VarMetadata {
                example_value: value,
                description: None,
                category: None,
                required: true,
                source: VarSource::BlueprintOverride,
            },
        );
    }
    Ok(vars)
}

/// `init --yes` with no stack named, in a folder that has its template: a
/// fresh clone (DQ-31). Keep the template, create `.env` from it when there
/// is none, keep the contract; nothing is generated.
///
/// ⚠️ This refused with exit 2 ("refusing to overwrite"), so the one command a
/// teammate or a CI job runs after cloning failed, and there was no
/// non-interactive way to get `.env` from the template except `cp`.
pub(crate) fn set_up_from_template(target: &Target, opts: WriteOptions) -> Result<Outcome> {
    let name = target.template_name();
    let planned = if opts.dry_run || opts.json {
        Some(super::report::plan_keep(target, &template_vars(target)?)?)
    } else {
        None
    };
    if opts.dry_run {
        emit(planned.as_ref(), opts, &target.root);
        return Ok(Outcome::DryRun);
    }
    let outcome = set_up(target, name)?;
    emit(planned.as_ref(), opts, &target.root);
    Ok(outcome)
}

fn set_up(target: &Target, name: &str) -> Result<Outcome> {
    if target.root.join(".env").exists() {
        let gitignore_line = update_gitignore(&target.root)?;
        if let Some(line) = gitignore_line {
            ui::say(line);
        }
        ui::success(format!(
            "already set up: {name} and .env are here; nothing was generated"
        ));
        warn_if_env_is_still_committable(&target.root);
        finish(target, &VarCollection::default(), &template_vars(target)?)?;
        return Ok(Outcome::KeptExisting);
    }
    ui::info(format!("{name} is the project's template: kept as it is"));
    env_from_template(&target.root, &target.template, false)?;
    warn_if_env_is_still_committable(&target.root);
    finish(target, &VarCollection::default(), &template_vars(target)?)?;
    Ok(Outcome::Written)
}

/// The env files init does not write, and a direnv `.envrc` its `.gitignore`
/// rule now hides (I-296, DQ-32, DQ-33). Said, never acted on.
///
/// ⚠️ init writes `.env` only. With `[defaults] env_name = "local"` every
/// reading command reads `.env.local`, and `validate` right after init exited 2
/// (".env.local does not exist"); beside `.env.production` and the rest, every
/// new name was missing there. `--env-name` for init is MEC-2 (0.11.0); until
/// then the run says which files it left unchanged and how to check one.
pub(crate) fn notices(target: &Target) {
    for (warning, line) in notices_of(target) {
        if warning {
            ui::warning(line);
        } else {
            ui::info(line);
        }
    }
}

/// The notices as text, for `--dry-run` and `--json`.
pub(crate) fn notice_lines(target: &Target) -> Vec<String> {
    notices_of(target)
        .into_iter()
        .map(|(_, line)| line)
        .collect()
}

/// Each notice, and whether it is a warning (shown under `-q`).
fn notices_of(target: &Target) -> Vec<(bool, String)> {
    let root = &target.root;
    let mut out = Vec::new();
    if let Some(name) = &target.config.defaults.env_name {
        out.push((
            true,
            format!(
                ".evnx.toml sets env_name = \"{name}\": validate, sync and diff read .env.{name}, \
                 not the .env init wrote"
            ),
        ));
    }
    let others = crate::core::env_name::available(root);
    if let Some(first) = others.first() {
        let files: Vec<String> = others.iter().map(|n| format!(".env.{n}")).collect();
        out.push((
            false,
            format!(
                "not changed: {} — check one with: evnx validate --env-name {first}",
                files.join(", ")
            ),
        ));
    }
    if others.iter().any(|n| n == "local") {
        out.push((
            false,
            ".env.local is read before .env by Next.js and Vite: a value there wins".to_string(),
        ));
    }
    // Ignored now, or once this run adds `.env*`: the same line before and
    // after the write, so a dry run lists it too.
    let envrc_ignored = || {
        crate::core::gitignore::env_file_is_protected(root, ".envrc")
            || super::report::gitignore_pending(root)
                .iter()
                .any(|e| e == ".env*")
    };
    if root.join(".envrc").is_file()
        && !crate::core::gitignore::is_tracked(root, ".envrc")
        && envrc_ignored()
    {
        out.push((
            false,
            ".envrc is ignored by the .env* rule in .gitignore; to commit it, add !.envrc there"
                .to_string(),
        ));
    }
    out
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
pub(crate) fn write_files(
    target: &Target,
    vars: &VarCollection,
    example_content: &str,
    template_content: &str,
    opts: WriteOptions,
) -> Result<Outcome> {
    let output_path = target.root.as_path();
    crate::core::safe_write::create_dirs(output_path)?;

    let example_path = target.template_path();
    let name = target.template_name();

    // Questions before writes: a refusal or a cancel leaves nothing behind.
    if example_path.exists() && !opts.force {
        if opts.yes {
            return Err(refusal(&example_path));
        }

        let overwrite = yes_no()
            .with_prompt(format!("{} exists. Overwrite it?", example_path.display()))
            .default(false)
            .interact_opt()
            .with_context(|| format!("could not ask whether to overwrite {name}"))?;

        match overwrite {
            None => return Ok(aborted()),
            Some(false) => {
                let gitignore_line = update_gitignore(output_path)?;
                crate::utils::ui::say(
                    format!("Kept the existing {name}. Nothing was generated.").yellow(),
                );
                if let Some(line) = gitignore_line {
                    crate::utils::ui::say(line);
                }
                // Keeping the template used to end here with no `.env`, while
                // the closing lines told the user to edit one (N-01, DQ-4).
                env_from_template(output_path, &target.template, true)?;
                warn_if_env_is_still_committable(output_path);
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
    let example = crate::core::safe_write::write_shared(
        &example_path,
        format!("{}\n", example_content.trim_start().trim_end()).as_bytes(),
    )
    .with_context(|| format!("Failed to write: {}", example_path.display()))?;
    crate::core::safe_write::report(&example);

    // `.env` holds real values, so it is never replaced. It is created, or
    // given the names the new template has and it lacks (DQ-29).
    let env_path = output_path.join(".env");
    let created_env = !env_path.exists();
    let mut caught_up = 0;
    if created_env {
        create_env(&env_path, template_content.as_bytes())?;
    } else {
        caught_up = catch_up_env(&env_path, vars)?;
    }

    // 3. Talk, in the order these lines have always appeared.
    if created_env {
        crate::utils::ui::say(format!("{} Created .env from template", "✓".green()));
    }
    if caught_up > 0 {
        crate::utils::ui::say(format!(
            "{} Added {} to .env; the values already there are kept",
            "✓".green(),
            pluralize(caught_up, "variable", "variables")
        ));
    }
    if let Some(line) = gitignore_line {
        crate::utils::ui::say(line);
    }
    warn_if_env_is_still_committable(output_path);

    Ok(Outcome::Written)
}

/// Append to an existing `.env` the names in `vars` it does not have, with
/// the template's placeholders, through add's writer. Returns how many.
///
/// ⚠️ A project with a `.env` and no template got a 10-variable template while
/// `.env` kept its 2, and nothing said so; "Next steps" told the user to
/// replace placeholders that were not there (I-297). A name `.env` has is
/// never touched, so no placeholder lands below a real value (N-11).
fn catch_up_env(env_path: &Path, vars: &VarCollection) -> Result<usize> {
    if vars.vars.is_empty() {
        return Ok(0);
    }
    let have = crate::commands::add::shared::read_assignments(env_path)?.unwrap_or_default();
    let mut missing = VarCollection::default();
    for (name, meta) in &vars.vars {
        if !have.contains_key(name) {
            missing.vars.insert(name.clone(), meta.clone());
        }
    }
    if missing.vars.is_empty() {
        return Ok(0);
    }
    let addition = crate::schema::formatter::format_addition_from(&missing, "evnx init")?;
    let existing = crate::commands::add::shared::read(env_path)?;
    crate::commands::add::shared::append_block(env_path, &existing, &addition)?;
    Ok(missing.vars.len())
}

/// Every new `.env` that init or add creates comes through here: 0600 on Unix,
/// atomic (I-237, DQ-16, core::safe_write). It used to take the umask's mode
/// (644), and `evnx doctor` then warned about the file init had just made.
fn create_env(path: &Path, bytes: &[u8]) -> Result<()> {
    let w = crate::core::safe_write::write_private(path, bytes)
        .with_context(|| format!("Failed to write: {}", path.display()))?;
    crate::core::safe_write::report(&w);
    Ok(())
}

/// Create `.env` from the project's template: a byte copy, the same as
/// `cp .env.example .env` (DQ-4a). Returns whether it was created.
///
/// Used where a template exists but `.env` does not: after the user keeps their
/// own template, and after `--from-source` appends to one (N-06). `ask` puts the
/// question first (`Create .env from your .env.example? [Y/n]`, default yes);
/// without it, as under `--yes`, the file is created (DQ-14).
///
/// ⚠️ Never replaces an existing `.env`, and updates `.gitignore` before the
/// file exists, for the same reason as [`write_files`] (I-008).
/// Esc at the question creates nothing and is not an error: the template the
/// user chose to keep stays as it was.
pub(crate) fn env_from_template(
    output_path: &Path,
    template: &Template,
    ask: bool,
) -> Result<bool> {
    let example_path = template.path(output_path);
    let env_path = output_path.join(".env");
    if env_path.exists() || !example_path.exists() {
        return Ok(false);
    }

    if ask {
        let create = yes_no()
            .with_prompt(format!("Create .env from your {}?", template.name))
            .default(true)
            .interact_opt()
            .context("could not ask whether to create .env")?;
        if create != Some(true) {
            return Ok(false);
        }
    }

    let gitignore_line = update_gitignore(output_path)?;
    let bytes = fs::read(&example_path)
        .with_context(|| format!("Failed to read: {}", example_path.display()))?;
    create_env(&env_path, &bytes)?;

    if let Some(line) = gitignore_line {
        crate::utils::ui::say(line);
    }
    crate::utils::ui::say(format!(
        "{} Created .env from {}",
        "✓".green(),
        template.name
    ));
    Ok(true)
}

/// Say so when `.gitignore` has the rules but a later line undoes them.
///
/// ⚠️ `update_gitignore` checks that each rule is *present*, line by line. A
/// `.gitignore` reading `.env*` … `!.env` has them all, so nothing is added and
/// nothing is said, yet git does not ignore `.env` (N-03). This asks git (or,
/// with no repository, the same gitignore logic `add` and `doctor` use) whether
/// `.env` is actually ignored. It warns and never edits: the negation is the
/// user's line, and may be deliberate. A warning, so `-q` does not hide it.
fn warn_if_env_is_still_committable(output_path: &Path) {
    // The shared wording (E6): a tracked `.env` is told `git rm --cached`.
    crate::utils::protect::warn_if_exposed(output_path, ".env");
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
pub(crate) fn update_gitignore(output_path: &Path) -> Result<Option<String>> {
    let gitignore_path = output_path.join(".gitignore");
    let existed = gitignore_path.exists();
    // The one `.gitignore` writer (A-28), shared with `add` and `doctor --fix`.
    let added = crate::core::gitignore::append_entries(&gitignore_path, GITIGNORE_ENTRIES)?;

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
        WriteOptions::new(yes, force)
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
        let outcome = write_files(
            &Target::new(dir.path(), Config::default(), false),
            &VarCollection::default(),
            "A=1",
            "A=\n",
            opts(true, false),
        )
        .unwrap();
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
