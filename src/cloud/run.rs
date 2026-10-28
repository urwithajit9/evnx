//! `evnx cloud run` — inject a vault into a subprocess, with no file on disk.
//!
//! ```bash
//! evnx cloud run --vault myapp/production -- ./deploy.sh
//! ```
//!
//! # What this removes, and what it does not
//!
//! It removes the **plaintext file**. The usual alternatives all create one, or
//! something worse:
//!
//! ```text
//! evnx cloud pull && ./deploy.sh && rm .env     # a file, and an `rm` that may not run
//! export $(cat .env | xargs) && ./deploy.sh     # the shell's environment, and its history
//! ```
//!
//! ⚠️ **It does not remove the master-password requirement.** The vault key is
//! wrapped under your master key, so decrypting needs the password wherever this
//! runs. An API token authenticates to the server; it cannot unwrap a vault key,
//! because the server never had one to give. In CI that means `--password-stdin`
//! fed from a secret, and the honest description of the win is "no file", not
//! "no secret to manage".
//!
//! # The property that makes it worth having
//!
//! Secrets reach the child through its **environment block**, never through its
//! argument vector. So `ps` shows the command and not the values — which is the
//! difference between this and every `export`-based workaround. There is a test
//! that checks exactly that by reading `/proc`.

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use indexmap::IndexMap;

use zeroize::Zeroize;

use super::client::Client;
use super::config::CloudConfig;
use super::{auth, vault};
use crate::utils::ui;

/// Decrypt a vault and run `command` with its variables in the environment.
///
/// Never returns on success: it exits with the child's own status, so
/// `evnx cloud run -- false` fails a pipeline the way `false` would.
#[allow(clippy::too_many_arguments)]
pub fn run(
    server_override: Option<&str>,
    vault_target: Option<String>,
    version: Option<i32>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    password_stdin: bool,
    allow_unsafe_names: bool,
    mask: bool,
    verbose: bool,
    command: Vec<String>,
) -> Result<()> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| anyhow!("no command given. Usage: evnx cloud run --vault <V> -- <cmd>"))?;

    let vault_target = super::sync::resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_credential()?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let version = match version {
        Some(v) => v,
        None => {
            let latest = super::sync::current_version(&client, &vault_ref)?;
            if latest == 0 {
                return Err(anyhow!(
                    "{} has no versions yet. Push one with `evnx cloud push --vault {}`.",
                    vault_ref.label(),
                    vault_ref.label()
                ));
            }
            latest
        }
    };

    // Fetched before the password is asked for, as `cloud pull` does: a version
    // that does not exist answers 404 rather than wasting the typing.
    let blob = super::sync::fetch_blob(&client, &vault_ref, version)?;

    // ⚠️ The password is read before the child is spawned, and it is read from
    // *our* stdin. Whatever remains on stdin is inherited by the child, so a
    // command that also reads stdin sees the rest of the stream, not the
    // password line.
    let password = auth::read_password(password_stdin, "Master password")?;
    let unlocked = auth::unlock(&client, &password)?;
    let vault_key = super::sync::unwrap_vault_key(&client, &vault_ref, &unlocked)?;
    let mut plaintext = super::sync::decrypt_version(&blob, &vault_ref, version, &vault_key)?;

    // ── Parse, then destroy the whole-file plaintext ────────────────────────
    //
    // ⚠️ Order matters, and this is the step that is easy to leave out. The
    // decrypted blob is the entire file in one buffer; the parsed map is the
    // same secrets in smaller pieces. Holding both for the child's lifetime
    // doubles the window for no benefit, so the buffer dies here — before the
    // process is even spawned.
    let mut env_vars = parse_env(&plaintext)?;
    plaintext.zeroize();

    // ── Narrow to what the child actually needs ─────────────────────────────
    //
    // ⚠️ Filtered here, *after* the buffer is gone and *before* anything is
    // handed over. A variable excluded by these flags is never written into the
    // child's environment at all — it is not passed and then unset.
    let before = env_vars.len();
    let mut skipped: Vec<String> = Vec::new();
    env_vars.retain(|name, _| {
        let keep = crate::core::glob::admits(name, include.as_deref(), exclude.as_deref());
        if !keep {
            skipped.push(name.clone());
        }
        keep
    });

    // ⚠️ A filter that matches nothing is almost always a typo, and silently
    // running a deploy with zero secrets is the kind of success that is worse
    // than a failure. `migrate` has the same shape and exits 0; this does not, and
    // neither does `convert` (one wording, `core::glob::nothing_left`).
    if env_vars.is_empty() && before > 0 {
        return Err(anyhow!(crate::core::glob::nothing_left(before, "inject")));
    }

    if env_vars.is_empty() {
        ui::warning(format!(
            "{} version {version} holds no variables — running anyway",
            vault_ref.label()
        ));
    } else if !skipped.is_empty() {
        // Names only, and on stderr, so it does not pollute a piped stdout.
        if !ui::is_quiet() {
            eprintln!(
                "  {} {} of {before} variable(s) filtered out",
                "·".cyan(),
                skipped.len()
            );
        }
    }

    if verbose {
        // Names only. Printing a value here would put it in the terminal
        // scrollback, which is most of what this command exists to avoid.
        eprintln!(
            "  injecting {} variable(s) from {} v{version}: {}",
            env_vars.len(),
            vault_ref.label(),
            env_vars.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }

    // Before the child exists: a refused name means nothing runs (I-228).
    crate::core::inject::check_names(
        &env_vars,
        allow_unsafe_names,
        "the vault",
        "push to this vault",
    )?;

    let masker = mask
        .then(|| crate::core::inject::Masker::new(&env_vars))
        .flatten();
    let status = crate::core::inject::spawn_with_env(program, args, &env_vars, masker.as_ref())
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!(
                    "no such command: {program}\n\
                 \x20 `evnx cloud run` runs the command after `--`; it does not go \
                 through a shell, so builtins and pipelines need one:\n\
                 \x20   evnx cloud run --vault {} -- sh -c 'a | b'",
                    vault_ref.label()
                )
            } else {
                anyhow::Error::new(e).context(format!("failed to run {program}"))
            }
        });

    // ⚠️ Zeroized whether the child succeeded, failed, or never started.
    //
    // This does not reach the copy the kernel made in the child's environment
    // block — that copy is the point, and it dies with the child. What it clears
    // is *our* remaining copy, so a later crash dump of this process does not
    // carry the vault.
    for (_, value) in env_vars.iter_mut() {
        value.zeroize();
    }
    env_vars.clear();

    let status = status?;

    // Exit as the child did: a pipeline branches on the child's own code.
    crate::core::inject::exit_as(status)
}

/// Parse `KEY=value` lines, reusing the parser every other command uses.
///
/// `pub(crate)` so `cloud export` parses a pulled version exactly the way `cloud
/// run` does. Two copies of "bytes to variables" inside one module is how `scan`
/// came to miss a secret in a multiline value (D21) — one parser, one answer.
pub(crate) fn parse_env(plaintext: &[u8]) -> Result<IndexMap<String, String>> {
    let text = std::str::from_utf8(plaintext)
        .context("the vault's contents are not UTF-8, so they cannot be parsed into variables")?;
    crate::core::parser::Parser::new(Default::default())
        .parse_content(text)
        .map_err(|e| anyhow!("could not parse the vault's contents: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_reuses_the_ordinary_env_parser() {
        let vars = parse_env(b"A=1\n# comment\nB=two words\n").unwrap();
        assert_eq!(vars.get("A").map(String::as_str), Some("1"));
        assert_eq!(vars.get("B").map(String::as_str), Some("two words"));
        assert_eq!(vars.len(), 2, "comments must not become variables");
    }

    #[test]
    fn a_vault_that_is_not_utf8_says_so_rather_than_panicking() {
        let err = parse_env(&[0xff, 0xfe, 0x00]).unwrap_err();
        assert!(err.to_string().contains("not UTF-8"), "{err}");
    }

    #[test]
    fn an_empty_vault_parses_to_nothing() {
        assert!(parse_env(b"").unwrap().is_empty());
        assert!(parse_env(b"# only a comment\n").unwrap().is_empty());
    }

    fn vars(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["-c".to_string(), script.to_string()]
    }

    use crate::core::inject::spawn_with_env;

    // ─── --include / --exclude, through the real injection ──────────────────
    //
    // The filtering itself is `core::glob`'s, tested there. These check the
    // thing that matters here: a filtered-out variable never reaches the child.

    fn filtered(
        all: &[(&str, &str)],
        include: Option<&[String]>,
        exclude: Option<&[String]>,
    ) -> IndexMap<String, String> {
        let mut m = vars(all);
        m.retain(|name, _| crate::core::glob::admits(name, include, exclude));
        m
    }

    fn pats(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_excluded_variable_is_absent_from_the_child() {
        let inc = pats(&["DB_*"]);
        let env = filtered(
            &[("DB_URL", "kept"), ("STRIPE_KEY", "sk_live_dropped")],
            Some(&inc),
            None,
        );

        // Exits 0 only when DB_URL is present and STRIPE_KEY is genuinely unset
        // — not merely empty, which a naive implementation would leave behind.
        let status = spawn_with_env(
            "sh",
            &sh(r#"[ "$DB_URL" = "kept" ] || exit 2
                   [ -z "${STRIPE_KEY+set}" ] || exit 3
                   exit 0"#),
            &env,
            None,
        )
        .unwrap();
        match status.code() {
            Some(0) => {}
            Some(2) => panic!("the included variable did not reach the child"),
            Some(3) => panic!("the excluded variable was still passed to the child"),
            other => panic!("unexpected status {other:?}"),
        }
    }

    #[test]
    fn exclude_wins_over_include_at_the_injection_point() {
        let inc = pats(&["APP_*"]);
        let exc = pats(&["*_LOCAL"]);
        let env = filtered(
            &[("APP_NAME", "kept"), ("APP_DB_LOCAL", "dropped")],
            Some(&inc),
            Some(&exc),
        );
        assert_eq!(env.len(), 1);
        assert!(env.contains_key("APP_NAME"));
    }

    #[test]
    fn no_filters_injects_everything() {
        let env = filtered(&[("A", "1"), ("B", "2")], None, None);
        assert_eq!(env.len(), 2);
    }
}
