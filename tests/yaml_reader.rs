//! How evnx reads YAML (`core::yaml`, on `yaml-rust2`), pinned.
//!
//! evnx reads YAML in three places: the scan engine's structured leaves,
//! `init`'s compose detection and `add import`. All three go through
//! `core::yaml`, which replaced `serde_yaml` in 0.16.0. These tests pin what a
//! YAML file means to evnx, so a change of reader or of rules shows up here
//! first:
//!
//! * the YAML 1.1 words `yes`, `no`, `on`, `off`, `y` are **strings** (YAML
//!   1.2), so scan checks them and import writes them as text;
//! * `true`/`false`, `~`/`null` and numbers are not strings: scan skips them,
//!   import writes `true`/`false` and the number **as written** (`1.10`);
//! * anchors and aliases are resolved; the merge key `<<` is **not** applied
//!   (it is an ordinary key holding the merged mapping);
//! * every document of a `---` stream is read;
//! * a key given twice: scan reads the file as text instead (so both values
//!   are seen), import takes the last one;
//! * an error is a line, a column and a fixed reason, never text from the file.

mod support;

use evnx::core::import::{Format as ImportFormat, read};
use evnx::core::secretscan::engine::{Format, leaves};
use evnx::core::yaml::{self, Duplicates, Value};
use support::{Project, run};

fn doc(text: &str) -> Value {
    yaml::document(text, Duplicates::Refuse).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

fn n(v: &str) -> Value {
    Value::Number(v.into())
}

/// `(path, value)` of every string leaf scan sees, or `None` when scan reads
/// the file as text.
fn scan(text: &str) -> Option<Vec<(String, String)>> {
    leaves(text, Format::Yaml).map(|ls| ls.into_iter().map(|l| (l.path, l.value)).collect())
}

/// `(source, name, value or refusal)` of `add import`'s reader.
fn import(text: &str) -> Vec<(String, String, Result<String, String>)> {
    read(ImportFormat::Yaml, text, None)
        .unwrap_or_else(|e| panic!("{e}\n{text}"))
        .into_iter()
        .map(|f| (f.source, f.name, f.value.map(|v| v.expose().to_string())))
        .collect()
}

fn ok(source: &str, name: &str, value: &str) -> (String, String, Result<String, String>) {
    (source.into(), name.into(), Ok(value.into()))
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

// ── scalars ──────────────────────────────────────────────────────────────

#[test]
fn yaml_1_1_booleans_are_strings_and_scan_and_import_keep_them() {
    let text = "a: yes\nb: no\nc: on\nd: off\ne: Yes\nf: y\n";
    let d = doc(text);
    for (k, v) in [
        ("a", "yes"),
        ("b", "no"),
        ("c", "on"),
        ("d", "off"),
        ("e", "Yes"),
        ("f", "y"),
    ] {
        assert_eq!(d.get(k), Some(&s(v)), "{k}");
    }
    assert_eq!(
        scan(text).unwrap(),
        pairs(&[
            ("a", "yes"),
            ("b", "no"),
            ("c", "on"),
            ("d", "off"),
            ("e", "Yes"),
            ("f", "y")
        ])
    );
    assert_eq!(import(text)[2], ok("c", "C", "on"));
    assert_eq!(import(text).len(), 6);
}

#[test]
fn true_and_false_are_booleans_that_scan_skips_and_import_writes_as_words() {
    let text = "a: true\nb: False\nc: TRUE\nd: 'true'\n";
    let d = doc(text);
    assert_eq!(d.get("a"), Some(&Value::Bool(true)));
    assert_eq!(d.get("b"), Some(&Value::Bool(false)));
    assert_eq!(d.get("c"), Some(&Value::Bool(true)));
    assert_eq!(d.get("d"), Some(&s("true")), "quoted is a string");
    assert_eq!(scan(text).unwrap(), pairs(&[("d", "true")]));
    assert_eq!(
        import(text),
        [
            ok("a", "A", "true"),
            ok("b", "B", "false"),
            ok("c", "C", "true"),
            ok("d", "D", "true")
        ]
    );
}

#[test]
fn tilde_null_and_nothing_are_null_and_import_refuses_them() {
    let text = "a: ~\nb: null\nc:\nd: Null\ne: \"~\"\n";
    let d = doc(text);
    for k in ["a", "b", "c", "d"] {
        assert_eq!(d.get(k), Some(&Value::Null), "{k}");
    }
    assert_eq!(d.get("e"), Some(&s("~")));
    assert_eq!(scan(text).unwrap(), pairs(&[("e", "~")]));
    let got = import(text);
    for row in &got[..4] {
        assert!(row.2.as_ref().unwrap_err().contains("null"), "{row:?}");
    }
    assert_eq!(got[4], ok("e", "E", "~"));
}

#[test]
fn numbers_are_kept_as_written_and_a_leading_zero_is_a_string() {
    let text = "v: 1.10\nw: 1.0\nhex: 0x1F\noct: 0o17\nexp: 1e3\ninf: .inf\nneg: -12\n\
                plus: +1\nbig: 12345678901234567890123\nzip: 007\nsep: 1_000\nword: inf\n";
    let d = doc(text);
    for (k, v) in [
        ("v", "1.10"),
        ("w", "1.0"),
        ("hex", "0x1F"),
        ("oct", "0o17"),
        ("exp", "1e3"),
        ("inf", ".inf"),
        ("neg", "-12"),
        ("plus", "+1"),
        ("big", "12345678901234567890123"),
    ] {
        assert_eq!(d.get(k), Some(&n(v)), "{k}");
    }
    for (k, v) in [("zip", "007"), ("sep", "1_000"), ("word", "inf")] {
        assert_eq!(d.get(k), Some(&s(v)), "{k}");
    }
    // Scan checks strings only.
    assert_eq!(
        scan(text).unwrap(),
        pairs(&[("zip", "007"), ("sep", "1_000"), ("word", "inf")])
    );
    // Import writes the text a person typed: a version stays `1.10`.
    let got = import(text);
    assert_eq!(got[0], ok("v", "V", "1.10"));
    assert_eq!(got[2], ok("hex", "HEX", "0x1F"));
    assert_eq!(got[8], ok("big", "BIG", "12345678901234567890123"));
}

#[test]
fn quoted_and_block_scalars_are_strings() {
    let text =
        "a: \"1.10\"\nb: 'yes'\nc: |\n  l1\n  l2\nd: >\n  f1\n  f2\ne: !!str 123\nf: !Ref thing\n";
    let d = doc(text);
    assert_eq!(d.get("a"), Some(&s("1.10")));
    assert_eq!(d.get("b"), Some(&s("yes")));
    assert_eq!(d.get("c"), Some(&s("l1\nl2\n")));
    assert_eq!(d.get("d"), Some(&s("f1 f2\n")));
    assert_eq!(d.get("e"), Some(&s("123")), "a core tag picks the type");
    assert_eq!(d.get("f"), Some(&s("thing")), "another tag is ignored");
    assert_eq!(doc("x: !!int \"7\"\n").get("x"), Some(&n("7")));
}

// ── structure ────────────────────────────────────────────────────────────

#[test]
fn anchors_and_aliases_are_resolved() {
    let text = "base: &b\n  token: abc\nprod:\n  x: *b\nlist: &l [p, q]\nother: *l\n";
    let d = doc(text);
    assert_eq!(d.get("prod").and_then(|p| p.get("x")), d.get("base"));
    assert_eq!(d.get("other"), d.get("list"));
    assert_eq!(
        scan(text).unwrap(),
        pairs(&[
            ("base.token", "abc"),
            ("prod.x.token", "abc"),
            ("list[0]", "p"),
            ("list[1]", "q"),
            ("other[0]", "p"),
            ("other[1]", "q"),
        ])
    );
}

/// The merge key is not applied, by evnx before 0.16.0 either: `<<` is a key
/// like any other, so scan still sees every value under it.
#[test]
fn the_merge_key_is_not_applied() {
    let text = "base: &b\n  token: abc\nprod:\n  <<: *b\n  host: h\n";
    let d = doc(text);
    let prod = d.get("prod").unwrap();
    assert_eq!(prod.get("token"), None);
    assert_eq!(prod.get("<<"), d.get("base"));
    assert_eq!(
        scan(text).unwrap(),
        pairs(&[
            ("base.token", "abc"),
            ("prod.<<.token", "abc"),
            ("prod.host", "h")
        ])
    );
    assert_eq!(import(text)[1].0, "prod.<<.token");
}

#[test]
fn every_document_of_a_stream_is_read() {
    let text = "a: one\n---\nb: two\n---\n";
    let docs = yaml::documents(text, Duplicates::Refuse).unwrap();
    assert_eq!(docs.len(), 3);
    assert!(docs[2].is_null(), "an empty document is null");
    assert_eq!(scan(text).unwrap(), pairs(&[("a", "one"), ("b", "two")]));
    assert_eq!(import(text), [ok("a", "A", "one"), ok("b", "B", "two")]);
    // One document is asked for: two is an error.
    assert!(yaml::document(text, Duplicates::Refuse).is_err());
}

#[test]
fn a_kubernetes_secret_scans_its_base64_data_and_imports_it_decoded() {
    let text = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: x\ntype: Opaque\n\
                data:\n  DB_PASS: aHVudGVyMg==\n";
    let leaves = scan(text).unwrap();
    assert!(
        leaves.contains(&("data.DB_PASS".to_string(), "aHVudGVyMg==".to_string())),
        "{leaves:?}"
    );
    assert_eq!(import(text), [ok("data.DB_PASS", "DB_PASS", "hunter2")]);
}

#[test]
fn a_key_given_twice_sends_scan_to_text_and_import_takes_the_last() {
    let text = "a: first\nb: y\na: second\n";
    assert_eq!(scan(text), None, "read as text, so both values are seen");
    assert_eq!(import(text), [ok("a", "A", "second"), ok("b", "B", "y")]);
}

#[test]
fn keys_are_named_as_written() {
    let text = "1: one\ntrue: t\n1.10: v\n~: n\n";
    assert_eq!(
        scan(text).unwrap(),
        pairs(&[("1", "one"), ("true", "t"), ("1.10", "v"), ("~", "n")])
    );
}

#[test]
fn a_byte_order_mark_is_skipped() {
    assert_eq!(doc("\u{feff}a: b\n").get("a"), Some(&s("b")));
}

/// A difference from `serde_yaml`: a tab right after `key:` is a syntax error
/// for `yaml-rust2`. Scan reads such a file as text; import says where.
#[test]
fn a_tab_after_the_colon_is_an_error() {
    assert!(yaml::document("a:\tb\n", Duplicates::Refuse).is_err());
    assert_eq!(scan("a:\tb\n"), None);
    assert_eq!(doc("a: \tb\n").get("a"), Some(&s("b")));
}

// ── errors ───────────────────────────────────────────────────────────────

const CANARY: &str = "yaml-canary-5b02e7d1";

#[test]
fn an_error_is_a_position_and_never_text_from_the_file() {
    for text in [
        format!("a: '{CANARY}\n  b: ["),
        format!("a: {CANARY}\n b: c\n"),
        format!("{CANARY}: 1\n{CANARY}: 2\n"),
        format!("? [{CANARY}]\n: v\n"),
        format!("a: *{CANARY}\n"),
        format!("a: \"{CANARY}\t\n"),
    ] {
        let e = yaml::documents(&text, Duplicates::Refuse)
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("not valid YAML (line "), "{e}");
        assert!(!e.contains(CANARY), "{e}");
        assert!(!e.contains("yaml-canary"), "{e}");
    }
}

#[test]
fn add_import_of_a_broken_yaml_file_names_a_line_and_no_value() {
    let p = Project::new()
        .env("KEEP=1\n")
        .example("KEEP=\n")
        .file("bad.yaml", &format!("token: '{CANARY}\n  other: ["));
    let out = run(p.path(), &["add", "import", "bad.yaml", "--yes"]);
    out.assert_code(2);
    out.assert_contains("not valid YAML (line ");
    assert!(!out.all().contains(CANARY), "{}", out.all());
    assert_eq!(p.read(".env"), "KEEP=1\n");
}

#[test]
fn deep_nesting_and_alias_bombs_are_refused_not_followed() {
    let deep = format!("a: {}x{}\n", "[".repeat(300), "]".repeat(300));
    assert!(yaml::documents(&deep, Duplicates::Refuse).is_err());
    assert_eq!(scan(&deep), None);
    let mut bomb = String::from("a: &a [x, x, x, x, x, x, x, x, x, x]\n");
    let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    for w in names.windows(2) {
        let refs = vec![format!("*{}", w[0]); 10].join(", ");
        bomb.push_str(&format!("{}: &{} [{refs}]\n", w[1], w[1]));
    }
    let e = yaml::documents(&bomb, Duplicates::Refuse).unwrap_err();
    assert!(e.reason.contains("alias"), "{e}");
}
