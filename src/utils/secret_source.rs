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
//! ⚠️ The cloud master password is not read here. It has its own floor (12
//! characters), `--password-stdin` and no key file; moving it onto this module
//! is the cloud half of B-WP3 (0.11.0, R-auth).

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
}
