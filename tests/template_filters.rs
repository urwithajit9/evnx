//! `evnx template` filters (R-template, 0.10.5).
//!
//! - `|title` is title case, Jinja's rule; `|capitalize` keeps the old
//!   behaviour, first character only (I-137, DEC-T7).
//! - `|bool` reads `true yes on 1 y` and `false no off 0 n` in any case, and
//!   anything else is an error, like `|int` (I-138, DEC-T7).
//! - A filter evnx does not know is an error naming it and its line, for any
//!   `{{NAME|filter}}`; chained filters are an error until the 0.11.0 engine
//!   (DEC-T7, owner 2026-10-17). They were written into the output unchanged.
//! - A bad value is not echoed when its name looks secret (`core::mask`).

mod support;

use support::{run, Output, Project};

/// Render `template` with `env` to stdout.
fn render(env: &str, template: &str) -> (Project, Output) {
    let p = Project::new().env(env).file("t.tpl", template);
    let out = run(
        p.path(),
        &[
            "template",
            "--input",
            "t.tpl",
            "--output",
            "-",
            "--no-gitignore",
        ],
    );
    (p, out)
}

/// Render `template` with `env` to a file, `out.yml`.
fn render_to_file(env: &str, template: &str) -> (Project, Output) {
    let p = Project::new().env(env).file("t.tpl", template);
    let out = run(
        p.path(),
        &[
            "template",
            "--input",
            "t.tpl",
            "--output",
            "out.yml",
            "--no-gitignore",
        ],
    );
    (p, out)
}

#[test]
fn t_i137_title_is_title_case() {
    let (_, out) = render(
        "A=hello wORLD-foo (bar) [baz]\nB=o'neil\n",
        "{{A|title}}\n{{ B | title }}\n",
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "Hello World-Foo (Bar) [Baz]\nO'neil\n");
}

#[test]
fn t_dec_t7_capitalize_is_the_old_title() {
    let (_, out) = render("A=hello wORLD\n", "{{A|capitalize}}\n");
    out.assert_code(0);
    assert_eq!(out.stdout, "Hello wORLD\n");
}

#[test]
fn t_i138_bool_reads_the_decided_words_in_any_case() {
    let words = [
        ("true", "true"),
        ("Yes", "true"),
        ("ON", "true"),
        ("1", "true"),
        ("y", "true"),
        ("False", "false"),
        ("no", "false"),
        ("off", "false"),
        ("0", "false"),
        ("N", "false"),
    ];
    for (value, want) in words {
        let (_, out) = render(&format!("FLAG={value}\n"), "{{FLAG|bool}}\n");
        out.assert_code(0);
        assert_eq!(out.stdout, format!("{want}\n"), "FLAG={value}");
    }
}

#[test]
fn t_i138_bool_refuses_anything_else_and_writes_nothing() {
    for value in ["maybe", "enabled", "ture", "", "\" yes\""] {
        let (p, out) = render_to_file(&format!("FLAG={value}\n"), "debug: {{FLAG|bool}}\n");
        out.assert_code(2)
            .assert_contains("FLAG")
            .assert_contains("|bool");
        assert!(!p.exists("out.yml"), "FLAG={value}: nothing is written");
    }
}

#[test]
fn t_i138_a_bad_value_is_not_echoed_for_a_secret_name() {
    let (_, out) = render("DB_PASSWORD=hunter2\n", "{{DB_PASSWORD|bool}}\n");
    out.assert_code(2).assert_not_contains("hunter2");
    let (_, out) = render("DB_PASSWORD=hunter2\n", "{{DB_PASSWORD|int}}\n");
    out.assert_code(2).assert_not_contains("hunter2");
    // A plain name still shows what was wrong with it.
    let (_, out) = render("PORT=80a\n", "{{PORT|int}}\n");
    out.assert_code(2).assert_contains("80a");
}

#[test]
fn t_dec_t7_an_unknown_filter_is_an_error_naming_it_and_its_line() {
    let (p, out) = render_to_file("X=v\n", "a: 1\nb: {{X|bogus}}\n");
    out.assert_code(2)
        .assert_contains("bogus")
        .assert_contains("line 2")
        .assert_contains("Nothing was written");
    assert!(!p.exists("out.yml"));
    // To stdout as well: nothing of the render is printed.
    let (_, out) = render("X=v\n", "a: 1\nb: {{X|bogus}}\n");
    out.assert_code(2);
    assert_eq!(out.stdout, "");
}

#[test]
fn t_dec_t7_an_unknown_filter_is_an_error_whether_the_name_is_defined_or_not() {
    let (_, out) = render("X=v\n", "{{NOPE|bogus}}\n");
    out.assert_code(2).assert_contains("bogus");
}

#[test]
fn t_dec_t7_every_unknown_filter_is_listed() {
    let (_, out) = render("X=v\n", "{{X|bogus}}\nok\n{{X|nope}}\n");
    out.assert_code(2)
        .assert_contains("line 1")
        .assert_contains("bogus")
        .assert_contains("line 3")
        .assert_contains("nope");
}

#[test]
fn t_dec_t7_a_close_filter_name_is_suggested() {
    let (_, out) = render("X=v\n", "{{ X | Upper }}\n");
    out.assert_code(2)
        .assert_contains("Upper")
        .assert_contains("did you mean `upper`");
}

#[test]
fn t_dec_t7_chained_filters_are_refused_until_the_engine_supports_them() {
    let (_, out) = render("X=v\n", "{{X|upper|json}}\n");
    out.assert_code(2).assert_contains("chained");
}

/// What is not an evnx filter must keep working: a default's text may hold a
/// pipe, and Helm's `{{ .Values.x | quote }}` is not an evnx reference.
#[test]
fn t_dec_t7_defaults_and_foreign_template_syntax_still_pass() {
    let (_, out) = render(
        "X=\nY=v\n",
        "{{X|default:a|b}} {{Y|upper}} {{ .Values.name | quote }}\n",
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "a|b V {{ .Values.name | quote }}\n");
}

/// D-93: `--help` names the reference styles and every filter.
#[test]
fn t_d93_help_lists_the_styles_and_the_filters() {
    let p = Project::new();
    let out = run(p.path(), &["template", "--help"]);
    out.assert_code(0);
    for needle in [
        "{{VAR}}",
        "${VAR}",
        "$VAR",
        "upper",
        "lower",
        "title",
        "capitalize",
        "bool",
        "int",
        "json",
        "default:",
        "--dry-run",
        "--mkdir",
    ] {
        out.assert_contains(needle);
    }
}
