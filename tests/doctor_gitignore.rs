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

use support::{run, Project};

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
    out.assert_not_contains("NOT in .gitignore");
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
