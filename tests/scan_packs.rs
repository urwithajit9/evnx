//! Rule packs: `evnx scan rules add | update | remove`, `rules.lock`, and the
//! gitleaks importer (C7; S2, S4, S6).
//!
//! The catalogue is a local mockito server: no test touches the internet.
//! ⚠️ Fixture keys are assembled at run time.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

fn evnx(project: &Path, home: &Path, catalogue: &str, args: &[&str]) -> Output {
    cargo_bin_cmd!("evnx")
        .current_dir(project)
        .env("NO_COLOR", "1")
        .env("EVNX_CONFIG_DIR", home)
        .env("EVNX_RULES_CATALOGUE", catalogue)
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn pack(ids: &[&str]) -> String {
    let mut s = String::from("version = 1\n");
    for id in ids {
        s.push_str(&format!(
            "\n[[rules]]\nid = \"{id}\"\nname = \"{id}\"\nregex = '{}-[0-9]{{8}}'\nseverity = \"high\"\n",
            id.to_uppercase()
        ));
    }
    s
}

fn sha(s: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A path source: added, locked, scanned, tampered with and refused.
#[test]
fn a_pack_from_a_path_is_locked_and_a_changed_one_is_refused() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    fs::write(p.path().join("acme.toml"), pack(&["acme"])).unwrap();
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "rules", "add", "acme.toml"],
    );
    assert!(out.status.success(), "{}", text(&out));
    let lock = fs::read_to_string(p.path().join(".evnx/rules.lock")).unwrap();
    assert!(lock.contains(&sha(&pack(&["acme"]))), "{lock}");

    fs::write(p.path().join("notes.txt"), "ACME-12345678\n").unwrap();
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "notes.txt"],
    );
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));

    // Edited by hand: refused by name, with the way out.
    let f = p.path().join(".evnx/rules/acme.toml");
    fs::write(&f, pack(&["acme"]).replace("high", "low")).unwrap();
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "notes.txt"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out).contains(".evnx/rules/acme.toml changed since it was added"),
        "{}",
        text(&out)
    );

    // `update --yes` restores it from its source; then `remove` takes it away.
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "rules", "update", "--yes"],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(fs::read_to_string(&f).unwrap(), pack(&["acme"]));
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "rules", "remove", "acme"],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(!f.exists());
    assert!(!fs::read_to_string(p.path().join(".evnx/rules.lock"))
        .unwrap()
        .contains("acme"));
}

#[test]
fn a_hand_written_rule_file_is_not_a_pack() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    fs::create_dir_all(p.path().join(".evnx/rules")).unwrap();
    fs::write(p.path().join(".evnx/rules/mine.toml"), pack(&["mine"])).unwrap();
    fs::write(p.path().join("mine.toml"), pack(&["mine-two"])).unwrap();
    // Not hash-checked: a scan runs.
    fs::write(p.path().join("a.txt"), "x\n").unwrap();
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "a.txt"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    // `add` will not write over it, and `remove` will not delete it.
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "rules", "add", "mine.toml"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out).contains("is not a pack"), "{}", text(&out));
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "rules", "remove", "mine"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(p.path().join(".evnx/rules/mine.toml").exists());
}

/// S5: a pack whose id is evnx's own is refused before anything is written.
#[test]
fn a_pack_that_reuses_a_core_id_is_refused() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    fs::write(p.path().join("dup.toml"), pack(&["aws-access-key"])).unwrap();
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "rules", "add", "dup.toml"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out).contains("evnx's own rules"), "{}", text(&out));
    assert!(!p.path().join(".evnx/rules/dup.toml").exists());
}

/// The pinned upstream file: exact counts, so an upstream change is a diff
/// here, and the one rule left out is named.
#[test]
fn the_pinned_gitleaks_rules_convert() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gitleaks/gitleaks.toml");
    fs::copy(&fixture, p.path().join("gitleaks.toml")).unwrap();
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &[
            "scan",
            "rules",
            "add",
            "gitleaks.toml",
            "--format",
            "gitleaks",
        ],
    );
    let all = text(&out);
    assert!(out.status.success(), "{all}");
    assert!(all.contains("added gitleaks: 221 rule(s)"), "{all}");
    assert!(all.contains("pkcs12-file: no regex"), "{all}");
    let converted = fs::read_to_string(p.path().join(".evnx/rules/gitleaks.toml")).unwrap();
    assert!(converted.contains("id = \"gitleaks-aws-access-token\""));
    assert!(converted.contains("keywords = ["));

    // A gitleaks rule finds a token evnx's own rules have no rule for, and
    // reports its secret group: the key, not the `adafruit_key = ` before it.
    let key = format!("k3x9q2m7v1zpab4c{}", "h8n5w0r6t2y4u1i3");
    fs::write(
        p.path().join("a.txt"),
        format!("adafruit_key = \"{key}\"\n"),
    )
    .unwrap();
    let out = evnx(
        p.path(),
        h.path(),
        "http://127.0.0.1:9/",
        &["scan", "--format", "json", "a.txt"],
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<&str> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule_id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"gitleaks-adafruit-api-key"), "{ids:?}");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains(&key),
        "printed in full"
    );
}

#[cfg(feature = "net")]
mod catalogue {
    use super::*;

    fn index(sha256: &str) -> String {
        format!(
            "version = 1\n[[pack]]\nname = \"acme\"\npath = \"packs/acme.toml\"\nsha256 = \"{sha256}\"\nrules = 1\n"
        )
    }

    #[test]
    fn a_catalogue_pack_is_added_by_name_and_must_match_the_index() {
        let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let mut server = mockito::Server::new();
        let body = pack(&["acme"]);
        let _i = server
            .mock("GET", "/index.toml")
            .with_body(index(&sha(&body)))
            .create();
        let _p = server
            .mock("GET", "/packs/acme.toml")
            .with_body(&body)
            .create();
        let cat = format!("{}/index.toml", server.url());
        let out = evnx(p.path(), h.path(), &cat, &["scan", "rules", "add", "acme"]);
        assert!(out.status.success(), "{}", text(&out));
        assert_eq!(
            fs::read_to_string(p.path().join(".evnx/rules/acme.toml")).unwrap(),
            body
        );
        let lock = fs::read_to_string(p.path().join(".evnx/rules.lock")).unwrap();
        assert!(lock.contains("kind = \"catalogue\""), "{lock}");

        // An unknown name lists what the catalogue has.
        let out = evnx(p.path(), h.path(), &cat, &["scan", "rules", "add", "nope"]);
        assert_eq!(out.status.code(), Some(2));
        assert!(text(&out).contains("It has: acme"), "{}", text(&out));
    }

    #[test]
    fn a_download_that_does_not_match_the_index_is_refused() {
        let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let mut server = mockito::Server::new();
        let _i = server
            .mock("GET", "/index.toml")
            .with_body(index(&sha("something else")))
            .create();
        let _p = server
            .mock("GET", "/packs/acme.toml")
            .with_body(pack(&["acme"]))
            .create();
        let cat = format!("{}/index.toml", server.url());
        let out = evnx(p.path(), h.path(), &cat, &["scan", "rules", "add", "acme"]);
        assert_eq!(out.status.code(), Some(2));
        assert!(
            text(&out).contains("does not match the sha256"),
            "{}",
            text(&out)
        );
        assert!(!p.path().join(".evnx/rules/acme.toml").exists());
    }

    /// An update shows what changes and asks; with no terminal it needs
    /// `--yes`, and `--yes` replaces it.
    #[test]
    fn update_shows_the_change_and_asks() {
        let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let mut server = mockito::Server::new();
        let v1 = pack(&["acme"]);
        let i1 = server
            .mock("GET", "/index.toml")
            .with_body(index(&sha(&v1)))
            .create();
        let p1 = server
            .mock("GET", "/packs/acme.toml")
            .with_body(&v1)
            .create();
        let cat = format!("{}/index.toml", server.url());
        assert!(
            evnx(p.path(), h.path(), &cat, &["scan", "rules", "add", "acme"])
                .status
                .success()
        );
        i1.remove();
        p1.remove();

        let v2 = pack(&["acme", "acme-two"]);
        let _i = server
            .mock("GET", "/index.toml")
            .with_body(index(&sha(&v2)))
            .create();
        let _p = server
            .mock("GET", "/packs/acme.toml")
            .with_body(&v2)
            .create();
        let out = evnx(
            p.path(),
            h.path(),
            &cat,
            &["scan", "rules", "update", "acme"],
        );
        assert_eq!(out.status.code(), Some(2), "{}", text(&out));
        assert!(text(&out).contains("acme: 1 → 2 rules"), "{}", text(&out));
        assert!(text(&out).contains("+ acme-two"), "{}", text(&out));
        assert!(text(&out).contains("--yes"), "{}", text(&out));

        let out = evnx(
            p.path(),
            h.path(),
            &cat,
            &["scan", "rules", "update", "acme", "-y"],
        );
        assert!(out.status.success(), "{}", text(&out));
        assert_eq!(
            fs::read_to_string(p.path().join(".evnx/rules/acme.toml")).unwrap(),
            v2
        );
    }

    /// A scan with packs installed never fetches: the catalogue is not asked.
    #[test]
    fn a_scan_makes_no_network_request() {
        let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let mut server = mockito::Server::new();
        let body = pack(&["acme"]);
        let _i = server
            .mock("GET", "/index.toml")
            .with_body(index(&sha(&body)))
            .create();
        let _p = server
            .mock("GET", "/packs/acme.toml")
            .with_body(&body)
            .create();
        let cat = format!("{}/index.toml", server.url());
        assert!(
            evnx(p.path(), h.path(), &cat, &["scan", "rules", "add", "acme"])
                .status
                .success()
        );
        let watch = server.mock("GET", mockito::Matcher::Any).expect(0).create();
        fs::write(p.path().join("a.txt"), "ACME-12345678\n").unwrap();
        let out = evnx(p.path(), h.path(), &cat, &["scan", "a.txt"]);
        assert_eq!(out.status.code(), Some(1), "{}", text(&out));
        watch.assert();
    }

    #[test]
    fn only_https_is_downloaded_from_another_host() {
        let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let out = evnx(
            p.path(),
            h.path(),
            "http://127.0.0.1:9/",
            &["scan", "rules", "add", "http://example.com/acme.toml"],
        );
        assert_eq!(out.status.code(), Some(2));
        assert!(text(&out).contains("over https only"), "{}", text(&out));
    }
}
