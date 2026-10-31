//! Every copy of evnx on this machine (RM-42, D7, D8).
//!
//! The owner's machines ran an old copy for weeks because a newer install
//! landed behind it on `PATH` (an npm 0.10.0 ahead of cargo 0.10.15). `update`
//! could only describe the copy that ran. This lists all of them: `PATH` in
//! order, then the folders installers use that may not be on it.
//!
//! # Versions, without guessing (D7)
//!
//! Read from what the installer left first: Homebrew's `Cellar/evnx/<v>`, pip's
//! `evnx-<v>.dist-info`, npm's `package.json`, the install script's receipt.
//! Only when none says, the copy is run with `--version` and nothing else, and
//! killed after two seconds. The output says which versions came from running.
//!
//! Nothing here touches the network.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::channel::{self, Channel};

/// One `evnx` binary.
#[derive(Debug)]
pub struct Copy {
    /// Where it was found, as `PATH` or the folder names it.
    pub path: PathBuf,
    /// After symlinks: two paths to one file are one copy.
    pub resolved: PathBuf,
    pub channel: Channel,
    pub version: Option<String>,
    pub version_from: VersionFrom,
    /// First on `PATH`: what typing `evnx` runs.
    pub runs: bool,
    /// The binary running this command.
    pub this: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionFrom {
    /// This binary's own version.
    Itself,
    /// What the installer left beside it.
    Metadata,
    /// `<copy> --version` (D7).
    Ran,
    Unknown,
}

impl VersionFrom {
    pub fn id(self) -> &'static str {
        match self {
            Self::Itself => "self",
            Self::Metadata => "metadata",
            Self::Ran => "ran",
            Self::Unknown => "unknown",
        }
    }
}

const RUN_TIMEOUT: Duration = Duration::from_secs(2);

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "evnx.exe"
    } else {
        "evnx"
    }
}

/// Folders installers write to that may not be on `PATH`.
fn known_folders() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".cargo").join("bin"));
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".npm-global").join("bin"));
        // pipx's venv, and the link it puts in `~/.local/bin`.
        dirs.push(
            home.join(".local")
                .join("share")
                .join("pipx")
                .join("venvs")
                .join("evnx")
                .join("bin"),
        );
    }
    // npm's global prefix, without spawning npm.
    if let Some(prefix) = std::env::var_os("npm_config_prefix").or_else(npmrc_prefix) {
        dirs.push(PathBuf::from(prefix).join("bin"));
    }
    if cfg!(unix) {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
        dirs.push("/home/linuxbrew/.linuxbrew/bin".into());
    }
    dirs
}

/// `prefix=` from `~/.npmrc`.
fn npmrc_prefix() -> Option<std::ffi::OsString> {
    let text = std::fs::read_to_string(dirs::home_dir()?.join(".npmrc")).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "prefix").then(|| v.trim().into())
    })
}

/// Every copy, `PATH` first, without versions: cheap enough for one line in
/// plain `update`.
pub fn find() -> Vec<Copy> {
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    find_in(&path_dirs, &known_folders())
}

fn find_in(path_dirs: &[PathBuf], extra: &[PathBuf]) -> Vec<Copy> {
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::canonicalize(p).ok());
    let mut copies: Vec<Copy> = Vec::new();
    let candidates = path_dirs
        .iter()
        .map(|d| (d, true))
        .chain(extra.iter().map(|d| (d, false)));
    for (dir, on_path) in candidates {
        let path = dir.join(exe_name());
        if !path.is_file() {
            continue;
        }
        let resolved = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if copies.iter().any(|c| c.resolved == resolved) {
            continue;
        }
        let runs = on_path && !copies.iter().any(|c| c.runs);
        copies.push(Copy {
            channel: Channel::from_install(&resolved),
            this: me.as_deref() == Some(resolved.as_path()),
            path,
            resolved,
            version: None,
            version_from: VersionFrom::Unknown,
            runs,
        });
    }
    copies
}

/// Fill in each copy's version (D7).
pub fn read_versions(copies: &mut [Copy]) {
    for c in copies {
        let (version, from) = if c.this {
            (
                Some(env!("CARGO_PKG_VERSION").to_string()),
                VersionFrom::Itself,
            )
        } else if let Some(v) = from_metadata(&c.resolved) {
            (Some(v), VersionFrom::Metadata)
        } else if let Some(v) = run_version(&c.resolved, RUN_TIMEOUT) {
            (Some(v), VersionFrom::Ran)
        } else {
            (None, VersionFrom::Unknown)
        };
        c.version = version;
        c.version_from = from;
    }
}

fn from_metadata(exe: &Path) -> Option<String> {
    // Homebrew: `…/Cellar/evnx/<version>/bin/evnx`.
    let parts: Vec<_> = exe.components().map(|c| c.as_os_str()).collect();
    if let Some(i) = parts.iter().position(|p| *p == "Cellar") {
        if parts.get(i + 1).is_some_and(|p| *p == "evnx") {
            return parts.get(i + 2).map(|v| v.to_string_lossy().into_owned());
        }
    }
    if let Some(v) = channel::pip_version(exe) {
        return Some(v);
    }
    if let Some(v) = channel::receipt_version(exe) {
        return Some(v);
    }
    npm_version(exe)
}

/// The `version` of the nearest `package.json` named `@evnx/…` above `exe`.
fn npm_version(exe: &Path) -> Option<String> {
    exe.ancestors().skip(1).take(5).find_map(|dir| {
        let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        json["name"]
            .as_str()?
            .starts_with("@evnx/")
            .then(|| json["version"].as_str().map(str::to_string))?
    })
}

/// `<exe> --version`, killed after `timeout`. Only the version is read.
fn run_version(exe: &Path, timeout: Duration) -> Option<String> {
    let spawn = || {
        Command::new(exe)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
    };
    // "Text file busy" (ETXTBSY) while another process still holds the file
    // open for writing, briefly, as during an install: try a few more times.
    let mut child = (0..5)
        .find_map(|i| match spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && i < 4 => {
                std::thread::sleep(Duration::from_millis(50));
                None
            }
            other => Some(other),
        })?
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    // `evnx 0.12.0`, or `evnx 0.12.0 (…)`.
    let v = out.split_whitespace().nth(1)?;
    v.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        .then(|| v.to_string())
}

/// The copy with the highest known version; the first on a tie.
pub fn newest(copies: &[Copy]) -> Option<usize> {
    fn key(v: &str) -> Vec<u64> {
        v.split(['.', '-', '+'])
            .map_while(|p| p.parse().ok())
            .collect()
    }
    copies
        .iter()
        .enumerate()
        .filter_map(|(i, c)| Some((i, key(c.version.as_deref()?))))
        .fold(None, |best: Option<(usize, Vec<u64>)>, (i, k)| match best {
            Some((_, ref b)) if *b >= k => best,
            _ => Some((i, k)),
        })
        .map(|(i, _)| i)
}

/// How to remove `c`, with the channel that installed it.
pub fn removal(c: &Copy) -> String {
    match c.channel {
        Channel::Cargo => "cargo uninstall evnx".into(),
        Channel::Homebrew => "brew uninstall evnx".into(),
        Channel::Scoop => "scoop uninstall evnx".into(),
        Channel::Winget => "winget uninstall urwithajit9.evnx".into(),
        Channel::Npm => "npm uninstall -g @evnx/cli".into(),
        Channel::PyPI | Channel::PipSystem => "pip uninstall evnx".into(),
        Channel::Pipx => "pipx uninstall evnx".into(),
        Channel::Script | Channel::Unknown => format!(
            "rm {}",
            crate::core::shell::word(&c.path.display().to_string())
        ),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn stub(dir: &Path, body: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join("evnx");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn rm42_path_order_symlinks_and_extra_folders() {
        let t = tempfile::TempDir::new().unwrap();
        let a = t.path().join("a");
        let b = t.path().join("b");
        let link = t.path().join("link");
        let off = t.path().join("off");
        stub(&a, "echo 'evnx 0.10.0'");
        let real = stub(&b, "echo 'evnx 0.11.0'");
        std::fs::create_dir_all(&link).unwrap();
        std::os::unix::fs::symlink(&real, link.join("evnx")).unwrap();
        stub(&off, "echo 'evnx 0.9.0'");

        let mut copies = find_in(&[a.clone(), b.clone(), link], &[off.clone(), a.clone()]);
        let dirs: Vec<_> = copies.iter().map(|c| c.path.parent().unwrap()).collect();
        assert_eq!(
            dirs,
            [a.as_path(), b.as_path(), off.as_path()],
            "{copies:?}"
        );
        assert!(copies[0].runs && !copies[1].runs && !copies[2].runs);

        read_versions(&mut copies);
        let v: Vec<_> = copies.iter().map(|c| c.version.as_deref()).collect();
        assert_eq!(v, [Some("0.10.0"), Some("0.11.0"), Some("0.9.0")]);
        assert!(copies.iter().all(|c| c.version_from == VersionFrom::Ran));
        // 0.11.0 is kept, though 0.10.0 runs.
        assert_eq!(newest(&copies), Some(1));
    }

    #[test]
    fn d7_a_copy_that_hangs_is_given_up_on() {
        let t = tempfile::TempDir::new().unwrap();
        let p = stub(t.path(), "sleep 30");
        let start = Instant::now();
        assert_eq!(run_version(&p, Duration::from_millis(300)), None);
        assert!(start.elapsed() < Duration::from_secs(5));
        let garbage = stub(&t.path().join("g"), "echo hello world");
        assert_eq!(run_version(&garbage, RUN_TIMEOUT), None);
    }

    #[test]
    fn d7_metadata_before_running() {
        let t = tempfile::TempDir::new().unwrap();
        let cellar = t.path().join("Cellar/evnx/0.10.15/bin");
        let exe = stub(&cellar, "exit 1");
        assert_eq!(from_metadata(&exe).as_deref(), Some("0.10.15"));

        let pkg = t.path().join("node_modules/@evnx/cli-linux-x64");
        let exe = stub(&pkg.join("bin"), "exit 1");
        std::fs::write(
            pkg.join("package.json"),
            r#"{"name":"@evnx/cli-linux-x64","version":"0.10.0"}"#,
        )
        .unwrap();
        assert_eq!(from_metadata(&exe).as_deref(), Some("0.10.0"));
    }
}
