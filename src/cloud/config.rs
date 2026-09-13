//! Cloud configuration — the non-secret half of `~/.config/evnx/`.
//!
//! Two files live side by side and are deliberately kept apart:
//!
//! | File | Contents | Written by |
//! |------|----------|------------|
//! | `config.toml` | server URL and other preferences | the user, by hand or by `evnx cloud config` |
//! | `credentials.json` | tokens | only the CLI, at mode 0600 — see [`crate::cloud::creds`] |
//!
//! # Why not `.evnx.toml`
//!
//! evnx already has a config file, and it is the wrong place for any of this.
//! `.evnx.toml` is *project-level*: [`crate::core::config`] searches the current
//! directory, then every parent, then `$HOME`. That is a file people commit. A
//! token in it would be pushed to a repository the first time someone ran
//! `git add -A`.
//!
//! A per-directory vault *binding* would be safe there — a vault id is not a
//! secret — but that is a later change and it still would not hold credentials.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The hosted evnx API. Overridable, but this is what users get by default.
pub const DEFAULT_SERVER: &str = "https://api.evnx.dev";

/// Overrides the whole config directory. Set by the test suite, and useful for
/// keeping work and personal accounts apart on one machine.
pub const ENV_CONFIG_DIR: &str = "EVNX_CONFIG_DIR";

/// Overrides the server URL for one invocation.
pub const ENV_SERVER: &str = "EVNX_SERVER";

/// Directory holding `config.toml` and `credentials.json`.
///
/// Platform-native via `dirs::config_dir()` — `~/.config/evnx` on Linux (honouring
/// `XDG_CONFIG_HOME`), `~/Library/Application Support/evnx` on macOS,
/// `%APPDATA%\evnx` on Windows — unless [`ENV_CONFIG_DIR`] overrides it.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(ENV_CONFIG_DIR) {
        if dir.is_empty() {
            return Err(anyhow!("{ENV_CONFIG_DIR} is set but empty"));
        }
        return Ok(PathBuf::from(dir));
    }
    dirs::config_dir()
        .map(|d| d.join("evnx"))
        .ok_or_else(|| anyhow!("could not determine a config directory; set {ENV_CONFIG_DIR}"))
}

/// Create the config directory if needed, restricted to the owner.
///
/// `0700` matters as much as the `0600` on the file itself: a world-executable
/// directory lets another local user stat and probe the names inside it, and on
/// some systems replace a file the CLI is about to write.
pub fn ensure_config_dir() -> Result<PathBuf> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating config directory {}", dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("restricting {} to 0700", dir.display()))?;
        }
    }

    Ok(dir)
}

/// Path to `config.toml`.
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Path to `credentials.json`.
pub fn credentials_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("credentials.json"))
}

/// Non-secret cloud preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CloudConfig {
    /// Server this machine talks to by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

impl CloudConfig {
    /// Read `config.toml`, or return defaults when it does not exist.
    ///
    /// A missing file is normal — most users never write one. A *malformed* file
    /// is an error rather than a silent fallback to defaults: quietly talking to
    /// `api.evnx.dev` because a self-hosted URL had a typo is exactly the failure
    /// a developer would not notice.
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        Self::load_from(&path)
    }

    /// [`CloudConfig::load`] against an explicit path.
    pub fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Write `config.toml`, creating the directory if needed.
    pub fn save(&self) -> Result<()> {
        let dir = ensure_config_dir()?;
        let path = dir.join("config.toml");
        let text = toml::to_string_pretty(self).context("serializing cloud config")?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Resolve the server URL for this invocation.
    ///
    /// Precedence, highest first: the `--server` flag, [`ENV_SERVER`],
    /// `config.toml`, then [`DEFAULT_SERVER`]. The result is canonicalised, so it
    /// is safe to use as the credential-store key.
    pub fn resolve_server(cli_override: Option<&str>) -> Result<String> {
        if let Some(s) = cli_override {
            return canonical_server(s).context("invalid --server value");
        }
        if let Ok(s) = std::env::var(ENV_SERVER) {
            if !s.trim().is_empty() {
                return canonical_server(&s).with_context(|| format!("invalid {ENV_SERVER} value"));
            }
        }
        if let Some(s) = Self::load()?.server {
            return canonical_server(&s).context("invalid `server` in config.toml");
        }
        canonical_server(DEFAULT_SERVER)
    }
}

/// Normalise a server URL into the form used as a credential-store key.
///
/// Trailing slashes are stripped and the scheme and host are lowercased, so
/// `https://API.evnx.dev/` and `https://api.evnx.dev` are one entry rather than
/// two half-authenticated ones.
///
/// # Plain HTTP is refused except on the loopback interface
///
/// Every authenticated request carries a bearer token in a header. Over `http://`
/// that token is readable by anything on the path, and a refresh token is valid
/// for thirty days. `http://localhost` and friends are allowed because that is
/// the local development server, and the traffic never leaves the machine.
pub fn canonical_server(raw: &str) -> Result<String> {
    let s = raw.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err(anyhow!("server URL is empty"));
    }

    let (scheme, rest) = s
        .split_once("://")
        .ok_or_else(|| anyhow!("server URL must start with http:// or https:// (got {raw:?})"))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(anyhow!("unsupported URL scheme {scheme:?}; use https://"));
    }
    if rest.is_empty() {
        return Err(anyhow!("server URL has no host (got {raw:?})"));
    }

    // Split host[:port] from any path so the loopback test looks at the host only.
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority
        .rsplit_once(':')
        .map(|(h, _port)| h)
        .unwrap_or(authority)
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();

    if scheme == "http" && !is_loopback(&host) {
        return Err(anyhow!(
            "refusing plain http:// to {host}: the access and refresh tokens travel in \
             request headers. Use https://, or a loopback address for local development."
        ));
    }

    Ok(format!(
        "{scheme}://{}{}",
        authority.to_ascii_lowercase(),
        path
    ))
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host.parse::<std::net::Ipv4Addr>().map(|a| a.is_loopback()) == Ok(true)
        || host.parse::<std::net::Ipv6Addr>().map(|a| a.is_loopback()) == Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testutil::ConfigDirGuard;
    use serial_test::serial;

    #[test]
    fn https_urls_are_canonicalised() {
        assert_eq!(
            canonical_server("https://API.evnx.dev/").unwrap(),
            "https://api.evnx.dev"
        );
        assert_eq!(
            canonical_server("  https://api.evnx.dev  ").unwrap(),
            "https://api.evnx.dev"
        );
    }

    #[test]
    fn a_trailing_slash_does_not_create_a_second_account() {
        // Both spellings must key the same credential entry, or signing in with
        // one and pushing with the other looks like being logged out.
        assert_eq!(
            canonical_server("https://api.evnx.dev/").unwrap(),
            canonical_server("https://api.evnx.dev").unwrap()
        );
    }

    #[test]
    fn plain_http_is_refused_for_remote_hosts() {
        let err = canonical_server("http://api.evnx.dev")
            .unwrap_err()
            .to_string();
        assert!(err.contains("refusing plain http"), "got: {err}");
        assert!(err.contains("headers"), "error should say why: {err}");
    }

    #[test]
    fn plain_http_is_allowed_on_loopback() {
        for url in [
            "http://localhost:8099",
            "http://127.0.0.1:8099",
            "http://[::1]:8099",
        ] {
            assert!(canonical_server(url).is_ok(), "{url} should be allowed");
        }
    }

    #[test]
    fn a_host_merely_starting_with_localhost_is_still_refused() {
        // `localhost.attacker.example` is a real host on the internet.
        assert!(canonical_server("http://localhost.attacker.example").is_err());
    }

    #[test]
    fn malformed_urls_are_rejected() {
        for bad in ["", "   ", "api.evnx.dev", "ftp://api.evnx.dev", "https://"] {
            assert!(canonical_server(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    #[serial]
    fn a_missing_config_file_yields_defaults() {
        let _g = ConfigDirGuard::new();
        assert_eq!(CloudConfig::load().unwrap(), CloudConfig::default());
        assert_eq!(
            CloudConfig::resolve_server(None).unwrap(),
            "https://api.evnx.dev"
        );
    }

    #[test]
    #[serial]
    fn a_malformed_config_file_is_an_error_not_a_silent_default() {
        let g = ConfigDirGuard::new();
        std::fs::write(g.path().join("config.toml"), "server = [1, 2]").unwrap();
        assert!(CloudConfig::load().is_err());
    }

    #[test]
    #[serial]
    fn server_precedence_is_flag_then_env_then_file_then_default() {
        let g = ConfigDirGuard::new();
        std::fs::write(
            g.path().join("config.toml"),
            "server = \"https://file.example\"\n",
        )
        .unwrap();

        assert_eq!(
            CloudConfig::resolve_server(None).unwrap(),
            "https://file.example"
        );

        std::env::set_var(ENV_SERVER, "https://env.example");
        assert_eq!(
            CloudConfig::resolve_server(None).unwrap(),
            "https://env.example"
        );

        assert_eq!(
            CloudConfig::resolve_server(Some("https://flag.example")).unwrap(),
            "https://flag.example"
        );
    }

    #[test]
    #[serial]
    fn config_round_trips_through_disk() {
        let _g = ConfigDirGuard::new();
        let cfg = CloudConfig {
            server: Some("https://self.hosted.example".into()),
        };
        cfg.save().unwrap();
        assert_eq!(CloudConfig::load().unwrap(), cfg);
    }

    #[cfg(unix)]
    #[test]
    #[serial]
    fn the_config_directory_is_created_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let _g = ConfigDirGuard::at(d.path().join("fresh"));
        let dir = ensure_config_dir().unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "got {mode:o}");
    }

    #[cfg(unix)]
    #[test]
    #[serial]
    fn a_world_readable_config_directory_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("loose");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _g = ConfigDirGuard::at(&dir);
        ensure_config_dir().unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "got {mode:o}");
    }
}
