//! `evnx convert`: the R-convert fixes (0.10.9).
//!
//! One test per issue from the review (`local/convert/review/16-convert.md`,
//! `17-convert-targets-and-migrate.md`, `17a-target-format-status.md`), named
//! after it. Each was seen failing on 0.10.8 first.
//!
//! - I-082 `--help` carried developer notes.
//! - I-083 no terminal and no `--to`: "cancelled by user … not a terminal".
//! - I-084 fixed placeholders (`my-project`, `my-keyvault`, `YOUR_APP_NAME`,
//!   `app-secrets`); the names are now flags, required for their format
//!   (DEC-CV1).
//! - I-085, I-087 labels: github-actions is for the web form; doppler, vercel
//!   and railway are experimental (DEC-CV4).
//! - I-089 a filter that matches nothing, or an empty `.env`, printed `{}` and
//!   exited 0; now exit 2 unless `--allow-empty` (DEC-CV3).
//! - I-090 unknown `--transform` ignored; `--include` had no `?`, no `*` in the
//!   middle, no list (A-13: `core::glob`, as migrate).
//! - I-091 a missing `--env` file gave a nested error.
//! - I-099 the gcp script failed for a secret that already exists.
//! - I-249 a format's own renaming merged two names in silence (DEC-PL2,
//!   DEC-CV2).
//! - I-263 `--prefix` reached the generated scripts unquoted.
//! - I-081 follow-up: `-o` in a git repository checks `.gitignore`, as
//!   `template` does (R-template note).

mod support;

use support::{run, Project};

fn project() -> Project {
    Project::new().env("APP_NAME=demo app\nAPI_SECRET=abc123\nPORT=8080\n")
}

// ── I-082 ─────────────────────────────────────────────────────────────────────

/// `convert --help` showed two commit notes written for developers ("clap
/// strips the markdown fence…", "This said … which was exactly backwards").
/// Checked over every command, so the next one is caught too.
#[test]
fn t_i082_no_help_carries_developer_notes() {
    let p = Project::new();
    for cmd in [
        "init",
        "add",
        "template",
        "spec",
        "validate",
        "convert",
        "migrate",
        "backup",
        "restore",
        "sync",
        "diff",
        "scan",
        "doctor",
        "auth",
        "vault",
        "cloud",
        "org",
        "update",
        "completions",
    ] {
        let out = run(p.path(), &[cmd, "--help"]);
        for note in [
            "clap strips",
            "This said",
            "exactly backwards",
            "doc comment",
        ] {
            assert!(
                !out.stdout.contains(note),
                "`evnx {cmd} --help` carries a developer note ({note:?}):\n{}",
                out.stdout
            );
        }
    }
}

/// D-47: there are 14 formats, not "14+".
#[test]
fn t_i082_help_counts_the_formats() {
    let out = run(Project::new().path(), &["convert", "--help"]);
    assert!(!out.stdout.contains("14+"), "{}", out.stdout);
    assert!(out.stdout.contains("14 formats"), "{}", out.stdout);
}

// ── I-083 ─────────────────────────────────────────────────────────────────────

/// No terminal and no `--to`: one error that names `--to` and the formats, in
/// migrate's words, before any header is drawn.
#[test]
fn t_i083_no_terminal_names_to_and_the_formats() {
    let p = project();
    let out = run(p.path(), &["convert"]);
    out.assert_code(2)
        .assert_contains("--to is required when there is no terminal to ask")
        .assert_contains("evnx convert --to <FORMAT>")
        .assert_contains("json, yaml, shell")
        .assert_not_contains("cancelled by user")
        .assert_not_contains("not a terminal: not a terminal")
        .assert_not_contains("Transform .env into different formats");
    assert!(out.stdout.is_empty(), "{}", out.stdout);
}

// ── I-084 ─────────────────────────────────────────────────────────────────────

/// DEC-CV1: each format that names a place requires the flag for it. Exit 2,
/// the flag named, nothing printed — never a placeholder to edit by hand.
#[test]
fn t_i084_each_named_target_requires_its_flag() {
    let p = project();
    for (format, flag) in [
        ("gcp-secrets", "--project"),
        ("azure-keyvault", "--vault-name"),
        ("heroku", "--heroku-app"),
        ("kubernetes", "--name"),
    ] {
        let out = run(p.path(), &["convert", "--to", format]);
        out.assert_code(2).assert_contains(flag);
        assert!(out.stdout.is_empty(), "{format}: {}", out.stdout);
    }
}

#[test]
fn t_i084_the_names_reach_the_output() {
    let p = project();
    let gcp = run(
        p.path(),
        &["convert", "--to", "gcp-secrets", "--project", "acme-prod"],
    );
    gcp.assert_code(0);
    assert!(gcp.stdout.contains("--project=acme-prod"), "{}", gcp.stdout);
    assert!(!gcp.stdout.contains("my-project"), "{}", gcp.stdout);

    let az = run(
        p.path(),
        &[
            "convert",
            "--to",
            "azure-keyvault",
            "--vault-name",
            "kv-prod",
        ],
    );
    az.assert_code(0);
    assert!(az.stdout.contains("--vault-name kv-prod"), "{}", az.stdout);
    assert!(!az.stdout.contains("my-keyvault"), "{}", az.stdout);

    let heroku = run(
        p.path(),
        &["convert", "--to", "heroku", "--heroku-app", "acme-web"],
    );
    heroku.assert_code(0);
    assert!(
        heroku.stdout.contains("--app acme-web"),
        "{}",
        heroku.stdout
    );
    assert!(
        !heroku.stdout.contains("YOUR_APP_NAME"),
        "{}",
        heroku.stdout
    );

    let k8s = run(
        p.path(),
        &[
            "convert",
            "--to",
            "kubernetes",
            "--name",
            "web-env",
            "--namespace",
            "prod",
        ],
    );
    k8s.assert_code(0);
    assert!(k8s.stdout.contains("  name: web-env\n"), "{}", k8s.stdout);
    assert!(k8s.stdout.contains("  namespace: prod\n"), "{}", k8s.stdout);
    assert!(!k8s.stdout.contains("app-secrets"), "{}", k8s.stdout);

    // `--namespace` is optional: without it the manifest names none.
    let k8s = run(p.path(), &["convert", "--to", "k8s", "--name", "web-env"]);
    k8s.assert_code(0);
    assert!(!k8s.stdout.contains("namespace:"), "{}", k8s.stdout);
}

/// A name flag for a format that does not use it is a mistake, not a no-op.
#[test]
fn t_i084_a_name_flag_for_another_format_is_refused() {
    let p = project();
    run(
        p.path(),
        &["convert", "--to", "json", "--project", "acme-prod"],
    )
    .assert_code(2)
    .assert_contains("--project")
    .assert_contains("gcp-secrets");
    run(
        p.path(),
        &["convert", "--to", "yaml", "--namespace", "prod"],
    )
    .assert_code(2)
    .assert_contains("--namespace")
    .assert_contains("kubernetes");
}

/// Names are checked by the destination's own rule (`core::args`, as migrate)
/// before anything is printed: a name with shell syntax never reaches a script.
#[test]
fn t_i084_invalid_names_are_refused() {
    let p = project();
    for args in [
        &["--to", "heroku", "--heroku-app", "x;echo PWNED"][..],
        &["--to", "azure-keyvault", "--vault-name", "kv prod"][..],
        &["--to", "gcp-secrets", "--project", "Acme_Prod"][..],
        &["--to", "kubernetes", "--name", "Web_Env"][..],
        &[
            "--to",
            "kubernetes",
            "--name",
            "web",
            "--namespace",
            "prod.eu",
        ][..],
    ] {
        let mut full = vec!["convert"];
        full.extend_from_slice(args);
        let out = run(p.path(), &full);
        out.assert_code(2);
        assert!(out.stdout.is_empty(), "{args:?}: {}", out.stdout);
    }
}

// ── I-085, I-087 ──────────────────────────────────────────────────────────────

#[test]
fn t_i085_help_says_github_actions_is_for_the_web_form() {
    let out = run(Project::new().path(), &["convert", "--help"]);
    assert!(out.stdout.contains("web form"), "{}", out.stdout);
}

/// DEC-CV4: doppler, vercel and railway are experimental — in `--help`, and
/// on stderr when one is chosen. stdout stays the artefact alone.
#[test]
fn t_i087_experimental_formats_are_marked() {
    let help = run(Project::new().path(), &["convert", "--help"]);
    for name in ["doppler", "vercel", "railway"] {
        assert!(
            help.stdout.contains(&format!("{name} (experimental)")),
            "{name} not marked in --help:\n{}",
            help.stdout
        );
    }
    let p = project();
    for name in ["doppler", "vercel", "railway"] {
        let out = run(p.path(), &["convert", "--to", name]);
        out.assert_code(0);
        assert!(
            out.stderr.contains("experimental"),
            "{name}: {}",
            out.stderr
        );
        serde_json::from_str::<serde_json::Value>(&out.stdout)
            .unwrap_or_else(|e| panic!("{name}: stdout is not the JSON alone: {e}"));
    }
    // The verified formats say nothing of the kind.
    let out = run(p.path(), &["convert", "--to", "json"]);
    assert!(!out.stderr.contains("experimental"), "{}", out.stderr);
}

// ── I-089 ─────────────────────────────────────────────────────────────────────

/// DEC-CV3: a filter that matches nothing is almost always a typo, and an empty
/// upload is worse than an error. Exit 2, stdout empty, the filters named.
#[test]
fn t_i089_a_filter_that_matches_nothing_is_refused() {
    let p = project();
    let out = run(
        p.path(),
        &["convert", "--to", "json", "--include", "NOPE_*"],
    );
    out.assert_code(2)
        .assert_contains("left nothing to convert")
        .assert_contains("all 3 variable(s) were filtered out")
        .assert_contains("--allow-empty");
    assert!(out.stdout.is_empty(), "{}", out.stdout);
}

#[test]
fn t_i089_an_empty_env_is_refused() {
    let p = Project::new().env("# nothing here\n");
    let out = run(p.path(), &["convert", "--to", "json"]);
    out.assert_code(2)
        .assert_contains("holds no variables")
        .assert_contains("--allow-empty");
    assert!(out.stdout.is_empty(), "{}", out.stdout);

    // Nothing is written either.
    run(p.path(), &["convert", "--to", "json", "-o", "out.json"]).assert_code(2);
    assert!(!p.exists("out.json"));
}

#[test]
fn t_i089_allow_empty_gives_the_empty_output() {
    let p = project();
    let out = run(
        p.path(),
        &[
            "convert",
            "--to",
            "json",
            "--include",
            "NOPE_*",
            "--allow-empty",
        ],
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "{}\n");

    let p = Project::new().env("");
    run(p.path(), &["convert", "--to", "yaml", "--allow-empty"]).assert_code(0);
}

// ── I-090 ─────────────────────────────────────────────────────────────────────

/// An unknown mode was ignored, so `--transform Lowercase`'s typo cousin
/// produced the input's names and exit 0. The one list rule (`core::args`).
#[test]
fn t_i090_an_unknown_transform_is_refused() {
    let p = project();
    let out = run(
        p.path(),
        &["convert", "--to", "json", "--transform", "Title"],
    );
    out.assert_code(2)
        .assert_contains("unknown transform 'title'")
        .assert_contains("uppercase, lowercase, camelCase, snake_case");
    assert!(out.stdout.is_empty(), "{}", out.stdout);

    // Case does not matter for a valid one, as for every other list flag.
    run(
        p.path(),
        &["convert", "--to", "json", "--transform", "CamelCase"],
    )
    .assert_code(0)
    .assert_contains("\"appName\"");
}

/// The keys of a JSON object, sorted (serde_json's map does not keep order).
fn keys(stdout: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(stdout).unwrap();
    v.as_object().unwrap().keys().cloned().collect()
}

/// A-13: `--include`/`--exclude` are `core::glob`, as in migrate and cloud run:
/// `*` anywhere, `?` for one character, a comma list or a repeated flag,
/// exclude wins, case-sensitive.
#[test]
fn t_i090_include_and_exclude_use_core_glob() {
    let p = Project::new().env(
        "PORT=1\nDB_PRIMARY_URL=2\nDB_PRIMARY_HOST=3\nAPP_NAME=4\nAPP_DEBUG=5\nport_lower=6\n",
    );
    let k = |args: &[&str]| {
        let mut full = vec!["convert", "--to", "json"];
        full.extend_from_slice(args);
        let out = run(p.path(), &full);
        out.assert_code(0);
        keys(&out.stdout)
    };
    assert_eq!(k(&["--include", "P?RT"]), ["PORT"]);
    assert_eq!(k(&["--include", "DB_*_URL"]), ["DB_PRIMARY_URL"]);
    assert_eq!(k(&["--include", "PORT,APP_NAME"]), ["APP_NAME", "PORT"]);
    assert_eq!(
        k(&["--include", "PORT", "--include", "APP_NAME"]),
        ["APP_NAME", "PORT"]
    );
    assert_eq!(
        k(&["--include", "APP_*", "--exclude", "*_DEBUG"]),
        ["APP_NAME"]
    );
    assert_eq!(k(&["--exclude", "DB_*,APP_*"]), ["PORT", "port_lower"]);
    // Case-sensitive: `port*` matches only the lowercase name.
    assert_eq!(k(&["--include", "port*"]), ["port_lower"]);
}

// ── I-091 ─────────────────────────────────────────────────────────────────────

/// One error, the wording `validate` and `diff` give, and no
/// "Convert command failed:" in front of it or of any other convert error.
#[test]
fn t_i091_a_missing_env_file_is_one_error() {
    let p = Project::new();
    let out = run(p.path(), &["convert", "--to", "json", "--env", "nope.env"]);
    out.assert_code(2)
        .assert_contains("nope.env does not exist")
        .assert_contains("Create it with `evnx init`, or point at another file with --env.")
        .assert_not_contains("Convert command failed")
        .assert_not_contains("Failed to validate input file")
        .assert_not_contains("Input file not found");
}

#[test]
fn t_i091_no_convert_error_carries_a_prefix() {
    let p = project().file("out.json", "KEEP\n");
    for args in [
        &["--to", "json", "-o", "out.json"][..],
        &["--to", "json", "-o", ".env"][..],
        &["--to", "json", "--transform", "nope"][..],
    ] {
        let mut full = vec!["convert"];
        full.extend_from_slice(args);
        run(p.path(), &full)
            .assert_code(2)
            .assert_not_contains("Convert command failed");
    }
}

// ── I-099 ─────────────────────────────────────────────────────────────────────

/// Run the generated gcp script against a stub `gcloud` that logs each call and
/// its stdin. `describe_rc` is what `gcloud secrets describe` answers: 0 when
/// the secret exists.
#[cfg(unix)]
fn run_gcp_script(describe_rc: i32) -> String {
    let p = Project::new().env("PORT=8080\n");
    let out = run(
        p.path(),
        &[
            "convert",
            "--to",
            "gcp-secrets",
            "--project",
            "acme-prod",
            "-o",
            "up.sh",
        ],
    );
    out.assert_code(0);
    std::fs::create_dir(p.join("bin")).unwrap();
    let stub = p.join("bin/gcloud");
    support::stub::write_stub(
        &stub,
        "#!/bin/sh\n\
         echo \"ARGS $*\" >> \"$LOG\"\n\
         if [ \"$1 $2\" = \"secrets describe\" ]; then exit \"$DESCRIBE_RC\"; fi\n\
         printf 'STDIN ' >> \"$LOG\"; cat >> \"$LOG\"; echo >> \"$LOG\"\n",
    );
    let log = p.join("gcloud.log");
    let status = std::process::Command::new("bash")
        .arg(p.join("up.sh"))
        .current_dir(p.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                p.join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("LOG", &log)
        .env("DESCRIBE_RC", describe_rc.to_string())
        .status()
        .unwrap();
    assert!(status.success(), "the script failed");
    std::fs::read_to_string(log).unwrap()
}

#[test]
#[cfg(unix)]
fn t_i099_gcp_creates_a_secret_that_does_not_exist() {
    let log = run_gcp_script(1);
    assert!(
        log.contains("ARGS secrets create port --data-file=- --project=acme-prod"),
        "{log}"
    );
    assert!(
        log.contains("STDIN 8080\n"),
        "the value, without a newline: {log}"
    );
    assert!(!log.contains("versions add"), "{log}");
}

#[test]
#[cfg(unix)]
fn t_i099_gcp_adds_a_version_to_a_secret_that_exists() {
    let log = run_gcp_script(0);
    assert!(
        log.contains("ARGS secrets versions add port --data-file=- --project=acme-prod"),
        "{log}"
    );
    assert!(log.contains("STDIN 8080\n"), "{log}");
    assert!(!log.contains("secrets create"), "{log}");
}

// ── I-249 ─────────────────────────────────────────────────────────────────────

fn colliding() -> Project {
    Project::new().env("A_B=one\na_b=two\nPORT=8080\n")
}

/// The format's own renaming merges `A_B` and `a_b`: gcp lowercases and
/// hyphenates, terraform lowercases, Key Vault and GitHub ignore case. Refused
/// like a `--transform` collision (DEC-PL2), naming both and the format.
#[test]
fn t_i249_a_format_rename_that_merges_names_is_refused() {
    let p = colliding();
    for args in [
        &["--to", "gcp-secrets", "--project", "acme-prod"][..],
        &["--to", "azure-keyvault", "--vault-name", "kv-prod"][..],
        &["--to", "terraform"][..],
        &["--to", "github-actions"][..],
    ] {
        let mut full = vec!["convert"];
        full.extend_from_slice(args);
        let out = run(p.path(), &full);
        out.assert_code(2)
            .assert_contains("A_B")
            .assert_contains("a_b")
            .assert_contains(args[1])
            .assert_contains("--allow-collisions");
        assert!(out.stdout.is_empty(), "{args:?}: {}", out.stdout);
    }
}

/// Formats that keep names as they are have two distinct keys: no collision.
#[test]
fn t_i249_formats_that_keep_names_do_not_collide() {
    let p = colliding();
    for args in [
        &["--to", "json"][..],
        &["--to", "yaml"][..],
        &["--to", "shell"][..],
        &["--to", "docker-compose"][..],
        &["--to", "kubernetes", "--name", "web-env"][..],
    ] {
        let mut full = vec!["convert"];
        full.extend_from_slice(args);
        run(p.path(), &full).assert_code(0);
    }
}

/// DEC-PL2: `--allow-collisions` keeps the last value, says so on stderr, and
/// the script names the secret once.
#[test]
fn t_i249_allow_collisions_keeps_the_last_value() {
    let p = colliding();
    let out = run(
        p.path(),
        &[
            "convert",
            "--to",
            "gcp-secrets",
            "--project",
            "acme-prod",
            "--allow-collisions",
        ],
    );
    out.assert_code(0);
    let lines: Vec<&str> = out.stdout.lines().filter(|l| l.contains(" a-b")).collect();
    assert_eq!(lines.len(), 1, "{}", out.stdout);
    assert!(
        lines[0].contains("'two'"),
        "the last value wins: {}",
        out.stdout
    );
    assert!(!out.stdout.contains("'one'"), "{}", out.stdout);
    assert!(out.stderr.contains("a_b"), "{}", out.stderr);

    // The same flag for a `--transform` collision (I-088).
    let p = Project::new().env("Api=1\nAPI=2\n");
    let out = run(
        p.path(),
        &[
            "convert",
            "--to",
            "json",
            "--transform",
            "uppercase",
            "--allow-collisions",
        ],
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "{\n  \"API\": \"2\"\n}\n");
}

// ── I-263 ─────────────────────────────────────────────────────────────────────

/// `--prefix` is copied into every name, and the gcp, azure, heroku and shell
/// scripts print names bare: `--prefix 'X;touch PWNED;'` ran `touch` when the
/// script ran. A prefix must keep names variable names.
#[test]
fn t_i263_prefix_must_keep_names_variable_names() {
    let p = project();
    for bad in ["X;touch PWNED;", "APP-", "1X_", "A B"] {
        let out = run(p.path(), &["convert", "--to", "shell", "--prefix", bad]);
        out.assert_code(2).assert_contains("--prefix");
        assert!(out.stdout.is_empty(), "{bad:?}: {}", out.stdout);
    }
    run(
        p.path(),
        &["convert", "--to", "shell", "--prefix", "MYAPP_"],
    )
    .assert_code(0)
    .assert_contains("export MYAPP_PORT='8080'");
    run(p.path(), &["convert", "--to", "json", "--prefix", "_x"]).assert_code(0);
}

// ── I-081 follow-up: -o and .gitignore (R-template note) ─────────────────────

/// The output holds every value; in a git repository it is checked against
/// `.gitignore` the way `template --output` is (`utils::protect`). No
/// terminal: a warning, nothing changed.
#[test]
fn t_i081_output_in_a_git_repo_is_checked_against_gitignore() {
    let p = project().git();
    let out = run(p.path(), &["convert", "--to", "json", "-o", "secrets.json"]);
    out.assert_code(0)
        .assert_contains("'secrets.json' may contain secrets and is not in .gitignore");
    assert!(!p.exists(".gitignore"));
}

#[test]
fn t_i081_gitignore_flag_adds_the_entry() {
    let p = project().git();
    run(
        p.path(),
        &[
            "convert",
            "--to",
            "json",
            "-o",
            "secrets.json",
            "--gitignore",
        ],
    )
    .assert_code(0)
    .assert_contains("Added 'secrets.json' to .gitignore");
    assert!(p.is_ignored("secrets.json"));
}

#[test]
fn t_i081_no_gitignore_flag_is_silent() {
    let p = project().git();
    run(
        p.path(),
        &[
            "convert",
            "--to",
            "json",
            "-o",
            "secrets.json",
            "--no-gitignore",
        ],
    )
    .assert_code(0)
    .assert_not_contains("gitignore");
}

/// The input named as the output: template's wording, one rule (owner note,
/// R-template).
#[test]
fn t_i081_the_same_file_refusal_matches_template() {
    let p = project();
    run(p.path(), &["convert", "--to", "json", "-o", ".env"])
        .assert_code(2)
        .assert_contains("--output '.env' is the same file as the env file '.env'")
        .assert_contains("Nothing was written; choose another --output.");
}

// ── D-45 … D-53: the README's examples run ────────────────────────────────────

/// Split a command line on spaces, honouring double quotes (enough for the
/// README's examples).
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Every one-line `evnx convert` example in the README's convert section runs
/// and exits 0 — the docs promised `14+` formats, examples that printed `{}`,
/// and a pipe nobody had run (D-45, D-46, D-47, D-53). An optional part in
/// `[brackets]` is left out; a trailing `# comment` is not part of the command.
#[test]
fn t_d45_readme_convert_examples_run() {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let section = readme
        .split("### `evnx convert`")
        .nth(1)
        .and_then(|s| s.split("### `evnx sync`").next())
        .expect("README has a convert section");
    assert!(!section.contains("14+"), "D-47: there are 14 formats");

    let p = Project::new()
        .env("DB_HOST=db\nDATABASE_URL=postgres://u:p@db/app\nAPI_KEY=abc\nPORT=8080\n");
    let mut ran = 0;
    for line in section.lines().map(str::trim) {
        if !line.starts_with("evnx convert") || line.ends_with('\\') {
            continue;
        }
        let line = line.split(" #").next().unwrap();
        let line: String = line
            .split('[')
            .enumerate()
            .map(|(i, part)| {
                if i == 0 {
                    part
                } else {
                    part.split(']').nth(1).unwrap_or("")
                }
            })
            .collect();
        let args = words(&line);
        let args: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
        let out = run(p.path(), &args);
        assert_eq!(
            out.code,
            Some(0),
            "README example failed: {line}\n{}",
            out.stderr
        );
        ran += 1;
    }
    assert!(ran >= 14, "only {ran} examples found");
}

/// `migrate --to gcp-secret-manager` suggests convert's script; since
/// `--project` is required (DEC-CV1) the suggestion must carry it, or the
/// command it prints exits 2.
#[test]
#[cfg(feature = "migrate")]
fn t_i084_migrate_suggests_a_convert_command_that_runs() {
    let p = Project::new().env("PORT=8080\n");
    let out = run(p.path(), &["migrate", "--to", "gcp-secret-manager"]);
    out.assert_code(0)
        .assert_contains("evnx convert --to gcp-secrets --project <PROJECT>");
}
