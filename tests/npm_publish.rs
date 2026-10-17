//! I-256: an npm release survives a slow registry, and a re-run finishes it.
//!
//! v0.10.6's "Publish to npm" uploaded all five platform packages, then gave up
//! after 300 s waiting for `evnx-linux-x64`, which npm showed 8.5 minutes after
//! its publish. The re-run could not finish: publishing linux-x64 again failed
//! with an error the loop did not recognise while npm was still processing the
//! first upload, and the wrapper step has no skip at all.
//!
//! These run `scripts/npm-await.sh` and `scripts/npm-publish-packages.sh`
//! against a stub `npm` (through `NPM_BIN`) whose answers come from a state
//! folder, so a slow registry, a version that is already there and an expired
//! token can each be produced on demand. Linux only: the stub is bash.
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

const AWAIT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/npm-await.sh");
const PUBLISH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/npm-publish-packages.sh"
);
/// The longest any one script run may take here.
const LIMIT: std::time::Duration = std::time::Duration::from_secs(30);
const WORKFLOW: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/.github/workflows/npm-publish.yml"
);

/// `npm view --prefer-online <pkg>@<version> version` answers the version when
/// the spec is listed in `present`, or from the `appear_after`-th view call on
/// (`never`: not at all), else a 404 as npm prints it. `npm publish` prints `publish_out` and exits `publish_rc`.
/// Every call is appended to `calls`.
const STUB: &str = r#"#!/usr/bin/env bash
s="$NPM_STUB_STATE"
echo "$*" >> "$s/calls"
case "$1" in
  view)
    n=$(( $(cat "$s/views" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$s/views"
    after=$(cat "$s/appear_after" 2>/dev/null || echo never)
    spec=$3
    if grep -qxF "$spec" "$s/present" 2>/dev/null; then echo "${spec##*@}"; exit 0; fi
    if [ "$after" != never ] && [ "$n" -ge "$after" ]; then echo "${spec##*@}"; exit 0; fi
    echo "npm ERR! code E404" >&2
    echo "npm ERR! 404 No match found for version ${spec##*@}" >&2
    exit 1 ;;
  publish)
    cat "$s/publish_out" 2>/dev/null
    exit "$(cat "$s/publish_rc" 2>/dev/null || echo 0)" ;;
esac
"#;

struct Registry {
    dir: TempDir,
}

impl Registry {
    fn new() -> Registry {
        let dir = TempDir::new().unwrap();
        let npm = dir.path().join("npm");
        std::fs::write(&npm, STUB).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir(dir.path().join("state")).unwrap();
        Registry { dir }
    }

    fn state(&self, name: &str, value: &str) -> &Registry {
        std::fs::write(self.dir.path().join("state").join(name), value).unwrap();
        self
    }

    /// The version is visible from the `n`-th view call on.
    fn appears_after(&self, n: u32) -> &Registry {
        self.state("appear_after", &n.to_string())
    }

    fn never_appears(&self) -> &Registry {
        self.state("appear_after", "never")
    }

    fn publish_answers(&self, rc: i32, out: &str) -> &Registry {
        self.state("publish_rc", &rc.to_string())
            .state("publish_out", out)
    }

    fn calls(&self, verb: &str) -> usize {
        std::fs::read_to_string(self.dir.path().join("state/calls"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.split(' ').next() == Some(verb))
            .count()
    }

    /// A package folder as the workflow builds it.
    fn package(&self, name: &str, version: &str) -> PathBuf {
        let dir = self.dir.path().join("pkg").join(name.replace('/', "_"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            format!("{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\"\n}}\n"),
        )
        .unwrap();
        dir
    }

    /// Run `script`, killed after [`LIMIT`]: a wait that ignores its deadline
    /// would otherwise hang the suite instead of failing it (with a 0 s poll
    /// interval the old `npm-await.sh` never reached its own timeout).
    fn run(&self, script: &str, args: &[&str], env: &[(&str, String)]) -> Output {
        let (out_path, err_path) = (self.dir.path().join("out"), self.dir.path().join("err"));
        let mut cmd = Command::new("bash");
        cmd.arg(script)
            .args(args)
            .env("NPM_BIN", self.dir.path().join("npm"))
            .env("NPM_STUB_STATE", self.dir.path().join("state"))
            .env("NPM_AWAIT_INTERVAL", "0")
            .env_remove("NPM_AWAIT_DEADLINE")
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&out_path).unwrap())
            .stderr(std::fs::File::create(&err_path).unwrap());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > LIMIT {
                child.kill().unwrap();
                break child.wait().unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let mut stderr = std::fs::read(&err_path).unwrap();
        if status.code().is_none() {
            stderr.extend_from_slice(format!("\n(killed after {}s)\n", LIMIT.as_secs()).as_bytes());
        }
        Output {
            status,
            stdout: std::fs::read(&out_path).unwrap(),
            stderr,
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn has_node() -> bool {
    Command::new("node").arg("--version").output().is_ok()
}

// ── npm-await.sh ────────────────────────────────────────────────────────────

/// I-256: with a shared deadline, a wait runs until the deadline rather than
/// its own timeout — 300 s gave up on a package npm showed 8.5 min later.
/// Scaled: a 2 s timeout, polled every second, a version that shows on the
/// fourth poll.
#[test]
fn t_i256_await_waits_past_its_timeout_within_the_deadline() {
    let r = Registry::new();
    r.appears_after(4);
    let interval = [("NPM_AWAIT_INTERVAL", "1".to_string())];

    // Without a deadline the call's own timeout still applies, as before.
    let alone = r.run(
        AWAIT,
        &["version", "@evnx/evnx-linux-x64", "0.10.7", "2"],
        &interval,
    );
    assert_eq!(alone.status.code(), Some(1), "{}", text(&alone));

    let r = Registry::new();
    r.appears_after(4);
    let shared = r.run(
        AWAIT,
        &["version", "@evnx/evnx-linux-x64", "0.10.7", "2"],
        &[
            interval[0].clone(),
            ("NPM_AWAIT_DEADLINE", (now() + 60).to_string()),
        ],
    );
    assert_eq!(shared.status.code(), Some(0), "{}", text(&shared));
}

/// I-256: at the shared deadline it gives up, names the package, and keeps the
/// diagnostics that tell "never published" from "no such package".
#[test]
fn t_i256_await_gives_up_at_the_shared_deadline() {
    let r = Registry::new();
    r.never_appears();
    let out = r.run(
        AWAIT,
        &["version", "@evnx/evnx-linux-x64", "0.10.7", "300"],
        &[("NPM_AWAIT_DEADLINE", (now() - 1).to_string())],
    );
    assert_eq!(out.status.code(), Some(1));
    let all = text(&out);
    assert!(all.contains("@evnx/evnx-linux-x64@0.10.7"), "{all}");
    assert!(all.contains("shared deadline"), "{all}");
    assert!(all.contains("this version was never published"), "{all}");
    assert_eq!(r.calls("view"), 1, "one look, then the deadline");
}

/// I-256: `exists` is one query and no waiting — 0 present, 1 not.
#[test]
fn t_i256_exists_is_one_query() {
    let r = Registry::new();
    r.appears_after(1);
    let yes = r.run(AWAIT, &["exists", "@evnx/cli", "0.10.7"], &[]);
    assert_eq!(yes.status.code(), Some(0), "{}", text(&yes));
    assert_eq!(r.calls("view"), 1);

    let r = Registry::new();
    r.never_appears();
    let no = r.run(AWAIT, &["exists", "@evnx/cli", "0.10.7"], &[]);
    assert_eq!(no.status.code(), Some(1), "{}", text(&no));
    assert_eq!(r.calls("view"), 1);

    assert_eq!(
        r.run(AWAIT, &["exists", "@evnx/cli"], &[]).status.code(),
        Some(2)
    );
}

// ── npm-publish-packages.sh ─────────────────────────────────────────────────

/// I-256: a version already in the registry is skipped without calling
/// `npm publish`, so a re-run gets past what an earlier attempt uploaded.
#[test]
fn t_i256_publish_skips_a_version_already_there() {
    if !has_node() {
        return;
    }
    let r = Registry::new();
    r.appears_after(1);
    let pkg = r.package("@evnx/evnx-linux-x64", "0.10.7");
    let out = r.run(PUBLISH, &[pkg.to_str().unwrap()], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert_eq!(r.calls("publish"), 0, "npm publish must not be called");
    assert!(text(&out).contains("already published"), "{}", text(&out));
}

/// I-256: a publish that fails while npm is still processing an earlier upload
/// counts as published once the version appears.
#[test]
fn t_i256_publish_waits_out_processing() {
    if !has_node() {
        return;
    }
    let r = Registry::new();
    // View 1 is the `exists` check; the version shows on view 3.
    r.appears_after(3).publish_answers(
        1,
        "npm ERR! code E409\nnpm ERR! 409 Conflict - PUT https://registry.npmjs.org/@evnx%2fevnx-linux-x64 - still processing\n",
    );
    let pkg = r.package("@evnx/evnx-linux-x64", "0.10.7");
    let out = r.run(PUBLISH, &[pkg.to_str().unwrap()], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(
        text(&out).contains("published by an earlier attempt"),
        "{}",
        text(&out)
    );
}

/// I-256: an authentication failure fails at once and says to check the
/// token. npm answers an unauthorised publish to an existing package with
/// "404 Not Found - PUT", which read like a missing package (v0.4.0).
#[test]
fn t_i256_publish_fails_fast_on_auth() {
    if !has_node() {
        return;
    }
    for answer in [
        "npm ERR! code E404\nnpm ERR! 404 Not Found - PUT https://registry.npmjs.org/@evnx%2fevnx-linux-x64 - Not found\n",
        "npm ERR! code ENEEDAUTH\nnpm ERR! need auth This command requires you to be logged in\n",
        "npm ERR! code E401\nnpm ERR! 401 Unauthorized - PUT https://registry.npmjs.org/@evnx%2fcli\n",
    ] {
        let r = Registry::new();
        r.never_appears().publish_answers(1, answer);
        let pkg = r.package("@evnx/evnx-linux-x64", "0.10.7");
        let started = Instant::now();
        let out = r.run(
            PUBLISH,
            &[pkg.to_str().unwrap()],
            &[("NPM_AWAIT_INTERVAL", "1".to_string())],
        );
        assert_eq!(out.status.code(), Some(1), "{}", text(&out));
        assert!(text(&out).contains("check NPM_TOKEN"), "{}", text(&out));
        assert_eq!(r.calls("view"), 1, "only the exists check, no polling");
        assert!(started.elapsed().as_secs() < 5, "an auth failure must not wait");
    }
}

/// I-256: a failed publish whose version never appears fails, with npm's
/// output, once the settle time or the deadline runs out.
#[test]
fn t_i256_publish_fails_when_nothing_appears() {
    if !has_node() {
        return;
    }
    let r = Registry::new();
    r.never_appears().publish_answers(
        1,
        "npm ERR! code E500\nnpm ERR! 500 Internal Server Error\n",
    );
    let pkg = r.package("@evnx/evnx-linux-x64", "0.10.7");
    let out = r.run(
        PUBLISH,
        &[pkg.to_str().unwrap()],
        &[
            ("NPM_AWAIT_INTERVAL", "1".to_string()),
            ("NPM_PUBLISH_SETTLE", "2".to_string()),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let all = text(&out);
    assert!(all.contains("500 Internal Server Error"), "{all}");
    assert!(all.contains("@evnx/evnx-linux-x64@0.10.7"), "{all}");
}

/// I-256: a partly published run, re-run: what is there is skipped, the
/// rest is published, in one pass.
#[test]
fn t_i256_a_rerun_publishes_only_what_is_missing() {
    if !has_node() {
        return;
    }
    let r = Registry::new();
    r.never_appears()
        .state("present", "@evnx/evnx-linux-x64@0.10.7\n")
        .publish_answers(0, "+ @evnx/evnx-win32-x64@0.10.7\n");
    let there = r.package("@evnx/evnx-linux-x64", "0.10.7");
    let missing = r.package("@evnx/evnx-win32-x64", "0.10.7");
    let out = r.run(
        PUBLISH,
        &[there.to_str().unwrap(), missing.to_str().unwrap()],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert_eq!(
        r.calls("publish"),
        1,
        "only the missing package is published"
    );
    let all = text(&out);
    assert!(
        all.contains("@evnx/evnx-linux-x64@0.10.7 is already published"),
        "{all}"
    );
    assert!(
        all.contains("@evnx/evnx-win32-x64@0.10.7 published"),
        "{all}"
    );
}

/// Wrong use is exit 2, as for npm-await.sh.
#[test]
fn t_i256_publish_needs_a_folder() {
    let r = Registry::new();
    assert_eq!(r.run(PUBLISH, &[], &[]).status.code(), Some(2));
    let nowhere = r.dir.path().join("no-such-package");
    assert_eq!(
        r.run(PUBLISH, &[nowhere.to_str().unwrap()], &[])
            .status
            .code(),
        Some(2)
    );
}

// ── the workflow ────────────────────────────────────────────────────────────

/// I-256: the publish job uses the scripts for both publishes, shares one
/// deadline per phase, and cannot hold a runner for six hours.
#[test]
fn t_i256_the_workflow_publishes_through_the_script() {
    let yml = std::fs::read_to_string(WORKFLOW).unwrap();
    let step = |name: &str| -> String {
        let start = yml
            .find(&format!("- name: {name}"))
            .unwrap_or_else(|| panic!("no step named {name}"));
        let rest = &yml[start + 1..];
        let end = rest.find("\n      - ").unwrap_or(rest.len());
        rest[..end].to_string()
    };
    for (name, call) in [
        (
            "Publish platform packages",
            "./scripts/npm-publish-packages.sh npm-publish/packages/*/",
        ),
        (
            "Publish wrapper package",
            "./scripts/npm-publish-packages.sh npm-publish/evnx",
        ),
    ] {
        let body = step(name);
        assert!(
            body.contains(call),
            "`{name}` does not run `{call}`:\n{body}"
        );
        assert!(
            !body.contains("npm publish"),
            "`{name}` still calls npm publish itself"
        );
    }
    for name in [
        "Publish platform packages",
        "Await platform packages in the registry",
        "Publish wrapper package",
        "Verify the registry and install",
    ] {
        assert!(
            step(name).contains("NPM_AWAIT_DEADLINE"),
            "`{name}` has no shared deadline"
        );
    }
    let job = &yml[yml.find("\n  publish:").expect("a publish job")..];
    let job = &job[..job.find("\n    steps:").unwrap()];
    assert!(
        job.contains("timeout-minutes:"),
        "the publish job has no time limit"
    );

    // It still parses, when PyYAML is there to say so.
    if let Ok(out) = Command::new("python3")
        .args([
            "-c",
            "import sys, yaml; yaml.safe_load(open(sys.argv[1]))",
            WORKFLOW,
        ])
        .output()
    {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.contains("No module named") {
            assert!(
                out.status.success(),
                "the workflow does not parse:\n{stderr}"
            );
        }
    }
}
