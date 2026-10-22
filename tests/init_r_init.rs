//! `evnx init` 0.10.16 (R-init): one test per issue, written before its fix.
//!
//! The plan, the decisions (DQ-26 … DQ-33) and the scenarios behind each test
//! are in the review package, `local/init/plan/PLAN-INIT-0.10.16.md`. The
//! interactive cases (the menu prompt, `CI=1`, stderr redirected) are in
//! `tests/init_pty.rs`.

mod support;

use support::{run, run_with, Project};

/// The framework ids `init --list-components` prints, in its order.
fn frameworks() -> Vec<String> {
    let out = run(std::path::Path::new("."), &["init", "--list-components"]);
    out.assert_code(0);
    let mut in_frameworks = false;
    let mut ids = Vec::new();
    for line in out.stdout.lines() {
        match line.trim() {
            "FRAMEWORK" => in_frameworks = true,
            "SERVICE" | "INFRASTRUCTURE" => in_frameworks = false,
            _ if in_frameworks && line.starts_with("    ") => {
                if let Some(id) = line.split_whitespace().next() {
                    ids.push(id.to_string());
                }
            }
            _ => {}
        }
    }
    assert!(
        ids.len() > 10,
        "frameworks parsed from --list-components: {ids:?}"
    );
    ids
}

// ── I-241 (T-INIT-31): CRLF, fixed in 0.10.2; locked here ─────────────

#[test]
fn t_i241_a_crlf_template_stays_crlf_when_from_source_appends() {
    let p = Project::new()
        .example("OLD=1\r\nKEEP=2\r\n")
        .file("src/a.js", "const x = process.env.NEW_ONE;\n");
    run(p.path(), &["init", "--from-source", "--yes"]).assert_code(0);
    let text = p.read(".env.example");
    assert!(text.starts_with("OLD=1\r\nKEEP=2\r\n"), "{text:?}");
    assert!(text.contains("NEW_ONE="), "{text:?}");
    let bare_lf = text.replace("\r\n", "").matches('\n').count();
    assert_eq!(bare_lf, 0, "every line ends in CRLF: {text:?}");
}

// ── I-298 (T-INIT-41): the target's .evnx.toml, not the working folder's ──

#[test]
fn t_i298_a_malformed_config_at_the_target_stops_before_anything_is_written() {
    let here = Project::new();
    let target = Project::new().git().toml("[vars\nbroken");
    let out = run(
        here.path(),
        &["init", "--yes", "-p", target.path().to_str().unwrap()],
    );
    out.assert_code(2).assert_contains("TOML parse error");
    for f in [".env", ".env.example", ".gitignore"] {
        assert!(!target.exists(f), "{f} written beside a broken config");
    }
}

#[test]
fn t_i298_a_bare_evnx_toml_at_the_target_stops_before_anything_is_written() {
    let here = Project::new();
    let target = Project::new()
        .git()
        .file("evnx.toml", "[vars.A]\nrequired = true\n");
    let out = run(
        here.path(),
        &["init", "--yes", "-p", target.path().to_str().unwrap()],
    );
    out.assert_code(2)
        .assert_contains("mv evnx.toml .evnx.toml");
    assert!(!target.exists(".env.example"));
}

// ── I-235 (T-INIT-25): blueprints get the component matcher ────────────

#[test]
fn t_i235_a_prefix_of_a_blueprint_is_suggested() {
    let p = Project::new();
    let out = run(p.path(), &["init", "--blueprint", "mern", "--yes"]);
    out.assert_code(2).assert_contains("Did you mean: mern_v2");
}

#[test]
fn t_i235_a_mistyped_blueprint_is_still_suggested() {
    let p = Project::new();
    let out = run(p.path(), &["init", "--blueprint", "t3modern", "--yes"]);
    out.assert_code(2)
        .assert_contains("Did you mean: t3_modern");
}

#[test]
fn t_i235_add_blueprint_suggests_the_same_way() {
    let p = Project::new();
    let out = run(p.path(), &["add", "blueprint", "mern", "--yes"]);
    out.assert_code(2).assert_contains("Did you mean: mern_v2");
}

#[test]
fn t_i235_a_blueprint_id_given_to_with_names_the_blueprint_flag() {
    let p = Project::new();
    let out = run(p.path(), &["init", "--with", "t3_modern", "--yes"]);
    out.assert_code(2)
        .assert_contains("t3_modern is a blueprint: use --blueprint t3_modern");
    assert!(!p.exists(".env.example"));
}

#[test]
fn t_i235_a_component_id_given_to_blueprint_names_the_with_flag() {
    let p = Project::new();
    let out = run(p.path(), &["init", "--blueprint", "postgresql", "--yes"]);
    out.assert_code(2)
        .assert_contains("postgresql is a component: use --with postgresql");
}

// ── I-002b (T-INIT-47, DQ-23): NEXT_PUBLIC_* names on Next.js only ─────

#[test]
fn t_i002b_next_public_names_appear_only_on_nextjs() {
    for fw in frameworks() {
        let p = Project::new();
        let with = format!("{fw},clerk,vercel");
        run(p.path(), &["init", "--with", &with, "--yes"]).assert_code(0);
        let example = p.read(".env.example");
        let next_public: Vec<&str> = example
            .lines()
            .filter(|l| l.starts_with("NEXT_PUBLIC_"))
            .collect();
        if fw == "nextjs" {
            assert!(
                example.contains("NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY=")
                    && example.contains("NEXT_PUBLIC_VERCEL_ENV="),
                "Next.js keeps its browser names:\n{example}"
            );
        } else {
            assert!(next_public.is_empty(), "{fw}: {next_public:?}");
        }
    }
}

#[test]
fn t_i002b_vite_gets_its_own_clerk_browser_name() {
    let p = Project::new();
    run(p.path(), &["init", "--with", "vite,clerk", "--yes"]).assert_code(0);
    let example = p.read(".env.example");
    assert!(example.contains("VITE_CLERK_PUBLISHABLE_KEY="), "{example}");
    assert!(!example.contains("NEXT_PUBLIC_"), "{example}");
}

// ── I-257 (T-INIT-46): every compose file name ─────────────────────────

#[test]
fn t_i257_every_compose_file_name_is_read() {
    for name in [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
    ] {
        let p = Project::new().file(name, "services:\n  db:\n    image: postgres:16\n");
        run(p.path(), &["init", "--detect"]).assert_code(0);
        let example = p.read(".env.example");
        assert!(example.contains("DATABASE_URL="), "{name}:\n{example}");
    }
}

// ── I-236 (T-INIT-26, DQ-22): what --from-source finds ─────────────────

#[test]
fn t_i236_destructuring_split_lines_and_other_languages_are_found() {
    let p = Project::new()
        .file(
            "src/a.js",
            "const { API_KEY, DB_URL: db, PORT = 3000, ...rest } = process.env;\n\
             const split = process\n  .env.SPLIT_LINE;\n",
        )
        .file("src/b.ts", "const {\n  VITE_A,\n  MODE,\n} = import.meta.env;\n")
        .file("src/c.ts", "const { BUN_ONE } = Bun.env;\n")
        .file("src/d.py", "import os\nv = os.getenv(\"PY_GETENV\", \"x\")\n")
        .file("src/e.rb", "k = ENV.fetch(\"RUBY_FETCH\")\n")
        .file("src/f.php", "<?php $k = getenv('PHP_GETENV');\n")
        .file(
            "src/g.rs",
            "const K: &str = env!(\"RUST_ENV_MACRO\");\nconst O: Option<&str> = option_env!(\"RUST_OPTION_ENV\");\n",
        );
    run(p.path(), &["init", "--from-source", "--yes"]).assert_code(0);
    let example = p.read(".env.example");
    for name in [
        "API_KEY",
        "DB_URL",
        "PORT",
        "SPLIT_LINE",
        "VITE_A",
        "BUN_ONE",
        "PY_GETENV",
        "RUBY_FETCH",
        "PHP_GETENV",
        "RUST_ENV_MACRO",
        "RUST_OPTION_ENV",
    ] {
        assert!(
            example.lines().any(|l| l.starts_with(&format!("{name}="))),
            "{name} not found:\n{example}"
        );
    }
    // A rest element and a framework built-in are not variables to set.
    assert!(!example.contains("rest="), "{example}");
    assert!(
        !example.lines().any(|l| l.starts_with("MODE=")),
        "{example}"
    );
}

#[test]
fn t_i236_a_destructuring_from_another_object_is_not_a_read() {
    let p = Project::new().file("src/a.js", "const { NOT_ENV } = settings;\n");
    let out = run_with(p.path(), &["init", "--from-source", "--yes"], &[], None);
    out.assert_code(0);
    assert!(!p.exists(".env.example") || !p.read(".env.example").contains("NOT_ENV="));
}
