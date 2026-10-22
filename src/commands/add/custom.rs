//! Handle `evnx add custom`: your own variables, as arguments or one prompt
//! at a time.
//!
//! # Two forms (I-025, decided 2026-10-10; DQ-A9, 2026-10-14)
//!
//! * **Arguments:** `evnx add custom API_URL=https://… DEBUG=false --yes`, with
//!   `--desc`, `--category`, `--required` and `--optional` applying to every
//!   name given. No
//!   terminal needed, so scripts, CI and agents can use it. `NAME` alone gets a
//!   placeholder; a secret-looking `NAME` alone, in a terminal and without
//!   `--yes`, is asked for without echo instead.
//! * **Prompts:** name, value, and one `More options?` question per variable;
//!   an empty name finishes.
//!
//! ⚠️ It was prompts only, six to eight questions per variable plus a final
//! confirmation, and `--yes` still needed a terminal (I-020), so nothing
//! automated could use it.

use anyhow::{bail, Result};
use dialoguer::{Input, Password, Select};
use std::path::Path;

use super::shared::{aborted, confirm_and_apply, Flow, Mode, Outcome};
use crate::schema::models::{VarCollection, VarMetadata, VarSource};
use crate::utils::prompt::{require_terminal, require_terminal_or, yes_no};
use crate::utils::string::count;
use crate::utils::ui::{info, print_header, print_preview_header};

/// The categories a custom variable can be filed under.
pub const CATEGORIES: &[&str] = &[
    "Application",
    "Security",
    "Database",
    "Authentication",
    "Service",
    "Other",
];

/// What `evnx add custom` was given on the command line.
pub struct Args {
    /// `NAME=value` or `NAME`, in order.
    pub vars: Vec<String>,
    pub desc: Option<String>,
    pub category: Option<String>,
    pub required: bool,
    /// `--optional` (I-036, DEC-VA2). Conflicts with `required` in clap.
    pub optional: bool,
}

/// One custom variable, before it is written.
struct Custom {
    name: String,
    value: String,
    description: Option<String>,
    category: Option<String>,
    required: bool,
    /// Said optional on purpose: the `.env.example` comment says so. Not
    /// required is the default and says nothing.
    optional: bool,
}

pub fn handle(output_path: &Path, args: Args, mode: &Mode) -> Result<Outcome> {
    // Asking (a hidden value, the "More options?" question) only when the run
    // may ask: not under --yes, --dry-run or --json.
    let yes = !mode.interactive();
    let category = args
        .category
        .as_deref()
        .map(canonical_category)
        .transpose()?;
    let customs = if args.vars.is_empty() {
        // With no arguments every variable is typed at a prompt: `--yes`
        // cannot stand in for a terminal here (I-020).
        require_terminal_or(
            "Give the variables as arguments instead:\n  \
             evnx add custom NAME=value OTHER_NAME=value --yes",
        )?;
        match prompt_loop(yes, args.desc, category, args.required, args.optional)? {
            Some(customs) => customs,
            None => return Ok(aborted()),
        }
    } else {
        if mode.interactive() {
            let shown: Vec<&str> = args.vars.iter().map(String::as_str).collect();
            require_terminal(&format!("evnx add custom {} --yes", shown.join(" ")))?;
        }
        from_arguments(
            &args.vars,
            args.desc,
            category,
            args.required,
            args.optional,
            yes,
        )?
    };

    if customs.is_empty() {
        println!("No variables added.");
        return Ok(Outcome::Aborted);
    }
    let outcome = write(output_path, &customs, mode)?;
    if outcome != Outcome::Aborted {
        say_how_to_make_optional(output_path, &customs);
    }
    Ok(outcome)
}

/// `--optional` with no `[vars]` contract to write it into (I-036, DEC-VA2).
///
/// ⚠️ `.env.example` cannot say "optional": every line in it is required to
/// `validate`. The contract can, but a lone `[vars.NAME]` would switch the
/// project to the contract and stop every other template line counting as
/// required — so the advice starts from `evnx spec init`, which declares them
/// all. stderr, and not hidden by `-q`: the flag asked for something this run
/// could not do.
fn say_how_to_make_optional(output_path: &Path, customs: &[Custom]) {
    let optional: Vec<&str> = customs
        .iter()
        .filter(|c| c.optional)
        .map(|c| c.name.as_str())
        .collect();
    if optional.is_empty() || has_contract(output_path) {
        return;
    }
    crate::utils::ui::warning(format!(
        "{} not marked optional: .env.example cannot say it, and there is no [vars] contract",
        optional.join(", ")
    ));
    crate::utils::ui::warning_detail(format!(
        "     →  run `evnx spec init`, then set `required = false` under [vars.{}]",
        if optional.len() == 1 {
            optional[0]
        } else {
            "NAME"
        }
    ));
}

/// Does the `.evnx.toml` governing `dir` declare a `[vars]` contract?
fn has_contract(dir: &Path) -> bool {
    crate::core::config::load(dir)
        .ok()
        .and_then(|l| l.source)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| crate::core::config::has_vars_section(&t))
}

/// The argument form. Every name is checked before anything is asked or
/// written (T-ADD-42).
fn from_arguments(
    vars: &[String],
    desc: Option<String>,
    category: Option<String>,
    required: bool,
    optional: bool,
    yes: bool,
) -> Result<Vec<Custom>> {
    let mut parsed: Vec<(String, Option<String>)> = Vec::new();
    for raw in vars {
        let (name, value) = match raw.split_once('=') {
            Some((n, v)) => (n.trim().to_string(), Some(v.to_string())),
            None => (raw.trim().to_string(), None),
        };
        if !crate::core::parser::is_valid_key(&name) {
            bail!(
                "`{name}` is not a valid variable name.\n\n\
                 Names start with a letter or underscore and contain only letters, \
                 digits and underscores. Nothing was written."
            );
        }
        if parsed.iter().any(|(n, _)| *n == name) {
            bail!("`{name}` is given twice. Nothing was written.");
        }
        parsed.push((name, value));
    }

    // DQ-A9: a secret without a value is asked for, unseen, when someone is
    // there to ask; otherwise it gets the placeholder like any other name.
    let can_ask = !yes && crate::utils::term::interactive();
    let mut customs = Vec::new();
    for (name, value) in parsed {
        let value = match value {
            Some(v) => v,
            None if can_ask && hides_value(&name) => {
                let v = Password::new()
                    .with_prompt(format!(
                        "Value for {name} (hidden; Enter for a placeholder)"
                    ))
                    .allow_empty_password(true)
                    .interact()?;
                if v.is_empty() {
                    placeholder_for(&name)
                } else {
                    v
                }
            }
            // The same placeholder `.env.example` gets, so the two files agree
            // (it was `your_value_here` here and `YOUR_VALUE_HERE` there).
            None => placeholder_for(&name),
        };
        customs.push(Custom {
            name,
            value,
            description: desc.clone(),
            category: category.clone(),
            required,
            optional,
        });
    }
    Ok(customs)
}

/// The prompt form: name, value, and one `More options?` per variable. An
/// empty name finishes. `None` means the user pressed Esc.
fn prompt_loop(
    yes: bool,
    desc: Option<String>,
    category: Option<String>,
    required: bool,
    optional: bool,
) -> Result<Option<Vec<Custom>>> {
    print_header(
        "Custom Variables",
        Some("Add your own environment variables interactively"),
    );
    info("Enter variables one at a time. Empty name to finish.");

    // Options given as flags apply to every variable and are not asked again.
    let flags_given = desc.is_some() || category.is_some() || required || optional;
    let mut customs: Vec<Custom> = Vec::new();
    loop {
        let name: String = Input::new()
            .with_prompt("Variable name (or Enter to finish)")
            .allow_empty(true)
            .interact_text()?;
        if name.trim().is_empty() {
            break;
        }

        // ⚠️ Check before writing. `add` had no validation at all, so typing
        // `NODE_VERSION=22` at this prompt wrote `# TODO: NODE_VERSION=22=22`
        // — a line evnx's own parser refuses to read (issue #10, item 4).
        // Re-prompt rather than abort: the user is mid-loop and has already
        // entered other variables.
        let name = name.trim().to_string();
        if !crate::core::parser::is_valid_key(&name) {
            if name.contains('=') {
                // By far the most likely slip, and the one in the issue. The
                // argument form takes it as it is.
                let (key, _) = name.split_once('=').expect("contains checked");
                crate::utils::ui::warning(format!(
                    "Enter the name on its own — the value is the next prompt.\n                       Did you mean {}?",
                    key.trim()
                ));
            } else {
                crate::utils::ui::warning(format!(
                    "`{name}` is not a valid variable name.\n                       Names start with a letter or underscore and contain only \
                     letters, digits and underscores."
                ));
            }
            continue;
        }

        // ⚠️ Says where the answer goes. It was "Example/placeholder value",
        // true until DEC-U8-1 (I-245) moved values out of `.env.example`: now
        // this is the real value, and it goes into `.env` only.
        let placeholder = placeholder_for(&name);
        // ⚠️ A secret-looking name is read without echo, as the argument form
        // always did (I-247): this was a visible `Input`, so a live key typed
        // here appeared on screen and in any recording of the terminal.
        let value: String = if !yes && hides_value(&name) {
            let v = Password::new()
                .with_prompt(format!(
                    "Value for {name} (hidden; .env only; Enter for {placeholder})"
                ))
                .allow_empty_password(true)
                .interact()?;
            if v.is_empty() {
                placeholder
            } else {
                v
            }
        } else {
            Input::new()
                .with_prompt(format!(
                    "Value for {name} (.env only; .env.example gets {placeholder})"
                ))
                .default(placeholder)
                .interact_text()?
        };

        let mut custom = Custom {
            name,
            value,
            description: desc.clone(),
            category: category.clone(),
            required,
            optional,
        };

        // One question instead of three (I-025): most variables need none of
        // them.
        if !yes && !flags_given {
            let Some(more) = yes_no()
                .with_prompt("More options? (description, category, required)")
                .default(false)
                .interact_opt()?
            else {
                return Ok(None);
            };
            if more {
                let d: String = Input::new()
                    .with_prompt("Description (Enter for none)")
                    .allow_empty(true)
                    .interact_text()?;
                custom.description = Some(d.trim().to_string()).filter(|d| !d.is_empty());

                // "None" first, so Enter alone files nothing (N-18).
                let mut items = vec!["None"];
                items.extend_from_slice(CATEGORIES);
                let Some(idx) = Select::with_theme(&crate::utils::prompt::MENU)
                    .with_prompt("Category")
                    .items(&items)
                    .default(0)
                    .interact_opt()?
                else {
                    return Ok(None);
                };
                custom.category = (idx > 0).then(|| items[idx].to_string());

                let Some(req) = yes_no()
                    .with_prompt("Is this variable required?")
                    .default(false)
                    .interact_opt()?
                else {
                    return Ok(None);
                };
                custom.required = req;
            }
        }
        customs.push(custom);
    }
    Ok(Some(customs))
}

/// Whether `name`'s value is asked for without echo: the shared secret-name
/// rule, at the Strict level (`*_SECRET`, `*_KEY`, `*_TOKEN`, `*_PASSWORD`,
/// `*_PWD`, …), for both forms (I-247, owner decision 2026-10-17).
///
/// ⚠️ One rule. The argument form used spec's `looks_secret` and the prompt
/// loop used none, so `ADMIN_PWD` was typed visibly in both and `SENTRY_DSN`
/// (a public DSN) was hidden in one.
fn hides_value(name: &str) -> bool {
    crate::core::classify::is_secret_name(name, crate::core::classify::Strictness::Strict)
}

/// `--category security` → `Security`; an unknown one is refused with the list.
fn canonical_category(given: &str) -> Result<String> {
    CATEGORIES
        .iter()
        .find(|c| c.eq_ignore_ascii_case(given))
        .map(|c| c.to_string())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown category '{given}'. Use one of: {}",
                CATEGORIES.join(", ")
            )
        })
}

/// Plan, ask, write: the same path as every other subcommand (AD-S), so a
/// name the files already have is skipped and reported, not appended again
/// (I-016).
fn write(output_path: &Path, customs: &[Custom], mode: &Mode) -> Result<Outcome> {
    let mut collected = VarCollection::default();
    for c in customs {
        // ⚠️ Built alongside the text rather than parsed back out of it.
        // `custom` is the one `add` subcommand with no `VarCollection` of its
        // own — it renders straight to lines — and the spec write-back needs
        // the same structure every other subcommand already has.
        collected.vars.insert(
            c.name.clone(),
            VarMetadata {
                // ⚠️ The placeholder, not the value (I-245, DEC-U8-1): this is what
                // goes into `.env.example`, and what the planner compares there,
                // so running the same command twice is not a conflict.
                example_value: placeholder_for(&c.name),
                description: c.description.clone(),
                category: c.category.clone(),
                required: c.required,
                source: VarSource::BlueprintOverride,
            },
        );
    }

    // One renderer, two files: `.env` gets the values, `.env.example` the
    // placeholders (I-245, DEC-U8-1 — every name, not only secret-looking ones).
    let render_with = |v: &VarCollection, values: bool| -> Result<String> {
        let mut content = String::from("\n# ── Custom Variables ──\n");
        for c in customs.iter().filter(|c| v.vars.contains_key(&c.name)) {
            // A categorised entry keeps the blank line above it that it always had.
            if c.category.is_some() {
                content.push('\n');
            }
            let shown = if values {
                c.value.clone()
            } else {
                placeholder_for(&c.name)
            };
            content.push_str(&entry(
                &c.name,
                &shown,
                c.description.as_deref(),
                c.category.as_deref(),
                c.required,
                c.optional,
            ));
            content.push('\n');
        }
        content.push_str(&format!(
            "\n# Added by evnx add custom on {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M")
        ));
        Ok(content)
    };
    let render = |v: &VarCollection| render_with(v, true);
    let render_example = |v: &VarCollection| render_with(v, false);
    let preview = |v: &VarCollection| {
        let mut out = format!("  • {} to add", count(v.vars.len(), "variable"));
        let categories: Vec<&str> = customs
            .iter()
            .filter(|c| v.vars.contains_key(&c.name))
            .filter_map(|c| c.category.as_deref())
            .collect();
        if !categories.is_empty() {
            out.push_str(&format!("\n  • Categories: {}", categories.join(", ")));
        }
        out
    };
    let flow = Flow {
        noun: "custom variable",
        what: String::new(),
        // Checked by the caller: prompts need a terminal, arguments need one
        // only to be asked for a confirmation.
        terminal_example: None,
        heading: &print_preview_header,
        preview: &preview,
        question: &|_, file| format!("Append these variables to {file}?"),
        render: &render,
        example_render: Some(&render_example),
        env_from_values: true,
    };
    confirm_and_apply(output_path, &collected, &flow, mode)
}

/// What `.env.example` holds for `name`: the same placeholder `sync` writes
/// (`YOUR_KEY_HERE`, …), never the value typed (I-245, DEC-U8-1).
fn placeholder_for(name: &str) -> String {
    crate::core::placeholder::generate_placeholder(
        name,
        None,
        &crate::core::placeholder::PlaceholderConfig::default(),
    )
}

/// One variable as it is written: an optional comment line, then `NAME=value`.
///
/// Same shape as `shared::format_var_line`: the marker belongs above the
/// variable, not on a detached line after it. The category goes *inside* the
/// comment line (`# [Security] API key (required)`, or `# [Security]` alone).
///
/// ⚠️ It used to be prefixed to the entry's first line. With no description
/// and not required, that first line is the assignment, so a category turned
/// `CAT=v1` into `# [Application] CAT=v1`: commented out, invisible to every
/// parser, while the run said "Added" (N-17).
fn entry(
    name: &str,
    example: &str,
    description: Option<&str>,
    category: Option<&str>,
    required: bool,
    optional: bool,
) -> String {
    let mut words = Vec::new();
    if let Some(cat) = category {
        words.push(format!("[{cat}]"));
    }
    if let Some(desc) = description {
        words.push(desc.to_string());
    }
    if required {
        words.push("(required)".to_string());
    } else if optional {
        words.push("(optional)".to_string());
    }
    let assignment = crate::core::envfile::line(name, example);
    if words.is_empty() {
        assignment
    } else {
        format!("# {}\n{assignment}", words.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::{entry, hides_value};

    /// I-247: the names that get a hidden prompt, by the Strict rule.
    #[test]
    fn t_i247_hides_value_follows_the_strict_rule() {
        for name in [
            "STRIPE_SECRET_KEY",
            "ADMIN_PWD",
            "DB_PASSWORD",
            "GITHUB_TOKEN",
            "API_KEY",
        ] {
            assert!(hides_value(name), "{name}");
        }
        for name in [
            "SENTRY_DSN",
            "API_URL",
            "DEBUG",
            "TOKEN_EXPIRY_SECONDS",
            "API_KEY_ID",
        ] {
            assert!(!hides_value(name), "{name}");
        }
    }

    #[test]
    fn plain_entry_is_one_assignment() {
        assert_eq!(entry("A", "1", None, None, false, false), "A=1");
    }

    #[test]
    fn description_and_required_go_above() {
        assert_eq!(
            entry("A", "1", Some("Key"), None, true, false),
            "# Key (required)\nA=1"
        );
        assert_eq!(
            entry("A", "1", None, None, true, false),
            "# (required)\nA=1"
        );
    }

    /// I-036, DEC-VA2: `--optional` is said in the comment, like `--required`.
    #[test]
    fn t_i036_optional_is_said_above() {
        assert_eq!(
            entry("A", "1", None, None, false, true),
            "# (optional)\nA=1"
        );
        assert_eq!(
            entry("A", "1", Some("Key"), None, false, true),
            "# Key (optional)\nA=1"
        );
    }

    /// N-17: every combination with a category leaves the assignment active.
    #[test]
    fn a_category_never_comments_out_the_assignment() {
        for (desc, required, comment) in [
            (None, false, "# [Security]"),
            (None, true, "# [Security] (required)"),
            (Some("Key"), false, "# [Security] Key"),
            (Some("Key"), true, "# [Security] Key (required)"),
        ] {
            let text = entry("A", "1", desc, Some("Security"), required, false);
            assert_eq!(text, format!("{comment}\nA=1"));
            assert!(text.lines().any(|l| l == "A=1"), "{text}");
        }
    }
}
