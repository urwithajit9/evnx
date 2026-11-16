//! WP-6 (v0.14.2): `init` for projects that already have files. The adopt
//! flow itself is in `layouts.rs`, beside the named-only layout it acts on.
//!
//! - INIT-1: a live-looking credential in the template is said, by name.
//! - INIT-4: `--no-contract` writes no `.evnx.toml`.
//! - GEN-20: a generated template never turns debugging on.

mod support;

use support::{run, Project};

#[test]
fn init1_a_real_looking_key_in_the_template_is_named_never_shown() {
    let key = "sk_live_51HxR8aK2mQ7xL4vR8tZ1wB6nC3hJ9pD";
    let p = Project::new().git().example(&format!(
        "STRIPE_SECRET_KEY={key}\nAPI_KEY=your-api-key-here\nPORT=8080\n"
    ));
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0);
    out.assert_contains("looks like a real credential: STRIPE_SECRET_KEY");
    out.assert_contains("rotate it");
    out.assert_not_contains(key);
    out.assert_not_contains("API_KEY.");
}

#[test]
fn init1_placeholders_alone_say_nothing() {
    let p = Project::new()
        .git()
        .example("API_KEY=your-api-key-here\nDATABASE_URL=postgresql://user:password@localhost:5432/dbname\n");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_not_contains("real credential");
}

#[test]
fn init4_no_contract_writes_no_evnx_toml() {
    let p = Project::new().git().example("PORT=8080\nAPI_KEY=\n");
    let out = run(p.path(), &["init", "--yes", "--no-contract"]);
    out.assert_code(0);
    out.assert_not_contains(".evnx.toml");
    assert!(!p.exists(".evnx.toml"));
    assert!(p.exists(".env"));

    let out = run(p.path(), &["init", "--yes", "--no-contract", "--dry-run"]);
    out.assert_contains(".evnx.toml: no contract (--no-contract)");
}

#[test]
fn init4_the_next_steps_say_what_the_contract_is() {
    let p = Project::new().git().example("PORT=8080\n");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_contains(".evnx.toml is the contract 'evnx validate' checks: safe to delete");
}

#[test]
fn init4_adoption_with_no_contract_still_sets_the_default() {
    let p = Project::new()
        .git()
        .file(".env.prod", "A=1\n")
        .file(".env.prod.example", "A=\n");
    run(p.path(), &["init", "--yes", "--no-contract"]).assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(toml.contains("env_name = \"prod\""), "{toml}");
    assert!(!toml.contains("[vars"), "{toml}");
}

#[test]
fn gen20_no_generated_template_turns_debugging_on() {
    for stack in ["django", "laravel"] {
        let p = Project::new().git();
        run(p.path(), &["init", "--yes", "--with", stack]).assert_code(0);
        let t = p.read(".env.example");
        for line in t.lines().filter(|l| l.contains("DEBUG=")) {
            assert!(
                line.ends_with("=False") || line.ends_with("=false"),
                "{stack}: {line}"
            );
        }
    }
}
