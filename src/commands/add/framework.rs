//! Handle `evnx add framework [--language <lang>] <framework>`

use anyhow::{Context, Result};
use colored::*;
use std::path::Path;

use super::shared::{aborted, append_to_env_files, count, kind_hint, AppendMode, Outcome};
use crate::schema::{component, formatter, loader};
use crate::utils::prompt::{require_terminal, yes_no};
use crate::utils::ui::{info, success};

/// Handle framework addition.
///
/// One id space (I-017, I-018; decided 2026-10-10): framework ids are unique
/// across languages, so `--language` is optional and only checked when given,
/// and an id from another group is added with a one-line hint, as `init --with`
/// would add it.
pub fn handle(
    language_id: Option<&str>,
    framework_id: &str,
    output_path: &Path,
    yes: bool,
    verbose: bool,
) -> Result<Outcome> {
    let schema = loader::schema()?;

    // 1. A language that was given must exist, whatever the id.
    if let Some(lang) = language_id {
        if !schema.languages.contains_key(lang) {
            anyhow::bail!(
                "Unknown language: '{}'. Available: {}",
                lang,
                schema
                    .languages
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    // 2. Find the id anywhere in the catalogue. An alias (`express`, `next`)
    // names the same component as its id.
    let framework_id = component::canonical(framework_id);
    let Some(found) = component::find(framework_id)? else {
        if let Some(lang) = language_id {
            let language = &schema.languages[lang];
            anyhow::bail!(
                "Unknown framework '{}' for language '{}'. Available: {}",
                framework_id,
                lang,
                language
                    .frameworks
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        // The shared message: near matches and a pointer to the full list.
        component::resolve(&[framework_id.to_string()])?;
        unreachable!("resolve refuses an id that find() did not know");
    };

    // 3. Not a framework: add it as `add service` would, and say what it is.
    if found.kind != component::Kind::Framework {
        if found.kind == component::Kind::Service {
            info(kind_hint(&found));
        }
        let path = output_path.to_string_lossy();
        return super::service::handle(framework_id, &path, yes, verbose);
    }

    // 4. A framework. `--language`, when given, has to be its language; say
    // which one it is rather than only listing the wrong language's (I-017b).
    if let Some(given) = language_id {
        if given != found.group {
            anyhow::bail!(
                "Unknown framework '{framework_id}' for language '{given}'.\n\n\
                 {framework_id} is a {lang} framework: use --language {lang}, \
                 or leave --language out.",
                lang = found.group
            );
        }
    }
    let language_id = found.group.as_str();
    let framework = schema.languages[language_id]
        .frameworks
        .get(framework_id)
        .context("the catalogue lists a framework its language does not have")?;

    if verbose {
        println!(
            "{} Resolving framework: {}/{}",
            "[DEBUG]".dimmed(),
            language_id,
            framework_id
        );
    }

    // ⚠️ Through the shared catalogue, like `add service` and `init --with`.
    // Framework ids are globally unique, so `--language` is now redundant for
    // resolution — it is still validated above, which keeps the existing error
    // for a framework that belongs to a different language.
    let vars = component::resolve(&[framework_id.to_string()])?;

    // 4–5. Preview and confirm, unless --yes. `--yes` never previews (DQ-A6):
    // it used to here and in `blueprint` but not in `service`, and `-q` did
    // not hide it (I-019, N-15).
    if !yes {
        require_terminal(&format!(
            "evnx add framework --language {language_id} {framework_id} --yes"
        ))?;
        println!("\n{}", "Preview:".bold());
        println!("{}", formatter::generate_preview(&vars).dimmed());

        let confirm = yes_no()
            .with_prompt(format!(
                "Add these {} for {} to .env.example?",
                count(vars.vars.len(), "variable"),
                framework.display_name.as_deref().unwrap_or(framework_id)
            ))
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            return Ok(aborted());
        }
    }

    // 6. Format as addition with framework header
    let addition = formatter::format_addition(&vars)?;
    let header = format!(
        "\n# ── Framework: {} ──\n",
        framework.display_name.as_deref().unwrap_or(framework_id)
    );
    let content = format!("{}{}", header, addition);

    // 7. Append to files
    append_to_env_files(
        output_path,
        &content,
        AppendMode::WithConflictWarning,
        verbose,
        &vars,
    )?;

    success(format!(
        "Added {} for {}",
        count(vars.vars.len(), "variable"),
        framework.display_name.as_deref().unwrap_or(framework_id)
    ));

    Ok(Outcome::Written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_handle_appends_framework_vars() {
        let dir = TempDir::new().unwrap();
        let example_path = dir.path().join(".env.example");

        // Create initial file
        fs::write(&example_path, "# Existing\nAPP_NAME=test\n").unwrap();

        // Handle framework addition
        handle(Some("python"), "django", dir.path(), true, false).unwrap();

        // Verify append (not overwrite)
        let content = fs::read_to_string(&example_path).unwrap();
        assert!(content.contains("APP_NAME=test")); // Original preserved
        assert!(content.contains("SECRET_KEY=")); // Django var added
        assert!(content.contains("Framework: Django")); // Section header
    }
}
