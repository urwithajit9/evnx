//! Project layouts that are not "one `.env` and one `.env.example`".
//!
//! Three folders under `tests/fixtures/layouts/`, each a layout seen in real
//! projects (values are fake):
//!
//! - `named-only/`: `.env.prod` and `.env.prod.example`, no `.env`. A server
//!   that holds one environment.
//! - `per-env-template/`: `.env.staging`, `.env.staging.example` and the
//!   project's `.env.example`, which lacks the staging-only names.
//! - `nested/`: a workspace `.evnx.toml` and, below it, `child/` with its own
//!   `.env` and no `.git`.
//!
//! Each fixture is copied to a temp folder before a command runs, so nothing
//! is written into the repository and the upward search for `.evnx.toml` never
//! reaches this repository's own.
//!
//! The first half of each group pins what is right today. The `#[ignore]`d
//! tests state the behaviour that is expected and not yet there; each one
//! names it, and the follow-up that makes it pass removes the `#[ignore]`.

mod support;

use std::path::{Path, PathBuf};
use support::{Output, run};

/// A fixture copied into a temp folder.
struct Layout {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Layout {
    fn new(name: &str) -> Layout {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/layouts")
            .join(name);
        let dir = tempfile::TempDir::new().unwrap();
        copy_tree(&src, dir.path());
        Layout {
            root: dir.path().to_path_buf(),
            _dir: dir,
        }
    }

    fn evnx(&self, args: &[&str]) -> Output {
        run(&self.root, args)
    }

    /// `evnx` run in a folder below the root.
    fn evnx_in(&self, sub: &str, args: &[&str]) -> Output {
        run(&self.root.join(sub), args)
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap_or_default()
    }

    /// Every file under the root with its content, to prove nothing changed.
    fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        collect(&self.root, &self.root, &mut files);
        files.sort();
        files
    }
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
            // The way a person keeps a real env file; keeps the mode warning
            // out of the output these tests read.
            #[cfg(unix)]
            if entry.file_name().to_string_lossy().starts_with(".env") {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            out.push((
                path.strip_prefix(root).unwrap().to_path_buf(),
                std::fs::read(&path).unwrap(),
            ));
        }
    }
}

// ─── named-only: .env.prod and .env.prod.example, no .env ────────────────────

#[test]
fn named_only_an_unknown_env_name_lists_the_ones_that_exist() {
    let l = Layout::new("named-only");
    let out = l.evnx(&["sync", "--check", "--env-name", "staging"]);
    out.assert_code(2);
    // WP-5: the per-environment template is not an environment (FLD-4).
    out.assert_contains("Available: prod");
    out.assert_not_contains("prod.example");
}

#[test]
fn named_only_without_an_env_file_is_no_verdict_never_a_pass() {
    let l = Layout::new("named-only");
    for args in [
        &["sync", "--check"][..],
        &["validate"][..],
        &["remove", "PORT", "--dry-run"][..],
    ] {
        let out = l.evnx(args);
        out.assert_code(2);
        out.assert_contains("No verdict");
    }
}

#[test]
fn named_only_remove_with_the_env_name_previews_both_files() {
    let l = Layout::new("named-only");
    let before = l.snapshot();
    let out = l.evnx(&["remove", "PORT", "--env-name", "prod", "--dry-run"]);
    out.assert_code(0);
    out.assert_contains("would remove PORT");
    out.assert_contains(".env.prod.example");
    assert_eq!(before, l.snapshot(), "--dry-run wrote a file");
}

#[test]
fn named_only_add_with_the_env_name_writes_the_named_env_file() {
    let l = Layout::new("named-only");
    let before = l.snapshot();
    let out = l.evnx(&["add", "custom", "K=v", "--env-name", "prod", "--dry-run"]);
    out.assert_code(0);
    out.assert_contains(".env.prod: would add 1 variable: K");
    assert_eq!(before, l.snapshot(), "--dry-run wrote a file");
}

// ─── per-env-template ────────────────────────────────────────────────────────

#[test]
fn per_env_template_an_unknown_env_name_lists_the_ones_that_exist() {
    let l = Layout::new("per-env-template");
    let out = l.evnx(&["sync", "--check", "--env-name", "prod"]);
    out.assert_code(2);
    // WP-5: the per-environment template is not an environment (FLD-4).
    out.assert_contains("Available: staging");
    out.assert_not_contains("staging.example");
}

#[test]
fn per_env_template_validate_with_the_env_name_passes() {
    let l = Layout::new("per-env-template");
    l.evnx(&["validate", "--env-name", "staging"])
        .assert_code(0);
}

#[test]
fn per_env_template_remove_with_the_env_name_previews_both_files() {
    let l = Layout::new("per-env-template");
    let before = l.snapshot();
    let out = l.evnx(&[
        "remove",
        "STAGING_BUCKET",
        "--env-name",
        "staging",
        "--dry-run",
    ]);
    out.assert_code(0);
    out.assert_contains("would remove STAGING_BUCKET");
    out.assert_contains(".env.staging.example");
    assert_eq!(before, l.snapshot(), "--dry-run wrote a file");
}

#[test]
fn per_env_template_add_with_the_env_name_writes_the_named_env_file() {
    let l = Layout::new("per-env-template");
    let before = l.snapshot();
    let out = l.evnx(&["add", "custom", "K=v", "--env-name", "staging", "--dry-run"]);
    out.assert_code(0);
    out.assert_contains(".env.staging: would add 1 variable: K");
    assert_eq!(before, l.snapshot(), "--dry-run wrote a file");
}

// ─── nested: a project below a folder that has .evnx.toml ────────────────────

/// Reading the parent's configuration stays: a monorepo's sub-folders share it.
#[test]
/// WP-5, D2 (FLD-11): with no git repository, the child is its own project.
/// The parent's `.evnx.toml` is not read, and `-v` says why.
fn nested_outside_a_repository_the_parents_config_is_not_read() {
    let l = Layout::new("nested");
    let before = l.snapshot();
    for args in [
        &["sync", "--check"][..],
        &["validate"][..],
        &["remove", "PORT", "--dry-run"][..],
        &["add", "custom", "K=v", "--dry-run"][..],
    ] {
        let out = l.evnx_in("child", args);
        out.assert_code(0);
        out.assert_not_contains("config    ../.evnx.toml");
    }
    let out = l.evnx_in("child", &["-v", "validate"]);
    out.assert_contains("../.evnx.toml not used: no git repository here");
    assert_eq!(before, l.snapshot(), "a read-only command wrote a file");
}

// ─── Expected, not there yet ─────────────────────────────────────────────────

#[test]
fn named_only_the_missing_env_message_names_the_env_file_that_exists() {
    let l = Layout::new("named-only");
    for args in [
        &["sync", "--check"][..],
        &["validate"][..],
        &["remove", "PORT", "--dry-run"][..],
    ] {
        let out = l.evnx(args);
        assert!(
            out.all().contains(".env.prod") && out.all().contains("--env-name prod"),
            "{args:?} does not name .env.prod:\n{}",
            out.all()
        );
    }
}

#[test]
fn sync_with_the_env_name_pairs_it_with_the_matching_template() {
    for (layout, name) in [("named-only", "prod"), ("per-env-template", "staging")] {
        let l = Layout::new(layout);
        let before = l.snapshot();
        let out = l.evnx(&["sync", "--check", "--env-name", name]);
        out.assert_code(0);
        out.assert_contains(&format!(".env.{name}.example"));
        assert_eq!(before, l.snapshot(), "{layout}: --check wrote a file");
    }
}

#[test]
fn validate_with_the_env_name_uses_the_matching_template() {
    let l = Layout::new("named-only");
    let out = l.evnx(&["validate", "--env-name", "prod"]);
    out.assert_code(0);
    out.assert_not_contains("does not exist");
}

#[test]
fn add_with_the_env_name_writes_the_matching_template() {
    for (layout, name) in [("named-only", "prod"), ("per-env-template", "staging")] {
        let l = Layout::new(layout);
        let out = l.evnx(&["add", "custom", "K=v", "--env-name", name, "--dry-run"]);
        out.assert_code(0);
        out.assert_contains(&format!(".env.{name}.example: would add 1 variable: K"));
        assert!(
            !out.all().contains(".env.example:"),
            "{layout}: the shared template is written:\n{}",
            out.all()
        );
    }
}

#[test]
fn nested_add_leaves_the_parents_config_alone() {
    let l = Layout::new("nested");
    let parent_before = l.read(".evnx.toml");
    l.evnx_in("child", &["add", "custom", "K=v", "--yes"]);
    assert_eq!(
        parent_before,
        l.read(".evnx.toml"),
        "the parent's .evnx.toml was written to"
    );
}

/// WP-5: inside a repository, `init` in a subfolder leaves the root's
/// `.evnx.toml` in force (I-177); `--here` makes the subfolder a project of
/// its own.
#[test]
fn init_here_writes_the_folders_own_config_inside_a_repository() {
    let p = support::Project::new()
        .toml("[vars.ROOT_ONLY]\nrequired = false\n")
        .git();
    std::fs::create_dir_all(p.path().join("svc")).unwrap();
    std::fs::write(p.path().join("svc/.env.example"), "PORT=\n").unwrap();
    let root_before = p.read(".evnx.toml");

    support::run(&p.path().join("svc"), &["init", "--here", "--yes"]).assert_code(0);
    assert_eq!(
        root_before,
        p.read(".evnx.toml"),
        "--here wrote the root's config"
    );
    assert!(p.read("svc/.evnx.toml").contains("PORT"));

    std::fs::create_dir_all(p.path().join("web")).unwrap();
    std::fs::write(p.path().join("web/.env.example"), "API_URL=\n").unwrap();
    support::run(&p.path().join("web"), &["init", "--yes"]).assert_code(0);
    // Without --here the root's config stays the one in force: no second
    // file below it (I-177). What init records into it is WP-6's question.
    assert!(!p.exists("web/.evnx.toml"));
}

// ─── WP-6: init adopts the named-only layout (FLD-10) ───────────────────────

#[test]
fn named_only_init_adopts_and_validate_then_needs_no_flags() {
    let l = Layout::new("named-only");
    let env_before = l.read(".env.prod");
    let template_before = l.read(".env.prod.example");

    let out = l.evnx(&["init", "--yes"]);
    out.assert_code(0);
    out.assert_contains("adopted .env.prod");
    out.assert_not_contains("Generate");
    let toml = l.read(".evnx.toml");
    assert!(toml.contains("[defaults]\nenv_name = \"prod\""), "{toml}");
    assert!(toml.contains("[vars.STRIPE_KEY]"), "{toml}");
    assert_eq!(
        l.read(".env.prod"),
        env_before,
        "adopt changed the env file"
    );
    assert_eq!(l.read(".env.prod.example"), template_before);
    assert!(!l.root.join(".env").exists(), "adopt created .env");

    let out = l.evnx(&["validate", "--exit-zero"]);
    out.assert_code(0);
    out.assert_contains(".env.prod ↔ .env.prod.example");
    out.assert_contains("All required variables present");
}

#[test]
fn named_only_init_dry_run_shows_the_adoption_and_writes_nothing() {
    let l = Layout::new("named-only");
    let before = l.snapshot();
    let out = l.evnx(&["init", "--dry-run"]);
    out.assert_code(0);
    out.assert_contains("Found");
    out.assert_contains(".env.prod (5 names)");
    out.assert_contains("would set [defaults] env_name = \"prod\"");
    assert_eq!(l.snapshot(), before);
}

#[test]
fn named_only_init_json_reports_the_default() {
    let l = Layout::new("named-only");
    let out = l.evnx(&["init", "--yes", "--json"]);
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(doc["defaults"]["env_name"], "prod");
    assert_eq!(doc["template"], ".env.prod.example");
}

#[test]
fn named_only_init_without_a_terminal_or_yes_asks_for_yes() {
    let l = Layout::new("named-only");
    let before = l.snapshot();
    let out = l.evnx(&["init"]);
    // No terminal and no --yes: declined (WP-9 D3).
    out.assert_code(4);
    out.assert_contains("evnx init --yes");
    assert_eq!(l.snapshot(), before);
}

#[test]
fn several_named_env_files_need_env_name_to_adopt() {
    let l = Layout::new("named-only");
    std::fs::write(l.root.join(".env.staging"), "APP_NAME=shop\n").unwrap();
    let out = l.evnx(&["init", "--yes"]);
    out.assert_code(2);
    out.assert_contains(".env.prod, .env.staging");
    out.assert_contains("evnx init --yes --env-name prod");
    let out = l.evnx(&["init", "--yes", "--env-name", "staging"]);
    out.assert_code(0);
    out.assert_contains("adopted .env.staging");
    assert!(l.read(".evnx.toml").contains("env_name = \"staging\""));
}

#[test]
fn named_only_adopted_contract_relaxes_only_real_defaults() {
    let l = Layout::new("named-only");
    l.evnx(&["init", "--yes"]).assert_code(0);
    let toml = l.read(".evnx.toml");
    let required = |name: &str| {
        let at = toml
            .find(&format!("[vars.{name}]"))
            .unwrap_or_else(|| panic!("{name} missing: {toml}"));
        toml[at..].lines().nth(1).unwrap().trim().to_string()
    };
    assert_eq!(
        required("PORT"),
        "required = false",
        "PORT=8080 is a default"
    );
    assert_eq!(required("APP_NAME"), "required = true", "an empty value");
    assert_eq!(required("STRIPE_KEY"), "required = true", "a secret");
    assert_eq!(
        required("DATABASE_URL"),
        "required = true",
        "a localhost URL is a sample"
    );
}
