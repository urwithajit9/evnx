//! `evnx scan` on YAML, JSON and TOML (C8, v0.13.0).
//!
//! Before C8 a structured file was read as text: only the regex rules and the
//! 32-character entropy fallback ran, so `password: <a real password>` in
//! `config.yml` scanned clean. Each string value is now checked by its key and
//! its value, alongside the per-line rules.
//!
//! ⚠️ Secret-shaped values are assembled at run time, so push protection never
//! sees one in the source.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

fn project(files: &[(&str, &str)]) -> TempDir {
    let d = TempDir::new().unwrap();
    for (name, body) in files {
        let p = d.path().join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }
    d
}

fn scan(d: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json"])
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{e}\n{text}\n{}", String::from_utf8_lossy(&out.stderr)))
}

/// A password no rule recognises: only its key says it is a secret.
fn password() -> String {
    format!("correct-{}", "horse-battery-staple")
}

fn stripe() -> String {
    format!("sk_live_{}", "51Habc1234567890ABCDEFGH")
}

#[test]
fn a_password_in_yaml_json_and_toml_is_reported_with_its_key_and_line() {
    let p = password();
    let d = project(&[
        (
            "config.yml",
            &format!("app:\n  name: demo\n  db:\n    password: {p}\n"),
        ),
        (
            "settings.json",
            &format!(
                "{{\n  \"name\": \"demo\",\n  \"api\": {{\n    \"password\": \"{p}\"\n  }}\n}}\n"
            ),
        ),
        (
            "app.toml",
            &format!("name = \"demo\"\n\n[database]\npassword = \"{p}\"\n"),
        ),
    ]);
    let doc = scan(&d, &[]);
    let locations: Vec<&str> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["location"].as_str().unwrap())
        .collect();
    for want in [
        "./app.toml:4 (database.password)",
        "./config.yml:4 (app.db.password)",
        "./settings.json:4 (api.password)",
    ] {
        assert!(locations.contains(&want), "{want} not in {locations:?}");
    }
    assert_eq!(doc["summary"]["total"], 3);
    for f in doc["files"].as_array().unwrap() {
        assert_eq!(f["read_as"], "structured", "{f}");
    }
}

#[test]
fn references_and_non_strings_are_not_reported() {
    let d = project(&[(
        "config.yml",
        "secret: ${DB_SECRET}\ntoken: null\napi_key: 12345678901234567890\n",
    )]);
    assert_eq!(scan(&d, &[])["summary"]["total"], 0);
}

#[test]
fn a_helm_template_falls_back_to_text_with_no_error() {
    let s = stripe();
    let d = project(&[(
        "templates/secret.yaml",
        // `{{- if }}` control lines are what make a chart invalid YAML; a
        // lone `{{ .Values.x }}` happens to parse, as a flow mapping.
        &format!(
            "{{{{- if .Values.enabled }}}}\napiVersion: v1\nkind: Secret\n\
             stringData:\n  password: {{{{ .Values.db.password }}}}\n  stripe: {s}\n{{{{- end }}}}\n"
        ),
    )]);
    let doc = scan(&d, &[]);
    let file = &doc["files"][0];
    assert_eq!(file["read_as"], "text", "{file}");
    assert_eq!(file["note"], "did not parse as yaml");
    // The line rules still ran.
    assert_eq!(doc["findings"][0]["rule_id"], "stripe-live-secret-key");
}

#[test]
fn a_value_in_a_yaml_comment_is_still_found() {
    let s = stripe();
    let d = project(&[("config.yml", &format!("name: demo\n# old key: {s}\n"))]);
    let doc = scan(&d, &[]);
    assert_eq!(doc["summary"]["total"], 1);
    assert_eq!(doc["findings"][0]["location"], "./config.yml:2");
}

#[test]
fn one_secret_found_by_both_paths_is_one_finding_with_its_key() {
    let s = stripe();
    let d = project(&[("config.yml", &format!("payments:\n  stripe_key: {s}\n"))]);
    let doc = scan(&d, &[]);
    assert_eq!(doc["summary"]["total"], 1, "{doc}");
    let f = &doc["findings"][0];
    assert_eq!(f["rule_id"], "stripe-live-secret-key");
    assert_eq!(f["location"], "./config.yml:2 (payments.stripe_key)");
}

/// A long sentence or a script in a structured value is not a secret, however
/// varied its characters: entropy judges tokens, as on the text path.
#[test]
fn prose_and_scripts_are_not_judged_by_entropy_as_a_whole() {
    let d = project(&[(
        "changelog.json",
        // Entropy 5.15 as one string, over the 4.8 threshold: a release note
        // like the ones that were reported on a real repository.
        "{\"text\": \"Reads YAML, JSON & TOML now: keys (db.password), lists [0..9], tabs; fixes #812/#820 @ v0.13!\"}\n",
    )]);
    assert_eq!(scan(&d, &[])["summary"]["total"], 0);
    // A value with spaces under a secret's name is still reported by name.
    let d = project(&[("config.yml", "password: \"correct horse battery staple\"\n")]);
    assert_eq!(scan(&d, &[])["summary"]["total"], 1);
}

/// Scan JSON gave a finding's place only as the `location` string, which a
/// script had to split. Each finding now has `file`, `line` and, in a
/// structured file, `key_path`; `location` is unchanged.
#[test]
fn json_findings_carry_file_line_and_key_path() {
    let p = password();
    let d = project(&[
        ("config.yml", &format!("app:\n  db:\n    password: {p}\n")),
        (".env", &format!("STRIPE_KEY={}\n", stripe())),
        ("notes.txt", &format!("key {}\n", stripe())),
    ]);
    let v = scan(&d, &[]);
    let findings = v["findings"].as_array().unwrap();
    let by_file = |f: &str| {
        findings
            .iter()
            .find(|x| x["file"] == f)
            .unwrap_or_else(|| panic!("{f}: {v:#}"))
    };
    let yml = by_file("./config.yml");
    assert_eq!(yml["line"], 3, "{yml:#}");
    assert_eq!(yml["key_path"], "app.db.password", "{yml:#}");
    assert_eq!(
        yml["location"], "./config.yml:3 (app.db.password)",
        "{yml:#}"
    );
    let env = by_file("./.env");
    assert_eq!(env["line"], 1, "{env:#}");
    assert!(
        env.get("key_path").is_none(),
        "a variable, not a key path: {env:#}"
    );
    assert_eq!(env["variable"], "STRIPE_KEY", "{env:#}");
    let txt = by_file("./notes.txt");
    assert_eq!(txt["line"], 1, "{txt:#}");
    assert!(txt.get("key_path").is_none(), "{txt:#}");
}

/// A pnpm lockfile names packages such as `@radix-ui/react-password-toggle-field`
/// under `dependencies:`; the name check read each as a password ("Sensitive
/// config key", high). In a lockfile only the key-format rules run: no name or
/// entropy guesses, while a credential in a registry URL or a real key is
/// still found, and the file says how it was read.
#[test]
fn lockfiles_get_key_format_rules_only() {
    let integrity = format!(
        "sha512-{}",
        "Kq3xV9pL2mN8rT5wY7zA1bC4dE6fG0hJ3kM5nP8qR2sU4vW6xY9zA1bC3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY=="
    );
    let pnpm = format!(
        "lockfileVersion: '9.0'\n\nsnapshots:\n\n  radix-ui@1.8.1(react@19.1.0):\n    \
         dependencies:\n      '@radix-ui/react-password-toggle-field': 0.2.1(@types/react-dom@19.3.0(@types/react@19.3.0))(react@19.1.0)\n      \
         '@radix-ui/react-one-time-password-field': 0.1.8(@types/react@19.3.0)(react@19.1.0)\n\npackages:\n\n  \
         left-pad@1.3.0:\n    resolution: {{integrity: {integrity}}}\n"
    );
    let npm = format!(
        "{{\n  \"name\": \"app\",\n  \"lockfileVersion\": 3,\n  \"packages\": {{\n    \
         \"node_modules/secret-handshake\": {{\n      \"version\": \"1.0.0\",\n      \
         \"resolved\": \"https://registry.npmjs.org/secret-handshake/-/secret-handshake-1.0.0.tgz\",\n      \
         \"integrity\": \"{integrity}\"\n    }},\n    \"node_modules/private\": {{\n      \
         \"resolved\": \"https://deploy:{}@registry.example.com/private/-/private-1.0.0.tgz\",\n      \
         \"api_token\": \"{}\"\n    }}\n  }}\n}}\n",
        "s3cretRegistryPw9",
        stripe()
    );
    let d = project(&[("web/pnpm-lock.yaml", &pnpm), ("package-lock.json", &npm)]);
    let v = scan(&d, &[]);
    let findings = v["findings"].as_array().unwrap();
    let rules: Vec<(&str, &str)> = findings
        .iter()
        .map(|f| (f["file"].as_str().unwrap(), f["rule_id"].as_str().unwrap()))
        .collect();
    assert!(
        !rules
            .iter()
            .any(|(_, r)| *r == "sensitive-config-key" || *r == "high-entropy-string"),
        "{rules:?}"
    );
    assert!(
        !rules.iter().any(|(f, _)| f.ends_with("pnpm-lock.yaml")),
        "{rules:?}"
    );
    assert!(
        rules.contains(&("./package-lock.json", "connection-string-credentials"))
            || rules
                .iter()
                .any(|(f, r)| *f == "./package-lock.json" && r.contains("url")),
        "a password in a registry URL is still found: {rules:?}"
    );
    assert!(
        rules
            .iter()
            .any(|(f, r)| *f == "./package-lock.json" && r.starts_with("stripe")),
        "a real key is still found: {rules:?}"
    );
    let notes: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["note"].as_str())
        .collect();
    assert_eq!(
        notes
            .iter()
            .filter(|n| n.contains("lockfile: only key-format rules"))
            .count(),
        2,
        "{notes:?}"
    );
}
