//! WP-11a: signals and exit of `evnx run` and `evnx cloud run` (Unix).
//!
//! Rows S1-S9 and X1-X6 of the WP-11 plan. Black box: the built binary is
//! started, signalled from outside, and what is left of it and of its child is
//! looked at. Two launch paths are covered, because they are different code:
//! `--no-mask` replaces evnx with the child (`exec`, so a signal death is the
//! child's own), `--mask` keeps evnx as a forwarder that returns the child's
//! code (128+n for a signal). Masking needs a value that looks like a secret,
//! so every project and vault here holds `DB_PASSWORD`.
//!
//! No assertion rests on a sleep: waits poll for a marker file the child
//! writes, with a deadline.
#![cfg(unix)]

mod support;

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use support::Project;

const ENV: &str = "PORT=8080\nDB_PASSWORD=hunter22hunter\n";
/// How long a signalled evnx and its child get to be gone.
const DEADLINE: Duration = Duration::from_secs(5);
/// Starting evnx (and, for `cloud run`, unlocking a vault in a debug build).
const START: Duration = Duration::from_secs(60);

const TERM: i32 = libc::SIGTERM;
const HUP: i32 = libc::SIGHUP;
const QUIT: i32 = libc::SIGQUIT;
const INT: i32 = libc::SIGINT;

fn flag(masked: bool) -> &'static str {
    if masked {
        "--mask"
    } else {
        "--no-mask"
    }
}

struct Done {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// evnx in `dir`, with the four signals at their default action (a runner under
/// `nohup` would otherwise hand evnx an ignored SIGHUP) unless `ignore_hup`.
fn start(
    dir: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    stdin: Option<&[u8]>,
    ignore_hup: bool,
) -> Child {
    let mut cmd = Command::new(support::evnx_bin());
    cmd.current_dir(dir)
        .args(args)
        .env("NO_COLOR", "1")
        .env_remove("CI")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    // SAFETY: runs in the forked child before exec and only calls `signal`,
    // `sigemptyset` and `sigprocmask`, which are async-signal-safe, with valid
    // arguments. The mask is cleared because a test runner (an IDE task, a
    // terminal multiplexer, a service manager) may start `cargo test` with some
    // of these signals blocked, and evnx would inherit that mask: a blocked
    // SIGTERM is never delivered, so the tests would wait for an exit that
    // cannot come.
    unsafe {
        cmd.pre_exec(move || {
            let mut empty = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
            libc::sigemptyset(empty.as_mut_ptr());
            libc::sigprocmask(libc::SIG_SETMASK, empty.as_ptr(), std::ptr::null_mut());
            for sig in [libc::SIGTERM, libc::SIGQUIT, libc::SIGINT] {
                libc::signal(sig, libc::SIG_DFL);
            }
            let hup = if ignore_hup {
                libc::SIG_IGN
            } else {
                libc::SIG_DFL
            };
            libc::signal(libc::SIGHUP, hup);
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn evnx");
    if let Some(bytes) = stdin {
        use std::io::Write;
        let mut pipe = child.stdin.take().expect("stdin piped");
        let _ = pipe.write_all(bytes);
    }
    child
}

fn poll(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_pid(dir: &Path, name: &str) -> i32 {
    std::fs::read_to_string(dir.join(name))
        .expect("pid file")
        .trim()
        .parse()
        .expect("a pid")
}

fn send(sig: i32, pid: u32) {
    let name = match sig {
        TERM => "TERM",
        HUP => "HUP",
        QUIT => "QUIT",
        INT => "INT",
        other => panic!("no name for {other}"),
    };
    let ok = Command::new("sh")
        .args(["-c", &format!("kill -{name} {pid}")])
        .status()
        .unwrap()
        .success();
    assert!(ok, "kill -{name} {pid} failed");
}

fn alive(pid: i32) -> bool {
    Command::new("sh")
        .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
        .status()
        .unwrap()
        .success()
}

/// Wait for the exit and collect both streams.
fn finish(mut child: Child, limit: Duration) -> Done {
    poll("evnx to exit", limit, || {
        child.try_wait().expect("try_wait").is_some()
    });
    let out = child.wait_with_output().expect("collect output");
    Done {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Died of `sig`: evnx exits 128+sig when it stayed to forward (masked), and
/// when it was replaced by the child the process itself died of it.
#[track_caller]
fn assert_died_of(d: &Done, masked: bool, sig: i32) {
    if masked {
        assert_eq!(
            d.status.code(),
            Some(128 + sig),
            "stderr: {}\nstatus {:?}",
            d.stderr,
            d.status
        );
        assert!(
            d.stderr.contains(&format!("killed by signal {sig}")),
            "{}",
            d.stderr
        );
    } else {
        assert_eq!(d.status.signal(), Some(sig), "status {:?}", d.status);
    }
}

/// A child that records its pid, then waits for a signal. `exec` makes the pid
/// the one `sleep` runs under.
const SLEEPER: &str = "echo $$ > pid.tmp; mv pid.tmp pid; exec sleep 30";

fn local(p: &Project, masked: bool, script: &str) -> Child {
    start(
        p.path(),
        &["run", flag(masked), "--", "sh", "-c", script],
        &[],
        None,
        false,
    )
}

/// Signal evnx while the child sleeps; the child must be gone, and evnx's exit
/// must say what killed it.
#[track_caller]
fn signal_local(sig: i32, masked: bool) {
    let p = Project::new().env(ENV);
    let evnx = local(&p, masked, SLEEPER);
    poll("the child to start", START, || p.join("pid").exists());
    let pid = read_pid(p.path(), "pid");
    send(sig, evnx.id());
    let d = finish(evnx, DEADLINE);
    assert_died_of(&d, masked, sig);
    poll("the child to be gone", DEADLINE, || !alive(pid));
}

// ─── S1-S3: signals reach the child, both launch paths ───────────────────

#[test]
fn s1_sigterm_reaches_the_child_masked() {
    signal_local(TERM, true);
}

#[test]
fn s1_sigterm_reaches_the_child_unmasked() {
    signal_local(TERM, false);
}

#[test]
fn s2_sighup_reaches_the_child() {
    signal_local(HUP, true);
    signal_local(HUP, false);
}

#[test]
fn s2_sigquit_reaches_the_child() {
    signal_local(QUIT, true);
    signal_local(QUIT, false);
}

#[test]
fn s3_sigint_sent_to_evnx_alone_reaches_the_child() {
    signal_local(INT, true);
    signal_local(INT, false);
}

/// `nohup` leaves SIGHUP ignored; the child must inherit that, not lose it to
/// a forwarding handler. SIGTERM still goes through.
#[test]
fn s3b_an_ignored_sighup_stays_ignored() {
    let p = Project::new().env(ENV);
    let evnx = start(
        p.path(),
        &["run", "--mask", "--", "sh", "-c", SLEEPER],
        &[],
        None,
        true,
    );
    poll("the child to start", START, || p.join("pid").exists());
    let pid = read_pid(p.path(), "pid");
    send(HUP, evnx.id());
    // Give a (wrongly) forwarded SIGHUP time to arrive; the check below is the
    // assertion, this only orders it.
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(pid), "a SIGHUP that nohup ignores killed the child");
    send(TERM, evnx.id());
    let d = finish(evnx, DEADLINE);
    assert_died_of(&d, true, TERM);
}

// ─── S4: a child that handles the signal gets to finish ──────────────────

#[test]
fn s4_a_child_that_traps_sigterm_is_waited_for() {
    let p = Project::new().env(ENV);
    let script = "trap 'touch cleaned; exit 3' TERM; echo $$ > pid.tmp; mv pid.tmp pid; \
                  while :; do sleep 0.05; done";
    let evnx = local(&p, true, script);
    poll("the child to start", START, || p.join("pid").exists());
    let pid = read_pid(p.path(), "pid");
    send(TERM, evnx.id());
    let d = finish(evnx, DEADLINE);
    assert_eq!(d.status.code(), Some(3), "{}", d.stderr);
    assert!(p.join("cleaned").exists(), "evnx left before the cleanup");
    poll("the child to be gone", DEADLINE, || !alive(pid));
}

// ─── S5: the child dies of a signal of its own ───────────────────────────

#[test]
fn s5_a_child_killed_by_sigkill_exits_137_with_a_note() {
    for masked in [true, false] {
        let p = Project::new().env(ENV);
        let d = finish(local(&p, masked, "kill -KILL $$"), START);
        assert_died_of(&d, masked, libc::SIGKILL);
    }
}

#[test]
fn s5_the_note_is_not_printed_under_quiet() {
    let p = Project::new().env(ENV);
    let evnx = start(
        p.path(),
        &["-q", "run", "--mask", "--", "sh", "-c", "kill -KILL $$"],
        &[],
        None,
        false,
    );
    let d = finish(evnx, START);
    assert_eq!(d.status.code(), Some(137));
    assert!(!d.stderr.contains("signal"), "{}", d.stderr);
}

// ─── S6: only the direct child is signalled (pins today's behaviour) ─────

#[test]
fn s6_a_grandchild_is_not_signalled() {
    for masked in [true, false] {
        let p = Project::new().env(ENV);
        // The grandchild's output is closed, so a masking evnx is not held open
        // by its pipe.
        let script = "sleep 30 >/dev/null 2>&1 & echo $! > gpid.tmp; mv gpid.tmp gpid; \
                      echo $$ > pid.tmp; mv pid.tmp pid; wait";
        let evnx = local(&p, masked, script);
        poll("the child to start", START, || p.join("pid").exists());
        let grandchild = read_pid(p.path(), "gpid");
        send(TERM, evnx.id());
        let d = finish(evnx, DEADLINE);
        assert_died_of(&d, masked, TERM);
        let survived = alive(grandchild);
        // Clean up before asserting.
        let _ = Command::new("sh")
            .args(["-c", &format!("kill -KILL {grandchild}")])
            .status();
        assert!(survived, "the grandchild was signalled (masked={masked})");
    }
}

// ─── S7: a reader that goes away ends the child ──────────────────────────

#[test]
fn s7_a_closed_pipe_ends_an_exec_d_child_by_sigpipe() {
    use std::io::{BufRead, BufReader};
    let p = Project::new().env(ENV);
    let mut evnx = start(
        p.path(),
        &["run", "--no-mask", "--", "yes"],
        &[],
        None,
        false,
    );
    {
        let mut line = String::new();
        let mut out = BufReader::new(evnx.stdout.take().unwrap());
        out.read_line(&mut line).unwrap();
        assert_eq!(line, "y\n");
    } // the reader is dropped: `yes | head -1`
    let d = finish(evnx, DEADLINE);
    assert_eq!(d.status.signal(), Some(libc::SIGPIPE), "{:?}", d.status);
    assert!(!d.stderr.contains("panicked"), "{}", d.stderr);
}

// ─── X1-X6: exit codes ───────────────────────────────────────────────────

#[test]
fn x1_the_childs_code_passes_through() {
    for masked in [true, false] {
        for code in [0, 1, 2, 4, 7, 42, 255] {
            let p = Project::new().env(ENV);
            let d = finish(local(&p, masked, &format!("exit {code}")), START);
            assert_eq!(
                d.status.code(),
                Some(code),
                "exit {code}, masked={masked}: {}",
                d.stderr
            );
            assert!(!d.stderr.contains("Error"), "{}", d.stderr);
        }
    }
}

#[test]
fn x2_a_missing_program_exits_2_with_the_sh_c_hint() {
    for masked in [true, false] {
        let p = Project::new().env(ENV);
        let evnx = start(
            p.path(),
            &["run", flag(masked), "--", "evnx-no-such-program-xyz"],
            &[],
            None,
            false,
        );
        let d = finish(evnx, START);
        assert_eq!(d.status.code(), Some(2), "{}", d.stderr);
        assert!(d.stderr.contains("no such command"), "{}", d.stderr);
        assert!(d.stderr.contains("sh -c"), "{}", d.stderr);
    }
}

#[test]
fn x3_a_file_that_is_not_executable_exits_2_failed_to_run() {
    for masked in [true, false] {
        let p = Project::new()
            .env(ENV)
            .file("script.sh", "#!/bin/sh\nexit 0\n");
        let evnx = start(
            p.path(),
            &["run", flag(masked), "--", "./script.sh"],
            &[],
            None,
            false,
        );
        let d = finish(evnx, START);
        assert_eq!(d.status.code(), Some(2), "{}", d.stderr);
        assert!(d.stderr.contains("failed to run"), "{}", d.stderr);
    }
}

#[test]
fn x4_a_refusal_before_the_child_means_it_never_ran() {
    for (env, code) in [
        ("LD_PRELOAD=/tmp/x.so\nDB_PASSWORD=hunter22hunter\n", 2),
        ("this is not an env file\n", 2),
    ] {
        let p = Project::new().env(env);
        let d = finish(local(&p, true, "touch ran"), START);
        assert_eq!(d.status.code(), Some(code), "{}", d.stderr);
        assert!(!p.join("ran").exists(), "the command ran after a refusal");
    }
    // No .env at all.
    let p = Project::new();
    let d = finish(local(&p, true, "touch ran"), START);
    assert_eq!(d.status.code(), Some(2), "{}", d.stderr);
    assert!(!p.join("ran").exists());
}

/// The reader of stdout is gone while the child still runs: the copy ends
/// quietly and the child's code is still evnx's.
#[test]
fn x5_a_closed_stdout_does_not_change_the_childs_code() {
    let p = Project::new().env(ENV);
    let out = support::run::closed_stdout(
        p.path(),
        &[
            "run",
            "--mask",
            "--",
            "sh",
            "-c",
            "echo hi; sleep 0.3; exit 5",
        ],
    );
    out.assert_code(5);
    assert!(!out.stderr.contains("panicked"), "{}", out.stderr);
    assert!(!out.stderr.contains("Error"), "{}", out.stderr);
}

/// X6: the masked path returns the code through `main` (so destructors run)
/// rather than ending the process itself. The architecture test pins that
/// `inject.rs` has no `process::exit`; this pins what the user sees: no extra
/// line for an ordinary failing child.
#[test]
fn x6_a_failing_child_adds_nothing_to_stderr() {
    let p = Project::new().env(ENV);
    let d = finish(local(&p, true, "echo out; echo err >&2; exit 9"), START);
    assert_eq!(d.status.code(), Some(9));
    assert_eq!(d.stdout, "out\n");
    assert_eq!(d.stderr, "err\n");
}

#[test]
fn s9_secrets_are_not_printed_by_the_signal_paths() {
    let p = Project::new().env(ENV);
    let evnx = local(
        &p,
        true,
        "echo $DB_PASSWORD; echo $$ > pid.tmp; mv pid.tmp pid; exec sleep 30",
    );
    poll("the child to start", START, || p.join("pid").exists());
    send(TERM, evnx.id());
    let d = finish(evnx, DEADLINE);
    assert!(
        !d.stdout.contains("hunter22hunter") && !d.stderr.contains("hunter22hunter"),
        "{}{}",
        d.stdout,
        d.stderr
    );
    assert!(d.stdout.contains("***"), "{}", d.stdout);
}

// ─── cloud run, against the mock server ──────────────────────────────────

#[cfg(feature = "cloud")]
mod cloud {
    use super::*;
    use evnx_crypto::{
        b64_encode, derive_master_key, encrypt_private_key, encrypt_vault, generate_keypair,
        generate_salt, salt_to_base64, vault_aad, wrap_vault_key_with_master_key, VaultKey,
    };

    const PW: &str = "correct horse battery staple";
    const VAULT_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";

    /// A server with one vault, `shared/development`, at version 1.
    fn server(text: &str) -> mockito::ServerGuard {
        let salt = generate_salt();
        let master = derive_master_key(PW.as_bytes(), &salt).unwrap();
        let keypair = generate_keypair();
        let sealed = encrypt_private_key(&keypair, &master).unwrap().to_base64();
        let me = format!(
            r#"{{"user_id":"u-me","email":"me@example.com","argon2_salt":"{}",
            "encrypted_private_key":"{sealed}","totp_enabled":false,"has_mlkem_key":true}}"#,
            salt_to_base64(&salt)
        );
        let mut s = mockito::Server::new();
        s.mock("GET", "/api/v1/auth/me")
            .with_status(200)
            .with_body(me)
            .create();
        s.mock("GET", "/api/v1/vaults")
            .with_status(200)
            .with_body(format!(
                r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"shared","environment":"development",
                "role":"owner","version_count":1,"updated_at":"2026-01-01T00:00:00Z"}}]}}"#
            ))
            .create();
        let vk = VaultKey::generate();
        let b = encrypt_vault(text.as_bytes(), &vk, &vault_aad(VAULT_ID, 1)).unwrap();
        let mut bytes = b.nonce.to_vec();
        bytes.extend_from_slice(&b.ciphertext);
        let wrapped = wrap_vault_key_with_master_key(&vk, &master).unwrap();
        s.mock(
            "GET",
            format!("/api/v1/vaults/{VAULT_ID}/versions/latest").as_str(),
        )
        .with_status(200)
        .with_body(r#"{"version_num":1}"#)
        .create();
        s.mock(
            "GET",
            format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
        )
        .with_status(200)
        .with_body(bytes)
        .create();
        s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
            .with_status(200)
            .with_body(format!(
                r#"{{"encrypted_vault_key":"{}"}}"#,
                b64_encode(&wrapped)
            ))
            .create();
        s
    }

    fn cloud_run(p: &Project, s: &mockito::ServerGuard, masked: bool, script: &str) -> Child {
        let cfg = p.path().join("cfg");
        support::cloud::login(&cfg, &s.url());
        start(
            p.path(),
            &[
                "cloud",
                "run",
                "--vault",
                "shared",
                "--password-stdin",
                flag(masked),
                "--",
                "sh",
                "-c",
                script,
            ],
            &[
                ("EVNX_SERVER", &s.url()),
                ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ],
            Some(format!("{PW}\n").as_bytes()),
            false,
        )
    }

    #[track_caller]
    fn signal_cloud(sig: i32, masked: bool) {
        let s = server("DB_PASSWORD=hunter22hunter\n");
        let p = Project::new();
        let evnx = cloud_run(&p, &s, masked, SLEEPER);
        poll("the child to start", START, || p.join("pid").exists());
        let pid = read_pid(p.path(), "pid");
        send(sig, evnx.id());
        let d = finish(evnx, DEADLINE);
        assert_died_of(&d, masked, sig);
        poll("the child to be gone", DEADLINE, || !alive(pid));
    }

    #[test]
    fn s1_cloud_sigterm_reaches_the_child_masked() {
        signal_cloud(TERM, true);
    }

    #[test]
    fn s1_cloud_sigterm_reaches_the_child_unmasked() {
        signal_cloud(TERM, false);
    }

    #[test]
    fn s2_cloud_sighup_reaches_the_child() {
        signal_cloud(HUP, true);
        signal_cloud(HUP, false);
    }

    #[test]
    fn x1_cloud_the_childs_code_passes_through() {
        let s = server("DB_PASSWORD=hunter22hunter\n");
        for masked in [true, false] {
            for code in [0, 2, 7, 255] {
                let p = Project::new();
                let d = finish(cloud_run(&p, &s, masked, &format!("exit {code}")), START);
                assert_eq!(d.status.code(), Some(code), "masked={masked}: {}", d.stderr);
            }
        }
    }

    #[test]
    fn x2_cloud_a_missing_program_exits_2() {
        let s = server("DB_PASSWORD=hunter22hunter\n");
        for masked in [true, false] {
            let p = Project::new();
            let cfg = p.path().join("cfg");
            support::cloud::login(&cfg, &s.url());
            let evnx = start(
                p.path(),
                &[
                    "cloud",
                    "run",
                    "--vault",
                    "shared",
                    "--password-stdin",
                    flag(masked),
                    "--",
                    "evnx-no-such-program-xyz",
                ],
                &[
                    ("EVNX_SERVER", &s.url()),
                    ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
                ],
                Some(format!("{PW}\n").as_bytes()),
                false,
            );
            let d = finish(evnx, START);
            assert_eq!(d.status.code(), Some(2), "{}", d.stderr);
            assert!(d.stderr.contains("no such command"), "{}", d.stderr);
        }
    }

    #[test]
    fn x4_cloud_not_signed_in_exits_10_and_the_child_never_ran() {
        let p = Project::new();
        let cfg = p.path().join("empty-cfg");
        let evnx = start(
            p.path(),
            &["cloud", "run", "--vault", "shared", "--", "touch", "ran"],
            &[
                ("EVNX_SERVER", "http://127.0.0.1:9"),
                ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ],
            None,
            false,
        );
        let d = finish(evnx, START);
        assert_eq!(d.status.code(), Some(10), "{}", d.stderr);
        assert!(!p.join("ran").exists());
    }
}
