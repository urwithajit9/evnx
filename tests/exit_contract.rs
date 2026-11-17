//! The exit-code contract, one table for every command (WP-9a, step 9-10).
//!
//! `core::exit_table` says which codes each command can produce. It feeds the
//! "Exit codes" block of every `--help`, `exit_codes` in `evnx commands
//! --json` and `docs/exit-codes.md`. This file holds the three to it:
//!
//! * (a) every command in `commands --json` has a table, drawn from the
//!   contract's codes, and the table agrees with the clap tree;
//! * (b) a fixture drives each local command to each code it can reach
//!   without a network;
//! * (c) no `--help` names an exit code that is not in its table;
//! * `docs/exit-codes.md` is the generated file. After a deliberate change:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test --all-features --test exit_contract
//! ```
//!
//! Not driven here, because no small fixture reaches them without a server:
//! 10-15 (cloud, auth, org, vault, platform; `tests/cloud_exit_codes.rs` and
//! `tests/platform_vercel.rs` hold those), `update --check` 1 and 2 (the
//! network), `migrate` 1 (a GitHub upload that fails) and 4, `sync` 4's
//! interactive twin, `scan rules update` 4 and 141 (broken pipe, which
//! `tests/exit_verdicts.rs` and the `term` tests hold). The two commands that
//! hand on a child's code (`run`, `cloud run`) are marked by `child_exit_code`.

mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;

use support::surface::{entries, Entry};
use support::{run, Project};

const ALLOWED: &[u64] = &[0, 1, 2, 4, 5, 10, 11, 12, 13, 14, 15, 141];

/// Commands that take `--yes` and never ask anything that can be declined.
/// ⚠️ Accepted for consistency (WP-8); the code wins over the flag.
const YES_WITHOUT_A_QUESTION: &[&str] = &["validate", "convert", "template"];

/// Commands that exit 4 and have no `--yes`: `get` refuses a secret without
/// `--reveal`. The flag that answers is `--reveal`, not `--yes`.
const FOUR_WITHOUT_YES: &[&str] = &["get"];

fn codes(e: &Entry) -> Vec<u64> {
    e.json["exit_codes"]
        .as_array()
        .expect("`exit_codes` is an array")
        .iter()
        .map(|r| r["code"].as_u64().unwrap())
        .collect()
}

fn has_yes(e: &Entry) -> bool {
    e.args().iter().any(|a| a["long"] == "yes")
}

// ── (a) the table, per command ───────────────────────────────────────────

#[test]
fn a_every_command_has_a_table_drawn_from_the_contract() {
    let all = entries();
    assert!(all.len() > 30, "the surface is not empty");
    for e in &all {
        let name = e.name();
        let c = codes(e);
        assert!(!c.is_empty(), "{name}: exit_codes is empty");
        for r in e.json["exit_codes"].as_array().unwrap() {
            assert!(
                r["id"].as_str().is_some_and(|s| !s.is_empty())
                    && r["meaning"].as_str().is_some_and(|s| !s.is_empty()),
                "{name}: a row without id or meaning: {r}"
            );
        }
        for code in &c {
            assert!(
                ALLOWED.contains(code),
                "{name}: {code} is not in the contract"
            );
        }
        assert!(
            c.contains(&0) && c.contains(&2) && c.contains(&141),
            "{name}: {c:?}"
        );
        let mut sorted = c.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(c, sorted, "{name}: in code order, once each");
    }
}

#[test]
fn a_child_codes_are_marked_on_run_and_cloud_run_only() {
    for e in entries() {
        let marked = e.json["child_exit_code"] == true;
        let child = matches!(e.name().as_str(), "run" | "cloud run" | "cloud");
        assert_eq!(marked, child, "{}", e.name());
    }
}

/// A `--yes` means the command can be declined (4), and 4 means a `--yes` to
/// ask for, so a command cannot gain a prompt without the table knowing.
#[test]
fn a_declined_goes_with_yes() {
    for e in entries() {
        let name = e.name();
        let has_4 = codes(&e).contains(&4);
        if has_yes(&e) && !YES_WITHOUT_A_QUESTION.contains(&name.as_str()) {
            assert!(has_4, "{name} takes --yes but cannot exit 4");
        }
        let leaf = e.json["subcommand_required"] == false
            && !entries()
                .iter()
                .any(|o| o.path.len() == e.path.len() + 1 && o.path[..e.path.len()] == e.path[..]);
        if leaf && has_4 && !FOUR_WITHOUT_YES.contains(&name.as_str()) {
            assert!(has_yes(&e), "{name} can exit 4 but has no --yes");
        }
    }
}

/// `restore` alone writes a result aside; only findings commands exit 1.
#[test]
fn a_one_and_five_belong_to_few_commands() {
    for e in entries() {
        let c = codes(&e);
        assert_eq!(c.contains(&5), e.name() == "restore", "{}", e.name());
    }
}

/// A parent that only groups subcommands can produce what any child can.
#[test]
fn a_a_parent_covers_its_children() {
    let all = entries();
    // Only a parent that does nothing alone: `scan` scans, and `scan rules
    // update` asking a question does not make `scan` declinable.
    for e in all.iter().filter(|e| e.json["subcommand_required"] == true) {
        let parent: BTreeSet<u64> = codes(e).into_iter().collect();
        for child in all
            .iter()
            .filter(|o| o.path.len() > e.path.len() && o.path[..e.path.len()] == e.path[..])
        {
            for c in codes(child) {
                assert!(
                    parent.contains(&c),
                    "{} lacks {c}, which {} has",
                    e.name(),
                    child.name()
                );
            }
        }
    }
}

// ── (b) a driven case per (command, code) ────────────────────────────────

struct Case {
    command: &'static str,
    code: i32,
    setup: fn() -> (Project, Vec<&'static str>),
}

fn clean() -> Project {
    Project::new()
        .env("A=1\n")
        .example("A=\n")
        .file(".gitignore", ".env\n")
}

fn case(command: &'static str, code: i32, setup: fn() -> (Project, Vec<&'static str>)) -> Case {
    Case {
        command,
        code,
        setup,
    }
}

const PW: &str = "demo-passphrase-123\n";

fn cases() -> Vec<Case> {
    vec![
        // init
        case("init", 0, || (Project::new(), vec!["init", "--yes"])),
        case("init", 2, || {
            (Project::new(), vec!["init", "--yes", "--with", "nonsense"])
        }),
        case("init", 4, || (Project::new(), vec!["init"])),
        // add
        case("add", 0, || {
            (clean(), vec!["add", "service", "redis", "--yes"])
        }),
        case("add", 2, || {
            (clean(), vec!["add", "service", "nonexistentid", "--yes"])
        }),
        case("add", 4, || (clean(), vec!["add", "service", "redis"])),
        // remove
        case("remove", 0, || (clean(), vec!["remove", "A", "--yes"])),
        case("remove", 1, || (clean(), vec!["remove", "A", "--check"])),
        case("remove", 2, || (clean(), vec!["remove", "ZZZ", "--yes"])),
        case("remove", 4, || (clean(), vec!["remove", "A"])),
        // set
        case("set", 0, || (clean(), vec!["set", "A=2", "--yes"])),
        case("set", 2, || (Project::new(), vec!["set", "A=2"])),
        case("set", 4, || (clean(), vec!["set", "A=2"])),
        // get: 1 is a name that is not there, 4 a secret without --reveal
        case("get", 0, || (clean(), vec!["get", "A"])),
        case("get", 1, || (clean(), vec!["get", "NOPE"])),
        case("get", 2, || (Project::new(), vec!["get", "A"])),
        case("get", 4, || {
            (
                Project::new().env("API_TOKEN=x\n"),
                vec!["get", "API_TOKEN"],
            )
        }),
        // validate
        case("validate", 0, || (clean(), vec!["validate"])),
        case("validate", 1, || {
            (Project::new().env("A=\n").example("A=\n"), vec!["validate"])
        }),
        case("validate", 2, || {
            (clean(), vec!["validate", "--env", "nofile"])
        }),
        // sync
        case("sync", 0, || (clean(), vec!["sync", "--check"])),
        case("sync", 1, || {
            (
                Project::new().env("A=1\nB=2\n").example("A=\n"),
                vec!["sync", "--check"],
            )
        }),
        case("sync", 2, || {
            (
                Project::new().env("my_key=1\nA=1\n").example("A=\n"),
                vec!["sync", "--naming-policy", "error", "--check"],
            )
        }),
        case("sync", 4, || {
            (
                Project::new().env("A=1\n").example("A=\nOLD=\n"),
                vec!["sync", "--prune"],
            )
        }),
        // diff
        case("diff", 0, || (clean(), vec!["diff"])),
        case("diff", 1, || {
            (
                Project::new().env("A=1\nB=2\n").example("A=\n"),
                vec!["diff"],
            )
        }),
        case("diff", 2, || (clean(), vec!["diff", "--example", "nofile"])),
        // scan
        case("scan", 0, || (clean(), vec!["scan"])),
        case("scan", 1, || {
            (
                clean().file("x.txt", "K=sk_live_abcdefghijklmnopqrstuvwx\n"),
                vec!["scan", "x.txt"],
            )
        }),
        case("scan", 2, || (clean(), vec!["scan", "nonexist"])),
        // doctor
        case("doctor", 0, || (clean(), vec!["doctor"])),
        case("doctor", 1, || {
            (Project::new().env("bad line\n"), vec!["doctor"])
        }),
        case("doctor", 2, || (clean(), vec!["doctor", "nofile"])),
        // parse
        case("parse", 0, || (clean(), vec!["parse"])),
        case("parse", 1, || {
            (Project::new().env("A=1\nbad line\n"), vec!["parse"])
        }),
        case("parse", 2, || (clean(), vec!["parse", "--env", "nofile"])),
        // align
        case("align", 0, || (clean(), vec!["align", "--check"])),
        case("align", 1, || {
            (
                Project::new().env("B=2\nA=1\n").example("A=\nB=\n"),
                vec!["align", "--check"],
            )
        }),
        case("align", 2, || (clean(), vec!["align", "--env", "nofile"])),
        // convert (--yes asks nothing that can be declined)
        case("convert", 0, || (clean(), vec!["convert", "--to", "json"])),
        case("convert", 2, || {
            (clean(), vec!["convert", "--to", "json", "--env", "nofile"])
        }),
        // template (the same)
        case("template", 0, || {
            (
                clean().file("t.in", "x={{A}}\n"),
                vec!["template", "--input", "t.in", "--output", "o.txt"],
            )
        }),
        case("template", 2, || {
            (
                clean().file("t.in", "x={{NOPE}}\n"),
                vec!["template", "--input", "t.in", "--output", "o.txt"],
            )
        }),
        // spec check
        case("spec check", 0, || (clean(), vec!["spec", "check"])),
        case("spec check", 1, || {
            (
                clean().file(
                    ".evnx.toml",
                    "[vars.A]\nrequired = true\n[vars.Z]\nrequired = true\n",
                ),
                vec!["spec", "check"],
            )
        }),
        case("spec check", 2, || {
            (
                clean().file(".evnx.toml", "[vars.A]\nrequired = true\n"),
                vec!["spec", "check", "--env", "nofile"],
            )
        }),
        // backup
        case("backup", 0, || {
            (
                clean().file("pw", PW),
                vec!["backup", "--password-file", "pw", "--output", "b.bak"],
            )
        }),
        case("backup", 2, || {
            (
                clean().file("pw", PW),
                vec![
                    "backup",
                    "nofile",
                    "--password-file",
                    "pw",
                    "--output",
                    "b.bak",
                ],
            )
        }),
        // restore
        case("restore", 0, || {
            (
                backed_up("A=1\n"),
                vec![
                    "restore",
                    "b.bak",
                    "--password-file",
                    "pw",
                    "--output",
                    "new.env",
                ],
            )
        }),
        case("restore", 2, || {
            (
                backed_up("A=1\n"),
                vec![
                    "restore",
                    "nofile.bak",
                    "--password-file",
                    "pw",
                    "--output",
                    "new.env",
                ],
            )
        }),
        case("restore", 4, || {
            (
                backed_up("A=1\n").file("new.env", "other\n"),
                vec![
                    "restore",
                    "b.bak",
                    "--password-file",
                    "pw",
                    "--output",
                    "new.env",
                ],
            )
        }),
        case("restore", 5, || {
            (
                backed_up("This is a letter.\nIt has no assignments.\n"),
                vec![
                    "restore",
                    "b.bak",
                    "--password-file",
                    "pw",
                    "--output",
                    "new.env",
                ],
            )
        }),
        // undo
        case("undo", 0, || (kept_copy(), vec!["undo", "--dry-run"])),
        case("undo", 2, || (clean(), vec!["undo"])),
        case("undo", 4, || (kept_copy(), vec!["undo", "--clear"])),
    ]
}

/// A project with `b.bak`, a backup of `content`, and the passphrase file.
fn backed_up(content: &str) -> Project {
    let p = Project::new().file("src.env", content).file("pw", PW);
    if support_has("backup") {
        run(
            p.path(),
            &[
                "backup",
                "src.env",
                "--password-file",
                "pw",
                "--output",
                "b.bak",
            ],
        );
    }
    p
}

/// A project where `remove` kept a copy for `undo` to find.
fn kept_copy() -> Project {
    let p = Project::new().env("A=1\nB=2\n").example("A=\nB=\n");
    run(p.path(), &["remove", "B", "--yes"]).assert_code(0);
    p
}

fn support_has(command: &str) -> bool {
    entries().iter().any(|e| e.name() == command)
}

/// Every code a local command's table lists that a fixture can reach.
const LOCAL: &[&str] = &[
    "init",
    "add",
    "set",
    "get",
    "remove",
    "validate",
    "sync",
    "diff",
    "scan",
    "doctor",
    "parse",
    "align",
    "convert",
    "template",
    "spec check",
    "backup",
    "restore",
    "undo",
];

/// Local codes in the table with no case, and why. `sync` 4 and `init` 4 are
/// driven above; none are left out today.
const NOT_DRIVEN: &[(&str, i32)] = &[];

#[test]
fn b_each_local_command_reaches_each_code_it_documents() {
    let present: BTreeSet<String> = entries().iter().map(|e| e.name()).collect();
    for c in cases() {
        if !present.contains(c.command) {
            continue; // a build without `backup`
        }
        let (project, args) = (c.setup)();
        let out = run(project.path(), &args);
        assert_eq!(
            out.code,
            Some(c.code),
            "evnx {} should exit {}\n--- stdout\n{}\n--- stderr\n{}",
            args.join(" "),
            c.code,
            out.stdout,
            out.stderr
        );
        let table = codes(entries().iter().find(|e| e.name() == c.command).unwrap());
        assert!(
            table.contains(&(c.code as u64)),
            "evnx {} exits {}, which its table does not list: {table:?}",
            c.command,
            c.code
        );
    }
}

#[test]
fn b_every_documented_local_code_has_a_case_or_a_reason() {
    let driven: BTreeSet<(String, i32)> = cases()
        .iter()
        .map(|c| (c.command.to_string(), c.code))
        .collect();
    for e in entries() {
        let name = e.name();
        if !LOCAL.contains(&name.as_str()) {
            continue;
        }
        for code in codes(&e) {
            let code = code as i32;
            if !matches!(code, 0 | 1 | 2 | 4 | 5) {
                continue;
            }
            assert!(
                driven.contains(&(name.clone(), code))
                    || NOT_DRIVEN.contains(&(name.as_str(), code)),
                "evnx {name}: exit {code} is documented and no fixture drives it"
            );
        }
    }
}

/// And the other way: a driven case for a code the table does not list is a
/// table that lies (the first assertion of the test above says it per run).
#[test]
fn b_the_driven_cases_name_real_commands() {
    for c in cases() {
        assert!(LOCAL.contains(&c.command), "{}", c.command);
    }
}

// ── (c) no help names a code the table lacks ─────────────────────────────

/// Numbers the help says an exit code is: the block's `  <code>  ` lines, and
/// "exit 4", "exits 2", "exit code is 14" in prose.
fn mentioned(help: &str) -> BTreeSet<u64> {
    let prose = regex::Regex::new(r"(?i)\bexit(?:s|ed)?(?:\s+code)?(?:\s+is)?\s+(\d+)\b").unwrap();
    let block = regex::Regex::new(r"(?m)^ +(\d+)  \S").unwrap();
    let mut out = BTreeSet::new();
    for re in [&prose, &block] {
        for m in re.captures_iter(help) {
            out.insert(m[1].parse().unwrap());
        }
    }
    out
}

#[test]
fn c_no_help_mentions_a_code_outside_its_table() {
    let dir = Project::new();
    for e in entries() {
        let name = e.name();
        let mut args: Vec<&str> = e.path.iter().map(String::as_str).collect();
        args.push("--help");
        let out = run(dir.path(), &args);
        out.assert_code(0);
        let table: BTreeSet<u64> = codes(&e).into_iter().collect();
        for n in mentioned(&out.stdout) {
            assert!(
                table.contains(&n),
                "evnx {name} --help says exit {n}; its table is {table:?}"
            );
        }
        // The block is there, and says what the table says.
        assert!(
            out.stdout.contains("Exit codes:\n    0  "),
            "evnx {name} --help has no Exit codes block"
        );
        for r in e.json["exit_codes"].as_array().unwrap() {
            let line = format!(
                "\n  {:>3}  {}",
                r["code"].as_u64().unwrap(),
                r["meaning"].as_str().unwrap()
            );
            assert!(
                out.stdout.contains(&line),
                "evnx {name} --help lacks `{}`",
                line.trim()
            );
        }
    }
}

// ── docs/exit-codes.md ───────────────────────────────────────────────────

/// Where `code` applies: the shortest paths such that every command at or
/// under them has it, so `auth` stands for the whole `auth` tree.
fn commands_with(code: u64, all: &[Entry]) -> String {
    let has = |e: &Entry| codes(e).contains(&code);
    if all.iter().all(has) {
        return "every command".to_string();
    }
    let under = |e: &Entry, p: &[String]| e.path.len() >= p.len() && e.path[..p.len()] == *p;
    let mut out: Vec<String> = Vec::new();
    for e in all {
        let covered_above = (1..e.path.len()).any(|n| {
            let p = &e.path[..n];
            all.iter().filter(|o| under(o, p)).all(has)
        });
        if covered_above || !has(e) {
            continue;
        }
        let whole = all.iter().filter(|o| under(o, &e.path)).all(has);
        let has_children = all
            .iter()
            .any(|o| o.path.len() > e.path.len() && under(o, &e.path));
        if whole && has_children {
            out.push(format!("`{}` and its subcommands", e.name()));
        } else if e.json["subcommand_required"] != true {
            out.push(format!("`{}`", e.name()));
        }
    }
    out.join(", ")
}

fn render_doc() -> String {
    let all = entries();
    // The meaning of each code is the same on every command: take it from the
    // first command that lists it.
    let mut rows: Vec<(u64, String, String)> = Vec::new();
    for code in ALLOWED {
        let r = all
            .iter()
            .flat_map(|e| e.json["exit_codes"].as_array().unwrap().iter())
            .find(|r| r["code"] == *code)
            .unwrap_or_else(|| panic!("no command lists {code}"));
        rows.push((
            *code,
            r["id"].as_str().unwrap().into(),
            r["meaning"].as_str().unwrap().into(),
        ));
    }
    let mut s = String::new();
    s.push_str("<!-- Generated by tests/exit_contract.rs from core::exit_table. Do not edit.\n");
    s.push_str(
        "     Rewrite with: UPDATE_GOLDEN=1 cargo test --all-features --test exit_contract -->\n\n",
    );
    s.push_str("# Exit codes\n\n");
    s.push_str("One table for every command. The same lines are in each command's `--help`\n");
    s.push_str("and in `exit_codes` of `evnx commands --json`.\n\n");
    s.push_str("| Code | Id | Meaning | Commands |\n|---|---|---|---|\n");
    for (code, id, meaning) in rows {
        s.push_str(&format!(
            "| {code} | `{id}` | {meaning} | {} |\n",
            commands_with(code, &all)
        ));
    }
    let child: Vec<String> = all
        .iter()
        .filter(|e| e.json["child_exit_code"] == true && e.json["subcommand_required"] != true)
        .map(|e| format!("`{}`", e.name()))
        .collect();
    s.push_str(&format!(
        "\n{}: once the command has started, evnx exits with the command's own code, or 128+n when signal n killed it (the shell's rule).\n",
        child.join(" and ")
    ));
    s.push_str("\nA code means the same everywhere: `2` is never a finding, and `1` never a failure to run.\n");
    s.push_str(
        "\n`get` answers with them too: `1` is a name that is not in the file (also `get --exists`), \
         and `4` a secret read without `--reveal`. `set` exits `4` when it needs a value or \
         an answer and there is no terminal and nothing on stdin.\n",
    );
    s
}

/// The doc covers every command: all features, or the file is not comparable.
#[test]
fn docs_exit_codes_md_is_current() {
    let features = support::surface::payload()["features"].clone();
    let full = ["net", "migrate", "backup", "cloud"]
        .iter()
        .all(|f| features.as_array().unwrap().iter().any(|x| x == f));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/exit-codes.md");
    if !full && std::env::var_os("UPDATE_GOLDEN").is_none() {
        return; // a smaller build lists fewer commands
    }
    let want = render_doc();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        assert!(full, "rewrite docs/exit-codes.md with --all-features: the commands column needs every command");
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "docs/exit-codes.md is stale. Regenerate: UPDATE_GOLDEN=1 cargo test --all-features --test exit_contract\n--- wanted\n{want}"
    );
}
