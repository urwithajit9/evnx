//! `EVNX_COMMIT_EPOCH`: when the commit this binary is built from was made, so
//! `evnx update` can say how old the release is without asking anything (RM-42).
//!
//! `SOURCE_DATE_EPOCH` wins when set, so a reproducible build stays
//! reproducible. Otherwise git's committer date. Neither (a crates.io source
//! package has no `.git`): no variable, and `update` says nothing about age.

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    let epoch = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .filter(|s| s.trim().parse::<i64>().is_ok())
        .or_else(|| {
            let out = std::process::Command::new("git")
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
