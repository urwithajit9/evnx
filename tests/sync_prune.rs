//! `evnx sync --prune` (N14-11, plan §9 phase 2). It shares `remove`'s editor,
//! writer and prompt, so the byte-level cases live with `remove`; these run the
//! binary for what is particular to pruning: what counts as unused, the refusals
//! and the flag conflicts.
//!
//! Fixture values are sentinels, built at run time, that no output may contain.

mod support;

#[cfg(unix)]
use support::pty::{Pty, ENTER};
use support::{run, Project};

fn sentinel() -> String {
    format!("SENTINEL_{}{}", "7b2e", "_decafe")
}

fn evnx(p: &Project, args: &[&str]) -> support::run::Output {
    run(p.path(), args)
}

/// `.env` and `.env.staging` between them hold KEEP_A and KEEP_B; the template
/// also lists STALE_1 and STALE_2, which nothing has.
fn project() -> Project {
    let s = sentinel();
    Project::new()
        .git()
        .file(".gitignore", ".env*\n!.env.example\n")
        .env(&format!("KEEP_A={s}\n"))
        .file(".env.staging", &format!("KEEP_B={s}\n"))
        .example("# the API\nKEEP_A=\nSTALE_1=\nKEEP_B=\nSTALE_2=\n")
}

#[test]
fn a_name_no_environment_file_has_is_pruned_and_nothing_else_changes() {
    let p = project();
    let env = p.read(".env");
    let staging = p.read(".env.staging");
    evnx(&p, &["sync", "--prune", "--yes"])
        .assert_code(0)
        .assert_contains("STALE_1")
        .assert_contains("STALE_2")
        .assert_not_contains(&sentinel());
    assert_eq!(p.read(".env.example"), "# the API\nKEEP_A=\nKEEP_B=\n");
    // Environment files are only read; the template is committed, so no .bak.
    assert_eq!(p.read(".env"), env);
    assert_eq!(p.read(".env.staging"), staging);
    assert!(!p.exists(".env.example.bak"));
}

#[test]
fn the_preview_names_each_file_and_name() {
    let p = project();
    let out = evnx(&p, &["sync", "--prune", "--dry-run"]);
    out.assert_code(0)
        .assert_contains("Dry run: nothing was written.")
        .assert_contains(".env.example")
        .assert_contains("would remove STALE_1")
        .assert_contains("STALE_2")
        .assert_contains(".env.staging");
    assert!(!out.all().contains("KEEP_A (line"), "{}", out.all());
}

#[test]
fn a_dry_run_writes_nothing() {
    let p = project();
    let before = p.read(".env.example");
    evnx(&p, &["sync", "--prune", "--dry-run"]).assert_code(0);
    assert_eq!(p.read(".env.example"), before);
    for f in std::fs::read_dir(p.path()).unwrap() {
        let n = f.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!n.ends_with(".bak") && !n.ends_with(".tmp"), "{n}");
    }
}

#[test]
fn no_terminal_and_no_yes_refuses_with_exit_2() {
    let p = project();
    let before = p.read(".env.example");
    evnx(&p, &["sync", "--prune"])
        .assert_code(2)
        .assert_contains("--yes");
    assert_eq!(p.read(".env.example"), before);
}

#[cfg(unix)]
#[test]
fn answering_no_is_exit_4_and_yes_prunes() {
    let p = project();
    let before = p.read(".env.example");
    let mut s = Pty::spawn(p.path(), &["sync", "--prune"]);
    s.wait_for("Remove 2 unused names from");
    s.press(&["n", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 4, "{rest}");
    assert!(rest.contains("Cancelled; nothing was changed."), "{rest}");
    assert_eq!(p.read(".env.example"), before);

    let mut s = Pty::spawn(p.path(), &["sync", "--prune"]);
    s.wait_for("Remove 2 unused names from");
    s.press(&["y", ENTER]);
    let (_, code) = s.finish();
    assert_eq!(code, 0);
    assert_eq!(p.read(".env.example"), "# the API\nKEEP_A=\nKEEP_B=\n");
}

#[test]
fn a_second_run_has_nothing_to_prune() {
    let p = project();
    evnx(&p, &["sync", "--prune", "--yes"]).assert_code(0);
    let after = p.read(".env.example");
    evnx(&p, &["sync", "--prune", "--yes"])
        .assert_code(0)
        .assert_contains("Nothing to prune");
    assert_eq!(p.read(".env.example"), after);
}

#[test]
fn a_name_the_contract_declares_is_kept_and_said() {
    let p = project().toml("[vars.STALE_1]\nrequired = true\n");
    evnx(&p, &["sync", "--prune", "--yes"])
        .assert_code(0)
        .assert_contains("kept STALE_1")
        .assert_contains("evnx remove STALE_1 --template --contract");
    assert_eq!(
        p.read(".env.example"),
        "# the API\nKEEP_A=\nSTALE_1=\nKEEP_B=\n"
    );
    // The template still declares what the contract does: spec check does not
    // report STALE_1 as missing from it.
    evnx(&p, &["spec", "check"]).assert_not_contains("missing from .env.example");
}

#[test]
fn no_environment_file_is_refused_and_changes_nothing() {
    let p = Project::new().example("A=\nB=\n");
    evnx(&p, &["sync", "--prune", "--yes"])
        .assert_code(2)
        .assert_contains("no environment file");
    assert_eq!(p.read(".env.example"), "A=\nB=\n");
}

#[test]
fn empty_environment_files_are_refused() {
    let p = Project::new().env("# nothing yet\n").example("A=\n");
    evnx(&p, &["sync", "--prune", "--yes"])
        .assert_code(2)
        .assert_contains("is empty");
    assert_eq!(p.read(".env.example"), "A=\n");
}

#[test]
fn an_environment_file_evnx_cannot_read_stops_the_run() {
    let p = project().file(".env.broken", "this line is not an assignment\n");
    let before = p.read(".env.example");
    evnx(&p, &["sync", "--prune", "--yes"])
        .assert_code(2)
        .assert_contains(".env.broken")
        .assert_contains("cannot read");
    assert_eq!(p.read(".env.example"), before);
}

#[test]
fn a_template_that_is_not_there_is_exit_2() {
    let p = Project::new().env("A=1\n");
    evnx(&p, &["sync", "--prune", "--yes"])
        .assert_code(2)
        .assert_contains("no template to prune");
}

#[test]
fn example_names_another_template() {
    let p = project().file("tpl.env", "KEEP_A=\nSTALE_1=\n");
    evnx(&p, &["sync", "--prune", "--yes", "--example", "tpl.env"]).assert_code(0);
    assert_eq!(p.read("tpl.env"), "KEEP_A=\n");
}

#[test]
fn it_refuses_to_mix_with_the_modes_it_does_not_share_a_file_with() {
    let p = project();
    let before = p.read(".env.example");
    for extra in [
        &["--check"][..],
        &["--env", ".env"],
        &["--env-name", "staging"],
        &["--direction", "forward"],
        &["--force"],
        &["--placeholder"],
        &["--skip-without-env"],
        &["--naming-policy", "warn"],
        &["--template-config", "x.json"],
        &["--check", "--format", "json"],
    ] {
        let mut args = vec!["sync", "--prune", "--yes"];
        args.extend_from_slice(extra);
        let out = evnx(&p, &args);
        out.assert_code(2).assert_contains("cannot be used with");
        assert_eq!(p.read(".env.example"), before, "{extra:?}");
    }
    // --yes means nothing without --prune.
    evnx(&p, &["sync", "--yes"]).assert_code(2);
    // --dry-run is allowed.
    evnx(&p, &["sync", "--prune", "--dry-run"]).assert_code(0);
}

#[test]
fn help_documents_prune_and_its_exit_codes() {
    let p = Project::new();
    evnx(&p, &["sync", "--help"])
        .assert_code(0)
        .assert_contains("--prune")
        .assert_contains("4  you answered no");
}
