//! `evnx scan` policy floor, `--strict`, `--exit-only` and short secrets (WP-13).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// A repository with one finding at `high` (a database password), the repo
/// config `config` and nothing else a scan reads.
fn repo(config: &str) -> TempDir {
    let d = TempDir::new().unwrap();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(d.path())
        .status()
        .unwrap();
    fs::write(
        d.path().join("settings.env"),
        "DB_PASSWORD=hunter2hunter2\n",
    )
    .unwrap();
    if !config.is_empty() {
        fs::write(d.path().join(".evnx.toml"), config).unwrap();
    }
    d
}

fn evnx(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_evnx"));
    c.current_dir(dir)
        .args(args)
        .env_remove("EVNX_POLICY")
        .env("NO_COLOR", "1");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

const DISABLES: &str = "[scan]\ndisable = [\"Sensitive config key\"]\n";

fn policy(d: &TempDir, text: &str) -> String {
    let p = d.path().join("policy.toml");
    fs::write(&p, text).unwrap();
    p.to_string_lossy().into_owned()
}

// ---- policy ---------------------------------------------------------------

/// GEN-14: the repository disables the rule that finds its own secret.
#[test]
fn wp13_policy_forbids_disable_exit_2() {
    let d = repo(DISABLES);
    // Without a policy the repository silences the finding.
    assert_eq!(code(&evnx(d.path(), &["scan", "."], &[])), 0);
    let p = policy(&d, "allow_disable = false\n");
    let o = evnx(d.path(), &["scan", ".", "--policy", &p], &[]);
    assert_eq!(code(&o), 2, "{}{}", out(&o), err(&o));
    let e = err(&o);
    assert!(e.contains("scan.disable"), "{e}");
    assert!(e.contains("Sensitive config key"), "{e}");
    assert!(e.contains("policy.toml"), "{e}");
}

#[test]
fn wp13_policy_required_rule_cannot_be_disabled() {
    let d = repo(DISABLES);
    // By id, against a name in the repository config.
    let p = policy(&d, "required_rules = [\"sensitive-config-key\"]\n");
    let o = evnx(d.path(), &["scan", ".", "--policy", &p], &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("required_rules"), "{}", err(&o));
    // Another rule disabled: fine.
    let d2 = repo("[scan]\ndisable = [\"Slack Token\"]\n");
    let p2 = policy(&d2, "required_rules = [\"Sensitive config key\"]\n");
    assert_eq!(
        code(&evnx(d2.path(), &["scan", ".", "--policy", &p2], &[])),
        1
    );
}

#[test]
fn wp13_policy_builtins_false_conflicts() {
    let d = repo("[scan]\nbuiltins = false\n");
    let p = policy(&d, "required_rules = [\"AWS Access Key\"]\n");
    let o = evnx(d.path(), &["scan", ".", "--policy", &p], &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("scan.builtins = false"), "{}", err(&o));
}

#[test]
fn wp13_policy_max_floor_blocks_higher_severity() {
    let d = repo("[scan]\nseverity = \"critical\"\n");
    let p = policy(&d, "max_severity_floor = \"medium\"\n");
    let o = evnx(d.path(), &["scan", ".", "--policy", &p], &[]);
    assert_eq!(code(&o), 2);
    let e = err(&o);
    assert!(
        e.contains("scan.severity") && e.contains("max_severity_floor"),
        "{e}"
    );
    // The flag is held to the same floor; at or below it is fine.
    let d = repo("");
    let p = policy(&d, "max_severity_floor = \"medium\"\n");
    let o = evnx(
        d.path(),
        &["scan", ".", "--policy", &p, "--severity", "high"],
        &[],
    );
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("--severity"), "{}", err(&o));
    let o = evnx(
        d.path(),
        &["scan", ".", "--policy", &p, "--severity", "medium"],
        &[],
    );
    assert_eq!(code(&o), 1, "{}{}", out(&o), err(&o));
}

#[test]
fn wp13_policy_allows_listed_disable() {
    let d = repo("[scan]\ndisable = [\"Slack Token\"]\n");
    let p = policy(&d, "allow_disable = [\"Slack Token\"]\n");
    assert_eq!(
        code(&evnx(d.path(), &["scan", ".", "--policy", &p], &[])),
        1
    );
    let d = repo("[scan]\ndisable = [\"Slack Token\", \"Sensitive config key\"]\n");
    let p = policy(&d, "allow_disable = [\"Slack Token\"]\n");
    let o = evnx(d.path(), &["scan", ".", "--policy", &p], &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("Sensitive config key"), "{}", err(&o));
}

#[test]
fn wp13_policy_survives_no_config() {
    // `--no-config` drops the repository's settings, not the policy: a flag
    // the policy forbids is still refused.
    let d = repo("");
    let p = policy(&d, "max_severity_floor = \"low\"\n");
    let o = evnx(
        d.path(),
        &[
            "scan",
            ".",
            "--no-config",
            "--policy",
            &p,
            "--severity",
            "high",
        ],
        &[],
    );
    assert_eq!(code(&o), 2);
    // And a clean combination still scans.
    let o = evnx(d.path(), &["scan", ".", "--no-config", "--policy", &p], &[]);
    assert_eq!(code(&o), 1);
}

#[test]
fn wp13_policy_env_var_and_flag_precedence() {
    let d = repo(DISABLES);
    let strict = policy(&d, "allow_disable = false\n");
    let lax = d.path().join("lax.toml");
    fs::write(&lax, "").unwrap();
    let lax = lax.to_string_lossy().into_owned();
    // The variable alone applies.
    let o = evnx(d.path(), &["scan", "."], &[("EVNX_POLICY", &strict)]);
    assert_eq!(code(&o), 2, "{}", err(&o));
    // The flag wins over the variable, both ways.
    let o = evnx(
        d.path(),
        &["scan", ".", "--policy", &lax],
        &[("EVNX_POLICY", &strict)],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    let o = evnx(
        d.path(),
        &["scan", ".", "--policy", &strict],
        &[("EVNX_POLICY", &lax)],
    );
    assert_eq!(code(&o), 2);
}

#[test]
fn wp13_policy_unknown_key_is_error() {
    let d = repo("");
    let p = policy(&d, "required_rule = []\n");
    let o = evnx(d.path(), &["scan", ".", "--policy", &p], &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("policy.toml"), "{}", err(&o));
}

#[test]
fn wp13_policy_names_must_be_rules() {
    let d = repo("");
    let p = policy(&d, "required_rules = [\"No Such Rule\"]\n");
    let o = evnx(d.path(), &["scan", ".", "--policy", &p], &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("No Such Rule"), "{}", err(&o));
}

#[test]
fn wp13_policy_missing_file_exit_2() {
    let d = repo("");
    let o = evnx(d.path(), &["scan", ".", "--policy", "nowhere.toml"], &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("nowhere.toml"), "{}", err(&o));
    // Not clean: nothing was scanned.
    assert!(!out(&o).contains("No secrets"), "{}", out(&o));
    let o = evnx(d.path(), &["scan", "."], &[("EVNX_POLICY", "nowhere.toml")]);
    assert_eq!(code(&o), 2);
    // Invalid TOML.
    let p = policy(&d, "required_rules = [\n");
    assert_eq!(
        code(&evnx(d.path(), &["scan", ".", "--policy", &p], &[])),
        2
    );
}

#[test]
fn wp13_policy_conflict_reads_no_files() {
    let d = repo(DISABLES);
    let p = policy(&d, "allow_disable = false\n");
    let o = evnx(d.path(), &["scan", ".", "--policy", &p], &[]);
    assert_eq!(code(&o), 2);
    // No scan header, no file list, no finding: nothing was read.
    let all = format!("{}{}", out(&o), err(&o));
    assert!(
        !all.contains("scanning") && !all.contains("DB_PASSWORD"),
        "{all}"
    );
}

// ---- unknown keys ---------------------------------------------------------

#[test]
fn wp13_unknown_key_did_you_mean_scan_section() {
    let d = repo("[scan]\nseverty = \"high\"\n");
    let o = evnx(d.path(), &["scan", "."], &[]);
    assert!(err(&o).contains("did you mean `severity`?"), "{}", err(&o));
}

#[test]
fn wp13_unknown_key_did_you_mean_every_section() {
    for (toml, want) in [
        ("[defaults]\nenv_nme = \"a\"\n", "env_name"),
        ("[validate]\nfail_no = \"error\"\n", "fail_on"),
        ("[sync]\nnaming_polcy = \"warn\"\n", "naming_policy"),
        ("[diff]\nignore_key = []\n", "ignore_keys"),
        ("[backup]\nkep = 2\n", "keep"),
        ("[cloud]\nvaut = \"a\"\n", "vault"),
        (
            "[scan]\n[[scan.patterns]]\nname = \"n\"\nregx = \"a\"\nregex = \"a\"\n",
            "regex",
        ),
        ("[vars.A]\nrequird = true\n", "required"),
    ] {
        let d = repo(toml);
        let o = evnx(d.path(), &["scan", "."], &[]);
        let e = err(&o);
        assert!(
            e.contains(&format!("did you mean `{want}`?")),
            "{toml}\n{e}"
        );
    }
}

#[test]
fn wp13_unknown_key_warning_shown_under_q_for_scan() {
    let d = repo("[scan]\nseverty = \"high\"\n");
    let o = evnx(d.path(), &["-q", "scan", "."], &[]);
    assert!(
        err(&o).contains("unknown key `scan.severty`"),
        "{}",
        err(&o)
    );
    // Other commands keep -q silent.
    let o = evnx(d.path(), &["-q", "validate", "--exit-zero"], &[]);
    assert!(!err(&o).contains("unknown key"), "{}", err(&o));
}

#[test]
fn wp13_strict_unknown_key_exit_2() {
    let d = repo("[scan]\nseverty = \"high\"\n");
    let o = evnx(d.path(), &["scan", ".", "--strict"], &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("scan.severty"), "{}", err(&o));
    assert!(!out(&o).contains("scanning"), "{}", out(&o));
    // Without it the same config scans (and finds).
    assert_eq!(code(&evnx(d.path(), &["scan", "."], &[])), 1);
}

#[test]
fn wp13_strict_clean_config_ok() {
    let d = repo("[scan]\nseverity = \"low\"\n");
    assert_eq!(code(&evnx(d.path(), &["scan", ".", "--strict"], &[])), 1);
    let d = repo("");
    assert_eq!(code(&evnx(d.path(), &["scan", ".", "--strict"], &[])), 1);
}

// ---- --exit-only ----------------------------------------------------------

#[test]
fn wp13_exit_only_prints_nothing_exit_1_on_finding() {
    let d = repo("");
    let o = evnx(d.path(), &["scan", ".", "--exit-only"], &[]);
    assert_eq!(code(&o), 1);
    assert!(
        o.stdout.is_empty() && o.stderr.is_empty(),
        "{}{}",
        out(&o),
        err(&o)
    );
}

#[test]
fn wp13_exit_only_clean_exit_0() {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join("a.txt"), "nothing here\n").unwrap();
    let o = evnx(d.path(), &["scan", ".", "--exit-only"], &[]);
    assert_eq!(code(&o), 0);
    assert!(
        o.stdout.is_empty() && o.stderr.is_empty(),
        "{}{}",
        out(&o),
        err(&o)
    );
}

#[test]
fn wp13_exit_only_error_exit_2_and_silent() {
    let d = repo("");
    // A path that does not exist, a bad config value, a policy conflict.
    let o = evnx(d.path(), &["scan", "no/such/path", "--exit-only"], &[]);
    assert_eq!(code(&o), 2);
    assert!(
        o.stdout.is_empty() && o.stderr.is_empty(),
        "{}{}",
        out(&o),
        err(&o)
    );
    let d = repo(DISABLES);
    let p = policy(&d, "allow_disable = false\n");
    let o = evnx(d.path(), &["scan", ".", "--exit-only", "--policy", &p], &[]);
    assert_eq!(code(&o), 2);
    assert!(
        o.stdout.is_empty() && o.stderr.is_empty(),
        "{}{}",
        out(&o),
        err(&o)
    );
    // A warning and a strict failure print nothing either.
    let d = repo("[scan]\nseverty = \"high\"\n");
    let o = evnx(d.path(), &["scan", ".", "--exit-only", "--strict"], &[]);
    assert_eq!(code(&o), 2);
    assert!(
        o.stdout.is_empty() && o.stderr.is_empty(),
        "{}{}",
        out(&o),
        err(&o)
    );
    let o = evnx(d.path(), &["scan", ".", "--exit-only"], &[]);
    assert_eq!(code(&o), 1);
    assert!(
        o.stdout.is_empty() && o.stderr.is_empty(),
        "{}{}",
        out(&o),
        err(&o)
    );
}

#[test]
fn wp13_exit_only_conflicts_with_json() {
    let d = repo("");
    for flag in [
        &["--json"][..],
        &["--format", "json"],
        &["--format", "sarif"],
    ] {
        let mut args = vec!["scan", ".", "--exit-only"];
        args.extend_from_slice(flag);
        let o = evnx(d.path(), &args, &[]);
        assert_eq!(code(&o), 2, "{flag:?}");
        assert!(err(&o).contains("--exit-only"), "{}", err(&o));
    }
}

#[test]
fn wp13_exit_only_keeps_q_meaning_for_others() {
    // `-q` still prints findings; only --exit-only prints nothing.
    let d = repo("");
    let o = evnx(d.path(), &["-q", "scan", "."], &[]);
    assert_eq!(code(&o), 1);
    assert!(out(&o).contains("DB_PASSWORD"), "{}", out(&o));
}

// ---- short values ---------------------------------------------------------

fn short_repo(env: &str) -> TempDir {
    let d = repo("");
    fs::remove_file(d.path().join("settings.env")).unwrap();
    fs::write(d.path().join("settings.env"), env).unwrap();
    d
}

#[test]
fn wp13_short_password_reported_medium_low() {
    let d = short_repo("PASSWORD=zq81\nAPI_KEY=k9x2zq\nPRIVATE_KEY=ab12\n");
    let o = evnx(
        d.path(),
        &["scan", ".", "--confidence", "low", "--format", "json"],
        &[],
    );
    assert_eq!(code(&o), 1, "{}{}", out(&o), err(&o));
    let json: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    let findings = json["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 3, "{json}");
    for f in findings {
        assert_eq!(f["severity"], "medium", "{f}");
        assert_eq!(f["confidence"], "low", "{f}");
        assert_eq!(f["rule_id"], "sensitive-config-key", "{f}");
    }
    // Never the value.
    for v in ["zq81", "k9x2zq", "ab12"] {
        assert!(
            !out(&o).contains(v) && !err(&o).contains(v),
            "{v} was printed"
        );
    }
}

#[test]
fn wp13_short_value_placeholders_not_reported() {
    let d = short_repo(
        "PASSWORD=true\nSECRET=changeme\nTOKEN=1234\nAPI_KEY=$KEY\nSECRET_MODE=local\n\
         PASSWD=test\nPRIVATE_KEY=/etc/k\nTOKEN_TYPE=bearer\nPASSWORD=xxxx\nSECRET=<set>\n",
    );
    let o = evnx(d.path(), &["scan", ".", "--confidence", "low"], &[]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
}

#[test]
fn wp13_short_value_hidden_at_default_confidence() {
    let d = short_repo("PASSWORD=zq81\n");
    // The default floor is `low`, but not named: no new findings in CI.
    let o = evnx(d.path(), &["scan", "."], &[]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    // Named in the repository config.
    fs::write(
        d.path().join(".evnx.toml"),
        "[scan]\nconfidence = \"low\"\n",
    )
    .unwrap();
    assert_eq!(code(&evnx(d.path(), &["scan", "."], &[])), 1);
    // Named, but higher: hidden.
    let o = evnx(d.path(), &["scan", ".", "--confidence", "medium"], &[]);
    assert_eq!(code(&o), 0);
}

#[test]
fn wp13_short_value_is_a_sensitive_config_key_disable() {
    let d = short_repo("PASSWORD=zq81\n");
    fs::write(d.path().join(".evnx.toml"), DISABLES).unwrap();
    let o = evnx(d.path(), &["scan", ".", "--confidence", "low"], &[]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
}
