//! The exit-code table: one source for `--help`, `evnx commands --json` and
//! `docs/exit-codes.md` (WP-9a).
//!
//! [`CODES`] says what each code means, once. [`exit_codes`] says which codes a
//! command can produce, by its path (`["cloud", "push"]`). Everything that
//! prints or checks an exit code reads one of the two:
//!
//! * `cli::exit_help` renders the "Exit codes" block of every `--help`;
//! * `commands --json` emits `exit_codes` per command;
//! * `docs/exit-codes.md` is generated from both, and a test fails when stale.
//!
//! ⚠️ Where a help text and the code disagree, the code wins: a command that
//! has `--yes` can be declined (4), whatever its old help said. The test
//! `tests/exit_contract.rs` checks the table against the clap tree (a `--yes`
//! flag means 4, a parent covers its children) so a new command cannot go
//! undocumented.

use serde::Serialize;

use crate::core::verdict::{EXIT_ASIDE, EXIT_DECLINED, EXIT_ERROR, EXIT_FINDINGS, EXIT_OK};

/// One code of the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExitRow {
    /// The process exit code.
    pub code: u8,
    /// A stable machine id (the same ids `EvnxError::code` uses).
    pub id: &'static str,
    /// One line, the same wherever it is shown.
    pub meaning: &'static str,
}

/// Broken pipe: stdout closed (`utils::term`).
pub const EXIT_BROKEN_PIPE: u8 = 141;

const fn row(code: u8, id: &'static str, meaning: &'static str) -> ExitRow {
    ExitRow { code, id, meaning }
}

/// Every code evnx can exit with, and what it means. In code order.
pub const CODES: &[ExitRow] = &[
    row(EXIT_OK, "ok", "Ran, nothing to report"),
    row(
        EXIT_FINDINGS,
        "findings",
        "Ran, and found something: findings, problems, a difference, not in sync",
    ),
    row(
        EXIT_ERROR,
        "error",
        "Could not run: usage, config, I/O, or refused before any change",
    ),
    row(
        EXIT_DECLINED,
        "declined",
        "Declined: answered no, or a confirmation was needed with no terminal and no --yes",
    ),
    row(
        EXIT_ASIDE,
        "restored_aside",
        "restore wrote the result aside, to <output>.restored",
    ),
    row(
        10,
        "not_signed_in",
        "Not signed in, or the token was rejected",
    ),
    row(11, "forbidden", "Forbidden, or the email is not verified"),
    row(12, "plan_limit", "The plan's limit is reached"),
    row(
        13,
        "unreachable",
        "Server unreachable, not an evnx server, or TLS roots unusable",
    ),
    row(14, "conflict", "Conflict, or locked"),
    row(15, "server_error", "Server error, or rate limited"),
    row(
        EXIT_BROKEN_PIPE,
        "broken_pipe",
        "Output closed early (broken pipe)",
    ),
];

/// The meaning of one code.
pub fn row_for(code: u8) -> Option<ExitRow> {
    CODES.iter().copied().find(|r| r.code == code)
}

/// 10-15 minus what a command cannot produce: every cloud request.
const SERVER: &[u8] = &[10, 11, 12, 13, 14, 15];
/// A Vercel request (platform): no plan limit, and only `push` conflicts.
const VERCEL: &[u8] = &[10, 11, 13, 15];

/// The codes a command can produce, and whether it passes a child's code on.
///
/// `path` is the command path without `evnx` (`[]` is the root, which has no
/// table of its own). A parent that only groups subcommands covers the union
/// of its children. 141 (broken pipe) is on every command.
fn spec(path: &[&str]) -> (Vec<u8>, bool) {
    let s = |base: &[u8], extra: &[&[u8]]| -> Vec<u8> {
        let mut v = base.to_vec();
        for e in extra {
            v.extend_from_slice(e);
        }
        v
    };
    let (mut codes, child): (Vec<u8>, bool) = match path {
        // ── local commands ──────────────────────────────────────────────
        ["init"] | ["add", ..] | ["undo"] => (vec![0, 2, 4], false),
        ["remove"] | ["sync"] | ["migrate"] => (vec![0, 1, 2, 4], false),
        // `set` asks before it replaces a value, and needs a value from stdin
        // or a prompt (4); `get` exits 1 for a name that is not there and 4
        // for a secret read without `--reveal`.
        ["set"] => (vec![0, 2, 4], false),
        ["get"] => (vec![0, 1, 2, 4], false),
        // ⚠️ `validate`, `convert` and `template` take `--yes` but never ask a
        // question that can be declined, so they never exit 4.
        ["validate"] | ["scan"] | ["diff"] | ["align"] | ["doctor"] | ["parse"] | ["update"] => {
            (vec![0, 1, 2], false)
        }
        ["convert"] | ["template"] => (vec![0, 2], false),
        ["scan", "rules"] => (vec![0, 1, 2, 4], false),
        ["scan", "rules", "test"] => (vec![0, 1, 2], false),
        ["scan", "rules", "update"] => (vec![0, 2, 4], false),
        ["scan", ..] => (vec![0, 2], false),
        ["spec"] | ["spec", "check"] => (vec![0, 1, 2], false),
        ["spec", ..] => (vec![0, 2], false),
        ["backup"] | ["history"] | ["commands"] | ["completions"] => (vec![0, 2], false),
        ["restore"] => (vec![0, 2, 4, 5], false),
        ["run"] => (vec![0, 2], true),
        // ── the platform (Vercel) ───────────────────────────────────────
        ["platform"] => (s(&[0, 1, 2, 4, 14], &[VERCEL]), false),
        ["platform", "diff"] => (s(&[0, 1, 2], &[VERCEL]), false),
        ["platform", "pull"] => (s(&[0, 2, 4], &[VERCEL]), false),
        ["platform", "push"] => (s(&[0, 1, 2, 4, 14], &[VERCEL]), false),
        // ── the evnx server ─────────────────────────────────────────────
        ["cloud"] => (s(&[0, 1, 2, 4], &[SERVER]), true),
        ["cloud", "run"] => (s(&[0, 2], &[SERVER]), true),
        ["cloud", "diff"] => (s(&[0, 1, 2], &[SERVER]), false),
        ["cloud", "unlink"] => (vec![0, 2], false),
        ["cloud", "push" | "pull" | "delete-version"] => (s(&[0, 2, 4], &[SERVER]), false),
        ["auth" | "org" | "vault"] => (s(&[0, 2, 4], &[SERVER]), false),
        ["auth", "token"] | ["auth", "sessions"] | ["auth", "devices"] => {
            (s(&[0, 2, 4], &[SERVER]), false)
        }
        ["auth", "token", "revoke"]
        | ["auth", "sessions", "revoke" | "revoke-others"]
        | ["auth", "devices", "disavow"]
        | ["auth", "rotate-master-password"]
        | ["org", "remove" | "delete" | "leave"]
        | ["vault", "delete" | "revoke" | "rekey" | "leave"] => (s(&[0, 2, 4], &[SERVER]), false),
        ["cloud", ..] | ["auth", ..] | ["org", ..] | ["vault", ..] => {
            (s(&[0, 2], &[SERVER]), false)
        }
        _ => (vec![0, 2], false),
    };
    codes.push(EXIT_BROKEN_PIPE);
    codes.sort_unstable();
    codes.dedup();
    (codes, child)
}

/// The exit codes `evnx <command_path>` can produce, in code order.
///
/// ```
/// use evnx::core::exit_table::exit_codes;
/// let codes: Vec<u8> = exit_codes(&["restore"]).iter().map(|r| r.code).collect();
/// assert_eq!(codes, [0, 2, 4, 5, 141]);
/// ```
pub fn exit_codes(command_path: &[&str]) -> Vec<ExitRow> {
    spec(command_path)
        .0
        .into_iter()
        .filter_map(row_for)
        .collect()
}

/// Whether the command returns its child's exit code once the child started
/// (`run`, `cloud run`).
pub fn passes_child_code(command_path: &[&str]) -> bool {
    spec(command_path).1
}

/// The "Exit codes" block of `--help` for one command: the same lines
/// everywhere, `  <code>  <meaning>`.
pub fn render_block(command_path: &[&str]) -> String {
    let mut out = String::from("Exit codes:");
    for r in exit_codes(command_path) {
        out.push_str(&format!("\n  {:>3}  {}", r.code, r.meaning));
    }
    if passes_child_code(command_path) {
        out.push_str("\n    N  Once the command has started: its own exit code, or 128+n if signal n killed it");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_unique_and_ascending() {
        let codes: Vec<u8> = CODES.iter().map(|r| r.code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(codes, sorted);
    }

    #[test]
    fn every_command_has_2_and_141() {
        for p in [&["init"][..], &["cloud", "push"], &["run"], &["nope"]] {
            let c: Vec<u8> = exit_codes(p).iter().map(|r| r.code).collect();
            assert!(
                c.contains(&0) && c.contains(&2) && c.contains(&141),
                "{p:?}"
            );
        }
    }

    #[test]
    fn block_shape() {
        let b = render_block(&["restore"]);
        assert!(b.starts_with("Exit codes:\n    0  "), "{b}");
        assert!(b.contains("\n    5  restore wrote"), "{b}");
    }
}
