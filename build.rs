//! `EVNX_COMMIT_EPOCH`: when the commit this binary is built from was made, so
//! `evnx update` can say how old the release is without asking anything (RM-42).
//!
//! `SOURCE_DATE_EPOCH` wins when set, so a reproducible build stays
//! reproducible. Otherwise git's committer date. Neither (a crates.io source
//! package has no `.git`): no variable, and `update` says nothing about age.

use std::path::Path;
use std::process::Command;

/// `git rev-parse --git-path <what>`: where git keeps `what` for this working
/// tree, which is not `.git/<what>` in a linked worktree (`.git` is a file
/// there) and for refs shared between worktrees.
fn git_path(what: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-path", what])
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    (out.status.success() && !s.is_empty()).then(|| s.to_string())
}

/// Rebuild when the commit changes: `HEAD` (a checkout), the branch it points
/// to (a commit moves that file) and `packed-refs` (a branch git has packed).
///
/// Not `index`: that changes on every `git add` and `git status`, which re-ran
/// this script, and so rebuilt the crate, without the commit changing.
///
/// ⚠️ Only paths that exist. Cargo treats a missing path as changed on every
/// build, so naming a loose ref that is packed instead would rebuild forever.
/// The cost: after `git pack-refs`, the first commit on that branch creates the
/// loose ref and is not noticed until `HEAD` or `packed-refs` changes too.
fn watch_commit() {
    let mut paths = vec![git_path("HEAD"), git_path("packed-refs")];
    if let Ok(out) = Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
    {
        if out.status.success() {
            if let Ok(name) = String::from_utf8(out.stdout) {
                paths.push(git_path(name.trim()));
            }
        }
    }
    for p in paths.into_iter().flatten() {
        if Path::new(&p).exists() {
            println!("cargo:rerun-if-changed={p}");
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    watch_commit();

    let epoch = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .filter(|s| s.trim().parse::<i64>().is_ok())
        .or_else(|| {
            let out = Command::new("git")
                .args(["log", "-1", "--format=%ct"])
                .output()
                .ok()?;
            let s = String::from_utf8(out.stdout).ok()?;
            let s = s.trim();
            (out.status.success() && s.parse::<i64>().is_ok()).then(|| s.to_string())
        });
    if let Some(e) = epoch {
        println!("cargo:rustc-env=EVNX_COMMIT_EPOCH={}", e.trim());
    }
}
