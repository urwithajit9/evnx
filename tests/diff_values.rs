//! What `evnx diff` shows of the values it compares (U3 part 1).
//!
//! - I-071, DEC-G7: `--format patch` and `--interactive` printed the real
//!   values of `.env` with no flag. They now print `<withheld>` unless
//!   `--show-values`; with it, exact values (a patch must apply) and a warning
//!   on stderr.
//! - I-076, DEC-G9: `--show-values` masked by key name only, so
//!   `DATABASE_URL=postgres://admin:s3cr3t@db` printed the password. Masking now
//!   reads the value: a URL keeps its structure, a token is masked whole.
//!
//! ⚠️ R-diff (I-078): against a template only key names are compared, so the
//! tests that read a changed value pass `--values`.

mod support;

use support::{run, run_with, Project};

const DB_URL_ENV: &str = "postgres://admin:s3cr3t@db.internal:5432/app";
const TOKEN: &str = "sk_live_51H1234567890abcdefghijk";

fn project() -> Project {
    Project::new()
        .env(&format!(
            "DATABASE_URL={DB_URL_ENV}\nUPSTREAM={TOKEN}\nPORT=9090\nONLY_IN_ENV=local\n"
        ))
        .example("DATABASE_URL=postgres://user:pass@localhost:5432/app\nUPSTREAM=\nPORT=8080\nNEW_ONE=x\n")
}

const SECRETS: &[&str] = &["s3cr3t", TOKEN];

#[test]
fn t_i071_patch_withholds_values_by_default() {
    let p = project();
    let out = run(p.path(), &["diff", "--format", "patch"]);
    for s in SECRETS {
        assert!(!out.all().contains(s), "{s} printed:\n{}", out.all());
    }
    assert!(out.stdout.contains("<withheld>"), "{}", out.stdout);
}

#[test]
fn t_dec_g7_show_values_gives_an_exact_patch_and_a_warning() {
    let p = project();
    let out = run(
        p.path(),
        &["diff", "--format", "patch", "--values", "--show-values"],
    );
    assert!(
        out.stdout.contains(&format!("- DATABASE_URL={DB_URL_ENV}")),
        "{}",
        out.stdout
    );
    assert!(out.stderr.contains("real values"), "{}", out.stderr);
}

#[test]
fn t_i071_interactive_withholds_values_by_default() {
    let p = project();
    let out = run_with(
        p.path(),
        &["diff", "--format", "patch", "--interactive"],
        &[],
        Some(b"n\nn\nn\nn\nn\n"),
    );
    for s in SECRETS {
        assert!(!out.all().contains(s), "{s} printed:\n{}", out.all());
    }
}

#[test]
fn t_i076_show_values_masks_credentials_inside_values() {
    let p = project();
    for format in ["pretty", "json"] {
        let out = run(
            p.path(),
            &["diff", "--format", format, "--values", "--show-values"],
        );
        for s in SECRETS {
            assert!(
                !out.all().contains(s),
                "{format}: {s} printed:\n{}",
                out.all()
            );
        }
        assert!(
            out.stdout
                .contains("postgres://admin:***@db.internal:5432/app"),
            "{format}: structure kept:\n{}",
            out.stdout
        );
        assert!(out.stdout.contains("9090"), "{format}: plain values shown");
    }
}
