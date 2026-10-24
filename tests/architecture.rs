//! Architecture rules for the shared modules (T-SHARE-01, A-10).
//!
//! One concept, one module: commands keep orchestration, and the decisions
//! about what a value *is* live in `core`. These tests read the source, so a
//! second copy of a shared rule fails here rather than drifting quietly — the
//! shape that produced the placeholder defined four ways (U3 survey: 32 of 56
//! values judged differently).

use std::path::Path;

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn src() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// `core` never depends on a command.
#[test]
fn t_share_01_core_does_not_import_commands() {
    let mut files = Vec::new();
    rust_files(&src().join("core"), &mut files);
    let offenders: Vec<String> = files
        .iter()
        .filter(|f| {
            std::fs::read_to_string(f)
                .unwrap()
                .lines()
                .any(|l| !l.trim_start().starts_with("//") && l.contains("crate::commands::"))
        })
        .map(|f| f.display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "core imports a command: {offenders:?}"
    );
}

/// A placeholder or mask function outside `core` only asks `core`. A body that
/// does anything else is a second definition.
#[test]
fn t_share_01_placeholder_and_mask_have_one_definition() {
    let mut files = Vec::new();
    rust_files(&src(), &mut files);
    let mut offenders = Vec::new();
    for f in files.iter().filter(|f| !f.starts_with(src().join("core"))) {
        let text = std::fs::read_to_string(f).unwrap();
        // F6 (DEC-G14): the secret-name rules too — `validate` kept its own.
        for name in [
            "fn is_placeholder(",
            "fn mask(",
            "fn disclose(",
            "fn is_sensitive_key(",
            "fn is_secret_shaped(",
            "fn is_secret_name(",
        ] {
            let mut rest = text.as_str();
            while let Some(at) = rest.find(name) {
                let body = &rest[at..];
                let end = body.find("\n}\n").unwrap_or(body.len());
                let body = &body[..end];
                let delegates = body.contains("core::classify::") || body.contains("core::mask::");
                if !delegates {
                    offenders.push(format!("{} {}", f.display(), name));
                }
                rest = &rest[at + name.len()..];
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "second definitions outside core: {offenders:?}"
    );
}

/// Commands that report through a `Verdict` never exit on their own (U4,
/// I-116): a `process::exit` inside one skips `main`, so its error got its own
/// wording and, under `sync --check`, the wrong code.
#[test]
fn t_u4_verdict_commands_do_not_exit_themselves() {
    let mut offenders = Vec::new();
    for dir in ["commands/sync", "commands/migrate"] {
        let mut files = Vec::new();
        rust_files(&src().join(dir), &mut files);
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            if text
                .lines()
                .any(|l| !l.trim_start().starts_with("//") && l.contains("process::exit("))
            {
                offenders.push(f.display().to_string());
            }
        }
    }
    assert!(offenders.is_empty(), "exits outside main: {offenders:?}");
}

/// Every `.env` line evnx writes goes through `core::envfile::line` (U5,
/// I-114): a raw `KEY=value` lost everything after ` #`.
#[test]
fn t_i114_env_lines_have_one_writer() {
    let mut offenders = Vec::new();
    for dir in ["commands/sync", "commands/add", "schema"] {
        let mut files = Vec::new();
        rust_files(&src().join(dir), &mut files);
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            for (n, l) in text.lines().enumerate() {
                let code = !l.trim_start().starts_with("//");
                let raw = l.contains(r#"format!("{}={}"#)
                    || l.contains(r#"format!("{name}={"#)
                    || l.contains(r#"format!("{key}={"#);
                if code && raw {
                    offenders.push(format!("{}:{}", f.display(), n + 1));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "raw KEY=value writes: {offenders:?}");
}

/// Every cloud request path is built by `cloud::client::api_path`, which keeps
/// each value inside its own segment and refuses `.`/`..` (U7, I-217, A-43).
/// A `format!("/api/v1/…{x}")` let `org uninvite "../members/u-bob"` reach
/// `DELETE /orgs/o/members/u-bob`.
#[test]
fn t_i217_cloud_paths_have_one_builder() {
    let mut files = Vec::new();
    rust_files(&src().join("cloud"), &mut files);
    let mut offenders = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, l) in text.lines().enumerate() {
            let code = !l.trim_start().starts_with("//");
            // `/api/v1/probe/` is a client test's own fake route.
            // ⚠️ The path text, not `format!("/api/v1`: six calls had `format!(`
            // on one line and the path on the next, and passed the first version
            // of this test. A `"/api/v1…{…}"` literal is a hand-built path
            // however it is laid out.
            let literal_with_placeholder = l
                .split('"')
                .skip(1)
                .step_by(2)
                .any(|s| s.starts_with("/api/v1") && s.contains('{'));
            if code && literal_with_placeholder && !l.contains("/api/v1/probe/") {
                offenders.push(format!("{}:{}", f.display(), n + 1));
            }
        }
    }
    assert!(offenders.is_empty(), "hand-built API paths: {offenders:?}");
}

/// Whether a credential may do something is decided in one place:
/// `Client::require_user_session` (no API token) or `Client::require_credential`
/// (U7, A-37, DEC-AU3). Five copies plus a sixth that accepted tokens are how
/// `delete-account` reached the server with `EVNX_TOKEN` set (I-211).
#[test]
fn t_i211_one_credential_guard() {
    let mut files = Vec::new();
    rust_files(&src().join("cloud"), &mut files);
    let mut offenders = Vec::new();
    for f in files.iter().filter(|f| !f.ends_with("client.rs")) {
        let text = std::fs::read_to_string(f).unwrap();
        if text.contains("fn require_session(") {
            offenders.push(format!("{}: fn require_session", f.display()));
        }
        let mut rest = text.as_str();
        while let Some(at) = rest.find("fn require_login(") {
            let body = &rest[at..];
            let body = &body[..body.find("\n}\n").unwrap_or(body.len())];
            if !body.contains(".require_user_session(") || body.contains("is_api_token") {
                offenders.push(format!("{}: require_login decides itself", f.display()));
            }
            rest = &rest[at + 1..];
        }
    }
    assert!(offenders.is_empty(), "{offenders:?}");
}

/// `.evnx.toml` is located by `core::config::project_file` / `find` and written
/// by `core::config::save` only (R-spec, A-31). Three writers — `spec init`, the
/// `[vars]` writeback and `cloud link` — each used `std::fs::write`, none
/// atomically, and only one refused text that does not parse.
#[test]
fn t_i177_evnx_toml_has_one_writer_and_one_vars_check() {
    let mut files = Vec::new();
    rust_files(&src(), &mut files);
    let mut offenders = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let code: String = text
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let in_config = f.ends_with("core/config.rs");
        let locates = [
            "PROJECT_FILE",
            "CONFIG_FILE",
            "config::find",
            "config::load",
            "project_file",
        ]
        .iter()
        .any(|n| code.contains(n));
        if !in_config && locates && code.contains("fs::write(") {
            offenders.push(format!("{}: fs::write beside a config lookup", f.display()));
        }
        // DEC-SP7: one `has_vars_section`; I-190 (DEC-SP14): spec's own
        // secret-name list is gone for `core::classify`.
        for name in ["fn has_vars_section(", "fn looks_secret(", "SECRET_WORDS"] {
            if code.contains(name) && !(in_config && name == "fn has_vars_section(") {
                offenders.push(format!("{} {name}", f.display()));
            }
        }
    }
    assert!(offenders.is_empty(), "second definitions: {offenders:?}");
}

/// Source lines outside comments and the `#[cfg(test)]` tail of a file.
fn code_lines(text: &str) -> Vec<(usize, &str)> {
    text.split("#[cfg(test)]")
        .next()
        .unwrap_or_default()
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .collect()
}

/// T-SF-50, A-35, DEC-SF9: one place builds an HTTP client and words a
/// transport failure. `update --check` and `cloud` each had a builder and a
/// `transport_reason`, so a fix for the proxy CA (I-192) would have had to be
/// made twice and could land once.
///
/// `Client::new()` counts too: `migrate`'s GitHub destination built one per
/// request, with no timeout, until R-migrate moved it here (DEC-MG3, I-112).
#[test]
fn t_sf50_one_http_client_builder_and_one_transport_reason() {
    let mut files = Vec::new();
    rust_files(&src(), &mut files);
    let mut offenders = Vec::new();
    for f in files.iter().filter(|f| !f.ends_with("core/http.rs")) {
        let text = std::fs::read_to_string(f).unwrap();
        for (n, l) in code_lines(&text) {
            if l.contains("Client::builder(")
                // reqwest's, which takes nothing; `cloud::Client::new(server)`
                // is evnx's own wrapper over the shared builder.
                || l.contains("Client::new()")
                || l.contains("fn transport_reason(")
            {
                offenders.push(format!("{}:{}", f.display(), n + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "HTTP clients outside core::http: {offenders:?}"
    );
}

/// DEC-SF2, DEC-SF11, I-199: every `cargo install evnx` command evnx prints
/// builds every feature. Five copies carried three flag sets, and the upgrade
/// one (`--features cloud`) dropped `migrate` and `backup`. A bare
/// `` `cargo install evnx` `` (naming the default build) is not a command
/// evnx tells anyone to run, and stays allowed.
#[test]
fn t_dec_sf11_every_cargo_line_builds_every_feature() {
    let mut files = Vec::new();
    rust_files(&src(), &mut files);
    let mut offenders = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        for (n, l) in code_lines(&text) {
            let mut rest = l;
            while let Some(at) = rest.find("cargo install evnx --") {
                rest = &rest[at + "cargo install evnx --".len()..];
                if !rest.starts_with("all-features") {
                    offenders.push(format!("{}:{}: {}", f.display(), n + 1, l.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "cargo lines without --all-features: {offenders:?}"
    );
}

/// A-13, I-090: one glob matcher. `core/converter.rs` carried a second one
/// that handled `*` only at the ends and took its arguments the other way
/// round, so `convert --include` and `migrate --include` matched different
/// names for the same pattern.
#[test]
fn t_a13_one_glob_matcher() {
    let mut files = Vec::new();
    rust_files(&src(), &mut files);
    let offenders: Vec<String> = files
        .iter()
        .filter(|f| !f.ends_with("core/glob.rs"))
        .filter(|f| {
            let text = std::fs::read_to_string(f).unwrap();
            code_lines(&text)
                .iter()
                .any(|(_, l)| l.contains("fn glob_match"))
        })
        .map(|f| f.display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "glob matching outside core::glob: {offenders:?}"
    );
}

/// I-264, I-265: one yes/no prompt, `utils::prompt::yes_no`. dialoguer's
/// `Confirm` either answered on the first key — so the Enter after `y`
/// answered the next question (N-09) — or, waiting for Enter, never showed
/// the key pressed. Twelve questions still built their own.
#[test]
fn t_i265_one_yes_no_prompt() {
    let mut files = Vec::new();
    rust_files(&src(), &mut files);
    let mut offenders = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        for (n, l) in code_lines(&text) {
            if l.contains("Confirm::new") || l.contains("Confirm::with_theme") {
                offenders.push(format!("{}:{}: {}", f.display(), n + 1, l.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "yes/no prompts outside utils::prompt::yes_no: {offenders:?}"
    );
}

/// A vault target is turned into a vault in one place (A-301, DEC-VL10). Six
/// commands listed the vaults and picked one themselves, so `vault members` by
/// id with a vault-scoped token failed at a list call `cloud pull` had long
/// stopped making (I-309), and a refused list was reported bare.
///
/// Counted in `vault.rs`, where `resolve` lives: a call of `resolve` itself (not
/// `fetch_and_resolve`) that picks from a list by the user's `target`. Its unit
/// tests call it with literals, and `token.rs` and `session.rs` have resolvers of
/// their own for other things.
#[test]
fn t_a301_a_vault_target_is_resolved_in_one_place() {
    let path = src().join("cloud").join("vault.rs");
    let text = std::fs::read_to_string(&path).unwrap();
    let calls_resolve = |l: &str| {
        l.match_indices("resolve(&").any(|(i, _)| {
            !l[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    };
    let sites: Vec<usize> = text
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            !l.trim_start().starts_with("//") && calls_resolve(l) && l.contains("target)")
        })
        .map(|(n, _)| n + 1)
        .collect();
    assert_eq!(
        sites.len(),
        1,
        "vault.rs resolves a target at lines {sites:?}"
    );
}

/// The master password is proven in one place (A-302, I-316, DEC-VL7):
/// `auth::unlock` derives the key and opens the account's sealed keypair with
/// it, so a wrong password is caught before anything uses the key.
/// `derive_master_key_for_account` derived without checking, and three more
/// copies opened the keypair on their own, each with its own message.
///
/// Allowed elsewhere: `auth.rs` itself (register, unlock, the login backfill)
/// and `rotate.rs`, which derives the *new* key and re-opens what it sealed —
/// but proves the *current* password through `auth::unlock_with`.
#[test]
fn t_a302_the_master_password_is_proven_in_one_place() {
    let mut files = Vec::new();
    rust_files(&src().join("cloud"), &mut files);
    let mut offenders = Vec::new();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(f).unwrap();
        if text.contains("fn derive_master_key_for_account") {
            offenders.push(format!("{name}: fn derive_master_key_for_account"));
        }
        if name == "rotate.rs" && !text.contains("auth::unlock_with(") {
            offenders.push("rotate.rs: does not use auth::unlock_with".into());
        }
        if name == "auth.rs" || name == "rotate.rs" {
            continue;
        }
        for (n, l) in text.lines().enumerate() {
            let code = !l.trim_start().starts_with("//");
            if code && (l.contains("derive_master_key(") || l.contains("decrypt_private_key(")) {
                offenders.push(format!("{name}:{}", n + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the master key outside auth::unlock: {offenders:?}"
    );
}
