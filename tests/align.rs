//! `evnx align` at the binary level: layout, preservation, refusals, writes,
//! and that nothing it prints carries a value.
//!
//! Every fixture is synthetic. Values, comments and broken lines carry
//! [`MARK`], and every run's stdout and stderr is checked for it.

mod support;

use std::path::Path;
use std::time::{Duration, SystemTime};

use support::{run, Output, Project};

/// In every value, comment and malformed line below; never in a key name or a
/// file name, so it may never appear in output.
const MARK: &str = "zzSYNTHMARKzz";

const REFERENCE: &str = "\
# Example settings — copy to .env and fill in

# === Database ===
# Primary database host
DB_HOST=localhost
DB_PORT=5432

# === Cache ===
REDIS_URL=redis://localhost:6379

# === Auth ===
JWT_SECRET=change-me
";

/// Run and check the output is free of the marker.
#[track_caller]
fn align(p: &Project, args: &[&str]) -> Output {
    let mut full = vec!["align"];
    full.extend_from_slice(args);
    let out = run(p.path(), &full);
    assert!(
        !out.all().contains(MARK),
        "a value reached output\n--- stdout\n{}\n--- stderr\n{}",
        out.stdout,
        out.stderr
    );
    out
}

/// Content, modification time and inode: "untouched" means all three.
#[derive(Debug, PartialEq)]
struct State {
    bytes: Vec<u8>,
    mtime: SystemTime,
    #[cfg(unix)]
    ino: u64,
}

fn state(path: &Path) -> State {
    let meta = std::fs::metadata(path).unwrap();
    State {
        bytes: std::fs::read(path).unwrap(),
        mtime: meta.modified().unwrap(),
        #[cfg(unix)]
        ino: std::os::unix::fs::MetadataExt::ino(&meta),
    }
}

/// Push a file's mtime into the past, so an unwanted rewrite cannot hide
/// inside the filesystem's timestamp resolution.
fn age(path: &Path) {
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
}

fn project(files: &[(&str, &str)]) -> Project {
    let mut p = Project::new().example(REFERENCE);
    for (name, content) in files {
        p = p.file(name, content);
    }
    p
}

fn assignment_lines(content: &str) -> Vec<&str> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .collect()
}

// ── Layout ───────────────────────────────────────────────────────────────────

#[test]
fn differently_ordered_environments_get_one_layout() {
    let dev =
        format!("JWT_SECRET=dev-{MARK}\nREDIS_URL=redis://dev\nDB_PORT=5433\nDB_HOST=db.dev\n");
    let test =
        format!("DB_PORT=5434\nJWT_SECRET=test-{MARK}\nDB_HOST=db.test\nREDIS_URL=redis://test\n");
    let p = project(&[(".env.development", &dev), (".env.test", &test)]);
    align(
        &p,
        &[
            "--example",
            ".env.example",
            "--env",
            ".env.development",
            "--env",
            ".env.test",
        ],
    )
    .assert_code(0);

    let skeleton = |s: &str| -> Vec<String> {
        s.lines()
            .map(|l| l.split('=').next().unwrap().to_string())
            .collect()
    };
    let (d, t) = (p.read(".env.development"), p.read(".env.test"));
    assert_eq!(skeleton(&d), skeleton(&t), "same layout\n{d}\n{t}");
    assert_eq!(skeleton(&d), skeleton(REFERENCE), "the reference's layout");
    assert!(
        d.contains("# === Database ===\n# Primary database host\nDB_HOST=db.dev\nDB_PORT=5433\n")
    );
    assert!(t.contains("# === Auth ===\nJWT_SECRET=test-"));
    assert!(
        !d.contains("localhost") && !d.contains("change-me"),
        "no reference value"
    );
}

#[test]
fn assignment_blocks_are_kept_byte_for_byte() {
    let target = format!(
        concat!(
            "JWT_SECRET=\"a=b=c {m} \\\"q\\\" \\n \\t \\\\ \\$LIT\"\n",
            "REDIS_URL='redis://u:{m}@h:6379/0#frag'   # inline {m}\n",
            "export DB_PORT=${{PORT:-5432}}\n",
            "DB_HOST=\"-----BEGIN {m}-----\n",
            "# inside the value {m}\n",
            "\n",
            "-----END-----\"\n",
            "EXTRA=$HOME/x#not-a-comment {m}\n",
            "BACKTICK=`{m} $NOPE`\n",
        ),
        m = MARK
    );
    let p = project(&[(".env", &target)]);
    align(&p, &[]).assert_code(0);
    let out = p.read(".env");

    let multiline =
        format!("DB_HOST=\"-----BEGIN {MARK}-----\n# inside the value {MARK}\n\n-----END-----\"\n");
    assert!(out.contains(&multiline), "{out}");
    for line in target.lines().filter(|l| l.contains('=')) {
        assert!(
            out.lines().any(|o| o == line),
            "lost or changed: {line}\n{out}"
        );
    }
    let mut before = assignment_lines(&target);
    let mut after = assignment_lines(&out);
    before.sort_unstable();
    after.sort_unstable();
    assert_eq!(before, after, "the same assignment lines, nothing more");
}

#[test]
fn missing_keys_get_a_placeholder_and_extra_keys_keep_their_order_at_the_end() {
    let p = project(&[(".env", &format!("ZETA=1{MARK}\nDB_HOST=h\nALPHA=2{MARK}\n"))]);
    let out = align(&p, &["--dry-run"]);
    out.assert_code(0)
        .assert_contains("missing  DB_PORT")
        .assert_contains("missing  REDIS_URL")
        .assert_contains("missing  JWT_SECRET")
        .assert_contains("extra  ZETA")
        .assert_contains("extra  ALPHA");

    align(&p, &[]).assert_code(0);
    let env = p.read(".env");
    for key in ["DB_PORT", "REDIS_URL", "JWT_SECRET"] {
        assert!(env.contains(&format!("# {key}=\n")), "{key}: {env}");
        assert!(
            !env.lines().any(|l| l.starts_with(&format!("{key}="))),
            "{key} must stay absent: {env}"
        );
    }
    assert!(
        env.ends_with(&format!(
            "# JWT_SECRET=\n\n# Environment-specific settings\nZETA=1{MARK}\nALPHA=2{MARK}\n"
        )),
        "{env}"
    );
}

/// Why a missing key is `# KEY=` and not `KEY=`: a blank assignment turned both
/// of these checks green on a file that still has no value for the key.
#[test]
fn evnx_still_reports_a_missing_key_after_aligning() {
    let p = Project::new()
        .example("A=\nB=template-value\n")
        .env("A=1\n");
    let before = (
        run(p.path(), &["sync", "--check", "--direction", "reverse"]).code,
        run(p.path(), &["diff", "--fail-on", "missing"]).code,
    );
    assert_eq!(before, (Some(1), Some(1)));

    align(&p, &[]).assert_code(0);
    assert_eq!(p.read(".env"), "A=1\n# B=\n");
    run(p.path(), &["sync", "--check", "--direction", "reverse"]).assert_code(1);
    run(p.path(), &["diff", "--fail-on", "missing"]).assert_code(1);
    // Aligned is a statement about layout: it passes, and says what is missing.
    align(&p, &["--check"])
        .assert_code(0)
        .assert_contains("./.env  already aligned; 1 key with no value");
}

#[test]
fn sync_fills_a_placeholder_and_aligning_again_tidies_up() {
    let p = Project::new()
        .example("A=\n# About B\nB=template-value\nC=\n")
        .env("C=3\nA=1\n");
    align(&p, &[]).assert_code(0);
    assert_eq!(p.read(".env"), "A=1\n# About B\n# B=\nC=3\n");

    run(p.path(), &["sync", "--direction", "reverse", "--yes"]).assert_code(0);
    assert!(p.read(".env").contains("B=template-value\n"));

    align(&p, &[]).assert_code(0);
    assert_eq!(
        p.read(".env"),
        "A=1\n# About B\n# Added by evnx sync\nB=template-value\nC=3\n",
        "the value in place and the placeholder gone; sync's note is only this file's \
         comment, directly above the key, so the comment rule keeps it"
    );
    align(&p, &["--check"]).assert_code(0);
}

#[test]
fn target_comments_are_kept_by_the_attachment_rule() {
    let target = format!(
        "\
# Local overrides for {MARK}

JWT_SECRET=j
# rotate monthly {MARK}
DB_PORT=1

# loose note {MARK}

# === Database ===
DB_HOST=h
"
    );
    let p = project(&[(".env", &target)]);
    align(&p, &["--dry-run"])
        .assert_code(0)
        .assert_contains("kept   3 comment lines found only in this file");
    align(&p, &[]).assert_code(0);
    let env = p.read(".env");
    // The preamble stays on top; "rotate" moves with DB_PORT; the loose note
    // stays after DB_PORT; the copied heading is not doubled.
    assert!(
        env.starts_with(&format!(
            "# Local overrides for {MARK}\n\n# Example settings"
        )),
        "{env}"
    );
    assert!(env.contains(&format!("DB_HOST=h\n# rotate monthly {MARK}\nDB_PORT=1\n\n# loose note {MARK}\n\n# === Cache ===")), "{env}");
    assert_eq!(env.matches("# === Database ===").count(), 1, "{env}");
}

#[test]
fn an_ambiguous_comment_is_refused_and_nothing_is_written() {
    let target = "# === Cache ===\nDB_HOST=1\n# === Cache ===\nDB_PORT=2\n";
    let p = project(&[(".env", target)]);
    age(&p.join(".env"));
    let before = state(&p.join(".env"));
    align(&p, &[])
        .assert_code(2)
        .assert_contains("line 1: this comment also appears in the template")
        .assert_contains("no file was written");
    assert_eq!(state(&p.join(".env")), before);
}

#[test]
fn crlf_and_a_missing_final_newline_are_kept() {
    let p = project(&[(".env", "REDIS_URL=r\r\nDB_HOST=h")]);
    align(&p, &[]).assert_code(0);
    let env = p.read(".env");
    assert!(
        !env.replace("\r\n", "").contains('\n'),
        "CRLF only: {env:?}"
    );
    assert!(env.contains("DB_HOST=h\r\n# DB_PORT=\r\n"), "{env:?}");
    assert!(env.ends_with("# JWT_SECRET="), "no final newline: {env:?}");
    align(&p, &["--check"]).assert_code(0);
}

#[test]
fn mixed_line_endings_are_refused() {
    let p = project(&[(".env", "DB_HOST=1\r\nDB_PORT=2\n")]);
    let before = state(&p.join(".env"));
    align(&p, &[])
        .assert_code(2)
        .assert_contains("mixed line endings (1 CRLF, 1 LF)");
    assert_eq!(state(&p.join(".env")), before);
}

// ── Refusals: nothing is written ─────────────────────────────────────────────

#[test]
fn duplicate_keys_are_refused_in_either_file() {
    let p = project(&[(
        ".env",
        &format!("DB_HOST=a{MARK}\nDB_PORT=1\nDB_HOST=b{MARK}\n"),
    )]);
    age(&p.join(".env"));
    let before = state(&p.join(".env"));
    align(&p, &[])
        .assert_code(2)
        .assert_contains("./.env: line 1: DB_HOST is assigned more than once (lines 1, 3)");
    assert_eq!(state(&p.join(".env")), before);

    let p = Project::new().example("A=\nB=\nA=\n").env("B=1\nA=2\n");
    let before = state(&p.join(".env"));
    align(&p, &[])
        .assert_code(2)
        .assert_contains("the template ./.env.example cannot be used")
        .assert_contains("A is assigned more than once");
    assert_eq!(state(&p.join(".env")), before);
}

#[test]
fn malformed_lines_are_refused_without_echoing_them() {
    let cases = [
        // An invalid key that is itself the secret. (`-`, `.`, `[` and `]` are
        // key characters, as in Docker Compose, so `/` makes it invalid.)
        format!("DB_HOST=h\n{MARK}/token=1\n"),
        format!("DB_HOST=h\nghp_{MARK}\n"),
        format!("DB_HOST=\"{MARK}\" trailing-{MARK}\n"),
        format!("DB_HOST=h\nDB_PORT=\"{MARK}\nnever closed\n"),
        format!("export ={MARK}\n"),
    ];
    for content in &cases {
        let p = project(&[(".env", content)]);
        age(&p.join(".env"));
        let before = state(&p.join(".env"));
        for mode in [&[][..], &["--check"], &["--dry-run"], &["--format", "json"]] {
            let out = align(&p, mode);
            out.assert_code(2).assert_contains("./.env: line ");
            assert!(out.stdout.is_empty(), "errors go to stderr: {}", out.stdout);
        }
        assert_eq!(state(&p.join(".env")), before, "{content:?}");
    }

    // The same, in the reference.
    let p = Project::new()
        .example(&format!("A=\n{MARK}/x=\n"))
        .env("A=1\n");
    align(&p, &[])
        .assert_code(2)
        .assert_contains("./.env.example: line 2: invalid key name (not shown)");
}

#[test]
fn every_target_is_checked_before_any_is_written() {
    let p = project(&[
        (".env.a", &format!("DB_PORT=1\nDB_HOST={MARK}\n")),
        (".env.b", &format!("DB_HOST=1\nbroken {MARK}\n")),
    ]);
    age(&p.join(".env.a"));
    let before = state(&p.join(".env.a"));
    align(&p, &["--env", ".env.a", "--env", ".env.b"])
        .assert_code(2)
        .assert_contains("./.env.b: line 2")
        .assert_not_contains("./.env.a:");
    assert_eq!(
        state(&p.join(".env.a")),
        before,
        "the valid file was not written"
    );
}

#[test]
fn a_missing_file_has_no_verdict() {
    let p = project(&[]);
    align(&p, &["--env", ".env.nope", "--check"])
        .assert_code(2)
        .assert_contains("./.env.nope not found.")
        .assert_contains("point at another file with --env FILE");
    let p = Project::new().env("A=1\n");
    align(&p, &["--check"])
        .assert_code(2)
        .assert_contains("template ./.env.example does not exist");
}

#[cfg(unix)]
#[test]
fn the_reference_under_another_name_is_refused() {
    let p = project(&[(".env", "DB_HOST=1\n")]).symlink(".env.example", "link.env");
    std::fs::hard_link(p.join(".env.example"), p.join("hard.env")).unwrap();
    let before = state(&p.join(".env.example"));
    for alias in ["link.env", "hard.env", "./.env.example", ".env.example"] {
        align(&p, &["--env", alias])
            .assert_code(2)
            .assert_contains("is the same file as the template");
    }
    assert_eq!(state(&p.join(".env.example")), before);

    std::fs::hard_link(p.join(".env"), p.join("again.env")).unwrap();
    align(&p, &["--env", ".env", "--env", "again.env"])
        .assert_code(2)
        .assert_contains("./.env and ./again.env are the same file");
}

// ── Writing ──────────────────────────────────────────────────────────────────

#[test]
fn check_and_dry_run_never_write() {
    let p = project(&[(".env", &format!("JWT_SECRET={MARK}\nDB_HOST=h\n"))]);
    age(&p.join(".env"));
    let before = state(&p.join(".env"));

    align(&p, &["--check"])
        .assert_code(1)
        .assert_contains("./.env  out of alignment")
        .assert_contains("Fix: evnx align --example ./.env.example --env ./.env");
    align(&p, &["--dry-run"])
        .assert_code(0)
        .assert_contains("./.env  would change")
        .assert_contains("no file was written");
    align(&p, &["--check", "--dry-run"]).assert_code(1);
    let json = align(&p, &["--check", "--format", "json"]);
    json.assert_code(1);
    let v: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(v["aligned"], false);
    assert_eq!(v["targets"][0]["status"], "out-of-alignment");
    assert_eq!(v["targets"][0]["moved"][0]["key"], "DB_HOST");
    assert_eq!(v["targets"][0]["missing"][0]["key"], "DB_PORT");
    assert!(v["targets"][0].get("added").is_none());

    assert_eq!(state(&p.join(".env")), before);
    let names: Vec<_> = std::fs::read_dir(p.path()).unwrap().collect();
    assert_eq!(names.len(), 2, "no temp file left behind");
}

#[test]
fn an_aligned_file_is_not_touched_and_alignment_is_idempotent() {
    let p = project(&[(".env", &format!("JWT_SECRET={MARK}\nLOCAL=1\n# x {MARK}\n"))]);
    align(&p, &[]).assert_code(0);
    let first = p.read(".env");
    age(&p.join(".env"));
    let before = state(&p.join(".env"));

    for _ in 0..2 {
        align(&p, &[])
            .assert_code(0)
            .assert_contains("./.env  already aligned");
    }
    align(&p, &["--check"]).assert_code(0);
    assert_eq!(state(&p.join(".env")), before, "content, mtime and inode");
    assert_eq!(p.read(".env"), first);
}

#[cfg(unix)]
#[test]
fn permissions_are_kept_and_a_symlink_is_written_through() {
    use std::os::unix::fs::PermissionsExt;
    let p = project(&[("real/.env.dev", "DB_PORT=1\nDB_HOST=2\n")])
        .mode("real/.env.dev", 0o640)
        .symlink("real/.env.dev", ".env.dev");
    align(&p, &["--env", ".env.dev"]).assert_code(0);
    let mode = std::fs::metadata(p.join("real/.env.dev"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640);
    assert!(std::fs::symlink_metadata(p.join(".env.dev"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(p.read("real/.env.dev").contains("DB_HOST=2\nDB_PORT=1\n"));
    let leftovers: Vec<_> = std::fs::read_dir(p.join("real"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers.len(), 1, "{leftovers:?}");
}

#[test]
fn defaults_come_from_the_project_config_and_env_name() {
    let p = Project::new()
        .file("layout.example", "B=\nA=\n")
        .toml("[defaults]\nexample = \"layout.example\"\n")
        .file(".env.staging", "A=1\nB=2\n");
    align(&p, &["--env-name", "staging"]).assert_code(0);
    assert_eq!(p.read(".env.staging"), "B=2\nA=1\n");
}

#[test]
fn quiet_prints_nothing_on_success_but_still_reports_drift() {
    let p = project(&[(".env", "DB_PORT=1\nDB_HOST=2\n")]);
    let out = run(p.path(), &["-q", "align", "--check"]);
    out.assert_code(1).assert_contains("out of alignment");
    let out = run(p.path(), &["-q", "align"]);
    out.assert_code(0);
    assert!(out.stdout.is_empty(), "{}", out.stdout);
}

#[test]
fn sync_is_unaffected_by_the_extras_heading() {
    // `sync --check` reads key names only; an aligned file is the same set.
    let p = project(&[(
        ".env",
        "LOCAL=1\nDB_HOST=a\nDB_PORT=b\nREDIS_URL=c\nJWT_SECRET=d\n",
    )]);
    let before = run(p.path(), &["sync", "--check", "--direction", "reverse"]);
    align(&p, &[]).assert_code(0);
    let after = run(p.path(), &["sync", "--check", "--direction", "reverse"]);
    assert_eq!(before.code, after.code);
}

// ── Backups ──────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn a_changed_file_is_kept_as_a_private_bak_first() {
    use std::os::unix::fs::PermissionsExt;
    let before = format!("DB_PORT=1\nDB_HOST={MARK}\n");
    let p = project(&[(".env", &before)]).mode(".env", 0o640);
    align(&p, &[])
        .assert_code(0)
        .assert_contains("    backup  ./.env.bak   undo: evnx undo .env");
    assert_eq!(p.read(".env.bak"), before);
    let mode = std::fs::metadata(p.join(".env.bak"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    // Every backup is 0600 whatever the original's mode (WP-7, VAL-2).
    assert_eq!(mode, 0o600, "the copy is private");

    // Already aligned: nothing written, so no second copy.
    align(&p, &[]).assert_code(0);
    assert!(!p.exists(".env.bak.2"));
}

#[test]
fn no_backup_keeps_no_copy_and_json_says_where_one_went() {
    let p = project(&[
        (".env", "DB_PORT=1\nDB_HOST=2\n"),
        (".env.b", "DB_PORT=1\nDB_HOST=2\n"),
    ]);
    align(&p, &["--no-backup"]).assert_code(0);
    assert!(!p.exists(".env.bak"));

    let out = align(&p, &["--env", ".env.b", "--format", "json"]);
    out.assert_code(0);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["targets"][0]["status"], "written");
    assert_eq!(v["targets"][0]["backup"], "./.env.b.bak");
}

#[test]
fn a_backup_git_would_see_is_git_ignored_for_you() {
    // `.gitignore` covers `.env` and not `.env.bak`. Since WP-7 every backup
    // adds `*.bak` itself, as for every command, instead of being refused.
    let p = project(&[
        (".env", &format!("DB_PORT=1\nDB_HOST={MARK}\n")),
        (".gitignore", ".env\n"),
    ])
    .git();
    align(&p, &[]).assert_code(0);
    assert!(p.exists(".env.bak") && p.is_ignored(".env.bak"));
    assert!(p.read(".gitignore").contains("*.bak"));

    // Listed by `evnx history`, and `evnx undo` puts it back.
    run(p.path(), &["history"])
        .assert_code(0)
        .assert_contains("align")
        .assert_not_contains(MARK);
    run(p.path(), &["undo", ".env", "--yes"])
        .assert_code(0)
        .assert_not_contains(MARK);
    assert_eq!(p.read(".env"), format!("DB_PORT=1\nDB_HOST={MARK}\n"));
}

// ── Which template: the same rule as every other command (core::resolve) ─────

/// The shared template, and a staging one with its own layout and a name the
/// shared one lacks.
fn per_environment() -> Project {
    Project::new()
        .example("A=\nB=\n")
        .file(".env.staging.example", "# Staging\nB=\nA=\nSTAGING_ONLY=\n")
        .file(
            ".env.staging",
            &format!("A=a-{MARK}\nSTAGING_ONLY=s-{MARK}\nB=b-{MARK}\n"),
        )
        .file(".env.prod", &format!("B=b-{MARK}\nA=a-{MARK}\n"))
}

#[test]
fn an_environment_follows_its_own_template_as_sync_pairs_it() {
    for args in [&["--env-name", "staging"][..], &["--env", ".env.staging"]] {
        let p = per_environment();
        align(&p, args)
            .assert_code(0)
            .assert_contains("files     .env.staging ↔ .env.staging.example")
            .assert_contains("Aligned 1 of 1 file to ./.env.staging.example")
            .assert_not_contains("extra");
        assert_eq!(
            p.read(".env.staging"),
            format!("# Staging\nB=b-{MARK}\nA=a-{MARK}\nSTAGING_ONLY=s-{MARK}\n")
        );
    }

    // `--example` wins over the environment's own template, as in `sync`.
    let p = per_environment();
    align(&p, &["--env-name", "staging", "--example", ".env.example"])
        .assert_code(0)
        .assert_contains("extra  STAGING_ONLY");
}

#[test]
fn files_with_different_templates_are_aligned_in_one_run() {
    let p = per_environment();
    let out = align(
        &p,
        &["--env-name", "staging", "--env-name", "prod", "--check"],
    );
    out.assert_code(1)
        .assert_contains("./.env.staging  out of alignment  (template ./.env.staging.example)")
        .assert_contains("./.env.prod  out of alignment  (template ./.env.example)")
        .assert_contains("2 of 2 files out of alignment with their templates.")
        .assert_contains("Fix: evnx align --example ./.env.staging.example --env ./.env.staging")
        .assert_contains("Fix: evnx align --example ./.env.example --env ./.env.prod");

    let out = align(
        &p,
        &[
            "--env-name",
            "staging",
            "--env-name",
            "prod",
            "--format",
            "json",
        ],
    );
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let pairs: Vec<(&str, &str)> = doc["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["file"].as_str().unwrap(), t["template"].as_str().unwrap()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("./.env.staging", "./.env.staging.example"),
            ("./.env.prod", "./.env.example")
        ]
    );
    assert_eq!(p.read(".env.prod"), format!("A=a-{MARK}\nB=b-{MARK}\n"));

    // A second run finds both aligned.
    align(
        &p,
        &["--env-name", "staging", "--env-name", "prod", "--check"],
    )
    .assert_code(0);
}

#[test]
fn a_file_that_is_another_files_template_is_refused() {
    let p = per_environment();
    // .env.staging follows .env.staging.example, so that file cannot also be
    // rewritten in the same run.
    let before = state(&p.join(".env.staging.example"));
    align(
        &p,
        &["--env-name", "staging", "--env", ".env.staging.example"],
    )
    .assert_code(2)
    .assert_contains("./.env.staging.example is the same file as the template")
    .assert_not_contains("Aligned");
    assert_eq!(state(&p.join(".env.staging.example")), before);
}

#[test]
fn no_env_file_says_which_environments_exist() {
    let p = Project::new()
        .example("A=\n")
        .file(".env.prod", &format!("A={MARK}\n"));
    align(&p, &[])
        .assert_code(2)
        .assert_contains(".env not found.")
        .assert_contains("Found: .env.prod")
        .assert_contains("evnx align --env-name prod");
    // A name with no file is refused before anything is read.
    align(&p, &["--env-name", "staging"]).assert_code(2);
}

/// Another run holding the file stops this one before any backup is taken,
/// as for `remove`: they share the writer.
#[test]
fn a_file_another_run_holds_is_left_alone() {
    let p = project(&[(".env", &format!("JWT_SECRET={MARK}\nDB_HOST=h\n"))]);
    let lock = p.join(".evnx-.env.lock");
    std::fs::write(&lock, format!("{}\n", std::process::id())).unwrap();
    let before = state(&p.join(".env"));
    align(&p, &[])
        .assert_code(2)
        .assert_contains("another evnx");
    assert_eq!(state(&p.join(".env")), before);
    assert!(!p.exists(".env.bak"));
    assert!(lock.exists(), "someone else's lock was removed");
}

/// One `.bak` per file, replaced by each write, as every command keeps it.
#[test]
fn a_second_write_replaces_the_backup() {
    let p = project(&[(".env", "JWT_SECRET=one\nDB_HOST=h\n")]);
    align(&p, &[]).assert_code(0);
    assert_eq!(p.read(".env.bak"), "JWT_SECRET=one\nDB_HOST=h\n");
    let now = p.read(".env");
    std::fs::write(p.join(".env"), "JWT_SECRET=two\nDB_HOST=h\n").unwrap();
    align(&p, &[]).assert_code(0);
    assert_eq!(p.read(".env.bak"), "JWT_SECRET=two\nDB_HOST=h\n");
    assert!(!p.exists(".env.bak.2"));
    assert_ne!(p.read(".env"), now, "the second edit was aligned too");
}
