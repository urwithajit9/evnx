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
