//! Scan precision fixes (v0.14.0): stateless GitHub tokens, names in
//! Dockerfiles / shell / Python, and the false positives the RM-70 corpus
//! found.
//!
//! ⚠️ Every secret-shaped value is assembled at run time from parts, so the
//! repository holds none.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// One finding: (rule id, line, variable).
type Hit = (String, usize, Option<String>);

/// Scan a project holding `files` (name, text) and return every finding.
fn scan(files: &[(&str, &str)]) -> Vec<Hit> {
    let d = TempDir::new().unwrap();
    for (name, text) in files {
        let p = d.path().join(Path::new(name));
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, text.replace("\r\n", "\n")).unwrap();
    }
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json", "."])
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let loc = f["location"].as_str().unwrap();
            // `./file:LINE (VAR)` or `./file:LINE`
            let after = loc.rsplit_once(':').map_or(loc, |(_, r)| r);
            let line = after
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap()
                .parse()
                .unwrap_or_else(|_| panic!("no line in {loc}"));
            (
                f["rule_id"].as_str().unwrap().to_string(),
                line,
                f["variable"].as_str().map(str::to_string),
            )
        })
        .collect()
}

fn rules(hits: &[Hit]) -> Vec<(&str, usize)> {
    hits.iter().map(|(r, l, _)| (r.as_str(), *l)).collect()
}

// ── SCAN-GHS ─────────────────────────────────────────────────────────────

/// `ghs_<appid>_<header>.<payload>.<signature>`, ~520 characters.
fn stateless() -> String {
    let seg = |n: usize, seed: usize| -> String {
        const A: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        (0..n)
            .map(|i| A[(i * 7 + seed * 13 + i / 5) % A.len()] as char)
            .collect()
    };
    format!(
        "ghs_{}_{}.{}.{}",
        "1234567",
        ["eyJ", &seg(33, 1)].concat(),
        ["eyJ", &seg(330, 2)].concat(),
        seg(140, 3)
    )
}

fn classic() -> String {
    ["ghs_", "r080UUTWjPBOaALo0vrQuGXc22ToCIyNaLQS", "WXYZ"].concat()
}

#[test]
fn stateless_installation_token_is_one_github_token_finding() {
    let t = stateless();
    assert!(t.len() > 500);
    let hits = scan(&[("ci.sh", &format!("echo start\nexport X={t}\n"))]);
    assert_eq!(rules(&hits), vec![("github-token", 2)], "{hits:?}");
}

#[test]
fn stateless_token_in_a_dotenv_is_not_also_a_jwt_or_entropy_hit() {
    let t = stateless();
    let hits = scan(&[(".env", &format!("GITHUB_TOKEN={t}\nNOTE=\"{t}\"\n"))]);
    // One finding per value, never a jwt or high-entropy-string beside it.
    assert_eq!(hits.len(), 2, "{hits:?}");
    for (rule, _, _) in &hits {
        assert!(rule != "jwt" && rule != "high-entropy-string", "{hits:?}");
    }
}

#[test]
fn trailing_punctuation_is_not_part_of_a_stateless_token() {
    let rule = evnx::core::secretscan::rules::builtin_rules()
        .into_iter()
        .find(|r| r.id.as_deref() == Some("github-token"))
        .unwrap();
    let re = regex::Regex::new(&rule.regex).unwrap();
    let t = stateless();
    for tail in [".", ",", ")", ";", "\""] {
        let text = format!("token ({t}{tail} done");
        let m = re
            .find(&text)
            .unwrap_or_else(|| panic!("no match before {tail:?}"));
        assert_eq!(m.as_str(), t, "swallowed {tail:?}");
    }
    // The classic form is untouched by the new branch.
    let c = classic();
    assert_eq!(re.find(&format!("({c}).")).unwrap().as_str(), c);
}

#[test]
fn classic_forty_char_form_still_matches() {
    let c = classic();
    assert_eq!(c.len(), 44);
    let hits = scan(&[(".env", &format!("GH={c}\n"))]);
    assert_eq!(rules(&hits), vec![("github-token", 1)], "{hits:?}");
}

// ── SCAN-NAMES ───────────────────────────────────────────────────────────

/// A random-looking 20-character password, built at run time.
fn pw() -> String {
    ["Zk3", "pQ9vL", "x2Rm", "T7bYw", "Nc4"].concat()
}

fn names(hits: &[Hit]) -> Vec<(&str, usize, &str)> {
    hits.iter()
        .map(|(r, l, v)| (r.as_str(), *l, v.as_deref().unwrap_or("")))
        .collect()
}

#[test]
fn dockerfile_env_and_arg_names_are_checked() {
    let p = pw();
    let text = format!(
        "FROM node:20\nARG NPM_TOKEN\nENV DB_PASSWORD={p}\nARG DB_PASSWORD={p}\nENV DB_PASSWORD {p}\n\
         ENV A=1 \\\n    DB_PASSWORD={p}\nENV NODE_ENV=production\n"
    );
    let hits = scan(&[("Dockerfile", &text)]);
    let lines: Vec<usize> = hits.iter().map(|h| h.1).collect();
    assert_eq!(lines, vec![3, 4, 5, 7], "{hits:?}");
    for (rule, _, var) in &hits {
        assert_eq!(rule, "sensitive-config-key");
        assert_eq!(var.as_deref(), Some("DB_PASSWORD"));
    }
}

#[test]
fn dockerfile_never_flags_a_name_without_a_value_or_a_reference() {
    let text = "FROM node:20\nARG DB_PASSWORD\nARG API_TOKEN=\nENV DB_PASSWORD=${DB_PASSWORD}\n\
                ENV API_TOKEN=$API_TOKEN\nENV DB_PASSWORD $PW\nENV SECRET=\"$(cat /run/secrets/x)\"\n\
                ENV DB_PASSWORD=changeme\nENV DB_PASSWORD=<your-password>\nENV DB_PASSWORD=xxxxxxxxxx\n\
                ENV DB_PASSWORD=your-password-here\nENV DB_PASSWORD=\"\"\n";
    assert_eq!(scan(&[("Dockerfile", text)]), vec![]);
}

#[test]
fn shell_script_names_are_checked_in_every_assignment_shape() {
    let p = pw();
    let text = format!(
        "#!/bin/bash\nexport DB_PASSWORD=\"{p}\"\nDB_PASSWORD={p}\nreadonly DB_PASSWORD='{p}'\n\
         DB_PASSWORD={p} psql -h db\nexport DB_PASSWORD={p}; echo ok\n"
    );
    for file in ["setup.sh", "run.bash", "run.zsh"] {
        let hits = scan(&[(file, &text)]);
        let lines: Vec<usize> = hits.iter().map(|h| h.1).collect();
        assert_eq!(lines, vec![2, 3, 4, 5, 6], "{file}: {hits:?}");
        assert!(
            hits.iter().all(|h| h.0 == "sensitive-config-key"),
            "{hits:?}"
        );
    }
    // A script with no extension is read by its shebang.
    let hits = scan(&[("deploy", &text)]);
    assert_eq!(hits.len(), 5, "{hits:?}");
}

#[test]
fn shell_never_flags_references_prompts_or_placeholders() {
    let text = "#!/bin/bash\nexport DB_PASSWORD=$DB_PASSWORD\nexport API_TOKEN=\"${API_TOKEN}\"\n\
                TOKEN=$(vault read -field=token secret/app)\nPASSWORD=`cat pw`\nexport DB_PASSWORD=\"\"\n\
                DB_PASSWORD=\nexport DB_PASSWORD\nread -s PASSWORD\nexport DB_PASSWORD=changeme\n\
                DB_PASSWORD=<password>\nDB_PASSWORD=xxxxxxxxxx\nDB_PASSWORD=your-password\n\
                # DB_PASSWORD=commentedout1234\n";
    assert_eq!(scan(&[("login.sh", text)]), vec![]);
}

#[test]
fn python_string_assignments_are_checked_by_name() {
    let p = pw();
    let text = format!(
        "import os\n\nDEBUG = False\nDB_PASSWORD = \"{p}\"\nAPI_TOKEN: str = '{p}'  # why\n"
    );
    let hits = scan(&[("settings.py", &text)]);
    assert_eq!(
        names(&hits),
        vec![
            ("sensitive-config-key", 4, "DB_PASSWORD"),
            ("sensitive-config-key", 5, "API_TOKEN")
        ],
        "{hits:?}"
    );
}

#[test]
fn python_never_flags_lookups_empties_or_placeholders() {
    let text = "import os\nfrom django.conf import settings\n\
                password = os.environ[\"DB_PASSWORD\"]\ntoken = os.getenv(\"API_TOKEN\")\n\
                secret = settings.SECRET\nDB_PASSWORD = \"\"\nDB_PASSWORD = \"changeme\"\n\
                DB_PASSWORD = \"<password>\"\nDB_PASSWORD = \"xxxxxxxxxx\"\nDB_PASSWORD = \"your-password\"\n\
                SECRET = get_secret(\"app/db\")\n\ndef check(password: str) -> bool:\n    return password != \"\"\n";
    assert_eq!(scan(&[("app.py", text)]), vec![]);
}

#[test]
fn names_are_checked_only_in_those_files_and_a_value_is_reported_once() {
    let p = pw();
    // A markdown file keeps value-only reading.
    assert_eq!(
        scan(&[("README.md", &format!("DB_PASSWORD={p}\n"))]),
        vec![]
    );
    // A key the value rules already found is one finding, not two.
    let stripe = format!("sk_live_{}", "51Habc1234567890ABCDEFGHq");
    let hits = scan(&[("setup.sh", &format!("export STRIPE_SECRET_KEY={stripe}\n"))]);
    assert_eq!(
        rules(&hits),
        vec![("stripe-live-secret-key", 1)],
        "{hits:?}"
    );
}

// ── SCAN-FP ──────────────────────────────────────────────────────────────

/// `n` base64 characters, deterministic and random-looking (high entropy).
fn b64(n: usize, seed: usize) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    (0..n)
        .map(|i| A[(i * 11 + seed * 17 + i / 3 + (i * i) % 7) % A.len()] as char)
        .collect()
}

fn pem(label: &str, seed: usize) -> String {
    format!(
        "-----BEGIN {label}-----\n{}\n{}\n{}\n-----END {label}-----\n",
        b64(64, seed),
        b64(64, seed + 1),
        b64(40, seed + 2)
    )
}

#[test]
fn a_private_key_block_is_one_finding_not_one_per_body_line() {
    for (file, label, rule) in [
        ("deploy.pem", "PRIVATE KEY", "private-key"),
        ("server.key", "RSA PRIVATE KEY", "private-key"),
        ("id_deploy", "OPENSSH PRIVATE KEY", "openssh-private-key"),
    ] {
        let hits = scan(&[(file, &pem(label, 3))]);
        assert_eq!(rules(&hits), vec![(rule, 1)], "{file}: {hits:?}");
    }
}

#[test]
fn a_secret_after_a_private_key_block_is_still_found() {
    let mut text = pem("PRIVATE KEY", 5);
    text.push_str(&format!("token {}\n", b64(48, 9).replace(['+', '/'], "x")));
    let hits = scan(&[("bundle.txt", &text)]);
    assert_eq!(
        rules(&hits),
        vec![("private-key", 1), ("high-entropy-string", 6)],
        "{hits:?}"
    );
}

#[test]
fn public_keys_and_certificates_are_not_secrets() {
    for label in [
        "PUBLIC KEY",
        "RSA PUBLIC KEY",
        "CERTIFICATE",
        "CERTIFICATE REQUEST",
    ] {
        let hits = scan(&[("public.pem", &pem(label, 7))]);
        assert_eq!(hits, vec![], "{label}");
    }
    let blob = format!("AAAAC3NzaC1lZDI1NTE5AAAA{}", b64(36, 4));
    let rsa = format!("AAAAB3NzaC1yc2EAAAADAQABAAABAQ{}", b64(120, 6));
    let text = format!("ssh-ed25519 {blob} deploy@laptop\nssh-rsa {rsa} ci@host\n");
    assert_eq!(scan(&[("authorized_keys", &text)]), vec![]);
    assert_eq!(scan(&[("id_ed25519.pub", &text)]), vec![]);
}

#[test]
fn a_cache_key_holding_a_digest_is_an_identifier() {
    let hex = |n: usize| -> String {
        (0..n)
            .map(|i| "0123456789abcdef".as_bytes()[(i * 7 + i / 3) % 16] as char)
            .collect()
    };
    let hits = scan(&[(
        ".env",
        &format!(
            "CACHE_KEY={}\nETAG_KEY={}\nIDEMPOTENCY_KEY={}\nCACHE_KEY={}\n",
            hex(32),
            hex(40),
            hex(64),
            hex(32)
        ),
    )]);
    assert_eq!(hits, vec![]);
}

#[test]
fn only_a_cache_digest_is_excused_by_its_name() {
    let hex = |n: usize| -> String {
        (0..n)
            .map(|i| "0123456789abcdef".as_bytes()[(i * 5 + i / 2) % 16] as char)
            .collect()
    };
    let text = format!(
        "API_KEY={}\nCACHE_API_KEY={}\nCACHE_SECRET_KEY={}\nHASH_KEY={}\nCHECKSUM_KEY={}\nSIGNING_KEY={}\n\
         CACHE_KEY={}\nCACHE_KEY={}\n",
        hex(32),
        hex(32),
        hex(32),
        hex(64),
        hex(64),
        hex(32),
        // not a digest length, and not hex
        hex(31),
        b64(32, 2).replace(['+', '/'], "k"),
    );
    let hits = scan(&[(".env", &text)]);
    let lines: Vec<usize> = hits.iter().map(|h| h.1).collect();
    assert_eq!(lines, vec![1, 2, 3, 4, 5, 6, 7, 8], "{hits:?}");
}

#[test]
fn a_stripe_publishable_key_is_public_but_a_secret_under_a_public_prefix_is_not() {
    let body: String = b64(24, 8).replace(['+', '/'], "q");
    let live = format!("pk_{}_{body}", "live");
    let test = format!("pk_{}_{body}", "test");
    let hits = scan(&[(
        ".env",
        &format!("NEXT_PUBLIC_STRIPE_KEY={live}\nVITE_STRIPE_KEY={test}\nSTRIPE_KEY={live}\n"),
    )]);
    assert_eq!(hits, vec![]);
    let secret = format!("sk_{}_{body}", "live");
    let hits = scan(&[(".env", &format!("NEXT_PUBLIC_STRIPE_KEY={secret}\n"))]);
    assert_eq!(
        rules(&hits),
        vec![("stripe-live-secret-key", 1)],
        "{hits:?}"
    );
}

#[test]
fn yaml_references_are_not_reported_as_secrets() {
    let text = "services:\n  app:\n    environment:\n      API_TOKEN: \"{{ .Values.apiToken }}\"\n\
                \x20     DB_PASSWORD: ${{ secrets.DB_PASSWORD }}\n    secrets:\n      - db_password\n      - api-token\n\
                env:\n  - name: DB_PASSWORD\n    valueFrom:\n      secretKeyRef:\n        name: app-secrets\n        key: password\n\
                envFrom:\n  - secretRef:\n      name: app-secrets\n";
    assert_eq!(scan(&[("docker-compose.yml", text)]), vec![]);
}

#[test]
fn yaml_real_values_next_to_references_are_still_reported() {
    let v = b64(30, 1).replace(['+', '/'], "z");
    let w = b64(30, 2).replace(['+', '/'], "z");
    let text = format!(
        "services:\n  app:\n    environment:\n      API_TOKEN: {v}\n    secrets:\n      - {w}\n      - Db7PassWord9\n"
    );
    let hits = scan(&[("docker-compose.yml", &text)]);
    let lines: Vec<usize> = hits.iter().map(|h| h.1).collect();
    assert_eq!(lines, vec![4, 6, 7], "{hits:?}");
}

#[test]
fn a_pass_suffix_is_a_password_name() {
    let p = pw();
    let hits = scan(&[(
        ".env",
        &format!(
            "RABBITMQ_DEFAULT_PASS={p}\nDB_PASS={p}\nBYPASS_CACHE=true-for-everyone\nMAX_PASS=3\n\
             COMPASS={p}\nLOW_PASS=changeme\n"
        ),
    )]);
    assert_eq!(
        names(&hits),
        vec![
            ("sensitive-config-key", 1, "RABBITMQ_DEFAULT_PASS"),
            ("sensitive-config-key", 2, "DB_PASS"),
        ],
        "{hits:?}"
    );
}
