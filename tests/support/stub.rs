//! Stub programs a test puts on `PATH` or names in an environment variable — a
//! fake `npm`, `gcloud`, `az` — written so that they can actually run.
//!
//! # Why not `fs::write` and `chmod`
//!
//! Linux refuses to run a file that any process holds open for writing
//! (ETXTBSY, "Text file busy"; bash reports it as `bad interpreter: Text file
//! busy`). Writing the stub opens it for writing for a moment, and if another
//! test thread starts a process in that moment, the new child inherits the
//! descriptor and keeps it until its own exec. A script that runs the stub
//! inside that window fails. CI hit it in `npm_publish.rs` (R-convert's PR,
//! 2026-10-20): `npm: /usr/bin/env: bad interpreter: Text file busy`.
//!
//! [`write_stub`] therefore runs the stub once, with `EVNX_STUB_PROBE=1` (a
//! guard line it adds makes that run exit at once), and retries while the
//! answer is ETXTBSY. Once one run has succeeded no writable descriptor is left
//! anywhere — ours was closed before the probe, and no process forked later can
//! inherit it — so every later run is safe.

use std::path::Path;
use std::time::{Duration, Instant};

/// How long [`write_stub`] waits for the file to become runnable.
const PATIENCE: Duration = Duration::from_secs(10);

/// Write `script` (starting `#!`) to `path`, executable, and return once it
/// can be run.
#[track_caller]
pub fn write_stub(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let (shebang, body) = script
        .split_once('\n')
        .filter(|(first, _)| first.starts_with("#!"))
        .expect("a stub starts with a #! line");
    let content = format!("{shebang}\n[ -n \"$EVNX_STUB_PROBE\" ] && exit 0\n{body}");
    std::fs::write(path, content).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let started = Instant::now();
    loop {
        let probe = std::process::Command::new(path)
            .env("EVNX_STUB_PROBE", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match probe {
            Ok(_) => return,
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && started.elapsed() < PATIENCE => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("stub {} cannot run: {e}", path.display()),
        }
    }
}
