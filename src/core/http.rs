//! The one HTTP client evnx builds, and how a failed request is worded (A-35,
//! DEC-SF9).
//!
//! `update --check` and every `cloud` command built their own
//! `reqwest::blocking::Client` and carried their own copy of the transport
//! wording, which differed only in the timeout it named. One builder is where
//! the user agent, the timeouts and the trusted root certificates (DEC-SF1 = A,
//! RM-68: the bundled webpki roots plus `SSL_CERT_FILE` / `SSL_CERT_DIR`) are
//! set, so a fix lands for every caller at once.
//!
//! `migrate`'s GitHub destination built `Client::new()` per request, with no
//! timeout and the user agent `evnx`. It uses this builder since 0.10.10
//! (DEC-MG3, I-112); `tests/architecture.rs` keeps it the only one.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;

/// Sent with every request. GitHub rejects requests with no user agent.
pub const USER_AGENT: &str = concat!("evnx/", env!("CARGO_PKG_VERSION"));

/// Names a PEM bundle of extra trusted root certificates.
pub const ENV_CERT_FILE: &str = "SSL_CERT_FILE";
/// Names a directory (or a path-separator list of them) of PEM certificates.
pub const ENV_CERT_DIR: &str = "SSL_CERT_DIR";

/// What to do when a certificate is not trusted. Appended to the error, so it
/// is the sentence a person behind a TLS-inspecting proxy needs to read.
pub const UNTRUSTED_ROOT_HINT: &str = "set SSL_CERT_FILE to your organisation's CA bundle (PEM)";

/// `SSL_CERT_FILE` / `SSL_CERT_DIR` named something that cannot be used.
///
/// A typed error so `main` can give it the same exit code as an unreachable
/// server (13) for cloud commands, and so the message always names the variable
/// and the path.
#[derive(Debug, Clone)]
pub struct TlsRootsError {
    /// `SSL_CERT_FILE` or `SSL_CERT_DIR`.
    pub variable: &'static str,
    /// The path the variable named (or the file inside the directory).
    pub path: PathBuf,
    /// What was wrong with it.
    pub reason: String,
}

impl std::fmt::Display for TlsRootsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is set to {}, but {}",
            self.variable,
            self.path.display(),
            self.reason
        )
    }
}

impl std::error::Error for TlsRootsError {}

/// The extra root certificates found in the environment.
#[derive(Debug, Clone, Default)]
pub struct ExtraRoots {
    /// Certificates to trust in addition to the bundled ones.
    pub certs: Vec<reqwest::Certificate>,
    /// Files inside `SSL_CERT_DIR` that were skipped, one line each.
    pub warnings: Vec<String>,
}

/// Parse one PEM file's bytes into certificates. `Err` is the reason.
fn parse_pem(bytes: &[u8]) -> std::result::Result<Vec<reqwest::Certificate>, String> {
    match reqwest::Certificate::from_pem_bundle(bytes) {
        Ok(c) if c.is_empty() => Err("it contains no certificate (expected PEM)".into()),
        Ok(c) => Ok(c),
        Err(e) => Err(format!("it holds no valid PEM certificate ({e})")),
    }
}

/// OpenSSL's directory conventions: `*.pem`, `*.crt`, and hashed names such as
/// `a1b2c3d4.0` (eight hex digits, a dot, a decimal index).
fn is_cert_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".pem") || lower.ends_with(".crt") {
        return true;
    }
    match name.split_once('.') {
        Some((stem, idx)) => {
            stem.len() == 8
                && stem.chars().all(|c| c.is_ascii_hexdigit())
                && !idx.is_empty()
                && idx.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Read the extra roots named by `file` and `dir` (the values of
/// `SSL_CERT_FILE` and `SSL_CERT_DIR`; empty counts as unset).
///
/// The file is strict: missing, unreadable, or without a valid certificate is
/// an error. The directory is lenient about its contents: a file inside it that
/// cannot be read or parsed is a warning, because system directories hold
/// files evnx has no business failing on. A directory that does not exist is an
/// error, as is the file.
pub fn load_extra_roots(
    file: Option<&OsStr>,
    dir: Option<&OsStr>,
) -> std::result::Result<ExtraRoots, TlsRootsError> {
    let mut roots = ExtraRoots::default();

    if let Some(file) = file.filter(|v| !v.is_empty()) {
        let path = PathBuf::from(file);
        let err = |reason: String| TlsRootsError {
            variable: ENV_CERT_FILE,
            path: path.clone(),
            reason,
        };
        let bytes = std::fs::read(&path).map_err(|e| {
            err(match e.kind() {
                std::io::ErrorKind::NotFound => "that file does not exist".to_string(),
                _ => format!("it cannot be read ({e})"),
            })
        })?;
        roots.certs.extend(parse_pem(&bytes).map_err(err)?);
    }

    if let Some(dirs) = dir.filter(|v| !v.is_empty()) {
        for dir in std::env::split_paths(dirs).filter(|p| !p.as_os_str().is_empty()) {
            load_dir(&dir, &mut roots)?;
        }
    }
    Ok(roots)
}

fn load_dir(dir: &Path, roots: &mut ExtraRoots) -> std::result::Result<(), TlsRootsError> {
    let err = |reason: String| TlsRootsError {
        variable: ENV_CERT_DIR,
        path: dir.to_path_buf(),
        reason,
    };
    let entries = std::fs::read_dir(dir).map_err(|e| {
        err(match e.kind() {
            std::io::ErrorKind::NotFound => "that directory does not exist".to_string(),
            _ => format!("it cannot be read as a directory ({e})"),
        })
    })?;
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // `metadata` follows symlinks, which is how `c_rehash` lays hashed names out.
        if !is_cert_file_name(name)
            || !std::fs::metadata(&path)
                .map(|m| m.is_file())
                .unwrap_or(false)
        {
            continue;
        }
        let loaded = std::fs::read(&path)
            .map_err(|e| format!("cannot be read ({e})"))
            .and_then(|b| parse_pem(&b));
        match loaded {
            Ok(certs) => roots.certs.extend(certs),
            Err(reason) => roots.warnings.push(format!(
                "ignoring {} (from {}): {reason}",
                path.display(),
                ENV_CERT_DIR
            )),
        }
    }
    Ok(())
}

/// The extra roots from this process's environment, loaded once, at first use.
///
/// The failure is cached too, so every client built in a run reports the same
/// thing. Warnings are printed to stderr once, here.
pub fn extra_roots() -> std::result::Result<&'static ExtraRoots, TlsRootsError> {
    static ROOTS: OnceLock<std::result::Result<ExtraRoots, TlsRootsError>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let loaded = load_extra_roots(
                std::env::var_os(ENV_CERT_FILE).as_deref(),
                std::env::var_os(ENV_CERT_DIR).as_deref(),
            );
            if let Ok(r) = &loaded {
                for w in &r.warnings {
                    eprintln!("warning: {w}");
                }
            }
            loaded
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// A blocking client with evnx's user agent, an overall `timeout`, and, when
/// given, a separate `connect_timeout`.
///
/// Trusts the bundled webpki roots plus whatever `SSL_CERT_FILE` and
/// `SSL_CERT_DIR` name (see [`extra_roots`]); the extras never replace the
/// bundle. A bad variable is an error here, at first use, not at startup.
pub fn client(
    timeout: Duration,
    connect_timeout: Option<Duration>,
) -> Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(USER_AGENT);
    if let Some(connect) = connect_timeout {
        builder = builder.connect_timeout(connect);
    }
    for cert in &extra_roots()?.certs {
        builder = builder.add_root_certificate(cert.clone());
    }
    Ok(builder.build()?)
}

/// Whether an error message is rustls saying the issuer is not trusted.
fn is_untrusted_issuer(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("unknownissuer") || t.contains("unknown issuer") || t.contains("untrusted root")
}

/// `root` with the fix appended when `chain` is an unknown-issuer failure.
fn with_tls_hint(chain: &str, root: String) -> String {
    if is_untrusted_issuer(chain) {
        format!("{root} (the server's certificate is not signed by a trusted authority; {UNTRUSTED_ROOT_HINT})")
    } else {
        root
    }
}

/// Shortest useful description of a request that never completed.
///
/// `reqwest::Error`'s own `Display` is "error sending request for url (...)",
/// which repeats the URL the caller already knows and says nothing about the
/// cause. The cause sits at the end of a chain through hyper down to the OS
/// error, so this walks to the root and reports that instead — "Connection
/// refused (os error 111)" rather than four wrappers around it.
///
/// Timeouts are special-cased because their root cause is an opaque internal
/// type, and because "timed out" is what the user needs to read; `timeout` is
/// the limit the caller set.
///
/// An unknown-issuer certificate error gets the fix appended
/// ([`UNTRUSTED_ROOT_HINT`]): behind a TLS-inspecting proxy that is the usual
/// cause.
pub fn transport_reason(e: &reqwest::Error, timeout: Duration) -> String {
    if e.is_timeout() {
        return format!("timed out after {}s", timeout.as_secs());
    }
    let mut chain = String::new();
    let mut root: &dyn std::error::Error = e;
    while let Some(next) = root.source() {
        chain.push_str(&next.to_string());
        chain.push('\n');
        root = next;
    }
    with_tls_hint(&chain, root.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refused connection reads as one line naming the OS error, not a
    /// chain through reqwest and hyper.
    #[test]
    fn a_refused_connection_is_one_line() {
        let timeout = Duration::from_secs(5);
        let c = client(timeout, Some(Duration::from_secs(2))).unwrap();
        let err = c.get("http://127.0.0.1:9/").send().unwrap_err();
        let reason = transport_reason(&err, timeout);
        assert_eq!(reason.lines().count(), 1, "{reason}");
        assert!(!reason.is_empty());
    }

    const CA_A: &str = include_str!("../../tests/fixtures/tls/ca-a.pem");
    const CA_B: &str = include_str!("../../tests/fixtures/tls/ca-b.pem");

    fn os(p: &Path) -> Option<&OsStr> {
        Some(p.as_os_str())
    }

    #[test]
    fn a_bundle_with_two_certificates_loads_both() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("bundle.pem");
        std::fs::write(&f, format!("{CA_A}\n{CA_B}")).unwrap();
        let r = load_extra_roots(os(&f), None).unwrap();
        assert_eq!(r.certs.len(), 2);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn a_directory_takes_pem_crt_and_hashed_names_and_skips_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let w = |n: &str, c: &str| std::fs::write(d.path().join(n), c).unwrap();
        w("a.pem", CA_A);
        w("b.crt", CA_B);
        w("abcd1234.0", CA_A);
        w("abcd1234.1", CA_B);
        w("README.txt", "not a certificate");
        w("notes", CA_A); // wrong name: ignored, not even a warning
        w("broken.pem", "garbage"); // right name, bad content: a warning
        std::fs::create_dir(d.path().join("sub.pem")).unwrap();
        let r = load_extra_roots(None, os(d.path())).unwrap();
        assert_eq!(r.certs.len(), 4);
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
        assert!(r.warnings[0].contains("broken.pem") && r.warnings[0].contains("SSL_CERT_DIR"));
    }

    #[test]
    fn file_and_dir_combine() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("one.pem");
        std::fs::write(&f, CA_A).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("two.pem"), CA_B).unwrap();
        let r = load_extra_roots(os(&f), os(dir.path())).unwrap();
        assert_eq!(r.certs.len(), 2);
    }

    #[test]
    fn empty_or_invalid_file_is_an_error_naming_the_variable_and_path() {
        let d = tempfile::tempdir().unwrap();
        for (name, content) in [
            ("empty.pem", ""),
            ("junk.pem", "hello"),
            (
                "half.pem",
                "-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n",
            ),
        ] {
            let f = d.path().join(name);
            std::fs::write(&f, content).unwrap();
            let e = load_extra_roots(os(&f), None).unwrap_err();
            let msg = e.to_string();
            assert!(msg.contains("SSL_CERT_FILE"), "{msg}");
            assert!(msg.contains(name), "{msg}");
        }
    }

    #[test]
    fn a_missing_file_or_directory_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("gone");
        let e = load_extra_roots(os(&gone), None).unwrap_err().to_string();
        assert!(
            e.contains("SSL_CERT_FILE") && e.contains("gone") && e.contains("does not exist"),
            "{e}"
        );
        let e = load_extra_roots(None, os(&gone)).unwrap_err().to_string();
        assert!(
            e.contains("SSL_CERT_DIR") && e.contains("gone") && e.contains("does not exist"),
            "{e}"
        );
    }

    #[test]
    fn empty_values_count_as_unset() {
        let r = load_extra_roots(Some(OsStr::new("")), Some(OsStr::new(""))).unwrap();
        assert!(r.certs.is_empty());
    }

    #[test]
    fn hashed_name_rule() {
        assert!(is_cert_file_name("abcd1234.0"));
        assert!(is_cert_file_name("ABCD1234.12"));
        assert!(is_cert_file_name("x.PEM"));
        assert!(!is_cert_file_name("abcd123.0"));
        assert!(!is_cert_file_name("abcd1234.r0"));
        assert!(!is_cert_file_name("abcd1234"));
    }

    /// The hint appears for an unknown issuer and for nothing else.
    #[test]
    fn unknown_issuer_names_the_fix() {
        let chain = "client error (Connect)\ninvalid peer certificate: UnknownIssuer\n";
        let out = with_tls_hint(chain, "invalid peer certificate: UnknownIssuer".into());
        assert!(
            out.contains("set SSL_CERT_FILE to your organisation's CA bundle (PEM)"),
            "{out}"
        );
        let other = with_tls_hint(
            "connection refused",
            "Connection refused (os error 111)".into(),
        );
        assert_eq!(other, "Connection refused (os error 111)");
    }

    /// Extra roots are accepted by the client builder on top of the bundle.
    #[test]
    fn extra_roots_build_into_a_client() {
        let certs = reqwest::Certificate::from_pem_bundle(CA_A.as_bytes()).unwrap();
        let mut b = reqwest::blocking::Client::builder();
        for c in certs {
            b = b.add_root_certificate(c);
        }
        b.build().unwrap();
    }
}
