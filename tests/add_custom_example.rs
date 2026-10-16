//! What `evnx add custom` writes where (U8, I-240, DEC-U8-1).
//!
//! `add custom STRIPE_SECRET_KEY=sk_live_…` wrote the live key into
//! `.env.example` — the file that gets committed — as well as `.env`. Now
//! `.env.example` gets a placeholder for **every** name (owner's decision, not
//! only secret-looking ones), and the real value goes only to `.env`, which is
//! created 0600 if it does not exist yet.

mod support;

use support::{run, Project};

const KEY: &str = "sk_live_51H1234567890abcdefghijk";

fn add(p: &Project) -> support::Output {
    run(
        p.path(),
        &[
            "add",
            "custom",
            &format!("STRIPE_SECRET_KEY={KEY}"),
            "PORT=8080",
            "--yes",
        ],
    )
}

#[test]
fn t_i240_the_template_gets_placeholders_and_env_the_values() {
    let p = Project::new().env("A=1\n");
    add(&p).assert_code(0);
    let example = p.read(".env.example");
    assert!(
        !example.contains(KEY),
        "a live key in the template:\n{example}"
    );
    assert!(
        !example.contains("8080"),
        "every name gets a placeholder:\n{example}"
    );
    assert!(
        example.contains("STRIPE_SECRET_KEY=") && example.contains("PORT="),
        "{example}"
    );
    let env = p.read(".env");
    assert!(env.contains(KEY) && env.contains("PORT=8080"), "{env}");
}

#[test]
fn t_i240_values_are_not_lost_without_a_dotenv() {
    let p = Project::new();
    add(&p).assert_code(0);
    let env = p.read(".env");
    assert!(env.contains(KEY) && env.contains("PORT=8080"), "{env}");
    assert!(!p.read(".env.example").contains(KEY));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(p.path().join(".env"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "a new .env holding secrets is private");
    }
}

#[test]
fn t_i240_running_it_twice_is_not_a_conflict() {
    let p = Project::new().env("A=1\n");
    add(&p).assert_code(0);
    let out = add(&p);
    out.assert_code(0);
    assert!(
        !out.all().to_lowercase().contains("conflict"),
        "{}",
        out.all()
    );
    assert_eq!(p.read(".env").matches("STRIPE_SECRET_KEY=").count(), 1);
    assert_eq!(
        p.read(".env.example").matches("STRIPE_SECRET_KEY=").count(),
        1
    );
}
