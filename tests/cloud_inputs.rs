//! Cloud command input that is refused before any request (U7 Part B, C-09).
//!
//! Every run points `EVNX_SERVER` at a closed local port and uses an empty
//! config directory: a refusal here happened before evnx tried to talk to a
//! server, which is the point.
//!
//! - I-205, I-210, DEC-AU2: `--expires-in-days` had no range — `0` and `=-5`
//!   minted a token already expired; `-5` got a clap tip suggesting it might be
//!   valid. Now 1–3650, one message.
//! - I-231, DEC-CL7: `cloud audit --last 9223372036854775807d` panicked
//!   ("TimeDelta::days out of bounds", exit 101).
#![cfg(feature = "cloud")]

mod support;

use support::{run_with, Project};

const CLOSED: &str = "http://127.0.0.1:9";

fn evnx(p: &Project, args: &[&str]) -> support::Output {
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    run_with(
        p.path(),
        args,
        &[
            ("EVNX_SERVER", CLOSED),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
        None,
    )
}

#[test]
fn t_i205_token_lifetime_is_1_to_3650_days() {
    let p = Project::new();
    for days in ["0", "-5", "3651", "9223372036854775807", "abc"] {
        let flag = format!("--expires-in-days={days}");
        let out = evnx(&p, &["auth", "token", "create", "ci", &flag]);
        out.assert_code(2);
        assert!(
            out.stderr.contains("1 to 3650"),
            "{days}: one clear message:\n{}",
            out.stderr
        );
    }
}

#[test]
fn t_i231_audit_window_never_panics() {
    let p = Project::new();
    for last in ["9223372036854775807d", "3651d", "0d"] {
        let out = evnx(&p, &["cloud", "audit", "--vault", "v", "--last", last]);
        out.assert_code(2);
        assert!(!out.stderr.contains("panicked"), "{last}:\n{}", out.stderr);
        assert!(out.stderr.contains("3650"), "{last}:\n{}", out.stderr);
    }
}

/// T-SHARE-52, I-213, DEC-AU7: every command that takes `--yes` also takes
/// `-y`. Seven did and five did not (`auth rotate-master-password`,
/// `org remove/delete/leave`, `cloud delete-version`), so a script's `-y`
/// worked on some destructive commands and was a clap error on others.
/// `delete-account` has no `--yes` on purpose (typed email) and is not listed.
#[test]
fn t_i213_every_yes_has_the_short_y() {
    let p = Project::new();
    let out = evnx(&p, &["commands", "--json"]);
    out.assert_code(0);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let mut missing = Vec::new();
    for c in json["commands"].as_array().unwrap() {
        for a in c["args"].as_array().unwrap() {
            if a["long"] == "yes" && a["short"] != "y" {
                missing.push(
                    c["path"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|s| s.as_str().unwrap())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
    }
    assert!(missing.is_empty(), "--yes without -y: {missing:?}");
}
