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
//!
//! # pip, and the folders it shares with the install script
//!
//! ⚠️ A wheel ships the binary as `bin` data (`bindings = "bin"` in
//! `pyproject.toml`), so pip puts it in the environment's plain `bin/` —
//! `~/.local/bin` with `--user`, `/usr/local/bin` for a system install,
//! `Scripts\` on Windows — never under `site-packages`. The first two are where
//! `install.sh` writes, so a pip user was told to run the curl installer over a
//! pip-managed binary (I-193). The path cannot tell them apart; the files pip
//! leaves can: a virtual environment has `pyvenv.cfg` one level above `bin/`,
//! and every pip install records itself in an `evnx-<version>.dist-info` folder
//! under the prefix's `site-packages`. With neither, those two folders get
//! [`Channel::ScriptOrPip`] and both commands, not a guess (DEC-SF3).

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
    /// `~/.local/bin` or `/usr/local/bin` with no sign of pip: `install.sh`
    /// writes there, and so does pip without leaving a record evnx can find.
    ScriptOrPip,
    /// Nothing matched. Not a failure — see the module docs.
    Unknown,
}

impl Channel {
    /// Every channel's upgrade command, for when detection cannot narrow it down.
    pub const ALL_COMMANDS: &'static [(&'static str, &'static str)] = &[
        ("cargo", crate::core::build::CARGO_UPGRADE),
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
    /// halves — the link reads as `ScriptOrPip`, the target as `Homebrew` — and
    /// that test is platform-independent.
    pub fn detect() -> Self {
        let Ok(exe) = std::env::current_exe() else {
            return Self::Unknown;
        };
        let resolved = std::fs::canonicalize(&exe).unwrap_or(exe);
        Self::from_install(&resolved)
    }

    /// [`Channel::from_path`], after looking for what pip leaves beside the
    /// binary (see the module docs). Evidence on disk wins over a path pattern.
    pub fn from_install(exe: &Path) -> Self {
        if pip_installed(exe) {
            return Self::PyPI;
        }
        Self::from_path(exe)
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
        // Last, because these are the least distinctive paths there are, and
        // shared by two channels: see the module docs.
        if p.contains("/usr/local/bin/") || p.contains("/.local/bin/") {
            return Self::ScriptOrPip;
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
            Self::ScriptOrPip => "the install script or pip",
            Self::Unknown => "an unrecognised location",
        }
    }

    /// A stable name for `update --check --json`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Homebrew => "homebrew",
            Self::Scoop => "scoop",
            Self::Winget => "winget",
            Self::Npm => "npm",
            Self::PyPI => "pip",
            Self::ScriptOrPip => "script-or-pip",
            Self::Unknown => "unknown",
        }
    }

    /// The upgrade commands that fit, labelled as in [`Channel::ALL_COMMANDS`]:
    /// one for a channel evnx recognised, two where the install script and pip
    /// share a folder, every one when it could not tell.
    pub fn candidates(self) -> Vec<(&'static str, &'static str)> {
        let labels: &[&str] = match self {
            Self::Cargo => &["cargo"],
            Self::Homebrew => &["Homebrew"],
            Self::Scoop => &["Scoop"],
            Self::Winget => &["winget"],
            Self::Npm => &["npm"],
            Self::PyPI => &["pip"],
            Self::ScriptOrPip => &["pip", "script"],
            Self::Unknown => return Self::ALL_COMMANDS.to_vec(),
        };
        Self::ALL_COMMANDS
            .iter()
            .filter(|(l, _)| labels.contains(l))
            .copied()
            .collect()
    }

    /// The command that upgrades this installation, when exactly one can be
    /// named.
    pub fn upgrade_command(self) -> Option<&'static str> {
        match self.candidates()[..] {
            [(_, command)] => Some(command),
            _ => None,
        }
    }
}

/// Whether pip installed the binary at `exe`: a virtual environment's
/// `pyvenv.cfg` one level above its folder, or an `evnx-*.dist-info` record in
/// that prefix's `site-packages`.
fn pip_installed(exe: &Path) -> bool {
    let Some(prefix) = exe.parent().and_then(Path::parent) else {
        return false;
    };
    prefix.join("pyvenv.cfg").is_file() || site_packages(prefix).iter().any(|d| has_record(d))
}

/// Where pip records a package under `prefix`: `lib/python3.X/site-packages`
/// (Debian's system pip: `dist-packages`; macOS user installs: `lib/python`),
/// `Lib\site-packages` on Windows, and `site-packages` itself for a Windows
/// `--user` prefix (`%APPDATA%\Python\Python312`).
fn site_packages(prefix: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        prefix.join("Lib").join("site-packages"),
        prefix.join("site-packages"),
    ];
    if let Ok(entries) = std::fs::read_dir(prefix.join("lib")) {
        for e in entries.flatten() {
            if e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .starts_with("python")
            {
                dirs.push(e.path().join("site-packages"));
                dirs.push(e.path().join("dist-packages"));
            }
        }
    }
    dirs
}

/// An `evnx-<version>.dist-info` folder in `dir`.
fn has_record(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name.starts_with("evnx-") && name.ends_with(".dist-info")
        })
    })
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
            // Shared by install.sh and pip: see `pip_evidence_decides_a_shared_folder`.
            ("/usr/local/bin/evnx", Channel::ScriptOrPip),
            ("/home/ajit/.local/bin/evnx", Channel::ScriptOrPip),
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

        // The link's own path reads as the folder install.sh shares with pip …
        assert_eq!(Channel::from_path(&link), Channel::ScriptOrPip);

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
        assert_eq!(detect("/home/ajit/.local/bin/evnx"), Channel::ScriptOrPip);
    }

    /// I-193, DEC-SF3: what pip leaves beside the binary decides a shared
    /// folder; without it, both commands.
    #[test]
    fn pip_evidence_decides_a_shared_folder() {
        let make = |bin: &str, files: &[&str]| {
            let root = tempfile::TempDir::new().unwrap();
            let exe = root.path().join(bin).join("evnx");
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"").unwrap();
            for f in files {
                let path = root.path().join(f);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, b"").unwrap();
            }
            (root, exe)
        };
        for (bin, files, want) in [
            (".local/bin", &[][..], Channel::ScriptOrPip),
            (
                ".local/bin",
                &[".local/lib/python3.12/site-packages/evnx-0.10.5.dist-info/RECORD"][..],
                Channel::PyPI,
            ),
            (
                "usr/local/bin",
                &["usr/local/lib/python3.11/dist-packages/evnx-0.10.5.dist-info/RECORD"][..],
                Channel::PyPI,
            ),
            // Another package's record is not evidence for evnx.
            (
                "usr/local/bin",
                &["usr/local/lib/python3.11/dist-packages/evnxtra-1.0.dist-info/RECORD"][..],
                Channel::ScriptOrPip,
            ),
            ("venv/bin", &["venv/pyvenv.cfg"][..], Channel::PyPI),
            ("env/Scripts", &["env/pyvenv.cfg"][..], Channel::PyPI),
            (
                "Python312/Scripts",
                &["Python312/site-packages/evnx-0.10.5.dist-info/RECORD"][..],
                Channel::PyPI,
            ),
            ("venv/bin", &[][..], Channel::Unknown),
        ] {
            let (_root, exe) = make(bin, files);
            assert_eq!(Channel::from_install(&exe), want, "{bin} {files:?}");
        }
    }

    /// DEC-SF3: a shared folder offers both commands and names neither alone.
    #[test]
    fn a_shared_folder_offers_the_script_and_pip() {
        let labels: Vec<&str> = Channel::ScriptOrPip
            .candidates()
            .iter()
            .map(|(l, _)| *l)
            .collect();
        assert_eq!(labels, ["pip", "script"]);
        assert_eq!(Channel::ScriptOrPip.upgrade_command(), None);
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
    /// silently drop features. `--features cloud` dropped `migrate` and `backup`
    /// from an `--all-features` install (I-199): the command builds every
    /// feature (DEC-SF2), and `--force` because the version is already
    /// installed.
    #[test]
    fn t_i199_the_cargo_command_builds_every_feature_and_forces() {
        let cmd = Channel::Cargo.upgrade_command().unwrap();
        assert_eq!(cmd, "cargo install evnx --all-features --force");
    }
}
