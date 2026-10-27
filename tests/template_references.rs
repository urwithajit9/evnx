//! `evnx template`: which references count, and what a default stands for.
//!
//! - I-135 (DEC-T3): `${NAME}` and `{{NAME}}` are always references. A bare
//!   `$NAME` is substituted when defined and otherwise left alone: in nginx or
//!   shell text it is that program's syntax. Since RM-14 made an unresolved
//!   reference an error, reporting it failed every such template.
//! - I-134: each `|default:` stands for its own text, not the first one's.

mod support;

use support::{run, Project};

#[test]
fn t_i135_bare_dollar_names_in_nginx_text_are_left_alone() {
    let p = Project::new().env("PORT=8080\n").file(
        "nginx.tpl",
        "listen {{PORT}};\nproxy_set_header Host $host;\nroot $document_root;\n",
    );
    let out = run(
        p.path(),
        &["template", "--input", "nginx.tpl", "--output", "-"],
    );
    out.assert_code(0);
    assert_eq!(
        out.stdout,
        "listen 8080;\nproxy_set_header Host $host;\nroot $document_root;\n"
    );
    assert!(!out.stderr.contains("host"), "{}", out.stderr);
}

#[test]
fn t_i135_a_defined_bare_name_is_still_substituted() {
    let p = Project::new()
        .env("PORT=8080\n")
        .file("run.tpl", "port=$PORT path=$PATH_UNSET\n");
    let out = run(
        p.path(),
        &["template", "--input", "run.tpl", "--output", "-"],
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "port=8080 path=$PATH_UNSET\n");
}

#[test]
fn t_i135_braced_references_are_still_errors() {
    let p = Project::new()
        .env("PORT=8080\n")
        .file("u.tpl", "a=${NOPE}\nb={{MISSING}}\nc=$host\n");
    let out = run(p.path(), &["template", "--input", "u.tpl", "--output", "-"]);
    out.assert_code(2);
    assert!(out.stderr.contains("NOPE"), "{}", out.stderr);
    assert!(out.stderr.contains("MISSING"), "{}", out.stderr);
    assert!(!out.stderr.contains("host"), "{}", out.stderr);
}

#[test]
fn t_i134_each_default_uses_its_own_text() {
    let p = Project::new()
        .env("E=\n")
        .file("d.tpl", "x={{E|default:one}} {{E|default:two}}\n");
    let out = run(p.path(), &["template", "--input", "d.tpl", "--output", "-"]);
    out.assert_code(0);
    assert_eq!(out.stdout, "x=one two\n");
}
