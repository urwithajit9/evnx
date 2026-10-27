//! `evnx template` end to end — the three silent-wrong-answer bugs it had.
//!
//! All three shared one cause: substitution iterated over the *variables that
//! exist* and built a regex per key, rather than scanning the template for
//! placeholders. A form the per-key regex did not match was not reported as
//! unmatched — it was written to the output file verbatim, with exit 0.
//!
//! Built on `tests/support` (T-029, R-template): the file had its own
//! `project()` and `render()`, a third copy of what the support module does.

mod support;

use support::{run, Output, Project};

fn project(template: &str) -> Project {
    Project::new()
        .env("PORT=8080\nAPP_ENV=production\n")
        .file("in.tmpl", template)
}

fn template(p: &Project, extra: &[&str]) -> Output {
    let mut args = vec!["template", "--input", "in.tmpl", "--output", "out.conf"];
    args.extend_from_slice(extra);
    run(p.path(), &args)
}

fn render(p: &Project, extra: &[&str]) -> String {
    template(p, extra).assert_code(0);
    p.read("out.conf")
}

/// ⚠️ `{{ PORT }}` — with the spaces most people write — matched nothing and was
/// copied into the generated file as literal text.
#[test]
fn whitespace_inside_a_placeholder_still_substitutes() {
    let p = project(
        "a={{PORT}}\n\
         b={{ PORT }}\n\
         c=${PORT}\n\
         d=${ PORT }\n\
         e=$PORT\n\
         f={{APP_ENV|upper}}\n\
         g={{ APP_ENV | upper }}\n",
    );

    let out = render(&p, &[]);
    for line in out.lines() {
        assert!(
            !line.contains('{') && !line.contains('$'),
            "unsubstituted placeholder: {line}"
        );
    }
    assert_eq!(out.matches("8080").count(), 5, "{out}");
    assert_eq!(out.matches("PRODUCTION").count(), 2, "{out}");
}

/// ⚠️ `|default:` fired only when the variable *existed* — the one case a default
/// is not needed. A genuinely missing variable rendered the whole
/// `{{NOPE|default:1234}}` into the output.
#[test]
fn a_default_applies_when_the_variable_is_absent() {
    let p = project(
        "present={{PORT|default:1234}}\n\
         absent={{NOPE|default:1234}}\n\
         spaced={{ NOPE | default:5678 }}\n",
    );

    let out = render(&p, &[]);
    assert!(
        out.contains("present=8080"),
        "a value beats its default: {out}"
    );
    assert!(out.contains("absent=1234"), "{out}");
    assert!(out.contains("spaced=5678"), "{out}");
    assert!(!out.contains("default:"), "{out}");
}

/// A variable with a fallback is not undefined, so `--strict` must accept it.
#[test]
fn strict_accepts_a_placeholder_that_has_a_default() {
    let p = project("a={{PORT}}\nb={{NOPE|default:ok}}\n");
    let out = render(&p, &["--strict"]);
    assert_eq!(out, "a=8080\nb=ok\n", "{out}");
}

/// ⚠️ The headline: an unresolved placeholder was written through and the command
/// exited 0. The generated file then holds `{{DATABASE_URL}}` where a host name
/// belongs, and whatever reads it next treats that as the setting.
#[test]
fn strict_refuses_and_writes_nothing() {
    let p = project("url={{DATABASE_URL}}\nport={{PORT}}\n");

    let out = template(&p, &["--strict"]);
    out.assert_code(2);
    assert!(out.stderr.contains("DATABASE_URL"), "{}", out.stderr);
    assert!(out.stderr.contains("Nothing was written"), "{}", out.stderr);

    assert!(
        !p.exists("out.conf"),
        "a refused run must not leave a half-built config on disk"
    );
}

/// RM-14 (DEC-T2): an unresolved placeholder is an error by default — exit 2,
/// nothing written, every name listed, and the way out named.
#[test]
fn rm14_unresolved_is_an_error_by_default() {
    let p = project("url={{DATABASE_URL}}\nb={{OTHER}}\n");
    let out = template(&p, &[]);
    out.assert_code(2);
    let all = format!("{}{}", out.stdout, out.stderr);
    for name in ["DATABASE_URL", "OTHER"] {
        assert!(all.contains(name), "{all}");
    }
    assert!(all.contains("--allow-unresolved"), "{all}");
    assert!(!p.exists("out.conf"));
}

/// `--allow-unresolved` keeps the old pass-through — someone's working
/// pipeline — and says what it did.
#[test]
fn allow_unresolved_writes_through_but_says_so() {
    let p = project("url={{DATABASE_URL}}\n");

    let out = template(&p, &["--allow-unresolved"]);
    out.assert_code(0);
    assert!(out.stdout.contains("unresolved"), "{}", out.stdout);
    assert!(out.stdout.contains("--allow-unresolved"), "{}", out.stdout);
    assert_eq!(p.read("out.conf"), "url={{DATABASE_URL}}\n");
}

/// `--strict` is still accepted, and says it is no longer needed.
#[test]
fn rm14_strict_is_an_accepted_alias_with_a_notice() {
    let p = project("url={{DATABASE_URL}}\n");
    let out = template(&p, &["--strict"]);
    out.assert_code(2);
    let all = format!("{}{}", out.stdout, out.stderr);
    assert!(
        all.contains("--strict is the default since 0.11.0"),
        "{all}"
    );
}

/// `--strict` names every unresolved placeholder, not just the first.
#[test]
fn strict_names_all_of_them() {
    let p = project("a={{ONE}}\nb={{TWO}}\nc={{ THREE }}\n");

    let out = template(&p, &["--strict"]);
    out.assert_code(2);
    for name in ["ONE", "TWO", "THREE"] {
        assert!(
            out.stderr.contains(name),
            "{name} missing from: {}",
            out.stderr
        );
    }
}
