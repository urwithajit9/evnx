//! Which channel installed the running binary, inferred from its own path.
//!
//! # Why the path, and not something recorded at build time
//!
//! A build-time marker would be exact, but there is nothing to put it in: all nine
//! channels wrap the **same** artefacts. `release.yml` builds one binary per target
//! and Homebrew, Scoop, winget, npm and PyPI all package that identical file, so a
//! constant compiled into it cannot distinguish them. The install *location* is the
//! only thing that differs, and it is available at runtime for free.
//!
//! # This is a heuristic, and it says so when it fails
//!
//! ⚠️ Every branch below can be wrong. A user may symlink the binary anywhere;
//! `current_exe` resolves symlinks on Linux (via `/proc/self/exe`) but is not
//! guaranteed to on macOS; and `/usr/local/bin` is both where the install script
//! puts it *and* where Homebrew on Intel macOS links it.
//!
//! So an unrecognised path is [`Channel::Unknown`], which prints every channel's
//! command and asks the reader to pick. **Guessing would be worse than admitting
//! the gap** — a confidently wrong upgrade instruction can uninstall a working
//! binary, and `brew uninstall` on something Homebrew never installed fails in
//! ways that are not obvious to read.

use std::path::{Path, PathBuf};

/// How evnx was most likely installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Cargo,
    Homebrew,
    Scoop,
    Winget,
    Npm,
    PyPI,
    /// `install.sh`, or a tarball unpacked by hand.
    Script,
    /// Nothing matched. Not a failure — see the module docs.
    Unknown,
}

impl Channel {
    /// Every channel's upgrade command, for when detection cannot narrow it down.
    pub const ALL_COMMANDS: &'static [(&'static str, &'static str)] = &[
        ("cargo", "cargo install evnx --features cloud --force"),
        ("Homebrew", "brew upgrade evnx"),
        ("Scoop", "scoop update evnx"),
        ("winget", "winget upgrade urwithajit9.evnx"),
        ("npm", "npm install -g @evnx/cli@latest"),
        ("pip", "pip install --upgrade evnx"),
        (
            "script",
            "curl -fsSL https://dotenv.space/install.sh | bash",
        ),
    ];

    /// Infer the channel from the running executable's path.
    ///
    /// ⚠️ **Canonicalised first, and that is the whole reason Homebrew on Intel
    /// macOS is detected at all.** Homebrew's real binary lives under `Cellar` and is
    /// *symlinked* into `/usr/local/bin`, which is also where `install.sh` writes.
    /// `std::env::current_exe` is not guaranteed to resolve symlinks — on Linux it
    /// reads `/proc/self/exe` and does, on macOS it may hand back the link — so
    /// without this the two are the same string and Homebrew users would be told to
    /// re-run a curl installer over a managed binary.
    ///
    /// Falls back to the unresolved path if canonicalisation fails, since a detected
    /// channel is better than none and the worst case is [`Channel::Unknown`].
    ///
    /// ⚠️ **Unverified on macOS, and it is only macOS this is for.** Checked by hand
    /// on Linux: invoking through a symlink at `usr/local/bin/evnx` reports Homebrew
    /// **with or without** this call, because `/proc/self/exe` has already resolved
    /// it. So the canonicalise is insurance for a platform not available here. The
    /// property it provides is asserted directly by
    /// `a_symlink_is_resolved_to_its_target_before_matching`, which checks both
    /// halves — the link reads as `Script`, the target as `Homebrew` — and that test
    /// is platform-independent.
    pub fn detect() -> Self {
        let Ok(exe) = std::env::current_exe() else {
            return Self::Unknown;
        };
        let resolved = std::fs::canonicalize(&exe).unwrap_or(exe);
        Self::from_path(&resolved)
    }

    /// The detection itself, over a path rather than the process, so it is testable.
    ///
    /// Order is most specific first. `site-packages` is checked before the generic
    /// `bin` directories because a PyPI install lands inside one.
    pub fn from_path(path: &Path) -> Self {
        // Compare with `/` separators and lowercased, so one set of patterns covers
        // Windows and Unix and `C:\Users\Ajit\scoop\` matches `/scoop/`.
        let p = path.to_string_lossy().replace('\\', "/").to_lowercase();

        // Windows package managers first: their paths are unmistakable.
        if p.contains("/winget/packages/") || p.contains("/microsoft/winget/") {
            return Self::Winget;
        }
        if p.contains("/scoop/") {
            return Self::Scoop;
        }
        // `/cellar/` rather than `/usr/local/bin`, which Homebrew only symlinks into
        // and shares with the install script. ⚠️ The order relative to the generic
        // directories below does **not** matter — `/usr/local/Cellar/…` never
        // contains `/usr/local/bin/` — so what makes this work is `detect`
        // canonicalising the path, not the position of this branch. An earlier
        // comment here claimed the ordering was load-bearing; sabotaging the order
        // changed no test, which is how that was caught.
        if p.contains("/cellar/") || p.contains("/homebrew/") || p.contains("/linuxbrew/") {
            return Self::Homebrew;
        }
        if p.contains("/.cargo/bin/") || p.contains("/cargo/bin/") {
            return Self::Cargo;
        }
        if p.contains("/node_modules/") || p.contains("/npm/") || p.contains("/.npm-global/") {
            return Self::Npm;
        }
        if p.contains("/site-packages/") || p.contains("/pipx/") || p.contains("/.venv/") {
            return Self::PyPI;
        }
        // Last, because these are the least distinctive paths there are.
        if p.contains("/usr/local/bin/") || p.contains("/.local/bin/") {
            return Self::Script;
        }

        Self::Unknown
    }

    /// What to call this channel in output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Homebrew => "Homebrew",
            Self::Scoop => "Scoop",
            Self::Winget => "winget",
            Self::Npm => "npm",
            Self::PyPI => "pip",
            Self::Script => "the install script",
            Self::Unknown => "an unrecognised location",
        }
    }

    /// The command that upgrades this installation, when one can be named.
    pub fn upgrade_command(self) -> Option<&'static str> {
        let label = match self {
            Self::Cargo => "cargo",
            Self::Homebrew => "Homebrew",
            Self::Scoop => "Scoop",
            Self::Winget => "winget",
            Self::Npm => "npm",
            Self::PyPI => "pip",
            Self::Script => "script",
            Self::Unknown => return None,
        };
        Self::ALL_COMMANDS
            .iter()
            .find(|(l, _)| *l == label)
            .map(|(_, c)| *c)
    }
}

/// Where a `PathBuf` is wanted rather than a `&Path`.
impl From<PathBuf> for Channel {
    fn from(path: PathBuf) -> Self {
        Self::from_path(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn detect(p: &str) -> Channel {
        Channel::from_path(&PathBuf::from(p))
    }

    #[test]
    fn each_channel_is_recognised_from_a_real_install_path() {
        for (path, want) in [
            // Unix
            ("/home/ajit/.cargo/bin/evnx", Channel::Cargo),
            (
                "/opt/homebrew/Cellar/evnx/0.6.0/bin/evnx",
                Channel::Homebrew,
            ),
            ("/usr/local/Cellar/evnx/0.6.0/bin/evnx", Channel::Homebrew),
            ("/home/linuxbrew/.linuxbrew/bin/evnx", Channel::Homebrew),
            ("/usr/lib/node_modules/@evnx/cli/bin/evnx", Channel::Npm),
            (
                "/usr/lib/python3.12/site-packages/evnx/bin/evnx",
                Channel::PyPI,
            ),
            ("/home/ajit/.local/pipx/venvs/evnx/bin/evnx", Channel::PyPI),
            ("/usr/local/bin/evnx", Channel::Script),
            ("/home/ajit/.local/bin/evnx", Channel::Script),
            // Windows, with backslashes and mixed case
            (
                r"C:\Users\Ajit\scoop\apps\evnx\current\evnx.exe",
                Channel::Scoop,
            ),
            (
                r"C:\Users\Ajit\AppData\Local\Microsoft\WinGet\Packages\urwithajit9.evnx\evnx.exe",
                Channel::Winget,
            ),
            (
                r"C:\Program Files\nodejs\node_modules\@evnx\cli\evnx.exe",
                Channel::Npm,
            ),
        ] {
            assert_eq!(detect(path), want, "{path}");
        }
    }

    /// ⚠️ The ambiguity that canonicalising resolves, tested on a real symlink.
    ///
    /// Homebrew's binary lives under `Cellar` and is symlinked into
    /// `/usr/local/bin`, which is also where `install.sh` writes — so the **link's**
    /// path says "install script" and the **target's** says "Homebrew". Telling an
    /// Intel-macOS Homebrew user to re-run a curl installer over a managed binary is
    /// the failure this prevents.
    ///
    /// This asserts the property directly rather than trusting branch order: an
    /// earlier version of this test claimed to pin the ordering and passed with the
    /// ordering reversed, because `/usr/local/Cellar/…` never contains
    /// `/usr/local/bin/` in the first place.
    #[test]
    fn a_symlink_is_resolved_to_its_target_before_matching() {
        let dir = tempfile::TempDir::new().unwrap();
        let cellar = dir.path().join("usr/local/Cellar/evnx/0.6.0/bin");
        let binv = dir.path().join("usr/local/bin");
        std::fs::create_dir_all(&cellar).unwrap();
        std::fs::create_dir_all(&binv).unwrap();
        let target = cellar.join("evnx");
        std::fs::write(&target, b"#!/bin/sh\n").unwrap();
        let link = binv.join("evnx");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(&target, &link).unwrap();

        // The link's own path reads as a script install …
        assert_eq!(Channel::from_path(&link), Channel::Script);

        // … and resolving it, which `detect` does, reads as Homebrew.
        #[cfg(unix)]
        {
            let resolved = std::fs::canonicalize(&link).unwrap();
            assert_eq!(
                Channel::from_path(&resolved),
                Channel::Homebrew,
                "canonicalised: {}",
                resolved.display()
            );
        }
    }

    #[test]
    fn a_pipx_venv_is_not_read_as_the_install_script() {
        // `.local/pipx/…` and `.local/bin/…` are different directories, so this is a
        // mapping assertion rather than an ordering one.
        assert_eq!(
            detect("/home/ajit/.local/pipx/venvs/evnx/bin/evnx"),
            Channel::PyPI
        );
        assert_eq!(detect("/home/ajit/.local/bin/evnx"), Channel::Script);
    }

    #[test]
    fn an_unrecognised_path_is_unknown_rather_than_a_guess() {
        for path in [
            "/tmp/evnx",
            "/home/ajit/Downloads/evnx",
            "/opt/evnx/evnx",
            "./target/debug/evnx",
        ] {
            assert_eq!(detect(path), Channel::Unknown, "{path}");
        }
        assert_eq!(Channel::Unknown.upgrade_command(), None);
    }

    /// Every channel that claims a command must actually have one, and the labels
    /// have to agree — the lookup is by string, so a typo would silently yield
    /// `None` and send the user to the full list instead of their own command.
    #[test]
    fn every_named_channel_resolves_to_a_command() {
        for channel in [
            Channel::Cargo,
            Channel::Homebrew,
            Channel::Scoop,
            Channel::Winget,
            Channel::Npm,
            Channel::PyPI,
            Channel::Script,
        ] {
            assert!(
                channel.upgrade_command().is_some(),
                "{channel:?} names no command — check its label against ALL_COMMANDS"
            );
        }
        assert_eq!(
            Channel::ALL_COMMANDS.len(),
            7,
            "a channel was added without a command, or vice versa"
        );
    }

    /// ⚠️ `cargo install` compiles with `default = []`, so a plain upgrade would
    /// silently drop the cloud commands. The suggested command has to carry the
    /// feature, and `--force` because the version is already installed.
    #[test]
    fn the_cargo_command_keeps_the_cloud_feature_and_forces() {
        let cmd = Channel::Cargo.upgrade_command().unwrap();
        assert!(cmd.contains("--features cloud"), "{cmd}");
        assert!(cmd.contains("--force"), "{cmd}");
    }
}
