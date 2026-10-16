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
        for name in ["fn is_placeholder(", "fn mask(", "fn disclose("] {
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
