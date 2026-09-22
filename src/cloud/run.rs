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
use std::process::Command;
use zeroize::Zeroize;

use super::client::Client;
use super::config::CloudConfig;
use super::sync::NONCE_LEN;
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
    verbose: bool,
    command: Vec<String>,
) -> Result<()> {
    use evnx_crypto::{decrypt_vault, vault_aad, EncryptedBlob};

    let (program, args) = command
        .split_first()
        .ok_or_else(|| anyhow!("no command given. Usage: evnx cloud run --vault <V> -- <cmd>"))?;

    let vault_target = super::sync::resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    vault::require_session(&client, &server)?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let version = match version {
        Some(v) => v,
        None => {
            let latest = super::sync::current_version(&client, &vault_ref)?;
            if latest == 0 {
                return Err(anyhow!(
                    "{} has no versions yet. Push one with `evnx cloud push`.",
                    vault_ref.label()
                ));
            }
            latest
        }
    };

    let blob_bytes = client
        .get_bytes(&format!(
            "/api/v1/vaults/{}/versions/{version}/blob",
            vault_ref.id
        ))
        .map_err(|e| anyhow!("{e}"))?;

    if blob_bytes.len() <= NONCE_LEN {
        return Err(anyhow!(
            "the server returned {} bytes, too short to be a blob",
            blob_bytes.len()
        ));
    }
    let (nonce_bytes, ciphertext) = blob_bytes.split_at(NONCE_LEN);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(nonce_bytes);

    // ⚠️ The password is read before the child is spawned, and it is read from
    // *our* stdin. Whatever remains on stdin is inherited by the child, so a
    // command that also reads stdin sees the rest of the stream, not the
    // password line.
    let password = auth::read_password(password_stdin, "Master password")?;
    let master_key = auth::derive_master_key_for_account(&client, &password)?;
    let vault_key = super::sync::unwrap_vault_key(&client, &vault_ref, &master_key)?;

    let aad = vault_aad(&vault_ref.id, version as u32);
    let mut plaintext = decrypt_vault(
        &EncryptedBlob {
            nonce,
            ciphertext: ciphertext.to_vec(),
        },
        &vault_key,
        &aad,
    )
    .map_err(|_| {
        anyhow!(
            "could not decrypt version {version} of {}.\n\
             \x20 Either the master password is wrong, or the server served a different \
             version than it claimed — the version number is authenticated into the \
             ciphertext, so a substituted blob fails here rather than decrypting to \
             stale secrets.",
            vault_ref.label()
        )
    })?;

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
    // than a failure. `migrate` has the same shape and exits 0; this does not.
    if env_vars.is_empty() && before > 0 {
        return Err(anyhow!(
            "--include/--exclude left nothing to inject — all {before} variable(s) were \
             filtered out.\n\
             \x20 Check the patterns: they are case-sensitive globs matched against \
             variable names."
        ));
    }

    if env_vars.is_empty() {
        ui::warning(format!(
            "{} version {version} holds no variables — running anyway",
            vault_ref.label()
        ));
    } else if !skipped.is_empty() {
        // Names only, and on stderr, so it does not pollute a piped stdout.
        eprintln!(
            "  {} {} of {before} variable(s) filtered out",
            "·".cyan(),
            skipped.len()
        );
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

    let status = spawn_with_env(program, args, &env_vars).map_err(|e| {
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

    // ── Exit as the child did ───────────────────────────────────────────────
    //
    // ⚠️ `std::process::exit`, not a returned `Result`. A pipeline branches on
    // the exit code, and mapping every child failure onto evnx's own `1` would
    // lose the distinction between "the deploy script returned 2" and "evnx
    // could not reach the server".
    match status.code() {
        Some(code) => std::process::exit(code),
        // Killed by a signal. There is no code to pass through, and the shell
        // convention of 128+signal is not portable, so report and use 1.
        None => {
            eprintln!("  {} the command was killed by a signal", "!".yellow());
            std::process::exit(1)
        }
    }
}

/// Run `program` with `env_vars` laid over the inherited environment.
///
/// ⚠️ Separated from `run` so the properties that matter can be tested without a
/// server, a vault or a password: that the child receives the variables, that
/// they do **not** appear in its argument vector, and that its exit code comes
/// back intact.
///
/// The parent's environment is inherited rather than cleared. Clearing it would
/// remove `PATH`, and nothing would start.
fn spawn_with_env(
    program: &str,
    args: &[String],
    env_vars: &IndexMap<String, String>,
) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    for (key, value) in env_vars {
        // `env`, never an argument. This one line is the whole reason to prefer
        // this command over `export $(cat .env | xargs)`.
        cmd.env(key, value);
    }
    cmd.status()
}

/// Parse `KEY=value` lines, reusing the parser every other command uses.
fn parse_env(plaintext: &[u8]) -> Result<IndexMap<String, String>> {
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

    // ─── spawn_with_env ─────────────────────────────────────────────────────

    fn vars(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["-c".to_string(), script.to_string()]
    }

    /// The child sees the vault's variables.
    #[test]
    fn the_child_receives_the_variables() {
        let status = spawn_with_env(
            "sh",
            &sh(r#"[ "$INJECTED" = "hello" ]"#),
            &vars(&[("INJECTED", "hello")]),
        )
        .unwrap();
        assert!(status.success(), "the child did not see INJECTED");
    }

    /// ⚠️ The property this command exists for. Secrets go in the environment
    /// block, never the argument vector, so `ps` shows the command and not the
    /// values. Checked by having the child read its own `/proc` entry.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_secret_never_reaches_the_argument_vector() {
        let secret = "sk_live_NEVER_IN_ARGV";

        // Exits 0 only when the secret is absent from its own cmdline *and*
        // present in its own environment — so a bug that dropped the injection
        // entirely cannot pass this.
        let script = format!(
            r#"cmdline=$(tr '\0' ' ' < /proc/self/cmdline)
               case "$cmdline" in *{secret}*) exit 1 ;; esac
               [ "$TOKEN" = "{secret}" ] || exit 2
               exit 0"#
        );

        let status = spawn_with_env("sh", &sh(&script), &vars(&[("TOKEN", secret)])).unwrap();
        match status.code() {
            Some(0) => {}
            Some(1) => panic!("the secret appeared in the child's argument vector"),
            Some(2) => panic!("the secret never reached the child's environment"),
            other => panic!("unexpected status {other:?}"),
        }
    }

    /// A pipeline branches on this, so the child's code must survive intact.
    #[test]
    fn the_childs_exit_code_comes_back_unchanged() {
        for code in [0, 1, 7, 42] {
            let status =
                spawn_with_env("sh", &sh(&format!("exit {code}")), &IndexMap::new()).unwrap();
            assert_eq!(status.code(), Some(code), "exit {code} was not preserved");
        }
    }

    /// Inherited, not cleared — otherwise PATH is gone and nothing runs.
    #[test]
    fn the_parent_environment_is_inherited() {
        std::env::set_var("EVNX_RUN_INHERIT_PROBE", "from-parent");
        let status = spawn_with_env(
            "sh",
            &sh(r#"[ "$EVNX_RUN_INHERIT_PROBE" = "from-parent" ]"#),
            &IndexMap::new(),
        )
        .unwrap();
        std::env::remove_var("EVNX_RUN_INHERIT_PROBE");
        assert!(status.success(), "the parent environment was not inherited");
    }

    /// And the vault wins where both define a name — that is what injection means.
    #[test]
    fn a_vault_variable_overrides_the_inherited_one() {
        std::env::set_var("EVNX_RUN_OVERRIDE_PROBE", "from-parent");
        let status = spawn_with_env(
            "sh",
            &sh(r#"[ "$EVNX_RUN_OVERRIDE_PROBE" = "from-vault" ]"#),
            &vars(&[("EVNX_RUN_OVERRIDE_PROBE", "from-vault")]),
        )
        .unwrap();
        std::env::remove_var("EVNX_RUN_OVERRIDE_PROBE");
        assert!(status.success(), "the inherited value won");
    }

    /// A missing command is an io::ErrorKind::NotFound, which `run` turns into
    /// the "use sh -c" hint rather than a bare OS error.
    #[test]
    fn a_missing_command_is_reported_as_not_found() {
        let err = spawn_with_env("evnx-no-such-binary-xyz", &[], &IndexMap::new()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

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
