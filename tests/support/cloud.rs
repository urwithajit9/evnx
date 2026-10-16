//! Helpers for cloud command tests: a stored login and an evnx run against a
//! mock server (U7).

use std::path::Path;

use evnx::cloud::creds::{SecretString, Session, Store};

use super::{run_with, Output, Project};

/// Store a login for `server` in `cfg/credentials.json`, as `evnx auth login`
/// would, with an access token that is not near expiry.
pub fn login(cfg: &Path, server: &str) {
    std::fs::create_dir_all(cfg).unwrap();
    let mut store = Store::default();
    store.set_session(
        server,
        Session {
            email: "me@example.com".into(),
            access_token: SecretString::new("access"),
            access_expires_at: 4_000_000_000,
            refresh_token: SecretString::new("refresh"),
        },
    );
    store.save_to(&cfg.join("credentials.json")).unwrap();
}

/// `evnx <args>` in `p` against `server`, signed in, with `stdin` if given.
pub fn signed_in(p: &Project, server: &str, args: &[&str], stdin: Option<&[u8]>) -> Output {
    let cfg = p.path().join("cfg");
    login(&cfg, server);
    run_with(
        p.path(),
        args,
        &[
            ("EVNX_SERVER", server),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
        stdin,
    )
}
