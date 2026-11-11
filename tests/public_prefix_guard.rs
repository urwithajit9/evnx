//! F14-5: a secret under a browser-exposed prefix (`VITE_`, `NEXT_PUBLIC_`,
//! `REACT_APP_`, `EXPO_PUBLIC_`) is worse than the same secret on a server:
//! `scan` raises its severity and `validate` fails it, and both say the value
//! ships in the browser bundle.

mod support;

use support::{run, Project};

/// A Stripe-shaped live key, built at run time so no literal sits in the source.
fn stripe_key() -> String {
    ["sk", "_live_", "51Habcdefghijklmnop", "qrstuvwxyz123456"].concat()
}

fn json(p: &Project, args: &[&str]) -> serde_json::Value {
    let out = run(p.path(), args);
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", out.stdout))
}

fn rank(s: &str) -> u8 {
    match s {
        "low" => 0,
        "medium" => 1,
        "high" => 2,
        "critical" => 3,
        other => panic!("severity {other}"),
    }
}

/// A GitHub-token-shaped value (severity `high` on a server).
fn github_token() -> String {
    ["gh", "p_", "aB3dE5gH7jK9mN1pQ3sT5vX7zA9cD1fG3hJ5"].concat()
}

fn scan_severity(prefix: &str) -> u8 {
    let p = Project::new().env(&format!("{prefix}GITHUB_TOKEN={}\n", github_token()));
    let doc = json(&p, &["scan", ".env", "--format", "json"]);
    let found = doc["findings"].as_array().expect("findings");
    assert_eq!(found.len(), 1, "{doc:#}");
    rank(found[0]["severity"].as_str().unwrap())
}

#[test]
fn scan_raises_severity_under_every_public_prefix() {
    let base = scan_severity("");
    for prefix in ["VITE_", "NEXT_PUBLIC_", "REACT_APP_", "EXPO_PUBLIC_"] {
        let got = scan_severity(prefix);
        assert!(got > base, "{prefix}: {got} vs {base}");
    }
}

#[test]
fn scan_says_the_value_is_in_the_browser_bundle() {
    let p = Project::new().env(&format!("VITE_STRIPE_SECRET_KEY={}\n", stripe_key()));
    run(p.path(), &["scan", ".env", "--no-color"])
        .assert_contains("inlined into the client bundle");
}

#[test]
fn validate_fails_a_secret_under_a_public_prefix() {
    for prefix in ["VITE_", "NEXT_PUBLIC_", "REACT_APP_", "EXPO_PUBLIC_"] {
        let name = format!("{prefix}STRIPE_KEY");
        let p = Project::new()
            .env(&format!("{name}={}\n", stripe_key()))
            .example(&format!("{name}=\n"));
        let doc = json(&p, &["validate", "--format", "json"]);
        let hits: Vec<_> = doc["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["type"] == "public_secret")
            .collect();
        assert_eq!(hits.len(), 1, "{prefix}: {doc:#}");
        assert_eq!(hits[0]["severity"], "error");
        let msg = hits[0]["message"].as_str().unwrap();
        assert!(
            msg.contains("browser bundle") && msg.contains(prefix),
            "{msg}"
        );
        assert!(
            !msg.contains(&stripe_key()),
            "the value must not be printed"
        );
    }
}

#[test]
fn validate_leaves_a_public_value_and_an_unprefixed_secret_alone() {
    let p = Project::new()
        .env(&format!(
            "VITE_API_URL=https://api.example.com\nSTRIPE_KEY={}\n",
            stripe_key()
        ))
        .example("VITE_API_URL=\nSTRIPE_KEY=\n");
    run(p.path(), &["validate", "--format", "json"]).assert_not_contains("public_secret");
}

#[test]
fn public_secret_can_be_ignored() {
    let p = Project::new()
        .env(&format!("VITE_STRIPE_KEY={}\n", stripe_key()))
        .example("VITE_STRIPE_KEY=\n");
    run(
        p.path(),
        &["validate", "--ignore", "public_secret", "--format", "json"],
    )
    .assert_not_contains("public_secret");
}
