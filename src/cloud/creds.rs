//! Credential store — `~/.config/evnx/credentials.json`, mode 0600.
//!
//! # What is stored, and what is deliberately not
//!
//! Stored: the access token, its expiry, the refresh token, and the account email
//! for display. Keyed by canonical server URL, so a local development server and
//! the hosted one can both be signed in at once.
//!
//! **Never stored: anything that can decrypt a vault.** Not the master key, not a
//! vault key, not the unwrapped private key. Those are derived from the master
//! password in memory, used, and dropped. That is the zero-knowledge guarantee
//! made concrete — a stolen laptop with a stolen `credentials.json` gets the
//! thief an authenticated session against the API, which serves them ciphertext.
//!
//! # File backend only
//!
//! Decided 2026-09-13. Mode 0600 in the user's config directory is the model
//! `~/.aws/credentials`, `~/.npmrc` and `~/.docker/config.json` all use. An OS
//! keychain is better on a desktop, but it costs roughly thirty crates — a second
//! async runtime through zbus — needs a file fallback for headless CI regardless,
//! and CI is the main place this CLI runs. It can be added later behind its own
//! feature flag without changing this format.
//!
//! # Zeroization is best effort
//!
//! [`SecretString`] wipes its buffer on drop, which covers the copies this module
//! controls. It cannot reach transient copies `serde_json` makes while parsing, or
//! anything the allocator has already handed back. Treated as defence in depth,
//! not as a guarantee — the file permissions are the real control.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use zeroize::Zeroize;

use super::config;

/// Current on-disk schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Treat an access token as expired this many seconds early, so a token does not
/// die in flight between the check and the server receiving it.
pub const EXPIRY_SKEW_SECS: i64 = 30;

/// A string that wipes itself on drop and never appears in `Debug` output.
///
/// Accidentally logging a struct containing a token is the failure this exists to
/// prevent: `{:?}` on a [`Session`] prints redaction markers, not bearer tokens.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    /// Wrap a secret value.
    pub fn new(s: impl Into<String>) -> Self {
        SecretString(s.into())
    }

    /// Borrow the secret. Named `expose` so call sites read as a deliberate act,
    /// matching `evnx-crypto`'s convention for key material.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when the value is empty — a token that would fail anyway.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for SecretString {}

/// One authenticated session against one server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Account email, shown by `evnx cloud status`. Not a credential.
    pub email: String,
    /// Short-lived bearer token, 15 minutes.
    pub access_token: SecretString,
    /// Access-token expiry as a Unix timestamp.
    ///
    /// Seconds since the epoch rather than an RFC 3339 string so the format does
    /// not depend on `chrono`'s optional `serde` feature, and so anything can read
    /// it without a date parser.
    pub access_expires_at: i64,
    /// Long-lived token, 30 days, rotated on every use by the server.
    pub refresh_token: SecretString,
}

impl Session {
    /// Whether the access token is still usable, allowing for [`EXPIRY_SKEW_SECS`].
    pub fn access_token_is_fresh(&self, now_unix: i64) -> bool {
        !self.access_token.is_empty() && now_unix + EXPIRY_SKEW_SECS < self.access_expires_at
    }
}

/// The whole credentials file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    /// Schema version, so a future format change can migrate rather than crash.
    pub version: u32,
    /// Sessions keyed by canonical server URL — see [`config::canonical_server`].
    #[serde(default)]
    pub sessions: BTreeMap<String, Session>,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            version: SCHEMA_VERSION,
            sessions: BTreeMap::new(),
        }
    }
}

impl Store {
    /// Load the credential store, or an empty one if the file does not exist.
    ///
    /// A missing file means "not signed in", which is not an error. A file written
    /// by a newer evnx *is* an error: guessing at a format we do not understand
    /// risks writing it back with fields silently dropped.
    pub fn load() -> Result<Self> {
        Self::load_from(&config::credentials_path()?)
    }

    /// [`Store::load`] against an explicit path.
    pub fn load_from(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };

        // Only warn once the file is known to exist and be readable.
        warn_and_tighten_if_permissive(path)?;

        let store: Store =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

        if store.version > SCHEMA_VERSION {
            return Err(anyhow!(
                "{} was written by a newer evnx (format v{}, this build understands v{}). \
                 Upgrade evnx, or remove the file to sign in again.",
                path.display(),
                store.version,
                SCHEMA_VERSION
            ));
        }
        Ok(store)
    }

    /// Persist the store atomically at mode 0600.
    pub fn save(&self) -> Result<()> {
        let dir = config::ensure_config_dir()?;
        self.save_to(&dir.join("credentials.json"))
    }

    /// [`Store::save`] against an explicit path.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self).context("serializing credentials")?;
        write_atomic_secure(path, &json)
    }

    /// The session for a server, if signed in.
    pub fn session(&self, server: &str) -> Option<&Session> {
        self.sessions.get(server)
    }

    /// Store or replace the session for a server.
    pub fn set_session(&mut self, server: impl Into<String>, session: Session) {
        self.sessions.insert(server.into(), session);
    }

    /// Remove one server's session. Returns whether there was one.
    pub fn remove_session(&mut self, server: &str) -> bool {
        self.sessions.remove(server).is_some()
    }

    /// Servers this machine has a session for.
    pub fn servers(&self) -> impl Iterator<Item = &str> {
        self.sessions.keys().map(String::as_str)
    }
}

/// Write a secret file atomically, owner-readable only.
///
/// Three properties the shared [`crate::utils::write_secure`] does not provide,
/// each of which matters for a file rewritten on every token refresh:
///
/// 1. **Atomic.** A temporary file in the same directory is renamed over the
///    target, so an interrupted write cannot leave a truncated file. `write_secure`
///    opens with `truncate(true)`, so a crash mid-write signs the user out.
/// 2. **Permissions on rewrite.** `OpenOptions::mode()` applies only when the file
///    is *created*, so writing over an existing 0644 file keeps 0644. The mode is
///    set here on the temp file, before any secret reaches it, and survives the
///    rename.
/// 3. **Durable.** `sync_all` before the rename, so a crash leaves either the old
///    file or the new one, never an empty one.
fn write_atomic_secure(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;

    // Same directory, so the rename stays within one filesystem and is atomic.
    // tempfile creates with 0600 on Unix, before anything is written into it.
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("creating a temporary file in {}", dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .context("restricting the temporary credentials file to 0600")?;
    }

    tmp.write_all(bytes).context("writing credentials")?;
    tmp.as_file().sync_all().context("flushing credentials")?;
    tmp.persist(path)
        .map_err(|e| anyhow!("replacing {}: {}", path.display(), e.error))?;
    Ok(())
}

/// Warn if the credentials file is readable by anyone else, and tighten it.
///
/// Tightening rather than refusing outright: OpenSSH refuses, but it is protecting
/// a key that unlocks remote machines, and its users know the convention. Here the
/// friendlier repair still closes the hole, and the warning goes to stderr so it
/// is visible without corrupting piped stdout. If the mode cannot be fixed, that
/// *is* an error — carrying on would mean writing a fresh token into a file
/// everyone can read.
#[cfg(unix)]
fn warn_and_tighten_if_permissive(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .with_context(|| format!("reading permissions of {}", path.display()))?
        .permissions()
        .mode()
        & 0o777;

    if mode & 0o077 != 0 {
        eprintln!(
            "warning: {} was mode {:o}, readable beyond its owner. Tightening to 0600.\n\
             \x20        Treat the stored tokens as exposed and run `evnx auth logout` \
             on all devices if this machine is shared.",
            path.display(),
            mode
        );
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("could not restrict {} to 0600", path.display()))?;
    }
    Ok(())
}

/// Windows relies on the user-profile ACL, which is owner-only by default.
#[cfg(not(unix))]
fn warn_and_tighten_if_permissive(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn session(email: &str) -> Session {
        Session {
            email: email.into(),
            access_token: SecretString::new("access-abc"),
            access_expires_at: 4_000_000_000,
            refresh_token: SecretString::new("refresh-xyz"),
        }
    }

    use crate::cloud::testutil::ConfigDirGuard;

    #[test]
    #[serial]
    fn a_missing_file_means_signed_out_not_broken() {
        let _g = ConfigDirGuard::new();
        let store = Store::load().unwrap();
        assert!(store.sessions.is_empty());
        assert_eq!(store.version, SCHEMA_VERSION);
    }

    #[test]
    #[serial]
    fn sessions_round_trip_through_disk() {
        let _g = ConfigDirGuard::new();
        let mut store = Store::default();
        store.set_session("https://api.evnx.dev", session("a@example.com"));
        store.save().unwrap();

        let loaded = Store::load().unwrap();
        assert_eq!(loaded, store);
        assert_eq!(
            loaded
                .session("https://api.evnx.dev")
                .unwrap()
                .refresh_token
                .expose(),
            "refresh-xyz"
        );
    }

    #[test]
    #[serial]
    fn two_servers_are_signed_in_independently() {
        // The development loop is a local server on :8099 alongside the hosted
        // one; signing into one must not sign out of the other.
        let _g = ConfigDirGuard::new();
        let mut store = Store::default();
        store.set_session("https://api.evnx.dev", session("prod@example.com"));
        store.set_session("http://localhost:8099", session("dev@example.com"));
        store.save().unwrap();

        let mut loaded = Store::load().unwrap();
        assert_eq!(loaded.servers().count(), 2);

        assert!(loaded.remove_session("http://localhost:8099"));
        loaded.save().unwrap();

        let after = Store::load().unwrap();
        assert!(after.session("http://localhost:8099").is_none());
        assert_eq!(
            after.session("https://api.evnx.dev").unwrap().email,
            "prod@example.com"
        );
    }

    #[test]
    #[serial]
    fn removing_a_session_that_is_not_there_reports_false() {
        let _g = ConfigDirGuard::new();
        let mut store = Store::default();
        assert!(!store.remove_session("https://api.evnx.dev"));
    }

    #[test]
    #[serial]
    fn a_file_from_a_newer_evnx_is_refused_rather_than_guessed_at() {
        let g = ConfigDirGuard::new();
        std::fs::write(
            g.path().join("credentials.json"),
            r#"{"version": 99, "sessions": {}}"#,
        )
        .unwrap();
        let err = Store::load().unwrap_err().to_string();
        assert!(err.contains("newer evnx"), "got: {err}");
    }

    #[test]
    #[serial]
    fn a_corrupt_file_is_an_error_naming_the_path() {
        let g = ConfigDirGuard::new();
        let path = g.path().join("credentials.json");
        std::fs::write(&path, "{ not json").unwrap();
        let err = format!("{:#}", Store::load().unwrap_err());
        assert!(err.contains("credentials.json"), "got: {err}");
    }

    #[test]
    fn debug_output_never_leaks_a_token() {
        let s = session("a@example.com");
        let rendered = format!("{s:?}");
        assert!(!rendered.contains("access-abc"), "leaked: {rendered}");
        assert!(!rendered.contains("refresh-xyz"), "leaked: {rendered}");
        assert!(rendered.contains("redacted"));
        // The email is not a credential and stays visible for diagnostics.
        assert!(rendered.contains("a@example.com"));
    }

    #[test]
    fn token_freshness_accounts_for_clock_skew() {
        let s = Session {
            access_expires_at: 1_000,
            ..session("a@example.com")
        };
        assert!(s.access_token_is_fresh(900));
        // Inside the skew window the token is treated as already gone, so it is
        // not sent on a request that would land after it expired.
        assert!(!s.access_token_is_fresh(1_000 - EXPIRY_SKEW_SECS));
        assert!(!s.access_token_is_fresh(2_000));
    }

    #[test]
    fn an_empty_access_token_is_never_fresh() {
        let s = Session {
            access_token: SecretString::new(""),
            ..session("a@example.com")
        };
        assert!(!s.access_token_is_fresh(0));
    }

    #[cfg(unix)]
    #[test]
    #[serial]
    fn the_credentials_file_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let g = ConfigDirGuard::new();
        let mut store = Store::default();
        store.set_session("https://api.evnx.dev", session("a@example.com"));
        store.save().unwrap();

        let mode = std::fs::metadata(g.path().join("credentials.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
    }

    #[cfg(unix)]
    #[test]
    #[serial]
    fn rewriting_an_existing_loose_file_restores_0600() {
        // The regression that motivated write_atomic_secure: OpenOptions::mode()
        // is ignored when the file already exists, so a plain rewrite would leave
        // a world-readable file holding a freshly refreshed token.
        use std::os::unix::fs::PermissionsExt;
        let g = ConfigDirGuard::new();
        let path = g.path().join("credentials.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut store = Store::default();
        store.set_session("https://api.evnx.dev", session("a@example.com"));
        store.save().unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
    }

    #[cfg(unix)]
    #[test]
    #[serial]
    fn reading_a_world_readable_file_tightens_it() {
        use std::os::unix::fs::PermissionsExt;
        let g = ConfigDirGuard::new();
        let path = g.path().join("credentials.json");
        std::fs::write(&path, r#"{"version":1,"sessions":{}}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        Store::load().unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
    }

    #[cfg(unix)]
    #[test]
    #[serial]
    fn a_failed_save_leaves_the_previous_file_intact() {
        // Atomicity: persisting into a directory that has gone away must not
        // destroy what was already on disk.
        let g = ConfigDirGuard::new();
        let path = g.path().join("credentials.json");
        let mut store = Store::default();
        store.set_session("https://api.evnx.dev", session("first@example.com"));
        store.save().unwrap();

        let bad = g.path().join("missing-dir").join("credentials.json");
        let mut other = Store::default();
        other.set_session("https://api.evnx.dev", session("second@example.com"));
        assert!(other.save_to(&bad).is_err());

        assert_eq!(
            Store::load_from(&path)
                .unwrap()
                .session("https://api.evnx.dev")
                .unwrap()
                .email,
            "first@example.com"
        );
    }
}
