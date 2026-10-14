//! `evnx convert -o FILE`: how the output file is written (I-081, U2 pilot).
//!
//! The converted file holds every value from `.env`. It used to be written with
//! `fs::write`: mode 0644 (world-readable), an existing file overwritten in
//! silence, and `-o .env` replaced the input with its own conversion. Now it
//! goes through `core::safe_write`:
//!
//! - a new file is 0600 (G9);
//! - an existing file is refused unless `--force` (DEC-G5), and then replaced
//!   atomically with "Replaced FILE", keeping its mode (DEC-G6);
//! - the input file is never the output, even with `--force`;
//! - a symlink is written through and says so (DEC-S6).

mod support;

use support::{run, Project};

fn project() -> Project {
    Project::new().env("DATABASE_URL=postgres://u:pw@db/app\nPORT=8080\n")
}

#[cfg(unix)]
fn mode(p: &Project, name: &str) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p.join(name))
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[test]
#[cfg(unix)]
fn t_i081_a_new_output_is_private() {
    let p = project();
    run(p.path(), &["convert", "--to", "json", "-o", "out.json"]).assert_code(0);
    assert_eq!(mode(&p, "out.json"), 0o600);
    assert!(p.read("out.json").contains("DATABASE_URL"));
}

#[test]
fn t_i081_an_existing_output_needs_force() {
    let p = project().file("out.json", "KEEP ME\n");
    run(p.path(), &["convert", "--to", "json", "-o", "out.json"])
        .assert_code(2)
        .assert_contains("--force");
    assert_eq!(p.read("out.json"), "KEEP ME\n", "nothing written");
}

#[test]
fn t_i081_force_replaces_and_says_so() {
    let p = project().file("out.json", "OLD\n");
    run(
        p.path(),
        &["convert", "--to", "json", "-o", "out.json", "--force"],
    )
    .assert_code(0)
    .assert_contains("Replaced out.json");
    assert!(p.read("out.json").contains("DATABASE_URL"));
}

#[test]
#[cfg(unix)]
fn t_i081_force_keeps_the_mode_and_warns_when_loose() {
    let p = project().file("out.json", "OLD\n").mode("out.json", 0o644);
    let out = run(
        p.path(),
        &["convert", "--to", "json", "-o", "out.json", "--force"],
    );
    out.assert_code(0);
    assert_eq!(
        mode(&p, "out.json"),
        0o644,
        "DEC-G6: an existing mode is kept"
    );
    assert!(
        out.stderr.contains("readable by other users"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("chmod 600 out.json"), "{}", out.stderr);
}

#[test]
fn t_i081_the_input_is_never_the_output() {
    for force in [false, true] {
        let p = project();
        let before = p.read(".env");
        let mut args = vec!["convert", "--to", "json", "-o", ".env"];
        if force {
            args.push("--force");
        }
        run(p.path(), &args)
            .assert_code(2)
            .assert_contains("same file");
        assert_eq!(p.read(".env"), before, "input untouched (force={force})");
    }
}

#[test]
#[cfg(unix)]
fn t_i081_the_input_by_another_name_is_refused() {
    let p = project();
    std::os::unix::fs::symlink(".env", p.join("link.json")).unwrap();
    std::fs::hard_link(p.join(".env"), p.join("hard.json")).unwrap();
    let before = p.read(".env");
    for out in ["link.json", "hard.json", "./.env"] {
        run(p.path(), &["convert", "--to", "json", "-o", out, "--force"])
            .assert_code(2)
            .assert_contains("same file");
    }
    assert_eq!(p.read(".env"), before);
}

#[test]
#[cfg(unix)]
fn t_i081_a_symlinked_output_is_written_through() {
    let p = project().dir("shared");
    std::fs::write(p.join("shared/real.json"), "OLD\n").unwrap();
    std::os::unix::fs::symlink("shared/real.json", p.join("out.json")).unwrap();
    run(
        p.path(),
        &["convert", "--to", "json", "-o", "out.json", "--force"],
    )
    .assert_code(0)
    .assert_contains("wrote through out.json -> shared/real.json");
    assert!(std::fs::symlink_metadata(p.join("out.json"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(p.read("shared/real.json").contains("DATABASE_URL"));
}

#[test]
fn t_i081_quiet_prints_nothing_on_success() {
    let p = project();
    let out = run(
        p.path(),
        &["-q", "convert", "--to", "json", "-o", "out.json"],
    );
    out.assert_code(0);
    assert!(out.all().trim().is_empty(), "{}", out.all());
}
