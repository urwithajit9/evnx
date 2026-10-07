//! Platform-specific file permission utilities.

use anyhow::{Context, Result};
// use std::fs::File;
use std::path::Path;

/// Write content to a file that only its owner can read.
///
/// On Unix the result is mode `0o600`, **whether or not the file already
/// existed**. On Windows the file inherits the directory's ACL, which is
/// user-only in a normal profile but is not enforced here — see the note below.
///
/// ⛔ **`.mode()` applies only when `open` creates the file.** The previous
/// version passed `.mode(0o600)` and trusted it, so writing over an existing
/// file kept that file's permissions:
///
/// ```text
/// new file                → 600
/// pre-existing file at    → 644 (before)
/// after write_secure      → 644   ⛔ secrets world-readable
/// via symlink → target is → 644   ⛔ wrote through the symlink
/// ```
///
/// Restoring a backup over a `.env` that was created by hand — which is the
/// normal case, since the file usually exists — therefore left the decrypted
/// secrets readable by every account on the machine, under messages that said
/// "written 0600".
///
/// Two changes:
///
/// * **`O_NOFOLLOW`**, so a symlink where the target was expected is an error
///   rather than a write to wherever it points. A `.env` replaced by a link to
///   somebody else's file is otherwise a way to have evnx write secrets there.
/// * **`set_permissions` after opening**, which applies to the file that is
///   actually open rather than only to one being created.
pub fn write_secure<P: AsRef<Path>>(path: P, content: &[u8]) -> Result<()> {
    let path = path.as_ref();

    #[cfg(unix)]
    {
        use std::fs::{OpenOptions, Permissions};
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        // ⚠️ `custom_flags` and `mode` are separate: the first is passed to
        // `open(2)` as-is, the second only consulted on creation.
        opts.custom_flags(libc::O_NOFOLLOW);

        let mut file = opts.open(path).with_context(|| {
            format!(
                "could not open {} for writing — if it is a symlink, evnx \
                 refuses to write through it",
                path.display()
            )
        })?;

        // ⛔ After opening, not before. This is the line the old version lacked.
        file.set_permissions(Permissions::from_mode(0o600))
            .with_context(|| format!("could not restrict permissions on {}", path.display()))?;

        std::io::Write::write_all(&mut file, content)?;
    }

    #[cfg(not(unix))]
    {
        // ⚠️ No ACL is set here, so do not claim one is. Callers that print
        // "0600" must not do so on Windows — see `describe_permissions`.
        std::fs::write(path, content)?;
    }

    Ok(())
}

/// How to describe what [`write_secure`] just did, for a message to a person.
///
/// ⚠️ Exists because three call sites printed "written 0600" unconditionally,
/// including on Windows where nothing sets a mode at all. A reassurance that is
/// false on one platform is worse than no reassurance.
#[must_use]
pub fn describe_permissions() -> &'static str {
    if cfg!(unix) {
        "mode 0600"
    } else {
        "inherited from the parent directory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "evnx-perm-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    #[cfg(unix)]
    fn mode_of(p: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    /// ⛔ **The defect.** `.mode()` applies only when `open` creates the file, so
    /// writing over an existing one kept that file's permissions:
    ///
    /// ```text
    /// pre-existing file at 644 → after write_secure → 644
    /// ```
    ///
    /// Restoring a backup over a `.env` created by hand is the normal case, and
    /// it left the decrypted secrets readable by every account on the machine.
    #[cfg(unix)]
    #[test]
    fn an_existing_file_is_restricted_too() {
        use std::os::unix::fs::PermissionsExt;
        let p = scratch("existing.env");
        fs::write(&p, b"old").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode_of(&p), 0o644, "fixture did not start world-readable");

        write_secure(&p, b"SECRET=live").unwrap();

        assert_eq!(mode_of(&p), 0o600, "an existing file kept its permissions");
        assert_eq!(fs::read_to_string(&p).unwrap(), "SECRET=live");
    }

    #[cfg(unix)]
    #[test]
    fn a_new_file_is_restricted() {
        let p = scratch("fresh.env");
        write_secure(&p, b"SECRET=live").unwrap();
        assert_eq!(mode_of(&p), 0o600);
    }

    /// ⛔ A `.env` replaced by a symlink was a way to have evnx write the
    /// decrypted secrets wherever it pointed, at whatever permissions the
    /// target already had.
    #[cfg(unix)]
    #[test]
    fn a_symlink_is_refused_rather_than_followed() {
        use std::os::unix::fs::PermissionsExt;
        let target = scratch("target.env");
        fs::write(&target, b"").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        let link = target.with_file_name("link.env");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(
            write_secure(&link, b"SECRET=live").is_err(),
            "wrote through a symlink"
        );
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "",
            "the symlink's target was written to"
        );
    }

    /// ⚠️ Three call sites printed "0600" unconditionally, including on Windows
    /// where nothing sets a mode at all.
    #[test]
    fn the_description_matches_the_platform() {
        let d = describe_permissions();
        if cfg!(unix) {
            assert_eq!(d, "mode 0600");
        } else {
            assert!(!d.contains("0600"), "claimed a mode Windows does not set");
        }
    }
}
