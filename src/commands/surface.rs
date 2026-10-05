//! `evnx surface` — the whole command tree as JSON.
//!
//! # Why this exists
//!
//! Five documentation errors of the same shape have been found by hand, and hand
//! sweeps do not repeat:
//!
//! * `evnx vault rekey` documented before it existed
//! * "arriving in the next release" callouts that went stale the moment 0.7.0 shipped
//! * `commands/auth` missing four subcommands for a whole release
//! * `cloud delete-version` shipping with no row in its own subcommand table
//! * `EVNX_COMMANDS` in evnx-web omitting `spec` entirely
//!
//! Every one of them is "a list of commands, maintained by hand, that drifted
//! from the binary". The fix is not a better sweep — it is to stop keeping a
//! second list. This walks **clap's own `Command` tree**, which is the same
//! structure that parses the arguments, so it cannot disagree with the binary.
//!
//! # ⚠️ The output is specific to THIS build's features
//!
//! `cloud`, `migrate` and `backup` are optional, and the commands they add do not
//! exist in a build without them. A consumer that assumed otherwise would report
//! `evnx org` as a documentation error against a `default = []` build, which is
//! exactly backwards.
//!
//! So `features` is part of the payload and is not optional reading. A
//! documentation check must compare against a surface emitted by a binary built
//! the way the docs assume — which, for evnx.dev, means `--all-features`.
//!
//! # ⚠️ It answers a question users actually have
//!
//! This was hidden when it was written, on the grounds that it was a build
//! interface. That was wrong, and the week it shipped proved it: **PyPI had been
//! publishing wheels with no cloud commands since 0.4.0** — five releases — and
//! nobody noticed, because every check anyone ran read `--version` and stopped.
//!
//! A user holding that wheel had no way to ask the binary what it could do.
//! `--help` lists what this build has, but says nothing about what is *missing*
//! or why, and `which -a evnx` tells you which binary answers, not what is in
//! it.
//!
//! So `evnx commands` is a feature:
//!
//! * **plain output** names the version, the compiled features, and every
//!   command, grouped
//! * ⚠️ **it says when `cloud` is absent**, and what to do about it — the one
//!   case where "my build is missing things" is invisible otherwise
//! * `--json` is the machine payload the documentation pipeline and the app's
//!   command tour consume
//!
//! `surface` remains as an alias, because scripts already call it by that name.
//!
//! ⚠️ The JSON is versioned by `schema` so its shape can change without that
//! being a breaking change to anybody's workflow. The **human** output carries
//! no such promise and is not something to parse.

use crate::cli::Cli;
use anyhow::Result;
use clap::{Command, CommandFactory};
use serde::Serialize;

/// Bumped when the shape below changes in a way a consumer would notice.
///
/// ⚠️ Consumers must check it. A tour generated from schema 1 against a schema 2
/// payload is the drift this command exists to prevent, reintroduced one layer up.
const SCHEMA: u32 = 1;

#[derive(Serialize)]
struct Surface {
    schema: u32,
    /// The crate version of the binary that produced this.
    evnx_version: &'static str,
    /// ⚠️ Which optional features this build has. See the module note.
    features: Vec<&'static str>,
    commands: Vec<CommandNode>,
}

#[derive(Serialize)]
struct CommandNode {
    /// Full path from the root, e.g. `["vault", "share"]`. The root itself is
    /// not emitted — there is nothing useful to say about `evnx` as a word.
    ///
    /// ⚠️ A flat list with paths rather than a nested tree: the question every
    /// consumer actually asks is "does `evnx vault share` resolve?", which is a
    /// set lookup here and a recursive walk in a tree. The tree is recoverable
    /// from the paths; the lookup is not recoverable from a tree without writing
    /// the walk again in every consumer.
    path: Vec<String>,
    name: String,
    about: Option<String>,
    /// Aliases that appear in `--help`.
    visible_aliases: Vec<String>,
    /// Every alias, including the ones `--help` does not show.
    aliases: Vec<String>,
    /// ⚠️ True for commands absent from `--help`, including this one. A
    /// documentation check should skip them; a tour certainly should.
    hidden: bool,
    /// The guide this command's `--help` footer points at, when it has one.
    ///
    /// ⚠️ Read from `docs::ALL` — the **same** strings the binary prints — not
    /// composed from the command name. A URL built as
    /// `docs.evnx.dev/cli/commands/{name}` would be a 404 for any command that
    /// shipped before its guide, and would look correct in review.
    ///
    /// `null` for subcommands (the guide is per top-level command), for hidden
    /// commands, and for the few with a guide but no `CommandDoc` entry.
    docs_url: Option<&'static str>,
    args: Vec<ArgNode>,
}

#[derive(Serialize)]
struct ArgNode {
    /// Clap's internal id. Positionals have no `--long`, so this is the only
    /// stable handle for them.
    id: String,
    long: Option<String>,
    short: Option<char>,
    help: Option<String>,
    value_name: Option<String>,
    required: bool,
    /// False for flags like `--verbose`, true for `--severity high`.
    takes_value: bool,
    /// A positional argument rather than a flag.
    positional: bool,
    /// `--include` and friends, which may be given more than once.
    multiple: bool,
    hidden: bool,
}

fn arg_node(arg: &clap::Arg) -> ArgNode {
    // ⚠️ The ACTION decides whether a value follows, not `num_args`.
    //
    // `get_num_args()` returns `None` on an `Arg` from `Cli::command()` because
    // clap fills it in during `_build()`, which has not run. Defaulting that
    // `None` to 1 — which the first version did — reported every boolean flag
    // as taking a value, so the tour rendered `--exit-zero <EXIT_ZERO>` and told
    // people to pass something that does not exist.
    //
    // Generated documentation that is wrong is worse than hand-written
    // documentation that is wrong, because it looks authoritative.
    let takes_value = match arg.get_action() {
        clap::ArgAction::SetTrue
        | clap::ArgAction::SetFalse
        | clap::ArgAction::Count
        | clap::ArgAction::Help
        | clap::ArgAction::Version => false,
        // `Set`, `Append`, and anything clap adds later. Falling back to
        // `num_args` here keeps a future action with an explicit arity correct.
        _ => arg.get_num_args().map(|n| n.takes_values()).unwrap_or(true),
    };
    ArgNode {
        id: arg.get_id().to_string(),
        long: arg.get_long().map(str::to_string),
        short: arg.get_short(),
        help: arg.get_help().map(|h| h.to_string()),
        // ⚠️ Only when a value is actually taken. Clap derives a value name for
        // boolean flags too, and emitting it is what let the renderer print
        // `--exit-zero <EXIT_ZERO>` even after `takes_value` was fixed.
        value_name: if takes_value {
            arg.get_value_names()
                .and_then(|v| v.first())
                .map(|v| v.to_string())
        } else {
            None
        },
        required: arg.is_required_set(),
        takes_value,
        positional: arg.is_positional(),
        // `takes_values` is about one occurrence; this is about how many times
        // the flag may appear. `--include a --include b` is the case that
        // matters, and it is invisible in `num_args`.
        multiple: matches!(
            arg.get_action(),
            clap::ArgAction::Append | clap::ArgAction::Count
        ),
        hidden: arg.is_hide_set(),
    }
}

/// Depth-first, so a parent always precedes its children in the output. Nothing
/// depends on that, but a human reading the file does.
fn walk(cmd: &Command, prefix: &[String], out: &mut Vec<CommandNode>) {
    for sub in cmd.get_subcommands() {
        // ⚠️ `help` is clap's own generated subcommand, not ours. Emitting it
        // would put `evnx help` in the tour and in any documentation index, as a
        // command nobody wrote and nobody should document.
        if sub.get_name() == "help" {
            continue;
        }

        let mut path = prefix.to_vec();
        path.push(sub.get_name().to_string());

        out.push(CommandNode {
            path: path.clone(),
            name: sub.get_name().to_string(),
            about: sub.get_about().map(|a| a.to_string()),
            visible_aliases: sub.get_visible_aliases().map(str::to_string).collect(),
            aliases: sub.get_all_aliases().map(str::to_string).collect(),
            hidden: sub.is_hide_set(),
            // Only top-level commands have a guide of their own.
            docs_url: if path.len() == 1 {
                crate::docs::url_for(sub.get_name())
            } else {
                None
            },
            args: sub
                .get_arguments()
                // `--help` and `--version` are on every command and say nothing
                // about it. Clap adds them; we did not write them.
                .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
                .map(arg_node)
                .collect(),
        });

        walk(sub, &path, out);
    }
}

/// Which optional features this binary was built with.
///
/// ⚠️ Read with `cfg!`, not from a list someone maintains. The whole point of
/// this command is to stop keeping a second copy of something the compiler
/// already knows.
fn enabled_features() -> Vec<&'static str> {
    let mut f = Vec::new();
    if cfg!(feature = "net") {
        f.push("net");
    }
    if cfg!(feature = "migrate") {
        f.push("migrate");
    }
    if cfg!(feature = "backup") {
        f.push("backup");
    }
    if cfg!(feature = "cloud") {
        f.push("cloud");
    }
    f
}

fn build_surface() -> Surface {
    let cmd = Cli::command();
    let mut commands = Vec::new();
    walk(&cmd, &[], &mut commands);
    Surface {
        schema: SCHEMA,
        evnx_version: env!("CARGO_PKG_VERSION"),
        features: enabled_features(),
        commands,
    }
}

/// Which top-level commands need an account.
///
/// ⚠️ Four names, not a second copy of the command list. It exists only to
/// split the printed output in two, and anything not named here is printed
/// under "on your machine" — so a new cloud command is mis-grouped at worst,
/// never missing.
const NEEDS_ACCOUNT: &[&str] = &["auth", "vault", "org", "cloud"];

fn print_human(s: &Surface) {
    println!();
    println!("  evnx {}", s.evnx_version);

    // ⚠️ The line that would have saved five releases of broken PyPI wheels.
    if s.features.is_empty() {
        println!("  built with no optional features");
    } else {
        println!("  built with: {}", s.features.join(", "));
    }
    println!();

    let roots: Vec<&CommandNode> = s
        .commands
        .iter()
        .filter(|c| c.path.len() == 1 && !c.hidden)
        .collect();
    let width = roots.iter().map(|c| c.name.len()).max().unwrap_or(8);

    let print_group = |title: &str, want_account: bool| {
        let group: Vec<&&CommandNode> = roots
            .iter()
            .filter(|c| NEEDS_ACCOUNT.contains(&c.name.as_str()) == want_account)
            .collect();
        if group.is_empty() {
            return;
        }
        println!("  {title}");
        for c in group {
            println!(
                "    {:width$}  {}",
                c.name,
                c.about.as_deref().unwrap_or(""),
                width = width
            );
        }
        println!();
    };

    print_group("On this machine", false);
    print_group("With an evnx account", true);

    // ⛔ The case that is otherwise invisible. A binary without `cloud` does not
    // merely hide those commands — it never had them, and nothing in `--help`
    // distinguishes "this tool cannot sync" from "I have not found it yet".
    if !s.features.contains(&"cloud") {
        // ⚠️ Printed here rather than through `ui::warning`, which starts at
        // column 0 and would break the two-space indent every other line in
        // this output uses. A warning that looks like a layout bug gets read as
        // one.
        use colored::Colorize;
        println!(
            "  {} This build has NO cloud commands — no auth, vault, cloud or org.",
            "!".yellow().bold()
        );
        println!("    It was compiled without the `cloud` feature. Every prebuilt");
        println!("    binary except PyPI's carries them; `pip install evnx` did not,");
        println!("    up to and including 0.9.0.");
        println!();
        println!("    Reinstall from another channel, or build with:");
        println!("      cargo install evnx --features cloud");
        println!();
    }

    let total = s.commands.iter().filter(|c| !c.hidden).count();
    println!("  {total} commands. `evnx <command> --help` for any of them.");
    println!("  `evnx commands --json` emits the whole tree, for tooling.");
    println!();
    // ⚠️ `docs::BASE_URL`, not a literal. Thirty-three URLs in this binary once
    // pointed at a host that had moved, and kept working via a 301 — so nothing
    // failed and nothing warned. One constant is what stops that recurring.
    println!("  Guides: {}", crate::docs::BASE_URL);
    println!();
}

pub fn run(json: bool, compact: bool) -> Result<()> {
    let surface = build_surface();

    if !json {
        // ⚠️ `--compact` without `--json` is a mistake worth naming rather than
        // ignoring: somebody expects machine output and would get prose.
        if compact {
            anyhow::bail!("--compact only applies to --json output");
        }
        print_human(&surface);
        return Ok(());
    }

    // Pretty by default: the output is committed to another repository and read
    // in diffs, where one line of 40 KB is unreviewable.
    let out = if compact {
        serde_json::to_string(&surface)?
    } else {
        serde_json::to_string_pretty(&surface)?
    };
    println!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> Vec<CommandNode> {
        let cmd = Cli::command();
        let mut out = Vec::new();
        walk(&cmd, &[], &mut out);
        out
    }

    fn paths(c: &[CommandNode]) -> Vec<String> {
        c.iter().map(|n| n.path.join(" ")).collect()
    }

    /// The emitter is only worth having if it agrees with the binary. This is
    /// the assertion that it does: every top-level command clap knows about is
    /// in the output.
    #[test]
    fn every_top_level_command_is_emitted() {
        let cmd = Cli::command();
        let expected: Vec<String> = cmd
            .get_subcommands()
            .filter(|s| s.get_name() != "help")
            .map(|s| s.get_name().to_string())
            .collect();
        let got = paths(&surface());
        for name in expected {
            assert!(
                got.contains(&name),
                "`{name}` is a real subcommand and the surface does not list it"
            );
        }
    }

    /// ⚠️ Clap generates `help` as a subcommand of every command that has
    /// subcommands. Emitting it would put `evnx help`, `evnx vault help` and a
    /// dozen more into the tour and into any documentation index — commands
    /// nobody wrote and nobody should document.
    #[test]
    fn clap_s_own_help_subcommand_is_never_emitted() {
        let s = surface();
        let bad: Vec<&String> = s
            .iter()
            .flat_map(|n| n.path.iter())
            .filter(|p| p.as_str() == "help")
            .collect();
        assert!(bad.is_empty(), "`help` leaked into the surface");
    }

    /// Nested commands must carry their full path, or a consumer cannot tell
    /// `evnx vault share` from a top-level `share`.
    #[test]
    fn nested_commands_carry_their_full_path() {
        let got = paths(&surface());
        assert!(
            got.iter().any(|p| p.contains(' ')),
            "no nested command found at all — the walk is not recursing"
        );
        // ⚠️ `add`, not `template`. `template` has no subcommands — the first
        // version of this test assumed it did and failed, which is the test
        // doing its job on itself.
        assert!(
            got.iter().any(|p| p.starts_with("add ")),
            "expected `add <sub>` in: {got:?}"
        );
    }

    /// ⚠️ `--help` and `--version` are on every command and describe none of
    /// them. Leaving them in would put two meaningless rows on every page of a
    /// generated reference.
    #[test]
    fn clap_s_builtin_flags_are_stripped() {
        for node in surface() {
            for a in &node.args {
                assert!(
                    a.id != "help" && a.id != "version",
                    "`{}` kept clap's builtin --{}",
                    node.path.join(" "),
                    a.id
                );
            }
        }
    }

    /// The feature list is the difference between "this build has no `org`" and
    /// "the docs are wrong". A consumer that cannot see it will conclude the
    /// latter.
    #[test]
    fn the_feature_list_matches_what_was_compiled() {
        let f = enabled_features();
        assert_eq!(f.contains(&"cloud"), cfg!(feature = "cloud"));
        assert_eq!(f.contains(&"backup"), cfg!(feature = "backup"));
        assert_eq!(f.contains(&"migrate"), cfg!(feature = "migrate"));
    }

    /// Under `--all-features` the cloud commands must be present, and `org` is
    /// the newest of them — the one a stale emitter would miss.
    #[cfg(feature = "cloud")]
    #[test]
    fn a_cloud_build_emits_the_cloud_commands() {
        let got = paths(&surface());
        for expected in [
            "cloud",
            "vault",
            "auth",
            "org",
            "org billing",
            "vault share",
        ] {
            assert!(
                got.contains(&expected.to_string()),
                "`{expected}` missing from a cloud build's surface"
            );
        }
    }

    /// ⚠️ A boolean flag must not be reported as taking a value.
    ///
    /// `--exit-zero` is declared `exit_zero: bool`. The first version of this
    /// emitter read `get_num_args()`, which is `None` on an unbuilt `Command`,
    /// defaulted it to 1, and told every consumer that every flag in the CLI
    /// takes an argument.
    #[test]
    fn boolean_flags_take_no_value() {
        let s = surface();
        let scan = s.iter().find(|n| n.path == ["scan"]).unwrap();
        let ez = scan
            .args
            .iter()
            .find(|a| a.long.as_deref() == Some("exit-zero"))
            .expect("`scan --exit-zero` should exist");
        assert!(!ez.takes_value, "--exit-zero is a flag, not a value option");
        assert!(ez.value_name.is_none(), "a flag should have no value name");

        // The positive control: an option that genuinely takes one.
        let sev = scan
            .args
            .iter()
            .find(|a| a.long.as_deref() == Some("severity"))
            .expect("`scan --severity` should exist");
        assert!(sev.takes_value, "--severity takes a value");
        assert_eq!(sev.value_name.as_deref(), Some("LEVEL"));
    }

    /// ⚠️ The documentation URL must be the one the binary prints, not one
    /// composed from the command name. If these ever diverge, the tour links
    /// somewhere `--help` does not — and only one of them is checked.
    #[test]
    fn the_docs_url_is_the_one_the_binary_prints() {
        let s = surface();
        let scan = s.iter().find(|n| n.path == ["scan"]).unwrap();
        assert_eq!(scan.docs_url, Some(crate::docs::SCAN.url));

        // A subcommand has no guide of its own, and must not borrow its
        // parent's — `evnx vault share` is documented inside `commands/vault`,
        // not at `commands/share`.
        if let Some(sub) = s.iter().find(|n| n.path.len() > 1) {
            assert_eq!(
                sub.docs_url,
                None,
                "`{}` claimed a guide of its own",
                sub.path.join(" ")
            );
        }

        // ⚠️ A command with no `CommandDoc` gets `None`, not a URL composed
        // from its name. `completions` is visible and has a guide on the site,
        // but no entry in `docs::ALL` — composing one would be right by luck
        // here and a 404 for the next command that ships before its guide.
        let completions = s.iter().find(|n| n.path == ["completions"]).unwrap();
        assert_eq!(
            completions.docs_url, None,
            "a command absent from docs::ALL must not get an invented URL"
        );

        // And one that IS in the registry carries exactly that string.
        let me = s.iter().find(|n| n.path == ["commands"]).unwrap();
        assert_eq!(me.docs_url, Some(crate::docs::COMMANDS.url));
    }

    /// It describes itself, and is visible — it was hidden when written, and
    /// the week it shipped showed why that was wrong: PyPI had been publishing
    /// cloud-less wheels for five releases and no user could ask the binary
    /// what it had.
    #[test]
    fn the_emitter_describes_itself_and_is_visible() {
        let s = surface();
        let me = s
            .iter()
            .find(|n| n.path == ["commands"])
            .expect("`commands` is missing from its own output");
        assert!(!me.hidden, "`commands` is a feature now, not a build hook");
        assert!(
            me.visible_aliases.iter().any(|a| a == "surface"),
            "`surface` must keep working — tooling calls it that: {:?}",
            me.visible_aliases
        );
    }

    /// Serialising must not panic, and must produce the keys consumers read.
    #[test]
    fn the_payload_serialises_with_the_documented_keys() {
        let cmd = Cli::command();
        let mut commands = Vec::new();
        walk(&cmd, &[], &mut commands);
        let json = serde_json::to_string(&Surface {
            schema: SCHEMA,
            evnx_version: env!("CARGO_PKG_VERSION"),
            features: enabled_features(),
            commands,
        })
        .expect("surface must serialise");
        for key in [
            "\"schema\"",
            "\"evnx_version\"",
            "\"features\"",
            "\"commands\"",
            "\"path\"",
        ] {
            assert!(json.contains(key), "missing {key} in the payload");
        }
    }
}
