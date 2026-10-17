//! `evnx commands` (alias `surface`), `completions` and `update` at the binary
//! level (R-surface, 0.10.7; review `24-surface.md`).
//!
//! None of the three touches a project's files, so nothing here needs a
//! project; `update` runs from fake install locations ([`support::install`])
//! because its answer depends on where the binary sits. `update --check` runs
//! against a loopback server through `EVNX_UPDATE_URL`, which only a debug
//! build honours (DEC-SF10), so those tests are `debug_assertions` only.

mod support;

use support::install::Install;
use support::{run, Project};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn evnx(args: &[&str]) -> support::Output {
    let p = Project::new();
    run(p.path(), args)
}

fn top_level() -> Vec<support::surface::Entry> {
    support::surface::entries()
        .into_iter()
        .filter(|e| e.path.len() == 1 && e.json["hidden"] == false)
        .collect()
}

// ── commands ────────────────────────────────────────────────────────────────

/// T-SF-01: the plain output names the version, the features line equals the
/// JSON's, and every visible top-level command is listed once.
#[test]
fn t_sf01_plain_output_names_the_build_and_every_command() {
    let out = evnx(&["commands"]);
    out.assert_code(0);
    assert!(
        out.stdout.contains(&format!("  evnx {VERSION}\n")),
        "{}",
        out.stdout
    );
    let features: Vec<String> = support::surface::payload()["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    let line = if features.is_empty() {
        "  built with no optional features\n".to_string()
    } else {
        format!("  built with: {}\n", features.join(", "))
    };
    assert!(out.stdout.contains(&line), "{}", out.stdout);
    for c in top_level() {
        let listed = out
            .stdout
            .lines()
            .filter(|l| l.split_whitespace().next() == Some(c.name().as_str()))
            .count();
        assert_eq!(listed, 1, "`{}` listed {listed} times", c.name());
    }
}

/// T-SF-02: `--json` from real stdout is schema 1, from this version, with no
/// clap `help` entry.
#[test]
fn t_sf02_json_is_schema_1_from_this_version() {
    let json = support::surface::payload();
    assert_eq!(json["schema"], 1);
    assert_eq!(json["evnx_version"], VERSION);
    assert!(support::surface::entries()
        .iter()
        .all(|e| !e.path.iter().any(|p| p == "help")));
}

/// T-SF-03: `--compact` is one line of the same payload, and needs `--json`.
#[test]
fn t_sf03_compact_is_one_line_and_needs_json() {
    let compact = evnx(&["commands", "--json", "--compact"]);
    compact.assert_code(0);
    assert_eq!(compact.stdout.lines().count(), 1);
    let parsed: serde_json::Value = serde_json::from_str(&compact.stdout).unwrap();
    assert_eq!(parsed, support::surface::payload());

    evnx(&["commands", "--compact"])
        .assert_code(2)
        .assert_contains("--compact only applies to --json output");
}

/// T-SF-04, T-SF-05: the `surface` alias answers the same, and two runs are
/// byte-identical (the output is diffed in another repository).
#[test]
fn t_sf04_the_surface_alias_and_a_repeat_run_are_identical() {
    let a = evnx(&["commands", "--json"]);
    let b = evnx(&["surface", "--json"]);
    let c = evnx(&["commands", "--json"]);
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stdout, c.stdout);
    assert_eq!(evnx(&["commands"]).stdout, evnx(&["surface"]).stdout);
}

/// I-196, DEC-SF7: the closing line counted every nested path ("83 commands")
/// under a list of 20. It counts what is listed, and says how many paths
/// there are in all.
#[test]
fn t_i196_the_count_line_counts_what_is_listed() {
    let top = top_level().len();
    let all = support::surface::entries()
        .iter()
        .filter(|e| e.json["hidden"] == false)
        .count();
    evnx(&["commands"]).assert_contains(&format!(
        "\n  {top} commands ({all} with sub-commands). `evnx <command> --help` for any of them.\n"
    ));
}

/// DEC-SF5: `-q` never suppresses an answer. The list and the script are the
/// answer, so `-q` changes neither; both say so in `--help`.
#[test]
fn t_dec_sf5_commands_and_completions_ignore_q() {
    assert_eq!(evnx(&["commands"]).stdout, evnx(&["-q", "commands"]).stdout);
    assert_eq!(
        evnx(&["completions", "bash"]).stdout,
        evnx(&["-q", "completions", "bash"]).stdout
    );
    for cmd in ["commands", "completions"] {
        let help = evnx(&[cmd, "--help"]).stdout;
        assert!(
            help.contains("`-q` has no effect"),
            "`{cmd} --help` does not say what -q does:\n{help}"
        );
    }
}

/// I-185: a closed stdout ends `commands --json` (large enough to fill the
/// pipe) with 141 and no panic.
#[cfg(unix)]
#[test]
fn t_i185_commands_with_a_closed_stdout_exits_141() {
    let p = Project::new();
    let out = support::run::closed_stdout(p.path(), &["commands", "--json"]);
    assert_eq!(out.code, Some(141), "{}", out.stderr);
    assert!(!out.stderr.contains("panicked"), "{}", out.stderr);
}

/// I-195, DEC-SF8: `completions` and `update` had no `docs_url`. Every
/// top-level command has one now, with no exemption list.
#[test]
fn t_i195_every_top_level_command_has_a_docs_url() {
    let missing: Vec<String> = top_level()
        .into_iter()
        .filter(|c| !c.json["docs_url"].is_string())
        .map(|c| c.name())
        .collect();
    assert!(missing.is_empty(), "no docs_url: {missing:?}");
}

/// I-195: and their `--help` ends with the guide, like every other command.
#[test]
fn t_i195_completions_and_update_help_name_their_guide() {
    for cmd in ["completions", "update"] {
        evnx(&[cmd, "--help"]).assert_contains(&format!(
            "Full guide: https://docs.evnx.dev/cli/commands/{cmd}"
        ));
    }
}

/// Arguments that still have no help text, each with the package that fixes it
/// (DEC-SF12). An entry is removed when its help is written; the test fails if
/// an entry here has help, so the list cannot outlive the gap.
const NO_HELP_YET: &[(&str, &str, &str)] = &[
    ("scan", "path", "R-scan"),
    ("scan", "exclude", "R-scan"),
    ("restore", "backup", "R-backup-restore"),
    ("restore", "output", "R-backup-restore"),
    ("org members", "org", "R-org-vault-cloud"),
    ("org invite", "org", "R-org-vault-cloud"),
    ("org invites", "org", "R-org-vault-cloud"),
    ("org uninvite", "org", "R-org-vault-cloud"),
    ("org billing", "org", "R-org-vault-cloud"),
    ("org seats", "org", "R-org-vault-cloud"),
    ("org assign", "org", "R-org-vault-cloud"),
    ("org release", "org", "R-org-vault-cloud"),
    ("org role", "org", "R-org-vault-cloud"),
    ("org remove", "org", "R-org-vault-cloud"),
    ("org delete", "org", "R-org-vault-cloud"),
    ("org leave", "org", "R-org-vault-cloud"),
];

/// T-061, I-194, DEC-SF12: every argument has help text, positionals included
/// (the `--help` test in devrel_review only reads `--flag` lines, so
/// `completions <SHELL>` passed it with none).
#[test]
fn t_dec_sf12_every_argument_has_help_or_is_listed() {
    let mut missing = Vec::new();
    let mut stale = Vec::new();
    for c in support::surface::entries() {
        for a in c.args() {
            let id = a["id"].as_str().unwrap();
            let listed = NO_HELP_YET
                .iter()
                .find(|(path, arg, _)| *path == c.name() && *arg == id);
            match (a["help"].is_string(), listed) {
                (false, None) => missing.push(format!("{} {id}", c.name())),
                (true, Some((_, _, pkg))) => {
                    stale.push(format!("{} {id} (listed for {pkg})", c.name()))
                }
                _ => {}
            }
        }
    }
    assert!(
        missing.is_empty(),
        "arguments with no help text: {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "these have help now; remove them from NO_HELP_YET: {stale:?}"
    );
}

// ── completions ─────────────────────────────────────────────────────────────

/// I-185: `completions` panicked inside clap_complete on a closed stdout
/// ("failed to write completion file", exit 101), which the print hook does
/// not match. It ends with 141 like every other command.
#[cfg(unix)]
#[test]
fn t_i185_completions_with_a_closed_stdout_exits_141() {
    let p = Project::new();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let out = support::run::closed_stdout(p.path(), &["completions", shell]);
        assert_eq!(out.code, Some(141), "{shell}: {}", out.stderr);
        assert!(!out.stderr.contains("panicked"), "{shell}: {}", out.stderr);
    }
}

/// I-194, DEC-SF6, T-SF-20/22/25: the five shells clap_complete knows,
/// elvish included, in any case. Every script knows the commands.
#[test]
fn t_i194_completions_accepts_five_shells_in_any_case() {
    for shell in [
        "bash",
        "zsh",
        "fish",
        "powershell",
        "elvish",
        "Bash",
        "ELVISH",
    ] {
        let out = evnx(&["completions", shell]);
        out.assert_code(0);
        assert!(
            out.stdout.contains("commands") && out.stdout.contains("completions"),
            "{shell}: the script does not know the commands"
        );
        assert!(out.stderr.is_empty(), "{shell}: {}", out.stderr);
    }
    assert!(evnx(&["completions", "bash"]).stdout.contains("surface"));
}

/// I-194, DEC-SF6, T-SF-23: anything else is clap's refusal, exit 2, naming
/// the five values.
#[test]
fn t_i194_completions_refuses_other_shells_with_the_list() {
    let out = evnx(&["completions", "tcsh"]);
    out.assert_code(2);
    for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
        assert!(
            out.stderr.contains(shell),
            "{shell} not named:\n{}",
            out.stderr
        );
    }
    evnx(&["completions"]).assert_code(2);
}

/// I-194, DEC-SF6, D-144, T-SF-24: `--help` lists the shells and how to
/// install the script for each.
#[test]
fn t_i194_completions_help_names_the_shells_and_install_lines() {
    let help = evnx(&["completions", "--help"]).stdout;
    assert!(
        help.contains("[possible values: bash, elvish, fish, powershell, zsh]"),
        "{help}"
    );
    for line in [
        "evnx completions bash > ~/.local/share/bash-completion/completions/evnx",
        "evnx completions zsh > ~/.zfunc/_evnx",
        "evnx completions fish > ~/.config/fish/completions/evnx.fish",
        "evnx completions powershell >> $PROFILE",
        "evnx completions elvish >> ~/.config/elvish/rc.elv",
    ] {
        assert!(help.contains(line), "missing `{line}`:\n{help}");
    }
}

/// T-SF-21, I-194: the bash script parses, completes `evnx sp` to `spec`, and
/// offers the shell names after `evnx completions` (it offered `<SHELL>`).
#[cfg(unix)]
#[test]
fn t_i194_the_bash_script_completes_the_shell_names() {
    if std::process::Command::new("bash")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let p = Project::new();
    std::fs::write(p.join("evnx.bash"), evnx(&["completions", "bash"]).stdout).unwrap();
    std::fs::write(
        p.join("t.sh"),
        r#"source ./evnx.bash
t() { COMP_WORDS=("$@"); COMP_CWORD=$(( ${#COMP_WORDS[@]} - 1 )); COMP_LINE="${COMP_WORDS[*]}"; COMP_POINT=${#COMP_LINE}; COMPREPLY=(); _evnx evnx "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD-1]}"; echo "${COMPREPLY[*]}"; }
t evnx sp
t evnx completions ""
"#,
    )
    .unwrap();
    let syntax = std::process::Command::new("bash")
        .args(["-n", "evnx.bash"])
        .current_dir(p.path())
        .status()
        .unwrap();
    assert!(syntax.success(), "bash -n rejected the script");
    let out = std::process::Command::new("bash")
        .arg("t.sh")
        .current_dir(p.path())
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.first().copied(), Some("spec"), "{out}");
    let shells = lines.get(1).copied().unwrap_or("");
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        assert!(
            shells.split(' ').any(|w| w == shell),
            "`evnx completions <TAB>` does not offer {shell}: {shells:?}"
        );
    }
}

// ── update, offline ─────────────────────────────────────────────────────────

const SCRIPT: &str = "curl -fsSL https://dotenv.space/install.sh | bash";
const PIP: &str = "pip install --upgrade evnx";

/// T-SF-30: each package manager's location names its channel, run from the
/// real binary rather than a string (T-058).
#[test]
fn t_sf30_each_package_manager_location_names_its_channel() {
    for (dir, label) in [
        (".cargo/bin", "cargo"),
        ("opt/homebrew/Cellar/evnx/0.10.6/bin", "Homebrew"),
        ("lib/node_modules/@evnx/cli/bin", "npm"),
        (".venv/bin", "pip"),
        ("scoop/apps/evnx/current", "Scoop"),
    ] {
        let out = Install::at(dir).run(&["update"]);
        out.assert_code(0);
        out.assert_contains(&format!("Installed via {label}. Upgrade with:"));
    }
}

/// I-199, DEC-SF2: the cargo command was `--features cloud --force`, which
/// drops `migrate` and `backup` from an `--all-features` install.
#[test]
fn t_i199_the_cargo_command_keeps_every_feature() {
    Install::at(".cargo/bin")
        .run(&["update"])
        .assert_contains("cargo install evnx --all-features --force")
        .assert_not_contains("--features cloud");
}

/// I-193, DEC-SF3: pip puts the binary in a plain `bin/` (`~/.local/bin` with
/// `--user`, `/usr/local/bin` for the system), which read as the install
/// script. The package record pip writes beside it is the evidence.
#[test]
fn t_i193_a_pip_install_into_a_shared_bin_is_read_as_pip() {
    for (bin, record) in [
        (
            ".local/bin",
            ".local/lib/python3.12/site-packages/evnx-0.10.5.dist-info/RECORD",
        ),
        (
            "usr/local/bin",
            "usr/local/lib/python3.11/dist-packages/evnx-0.10.5.dist-info/RECORD",
        ),
        (
            "Python312/Scripts",
            "Python312/Lib/site-packages/evnx-0.10.5.dist-info/RECORD",
        ),
    ] {
        let out = Install::at(bin).file(record).run(&["update"]);
        out.assert_contains("Installed via pip. Upgrade with:")
            .assert_contains(PIP)
            .assert_not_contains(SCRIPT);
    }
}

/// I-193, DEC-SF3: any virtual environment is pip, whatever its name — only
/// `.venv` was recognised. `pyvenv.cfg` beside `bin/` (or `Scripts\`) marks one.
#[test]
fn t_i193_any_virtual_environment_is_pip() {
    for (bin, cfg) in [
        ("venv/bin", "venv/pyvenv.cfg"),
        ("env/Scripts", "env/pyvenv.cfg"),
    ] {
        Install::at(bin)
            .file(cfg)
            .run(&["update"])
            .assert_contains("Installed via pip. Upgrade with:");
    }
}

/// I-193, DEC-SF3: with no Python evidence, the two folders the install script
/// and pip share get both commands, never a lone guess.
#[test]
fn t_i193_a_shared_bin_without_evidence_names_both_commands() {
    for bin in [".local/bin", "usr/local/bin"] {
        let out = Install::at(bin).run(&["update"]);
        out.assert_code(0)
            .assert_contains(SCRIPT)
            .assert_contains(PIP)
            .assert_not_contains("Installed via the install script.");
    }
}

/// T-SF-33: an unrecognised location lists every channel and guesses none.
#[test]
fn t_sf33_an_unknown_location_lists_every_channel() {
    let out = Install::at("opt/tools").run(&["update"]);
    out.assert_code(0)
        .assert_contains("could not tell how evnx was installed")
        .assert_contains("cargo install evnx --all-features --force")
        .assert_contains(SCRIPT)
        .assert_contains(PIP)
        .assert_contains("brew upgrade evnx");
}

/// DEC-SF5: `update -q` prints only the upgrade command — bare for one
/// channel, and the labelled list (no headings) when evnx cannot narrow it.
#[test]
fn t_dec_sf5_update_q_prints_only_the_upgrade_command() {
    let one = Install::at(".cargo/bin").run(&["-q", "update"]);
    one.assert_code(0);
    assert_eq!(one.stdout, "cargo install evnx --all-features --force\n");
    assert_eq!(one.stderr, "");

    let both = Install::at(".local/bin").run(&["-q", "update"]);
    assert_eq!(both.stdout.lines().count(), 2, "{}", both.stdout);
    assert!(both.stdout.contains(SCRIPT) && both.stdout.contains(PIP));

    let all = Install::at("opt/tools").run(&["-q", "update"]);
    assert_eq!(all.stdout.lines().count(), 7, "{}", all.stdout);
    assert!(!all.stdout.contains("could not tell"), "{}", all.stdout);
}

/// D-140, DEC-SF8: the help said "nine install channels" and named none;
/// seven are detected. It names them.
#[test]
fn t_d140_update_help_names_the_channels_it_detects() {
    let help = evnx(&["update", "--help"]).stdout;
    for name in [
        "cargo",
        "Homebrew",
        "Scoop",
        "winget",
        "npm",
        "pip",
        "the install script",
    ] {
        assert!(help.contains(name), "`update --help` does not name {name}");
    }
    assert!(!help.to_lowercase().contains("nine"), "{help}");
}

/// DEC-SF4: `--json` belongs to `--check`; alone it is a usage error.
#[test]
fn t_dec_sf4_update_json_needs_check() {
    evnx(&["update", "--json"]).assert_code(2);
}

// ── update --check, against a loopback server (debug builds) ────────────────

#[cfg(all(feature = "net", debug_assertions))]
mod check {
    use super::*;

    const NO_PROXY: (&str, &str) = ("NO_PROXY", "127.0.0.1,localhost");

    fn server(status: usize, body: &str) -> (mockito::ServerGuard, String) {
        let mut s = mockito::Server::new();
        s.mock("GET", "/latest")
            .with_status(status)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create();
        let url = format!("{}/latest", s.url());
        (s, url)
    }

    fn release(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/urwithajit9/evnx/releases/tag/{tag}"}}"#
        )
    }

    fn check(url: &str, args: &[&str]) -> support::Output {
        let mut all = vec!["update", "--check"];
        all.extend_from_slice(args);
        Install::at(".cargo/bin").run_with(&all, &[("EVNX_UPDATE_URL", url), NO_PROXY])
    }

    /// I-198, DEC-SF4: exit 0 when this is the latest release.
    #[test]
    fn t_i198_current_exits_0() {
        let (_s, url) = server(200, &release(&format!("v{VERSION}")));
        check(&url, &[])
            .assert_code(0)
            .assert_contains("you are on the latest release");
    }

    /// I-198, DEC-SF4: exit 1 when a newer release exists — the answer a
    /// script asks for — with the channel's command and the release notes.
    #[test]
    fn t_i198_behind_exits_1_with_the_command() {
        let (_s, url) = server(200, &release("v99.0.0"));
        check(&url, &[])
            .assert_code(1)
            .assert_contains("a newer release is available")
            .assert_contains("cargo install evnx --all-features --force")
            .assert_contains("releases/tag/v99.0.0");
    }

    /// I-198, DEC-SF4: a build newer than the latest release is not behind.
    #[test]
    fn t_i198_ahead_exits_0() {
        let (_s, url) = server(200, &release("v0.0.1"));
        check(&url, &[])
            .assert_code(0)
            .assert_contains("newer than the latest release");
    }

    /// I-198, DEC-SF4, S-SF4: `--check --json` is schema 1 on a clean stdout.
    #[test]
    fn t_i198_json_reports_status_channel_and_command() {
        let (_s, url) = server(200, &release("v99.0.0"));
        let out = check(&url, &["--json"]);
        out.assert_code(1);
        let json: serde_json::Value = serde_json::from_str(&out.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}", out.stdout));
        assert_eq!(json["schema"], 1);
        assert_eq!(json["installed"], VERSION);
        assert_eq!(json["latest"], "99.0.0");
        assert_eq!(json["status"], "behind");
        assert_eq!(json["channel"], "cargo");
        assert_eq!(json["command"], "cargo install evnx --all-features --force");
        assert_eq!(
            json["release_url"],
            "https://github.com/urwithajit9/evnx/releases/tag/v99.0.0"
        );

        let (_s, url) = server(200, &release(&format!("v{VERSION}")));
        let current: serde_json::Value =
            serde_json::from_str(&check(&url, &["--json"]).assert_code(0).stdout).unwrap();
        assert_eq!(current["status"], "current");
    }

    /// T-SF-41, T-063: a tag without `v` and a release with no `html_url`
    /// still give an answer.
    #[test]
    fn t_sf41_a_bare_tag_and_no_html_url_still_answer() {
        let (_s, url) = server(200, r#"{"tag_name":"99.0.0"}"#);
        check(&url, &[])
            .assert_code(1)
            .assert_contains("https://github.com/urwithajit9/evnx/releases");
    }

    /// T-SF-42: rate limits, server errors and a body that is not a release
    /// are "could not check": exit 2, no verdict.
    #[test]
    fn t_sf42_failures_are_exit_2() {
        for (status, body, says) in [
            (403, "{}", "rate-limiting"),
            (429, "{}", "rate-limiting"),
            (500, "{}", "HTTP 500"),
            (200, "not json", "reading the release information"),
        ] {
            let (_s, url) = server(status, body);
            check(&url, &[])
                .assert_code(2)
                .assert_contains(says)
                .assert_contains("No verdict");
        }
    }

    /// T-SF-43: nothing listening is exit 2 with one line of reason.
    #[test]
    fn t_sf43_no_server_is_exit_2() {
        check("http://127.0.0.1:9/latest", &[])
            .assert_code(2)
            .assert_contains("could not reach api.github.com");
    }

    /// DEC-SF5: `--check -q` prints only the status line and the command.
    #[test]
    fn t_dec_sf5_check_q_prints_the_status_and_the_command() {
        let (_s, url) = server(200, &release("v99.0.0"));
        let out = check(&url, &["-q"]);
        out.assert_code(1);
        let lines: Vec<&str> = out.stdout.lines().collect();
        assert_eq!(lines.len(), 2, "{}", out.stdout);
        assert!(
            lines[0].contains("a newer release is available"),
            "{}",
            out.stdout
        );
        assert_eq!(lines[1], "cargo install evnx --all-features --force");
        assert_eq!(out.stderr, "");
    }

    /// T-SF-35: plain `update` asks nobody, even with the override set.
    #[test]
    fn t_sf35_update_without_check_makes_no_request() {
        let mut s = mockito::Server::new();
        let m = s.mock("GET", mockito::Matcher::Any).expect(0).create();
        let url = format!("{}/latest", s.url());
        Install::at(".cargo/bin")
            .run_with(&["update"], &[("EVNX_UPDATE_URL", &url), NO_PROXY])
            .assert_code(0);
        m.assert();
    }
}

/// T-SF-46: a build with no network support says so, exit 2.
#[cfg(not(feature = "net"))]
#[test]
fn t_sf46_check_without_network_support_exits_2() {
    evnx(&["update", "--check"])
        .assert_code(2)
        .assert_contains("needs network support");
}

// ── the release gate ────────────────────────────────────────────────────────

/// I-200, DEC-SF8, T-SF-52: `scripts/check-build-features.sh` passes this
/// build when it has every feature, and names what a build without `cloud`
/// is missing — the wheel failure the old `--help` grep was written for.
#[cfg(unix)]
#[test]
fn t_i200_the_release_gate_reads_the_feature_list() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    if Command::new("jq").arg("--version").output().is_err() {
        return;
    }
    let gate = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/scripts/check-build-features.sh"
    );

    let all = ["net", "migrate", "backup", "cloud"];
    let built = support::surface::payload()["features"].clone();
    let this = Command::new(gate)
        .arg(support::evnx_bin())
        .output()
        .unwrap();
    let complete = all
        .iter()
        .all(|f| built.as_array().unwrap().iter().any(|b| b == f));
    assert_eq!(this.status.code(), Some(if complete { 0 } else { 1 }));

    let p = Project::new();
    let stub = p.join("evnx");
    std::fs::write(
        &stub,
        "#!/bin/sh\necho '{\"evnx_version\":\"9.9.9\",\"features\":[\"net\",\"migrate\",\"backup\"]}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new(gate).arg(&stub).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("missing: cloud"), "{stderr}");

    std::fs::write(&stub, "#!/bin/sh\nexit 3\n").unwrap();
    assert_eq!(
        Command::new(gate).arg(&stub).status().unwrap().code(),
        Some(2)
    );
}
