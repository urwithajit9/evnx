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

// ---- WP-11c: more names that start a program, proxy and CA warnings ----------

/// The names the child must not be handed without `--allow-unsafe-names`.
const NEW_REFUSED: &[&str] = &[
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "PAGER",
    "GIT_PAGER",
    "VISUAL",
    "EDITOR",
    "RUSTC_WRAPPER",
    "CLASSPATH",
    "PERL5DB",
    "PYTHONINSPECT",
    "npm_config_registry",
    "NPM_CONFIG_REGISTRY",
    "GIT_CONFIG_COUNT",
];

const SECRET: &str = "zq-hijack-secret-value-93";

#[test]
fn u1_each_new_name_refused_in_run() {
    for name in NEW_REFUSED {
        let p = Project::new().env(&format!("{name}={SECRET}\n"));
        let marker = p.path().join("ran");
        let script = format!("touch {}", marker.display());
        let out = run(p.path(), &["run", "--", "sh", "-c", &script]);
        out.assert_code(2);
        assert!(!marker.exists(), "{name}: the child ran");
        assert!(out.stderr.contains(name), "{name}: {}", out.stderr);
        assert!(
            !out.all().contains(SECRET),
            "{name}: a value was printed: {}",
            out.all()
        );
    }
}

#[test]
fn u1_the_message_says_editor_and_pager_start_a_program() {
    let p = Project::new().env(&format!("EDITOR={SECRET}\n"));
    let out = run(p.path(), &["run", "--", "true"]);
    out.assert_code(2);
    assert!(out.stderr.contains("start a program"), "{}", out.stderr);
    // A name that is unsafe for another reason does not get the sentence.
    let p = Project::new().env("LD_PRELOAD=/tmp/x.so\n");
    let out = run(p.path(), &["run", "--", "true"]);
    assert!(!out.stderr.contains("start a program"), "{}", out.stderr);
}

#[test]
fn u1_allow_unsafe_names_lets_editor_through() {
    let p = Project::new().env(&format!("EDITOR={SECRET}\n"));
    let out = run(
        p.path(),
        &[
            "run",
            "--allow-unsafe-names",
            "--no-mask",
            "--",
            "sh",
            "-c",
            "printf '%s' \"$EDITOR\"",
        ],
    );
    out.assert_code(0);
    assert_eq!(out.stdout, SECRET);
}

#[test]
fn u1_near_miss_names_are_not_refused() {
    let p = Project::new().env("EDITORIAL=1\nMY_PAGER=2\nNPM_CONFIGURATION=3\n");
    run(p.path(), &["run", "--", "true"]).assert_code(0);
}

#[test]
fn u1_proxy_name_warns_and_child_runs() {
    let p = Project::new().env(&format!(
        "HTTPS_PROXY=http://user:{SECRET}@proxy.local:3128\n"
    ));
    let out = run(
        p.path(),
        &[
            "run",
            "--",
            "sh",
            "-c",
            "test -n \"$HTTPS_PROXY\" && echo ran",
        ],
    );
    out.assert_code(0);
    assert!(out.stdout.contains("ran"), "{}", out.stdout);
    assert!(out.stderr.contains("HTTPS_PROXY"), "{}", out.stderr);
    assert!(!out.all().contains(SECRET), "{}", out.all());
    assert!(!out.all().contains("proxy.local"), "{}", out.all());
}

#[test]
fn u1_ca_name_warns_too() {
    let p = Project::new().env("SSL_CERT_FILE=/tmp/ca.pem\nNODE_EXTRA_CA_CERTS=/tmp/x.pem\n");
    let out = run(p.path(), &["run", "--", "true"]);
    out.assert_code(0);
    assert!(out.stderr.contains("SSL_CERT_FILE"), "{}", out.stderr);
    assert!(out.stderr.contains("NODE_EXTRA_CA_CERTS"), "{}", out.stderr);
    assert!(!out.all().contains("/tmp/ca.pem"), "{}", out.all());
}

#[test]
fn u1_proxy_warning_hidden_by_quiet() {
    let p = Project::new().env("HTTPS_PROXY=http://proxy.local:3128\n");
    let out = run(p.path(), &["-q", "run", "--", "true"]);
    out.assert_code(0);
    assert!(!out.stderr.contains("HTTPS_PROXY"), "{}", out.stderr);
}

#[test]
fn a1_empty_command_is_usage_exit_2_no_footer() {
    let p = Project::new().env(ENV);
    let out = run(p.path(), &["run", "--"]);
    out.assert_code(2);
    assert!(
        out.stderr.contains("Usage: evnx run -- <cmd>"),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("stopped before"), "{}", out.stderr);
}

#[test]
fn a1_no_command_at_all_is_the_same_usage_error() {
    let p = Project::new().env(ENV);
    let out = run(p.path(), &["run"]);
    out.assert_code(2);
    assert!(
        out.stderr.contains("Usage: evnx run -- <cmd>"),
        "{}",
        out.stderr
    );
}

#[test]
fn a1_missing_double_dash_is_refused_and_the_usage_shows_it() {
    // clap's own text; its Usage line already shows where `--` goes.
    let p = Project::new().env(ENV);
    let marker = p.path().join("ran");
    let out = run(p.path(), &["run", "touch", marker.to_str().unwrap()]);
    out.assert_code(2);
    assert!(out.stderr.contains("-- <COMMAND>"), "{}", out.stderr);
    assert!(!marker.exists());
}

#[test]
fn a1_dash_flags_pass_through_to_the_child() {
    let p = Project::new().env(ENV);
    let out = run(
        p.path(),
        &["run", "--", "sh", "-c", "echo \"$1\"", "x", "--flag"],
    );
    out.assert_code(0);
    assert!(out.stdout.contains("--flag"), "{}", out.stdout);
}

#[test]
fn j1_json_and_format_json_are_rejected() {
    let p = Project::new().env(ENV);
    run(p.path(), &["run", "--json", "--", "true"]).assert_code(2);
    run(p.path(), &["run", "--format", "json", "--", "true"]).assert_code(2);
}

#[test]
fn the_help_documents_masking_limits_and_pythonunbuffered() {
    let p = Project::new();
    let out = run(p.path(), &["run", "--help"]);
    out.assert_code(0);
    for needle in ["PYTHONUNBUFFERED=1", "/proc/<pid>/environ", "core dumps"] {
        assert!(out.stdout.contains(needle), "{needle}: {}", out.stdout);
    }
}
