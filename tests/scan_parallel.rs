//! `evnx scan` reads files in parallel (RM-74) and reports them in path order.
//!
//! The report, the exit code and what goes to stderr are the same whatever
//! `RAYON_NUM_THREADS` is. No timing is asserted: speed is not a contract.
//!
//! ⚠️ Secret-shaped values are assembled at run time.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

fn aws(n: usize) -> String {
    format!("AKIA{}{:04}", "IOSFODNN7EXAM", n % 10000)
}

fn stripe() -> String {
    format!("sk_live_{}", "51Habc1234567890ABCDEFGH")
}

/// 12 folders of 12 files each, most with findings, of several kinds: dotenv,
/// text, YAML, a notebook, one with an `evnx:allow` marker that warns, and a
/// clean one. Names are chosen so path order differs from creation order.
fn tree() -> TempDir {
    let d = TempDir::new().unwrap();
    let mut n = 0;
    for dir in (0..12).rev() {
        let sub = d.path().join(format!("pkg{dir:02}")).join("src");
        fs::create_dir_all(&sub).unwrap();
        for file in (0..12).rev() {
            n += 1;
            let (name, body) = match file % 6 {
                0 => (
                    format!("f{file:02}.env"),
                    format!("DB_PASSWORD=correct-horse-{n}-battery\nKEY={}\n", aws(n)),
                ),
                1 => (format!("f{file:02}.txt"), format!("token {}\nok\n{}\n", aws(n), stripe())),
                2 => (
                    format!("f{file:02}.yml"),
                    format!("app:\n  password: correct-horse-{n}-staple\n  k: {}\n", aws(n)),
                ),
                3 => (
                    format!("f{file:02}.ipynb"),
                    serde_json::json!({"cells": [{"cell_type": "code", "source": [format!("k = '{}'\n", aws(n))],
                        "outputs": [{"output_type": "stream", "name": "stdout", "text": [format!("{}\n", aws(n + 1))]}]}],
                        "metadata": {}, "nbformat": 4, "nbformat_minor": 5})
                    .to_string(),
                ),
                4 => (
                    format!("f{file:02}.py"),
                    // A marker with no reason: a warning on stderr.
                    format!("k = '{}'  # evnx:allow aws-access-key\n", aws(n)),
                ),
                _ => (format!("f{file:02}.md"), "nothing to see\n".to_string()),
            };
            fs::write(sub.join(name), body).unwrap();
        }
    }
    d
}

fn run(d: &TempDir, threads: &str, args: &[&str]) -> (Option<i32>, String, String) {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .env("RAYON_NUM_THREADS", threads)
        .arg("scan")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn output_is_identical_across_thread_counts_in_every_format() {
    let d = tree();
    for args in [
        vec!["--format", "json"],
        vec!["--format", "sarif"],
        vec!["--format", "github"],
        vec![],
        vec!["-v", "--format", "json"],
        vec!["--include-ignored", "--format", "json"],
    ] {
        let one = run(&d, "1", &args);
        let four = run(&d, "4", &args);
        assert_eq!(one.0, Some(1), "{args:?}: secrets found");
        assert_eq!(one, four, "{args:?} differs between 1 and 4 threads");
        assert!(
            one.1.len() > 1000,
            "{args:?}: fixture should produce output"
        );
    }
}

#[test]
fn findings_are_in_path_order() {
    let d = tree();
    let (_, stdout, _) = run(&d, "4", &["--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let files: Vec<String> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(files.len(), 12 * 12);
    let mut sorted = files.clone();
    sorted.sort();
    assert_eq!(files, sorted);
    // Findings follow their files.
    let locs: Vec<&str> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["location"].as_str().unwrap())
        .collect();
    let at = |loc: &str| {
        files
            .iter()
            .position(|f| loc.starts_with(f.as_str()))
            .unwrap()
    };
    assert!(locs.windows(2).all(|w| at(w[0]) <= at(w[1])), "{locs:?}");
}

#[test]
fn the_exit_code_is_the_same_clean_and_dirty() {
    let d = TempDir::new().unwrap();
    for i in 0..30 {
        fs::write(d.path().join(format!("n{i:02}.txt")), "nothing\n").unwrap();
    }
    for t in ["1", "4"] {
        assert_eq!(run(&d, t, &["--format", "json"]).0, Some(0));
    }
    fs::write(d.path().join("n17.txt"), format!("{}\n", aws(1))).unwrap();
    for t in ["1", "4"] {
        assert_eq!(run(&d, t, &["--format", "json"]).0, Some(1));
        assert_eq!(run(&d, t, &["--format", "json", "--exit-zero"]).0, Some(0));
    }
}
