//! One secret-name rule, at two levels (U8c, F6, DEC-G14).
//!
//! `validate` had its own suffix rule (`checks::is_secret_shaped`) beside the
//! substring rule everything else used (`patterns::is_sensitive_key`). They
//! disagreed on 8 of 32 names measured in U3. Both now live in
//! `core::classify`:
//!
//! - **Strict** — `validate` and `--fix`, where a hit demands a strong value
//!   and `--fix` may rewrite it. The suffix rule plus `PASSWD`, `*_PWD`,
//!   `*APIKEY`, `*CREDENTIALS`, so `ADMIN_PWD=123` is no longer waved through.
//! - **Lenient** — scan, masking, sync, where missing a secret costs the
//!   secret. The substring rule, unchanged.

mod support;

use support::{run, Project};

fn weak_vars(stdout: &str) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_str(stdout).unwrap();
    let mut v: Vec<String> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "weak_secret")
        .map(|i| i["variable"].as_str().unwrap().to_string())
        .collect();
    v.sort();
    v
}

/// The four of U3's eight that name a credential: validate now asks them for
/// a strong value. Before, all four validated clean with `123`.
#[test]
fn t_f6_validate_checks_pwd_apikey_credentials_passwd() {
    let body = "ADMIN_PWD=123\nSTRIPE_APIKEY=123\nCREDENTIALS=123\nPASSWD=123\n";
    let p = Project::new().env(body).example(body);
    let out = run(p.path(), &["validate", "--format", "json"]);
    assert_eq!(
        weak_vars(&out.stdout),
        ["ADMIN_PWD", "CREDENTIALS", "PASSWD", "STRIPE_APIKEY"],
        "{}",
        out.stdout
    );
}

/// The other four, and the rest of what the suffix rule exists for: a count,
/// a flag, an id, and the shell's own `PWD` are not credentials.
#[test]
fn t_f6_validate_leaves_settings_alone() {
    let body = "QUOTA_FREE_TOKENS=5\nAUTH_ENABLED=true\nKEY=1\nAPI_KEY_ID=12\n\
                SECRET_KEY_ROTATION_DAYS=30\nPWD=/srv/app\n";
    let p = Project::new().env(body).example(body);
    let out = run(p.path(), &["validate", "--format", "json"]);
    assert!(weak_vars(&out.stdout).is_empty(), "{}", out.stdout);
}

/// Lenient is unchanged: `diff --show-values` still masks all eight.
#[test]
fn t_f6_masking_stays_lenient() {
    let p = Project::new()
        .env("ADMIN_PWD=hunter2xyz\nAUTH_ENABLED=topsecretvalue\n")
        .example("ADMIN_PWD=\nAUTH_ENABLED=\n");
    let out = run(p.path(), &["diff", "--show-values"]);
    assert!(!out.all().contains("hunter2xyz"), "{}", out.all());
    assert!(!out.all().contains("topsecretvalue"), "{}", out.all());
}
