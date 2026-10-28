//! Running a command with variables in its environment: `evnx run` and
//! `evnx cloud run` (RM-43). Moved here from `cloud::run` so the local command
//! works in a build without the `cloud` feature.
//!
//! Secrets travel in the child's **environment block**, never its argument
//! vector, so `ps` shows the command and not the values. With a [`Masker`],
//! the child's output passes through it and secret values come out as `***`,
//! the way GitHub Actions masks secrets in a log.
//!
//! ⚠️ Masking prevents accidents (`echo $KEY`, a stack trace that prints its
//! config), not exfiltration: a child that transforms a value (base64, split
//! it) gets past it.

use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// What a masked secret is printed as.
pub const MASK: &str = "***";

/// Whether to mask: `--mask` and `--no-mask` decide; otherwise masked when
/// stdout is not a terminal (a CI log, an agent), where printed output is
/// kept (owner, 2026-10-28).
pub fn masking(mask: bool, no_mask: bool) -> bool {
    use std::io::IsTerminal as _;
    if mask {
        true
    } else if no_mask {
        false
    } else {
        !std::io::stdout().is_terminal()
    }
}

/// Replaces secret values in a byte stream, across read boundaries.
pub struct Masker {
    /// Longest first, so a secret containing another is masked whole.
    secrets: Vec<Vec<u8>>,
}

impl Masker {
    /// The values that hold a secret by the shared classifier, each line of a
    /// multi-line value on its own; short values (under 6 bytes) are left
    /// alone, or every `1` and `true` would become `***`. `None` when there
    /// is nothing to mask.
    pub fn new(vars: &IndexMap<String, String>) -> Option<Self> {
        let mut secrets: Vec<Vec<u8>> = vars
            .iter()
            .filter(|(k, v)| crate::core::classify::holds_secret(k, v))
            .flat_map(|(_, v)| v.lines().map(|l| l.trim().as_bytes().to_vec()))
            .filter(|s| s.len() >= 6)
            .collect();
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        secrets.dedup();
        (!secrets.is_empty()).then_some(Self { secrets })
    }

    fn mask(&self, buf: &mut Vec<u8>) {
        for secret in &self.secrets {
            let mut at = 0;
            while let Some(i) = find(&buf[at..], secret) {
                let i = at + i;
                buf.splice(i..i + secret.len(), MASK.bytes());
                at = i + MASK.len();
            }
        }
    }

    /// How many trailing bytes of `buf` could start a secret, and so must
    /// wait for the next read. Usually zero, so output is not held back.
    fn pending_tail(&self, buf: &[u8]) -> usize {
        let longest = self.secrets.first().map_or(0, Vec::len);
        (1..longest.min(buf.len() + 1))
            .rev()
            .find(|&k| {
                let tail = &buf[buf.len() - k..];
                self.secrets.iter().any(|s| s.starts_with(tail))
            })
            .unwrap_or(0)
    }

    /// Copy `from` to `to`, masked. Errors writing (a closed pipe) end the
    /// copy quietly; the child's exit status is what the caller reports.
    pub fn copy(&self, mut from: impl Read, mut to: impl Write) {
        let mut pending: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = match from.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            pending.extend_from_slice(&chunk[..n]);
            self.mask(&mut pending);
            let emit = pending.len() - self.pending_tail(&pending);
            if to
                .write_all(&pending[..emit])
                .and_then(|()| to.flush())
                .is_err()
            {
                return;
            }
            pending.drain(..emit);
        }
        self.mask(&mut pending);
        let _ = to.write_all(&pending).and_then(|()| to.flush());
    }
}

impl Drop for Masker {
    fn drop(&mut self) {
        for s in &mut self.secrets {
            zeroize::Zeroize::zeroize(s);
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Exit as the child did: its code, or 1 with a note when a signal killed it
/// (128+signal is not portable).
pub fn exit_as(status: std::process::ExitStatus) -> ! {
    match status.code() {
        Some(code) => std::process::exit(code),
        None => {
            eprintln!("  ! the command was killed by a signal");
            std::process::exit(1)
        }
    }
}

/// Refuse vault variables that would make the child load or run other code
/// (`PATH`, `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `BASH_ENV`, …), unless `allow`.
///
/// ⚠️ Any name from the vault was injected, so anyone who can push to a vault
/// could set `LD_PRELOAD` and run code inside every reader's `cloud run`
/// (I-228, DEC-CL1). Checked before the child starts: nothing runs.
///
/// `source` names where the variables came from, and `who` who can change
/// it: `("the vault", "push to this vault")` or `(".env", "edit .env")`.
pub fn check_names(
    env_vars: &IndexMap<String, String>,
    allow: bool,
    source: &str,
    who: &str,
) -> Result<()> {
    if allow {
        return Ok(());
    }
    let unsafe_names: Vec<&str> = env_vars
        .keys()
        .map(String::as_str)
        .filter(|k| crate::core::envsource::is_unsafe_name(k))
        .collect();
    if unsafe_names.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "{source} sets {}, which make a program load or run other code.\n\
         \x20 Anyone who can {who} could use them to run code here.\n\
         \x20 Nothing was run. Exclude them with --exclude, or pass\n\
         \x20 --allow-unsafe-names if you meant to set them.",
        unsafe_names.join(", ")
    ))
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
pub fn spawn_with_env(
    program: &str,
    args: &[String],
    env_vars: &IndexMap<String, String>,
    mask: Option<&Masker>,
) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    // ⚠️ evnx's own settings stay with evnx (I-227, DEC-CL1). `EVNX_TOKEN`,
    // `EVNX_PASSWORD` and `EVNX_SERVER` were inherited, so every child of a CI
    // step held the pipeline's API token.
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(crate::core::envsource::is_evnx_own)
        {
            cmd.env_remove(&key);
        }
    }
    for (key, value) in env_vars {
        // `env`, never an argument. This one line is the whole reason to prefer
        // this command over `export $(cat .env | xargs)`.
        cmd.env(key, value);
    }
    let Some(masker) = mask else {
        return cmd.status();
    };
    // RM-43: the child's output passes through the masker on its way out.
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let out = child.stdout.take();
    let err = child.stderr.take();
    std::thread::scope(|scope| {
        if let Some(out) = out {
            scope.spawn(move || masker.copy(out, std::io::stdout()));
        }
        if let Some(err) = err {
            scope.spawn(move || masker.copy(err, std::io::stderr()));
        }
    });
    child.wait()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            None,
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

        let status = spawn_with_env("sh", &sh(&script), &vars(&[("TOKEN", secret)]), None).unwrap();
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
                spawn_with_env("sh", &sh(&format!("exit {code}")), &IndexMap::new(), None).unwrap();
            assert_eq!(status.code(), Some(code), "exit {code} was not preserved");
        }
    }

    /// Inherited, not cleared — otherwise PATH is gone and nothing runs.
    #[test]
    fn the_parent_environment_is_inherited() {
        std::env::set_var("RUN_INHERIT_PROBE", "from-parent");
        let status = spawn_with_env(
            "sh",
            &sh(r#"[ "$RUN_INHERIT_PROBE" = "from-parent" ]"#),
            &IndexMap::new(),
            None,
        )
        .unwrap();
        std::env::remove_var("RUN_INHERIT_PROBE");
        assert!(status.success(), "the parent environment was not inherited");
    }

    /// And the vault wins where both define a name — that is what injection means.
    #[test]
    fn a_vault_variable_overrides_the_inherited_one() {
        std::env::set_var("RUN_OVERRIDE_PROBE", "from-parent");
        let status = spawn_with_env(
            "sh",
            &sh(r#"[ "$RUN_OVERRIDE_PROBE" = "from-vault" ]"#),
            &vars(&[("RUN_OVERRIDE_PROBE", "from-vault")]),
            None,
        )
        .unwrap();
        std::env::remove_var("RUN_OVERRIDE_PROBE");
        assert!(status.success(), "the inherited value won");
    }

    /// I-227, DEC-CL1: evnx's own settings — the API token above all — do not
    /// reach the child. A test used to lock in that they did.
    #[test]
    fn t_i227_evnx_settings_are_not_inherited() {
        std::env::set_var("EVNX_TOKEN_PROBE_I227", "evnx_tok_secret");
        let status = spawn_with_env(
            "sh",
            &sh(r#"[ -z "${EVNX_TOKEN_PROBE_I227+set}" ]"#),
            &IndexMap::new(),
            None,
        )
        .unwrap();
        std::env::remove_var("EVNX_TOKEN_PROBE_I227");
        assert!(status.success(), "an EVNX_ variable reached the child");
    }

    /// I-228, DEC-CL1: a vault variable that makes the child load or run other
    /// code is refused, naming it, unless the user allows it.
    #[test]
    fn t_i228_unsafe_names_are_refused_unless_allowed() {
        let v = vars(&[
            ("API_KEY", "k"),
            ("LD_PRELOAD", "/tmp/x.so"),
            ("NODE_OPTIONS", "-r x"),
        ]);
        let err = check_names(&v, false, "the vault", "push to this vault")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("LD_PRELOAD") && err.contains("NODE_OPTIONS"),
            "{err}"
        );
        assert!(err.contains("--allow-unsafe-names"), "{err}");
        assert!(!err.contains("API_KEY"), "{err}");
        assert!(check_names(&v, true, "the vault", "push to this vault").is_ok());
        assert!(check_names(
            &vars(&[("API_KEY", "k")]),
            false,
            "the vault",
            "push to this vault"
        )
        .is_ok());
    }

    /// A missing command is an io::ErrorKind::NotFound, which `run` turns into
    /// the "use sh -c" hint rather than a bare OS error.
    #[test]
    fn a_missing_command_is_reported_as_not_found() {
        let err =
            spawn_with_env("evnx-no-such-binary-xyz", &[], &IndexMap::new(), None).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    // ─── masking (RM-43) ───────────────────────────────────────────────────

    fn masked(secrets: &[(&str, &str)], chunks: &[&str]) -> String {
        let m = Masker::new(&vars(secrets)).unwrap();
        struct Chunks(Vec<Vec<u8>>);
        impl Read for Chunks {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let c = self.0.remove(0);
                buf[..c.len()].copy_from_slice(&c);
                Ok(c.len())
            }
        }
        let mut out = Vec::new();
        m.copy(
            Chunks(chunks.iter().map(|c| c.as_bytes().to_vec()).collect()),
            &mut out,
        );
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn rm43_a_secret_split_across_reads_is_masked() {
        let s = [("API_KEY", "sk_live_51H8xQ2eZvKYlo2C")];
        assert_eq!(
            masked(&s, &["key=sk_live_51H8", "xQ2eZvKYlo2C done\n"]),
            "key=*** done\n"
        );
        assert_eq!(masked(&s, &["no secret here\n"]), "no secret here\n");
    }

    #[test]
    fn rm43_only_secrets_are_masked_and_short_values_are_left() {
        let vars_ = vars(&[
            ("PORT", "8080"),
            ("DEBUG", "true"),
            ("DB_PASSWORD", "hunter22"),
        ]);
        let m = Masker::new(&vars_).unwrap();
        let mut out = Vec::new();
        m.copy(&b"port 8080 debug true pw hunter22\n"[..], &mut out);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "port 8080 debug true pw ***\n"
        );
        assert!(Masker::new(&vars(&[("PORT", "8080")])).is_none());
    }

    #[test]
    fn rm43_the_child_output_is_masked() {
        let env = vars(&[("API_TOKEN", "tok_abcdef123456")]);
        let m = Masker::new(&env).unwrap();
        let status = spawn_with_env(
            "sh",
            &sh(r#"[ "$API_TOKEN" = "tok_abcdef123456" ]"#),
            &env,
            Some(&m),
        )
        .unwrap();
        assert!(
            status.success(),
            "masking must not change what the child receives"
        );
    }
}
