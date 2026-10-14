//! Run a training demo scene and lock its output (P0-3, unit U0).
//!
//! # Why this exists
//!
//! Each reviewed command has a shell runner in the review package that walks
//! it scene by scene and prints every command, its output and its exit code.
//! Those runners are the behaviour lock the review relies on: before a change
//! and after it, the output must be identical except for lines changed on
//! purpose. Until now they ran only on a reviewer's machine. The copies in
//! `tests/demos/` run here, one `#[test]` per scene, against
//! `tests/golden/demos/<cmd>/<scene>.txt`.
//!
//! A scene that differs from its golden file fails with the first differing
//! line. If the change is intended, regenerate and read the diff:
//!
//! ```bash
//! UPDATE_GOLDEN=1 cargo test --all-features --test sync_runner
//! git diff tests/golden/demos/sync/
//! ```
//!
//! A runner prints `MISMATCH` when a check it makes no longer holds. Those
//! checks mark known defects (`KNOWN I-nnn`); when a fix lands the check
//! flips, and the runner line must be changed **on purpose** in the same
//! commit. A scene that prints `MISMATCH` fails even if the golden file was
//! regenerated, so a flip cannot be waved through with `UPDATE_GOLDEN=1`.
//!
//! Linux only: the runners use bash, GNU coreutils, git and, for a few scenes,
//! python3 and nc.

use std::path::Path;
use std::process::Command;

use regex::Regex;

use super::snapshot::{check_golden, normalise};

/// Run `tests/demos/run-<cmd>-demo.sh <scene>` against the binary under test
/// and return its output with paths, dates, the version and random values
/// replaced by markers.
pub fn run_scene(cmd: &str, scene: &str) -> String {
    let root = tempfile::TempDir::new().expect("create temp dir");
    let scenes = root.path().join("scenes");
    let tmp = root.path().join("tmp");
    let home = root.path().join("home");
    for d in [&scenes, &tmp, &home] {
        std::fs::create_dir_all(d).expect("create dir");
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/demos")
        .join(format!("run-{cmd}-demo.sh"));

    // ⚠️ The umask is pinned. Files evnx creates take their mode from it, and
    // several scenes print a mode (I-081, I-130, I-237): 664 under a desktop's
    // 0002, 644 under CI's 0022. 022 is the common default and what the KNOWN
    // lines describe.
    let mut c = Command::new("bash");
    c.arg("-c")
        .arg(r#"umask 022 && exec bash "$0" "$@""#)
        .arg(&script)
        .arg(scene)
        .current_dir(root.path())
        // Nothing from the developer's machine may leak into the output: no
        // evnx credentials or settings, no git identity, no locale.
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("EVNX", super::evnx_bin())
        .env("DEMO_ROOT", &scenes)
        .env("TMPDIR", &tmp)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("NO_COLOR", "1")
        .env("LC_ALL", "C.UTF-8")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "demo")
        .env("GIT_AUTHOR_EMAIL", "demo@example.com")
        .env("GIT_COMMITTER_NAME", "demo")
        .env("GIT_COMMITTER_EMAIL", "demo@example.com");
    let out = c.output().expect("run bash");
    let raw = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // A few scenes print the binary's own path (`bash -c '<evnx>' …`); it
    // depends on the checkout and the target directory.
    let raw = raw.replace(super::evnx_bin(), "evnx");
    mask(&normalise(&raw, &[root.path()]))
}

/// What varies between two runs of the same binary beyond what
/// [`normalise`] handles.
fn mask(s: &str) -> String {
    // The runners end with `(scratch: <dir>)`; it carries no information here.
    let s = Regex::new(r"\(scratch: [^)]*\)")
        .unwrap()
        .replace_all(s, "");
    // `validate --fix` generates a 64-hex SECRET_KEY (T-066).
    let s = Regex::new(r"\b[0-9a-f]{64}\b")
        .unwrap()
        .replace_all(&s, "<random-hex64>");
    // Fractional seconds after a date `normalise` already replaced (doctor's
    // JSON `timestamp`).
    let s = Regex::new(r"<date>\.\d+")
        .unwrap()
        .replace_all(&s, "<date>");
    // Timestamps that are not ISO dates: backup and template file stamps.
    let s = Regex::new(r"\b\d{8}[_-]\d{6}\b")
        .unwrap()
        .replace_all(&s, "<stamp>");
    s.into_owned()
}

/// Run one scene, compare with its golden file, and refuse a `MISMATCH`.
#[track_caller]
pub fn lock_scene(cmd: &str, scene: &str) {
    let out = run_scene(cmd, scene);
    check_golden(&format!("demos/{cmd}/{scene}.txt"), &out);
    let flipped: Vec<&str> = out.lines().filter(|l| l.contains("MISMATCH")).collect();
    assert!(
        flipped.is_empty(),
        "{cmd} {scene}: a runner check no longer holds. If a fix flipped a KNOWN check, \
         change the line in tests/demos/run-{cmd}-demo.sh on purpose.\n{}",
        flipped.join("\n")
    );
}

/// One `#[test]` per scene: `demo_scenes!("sync", s0 => "S0", s1 => "S1");`
#[macro_export]
macro_rules! demo_scenes {
    ($cmd:literal, $($name:ident => $scene:literal),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                support::demo::lock_scene($cmd, $scene);
            }
        )+
    };
}
