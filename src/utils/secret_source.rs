//! Where a backup passphrase comes from: one rule for `backup` and `restore`
//! (DEC-B13, R-backup-restore).
//!
//! Highest priority first:
//!
//! 1. a key file (`--key-file`, also spelled `--password-file`),
//! 2. the `EVNX_PASSWORD` environment variable,
//! 3. a hidden prompt, when there is a terminal to show it on.
//!
//! Both commands carried their own copy of this until 0.10.11, and the copies
//! had drifted: the 8-character floor held for `backup`'s prompt only (I-147),
//! a key file silently beat `EVNX_PASSWORD` (I-155), `restore` warned about a
//! key file that `backup` merely noted, and with no terminal both failed with
//! dialoguer's "IO error: not a terminal". The key-file reader was already
//! shared, because it decides what string Argon2id sees (F7).
//!
//! The cloud master password is read here too, by [`resolve_master`] (CLOUD-PW,
//! the cloud half of B-WP3): `--password-stdin`, then `EVNX_MASTER_PASSWORD_FILE`,
//! then `EVNX_MASTER_PASSWORD`, then a prompt. It is a different secret with its
//! own variables (one leaked variable must not open both a backup and a vault),
//! its own floor (12 characters, applied where an account is created, not here)
//! and no confirmation prompt when unlocking, but the sources are read by this
//! module's code, not a second copy of it.

use std::path::Path;

use anyhow::{anyhow, Result};
use zeroize::{Zeroize, Zeroizing};

use crate::utils::ui;

/// The shortest passphrase a new backup accepts, in characters (DEC-B2).
pub const MIN_LEN: usize = 8;

/// The variable read when no key file is given.
pub const ENV_VAR: &str = "EVNX_PASSWORD";

/// What the passphrase is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A new backup: the floor applies to every source, and a typed
    /// passphrase is asked twice.
    Encrypt,
    /// An existing backup: no floor, so a backup made before the floor
    /// existed still opens (DEC-B2); asked once.
    Decrypt,
}

impl Purpose {
    fn word(self) -> &'static str {
        match self {
            Purpose::Encrypt => "encryption",
            Purpose::Decrypt => "decryption",
        }
    }
}

/// The typed confirmation did not match the passphrase. `backup` reports it
/// as its own exit code 3.
#[derive(Debug, thiserror::Error)]
#[error("Passwords do not match")]
pub struct Mismatch;

/// The passphrase for `purpose`, from the first source that is set.
///
/// `key_file` is the flag's value; `example` is the command line a script
/// would run instead of answering a prompt (`evnx backup .env`), for the
/// message shown when there is no terminal.
///
/// # Errors
///
/// A missing, unreadable or empty key file; an empty `EVNX_PASSWORD`; a new
/// backup's passphrase under [`MIN_LEN`] characters; no source and no
/// terminal; [`Mismatch`] when the confirmation differs.
pub fn resolve(
    key_file: Option<&str>,
    purpose: Purpose,
    example: &str,
    verbose: bool,
) -> Result<Zeroizing<String>> {
    if let Some(kf) = key_file {
        return from_key_file(kf, purpose, verbose);
    }
    if let Ok(pw) = std::env::var(ENV_VAR) {
        return from_env(Zeroizing::new(pw), purpose, verbose);
    }
    from_prompt(purpose, example, verbose)
}

/// Read a key file as the passphrase string Argon2id is given.
///
/// UTF-8 content is trimmed of leading and trailing whitespace (the common
/// trailing newline); binary content is Base64-encoded, which gives a stable
/// ASCII string for any bytes.
///
/// ⚠️ **One reader for both commands.** Whatever this returns is what Argon2id
/// saw at backup time, so it is the only string that can open the archive.
/// `restore` once read its `--password-file` with `read_to_string` and a
/// trailing-newline strip, which differed for a binary file (an error instead
/// of the Base64) and for a file with leading or trailing whitespace (F7).
pub fn read_key_file(path: &Path) -> Result<Zeroizing<String>> {
    use base64::{engine::general_purpose, Engine as _};

    let bytes = Zeroizing::new(
        std::fs::read(path)
            .map_err(|e| anyhow!("Failed to read key file {}: {}", path.display(), e))?,
    );
    Ok(Zeroizing::new(match std::str::from_utf8(&bytes) {
        Ok(s) => s.trim().to_owned(),
        Err(_) => general_purpose::STANDARD.encode(&*bytes),
    }))
}

fn from_key_file(kf: &str, purpose: Purpose, verbose: bool) -> Result<Zeroizing<String>> {
    let path = Path::new(kf);
    if !path.exists() {
        return Err(anyhow!("Key file not found: {kf}"));
    }
    if !path.is_file() {
        return Err(anyhow!("Key file is not a regular file: {kf}"));
    }
    let pw = read_key_file(path)?;
    if pw.is_empty() {
        return Err(anyhow!("Key file is empty: {kf}"));
    }
    if purpose == Purpose::Encrypt {
        floor(&format!("The key file {kf}"), &pw, true)?;
    }
    if verbose {
        ui::verbose_stderr(format!(
            "Password source: key file {kf} ({} characters)",
            pw.chars().count()
        ));
    }
    ui::info(format!(
        "Using key file for {} (non-interactive mode)",
        purpose.word()
    ));

    // I-155: the variable lost without a word, so a pipeline that set both
    // could believe it was using the one it had just rotated.
    if std::env::var_os(ENV_VAR).is_some() {
        ui::warning(format!(
            "{ENV_VAR} is set as well; the key file wins, so {ENV_VAR} was not used"
        ));
        std::env::remove_var(ENV_VAR);
    }
    Ok(pw)
}

fn from_env(pw: Zeroizing<String>, purpose: Purpose, verbose: bool) -> Result<Zeroizing<String>> {
    // Best-effort removal, empty or not: it shortens the window in which a
    // child process inherits the value. It cannot unsee what the shell, a
    // process monitor or a CI log already captured.
    std::env::remove_var(ENV_VAR);
    if pw.is_empty() {
        let why = match purpose {
            Purpose::Encrypt => {
                "An empty passphrase would encrypt the backup behind nothing. Unset it to be \
                 prompted, or give it a value."
            }
            Purpose::Decrypt => "Unset it to be prompted, or give it a value.",
        };
        return Err(anyhow!("{ENV_VAR} is set but empty.\n\n{why}"));
    }
    if purpose == Purpose::Encrypt {
        floor(ENV_VAR, &pw, true)?;
    }
    ui::warning(format!(
        "Reading password from {ENV_VAR} environment variable.\n  \
         Less secure than interactive prompt — avoid in shared environments."
    ));
    if verbose {
        ui::verbose_stderr(format!("Password source: {ENV_VAR} env var"));
    }
    Ok(pw)
}

fn from_prompt(purpose: Purpose, example: &str, verbose: bool) -> Result<Zeroizing<String>> {
    use dialoguer::Password;

    if !crate::utils::term::interactive() {
        return Err(anyhow!(
            "there is no terminal to ask for the passphrase on.\n\n\
             \x20 Give it without a prompt, from a file or the environment:\n\
             \x20   {example} --key-file <path>\n\
             \x20   {ENV_VAR}=… {example}"
        ));
    }
    if verbose {
        ui::verbose_stderr("Password source: interactive prompt");
    }

    let pw = Zeroizing::new(
        Password::new()
            .with_prompt(format!("Enter {} password", purpose.word()))
            .interact()?,
    );
    if pw.is_empty() {
        return Err(anyhow!("Password must not be empty"));
    }
    if purpose == Purpose::Decrypt {
        return Ok(pw);
    }

    floor("Password", &pw, false)?;
    if verbose {
        ui::verbose_stderr("Password accepted — awaiting confirmation");
    }
    let mut confirm = Password::new().with_prompt("Confirm password").interact()?;
    let matches = *pw == confirm;
    confirm.zeroize();
    if !matches {
        return Err(Mismatch.into());
    }
    if verbose {
        ui::verbose_stderr("Passwords match");
    }
    Ok(pw)
}

// ── The cloud master password (CLOUD-PW) ─────────────────────────────────────

/// The variable the cloud master password is read from when unlocking.
///
/// ⚠️ Its own variable, never [`ENV_VAR`], which unlocks `backup` files: one
/// leaked variable must not open both.
pub const MASTER_ENV_VAR: &str = "EVNX_MASTER_PASSWORD";

/// A file holding the cloud master password, for a machine that has one
/// (a mounted secret) and no terminal. Read before [`MASTER_ENV_VAR`].
pub const MASTER_FILE_ENV_VAR: &str = "EVNX_MASTER_PASSWORD_FILE";

/// Where [`resolve_master`] found the password, so the caller can say so
/// (without the value) on stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MasterSource {
    /// `--password-stdin`: the first line of standard input.
    Stdin,
    /// The file named by [`MASTER_FILE_ENV_VAR`].
    File(String),
    /// [`MASTER_ENV_VAR`].
    Env,
    /// A hidden prompt on the terminal.
    Prompt,
}

/// The first line of `input` without its line ending, and nothing else
/// stripped: leading and inner spaces are part of a passphrase.
///
/// # Errors
///
/// The read failed.
pub fn first_line(input: &mut dyn std::io::BufRead) -> Result<Zeroizing<String>> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
    line.zeroize();
    Ok(Zeroizing::new(trimmed))
}

/// The password in `path`: its first line, as [`first_line`] reads stdin.
///
/// Not [`read_key_file`], which trims every edge and Base64-encodes binary
/// content for Argon2id to be given a stable string: a master password typed
/// at a prompt keeps a trailing space, so a file must not lose it.
fn password_file(path: &str) -> Result<Zeroizing<String>> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err(anyhow!(
            "{MASTER_FILE_ENV_VAR} names {path}, which is not a readable file"
        ));
    }
    let bytes = Zeroizing::new(
        std::fs::read(p).map_err(|e| anyhow!("reading {MASTER_FILE_ENV_VAR} ({path}): {e}"))?,
    );
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow!("{path} (from {MASTER_FILE_ENV_VAR}) is not valid UTF-8"))?;
    let pw = first_line(&mut std::io::Cursor::new(text.as_bytes()))?;
    if pw.is_empty() {
        return Err(anyhow!("{path} (from {MASTER_FILE_ENV_VAR}) is empty"));
    }
    crate::core::permissions::warn_if_readable(p);
    Ok(pw)
}

/// What [`resolve_master`] reads from. The real process in production; fixed
/// values in a test, so no test touches the environment or the terminal.
pub struct MasterInputs<'a> {
    /// `--password-stdin`: read standard input and nothing else.
    pub from_stdin: bool,
    /// Whether the environment variables and the file may supply it. `false`
    /// for a password being *changed* (`rotate-master-password`), which always
    /// asks.
    pub allow_env: bool,
    /// Standard input.
    pub stdin: &'a mut dyn std::io::BufRead,
    /// An environment variable's value, `None` when unset or empty.
    pub env: &'a dyn Fn(&str) -> Option<std::ffi::OsString>,
    /// Ask on the terminal, or fail when there is none.
    pub prompt: &'a mut dyn FnMut() -> Result<Zeroizing<String>>,
}

/// The cloud master password, from the first source that has one:
/// `--password-stdin`, [`MASTER_FILE_ENV_VAR`], [`MASTER_ENV_VAR`], a prompt.
///
/// An empty variable counts as unset (it falls through to the prompt), as it
/// always did; an empty or missing *file* is an error, because someone
/// pointed at it on purpose.
///
/// # Errors
///
/// An unreadable or empty file, a value that is not UTF-8, a failed read, or
/// whatever `prompt` returns (no terminal).
pub fn resolve_master(inputs: MasterInputs<'_>) -> Result<(Zeroizing<String>, MasterSource)> {
    if inputs.from_stdin {
        return Ok((first_line(inputs.stdin)?, MasterSource::Stdin));
    }
    if inputs.allow_env {
        if let Some(path) = (inputs.env)(MASTER_FILE_ENV_VAR) {
            let path = path
                .into_string()
                .map_err(|_| anyhow!("{MASTER_FILE_ENV_VAR} is not valid UTF-8"))?;
            return Ok((password_file(&path)?, MasterSource::File(path)));
        }
        if let Some(pw) = (inputs.env)(MASTER_ENV_VAR) {
            let pw = pw
                .into_string()
                .map_err(|_| anyhow!("{MASTER_ENV_VAR} is not valid UTF-8"))?;
            return Ok((Zeroizing::new(pw), MasterSource::Env));
        }
    }
    Ok(((inputs.prompt)()?, MasterSource::Prompt))
}

/// Refuse a new backup's passphrase under [`MIN_LEN`] characters.
///
/// `label` names the source as the message's subject. Characters, not bytes,
/// as the cloud master password's floor counts them.
fn floor(label: &str, pw: &str, not_typed: bool) -> Result<()> {
    let n = pw.chars().count();
    if n >= MIN_LEN {
        return Ok(());
    }
    let mut msg = format!("{label} must be at least {MIN_LEN} characters (got {n})");
    if not_typed {
        msg.push_str(
            "\n\n  Only a new backup needs it: one made earlier with a shorter \
             passphrase still restores.",
        );
    }
    Err(anyhow!(msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_key_files_are_trimmed_and_binary_ones_encoded() {
        let dir = tempfile::TempDir::new().unwrap();
        let text = dir.path().join("text");
        std::fs::write(&text, "  secret-passphrase \n").unwrap();
        assert_eq!(&*read_key_file(&text).unwrap(), "secret-passphrase");

        let bin = dir.path().join("bin");
        std::fs::write(&bin, [0xff, 0xfe, 0x00, 0x01]).unwrap();
        assert_eq!(&*read_key_file(&bin).unwrap(), "//4AAQ==");
    }

    #[test]
    fn the_floor_counts_characters_and_names_its_source() {
        assert!(floor("EVNX_PASSWORD", "äöüßäöüß", true).is_ok());
        let e = floor("EVNX_PASSWORD", "abc", true).unwrap_err().to_string();
        assert!(e.starts_with("EVNX_PASSWORD must be at least 8 characters (got 3)"));
        assert!(e.contains("still restores"));
        // The prompt's message is the one it always printed.
        assert_eq!(
            floor("Password", "abc", false).unwrap_err().to_string(),
            "Password must be at least 8 characters (got 3)"
        );
    }

    // ── CLOUD-PW: the cloud master password's sources ─────────────────────────

    use std::ffi::OsString;

    /// Run `resolve_master` against fixed inputs. `prompted` is what the prompt
    /// would return, `None` for "no terminal".
    fn master(
        from_stdin: bool,
        allow_env: bool,
        stdin: &str,
        env: &[(&str, &str)],
        prompted: Option<&str>,
    ) -> Result<(String, MasterSource)> {
        let mut input = std::io::Cursor::new(stdin.as_bytes().to_vec());
        let lookup = |name: &str| {
            env.iter()
                .find(|(k, v)| *k == name && !v.is_empty())
                .map(|(_, v)| OsString::from(v))
        };
        let mut ask = || match prompted {
            Some(p) => Ok(Zeroizing::new(p.to_string())),
            None => Err(anyhow!("no terminal")),
        };
        resolve_master(MasterInputs {
            from_stdin,
            allow_env,
            stdin: &mut input,
            env: &lookup,
            prompt: &mut ask,
        })
        .map(|(pw, src)| (pw.to_string(), src))
    }

    #[test]
    fn master_stdin_wins_and_keeps_spaces() {
        let env = [(MASTER_ENV_VAR, "from-env")];
        let (pw, src) = master(true, true, "  pass phrase \r\nsecond\n", &env, None).unwrap();
        assert_eq!((pw.as_str(), src), ("  pass phrase ", MasterSource::Stdin));
    }

    #[test]
    fn master_env_then_prompt_and_an_empty_variable_is_unset() {
        let (pw, src) = master(false, true, "", &[(MASTER_ENV_VAR, "from-env")], None).unwrap();
        assert_eq!((pw.as_str(), src), ("from-env", MasterSource::Env));

        let (pw, src) = master(false, true, "", &[(MASTER_ENV_VAR, "")], Some("typed")).unwrap();
        assert_eq!((pw.as_str(), src), ("typed", MasterSource::Prompt));

        let e = master(false, true, "", &[], None).unwrap_err().to_string();
        assert_eq!(e, "no terminal");
    }

    #[test]
    fn master_env_is_ignored_when_the_password_is_being_changed() {
        let env = [(MASTER_ENV_VAR, "from-env")];
        let (pw, src) = master(false, false, "", &env, Some("typed")).unwrap();
        assert_eq!((pw.as_str(), src), ("typed", MasterSource::Prompt));
    }

    #[test]
    fn master_file_beats_env_and_is_read_like_stdin() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = dir.path().join("pw");
        std::fs::write(&f, " keeps edges \nignored second line\n").unwrap();
        let path = f.to_str().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let env = [(MASTER_FILE_ENV_VAR, path), (MASTER_ENV_VAR, "from-env")];
        let (pw, src) = master(false, true, "", &env, None).unwrap();
        assert_eq!(
            (pw.as_str(), src),
            (" keeps edges ", MasterSource::File(path.to_string()))
        );
        // ...and stdin still beats the file.
        let (pw, src) = master(true, true, "piped\n", &env, None).unwrap();
        assert_eq!((pw.as_str(), src), ("piped", MasterSource::Stdin));
    }

    #[test]
    fn master_file_problems_are_errors_not_a_fall_through() {
        let dir = tempfile::TempDir::new().unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, "\n").unwrap();
        let missing = dir.path().join("missing");
        for (path, needle) in [
            (empty.to_str().unwrap(), "is empty"),
            (missing.to_str().unwrap(), "not a readable file"),
        ] {
            let env = [(MASTER_FILE_ENV_VAR, path), (MASTER_ENV_VAR, "from-env")];
            let e = master(false, true, "", &env, Some("typed")).unwrap_err();
            assert!(e.to_string().contains(needle), "{e}");
            assert!(e.to_string().contains(MASTER_FILE_ENV_VAR), "{e}");
        }
    }
}
