//! `evnx run` (RM-43): a local `.env` into a command's environment, with the
//! secrets masked in its output when that output is not a terminal (owner,
//! 2026-10-28). Under the test harness stdout is a pipe, which is that case.
#![cfg(unix)]

mod support;

use support::{run, Project};

const ENV: &str = "PORT=8080\nDB_PASSWORD=hunter22hunter\n";

#[test]
fn rm43_the_command_gets_the_variables_and_its_output_is_masked() {
    let p = Project::new().env(ENV);
    let out = run(
        p.path(),
        &[
            "run",
            "--",
            "sh",
            "-c",
            "echo port=$PORT pw=$DB_PASSWORD; echo err=$DB_PASSWORD >&2",
        ],
    );
    out.assert_code(0);
    assert!(out.stdout.contains("port=8080 pw=***"), "{}", out.stdout);
    assert!(out.stderr.contains("err=***"), "{}", out.stderr);
    assert!(!out.all().contains("hunter22hunter"), "{}", out.all());
}

#[test]
fn rm43_no_mask_shows_the_values() {
    let p = Project::new().env(ENV);
    let out = run(
        p.path(),
        &["run", "--no-mask", "--", "sh", "-c", "echo pw=$DB_PASSWORD"],
    );
    assert!(out.stdout.contains("pw=hunter22hunter"), "{}", out.stdout);
}

#[test]
fn rm43_the_exit_code_is_the_commands() {
    let p = Project::new().env(ENV);
    run(p.path(), &["run", "--", "sh", "-c", "exit 7"]).assert_code(7);
}

#[test]
fn rm43_unsafe_names_are_refused_naming_the_file() {
    let p = Project::new().env("LD_PRELOAD=/tmp/x.so\n");
    let out = run(p.path(), &["run", "--", "true"]);
    out.assert_code(2);
    assert!(
        out.stderr.contains(".env sets LD_PRELOAD"),
        "{}",
        out.stderr
    );
}

#[test]
fn rm43_include_narrows_what_the_command_gets() {
    let p = Project::new().env(ENV);
    let out = run(
        p.path(),
        &[
            "run",
            "--include",
            "PORT",
            "--",
            "sh",
            "-c",
            "[ -z \"${DB_PASSWORD+set}\" ] && echo only-port=$PORT",
        ],
    );
    out.assert_code(0);
    assert!(out.stdout.contains("only-port=8080"), "{}", out.stdout);
}
