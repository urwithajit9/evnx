//! `evnx spec init` and the `[vars]` contract after 0.10.5 (package R-spec,
//! 0.10.6): what the generated contract says, where it is written, which inputs
//! it reads, and how the config that holds it is read back.
//!
//! One test per issue or decision, named after it. Review: the package's
//! `local/spec/review/23-spec.md` (decisions DEC-SP2, SP4, SP5, SP7..SP10,
//! SP13..SP16).

mod support;

use support::{run, Project};

/// The `[vars.NAME]` table for `name` in `text`, up to the next table.
fn table<'a>(text: &'a str, name: &str) -> &'a str {
    let head = format!("[vars.{name}]");
    let start = text
        .find(&head)
        .unwrap_or_else(|| panic!("no {head} in:\n{text}"));
    let rest = &text[start + head.len()..];
    let end = rest.find("\n[").map(|i| i + 1).unwrap_or(rest.len());
    &rest[..end]
}

/// Names of the `[vars.*]` tables, in file order.
fn names(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|l| l.strip_prefix("[vars.")?.strip_suffix(']'))
        .collect()
}

// ── I-180, DEC-SP4: a format is written only when the sample fits it ────────

const AWKWARD: &str = "PORT=0\nREDIS_PORT=70000\nDEBUG=2\nUSER_ID=18446744073709551616000\nOK=5\n";

#[test]
fn t_i180_a_generated_spec_validates_its_own_project() {
    let p = Project::new().dir(".git").env(AWKWARD).example(AWKWARD);
    run(p.path(), &["spec", "init"]).assert_code(0);

    let t = p.read(".evnx.toml");
    for name in ["PORT", "REDIS_PORT", "DEBUG", "USER_ID"] {
        assert!(
            !table(&t, name).contains("format"),
            "{name} kept a format its own value fails:\n{t}"
        );
    }
    assert!(table(&t, "OK").contains("format = \"int\""), "{t}");

    // The point of the decision: the first validate after spec init is green.
    run(p.path(), &["validate"]).assert_code(0);
}

#[test]
fn t_i180_each_left_out_format_is_named_on_stderr_without_the_value() {
    let p = Project::new().dir(".git").env(AWKWARD).example(AWKWARD);
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(0);
    toml::from_str::<toml::Table>(&out.stdout).expect("stdout is TOML only");

    for name in ["PORT", "REDIS_PORT", "DEBUG", "USER_ID"] {
        assert!(
            out.stderr
                .lines()
                .any(|l| l.contains(name) && l.contains("format")),
            "{name} not named:\n{}",
            out.stderr
        );
    }
    assert!(!out.stderr.contains("OK"), "{}", out.stderr);
    // ⚠️ A sample value can be a secret: the note says what it is not, never
    // what it is.
    assert!(!out.stderr.contains("70000"), "{}", out.stderr);
}

// ── A1: one list of boolean words (core::value::parse_bool) ──────────────────

#[test]
fn t_bool_format_accepts_the_shared_words() {
    let p = Project::new()
        .dir(".git")
        .env("A=on\nB=OFF\nC=y\n")
        .example("A=\nB=\nC=\n")
        .toml(
            "[vars.A]\nformat = \"bool\"\n[vars.B]\nformat = \"bool\"\n[vars.C]\nformat = \"bool\"\n",
        );
    run(p.path(), &["validate"]).assert_code(0);
}

#[test]
fn t_bool_words_are_inferred_as_bool() {
    let p = Project::new()
        .dir(".git")
        .env("FEATURE_X=on\nCOUNT=1\n")
        .example("FEATURE_X=\nCOUNT=\n");
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(0);
    assert!(
        table(&out.stdout, "FEATURE_X").contains("format = \"bool\""),
        "{}",
        out.stdout
    );
    // `1` is still an int unless the name says boolean.
    assert!(
        table(&out.stdout, "COUNT").contains("format = \"int\""),
        "{}",
        out.stdout
    );
}

// ── I-177, DEC-SP2: the contract goes into the project's one .evnx.toml ─────

#[test]
fn t_i177_from_a_subdirectory_the_root_config_is_written() {
    let root = "[scan]\nseverity = \"high\"\n";
    let p = Project::new()
        .dir(".git")
        .toml(root)
        .file("sub/.env.example", "A=\n")
        .file("sub/.env", "A=1\n");
    let out = run(&p.join("sub"), &["spec", "init"]);
    out.assert_code(0);

    assert!(!p.exists("sub/.evnx.toml"), "a second config was created");
    let t = p.read(".evnx.toml");
    assert!(t.starts_with(root), "root config rewritten:\n{t}");
    assert!(t.contains("[vars.A]"), "{t}");
    assert!(out.all().contains("../.evnx.toml"), "{}", out.all());
}

#[test]
fn t_i177_with_no_config_it_is_created_at_the_git_root() {
    let p = Project::new()
        .dir(".git")
        .file("sub/.env.example", "A=\n")
        .file("sub/.env", "A=1\n");
    run(&p.join("sub"), &["spec", "init"]).assert_code(0);
    assert!(p.exists(".evnx.toml"), "not created at the git root");
    assert!(!p.exists("sub/.evnx.toml"));
}

#[test]
fn t_i177_the_vars_guard_reads_the_root_config() {
    let root = "[vars.A]\nrequired = false\n";
    let p = Project::new()
        .dir(".git")
        .toml(root)
        .file("sub/.env.example", "A=\nB=\n")
        .file("sub/.env", "A=1\nB=2\n");
    let out = run(&p.join("sub"), &["spec", "init"]);
    out.assert_code(2);
    assert!(out.all().contains("--merge"), "{}", out.all());
    assert_eq!(p.read(".evnx.toml"), root);
    assert!(!p.exists("sub/.evnx.toml"));

    run(&p.join("sub"), &["spec", "init", "--merge"]).assert_code(0);
    let t = p.read(".evnx.toml");
    assert!(t.starts_with(root), "{t}");
    assert!(t.contains("[vars.B]"), "{t}");
}

// ── I-189, DEC-SP2: [defaults] and --env-name, as validate reads them ───────

#[test]
fn t_i189_defaults_example_is_honoured() {
    let p = Project::new()
        .dir(".git")
        .toml("[defaults]\nexample = \"env.sample\"\n")
        .file("env.sample", "SAMPLE_ONLY=\n");
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(0);
    assert!(out.stdout.contains("[vars.SAMPLE_ONLY]"), "{}", out.all());
}

#[test]
fn t_i189_env_name_reads_that_environment() {
    let p = Project::new()
        .dir(".git")
        .file(".env.staging", "STAGING_ONLY=1\n");
    let out = run(
        p.path(),
        &["spec", "init", "--env-name", "staging", "--stdout"],
    );
    out.assert_code(0);
    assert!(out.stdout.contains("[vars.STAGING_ONLY]"), "{}", out.all());
}

#[test]
fn t_i189_defaults_env_name_is_honoured() {
    let p = Project::new()
        .dir(".git")
        .toml("[defaults]\nenv_name = \"staging\"\n")
        .file(".env.staging", "STAGING_ONLY=1\n");
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(0);
    assert!(out.stdout.contains("[vars.STAGING_ONLY]"), "{}", out.all());
}

#[test]
fn t_i189_an_env_name_with_no_file_is_exit_2() {
    let p = Project::new().dir(".git").env("A=1\n");
    let out = run(
        p.path(),
        &["spec", "init", "--env-name", "nothere", "--stdout"],
    );
    out.assert_code(2);
    assert!(out.stdout.is_empty(), "{}", out.stdout);
}

// ── I-182, I-186, DEC-SP8: explicit inputs, flag conflicts, exit codes ──────

#[test]
fn t_i182_an_explicit_env_that_does_not_exist_is_exit_2() {
    let p = Project::new().dir(".git").example("A=\n");
    let out = run(p.path(), &["spec", "init", "--env", "nope.env"]);
    out.assert_code(2);
    assert!(out.all().contains("nope.env"), "{}", out.all());
    assert!(!p.exists(".evnx.toml"));
}

#[test]
fn t_i182_an_explicit_example_that_does_not_exist_is_exit_2() {
    let p = Project::new().dir(".git").env("A=1\n");
    let out = run(
        p.path(),
        &["spec", "init", "--example", "missing", "--stdout"],
    );
    out.assert_code(2);
    assert!(out.all().contains("missing"), "{}", out.all());
    assert!(out.stdout.is_empty(), "{}", out.stdout);
}

#[test]
fn t_i182_a_configured_example_that_does_not_exist_is_exit_2() {
    let p = Project::new()
        .dir(".git")
        .toml("[defaults]\nexample = \"env.sample\"\n")
        .env("A=1\n");
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(2);
    assert!(out.all().contains("env.sample"), "{}", out.all());
}

/// The defaults may be absent: a project with only `.env`, or only a template.
#[test]
fn t_i182_absent_default_files_are_still_fine() {
    let p = Project::new().dir(".git").env("A=1\n");
    run(p.path(), &["spec", "init", "--stdout"]).assert_code(0);
    let p = Project::new().dir(".git").example("A=\n");
    run(p.path(), &["spec", "init", "--stdout"]).assert_code(0);
}

#[test]
fn t_i182_with_and_project_inputs_cannot_be_combined() {
    let p = Project::new().dir(".git").env("A=1\n").example("A=\n");
    for extra in [
        &["--env", ".env"][..],
        &["--example", ".env.example"],
        &["--env-name", "staging"],
    ] {
        let mut args = vec!["spec", "init", "--with", "django", "--stdout"];
        args.extend_from_slice(extra);
        let out = run(p.path(), &args);
        out.assert_code(2);
        assert!(out.stdout.is_empty(), "{extra:?}: {}", out.stdout);
    }
}

#[test]
fn t_i186_nothing_to_write_is_exit_2() {
    let p = Project::new().dir(".git");
    let out = run(p.path(), &["spec", "init", "--with", ""]);
    out.assert_code(2);
    assert!(!p.exists(".evnx.toml"));
}

// ── I-183, DEC-SP9: template order, then names only in .env ─────────────────

#[test]
fn t_i183_template_order_then_names_only_in_env() {
    let p = Project::new()
        .dir(".git")
        .example("Z=\nDATABASE_URL=\nA=\n")
        .env("ONLY_IN_ENV=1\nA=x\nDATABASE_URL=postgres://h/d\nZ=y\n");
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(0);
    assert_eq!(
        names(&out.stdout),
        ["Z", "DATABASE_URL", "A", "ONLY_IN_ENV"],
        "{}",
        out.stdout
    );
    let extra = table(&out.stdout, "ONLY_IN_ENV");
    assert!(extra.contains("# only in .env"), "{extra}");
    assert!(extra.contains("required = false"), "{extra}");
    assert!(table(&out.stdout, "A").contains("required = true"));
}

/// The comment names the file the values came from.
#[test]
fn t_i183_the_comment_names_the_environment_file() {
    let p = Project::new()
        .dir(".git")
        .example("A=\n")
        .file(".env.staging", "A=1\nSTAGE=1\n");
    let out = run(
        p.path(),
        &["spec", "init", "--env-name", "staging", "--stdout"],
    );
    out.assert_code(0);
    assert!(
        table(&out.stdout, "STAGE").contains("# only in .env.staging"),
        "{}",
        out.stdout
    );
}

/// No template: names come from `.env`, in its order, all required as before.
#[test]
fn t_i183_without_a_template_env_order_is_kept() {
    let p = Project::new().dir(".git").env("Z=1\nA=2\n");
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(0);
    assert_eq!(names(&out.stdout), ["Z", "A"]);
    assert!(!out.stdout.contains("only in"), "{}", out.stdout);
}

// ── I-184, DEC-SP10 and I-190, DEC-SP14: what is declared secret ────────────

#[test]
fn t_i184_a_url_with_a_password_is_secret() {
    let p = Project::new()
        .dir(".git")
        .env("DATABASE_URL=postgres://u:p@h/d\nREDIS_URL=redis://h:6379/0\n")
        .example("DATABASE_URL=\nREDIS_URL=\n");
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(0);
    assert!(
        table(&out.stdout, "DATABASE_URL").contains("secret = true"),
        "{}",
        out.stdout
    );
    assert!(!table(&out.stdout, "REDIS_URL").contains("secret"));
}

#[test]
fn t_i190_secret_names_follow_validates_strict_rule() {
    let p = Project::new()
        .dir(".git")
        .env("ADMIN_PWD=x\nSENTRY_DSN=x\nPASSWORD_MIN_LENGTH=8\nSTRIPE_SECRET_KEY=x\n")
        .example("ADMIN_PWD=\nSENTRY_DSN=\nPASSWORD_MIN_LENGTH=\nSTRIPE_SECRET_KEY=\n");
    let out = run(p.path(), &["spec", "init", "--stdout"]);
    out.assert_code(0);
    let s = &out.stdout;
    assert!(table(s, "ADMIN_PWD").contains("secret = true"), "{s}");
    assert!(
        table(s, "STRIPE_SECRET_KEY").contains("secret = true"),
        "{s}"
    );
    assert!(!table(s, "SENTRY_DSN").contains("secret"), "{s}");
    assert!(!table(s, "PASSWORD_MIN_LENGTH").contains("secret"), "{s}");
}

// ── I-187, DEC-SP5: a typo in [vars.X] is reported ──────────────────────────

#[test]
fn t_i187_an_unknown_vars_key_fails_validate_and_names_the_fix() {
    let p = Project::new()
        .dir(".git")
        .env("A=1\n")
        .example("A=\n")
        .toml("[vars.A]\nrequird = false\n");
    let out = run(p.path(), &["validate"]);
    out.assert_code(1);
    assert!(out.all().contains("requird"), "{}", out.all());
    assert!(out.all().contains("`required`"), "{}", out.all());
}

#[test]
fn t_i187_every_command_warns_about_it() {
    let p = Project::new()
        .dir(".git")
        .env("B=x\n")
        .toml("[vars.B]\nsecrett = true\n");
    let out = run(p.path(), &["scan", ".env"]);
    assert!(out.stderr.contains("vars.B.secrett"), "{}", out.all());
}

// ── I-040, DEC-SP15: the bare file name ─────────────────────────────────────

#[test]
fn t_i040_a_bare_evnx_toml_is_exit_2_naming_the_rename() {
    let p = Project::new()
        .dir(".git")
        .env("A=1\n")
        .example("A=\n")
        .file("evnx.toml", "[scan]\nseverity = \"high\"\n");
    for args in [&["validate"][..], &["scan", ".env"]] {
        let out = run(p.path(), args);
        out.assert_code(2);
        assert!(out.all().contains(".evnx.toml"), "{}", out.all());
        assert!(out.all().contains("rename"), "{}", out.all());
    }
}

#[test]
fn t_i040_beside_the_dotted_file_it_is_one_warning() {
    let p = Project::new()
        .dir(".git")
        .env("A=1\n")
        .example("A=\n")
        .toml("[validate]\nstrict = false\n")
        .file("evnx.toml", "[scan]\nseverity = \"high\"\n");
    let out = run(p.path(), &["validate"]);
    out.assert_code(0);
    assert_eq!(
        out.stderr.matches("evnx.toml is ignored").count(),
        1,
        "{}",
        out.all()
    );
}

// ── DEC-SP13: a symlinked config is written through ─────────────────────────

#[cfg(unix)]
#[test]
fn t_sp13_a_symlinked_config_is_written_through() {
    let p = Project::new()
        .dir(".git")
        .env("A=1\n")
        .example("A=\n")
        .file("shared/project.toml", "[scan]\nseverity = \"high\"\n")
        .symlink("shared/project.toml", ".evnx.toml");
    let out = run(p.path(), &["spec", "init"]);
    out.assert_code(0);
    assert!(
        std::fs::symlink_metadata(p.join(".evnx.toml"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was replaced by a file"
    );
    assert!(p.read("shared/project.toml").contains("[vars.A]"));
    assert!(out.all().contains("wrote through"), "{}", out.all());
}
