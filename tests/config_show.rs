//! `evnx config show`: every effective setting with its source.
//!
//! The order of precedence (flag, environment, files, default) is checked one
//! rung at a time, and the values that may be secret are checked with canaries.

mod support;

use serde_json::Value;
use support::{Project, run_with};

fn json(p: &Project, env: &[(&str, &str)]) -> Value {
    let out = run_with(p.path(), &["config", "show", "--json"], env, None);
    out.assert_code(0);
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}\n{}", out.stdout))
}

fn setting<'a>(doc: &'a Value, key: &str) -> &'a Value {
    doc["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("no setting {key}"))
}

#[test]
fn a_set_key_names_its_line_and_the_rest_name_default() {
    let p = Project::new().file(
        ".evnx.toml",
        "# policy\n[scan]\nseverity = \"high\"\n\n[backup]\nkeep = 7\n",
    );
    let doc = json(&p, &[]);
    assert_eq!(doc["run"]["tool"], "evnx");
    assert_eq!(doc["run"]["schema_version"], 1);
    let sev = setting(&doc, "scan.severity");
    assert_eq!(sev["value"], "high");
    assert_eq!(sev["source"], ".evnx.toml:3");
    let keep = setting(&doc, "backup.keep");
    assert_eq!(
        (&keep["value"], &keep["source"]),
        (&Value::from(7), &Value::from(".evnx.toml:6"))
    );
    let conf = setting(&doc, "scan.confidence");
    assert_eq!(conf["source"], "default");
    assert_eq!(doc["files"][0]["role"], "project");
    assert_eq!(doc["files"][0]["path"], ".evnx.toml");
}

#[test]
fn no_project_file_means_every_row_is_a_default() {
    let p = Project::new();
    let doc = json(&p, &[]);
    assert!(doc["files"][0]["path"].is_null());
    assert_eq!(setting(&doc, "scan.severity")["source"], "default");
    assert_eq!(setting(&doc, "backup.keep")["value"], 3);
}

#[test]
fn an_unknown_key_is_a_warning_and_the_text_form_lists_it() {
    let p = Project::new().file(".evnx.toml", "[scan]\nseverty = \"high\"\n");
    let doc = json(&p, &[]);
    let w = doc["warnings"].as_array().unwrap();
    assert!(
        w.iter()
            .any(|w| w.as_str().unwrap().contains("scan.severty")),
        "{w:?}"
    );
    let out = run_with(p.path(), &["config", "show"], &[], None);
    out.assert_code(0);
    assert!(
        out.stdout.contains("Warnings") && out.stdout.contains("severty"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("scan.severity") && out.stdout.contains("SOURCE"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_file_that_does_not_parse_exits_2() {
    let p = Project::new().file(".evnx.toml", "[scan\nseverity = ");
    run_with(p.path(), &["config", "show"], &[], None).assert_code(2);
}

#[test]
fn global_flags_are_named_as_the_source() {
    let p = Project::new();
    let out = run_with(
        p.path(),
        &["-v", "--no-color", "config", "show", "--json"],
        &[],
        None,
    );
    out.assert_code(0);
    let doc: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(setting(&doc, "verbose")["source"], "flag --verbose");
    assert_eq!(setting(&doc, "color")["source"], "flag --no-color");
    assert_eq!(setting(&doc, "quiet")["source"], "default");
}

#[test]
fn a_policy_file_adds_its_rows_and_names_itself() {
    let p = Project::new().file(
        "ci-policy.toml",
        "required_rules = [\"aws-access-key\"]\nmax_severity_floor = \"high\"\n",
    );
    let doc = json(&p, &[("EVNX_POLICY", "ci-policy.toml")]);
    assert_eq!(setting(&doc, "policy")["source"], "env EVNX_POLICY");
    let floor = setting(&doc, "policy.max_severity_floor");
    assert_eq!(floor["value"], "high");
    assert!(
        floor["source"].as_str().unwrap().starts_with("policy "),
        "{floor}"
    );
    let file = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["role"] == "policy")
        .unwrap();
    assert!(file["path"].as_str().unwrap().ends_with("ci-policy.toml"));
}

#[test]
fn a_policy_file_that_is_missing_exits_2() {
    let p = Project::new();
    run_with(
        p.path(),
        &["config", "show"],
        &[("EVNX_POLICY", "nope.toml")],
        None,
    )
    .assert_code(2);
}

#[cfg(feature = "cloud")]
mod cloud {
    use super::*;

    const CANARY: &str = "canary-Zq7-do-not-print";

    fn cfg_dir(p: &Project, config_toml: Option<&str>) -> String {
        let dir = p.path().join("cfg");
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(t) = config_toml {
            std::fs::write(dir.join("config.toml"), t).unwrap();
        }
        dir.to_str().unwrap().to_string()
    }

    /// default, then `config.toml` (with its line), then the environment.
    #[test]
    fn the_server_follows_default_then_user_file_then_environment() {
        let p = Project::new();
        let dir = cfg_dir(&p, None);
        let doc = json(&p, &[("EVNX_CONFIG_DIR", &dir)]);
        let s = setting(&doc, "server");
        assert_eq!(
            (s["value"].as_str().unwrap(), s["source"].as_str().unwrap()),
            ("https://api.evnx.dev", "default")
        );
        assert!(doc["files"][1]["path"].is_null());

        let dir = cfg_dir(&p, Some("# mine\nserver = \"https://evnx.example.org/\"\n"));
        let doc = json(&p, &[("EVNX_CONFIG_DIR", &dir)]);
        let s = setting(&doc, "server");
        assert_eq!(s["value"], "https://evnx.example.org");
        assert!(
            s["source"].as_str().unwrap().ends_with("config.toml:2"),
            "{s}"
        );
        assert!(
            doc["files"][1]["path"]
                .as_str()
                .unwrap()
                .ends_with("config.toml")
        );
        assert_eq!(setting(&doc, "config_dir")["source"], "env EVNX_CONFIG_DIR");

        let doc = json(
            &p,
            &[
                ("EVNX_CONFIG_DIR", &dir),
                ("EVNX_SERVER", "https://env.example.org"),
            ],
        );
        let s = setting(&doc, "server");
        assert_eq!(s["value"], "https://env.example.org");
        assert_eq!(s["source"], "env EVNX_SERVER");
    }

    /// No credential, in any form, reaches stdout or stderr.
    #[test]
    fn credentials_are_never_printed() {
        let p = Project::new();
        let dir = cfg_dir(&p, None);
        let env = [
            ("EVNX_CONFIG_DIR", dir.as_str()),
            ("EVNX_TOKEN", CANARY),
            ("EVNX_PASSWORD", CANARY),
            ("EVNX_MASTER_PASSWORD", CANARY),
            ("EVNX_MASTER_PASSWORD_FILE", CANARY),
            ("EVNX_KEY_BUNDLE", CANARY),
            ("EVNX_KEY_BUNDLE_FILE", CANARY),
        ];
        for args in [
            &["config", "show"][..],
            &["config", "show", "--json"][..],
            &["-v", "config", "show"][..],
        ] {
            let out = run_with(p.path(), args, &env, None);
            out.assert_code(0);
            assert!(!out.stdout.contains(CANARY), "{args:?}\n{}", out.stdout);
            assert!(!out.stderr.contains(CANARY), "{args:?}\n{}", out.stderr);
        }
        let doc = json(&p, &env);
        for key in [
            "token",
            "password",
            "master_password",
            "master_password_file",
            "key_bundle",
            "key_bundle_file",
        ] {
            let s = setting(&doc, key);
            assert_eq!(s["value"], "set (hidden)", "{key}");
        }
        assert_eq!(setting(&doc, "token")["source"], "env EVNX_TOKEN");
    }

    /// A server value that is not a URL is refused without being quoted: it may
    /// hold a password.
    #[test]
    fn a_bad_server_is_refused_without_its_text() {
        let p = Project::new();
        let dir = cfg_dir(&p, None);
        let bad = format!("user:{CANARY}@host");
        let out = run_with(
            p.path(),
            &["config", "show"],
            &[("EVNX_CONFIG_DIR", &dir), ("EVNX_SERVER", &bad)],
            None,
        );
        out.assert_code(2);
        assert!(
            !out.stdout.contains(CANARY) && !out.stderr.contains(CANARY),
            "{}{}",
            out.stdout,
            out.stderr
        );
        assert!(out.stderr.contains("EVNX_SERVER"), "{}", out.stderr);
    }

    /// A user file that does not parse exits 2.
    #[test]
    fn a_malformed_user_file_exits_2() {
        let p = Project::new();
        let dir = cfg_dir(&p, Some("server = [1, 2]"));
        run_with(
            p.path(),
            &["config", "show"],
            &[("EVNX_CONFIG_DIR", &dir)],
            None,
        )
        .assert_code(2);
    }
}
