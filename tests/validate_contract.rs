//! `evnx validate` after 0.10.7 (package R-validate, 0.10.8): what it counts,
//! what it reports once, what `--fix` calls fixed, which inputs it refuses, and
//! the checks the review asked for (environments, Docker files, `.gitignore`).
//!
//! One test per issue or decision, named after it. Review: the package's
//! `local/validate/review/04-validate.md` (decisions in §2i and §7, DEC-VA1..3).

mod support;

use serde_json::Value;
use support::{run, Project};

/// The review's fixture: eight declared variables, two of them missing
/// (`STRIPE_KEY`, `OPTIONAL_X`), one extra, one duplicate, bad values.
const EXAMPLE: &str = "DATABASE_URL=postgres://user:pass@localhost:5432/app\n\
                       SECRET_KEY=generate-me\n\
                       API_PORT=8080\n\
                       ADMIN_EMAIL=admin@example.com\n\
                       DEBUG=false\n\
                       STRIPE_KEY=sk_test_xxx\n\
                       OPTIONAL_X=\n\
                       A=1\n";
const ENV: &str = "DATABASE_URL=not-a-url\n\
                   SECRET_KEY=changeme\n\
                   API_PORT=99999\n\
                   ADMIN_EMAIL=nobody\n\
                   DEBUG=False\n\
                   EXTRA_VAR=1\n\
                   A=1\n\
                   A=2\n";

fn fixture() -> Project {
    Project::new().example(EXAMPLE).env(ENV)
}

fn json(p: &Project, extra: &[&str]) -> Value {
    let mut args = vec!["validate", "--format", "json", "--exit-zero"];
    args.extend_from_slice(extra);
    let out = run(p.path(), &args);
    serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}\n{}", out.stdout, out.stderr))
}

/// `(type, variable)` of every issue.
fn issues(v: &Value) -> Vec<(String, String)> {
    v["issues"]
        .as_array()
        .expect("issues[]")
        .iter()
        .map(|i| {
            (
                i["type"].as_str().unwrap().to_string(),
                i["variable"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn of_type(v: &Value, ty: &str) -> Vec<Value> {
    v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == ty)
        .cloned()
        .collect()
}

// ── I-034: required counts are declared names that are present ─────────────

#[test]
fn t_i034_required_counts_are_declared_names_present() {
    let v = json(&fixture(), &[]);
    assert_eq!(
        (v["required_present"].as_u64(), v["required_total"].as_u64()),
        (Some(6), Some(8)),
        "two of eight declared variables are missing"
    );
}

#[test]
fn t_i034_extra_variables_do_not_inflate_the_count() {
    let p = Project::new().example("A=\nB=\n").env("X=1\nY=2\nZ=3\n");
    let v = json(&p, &[]);
    assert_eq!(v["required_present"], 0, "{v}");
    assert_eq!(v["required_total"], 2, "{v}");
}

// ── I-035: one problem, one finding ─────────────────────────────────────────

#[test]
fn t_i035_a_placeholder_secret_is_one_finding() {
    let v = json(&fixture(), &[]);
    let about: Vec<_> = issues(&v)
        .into_iter()
        .filter(|(_, var)| var == "SECRET_KEY")
        .collect();
    assert_eq!(
        about,
        [("placeholder_value".to_string(), "SECRET_KEY".to_string())],
        "{v}"
    );
    assert_eq!(v["summary"]["errors"], 3, "{v}");
}

#[test]
fn t_i035_a_real_but_weak_secret_is_one_weak_finding() {
    let p = Project::new()
        .example("DB_PASSWORD=\n")
        .env("DB_PASSWORD=hunter2\n");
    let v = json(&p, &[]);
    assert_eq!(
        issues(&v),
        [("weak_secret".to_string(), "DB_PASSWORD".to_string())],
        "{v}"
    );
}

/// The de-duplication happens in the report, so ignoring the placeholder check
/// still leaves `changeme` caught as weak (review run 2d).
#[test]
fn t_i035_ignoring_placeholders_leaves_the_weak_finding() {
    let v = json(&fixture(), &["--ignore", "placeholder_value"]);
    let about: Vec<_> = issues(&v)
        .into_iter()
        .filter(|(_, var)| var == "SECRET_KEY")
        .collect();
    assert_eq!(
        about,
        [("weak_secret".to_string(), "SECRET_KEY".to_string())],
        "{v}"
    );
}

// ── I-038: an unknown issue type is refused, like an unknown format ────────

const TYPES: &[&str] = &[
    "missing_variable",
    "extra_variable",
    "placeholder_value",
    "boolean_trap",
    "weak_secret",
    "localhost_in_docker",
    "invalid_url",
    "invalid_port",
    "invalid_email",
    "format_mismatch",
    "duplicate_key",
    "unknown_spec_key",
    "unknown_environment",
    "env_not_ignored",
    "env_tracked",
];

#[test]
fn t_i038_an_unknown_ignore_type_exits_2_and_lists_the_types() {
    let out = run(
        fixture().path(),
        &["validate", "--ignore", "placeholder_vlaue"],
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains("unknown issue type 'placeholder_vlaue'"),
        "{}",
        out.stderr
    );
    for t in TYPES {
        assert!(out.stderr.contains(t), "{t} not listed:\n{}", out.stderr);
    }
}

#[test]
fn t_i038_an_unknown_type_in_the_config_exits_2_and_names_the_file() {
    let p = fixture().toml("[validate]\nignore = [\"bogus_type\"]\n");
    let out = run(p.path(), &["validate"]);
    out.assert_code(2);
    assert!(
        out.stderr.contains(".evnx.toml") && out.stderr.contains("[validate] ignore"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("'bogus_type'"), "{}", out.stderr);
}

#[test]
fn t_i038_every_listed_type_is_accepted() {
    let p = fixture();
    for t in TYPES {
        let out = run(p.path(), &["validate", "--ignore", t]);
        assert_ne!(out.code, Some(2), "--ignore {t} refused:\n{}", out.stderr);
    }
}

/// `--format` takes the same spellings as `scan` and `doctor` (one helper).
#[test]
fn t_i038_format_is_case_insensitive_like_scan_and_doctor() {
    let out = run(
        fixture().path(),
        &["validate", "--format", " JSON ", "--exit-zero"],
    );
    out.assert_code(0);
    serde_json::from_str::<Value>(&out.stdout).expect("JSON on stdout");
}

/// The other callers of the shared helper keep their exact messages.
#[test]
fn t_i038_scan_and_doctor_messages_are_unchanged() {
    let p = fixture();
    let scan = run(p.path(), &["scan", "--format", "Bogus"]);
    scan.assert_code(2);
    assert!(
        scan.stderr
            .contains("unknown format 'bogus' — expected one of: pretty, json, sarif, github"),
        "{}",
        scan.stderr
    );
    let sev = run(p.path(), &["scan", "--severity", "hgh"]);
    sev.assert_code(2);
    assert!(
        sev.stderr
            .contains("unknown severity 'hgh' — expected one of: high, medium, low"),
        "{}",
        sev.stderr
    );
    let doc = run(p.path(), &["doctor", "--format", "xml"]);
    doc.assert_code(2);
    assert!(
        doc.stderr
            .contains("unknown format 'xml' — expected one of: pretty, json"),
        "{}",
        doc.stderr
    );
}

// ── I-039, I-043: with a contract, --strict measures against [vars] ────────

#[test]
fn t_i039_with_a_contract_strict_names_vars() {
    let p = Project::new()
        .example("A=1\nB=1\n")
        .env("A=1\nB=1\nEXTRA=1\n")
        .toml("[vars.A]\n[vars.B]\n");
    let v = json(&p, &["--strict"]);
    let extra = of_type(&v, "extra_variable");
    assert_eq!(extra.len(), 1, "{v}");
    let msg = extra[0]["message"].as_str().unwrap();
    let hint = extra[0]["suggestion"].as_str().unwrap();
    assert!(
        msg.contains("[vars]") && !msg.contains(".env.example"),
        "{msg}"
    );
    assert!(hint.contains("[vars.EXTRA]"), "{hint}");
}

/// `--help`: "Measured against `.env.example`, or against `[vars]` when the
/// project has a contract."
#[test]
fn t_i039_with_a_contract_the_template_is_not_the_measure() {
    let p = Project::new()
        .example("A=1\nONLY_EX=1\n")
        .env("A=1\nONLY_EX=1\n")
        .toml("[vars.A]\n");
    let v = json(&p, &["--strict"]);
    let names: Vec<_> = of_type(&v, "extra_variable")
        .iter()
        .map(|i| i["variable"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["ONLY_EX"], "{v}");
}

#[test]
fn t_i039_without_a_contract_the_message_is_unchanged() {
    let p = Project::new().example("A=1\n").env("A=1\nEXTRA=1\n");
    let v = json(&p, &["--strict"]);
    assert_eq!(
        of_type(&v, "extra_variable")[0]["message"],
        "Extra variable not in .env.example: EXTRA"
    );
}

fn staged() -> Project {
    Project::new()
        .example("A=1\nPROD_REGION=\n")
        .file(".env.staging", "A=1\nPROD_REGION=eu-west-1\n")
        .file(".env.production", "A=1\nPROD_REGION=eu-west-1\n")
        .toml("[vars.A]\n[vars.PROD_REGION]\nenvironments = [\"production\"]\n")
}

#[test]
fn t_i043_a_production_only_variable_in_staging_is_reported_under_strict() {
    let v = json(&staged(), &["--env-name", "staging", "--strict"]);
    let extra = of_type(&v, "extra_variable");
    assert_eq!(extra.len(), 1, "{v}");
    assert_eq!(extra[0]["variable"], "PROD_REGION");
    assert!(
        extra[0]["message"].as_str().unwrap().contains("production"),
        "{}",
        extra[0]["message"]
    );
}

#[test]
fn t_i043_not_without_strict_and_not_where_it_applies() {
    let plain = json(&staged(), &["--env-name", "staging"]);
    assert!(of_type(&plain, "extra_variable").is_empty(), "{plain}");
    let prod = json(&staged(), &["--env-name", "production", "--strict"]);
    assert!(of_type(&prod, "extra_variable").is_empty(), "{prod}");
}

// ── I-042: environment names in the contract are checked ───────────────────

#[test]
fn t_i042_a_listed_name_that_matches_nothing_warns() {
    let p = Project::new()
        .example("A=1\n")
        .file(".env.production", "A=1\n")
        .toml("[vars.A]\nenvironments = [\"prodution\"]\n");
    let out = run(p.path(), &["validate", "--env-name", "production"]);
    out.assert_code(0);
    assert!(out.stdout.contains("prodution"), "{}", out.stdout);
    assert!(
        out.stdout.contains("did you mean 'production'"),
        "{}",
        out.stdout
    );
    let v = json(&p, &["--env-name", "production"]);
    let w = of_type(&v, "unknown_environment");
    assert_eq!(w.len(), 1, "one warning per name, not two: {v}");
    assert_eq!(w[0]["severity"], "warning");
}

#[test]
fn t_i042_a_file_named_near_a_listed_name_warns() {
    let p = Project::new()
        .example("A=1\n")
        .file(".env.prod", "A=1\n")
        .toml("[vars.A]\nenvironments = [\"production\"]\n");
    let v = json(&p, &["--env-name", "prod"]);
    let w = of_type(&v, "unknown_environment");
    assert_eq!(w.len(), 1, "{v}");
    let msg = w[0]["message"].as_str().unwrap();
    assert!(
        msg.contains(".env.prod") && msg.contains("production"),
        "{msg}"
    );
}

#[test]
fn t_i042_known_names_say_nothing() {
    let p = Project::new()
        .example("A=1\n")
        .file(".env.production", "A=1\n")
        .file(".env.ci", "A=1\n")
        .toml("[vars.A]\nenvironments = [\"production\", \"staging\", \"ci\"]\n");
    let v = json(&p, &["--env-name", "production"]);
    assert!(of_type(&v, "unknown_environment").is_empty(), "{v}");
}

// ── I-046, I-257: Docker files beside the env file, compose.yaml ───────────

#[test]
fn t_i046_a_dockerfile_beside_the_env_file_is_seen() {
    let p = Project::new()
        .file("api/.env.example", "DB_HOST=db\n")
        .file("api/.env", "DB_HOST=localhost\n")
        .file("api/Dockerfile", "FROM alpine\n");
    let v = json(&p, &["--env", "api/.env", "--example", "api/.env.example"]);
    assert_eq!(of_type(&v, "localhost_in_docker").len(), 1, "{v}");
}

#[test]
fn t_i046_a_dockerfile_at_the_project_root_is_seen_from_a_subfolder() {
    let p = Project::new()
        .git()
        .file("Dockerfile", "FROM alpine\n")
        .file("sub/.env.example", "DB_HOST=db\n")
        .file("sub/.env", "DB_HOST=localhost\n");
    let out = run(
        &p.join("sub"),
        &["validate", "--format", "json", "--exit-zero"],
    );
    let v: Value = serde_json::from_str(&out.stdout).expect("JSON");
    assert_eq!(of_type(&v, "localhost_in_docker").len(), 1, "{v}");
}

#[test]
fn t_i046_no_docker_file_no_warning() {
    let p = Project::new()
        .git()
        .example("DB_HOST=db\n")
        .env("DB_HOST=localhost\n");
    let v = json(&p, &[]);
    assert!(of_type(&v, "localhost_in_docker").is_empty(), "{v}");
}

#[test]
fn t_i257_compose_yaml_is_a_docker_file() {
    for name in ["compose.yaml", "compose.yml"] {
        let p = Project::new()
            .example("DB_HOST=db\n")
            .env("DB_HOST=localhost\n")
            .file(name, "services: {}\n");
        let v = json(&p, &[]);
        assert_eq!(of_type(&v, "localhost_in_docker").len(), 1, "{name}: {v}");
    }
}

#[test]
fn t_i257_doctor_finds_compose_yaml() {
    let p = Project::new().file("compose.yaml", "services: {}\n");
    let out = run(p.path(), &["doctor", "--format", "json"]);
    let v: Value = serde_json::from_str(&out.stdout).expect("JSON");
    let check = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "docker_config")
        .expect("docker_config check")
        .clone();
    assert_eq!(check["severity"], "ok", "{check}");
}

// ── D-27, DEC-VA1: the env file and .gitignore ─────────────────────────────

#[test]
fn t_d27_an_unignored_env_file_is_a_warning() {
    let p = Project::new().git().example("A=\n").env("A=1\n");
    let out = run(p.path(), &["validate"]);
    out.assert_code(0);
    assert!(out.stdout.contains(".gitignore"), "{}", out.stdout);
    let v = json(&p, &[]);
    let w = of_type(&v, "env_not_ignored");
    assert_eq!(w.len(), 1, "{v}");
    assert_eq!(w[0]["severity"], "warning");
}

#[test]
fn t_d27_a_tracked_env_file_is_an_error() {
    let p = Project::new()
        .git()
        .example("A=\n")
        .env("A=1\n")
        .file(".gitignore", ".env\n");
    p.force_add(".env");
    p.commit_all("oops");
    let out = run(p.path(), &["validate"]);
    out.assert_code(1);
    assert!(out.stdout.contains("tracked by git"), "{}", out.stdout);
    let v = json(&p, &[]);
    assert_eq!(of_type(&v, "env_tracked").len(), 1, "{v}");
    assert!(of_type(&v, "env_not_ignored").is_empty(), "{v}");
}

#[test]
fn t_d27_an_ignored_env_file_says_nothing() {
    let p = Project::new()
        .git()
        .example("A=\n")
        .env("A=1\n")
        .file(".gitignore", ".env*\n!.env.example\n");
    let v = json(&p, &[]);
    assert!(of_type(&v, "env_not_ignored").is_empty(), "{v}");
    assert!(of_type(&v, "env_tracked").is_empty(), "{v}");
}

#[test]
fn t_d27_outside_a_repository_says_nothing() {
    let p = Project::new().example("A=\n").env("A=1\n");
    let v = json(&p, &[]);
    assert!(of_type(&v, "env_not_ignored").is_empty(), "{v}");
}

#[test]
fn t_d27_the_check_can_be_ignored() {
    let p = Project::new().git().example("A=\n").env("A=1\n");
    let v = json(&p, &["--ignore", "env_not_ignored"]);
    assert!(of_type(&v, "env_not_ignored").is_empty(), "{v}");
}

// ── I-037, D-25: fixed vs needs your input ──────────────────────────────────

fn fixed_names(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key}[] in {v}"))
        .iter()
        .map(|f| f["variable"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn t_i037_fix_lists_fixed_and_needs_input_separately() {
    let p = fixture();
    let out = run(p.path(), &["validate", "--fix"]);
    out.assert_code(1);
    assert!(out.stdout.contains("Fixed (2)"), "{}", out.stdout);
    assert!(
        out.stdout.contains("Needs your input (2)"),
        "{}",
        out.stdout
    );
    // Listed once, under "Needs your input" — not again as a fresh error.
    assert!(
        !out.stdout.contains("STRIPE_KEY looks like a placeholder"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("2 need your input"), "{}", out.stdout);
    assert!(!out.stdout.contains("4 fixed"), "{}", out.stdout);
}

#[test]
fn t_i037_json_separates_fixed_from_needs_input() {
    let p = fixture();
    let out = run(p.path(), &["validate", "--fix", "--format", "json"]);
    out.assert_code(1);
    let v: Value = serde_json::from_str(&out.stdout).expect("JSON");
    assert_eq!(fixed_names(&v, "fixed"), ["SECRET_KEY", "DEBUG"], "{v}");
    assert_eq!(
        fixed_names(&v, "needs_input"),
        ["STRIPE_KEY", "OPTIONAL_X"],
        "{v}"
    );
    assert_eq!(v["summary"]["fixed_count"], 2, "{v}");
    assert_eq!(v["summary"]["needs_input"], 2, "{v}");
    // Still findings for a script: status, errors and exit agree.
    assert_eq!(v["status"], "failed");
}

#[test]
fn t_i037_exit_1_until_the_value_is_filled() {
    let p = Project::new().example("A=\nB=\n").env("A=1\n");
    run(p.path(), &["validate", "--fix"]).assert_code(1);
    std::fs::write(p.join(".env"), "A=1\nB=real\n").unwrap();
    run(p.path(), &["validate"]).assert_code(0);
}

#[test]
fn t_i037_fix_copies_usable_defaults_from_the_example() {
    let p = Project::new()
        .example(
            "API_PORT=8080\nREDIS_HOST=localhost\nOPTIONAL_X=\nSTRIPE_SECRET_KEY=sk_test_abc\n",
        )
        .env("");
    run(p.path(), &["validate", "--fix"]);
    let env = p.read(".env");
    assert!(env.contains("API_PORT=8080\n"), "{env}");
    assert!(env.contains("REDIS_HOST=localhost\n"), "{env}");
    assert!(env.contains("OPTIONAL_X=\n"), "{env}");
    // A secret-shaped name never takes the template's value.
    assert!(
        env.contains("STRIPE_SECRET_KEY=CHANGE_ME_SECURE_32_CHARS_MIN\n"),
        "{env}"
    );
    // Copied defaults that are real values are fixed, not "needs input".
    let v = json(&p, &[]);
    assert!(of_type(&v, "missing_variable").is_empty(), "{v}");
}

// ── S1: --fix --dry-run ─────────────────────────────────────────────────────

#[test]
fn t_s1_dry_run_writes_nothing() {
    let p = fixture();
    let before = p.read(".env");
    let out = run(p.path(), &["validate", "--fix", "--dry-run"]);
    // The verdict is the file as it stands: nothing was changed.
    out.assert_code(1);
    assert_eq!(p.read(".env"), before, "the dry run wrote .env");
    assert!(!p.exists(".env.bak"), "the dry run left a backup");
    assert!(out.stdout.contains("Would fix (2)"), "{}", out.stdout);
    assert!(
        out.stdout.contains("Would need your input (2)"),
        "{}",
        out.stdout
    );
    assert!(out.all().contains("nothing was written"), "{}", out.all());
    // A secret it would generate is not shown: the real run makes a new one.
    let hex64 = out
        .stdout
        .split(|c: char| !c.is_ascii_hexdigit())
        .any(|w| w.len() == 64);
    assert!(!hex64, "a generated secret was printed:\n{}", out.stdout);
}

#[test]
fn t_s1_dry_run_json_says_so() {
    let p = fixture();
    let out = run(
        p.path(),
        &["validate", "--fix", "--dry-run", "--format", "json"],
    );
    let v: Value = serde_json::from_str(&out.stdout).expect("JSON");
    assert_eq!(v["dry_run"], true, "{v}");
    assert_eq!(fixed_names(&v, "fixed"), ["SECRET_KEY", "DEBUG"], "{v}");
    assert!(!p.exists(".env.bak"));
}

#[test]
fn t_s1_dry_run_needs_fix() {
    run(fixture().path(), &["validate", "--dry-run"]).assert_code(2);
}

// ── S2: --fail-on warning|error ─────────────────────────────────────────────

fn warning_only() -> Project {
    Project::new().example("DEBUG=false\n").env("DEBUG=False\n")
}

#[test]
fn t_s2_fail_on_warning_exits_1_on_a_warning() {
    let p = warning_only();
    run(p.path(), &["validate"]).assert_code(0);
    let out = run(p.path(), &["validate", "--fail-on", "warning"]);
    out.assert_code(1);
    assert!(out.stderr.contains("1 warning"), "{}", out.stderr);
    run(p.path(), &["validate", "--fail-on", "error"]).assert_code(0);
}

#[test]
fn t_s2_fail_on_from_the_config_and_the_flag_wins() {
    let p = warning_only().toml("[validate]\nfail_on = \"warning\"\n");
    run(p.path(), &["validate"]).assert_code(1);
    run(p.path(), &["validate", "--fail-on", "error"]).assert_code(0);
}

#[test]
fn t_s2_exit_zero_still_wins() {
    let p = warning_only();
    run(
        p.path(),
        &["validate", "--fail-on", "warning", "--exit-zero"],
    )
    .assert_code(0);
}

#[test]
fn t_s2_an_unknown_level_exits_2() {
    let out = run(
        warning_only().path(),
        &["validate", "--fail-on", "warnings"],
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("expected one of: warning, error"),
        "{}",
        out.stderr
    );
    let p = warning_only().toml("[validate]\nfail_on = \"never\"\n");
    run(p.path(), &["validate"]).assert_code(2);
}

// ── I-036, D-29, DEC-VA2: how a variable is marked optional ────────────────

#[test]
fn t_i036_an_empty_template_value_says_how_to_make_it_optional() {
    let p = Project::new().example("A=1\nOPTIONAL_X=\n").env("A=1\n");
    let out = run(p.path(), &["validate"]);
    out.assert_code(1);
    assert!(out.stdout.contains("required = false"), "{}", out.stdout);
    assert!(out.stdout.contains("evnx spec init"), "{}", out.stdout);
}

#[test]
fn t_i036_add_custom_optional_writes_required_false() {
    let p = Project::new()
        .git()
        .example("A=1\n")
        .env("A=1\n")
        .toml("[vars.A]\n");
    run(p.path(), &["add", "custom", "OPT", "--optional", "--yes"]).assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(toml.contains("[vars.OPT]\nrequired = false"), "{toml}");
    assert!(
        p.read(".env.example").contains("(optional)"),
        "{}",
        p.read(".env.example")
    );
}

#[test]
fn t_i036_optional_and_required_conflict() {
    let p = Project::new().example("A=1\n");
    run(
        p.path(),
        &["add", "custom", "OPT", "--optional", "--required", "--yes"],
    )
    .assert_code(2);
}

#[test]
fn t_i036_optional_without_a_contract_says_how() {
    let p = Project::new().git().example("A=1\n").env("A=1\n");
    let out = run(p.path(), &["add", "custom", "OPT", "--optional", "--yes"]);
    out.assert_code(0);
    assert!(out.all().contains("evnx spec init"), "{}", out.all());
    assert!(!p.exists(".evnx.toml"), "a contract appeared unasked");
}

// ── --fix writes through core::safe_write ──────────────────────────────────

/// A guard for the switch from an in-place `fs::write` to `core::safe_write`
/// (atomic): what the old write got right — the link stays a link, the file
/// and its backup keep the mode — must survive it.
#[cfg(unix)]
#[test]
fn t_fix_keeps_the_mode_and_writes_through_a_symlink() {
    use std::os::unix::fs::PermissionsExt;
    let p = Project::new()
        .example("DEBUG=false\n")
        .file("real.env", "DEBUG=True\n")
        .mode("real.env", 0o640)
        .symlink("real.env", ".env");
    run(p.path(), &["validate", "--fix"]).assert_code(0);
    let meta = std::fs::symlink_metadata(p.join(".env")).unwrap();
    assert!(meta.file_type().is_symlink(), "the link was replaced");
    assert_eq!(p.read("real.env"), "DEBUG=true\n");
    let mode = std::fs::metadata(p.join("real.env"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640, "mode changed to {mode:o}");
    // The backup holds the same values, at the same mode.
    let bak = std::fs::metadata(p.join(".env.bak"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(bak, 0o640, "backup mode {bak:o}");
    assert_eq!(p.read(".env.bak"), "DEBUG=True\n");
}

// ── T-005: named tests for what had none ────────────────────────────────────

#[test]
fn t_t005_invalid_email_is_reported_with_formats() {
    let p = Project::new()
        .example("ADMIN_EMAIL=\nOPS_EMAIL=\n")
        .env("ADMIN_EMAIL=nobody\nOPS_EMAIL=ops+alerts@acme.io\n");
    let v = json(&p, &["--validate-formats"]);
    let bad: Vec<_> = of_type(&v, "invalid_email")
        .iter()
        .map(|i| i["variable"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(bad, ["ADMIN_EMAIL"], "{v}");
}

#[test]
fn t_t005_example_flag_is_the_template() {
    let p = Project::new()
        .file("tpl.example", "A=\nB=\n")
        .example("A=\n")
        .env("A=1\n");
    let v = json(&p, &["--example", "tpl.example"]);
    assert_eq!(
        issues(&v),
        [("missing_variable".to_string(), "B".to_string())],
        "{v}"
    );
}

#[test]
fn t_t005_exit_zero_is_findings_only() {
    let p = fixture();
    run(p.path(), &["validate", "--exit-zero"]).assert_code(0);
    run(p.path(), &["validate", "--exit-zero", "--env", "nope.env"]).assert_code(2);
    run(p.path(), &["validate", "--exit-zero", "--format", "xml"]).assert_code(2);
}

#[test]
fn t_t005_ignore_from_the_command_line_and_the_config_add_up() {
    let p = fixture().toml("[validate]\nignore = [\"duplicate_key\"]\n");
    let v = json(&p, &["--ignore", "boolean_trap"]);
    let types: Vec<_> = issues(&v).into_iter().map(|(t, _)| t).collect();
    assert!(!types.contains(&"duplicate_key".to_string()), "{v}");
    assert!(!types.contains(&"boolean_trap".to_string()), "{v}");
    assert!(types.contains(&"placeholder_value".to_string()), "{v}");
}

// ── I-260, DEC-VA4..VA6: what --fix may invent ───────────────────────────────

/// Every `NAME=value` line of `.env`, as a map.
fn env_values(p: &Project) -> std::collections::BTreeMap<String, String> {
    p.read(".env")
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn is_generated(v: &str) -> bool {
    v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit())
}

/// ⛔ The bug: `STRIPE_KEY=changeme` became 64 random hex characters, which
/// then passed validate — a fake provider key, reported as fixed.
#[test]
fn t_i260_a_provider_key_placeholder_is_not_invented() {
    let p = Project::new()
        .example("STRIPE_KEY=\nVERCEL_TOKEN=\n")
        .env("STRIPE_KEY=changeme\nVERCEL_TOKEN=changeme\n");
    run(p.path(), &["validate", "--fix"]).assert_code(1);
    let env = env_values(&p);
    assert_eq!(env["STRIPE_KEY"], "changeme");
    assert_eq!(env["VERCEL_TOKEN"], "changeme");
    // And the report does not offer `--fix` for them.
    let v = json(&p, &[]);
    for i in of_type(&v, "placeholder_value") {
        assert_eq!(i["auto_fixable"], false, "{i}");
    }
}

/// DEC-VA4: the app's own secrets and passwords are still generated.
#[test]
fn t_i260_app_secrets_and_passwords_are_still_generated() {
    let p = Project::new()
        .example("SECRET_KEY=\nJWT_SECRET=\nDB_PASSWORD=\n")
        .env("SECRET_KEY=changeme\nJWT_SECRET=changeme\nDB_PASSWORD=changeme\n");
    run(p.path(), &["validate", "--fix"]).assert_code(0);
    let env = env_values(&p);
    for name in ["SECRET_KEY", "JWT_SECRET", "DB_PASSWORD"] {
        assert!(is_generated(&env[name]), "{name}={}", env[name]);
    }
}

/// DEC-VA6: a missing secret evnx may generate is generated in one run, and
/// listed as fixed; a missing provider key still needs a person.
#[test]
fn t_i260_a_missing_app_secret_is_generated_at_once() {
    let p = Project::new()
        .example("SECRET_KEY=\nDB_PASSWORD=\nSTRIPE_KEY=\n")
        .env("");
    let out = run(p.path(), &["validate", "--fix", "--format", "json"]);
    out.assert_code(1);
    let v: Value = serde_json::from_str(&out.stdout).expect("JSON");
    assert_eq!(
        fixed_names(&v, "fixed"),
        ["SECRET_KEY", "DB_PASSWORD"],
        "{v}"
    );
    assert_eq!(fixed_names(&v, "needs_input"), ["STRIPE_KEY"], "{v}");
    let env = env_values(&p);
    assert!(is_generated(&env["SECRET_KEY"]), "{env:?}");
    assert!(is_generated(&env["DB_PASSWORD"]), "{env:?}");
    assert_eq!(env["STRIPE_KEY"], "CHANGE_ME_SECURE_32_CHARS_MIN");
}

/// The second half of the bug: a second `--fix` turned the "Needs your input"
/// placeholder into random hex.
#[test]
fn t_i260_a_second_fix_leaves_a_needs_input_placeholder() {
    let p = Project::new().example("STRIPE_KEY=\n").env("");
    run(p.path(), &["validate", "--fix"]).assert_code(1);
    run(p.path(), &["validate", "--fix"]).assert_code(1);
    assert_eq!(
        env_values(&p)["STRIPE_KEY"],
        "CHANGE_ME_SECURE_32_CHARS_MIN"
    );
}

/// DEC-VA5: `--rotate-weak-secrets` follows the same rule. A provider always
/// holds its key, so a value evnx invents is never right there.
#[test]
fn t_i260_rotate_does_not_replace_a_provider_key() {
    let p = Project::new()
        .example("STRIPE_SECRET_KEY=\nJWT_SECRET=\n")
        .env("STRIPE_SECRET_KEY=sk_live_short123\nJWT_SECRET=short\n");
    let out = run(p.path(), &["validate", "--fix", "--rotate-weak-secrets"]);
    out.assert_code(1);
    let env = env_values(&p);
    assert_eq!(env["STRIPE_SECRET_KEY"], "sk_live_short123");
    assert!(is_generated(&env["JWT_SECRET"]), "{env:?}");
    assert!(
        out.stdout
            .contains("Rotate STRIPE_SECRET_KEY where it is issued"),
        "{}",
        out.stdout
    );
    // It must not point at the flag that was just declined.
    let stripe = out
        .stdout
        .lines()
        .skip_while(|l| !l.contains("STRIPE_SECRET_KEY is too weak"))
        .take(3)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!stripe.contains("--rotate-weak-secrets"), "{stripe}");
}
