//! A throwaway project folder to run evnx in.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// A temp folder holding the files one scenario needs. Deleted on drop.
///
/// Builder methods take `self` and return it, so a scenario reads as one
/// expression:
///
/// ```text
/// let p = Project::new()
///     .env("DATABASE_URL=postgres://real\n")
///     .example("DATABASE_URL=\n")
///     .git();
/// ```
pub struct Project {
    dir: TempDir,
    /// A separate folder for git's global config, so a developer's own
    /// `~/.gitconfig` (a global excludes file, hooks, `init.defaultBranch`)
    /// cannot change what a test sees. `None` until [`Project::git`].
    git_home: Option<TempDir>,
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    pub fn new() -> Project {
        Project {
            dir: TempDir::new().expect("create temp dir"),
            git_home: None,
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// `name` inside the project.
    pub fn join(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    // ── Files ──────────────────────────────────────────────────────────────

    /// Write `content` to `name`, creating parent folders.
    pub fn file(self, name: &str, content: &str) -> Project {
        let path = self.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&path, content).unwrap_or_else(|e| panic!("write {name}: {e}"));
        self
    }

    pub fn env(self, content: &str) -> Project {
        self.file(".env", content)
    }

    pub fn example(self, content: &str) -> Project {
        self.file(".env.example", content)
    }

    /// `.evnx.toml`, the project config and spec.
    pub fn toml(self, content: &str) -> Project {
        self.file(".evnx.toml", content)
    }

    pub fn dir(self, name: &str) -> Project {
        fs::create_dir_all(self.join(name)).unwrap_or_else(|e| panic!("mkdir {name}: {e}"));
        self
    }

    /// Set Unix permission bits, e.g. `0o444` for a read-only file.
    #[cfg(unix)]
    pub fn mode(self, name: &str, mode: u32) -> Project {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(self.join(name), fs::Permissions::from_mode(mode))
            .unwrap_or_else(|e| panic!("chmod {name}: {e}"));
        self
    }

    /// `link` → `target`, where `target` is taken as written (relative to the
    /// link's folder, or absolute).
    #[cfg(unix)]
    pub fn symlink(self, target: &str, link: &str) -> Project {
        std::os::unix::fs::symlink(target, self.join(link))
            .unwrap_or_else(|e| panic!("symlink {link} -> {target}: {e}"));
        self
    }

    pub fn read(&self, name: &str) -> String {
        fs::read_to_string(self.join(name)).unwrap_or_else(|e| panic!("read {name}: {e}"))
    }

    pub fn exists(&self, name: &str) -> bool {
        self.join(name).exists()
    }

    /// How many lines of `name` start with `prefix`. Missing file → 0.
    ///
    /// The question most `add` tests ask: "is `REDIS_URL=` in there once?"
    pub fn count_lines(&self, name: &str, prefix: &str) -> usize {
        fs::read_to_string(self.join(name))
            .map(|s| s.lines().filter(|l| l.starts_with(prefix)).count())
            .unwrap_or(0)
    }

    // ── Git (T-SHARE-32) ───────────────────────────────────────────────────

    /// Make the folder a real git repository on branch `main`, isolated from the
    /// machine's git configuration.
    ///
    /// ⚠️ Not `fs::create_dir(".git")`. An empty `.git` makes a folder *look*
    /// like a repository to code that only checks for the directory, while
    /// `git check-ignore`, `git ls-files` and `git status` fail against it. A
    /// test built on one tests a program no user runs.
    pub fn git(mut self) -> Project {
        let home = TempDir::new().expect("create git home");
        fs::write(home.path().join("gitconfig"), "").expect("write empty gitconfig");
        self.git_home = Some(home);
        // `-b` needs git 2.28; set HEAD by hand so older runners work too.
        self.git_cmd(&["init", "-q"]);
        self.git_cmd(&["symbolic-ref", "HEAD", "refs/heads/main"]);
        self.git_cmd(&["config", "user.name", "evnx tests"]);
        self.git_cmd(&["config", "user.email", "tests@evnx.invalid"]);
        self.git_cmd(&["config", "commit.gpgsign", "false"]);
        self.git_cmd(&["config", "core.autocrlf", "false"]);
        self
    }

    /// Stage `name` (respects `.gitignore`: an ignored file fails, as with git).
    pub fn track(&self, name: &str) -> &Project {
        self.git_cmd(&["add", "--", name]);
        self
    }

    /// Stage `name` even if it is ignored, as `git add -f` does.
    pub fn force_add(&self, name: &str) -> &Project {
        self.git_cmd(&["add", "-f", "--", name]);
        self
    }

    /// Stage everything git would stage (`git add -A`) and commit.
    pub fn commit_all(&self, message: &str) -> &Project {
        self.git_cmd(&["add", "-A"]);
        self.git_cmd(&["commit", "-q", "--allow-empty", "-m", message]);
        self
    }

    /// What git itself says: is `name` ignored?
    pub fn is_ignored(&self, name: &str) -> bool {
        let out = self.git_command(&["check-ignore", "-q", "--", name]);
        match out.status.code() {
            Some(0) => true,
            Some(1) => false,
            _ => panic!(
                "git check-ignore {name} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ),
        }
    }

    /// Run git in the project and panic with its stderr if it fails.
    pub fn git_cmd(&self, args: &[&str]) -> String {
        let out = self.git_command(args);
        assert!(
            out.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn git_command(&self, args: &[&str]) -> std::process::Output {
        let home = self
            .git_home
            .as_ref()
            .expect("call .git() before using git helpers");
        Command::new("git")
            .current_dir(self.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", home.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path())
            .output()
            .unwrap_or_else(|e| panic!("git is needed for this test: {e}"))
    }
}
