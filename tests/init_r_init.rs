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

// ═════════════════════════════════════════════════════════════════════
// PR 2: the contract
// ═════════════════════════════════════════════════════════════════════

/// The blueprint ids, from the unknown-blueprint list.
fn blueprints() -> Vec<String> {
    let out = run(
        std::path::Path::new("."),
        &["init", "--blueprint", "x", "--yes"],
    );
    let ids: Vec<String> = out
        .all()
        .lines()
        .skip_while(|l| !l.starts_with("Available blueprints:"))
        .skip(1)
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .take_while(|id| {
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        })
        .collect();
    assert!(ids.len() >= 10, "{ids:?}");
    ids
}

fn names_in(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_string()))
        .filter(|k| !k.is_empty() && !k.starts_with('#') && !k.starts_with('['))
        .collect()
}

// ── I-295 (T-INIT-38, DQ-28): the project's own template ─────────────

#[test]
fn t_i295_a_project_whose_template_is_env_sample_gets_no_second_template() {
    let p = Project::new().file(".env.sample", "FROM_SAMPLE=x\n");
    let out = run(p.path(), &["init", "--with", "redis", "--yes"]);
    out.assert_code(2)
        .assert_contains(".env.sample already exists");
    assert!(!p.exists(".env.example"));

    run(p.path(), &["init", "--with", "redis", "--yes", "--force"])
        .assert_code(0)
        .assert_contains("Created .env.sample with");
    assert!(!p.exists(".env.example"));
    assert!(p.read(".env.sample").contains("REDIS_URL="));
}

#[test]
fn t_i295_defaults_example_names_the_template_even_when_missing() {
    let p = Project::new().toml("[defaults]\nexample = \".env.template\"\n");
    run(p.path(), &["init", "--with", "redis", "--yes"]).assert_code(0);
    assert!(p.read(".env.template").contains("REDIS_URL="));
    assert!(!p.exists(".env.example"));
}

#[test]
fn t_i295_add_appends_to_the_projects_template() {
    let p = Project::new().file(".env.sample", "FROM_SAMPLE=x\n");
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    assert!(p.read(".env.sample").contains("REDIS_URL="));
    assert!(!p.exists(".env.example"));
}

#[test]
fn t_i295_from_source_compares_against_the_projects_template() {
    let p = Project::new()
        .file(".env.sample", "KNOWN_ONE=x\n")
        .file("src/a.js", "process.env.KNOWN_ONE; process.env.NEW_ONE;\n");
    run(p.path(), &["init", "--from-source", "--yes"])
        .assert_code(0)
        .assert_contains("1 already in .env.sample");
    assert!(p.read(".env.sample").contains("NEW_ONE="));
    assert!(!p.exists(".env.example"));
}

// ── I-297 (T-INIT-40, DQ-29): an existing .env catches up ─────────────

#[test]
fn t_i297_an_existing_env_gets_the_missing_names_and_keeps_its_values() {
    let p = Project::new().env("DATABASE_URL=postgres://real@db/app\nSECRET_KEY=s3cr3t\n");
    run(p.path(), &["init", "--with", "nextjs,postgresql", "--yes"]).assert_code(0);
    let env = p.read(".env");
    let example = p.read(".env.example");
    assert!(
        env.starts_with("DATABASE_URL=postgres://real@db/app\nSECRET_KEY=s3cr3t\n"),
        "{env}"
    );
    for name in names_in(&example) {
        assert!(
            names_in(&env).contains(&name),
            "{name} missing from .env:\n{env}"
        );
    }
    assert_eq!(
        env.matches("DATABASE_URL=").count(),
        1,
        "a real value never gets a placeholder below it:\n{env}"
    );
}

// ── DQ-31 (T-INIT-43): a fresh clone ──────────────────────────────────

#[test]
fn t_dq31_yes_on_a_fresh_clone_creates_env_from_the_template() {
    let p = Project::new().git().example("A=\nB=2\n");
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0)
        .assert_contains("Created .env from .env.example");
    assert_eq!(p.read(".env"), "A=\nB=2\n");
    assert_eq!(
        p.read(".env.example"),
        "A=\nB=2\n",
        "the template is not touched"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(p.join(".env"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    assert!(p.is_ignored(".env"));
}

#[test]
fn t_dq31_yes_when_both_files_exist_is_already_set_up() {
    let p = Project::new().example("A=\n").env("A=real\n");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_contains("already set up");
    assert_eq!(p.read(".env"), "A=real\n");
}

#[test]
fn t_dq31_naming_a_stack_still_refuses_over_a_template() {
    let p = Project::new().example("A=\n");
    run(p.path(), &["init", "--with", "redis", "--yes"]).assert_code(2);
    assert_eq!(p.read(".env.example"), "A=\n");
}

// ── IN-15 (I-242, DQ-17, DQ-18, DQ-26, DQ-27): .evnx.toml ─────────────

#[test]
fn t_i242_init_creates_the_contract() {
    let p = Project::new().git();
    run(p.path(), &["init", "--with", "redis", "--yes"]).assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(toml.contains("[vars.REDIS_URL]"), "{toml}");
    toml.parse::<toml::Table>().expect("the contract parses");
    // Committed, not ignored.
    assert!(!p.is_ignored(".evnx.toml"));
}

#[test]
fn t_i242_blank_init_creates_an_empty_contract_that_validates() {
    let p = Project::new().git();
    run(p.path(), &["init", "--yes"]).assert_code(0);
    let toml = p.read(".evnx.toml");
    let table: toml::Table = toml.parse().expect("parses");
    assert!(table.contains_key("vars"), "{toml}");
    run(p.path(), &["validate"]).assert_code(0);
}

/// T-INIT-32: for every blueprint, `init --yes` then `validate` fails only on
/// placeholders: the contract adds no error (no format the catalogue's own
/// samples fail). Warnings stay warnings (Django's `DEBUG=True` is a
/// `boolean_trap` warning with or without a contract).
#[test]
fn t_init_32_every_blueprint_validates_apart_from_placeholders() {
    for id in blueprints() {
        let p = Project::new().git();
        run(p.path(), &["init", "--blueprint", &id, "--yes"]).assert_code(0);
        assert!(p.exists(".evnx.toml"), "{id}");
        let out = run(p.path(), &["validate", "--format", "json"]);
        let doc: serde_json::Value = serde_json::from_str(&out.stdout)
            .unwrap_or_else(|e| panic!("{id}: {e}\n{}", out.all()));
        for issue in doc["issues"].as_array().unwrap() {
            if issue["severity"] == "error" {
                assert_eq!(issue["type"], "placeholder_value", "{id}: {issue}");
            }
        }
    }
}

#[test]
fn t_dq17_new_names_are_recorded_into_an_existing_contract() {
    let p = Project::new()
        .git()
        .toml("[vars.EXISTING]\nrequired = false\n");
    run(p.path(), &["init", "--with", "redis", "--yes"])
        .assert_code(0)
        .assert_contains("[vars]");
    let toml = p.read(".evnx.toml");
    assert!(
        toml.starts_with("[vars.EXISTING]\nrequired = false\n"),
        "{toml}"
    );
    assert!(toml.contains("[vars.REDIS_URL]"), "{toml}");
}

#[test]
fn t_dq27_a_config_without_vars_gets_vars_entries() {
    let p = Project::new().git().toml("[scan]\nseverity = \"high\"\n");
    run(p.path(), &["init", "--with", "redis", "--yes"]).assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(toml.starts_with("[scan]\nseverity = \"high\"\n"), "{toml}");
    assert!(toml.contains("[vars.REDIS_URL]"), "{toml}");
}

#[test]
fn t_dq27_blank_init_gives_a_config_without_vars_an_empty_vars() {
    let p = Project::new().git().toml("[scan]\nseverity = \"high\"\n");
    run(p.path(), &["init", "--yes"]).assert_code(0);
    let table: toml::Table = p.read(".evnx.toml").parse().unwrap();
    assert!(table.contains_key("vars") && table.contains_key("scan"));
}

/// T-INIT-34, DQ-26: a package below the git root, no config anywhere.
#[test]
fn t_dq26_a_package_below_the_root_gets_its_own_contract() {
    let p = Project::new().git().dir("apps/web");
    run(
        p.path(),
        &["init", "--with", "nextjs", "--yes", "-p", "apps/web"],
    )
    .assert_code(0);
    assert!(p
        .read("apps/web/.evnx.toml")
        .contains("[vars.NEXTAUTH_SECRET]"));
    assert!(!p.exists(".evnx.toml"));
}

/// DQ-26: a config found at or above the package is the one recorded into.
#[test]
fn t_dq26_a_config_above_the_package_is_recorded_into() {
    let p = Project::new()
        .git()
        .toml("[vars.ROOT_ONE]\nrequired = true\n")
        .dir("apps/web");
    run(
        p.path(),
        &["init", "--with", "redis", "--yes", "-p", "apps/web"],
    )
    .assert_code(0);
    assert!(p.read(".evnx.toml").contains("[vars.REDIS_URL]"));
    assert!(!p.exists("apps/web/.evnx.toml"));
}

#[test]
fn t_i242_from_source_creates_the_contract_too() {
    let p = Project::new()
        .git()
        .file("src/a.js", "const x = process.env.NEW_ONE;\n");
    run(p.path(), &["init", "--from-source", "--yes"]).assert_code(0);
    assert!(p.read(".evnx.toml").contains("[vars.NEW_ONE]"));
}

#[test]
fn t_i242_from_source_beside_a_config_without_vars_does_not_say_run_spec_init() {
    let p = Project::new()
        .git()
        .toml("[scan]\nseverity = \"high\"\n")
        .example("OLD=1\n")
        .file("src/a.js", "process.env.OLD; process.env.NEW_ONE;\n");
    let out = run(p.path(), &["init", "--from-source", "--yes"]);
    out.assert_code(0).assert_not_contains("spec init");
    assert!(p.read(".evnx.toml").contains("[vars.NEW_ONE]"));
}

// ── I-296 (T-INIT-39, DQ-32): the env files init does not write ───────

#[test]
fn t_i296_defaults_env_name_is_named() {
    let p = Project::new()
        .git()
        .toml("[defaults]\nenv_name = \"local\"\n");
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0)
        .assert_contains("env_name")
        .assert_contains(".env.local");
}

#[test]
fn t_i296_other_env_files_are_listed_as_unchanged() {
    let p = Project::new()
        .git()
        .file(".env.production", "APP_ENV=production\n")
        .file(".env.staging", "APP_ENV=staging\n");
    let out = run(p.path(), &["init", "--with", "stripe", "--yes"]);
    out.assert_code(0)
        .assert_contains(".env.production")
        .assert_contains(".env.staging")
        .assert_contains("evnx validate --env-name production");
    assert_eq!(p.read(".env.production"), "APP_ENV=production\n");
}

#[test]
fn t_i296_env_local_is_said_to_be_read_first() {
    let p = Project::new().git().file(".env.local", "A=1\n");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_contains(".env.local is read before .env");
}

// ── DQ-33 (T-INIT-45): .envrc ─────────────────────────────────────────

#[test]
fn t_dq33_an_untracked_envrc_is_said_to_be_ignored() {
    let p = Project::new().git().file(".envrc", "export A=1\n");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_contains("!.envrc");
    assert!(p.is_ignored(".envrc"), "stays ignored (DQ-33)");
}

#[test]
fn t_dq33_a_tracked_envrc_is_not_mentioned() {
    let p = Project::new().git().file(".envrc", "export A=1\n");
    p.track(".envrc");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_not_contains("envrc");
}

// ── E6: the shared exposure warning ───────────────────────────────────

/// A tracked `.env` is told what helps (`git rm --cached`), not that a
/// `.gitignore` rule re-includes it.
#[test]
fn t_e6_a_tracked_env_is_told_git_rm_cached() {
    let p = Project::new().git().env("A=1\n");
    p.force_add(".env");
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0)
        .assert_contains("'.env' is tracked by git")
        .assert_contains("git rm --cached .env")
        .assert_not_contains("re-includes");
}

// ── IN-16 (DQ-21): --dry-run and --json ───────────────────────────────

/// Every file under `p` with its bytes, `.git` left out: what a dry run must
/// leave as it was.
fn snapshot(p: &Project) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            if e.file_name() == ".git" {
                continue;
            }
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().display().to_string();
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(p.path(), p.path(), &mut out);
    out
}

fn json_of(out: &support::Output) -> serde_json::Value {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}\n{}", out.all()))
}

/// T-INIT-35: a dry run writes nothing, and says exactly what the real run
/// then reports — template, `.env`, `.gitignore`, `.evnx.toml`, notices.
#[test]
fn t_init_35_a_dry_run_writes_nothing_and_predicts_the_run() {
    type Setup = fn() -> Project;
    let cases: Vec<(&str, Setup, Vec<&str>)> = vec![
        ("new repo", || Project::new().git(), vec!["--with", "redis"]),
        (
            "existing .env",
            || {
                Project::new()
                    .git()
                    .env("DATABASE_URL=postgres://real@db/app\n")
            },
            vec!["--with", "nextjs,postgresql"],
        ),
        (
            "existing [vars]",
            || {
                Project::new()
                    .git()
                    .toml("[vars.EXISTING]\nrequired = true\n")
            },
            vec!["--with", "redis"],
        ),
        (
            "config without [vars]",
            || Project::new().git().toml("[scan]\nseverity = \"high\"\n"),
            vec!["--with", "redis"],
        ),
        (
            "fresh clone",
            || Project::new().git().example("A=\nB=2\n"),
            vec![],
        ),
        (
            "from source beside a template",
            || {
                Project::new()
                    .git()
                    .example("OLD=1\n")
                    .file("src/a.js", "process.env.OLD; process.env.NEW_ONE;\n")
                    .file(".env.production", "OLD=2\n")
            },
            vec!["--from-source"],
        ),
    ];
    for (what, setup, args) in cases {
        let p = setup();
        let before = snapshot(&p);
        let mut dry = vec!["init", "--dry-run", "--json"];
        dry.extend(&args);
        let planned = run(p.path(), &dry);
        planned.assert_code(0);
        assert_eq!(snapshot(&p), before, "{what}: a dry run wrote something");

        let mut real = vec!["init", "--yes", "--json"];
        real.extend(&args);
        let done = run(p.path(), &real);
        done.assert_code(0);
        let (mut a, mut b) = (json_of(&planned), json_of(&done));
        assert_eq!(a["dry_run"], true, "{what}");
        assert_eq!(b["written"], true, "{what}");
        for doc in [&mut a, &mut b] {
            let o = doc.as_object_mut().unwrap();
            for k in ["dry_run", "written", "env_committable"] {
                o.remove(k);
            }
        }
        assert_eq!(a, b, "{what}: the dry run and the run disagree");
        assert!(p.exists(".evnx.toml"), "{what}");
    }
}

/// T-INIT-36: `init --json` and `add --json` print the same fields.
#[test]
fn t_init_36_init_and_add_share_one_json_schema() {
    let keys = |v: &serde_json::Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    let p = Project::new().git();
    let init = json_of(&run(
        p.path(),
        &["init", "--with", "redis", "--yes", "--json"],
    ));
    let add = json_of(&run(
        p.path(),
        &["add", "service", "stripe", "--yes", "--json"],
    ));
    assert_eq!(keys(&init), keys(&add));
    assert_eq!(init["contract"]["action"], "created");
    assert_eq!(add["contract"]["action"], "recorded");
    assert_eq!(add["template"], ".env.example");
}

#[test]
fn t_in16_the_dry_run_text_names_each_file() {
    let p = Project::new().git();
    let out = run(p.path(), &["init", "--with", "redis", "--dry-run"]);
    out.assert_code(0)
        .assert_contains("Dry run: nothing was written.")
        .assert_contains(".env.example: would be created with 5 variables")
        .assert_contains(".env: would be created (0600) with 5 variables")
        .assert_contains(".gitignore: would add .env*")
        .assert_contains(".evnx.toml: would be created with 5 variables in [vars]");
    assert!(!p.exists(".env.example") && !p.exists(".env") && !p.exists(".evnx.toml"));
}

#[test]
fn t_in16_json_needs_yes_or_dry_run() {
    let p = Project::new();
    run(p.path(), &["init", "--with", "redis", "--json"])
        .assert_code(2)
        .assert_contains("--json needs --yes or --dry-run");
}

#[test]
fn t_in16_a_dry_run_over_a_template_is_refused_as_the_run_would_be() {
    let p = Project::new().example("A=\n");
    run(p.path(), &["init", "--with", "redis", "--dry-run"]).assert_code(2);
    assert_eq!(p.read(".env.example"), "A=\n");
}

// ── IN-14 (T-INIT-37, DQ-25): five catalogue entries ──────────────────

#[test]
fn t_init_37_the_five_new_components_are_listed() {
    let out = run(std::path::Path::new("."), &["init", "--list-components"]);
    let ids: Vec<&str> = out
        .stdout
        .lines()
        .filter(|l| l.starts_with("    ") && !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(ids.len(), 56, "{ids:?}");
    for id in ["astro", "hono", "neon", "turnstile", "fly"] {
        assert!(ids.contains(&id), "{id}");
    }
}

/// Astro's browser prefix is `PUBLIC_`; only the public halves get a copy.
#[test]
fn t_init_37_astro_gets_public_copies_of_public_values_only() {
    let p = Project::new();
    run(
        p.path(),
        &["init", "--with", "astro,supabase,turnstile", "--yes"],
    )
    .assert_code(0);
    let example = p.read(".env.example");
    for name in [
        "PUBLIC_SUPABASE_URL",
        "PUBLIC_TURNSTILE_SITE_KEY",
        "TURNSTILE_SECRET_KEY",
        "HOST",
        "PORT",
    ] {
        assert!(
            example.contains(&format!("\n{name}=")),
            "{name}:\n{example}"
        );
    }
    assert!(
        !example.contains("PUBLIC_TURNSTILE_SECRET_KEY"),
        "{example}"
    );
    assert!(!example.contains("NEXT_PUBLIC_"), "{example}");
}

/// Hono is a server framework: no browser copies.
#[test]
fn t_init_37_a_server_stack_gets_no_copies() {
    let p = Project::new();
    run(
        p.path(),
        &["init", "--with", "hono,neon,fly,turnstile", "--yes"],
    )
    .assert_code(0);
    let example = p.read(".env.example");
    for name in [
        "PORT",
        "DATABASE_URL",
        "DATABASE_URL_UNPOOLED",
        "FLY_API_TOKEN",
        "TURNSTILE_SITE_KEY",
    ] {
        assert!(
            example.contains(&format!("\n{name}=")),
            "{name}:\n{example}"
        );
    }
    assert!(
        !example
            .lines()
            .any(|l| l.starts_with("PUBLIC_") || l.starts_with("VITE_")),
        "{example}"
    );
}

#[test]
fn t_init_37_detect_proposes_the_new_components() {
    let p = Project::new()
        .file(
            "package.json",
            r#"{"dependencies":{"astro":"5","hono":"4","@neondatabase/serverless":"1"}}"#,
        )
        .file("fly.toml", "app = \"demo\"\n");
    let out = run(p.path(), &["init", "--detect"]);
    out.assert_code(0);
    for shown in ["Astro", "Hono", "Neon", "Fly.io"] {
        out.assert_contains(shown);
    }
    assert!(p.read(".env.example").contains("FLY_API_TOKEN="));
}

// ── I-302: the refusal over a template points to `evnx add` ───────────

/// A stack named over an existing template is refused; the refusal says how
/// to add the stack's variables to it instead (PLAN-INIT-0.10.16 §2, S04).
#[test]
fn t_i302_the_refusal_over_a_template_names_evnx_add() {
    let p = Project::new().example("MINE=1\n");
    run(p.path(), &["init", "--with", "redis", "--yes"])
        .assert_code(2)
        .assert_contains("--force")
        .assert_contains("evnx add blueprint <id> or evnx add service <id>");
    assert_eq!(p.read(".env.example"), "MINE=1\n");
}
