//! The built binary placed where a package manager would put it (T-SHARE-45).
//!
//! `evnx update` works out its install channel from the path of the running
//! executable and the files around it, so the only honest test is to run the
//! real binary from such a path. Each call makes a fresh folder under cargo's
//! per-target temp directory, so the binary can be hard-linked rather than
//! copied (a debug build is large); a copy is the fallback.

use super::run::{run_program, Output};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A fake install location holding the evnx binary at `<root>/<bin_dir>/evnx`.
pub struct Install {
    root: TempDir,
    exe: PathBuf,
}

impl Install {
    /// The binary at `bin_dir` (relative, e.g. `.cargo/bin`), with nothing else
    /// around it.
    pub fn at(bin_dir: &str) -> Install {
        let root = tempfile::Builder::new()
            .prefix("install-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .expect("create install folder");
        let dir = root.path().join(bin_dir);
        std::fs::create_dir_all(&dir).expect("create bin folder");
        let exe = dir.join(if cfg!(windows) { "evnx.exe" } else { "evnx" });
        let built = Path::new(super::evnx_bin());
        if std::fs::hard_link(built, &exe).is_err() {
            std::fs::copy(built, &exe).expect("copy evnx");
        }
        Install { root, exe }
    }

    /// Add a file (and its folders) relative to the install root, e.g.
    /// `pyvenv.cfg` or `lib/python3.12/site-packages/evnx-0.10.5.dist-info/RECORD`.
    pub fn file(self, rel: &str) -> Install {
        let path = self.root.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create folders");
        std::fs::write(&path, b"").expect("write file");
        self
    }

    /// Run the placed binary with `args`.
    pub fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, &[])
    }

    /// [`Install::run`] with extra environment variables.
    pub fn run_with(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        run_program(&self.exe, self.root.path(), args, env, None)
    }
}
