//! What this binary was built with, and the one cargo command evnx tells
//! anyone to run (A-36, DEC-SF2, DEC-SF11).
//!
//! `cloud`, `migrate`, `backup` and `net` are optional, so two binaries of the
//! same version can do different things. `evnx commands` reports the set, and
//! every message that says how to build evnx again names the same command.
//! Five copies of that command carried three flag sets, and the upgrade one
//! (`--features cloud`) quietly dropped `migrate` and `backup` from anyone who
//! had installed with `--all-features` (I-199).

/// The cargo command that installs evnx with every feature, as the prebuilt
/// binaries are built.
///
/// ⚠️ `tests/architecture.rs` fails on any other `cargo install evnx --…`
/// in `src/`. Help text that clap needs as a literal spells it out in full.
pub const CARGO_INSTALL: &str = "cargo install evnx --all-features";

/// [`CARGO_INSTALL`] over an existing install: `--force`, because cargo
/// refuses to reinstall a crate it already has.
pub const CARGO_UPGRADE: &str = "cargo install evnx --all-features --force";

/// The one install URL (WP-16): the install script, served by evnx.dev (the
/// old dotenv.space address redirects to it). `evnx update`, the scan hook and
/// every doc say this spelling; `tests/wp16_install_docs.rs` fails on another.
pub const INSTALL_URL: &str = "https://evnx.dev/install.sh";

/// The command `evnx update` prints for a copy the install script put there.
pub const INSTALL_COMMAND: &str = "curl -fsSL https://evnx.dev/install.sh | bash";

/// Which optional features this binary was built with.
///
/// ⚠️ Read with `cfg!`, not from a list someone maintains: the point of
/// reporting it is to stop keeping a second copy of something the compiler
/// already knows.
pub fn features() -> Vec<&'static str> {
    let mut f = Vec::new();
    // Alphabetical: `run.features` in every JSON output is the same list.
    if cfg!(feature = "backup") {
        f.push("backup");
    }
    if cfg!(feature = "cloud") {
        f.push("cloud");
    }
    if cfg!(feature = "migrate") {
        f.push("migrate");
    }
    if cfg!(feature = "net") {
        f.push("net");
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The feature list is the difference between "this build has no `org`" and
    /// "the docs are wrong". A consumer that cannot see it will conclude the
    /// latter.
    #[test]
    fn the_feature_list_matches_what_was_compiled() {
        let f = features();
        assert_eq!(f.contains(&"net"), cfg!(feature = "net"));
        assert_eq!(f.contains(&"cloud"), cfg!(feature = "cloud"));
        assert_eq!(f.contains(&"backup"), cfg!(feature = "backup"));
        assert_eq!(f.contains(&"migrate"), cfg!(feature = "migrate"));
    }

    /// WP-16: the printed command and the URL are one spelling.
    #[test]
    fn wp16_install_command_is_built_on_the_install_url() {
        assert_eq!(
            INSTALL_COMMAND,
            format!("curl -fsSL {INSTALL_URL} | bash").as_str()
        );
    }

    #[test]
    fn the_upgrade_is_the_install_forced() {
        assert_eq!(CARGO_UPGRADE, format!("{CARGO_INSTALL} --force"));
    }
}
