//! WP-11b: `--prefer env|vault` on `evnx run` and `evnx cloud run`.
//!
//! Rows P1-P6 of the WP-11 plan, adapted to the owner's decision: one explicit
//! flag, default `vault` (the env file or vault replaces what the environment
//! holds, as before), `--prefer env` keeps a variable the environment already
//! has (an empty one counts) and only fills what is unset. Notices name
//! variables, never values or lengths.
#![cfg(unix)]

mod support;

use support::{run_with, Output, Project};

const FILE_PW: &str = "file_secret_value_AAA1";
const ENV_PW: &str = "env_secret_value_BBB2";
const FILE_PLAIN: &str = "from_file_plain";
const ENV_PLAIN: &str = "from_env_plain";
const TOKEN: &str = "tok_secret_value_CCC3";

const FILE: &str = "PREFER_PLAIN=from_file_plain\nDB_PASSWORD=file_secret_value_AAA1\n\
                    API_TOKEN=tok_secret_value_CCC3\nONLY_IN_FILE=filled\n";

/// The values must never show up in evnx's own words.
const FIXTURE_VALUES: [&str; 5] = [FILE_PW, ENV_PW, FILE_PLAIN, ENV_PLAIN, TOKEN];

const SHOW: &str =
    r#"echo "plain=[$PREFER_PLAIN] pw=[$DB_PASSWORD] tok=[$API_TOKEN] only=[$ONLY_IN_FILE]""#;

fn local(args: &[&str], env: &[(&str, &str)]) -> Output {
    let p = Project::new().env(FILE);
    local_in(&p, args, env)
}

fn local_in(p: &Project, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut a = vec!["run"];
    a.extend_from_slice(args);
    a.extend(["--", "sh", "-c", SHOW]);
    run_with(p.path(), &a, env, None)
}

const BOTH: [(&str, &str); 2] = [("PREFER_PLAIN", ENV_PLAIN), ("DB_PASSWORD", ENV_PW)];

#[track_caller]
fn no_values_in_evnx_output(out: &Output, allowed_stdout: bool) {
    let text = if allowed_stdout {
        out.stderr.clone()
    } else {
        out.all()
    };
    for v in FIXTURE_VALUES {
        assert!(!text.contains(v), "a value leaked: {text}");
    }
}

#[test]
fn p1_default_the_file_wins_as_before() {
    for mask in ["--mask", "--no-mask"] {
        let out = local(&[mask], &BOTH);
        out.assert_code(0);
        assert!(
            out.stdout.contains(&format!("plain=[{FILE_PLAIN}]")),
            "{}",
            out.stdout
        );
        assert!(
            out.stderr.is_empty(),
            "default output must not change: {}",
            out.stderr
        );
    }
    let out = local(&["--prefer", "vault", "--no-mask"], &BOTH);
    assert!(
        out.stdout.contains(&format!("pw=[{FILE_PW}]")),
        "{}",
        out.stdout
    );
}

#[test]
fn p2_prefer_env_keeps_the_existing_value_and_fills_the_rest() {
    for mask in ["--mask", "--no-mask"] {
        let out = local(&["--prefer", "env", mask], &BOTH);
        out.assert_code(0);
        assert!(
            out.stdout.contains(&format!("plain=[{ENV_PLAIN}]")),
            "{}",
            out.stdout
        );
        assert!(out.stdout.contains("only=[filled]"), "{}", out.stdout);
    }
}

#[test]
fn p3_empty_but_present_counts_as_set() {
    let out = local(&["--prefer", "env"], &[("PREFER_PLAIN", "")]);
    out.assert_code(0);
    assert!(out.stdout.contains("plain=[]"), "{}", out.stdout);
    assert!(
        out.stderr
            .contains("kept from your environment: PREFER_PLAIN"),
        "{}",
        out.stderr
    );
}

#[test]
fn p3b_an_unset_name_is_filled_from_the_file_either_way() {
    for args in [&["--prefer", "env"][..], &["--prefer", "vault"][..]] {
        let out = local(args, &[]);
        assert!(out.stdout.contains("only=[filled]"), "{}", out.stdout);
        assert!(
            out.stdout.contains(&format!("plain=[{FILE_PLAIN}]")),
            "{}",
            out.stdout
        );
        assert!(!out.stderr.contains("kept from"), "{}", out.stderr);
    }
}

#[test]
fn p4_the_notice_names_variables_only_and_is_quiet_under_q() {
    let out = local(&["--prefer", "env"], &BOTH);
    assert!(
        out.stderr
            .contains("kept from your environment: PREFER_PLAIN, DB_PASSWORD"),
        "{}",
        out.stderr
    );
    no_values_in_evnx_output(&out, true);

    let out = run_with(
        Project::new().env(FILE).path(),
        &["-q", "run", "--prefer", "env", "--", "true"],
        &BOTH,
        None,
    );
    out.assert_code(0);
    assert!(!out.stderr.contains("kept from"), "{}", out.stderr);

    // The same value on both sides is not a shadow: nothing to say.
    let out = local(&["--prefer", "env"], &[("PREFER_PLAIN", FILE_PLAIN)]);
    assert!(!out.stderr.contains("kept from"), "{}", out.stderr);
}

#[test]
fn p4b_the_default_names_replaced_variables_only_under_verbose() {
    let quiet = local(&[], &BOTH);
    assert!(!quiet.stderr.contains("replaced"), "{}", quiet.stderr);
    let p = Project::new().env(FILE);
    let out = run_with(p.path(), &["--verbose", "run", "--", "true"], &BOTH, None);
    out.assert_code(0);
    assert!(
        out.stderr
            .contains("replaced from the vault or env file: PREFER_PLAIN, DB_PASSWORD"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("injecting 4 variable(s)"),
        "{}",
        out.stderr
    );
    no_values_in_evnx_output(&out, false);
}

#[test]
fn p4c_a_long_list_is_shortened_without_verbose_and_spelled_out_with_it() {
    let names = ["N1", "N2", "N3", "N4", "N5"];
    let file: String = names.iter().map(|n| format!("{n}=file\n")).collect();
    let env: Vec<(&str, &str)> = names.iter().map(|n| (*n, "env")).collect();
    let p = Project::new().env(&file);
    let out = run_with(
        p.path(),
        &["run", "--prefer", "env", "--", "true"],
        &env,
        None,
    );
    assert!(
        out.stderr.contains("N1, N2, N3 and 2 more"),
        "{}",
        out.stderr
    );
    let out = run_with(
        p.path(),
        &["--verbose", "run", "--prefer", "env", "--", "true"],
        &env,
        None,
    );
    assert!(out.stderr.contains("N1, N2, N3, N4, N5"), "{}", out.stderr);
}

#[test]
fn p5_masking_follows_what_the_child_receives() {
    // DB_PASSWORD is kept from the environment: not masked. API_TOKEN comes
    // from the file: masked.
    let out = local(&["--prefer", "env", "--mask"], &BOTH);
    assert!(
        out.stdout.contains(&format!("pw=[{ENV_PW}]")),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("tok=[***]"), "{}", out.stdout);
    assert!(!out.stdout.contains(TOKEN), "{}", out.stdout);
    // Default: the injected file value is masked, the replaced one is gone.
    let out = local(&["--mask"], &BOTH);
    assert!(out.stdout.contains("pw=[***]"), "{}", out.stdout);
    assert!(!out.all().contains(FILE_PW), "{}", out.all());
}

#[test]
fn p6_names_are_still_checked_whichever_side_wins() {
    let p = Project::new().env("NODE_OPTIONS=--require=/tmp/x.js\nOTHER=1\n");
    let marker = p.path().join("ran");
    let cmd = format!("touch {}", marker.display());
    let out = run_with(
        p.path(),
        &["run", "--prefer", "env", "--", "sh", "-c", &cmd],
        &[("NODE_OPTIONS", "--max-old-space-size=1")],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains(".env sets NODE_OPTIONS"),
        "{}",
        out.stderr
    );
    assert!(!marker.exists(), "the command ran");
    assert!(
        !out.stderr.contains("x.js") && !out.stderr.contains("max-old"),
        "{}",
        out.stderr
    );
}

#[test]
fn p7_an_unknown_side_is_a_usage_error_listing_both() {
    let p = Project::new().env(FILE);
    let out = run_with(
        p.path(),
        &["run", "--prefer", "foo", "--", "true"],
        &[],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("env") && out.stderr.contains("vault"),
        "{}",
        out.stderr
    );
}

#[cfg(feature = "cloud")]
mod cloud {
    use super::*;
    use evnx_crypto::{
        b64_encode, derive_master_key, encrypt_private_key, encrypt_vault, generate_keypair,
        generate_salt, salt_to_base64, vault_aad, wrap_vault_key_with_master_key, VaultKey,
    };

    const PW: &str = "correct horse battery staple";
    const VAULT_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";

    /// A server with one vault, `shared/development`, at version 1.
    fn server(text: &str) -> mockito::ServerGuard {
        let salt = generate_salt();
        let master = derive_master_key(PW.as_bytes(), &salt).unwrap();
        let keypair = generate_keypair();
        let sealed = encrypt_private_key(&keypair, &master).unwrap().to_base64();
        let me = format!(
            r#"{{"user_id":"u-me","email":"me@example.com","argon2_salt":"{}",
            "encrypted_private_key":"{sealed}","totp_enabled":false,"has_mlkem_key":true}}"#,
            salt_to_base64(&salt)
        );
        let mut s = mockito::Server::new();
        s.mock("GET", "/api/v1/auth/me")
            .with_status(200)
            .with_body(me)
            .create();
        s.mock("GET", "/api/v1/vaults")
            .with_status(200)
            .with_body(format!(
                r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"shared","environment":"development",
                "role":"owner","version_count":1,"updated_at":"2026-01-01T00:00:00Z"}}]}}"#
            ))
            .create();
        let vk = VaultKey::generate();
        let b = encrypt_vault(text.as_bytes(), &vk, &vault_aad(VAULT_ID, 1)).unwrap();
        let mut bytes = b.nonce.to_vec();
        bytes.extend_from_slice(&b.ciphertext);
        let wrapped = wrap_vault_key_with_master_key(&vk, &master).unwrap();
        s.mock(
            "GET",
            format!("/api/v1/vaults/{VAULT_ID}/versions/latest").as_str(),
        )
        .with_status(200)
        .with_body(r#"{"version_num":1}"#)
        .create();
        s.mock(
            "GET",
            format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
        )
        .with_status(200)
        .with_body(bytes)
        .create();
        s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
            .with_status(200)
            .with_body(format!(
                r#"{{"encrypted_vault_key":"{}"}}"#,
                b64_encode(&wrapped)
            ))
            .create();
        s
    }

    fn cloud(s: &mockito::ServerGuard, args: &[&str], env: &[(&str, &str)]) -> Output {
        let p = Project::new();
        let cfg = p.path().join("cfg");
        support::cloud::login(&cfg, &s.url());
        let mut a = vec!["cloud", "run", "--vault", "shared", "--password-stdin"];
        a.extend_from_slice(args);
        a.extend(["--", "sh", "-c", SHOW]);
        let mut e = vec![
            ("EVNX_SERVER", s.url()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap().to_string()),
        ];
        e.extend(env.iter().map(|(k, v)| (*k, v.to_string())));
        let e: Vec<(&str, &str)> = e.iter().map(|(k, v)| (*k, v.as_str())).collect();
        run_with(p.path(), &a, &e, Some(format!("{PW}\n").as_bytes()))
    }

    #[test]
    fn cloud_default_the_vault_wins_and_prefer_env_keeps_the_environment() {
        let s = server(FILE);
        let out = cloud(&s, &["--mask"], &BOTH);
        out.assert_code(0);
        assert!(
            out.stdout.contains(&format!("plain=[{FILE_PLAIN}]")),
            "{}",
            out.stdout
        );
        assert!(out.stdout.contains("pw=[***]"), "{}", out.stdout);
        assert!(
            !out.stderr.contains("kept from") && !out.stderr.contains("replaced"),
            "{}",
            out.stderr
        );

        let out = cloud(&s, &["--prefer", "env", "--mask"], &BOTH);
        out.assert_code(0);
        assert!(
            out.stdout.contains(&format!("plain=[{ENV_PLAIN}]")),
            "{}",
            out.stdout
        );
        assert!(
            out.stdout.contains(&format!("pw=[{ENV_PW}]")),
            "{}",
            out.stdout
        );
        assert!(
            out.stdout.contains("tok=[***]") && out.stdout.contains("only=[filled]"),
            "{}",
            out.stdout
        );
        assert!(
            out.stderr
                .contains("kept from your environment: PREFER_PLAIN, DB_PASSWORD"),
            "{}",
            out.stderr
        );
        no_values_in_evnx_output(&out, true);
    }

    #[test]
    fn cloud_no_mask_prefer_env_and_verbose_default() {
        let s = server(FILE);
        let out = cloud(
            &s,
            &["--prefer", "env", "--no-mask"],
            &[("PREFER_PLAIN", "")],
        );
        assert!(out.stdout.contains("plain=[]"), "{}", out.stdout);
        let out = cloud(&s, &["--verbose"], &BOTH);
        assert!(
            out.stderr
                .contains("replaced from the vault or env file: PREFER_PLAIN, DB_PASSWORD"),
            "{}",
            out.stderr
        );
        no_values_in_evnx_output(&out, true);
    }

    #[test]
    fn cloud_an_unknown_side_is_a_usage_error() {
        let s = server(FILE);
        let out = cloud(&s, &["--prefer", "foo"], &[]);
        out.assert_code(2);
        assert!(
            out.stderr.contains("env") && out.stderr.contains("vault"),
            "{}",
            out.stderr
        );
    }
}
