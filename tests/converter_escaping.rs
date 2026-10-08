//! Every converter's output, handed to the thing that consumes it.
//!
//! ⛔ These exist because each emitter escaped inline and each got it
//! differently wrong. An escape list reviewed by eye passes review; output fed
//! to `sh` does not.
//!
//! Measured against the shipped 0.9.0 converter with
//! `CMD_SUB='$(touch /tmp/evnx-pwned)'`:
//!
//! ```text
//! $ evnx convert --to shell > out.sh
//! $ sh -n out.sh
//! out.sh: 13: Syntax error: Unterminated quoted string
//! $ . ./out.sh
//! /tmp/evnx-pwned          ← created
//! ```
//!
//! ⚠️ `cloud export --to shell` reaches the same converters, and there the
//! values come from **other vault members** — so a hostile value is reachable
//! by someone who is not the person running the command.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

/// The values that broke the emitters, as a `.env`.
const HOSTILE_ENV: &str = concat!(
    "CMD_SUB=$(touch /tmp/evnx-should-not-exist)\n",
    "BACKTICK=`touch /tmp/evnx-should-not-exist2`\n",
    "DOLLAR=pa$sw0rd\n",
    "PORT=8080\n",
    "BOOL=true\n",
    "COLON=key: value\n",
    "TRAIL=ends-with-backslash\\\n",
    "QUOTE=it's\n",
);

fn fixture() -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env"), HOSTILE_ENV).unwrap();
    d
}

fn convert(d: &TempDir, to: &str) -> String {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", to])
        .output()
        .unwrap();
    assert!(out.status.success(), "convert --to {to} failed");
    String::from_utf8(out.stdout).expect("utf-8")
}

/// ⛔ The decisive one: `sh` is the consumer, so `sh` decides.
#[test]
fn shell_output_parses_runs_nothing_and_keeps_every_value() {
    let d = fixture();
    let script = d.path().join("out.sh");
    fs::write(&script, convert(&d, "shell")).unwrap();

    // 1 · it is a valid script at all. The shipped version was not.
    let syntax = std::process::Command::new("sh")
        .arg("-n")
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        syntax.status.success(),
        "sh -n rejected the generated script: {}",
        String::from_utf8_lossy(&syntax.stderr)
    );

    // 2 · sourcing it runs no code and yields the values unchanged.
    let probe = "/tmp/evnx-should-not-exist";
    let _ = fs::remove_file(probe);
    let got = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            ". '{}'; printf '%s\\n%s\\n%s\\n%s' \"$CMD_SUB\" \"$DOLLAR\" \"$TRAIL\" \"$QUOTE\"",
            script.display()
        ))
        .output()
        .unwrap();
    assert!(
        !std::path::Path::new(probe).exists(),
        "sourcing the generated script executed a command substitution"
    );
    let lines: Vec<&str> = std::str::from_utf8(&got.stdout)
        .unwrap()
        .split('\n')
        .collect();
    assert_eq!(lines[0], "$(touch /tmp/evnx-should-not-exist)");
    assert_eq!(lines[1], "pa$sw0rd", "`$sw0rd` expanded away");
    assert_eq!(lines[2], "ends-with-backslash\\");
    assert_eq!(lines[3], "it's");
    let _ = fs::remove_file(probe);
}

/// Under `stringData:` kubectl requires strings. Unquoted YAML gave it an
/// integer for `PORT` and a boolean for `BOOL`, and rejected the Secret.
#[test]
fn kubernetes_output_is_valid_yaml_with_only_string_values() {
    let d = fixture();
    let doc: serde_yaml::Value = serde_yaml::from_str(&convert(&d, "kubernetes"))
        .expect("kubernetes output is not valid YAML");
    let sd = doc
        .get("stringData")
        .and_then(|v| v.as_mapping())
        .expect("stringData mapping");

    assert!(!sd.is_empty());
    for (k, v) in sd {
        assert!(
            v.is_string(),
            "{k:?} came back as {v:?}, which kubectl rejects under stringData"
        );
    }
    assert_eq!(
        sd.get(serde_yaml::Value::String("PORT".into()))
            .and_then(|v| v.as_str()),
        Some("8080")
    );
    assert_eq!(
        sd.get(serde_yaml::Value::String("COLON".into()))
            .and_then(|v| v.as_str()),
        Some("key: value")
    );
}

/// Compose reads `environment:` items as YAML and then interpolates `$`.
#[test]
fn compose_output_is_valid_yaml_and_escapes_interpolation() {
    let d = fixture();
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&convert(&d, "docker")).expect("compose output is not valid YAML");
    let items = doc
        .get("environment")
        .and_then(|v| v.as_sequence())
        .expect("environment sequence");

    for item in items {
        let s = item.as_str().expect("每 item is a string");
        assert!(s.contains('='), "item is not KEY=value: {s:?}");
        // A literal `$` must be written `$$`, or Compose expands it.
        if s.starts_with("DOLLAR=") {
            assert!(
                s.contains("$$"),
                "a literal $ was not escaped for Compose: {s:?}"
            );
        }
    }
}

/// `${` and `%{` are HCL interpolation and directive markers.
#[test]
fn terraform_output_does_not_interpolate_values() {
    let d = TempDir::new().unwrap();
    // ⚠️ Single-quoted on purpose. Unquoted, `${var.secret}` is a hard parse
    // error — evnx treats an undefined `${…}` as fatal with no fallback, which
    // is P3 in the review and a separate finding. Single quotes make the value
    // literal, which is what a person writing an HCL reference would do anyway.
    fs::write(
        d.path().join(".env"),
        "REF='${var.secret}'\nDIR='%{ if true }'\nQ='a\"b'\n",
    )
    .unwrap();
    let out = convert(&d, "terraform");
    assert!(
        out.contains("$${var.secret}"),
        "`${{` was left live:\n{out}"
    );
    assert!(out.contains("%%{ if true }"), "`%{{` was left live:\n{out}");
    assert!(out.contains(r#"a\"b"#), "a quote was not escaped:\n{out}");
}

/// ⛔ A value could inject a command into the generated Azure script, and
/// `echo` added a byte to every GCP secret. Both are run against stub binaries
/// that record what they were actually handed.
#[test]
fn generated_cloud_commands_pass_values_through_unchanged() {
    let d = TempDir::new().unwrap();
    let hostile = "x';touch /tmp/evnx-cloud-pwned;'";
    fs::write(d.path().join(".env"), format!("PWNED={hostile}\n")).unwrap();

    let bin = d.path().join("fakebin");
    fs::create_dir_all(&bin).unwrap();
    let got_az = d.path().join("az-got");
    let got_gc = d.path().join("gc-got");
    fs::write(
        bin.join("az"),
        format!(
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = --value ] && printf %s \"$2\" > '{}'; shift; done\n",
            got_az.display()
        ),
    )
    .unwrap();
    fs::write(
        bin.join("gcloud"),
        format!("#!/bin/sh\ncat > '{}'\n", got_gc.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for n in ["az", "gcloud"] {
            fs::set_permissions(bin.join(n), fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    let probe = "/tmp/evnx-cloud-pwned";
    let _ = fs::remove_file(probe);

    for (to, grep, got) in [
        ("azure", "az keyvault", &got_az),
        ("gcp", "gcloud secrets", &got_gc),
    ] {
        let script: String = convert(&d, to)
            .lines()
            .filter(|l| l.contains(grep))
            .collect::<Vec<_>>()
            .join("\n");
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "the generated {to} script failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        assert_eq!(
            fs::read_to_string(got).unwrap_or_default(),
            hostile,
            "{to} did not receive the value byte-for-byte"
        );
    }

    assert!(
        !std::path::Path::new(probe).exists(),
        "a generated cloud command injected a shell command"
    );
    let _ = fs::remove_file(probe);
}
