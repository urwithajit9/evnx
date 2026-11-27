//! `evnx doctor` and git's view of env files (U2 part 3).
//!
//! - I-156: `--fix` reported a repair that did not happen. With `.env*` then
//!   `!.env`, it showed ✓ and exited 0 while git still committed `.env`. Now
//!   every fix is followed by the check again (DEC-D1); a later `!.env` is the
//!   user's line and is reported, never edited (G8).
//! - I-157: a committed `.env` was called "NOT in .gitignore". It is tracked:
//!   the values are already in git, and only `git rm --cached` plus rotating the
//!   secrets helps. `--fix` never runs git (DEC-D2).

mod support;

use support::{Project, run};

fn healthy() -> Project {
    let p = Project::new().git().example("A=\n").env("A=1\n");
    // Unix only: a mode doctor would otherwise flag. Windows has no modes and
    // doctor does not check them there.
    #[cfg(unix)]
    let p = p.mode(".env", 0o600);
    p.track(".env.example");
    p
}

#[test]
fn t_i156_fix_is_not_fixed_when_a_later_line_re_includes_env() {
    let p = healthy().file(".gitignore", ".env*\n!.env\n");
    let out = run(p.path(), &["doctor", "--fix"]);
    out.assert_code(1);
    out.assert_contains("not fixed");
    out.assert_not_contains("can repair");
    assert!(
        !p.is_ignored(".env"),
        "doctor must not edit the user's !.env"
    );
    assert!(p.read(".gitignore").contains("!.env\n"));
}

#[test]
fn t_dec_d1_a_real_fix_says_what_it_changed() {
    let p = healthy();
    let out = run(p.path(), &["doctor", "--fix"]);
    out.assert_code(0);
    assert!(
        out.stderr.contains("Fixed: .gitignore: added .env*"),
        "{}",
        out.stderr
    );
    assert!(p.is_ignored(".env"));
    // Nothing left to do: a second run changes nothing and says nothing.
    let again = run(p.path(), &["doctor", "--fix"]);
    again.assert_code(0);
    assert!(!again.stderr.contains("Fixed:"), "{}", again.stderr);
}

#[test]
fn t_i157_a_committed_env_is_reported_as_tracked() {
    let p = healthy().file(".gitignore", ".env\n");
    p.force_add(".env");
    p.commit_all("oops");
    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains("tracked by git");
    out.assert_contains("git rm --cached .env");
    out.assert_not_contains("git would commit it");
}

#[test]
fn t_dec_d2_fix_never_runs_git() {
    let p = healthy().file(".gitignore", ".env\n");
    p.force_add(".env");
    p.commit_all("oops");
    let out = run(p.path(), &["doctor", "--fix"]);
    out.assert_code(1);
    let tracked = p.git_cmd(&["ls-files", ".env"]);
    assert_eq!(
        tracked.trim(),
        ".env",
        "still tracked: --fix does not run git"
    );
}

#[test]
fn json_output_stays_json_under_fix() {
    let p = healthy();
    let out = run(p.path(), &["doctor", "--fix", "--format", "json"]);
    serde_json::from_str::<serde_json::Value>(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}", out.stdout));
}

/// A dated backup git would commit was "NOT in .gitignore", which reads as a
/// fact about one file when git's answer is what counts (a global gitignore
/// included). It now says git would commit it, and `--fix` adds `.env*.bak*`
/// inside evnx's block and makes the copies owner-only.
#[test]
fn an_unignored_backup_says_git_would_commit_it_and_fix_ignores_it() {
    let p = healthy()
        .file(".gitignore", ".env\n")
        .file(".env.bak-20261120", "A=1\n");
    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains(".env.bak-20261120 is not ignored: git would commit it");
    out.assert_not_contains("NOT in .gitignore");

    let out = run(p.path(), &["doctor", "--fix"]);
    out.assert_code(0);
    assert!(p.is_ignored(".env.bak-20261120"), "{}", out.all());
    let gi = p.read(".gitignore");
    let block = gi
        .split("evnx generated")
        .nth(1)
        .unwrap_or_else(|| panic!("{gi}"));
    assert!(block.lines().any(|l| l == ".env*.bak*"), "{gi}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(p.join(".env.bak-20261120"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{}", out.all());
    }
}

/// Outside a git repository there is no commit to make: ".env is not ignored:
/// git would commit it (security risk)" was an error about a repository that
/// does not exist. It is information now, like the template's tracking.
#[test]
fn outside_a_repository_whether_git_would_commit_env_is_not_checked() {
    let p = Project::new().example("A=\n").env("A=1\n");
    #[cfg(unix)]
    let p = p.mode(".env", 0o600);
    let out = run(p.path(), &["doctor", "--strict"]);
    out.assert_not_contains("git would commit it");
    out.assert_contains("Not a git repository: whether git would commit .env was not checked");
    out.assert_code(0);
    // A `.gitignore` that covers it is still read, as git would read it.
    let p = p.file(".gitignore", ".env\n");
    let out = run(p.path(), &["doctor", "--strict"]);
    out.assert_not_contains("Not a git repository: whether git would commit");
    out.assert_code(0);
}
