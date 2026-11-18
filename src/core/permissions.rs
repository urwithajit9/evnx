//! Who can read a file: one definition of "private" for every command (C-04, A-29).
//!
//! "Private" means no permission bits for group or others (`mode & 0o077 == 0`).
//! Before this module the rule lived in three places (`utils::fs`, `sync`'s
//! permission check and `doctor`), worded three ways.
//!
//! ⚠️ **Windows:** there is no mode to read. [`is_private`] answers `true`
//! (an ACL cannot be checked cheaply) and [`MODES_APPLY`] is `false`, so no
//! caller prints "0600" there. What evnx does set is an ACL: [`owner_only`]
//! runs `icacls <file> /inheritance:r /grant:r <DOMAIN\USER>:(F)`, which drops
//! the entries inherited from the folder and leaves the current account as the
//! only one with access (Administrators and SYSTEM included: an administrator
//! can take ownership, as with any file). The argument list is built by a pure
//! function, [`icacls_owner_only_args`], so the Linux tests cover it.

use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;

/// Whether this platform has Unix modes. Callers that would print a mode, or
/// promise one, check this first.
pub const MODES_APPLY: bool = cfg!(unix);

/// Mode for a new file that holds secrets (`.env`, rendered templates,
/// `convert -o`, backups).
pub const PRIVATE: u32 = 0o600;

/// Mode for a new file meant to be shared and committed (`.env.example`).
pub const SHARED: u32 = 0o644;

/// Mode for a directory evnx creates to hold secrets (`cloud export`,
/// `template --mkdir`): only the owner can list it.
pub const PRIVATE_DIR: u32 = 0o700;

/// The permission bits of `path` (follows symlinks), or `None` when the file
/// cannot be read or the platform has no modes.
pub fn mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// `true` when no one but the owner can read or write `path`. On Windows,
/// always `true` (see the module note). A file that cannot be read is not
/// private.
pub fn is_private(path: &Path) -> bool {
    if !MODES_APPLY {
        return true;
    }
    mode(path).is_some_and(is_private_mode)
}

/// The rule itself, for a caller that already holds the mode to print it:
/// no bits for group or others. `700` and `400` are private; `640` is not.
pub fn is_private_mode(mode: u32) -> bool {
    mode & 0o077 == 0
}

/// Set `path` to [`PRIVATE`]. Returns `false` on Windows, where nothing was
/// changed.
pub fn make_private(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use anyhow::Context;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE))
            .with_context(|| format!("could not restrict permissions on {}", path.display()))?;
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        // Windows: an ACL, not a mode. Anywhere else with no modes: nothing.
        owner_only(path, AclFailure::Error)
    }
}

/// What a failed ACL change does: a warning (the file is still written, the
/// way an existing `.env` is kept) or an error (a `credentials.json` that
/// cannot be restricted must not be written, as on Unix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclFailure {
    Warn,
    Error,
}

/// The `DOMAIN\USER` `icacls` grants to, from the two variables Windows sets
/// (`USERDOMAIN`, `USERNAME`). `None` when either is missing or holds a
/// character that would change the meaning of the grant (`:`, `(`, `)`, `/`,
/// `*`, a quote or a control character).
pub fn account_name(domain: Option<&str>, user: Option<&str>) -> Option<String> {
    let ok = |s: &str| {
        !s.trim().is_empty()
            && !s
                .chars()
                .any(|c| c.is_control() || matches!(c, ':' | '(' | ')' | '/' | '*' | '"' | ','))
    };
    let (d, u) = (domain?.trim(), user?.trim());
    (ok(d) && ok(u)).then(|| format!("{d}\\{u}"))
}

/// The arguments of the `icacls` call that leaves `path` to `account` alone:
/// inheritance removed, one full-control grant. Pure, so it is tested
/// everywhere.
pub fn icacls_owner_only_args(path: &Path, account: &str) -> Vec<OsString> {
    vec![
        path.as_os_str().to_owned(),
        "/inheritance:r".into(),
        "/grant:r".into(),
        format!("{account}:(F)").into(),
    ]
}

/// Decide what a failed ACL change means. `Warn` prints the warning and
/// carries on; `Error` returns it.
pub fn acl_failure(policy: AclFailure, path: &Path, why: &str) -> Result<bool> {
    let message = format!(
        "could not restrict {} to your account ({why})",
        path.display()
    );
    match policy {
        AclFailure::Warn => {
            crate::utils::ui::warning(message);
            Ok(false)
        }
        AclFailure::Error => Err(anyhow::anyhow!(message)),
    }
}

/// Leave `path` to the current account alone, where the platform expresses it
/// as an ACL (Windows). `Ok(true)` when it was applied; `Ok(false)` where
/// there is nothing to apply (Unix uses modes, see [`make_private`]) or the
/// change failed under [`AclFailure::Warn`].
pub fn owner_only(path: &Path, policy: AclFailure) -> Result<bool> {
    #[cfg(windows)]
    {
        let account = account_name(
            std::env::var("USERDOMAIN").ok().as_deref(),
            std::env::var("USERNAME").ok().as_deref(),
        );
        let Some(account) = account else {
            return acl_failure(policy, path, "USERDOMAIN or USERNAME is not usable");
        };
        let out = std::process::Command::new("icacls")
            .args(icacls_owner_only_args(path, &account))
            .output();
        match out {
            Ok(o) if o.status.success() => Ok(true),
            Ok(o) => {
                let text = String::from_utf8_lossy(&o.stdout);
                let err = String::from_utf8_lossy(&o.stderr);
                let why = format!(
                    "icacls: {}",
                    err.trim().lines().next().unwrap_or(text.trim())
                );
                acl_failure(policy, path, &why)
            }
            Err(e) => acl_failure(policy, path, &format!("icacls could not run: {e}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, policy);
        Ok(false)
    }
}

/// The command that makes `path` private, for a message to a person.
pub fn chmod_hint(path: &Path) -> String {
    format!("chmod 600 {}", path.display())
}

/// Warn that `path` is readable by other users, with the `chmod` that fixes
/// it. `note` ends the first line (`safe_write::report` adds "evnx keeps the
/// mode you set"). One wording for every command (I-276).
pub fn readable_warning(path: &Path, mode: u32, note: Option<&str>) {
    use crate::utils::ui;
    ui::warning(format!(
        "{} is readable by other users (mode {:o}){}",
        path.display(),
        mode,
        note.map(|n| format!("; {n}")).unwrap_or_default()
    ));
    // SYNC-4: the one command that tightens every env file at once, named
    // only for a file it would fix.
    let is_env = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| crate::core::env_name::kind(n) == crate::core::env_name::FileKind::Env);
    if is_env {
        ui::warning_detail(format!(
            "     →  {}   (or evnx doctor --fix, for every env file)",
            chmod_hint(path)
        ));
    } else {
        ui::warning_detail(format!("     →  {}", chmod_hint(path)));
    }
}

/// [`readable_warning`] for `path` when it exists and group or others can
/// read it. Returns whether it warned. Nothing on Windows.
///
/// ⚠️ `sync` had its own check, worded `(0o644)`, and then `safe_write`
/// warned again after the write — two warnings for one file (I-126, I-276).
pub fn warn_if_readable(path: &Path) -> bool {
    match mode(path) {
        Some(m) if MODES_APPLY && !is_private_mode(m) => {
            readable_warning(path, m, None);
            true
        }
        _ => false,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn file_with_mode(m: u32) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, "x").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(m)).unwrap();
        (d, p)
    }

    #[test]
    fn private_means_no_group_or_other_bits() {
        for (m, private) in [
            (0o600, true),
            (0o400, true),
            // I-170: doctor accepted exactly 600 and 400, so 700 warned there.
            (0o700, true),
            (0o500, true),
            (0o640, false),
            (0o644, false),
            (0o604, false),
        ] {
            let (_d, p) = file_with_mode(m);
            assert_eq!(is_private(&p), private, "{m:o}");
        }
    }

    #[test]
    fn a_missing_file_is_not_private() {
        assert!(!is_private(Path::new("/nonexistent/evnx/f")));
    }

    #[test]
    fn make_private_sets_0600() {
        let (_d, p) = file_with_mode(0o644);
        assert!(make_private(&p).unwrap());
        assert_eq!(mode(&p), Some(0o600));
    }
}

#[cfg(test)]
mod acl_tests {
    use super::*;

    #[test]
    fn the_account_is_domain_backslash_user() {
        assert_eq!(
            account_name(Some("DESKTOP-1"), Some("ajit")).as_deref(),
            Some("DESKTOP-1\\ajit")
        );
    }

    #[test]
    fn an_account_that_could_change_the_grant_is_refused() {
        for (d, u) in [
            (None, Some("u")),
            (Some("D"), None),
            (Some(""), Some("u")),
            (Some("D"), Some("   ")),
            (Some("D"), Some("u:(F)")),
            (Some("D"), Some("u /grant Everyone")),
            (Some("D"), Some("*S-1-1-0")),
            (Some("D\"E"), Some("u")),
        ] {
            assert_eq!(account_name(d, u), None, "{d:?} {u:?}");
        }
    }

    #[test]
    fn wp12_icacls_args_are_owner_only() {
        let args = icacls_owner_only_args(Path::new("C:\\p\\.env"), "PC\\ajit");
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            ["C:\\p\\.env", "/inheritance:r", "/grant:r", "PC\\ajit:(F)"]
        );
        // One grant, full control, to the one account: nothing for Everyone,
        // Users or Administrators.
        assert_eq!(args.iter().filter(|a| a.contains("/grant")).count(), 1);
        assert!(!args
            .iter()
            .any(|a| a.contains("Everyone") || a.contains("Users")));
    }

    #[test]
    fn a_failed_acl_change_is_an_error_or_a_warning_by_policy() {
        let p = Path::new("credentials.json");
        let e = acl_failure(AclFailure::Error, p, "icacls: denied").unwrap_err();
        assert!(e.to_string().contains("credentials.json"), "{e}");
        assert!(e.to_string().contains("icacls: denied"), "{e}");
        assert!(!acl_failure(AclFailure::Warn, p, "x").unwrap());
    }

    #[cfg(not(windows))]
    #[test]
    fn owner_only_does_nothing_off_windows() {
        assert!(!owner_only(Path::new("/nonexistent/f"), AclFailure::Error).unwrap());
    }

    /// Only on the `windows-latest` CI job: the real `icacls` call, read back.
    #[cfg(windows)]
    #[test]
    fn wp12_acl_applied_on_windows() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(".env");
        std::fs::write(&f, "A=1\n").unwrap();
        assert!(owner_only(&f, AclFailure::Error).unwrap());
        let out = std::process::Command::new("icacls")
            .arg(&f)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let user = std::env::var("USERNAME").unwrap();
        assert!(text.contains(&format!("{user}:(F)")), "{text}");
        // Nothing inherited from the folder, and no one else.
        assert!(!text.contains("(I)"), "{text}");
        for other in [
            "Everyone",
            "BUILTIN\\Users",
            "BUILTIN\\Administrators",
            "NT AUTHORITY\\SYSTEM",
        ] {
            assert!(!text.contains(other), "{other} still has access:\n{text}");
        }
        // The file is still ours to read.
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "A=1\n");
    }
}
