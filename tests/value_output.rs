//! No command prints a secret's value, or a recognisable part of it, unless
//! printing values is what the command is for.
//!
//! A canary is generated at run time (random, never a literal in the
//! repository) and planted in a project under secret-looking names: as a plain
//! value, as the password of a connection URL, and as a `?token=` query value.
//! Every command in the tables below is run in a fresh copy of that project;
//! stdout and stderr must hold neither the canary nor its first or last four
//! characters next to an ellipsis (`abcd…`, `…wxyz`, `abcd...`), which is how a
//! masked preview looks.
//!
//! # Commands that print values by design (not covered here)
//!
//! - `convert`: its output is the converted file.
//! - `run`: the values are the child process's environment.
//! - `cloud export`, `cloud pull --env -`: the decrypted vault is the output.
//! - `migrate` without `--dry-run`: the values are uploaded or printed as the
//!   commands that upload them.
//! - `diff --format patch` with `--values --show-values`: the patch carries the
//!   values so that it applies.
//! - `backup` / `restore`: the encrypted file and the restored file.
//!
//! # Known gaps
//!
//! Cases that show part of a value sit in their own `#[ignore]`d functions, so
//! the passing ones run in CI and each gap names what it expects. None today.

mod support;

use support::{run, Output, Project};

/// One command line, run in a fresh project.
struct Case {
    /// What the failure message calls it.
    name: &'static str,
    args: &'static [&'static str],
}

const fn case(name: &'static str, args: &'static [&'static str]) -> Case {
    Case { name, args }
}

/// The values planted in the project. Built from OS randomness on every run.
struct Canary {
    /// 24 characters: long enough that a preview shows a part of it.
    long: String,
    /// 9 characters: the length at which a preview shows most of a value.
    short: String,
}

fn random_alnum(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("OS randomness");
    bytes
        .iter()
        .map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char)
        .collect()
}

impl Canary {
    fn new() -> Canary {
        Canary {
            long: random_alnum(24),
            short: random_alnum(9),
        }
    }

    /// A project holding the canary under names a secret scanner looks at.
    fn project(&self) -> Project {
        let c = &self.long;
        let values = format!(
            "SECRET_KEY={c}\n\
             DB_PASSWORD={c}\n\
             API_TOKEN={c}\n\
             DATABASE_URL=postgres://app:{c}@db.internal:5432/app\n\
             WEBHOOK_URL=https://hooks.example.com/notify?token={c}\n\
             SHORT_SECRET={}\n",
            self.short
        );
        let template = "SECRET_KEY=\nDB_PASSWORD=\nAPI_TOKEN=\nDATABASE_URL=\nWEBHOOK_URL=\n\
                        SHORT_SECRET=\nNEW_SECRET_KEY=\n";
        Project::new()
            .env(&values)
            .example(template)
            .file(".env.staging", &values)
    }

    /// Every planted value that must not appear in output.
    fn secrets(&self) -> Vec<&str> {
        vec![&self.long, &self.short]
    }
}

/// The ways `secret` can show in `text`: whole, or four characters of either
/// end beside an ellipsis. Empty when it does not.
fn shown_in(text: &str, secret: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    if text.contains(secret) {
        found.push("whole value");
    }
    // A short preview shows both ends at once (`ab…yz`), two characters each.
    if secret.len() >= 6 {
        let (head, tail) = (&secret[..2], &secret[secret.len() - 2..]);
        if ["…", "..."]
            .iter()
            .any(|dots| text.contains(&format!("{head}{dots}{tail}")))
        {
            found.push("first and last two characters around an ellipsis");
        }
    }
    if secret.len() >= 8 {
        let (head, tail) = (&secret[..4], &secret[secret.len() - 4..]);
        if ["…", "..."]
            .iter()
            .any(|dots| text.contains(&format!("{head}{dots}")))
        {
            found.push("first four characters before an ellipsis");
        }
        if ["…", "..."]
            .iter()
            .any(|dots| text.contains(&format!("{dots}{tail}")))
        {
            found.push("last four characters after an ellipsis");
        }
    }
    found
}

/// `text` with the canary parts replaced, so a failure shows the shape of the
/// output and not the value.
fn redact(text: &str, secret: &str, label: &str) -> String {
    let (head, tail) = (
        &secret[..4.min(secret.len())],
        &secret[secret.len() - 4.min(secret.len())..],
    );
    text.replace(secret, &format!("<{label}>"))
        .replace(head, &format!("<{label}:head>"))
        .replace(tail, &format!("<{label}:tail>"))
}

/// Run `case` in a fresh project and return each planted value it showed, one line per finding.
fn shown_by_case(canary: &Canary, case: &Case) -> Vec<String> {
    let p = canary.project();
    let out = run(p.path(), case.args);
    check_output(canary, case.name, &out)
}

fn check_output(canary: &Canary, name: &str, out: &Output) -> Vec<String> {
    // Proves the command ran: a usage error (2) would pass any output check.
    assert!(
        matches!(out.code, Some(0) | Some(1)),
        "{name}: did not run (exit {:?})\n{}",
        out.code,
        redact(
            &redact(&out.all(), &canary.long, "long"),
            &canary.short,
            "short"
        )
    );
    // A command that printed nothing proves nothing about what it would show.
    assert!(
        !out.all().trim().is_empty(),
        "{name}: printed nothing, so there is nothing to check"
    );
    let text = out.all();
    let mut found = Vec::new();
    for (label, secret) in [("long", &canary.long), ("short", &canary.short)] {
        for how in shown_in(&text, secret) {
            found.push(format!("{name}: the {label} canary shows as its {how}"));
        }
    }
    found
}

#[track_caller]
fn assert_none_shown(cases: &[Case]) {
    let canary = Canary::new();
    let mut all = Vec::new();
    for c in cases {
        all.extend(shown_by_case(&canary, c));
    }
    assert!(all.is_empty(), "values shown:\n{}", all.join("\n"));
}

// ─── Commands that hold today ────────────────────────────────────────────────

#[test]
fn validate_doctor_sync_parse_remove_and_migrate_print_no_value() {
    assert_none_shown(&[
        case("validate", &["validate"]),
        case("validate --format json", &["validate", "--format", "json"]),
        case(
            "validate --format github",
            &["validate", "--format", "github"],
        ),
        case("validate --strict", &["validate", "--strict"]),
        case(
            "validate --fix --dry-run",
            &["validate", "--fix", "--dry-run"],
        ),
        case("sync --dry-run", &["sync", "--dry-run"]),
        case(
            "sync --dry-run --direction reverse",
            &["sync", "--dry-run", "--direction", "reverse"],
        ),
        case("doctor", &["doctor"]),
        case("doctor --format json", &["doctor", "--format", "json"]),
        case("parse", &["parse"]),
        case("parse --format json", &["parse", "--format", "json"]),
        case("remove --dry-run", &["remove", "API_TOKEN", "--dry-run"]),
        case(
            "remove --all --dry-run",
            &["remove", "API_TOKEN", "--all", "--dry-run"],
        ),
        case(
            "migrate --dry-run",
            &[
                "migrate",
                "--to",
                "doppler",
                "--project",
                "p",
                "--doppler-config",
                "dev",
                "--dry-run",
            ],
        ),
    ]);
}

/// `align` moves the values without reading them; its report names keys and
/// lines. The template lacks `NEW_SECRET_KEY` in `.env`, so every run here has
/// something to say, and the plain run writes.
#[test]
fn align_prints_no_value() {
    assert_none_shown(&[
        case("align --dry-run", &["align", "--dry-run"]),
        case("align --check", &["align", "--check"]),
        case(
            "align --format json --dry-run",
            &["align", "--format", "json", "--dry-run"],
        ),
        case("align", &["align"]),
        case(
            "align --env-name staging --format json",
            &["align", "--env-name", "staging", "--format", "json"],
        ),
    ]);
}

#[test]
fn scan_sarif_and_diff_without_show_values_print_no_value() {
    assert_none_shown(&[
        case("scan . --format sarif", &["scan", ".", "--format", "sarif"]),
        case(
            "diff --values",
            &[
                "diff",
                "--env",
                ".env",
                "--example",
                ".env.example",
                "--values",
            ],
        ),
        case(
            "diff --values --format json",
            &[
                "diff",
                "--env",
                ".env",
                "--example",
                ".env.example",
                "--values",
                "--format",
                "json",
            ],
        ),
        case(
            "diff --against",
            &["diff", "--env", ".env", "--against", "staging"],
        ),
        case(
            "diff --against --show-values",
            &[
                "diff",
                "--env",
                ".env",
                "--against",
                "staging",
                "--show-values",
            ],
        ),
        case(
            "diff --against --show-values --format json",
            &[
                "diff",
                "--env",
                ".env",
                "--against",
                "staging",
                "--show-values",
                "--format",
                "json",
            ],
        ),
    ]);
}

// ─── Previews, URL queries and generated values ──────────────────────────────

#[test]
fn scan_previews_show_no_part_of_a_value() {
    assert_none_shown(&[
        case("scan .", &["scan", "."]),
        case("scan . --format json", &["scan", ".", "--format", "json"]),
    ]);
}

#[test]
fn diff_show_values_masks_a_token_in_a_url_query() {
    assert_none_shown(&[
        case(
            "diff --values --show-values",
            &[
                "diff",
                "--env",
                ".env",
                "--example",
                ".env.example",
                "--values",
                "--show-values",
            ],
        ),
        case(
            "diff --values --show-values --format json",
            &[
                "diff",
                "--env",
                ".env",
                "--example",
                ".env.example",
                "--values",
                "--show-values",
                "--format",
                "json",
            ],
        ),
    ]);
}

/// `--fix` writes the secrets it generates to `.env`; the report must say that
/// it did, by name and length, and not print them. The canary cannot catch
/// this (the generated value is not the canary), so the test reads the value
/// back from the file `--fix` wrote.
#[test]
fn validate_fix_does_not_print_the_secrets_it_generates() {
    let canary = Canary::new();
    let p = Project::new()
        .example("SECRET_KEY=CHANGE_ME\nDB_PASSWORD=changeme\nAPI_TOKEN=\n")
        .env(&format!("API_TOKEN={}\n", canary.long));
    let out = run(p.path(), &["validate", "--fix"]);
    assert!(
        matches!(out.code, Some(0) | Some(1)),
        "validate --fix did not run (exit {:?})",
        out.code
    );
    let written = p.read(".env");
    let generated: Vec<String> = ["SECRET_KEY", "DB_PASSWORD"]
        .iter()
        .filter_map(|name| {
            written
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{name}=")))
                .map(str::to_string)
        })
        .collect();
    assert_eq!(generated.len(), 2, "--fix generated nothing to check");

    let text = out.all();
    let mut found = Vec::new();
    for secret in canary
        .secrets()
        .into_iter()
        .chain(generated.iter().map(String::as_str))
    {
        for how in shown_in(&text, secret) {
            found.push(format!("validate --fix: a value shows as its {how}"));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// The one branch that shows characters: a built-in format rule, 20+
/// characters, a known prefix. Built at run time from random characters.
#[test]
fn a_preview_shows_a_known_prefix_and_two_characters_and_nothing_else() {
    use evnx::commands::scan::preview_value;
    let key = format!("{}{}", "AKIA", random_alnum(16).to_uppercase());
    let shown = preview_value("aws-access-key", &key);
    assert_eq!(shown, format!("AKIA…{}", &key[key.len() - 2..]));
    assert!(!shown.contains(&key[4..key.len() - 2]));

    let short = random_alnum(9);
    assert_eq!(preview_value("aws-access-key", &short), "<9 chars>");
    assert_eq!(preview_value("high-entropy-string", &key), "<20 chars>");
}
