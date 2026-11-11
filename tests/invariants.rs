//! evnx accepts what evnx generates (F14-8, OW-17).
//!
//! The suites elsewhere check one command against hand-written input. This one
//! chains commands: what `init`, `convert`, `migrate --print-only`, `sync`,
//! `add` and `remove` write is fed back to evnx's own readers, and to an
//! independent one (`serde_json`, `serde_yaml`, `toml`, `sh`) where a format has
//! one. A disagreement is a bug in the writer or the reader, never in the test:
//! the expected values are written out below, and a self-check holds the
//! fixture to what they say.
//!
//! Cases are collected and reported together, so one run names every failure
//! instead of the first.
//!
//! Windows: output is normalised (CRLF), paths use `Path::join`, and everything
//! that needs `sh` or a stub program is `cfg(unix)`.

mod support;

use std::collections::BTreeMap;
use support::{run, Project};

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Line endings do not count as a difference.
fn lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// The first `{…}` document in `s`: evnx prints the config note ahead of JSON
/// on some paths.
fn json_in(s: &str) -> serde_json::Value {
    let start = s
        .find('{')
        .unwrap_or_else(|| panic!("no JSON document in:\n{s}"));
    serde_json::from_str(&s[start..]).unwrap_or_else(|e| panic!("bad JSON ({e}) in:\n{s}"))
}

/// What evnx's own parser reads from `file`: `evnx parse --values`.
fn parsed(p: &Project, file: &str) -> Result<BTreeMap<String, String>, String> {
    let out = run(
        p.path(),
        &["parse", "--env", file, "--format", "json", "--values"],
    );
    if out.code != Some(0) {
        return Err(format!(
            "`evnx parse {file}` exited {:?}:\n{}",
            out.code,
            out.all()
        ));
    }
    let doc = json_in(&out.stdout);
    let mut map = BTreeMap::new();
    for e in doc["entries"].as_array().ok_or("no entries")? {
        map.insert(
            e["key"].as_str().ok_or("no key")?.to_string(),
            lf(e["value"].as_str().unwrap_or("")),
        );
    }
    Ok(map)
}

/// Failures gathered across a loop of cases.
#[derive(Default)]
struct Failures(Vec<String>);

impl Failures {
    fn add(&mut self, case: &str, what: impl std::fmt::Display) {
        self.0.push(format!("[{case}] {what}"));
    }
    fn check(&mut self, ok: bool, case: &str, what: impl std::fmt::Display) {
        if !ok {
            self.add(case, what);
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} invariant failure(s):\n\n{}",
            self.0.len(),
            self.0.join("\n\n")
        );
    }
}

// ── 1. init ─────────────────────────────────────────────────────────────────

/// Every component and blueprint id, read from the schema `init` ships.
fn init_ids() -> (Vec<String>, Vec<String>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("assets")
        .join("schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut components = Vec::new();
    for lang in schema["languages"].as_object().unwrap().values() {
        components.extend(lang["frameworks"].as_object().unwrap().keys().cloned());
    }
    for group in schema["services"].as_object().unwrap().values() {
        components.extend(group.as_object().unwrap().keys().cloned());
    }
    components.extend(
        schema["infrastructure"]
            .as_object()
            .unwrap()
            .keys()
            .cloned(),
    );
    let blueprints = schema["stacks"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    (components, blueprints)
}

/// The three checks every generated project must pass, whatever made it.
fn check_generated_project(f: &mut Failures, case: &str, p: &Project) {
    // validate: a fresh .env holds placeholders the user is told to replace, so
    // `placeholder_value` is the one error that is the design. Any other error
    // (a missing variable, a bad format) is evnx rejecting what evnx wrote.
    // Warnings stay warnings: Django's `DEBUG=True` is a `boolean_trap` warning
    // by a standing decision (T-INIT-32).
    let v = run(p.path(), &["validate", "--format", "json"]);
    match v.code {
        Some(0) | Some(1) => {
            let doc = json_in(&v.stdout);
            for issue in doc["issues"].as_array().into_iter().flatten() {
                let ty = issue["type"].as_str().unwrap_or("?");
                if issue["severity"] == "error" && ty != "placeholder_value" {
                    f.add(
                        case,
                        format!(
                            "validate rejects what init wrote: {ty} on {} ({})",
                            issue["variable"], issue["message"]
                        ),
                    );
                }
            }
        }
        _ => f.add(case, format!("validate did not run:\n{}", v.all())),
    }

    // doctor: warnings are fine (no git repo here), errors are not.
    let d = run(p.path(), &["doctor", "--format", "json"]);
    f.check(
        d.code == Some(0),
        case,
        format!("doctor exited {:?}:\n{}", d.code, d.all()),
    );
    if let Some(start) = d.stdout.find('{') {
        if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&d.stdout[start..]) {
            for c in doc["checks"].as_array().into_iter().flatten() {
                f.check(
                    c["severity"] != "error",
                    case,
                    format!("doctor error: {} {}", c["name"], c["details"]),
                );
            }
        }
    }

    // scan: a placeholder must not look like a secret.
    let s = run(p.path(), &["scan"]);
    f.check(
        s.code == Some(0),
        case,
        format!("scan flags what init wrote ({:?}):\n{}", s.code, s.all()),
    );

    // The files are readable by the parser.
    for file in [".env", ".env.example"] {
        if let Err(e) = parsed(p, file) {
            f.add(case, e);
        }
    }
    f.check(
        p.join(".evnx.toml").exists(),
        case,
        "init wrote no .evnx.toml",
    );
}

#[test]
fn init_output_is_accepted_for_every_component() {
    let (components, _) = init_ids();
    assert!(components.len() > 40, "schema read wrong: {components:?}");
    let mut f = Failures::default();
    for id in &components {
        let p = Project::new();
        let out = run(p.path(), &["init", "--yes", "--with", id]);
        if out.code != Some(0) {
            f.add(
                id,
                format!("init --with {id} exited {:?}:\n{}", out.code, out.all()),
            );
            continue;
        }
        check_generated_project(&mut f, id, &p);
    }
    f.finish();
}

#[test]
fn init_output_is_accepted_for_every_blueprint() {
    let (_, blueprints) = init_ids();
    assert!(blueprints.len() >= 8, "schema read wrong: {blueprints:?}");
    let mut f = Failures::default();
    for id in &blueprints {
        let p = Project::new();
        let out = run(p.path(), &["init", "--yes", "--blueprint", id]);
        if out.code != Some(0) {
            f.add(
                id,
                format!(
                    "init --blueprint {id} exited {:?}:\n{}",
                    out.code,
                    out.all()
                ),
            );
            continue;
        }
        check_generated_project(&mut f, id, &p);
    }
    f.finish();
}

#[test]
fn init_output_is_accepted_for_a_mixed_stack() {
    let p = Project::new();
    run(
        p.path(),
        &[
            "init",
            "--yes",
            "--with",
            "nextjs,postgresql,stripe,anthropic,aws,docker",
        ],
    )
    .assert_code(0);
    let mut f = Failures::default();
    check_generated_project(&mut f, "mixed", &p);
    f.finish();
}

// ── 2. convert ──────────────────────────────────────────────────────────────

/// `(key, the line as written in .env, the value evnx must read from it)`.
///
/// Quotes, `$`, `#`, spaces, `=`, unicode, empty, a multi-line value, and the
/// words YAML and friends read as something other than a string.
const FIXTURE: &[(&str, &str, &str)] = &[
    ("PLAIN", "PLAIN=plain", "plain"),
    ("SOME_KEY", "SOME_KEY=under_score", "under_score"),
    (
        "DQUOTES",
        r#"DQUOTES="he said \"hi\" it's""#,
        r#"he said "hi" it's"#,
    ),
    (
        "DOLLAR",
        "DOLLAR='cost $5 and ${HOME}'",
        "cost $5 and ${HOME}",
    ),
    ("HASH", r#"HASH="a#b""#, "a#b"),
    ("HASHSP", r#"HASHSP="a #b""#, "a #b"),
    ("SPACES", r#"SPACES="  two  words  ""#, "  two  words  "),
    ("EQ", "EQ=a=b=c", "a=b=c"),
    ("UNI", r#"UNI="héllo wörld ✓ 日本""#, "héllo wörld ✓ 日本"),
    ("EMPTY", "EMPTY=", ""),
    ("MULTI", "MULTI=\"line1\nline2\"", "line1\nline2"),
    ("BACKSL", r"BACKSL='C:\new\table'", r"C:\new\table"),
    ("TICKS", "TICKS='`cmd` $(cmd)'", "`cmd` $(cmd)"),
    ("T", "T=true", "true"),
    ("N", "N=123", "123"),
    ("NUL", "NUL=null", "null"),
    ("TILDE", "TILDE=~", "~"),
    ("HEX", "HEX=0x1F", "0x1F"),
    ("YES", "YES=yes", "yes"),
    ("NO", "NO=no", "no"),
    ("ON", "ON=on", "on"),
    ("FLOAT", "FLOAT=1e3", "1e3"),
    ("DASH", r#"DASH="- item""#, "- item"),
    ("COLON", r#"COLON="key: value""#, "key: value"),
    ("COLONEND", r#"COLONEND="end:""#, "end:"),
    ("TAG", r#"TAG="!tag""#, "!tag"),
    ("ANCHOR", r#"ANCHOR="&anchor""#, "&anchor"),
    ("ALIAS", r#"ALIAS="*alias""#, "*alias"),
    ("AT", r#"AT="@at""#, "@at"),
    ("PCT", r#"PCT="%pct""#, "%pct"),
    ("BRACE", r#"BRACE="{brace}""#, "{brace}"),
    ("BRACKET", r#"BRACKET="[bracket]""#, "[bracket]"),
    ("SQ", r#"SQ="'single'""#, "'single'"),
    ("DATE", "DATE=2024-01-01", "2024-01-01"),
];

fn fixture_env() -> String {
    let mut s = String::new();
    for (_, line, _) in FIXTURE {
        s.push_str(line);
        s.push('\n');
    }
    s
}

fn fixture_values() -> BTreeMap<String, String> {
    FIXTURE
        .iter()
        .map(|(k, _, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The fixture means what the table says: evnx's parser agrees with it.
#[test]
fn the_fixture_reads_as_written() {
    let p = Project::new().env(&fixture_env());
    assert_eq!(parsed(&p, ".env").unwrap(), fixture_values());
}

/// The name a format gives a key (some lowercase it, some cannot hold `_`).
fn norm(format: &str, key: &str) -> String {
    match format {
        "terraform" => key.to_lowercase(),
        "gcp-secrets" => key.to_lowercase().replace('_', "-"),
        "azure-keyvault" => key.replace('_', "-"),
        _ => key.to_string(),
    }
}

fn expected_for(format: &str) -> BTreeMap<String, String> {
    fixture_values()
        .into_iter()
        .map(|(k, v)| (norm(format, &k), v))
        .collect()
}

/// The flags a format cannot run without.
fn convert_args(format: &str) -> Vec<&'static str> {
    let mut a = vec!["convert", "--to"];
    a.push(Box::leak(format.to_string().into_boxed_str()));
    match format {
        "gcp-secrets" => a.extend(["--project", "my-project"]),
        "azure-keyvault" => a.extend(["--vault-name", "my-vault"]),
        "heroku" => a.extend(["--heroku-app", "my-app"]),
        "kubernetes" => a.extend(["--name", "mysecret"]),
        _ => {}
    }
    a
}

fn json_strings(v: &serde_json::Value) -> Result<BTreeMap<String, String>, String> {
    let obj = v.as_object().ok_or("not an object")?;
    obj.iter()
        .map(|(k, v)| {
            v.as_str()
                .map(|s| (k.clone(), lf(s)))
                .ok_or_else(|| format!("{k} is not a string: {v}"))
        })
        .collect()
}

fn yaml_strings(v: &serde_yaml::Value) -> Result<BTreeMap<String, String>, String> {
    let map = v.as_mapping().ok_or("not a mapping")?;
    map.iter()
        .map(|(k, v)| {
            let k = k.as_str().ok_or("non-string key")?.to_string();
            match v {
                serde_yaml::Value::String(s) => Ok((k, lf(s))),
                other => Err(format!("{k} reads back as {other:?}, not a string")),
            }
        })
        .collect()
}

/// A standard base64 decoder: the test depends on no crate for it.
fn unbase64(s: &str) -> Result<String, String> {
    let val = |c: u8| -> Result<u32, String> {
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as u32),
            b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("bad base64 byte {c}")),
        }
    };
    let bytes: Vec<u8> = s.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::new();
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    String::from_utf8(out).map_err(|e| e.to_string())
}

/// The `Name:` / `Value:` blocks `github-actions` prints for pasting.
fn github_blocks(text: &str) -> Result<BTreeMap<String, String>, String> {
    let body = text
        .split_once("\n\n")
        .ok_or("no heading")?
        .1
        .split("\n\n(")
        .next()
        .unwrap_or("");
    let mut map = BTreeMap::new();
    for block in body.split("\n---\n") {
        let rest = block
            .strip_prefix("Name: ")
            .ok_or_else(|| format!("block without a Name: {block:?}"))?;
        let (name, value) = rest
            .split_once("\nValue: ")
            .ok_or_else(|| format!("block without a Value: {block:?}"))?;
        map.insert(name.to_string(), value.to_string());
    }
    Ok(map)
}

/// Read `out` back as `format` says it should be read. `Err` is a defect.
fn read_back(format: &str, out: &str, p: &Project) -> Result<BTreeMap<String, String>, String> {
    let _ = p;
    match format {
        "json" | "aws-secrets" | "railway" => json_strings(&json_in(out)),
        "doppler" | "vercel" => {
            let doc = json_in(out);
            let obj = doc.as_object().ok_or("not an object")?;
            obj.iter()
                .map(|(k, v)| {
                    v["value"]
                        .as_str()
                        .map(|s| (k.clone(), lf(s)))
                        .ok_or_else(|| format!("{k} has no string value"))
                })
                .collect()
        }
        "yaml" => yaml_strings(&serde_yaml::from_str(out).map_err(|e| e.to_string())?),
        "kubernetes" => {
            let doc: serde_yaml::Value = serde_yaml::from_str(out).map_err(|e| e.to_string())?;
            yaml_strings(&doc["stringData"])
        }
        "docker-compose" => {
            // Compose reads `$$` as a literal `$`.
            let doc: serde_yaml::Value = serde_yaml::from_str(out).map_err(|e| e.to_string())?;
            let list = doc["environment"]
                .as_sequence()
                .ok_or("no environment list")?;
            let mut map = BTreeMap::new();
            for item in list {
                let s = item.as_str().ok_or("list item is not a string")?;
                let (k, v) = s
                    .split_once('=')
                    .ok_or_else(|| format!("no `=` in {s:?}"))?;
                map.insert(k.to_string(), lf(&v.replace("$$", "$")));
            }
            Ok(map)
        }
        "terraform" => {
            // Terraform reads `$${` as a literal `${`.
            let doc: toml::Table = out.parse().map_err(|e: toml::de::Error| e.to_string())?;
            doc.iter()
                .map(|(k, v)| {
                    v.as_str()
                        .map(|s| (k.clone(), lf(&s.replace("$${", "${"))))
                        .ok_or_else(|| format!("{k} is not a string"))
                })
                .collect()
        }
        "github-actions" => {
            github_blocks(out).map(|m| m.into_iter().map(|(k, v)| (k, lf(&v))).collect())
        }
        other => Err(format!(
            "no reader for format {other:?}: add one to this test"
        )),
    }
}

/// Formats whose output is a script: run it with stand-ins for the tool it
/// drives, and see what the tool would have been given. Unix only.
#[cfg(unix)]
mod scripts {
    use super::*;
    use std::path::Path;
    use std::process::{Command, Stdio};

    fn stubs(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        support::stub::write_stub(&bin.join(name), &format!("#!/bin/sh\n{body}"));
        bin
    }

    /// Run `script` under `sh` with `bin` first on PATH and `OUT` set; the stub
    /// writes what it receives into `OUT/<name>`.
    fn run_script(
        script: &Path,
        bin: &Path,
        out: &Path,
    ) -> Result<BTreeMap<String, String>, String> {
        std::fs::create_dir_all(out).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let st = Command::new("sh")
            .arg(script)
            .env("PATH", path)
            .env("OUT", out)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if !st.status.success() {
            return Err(format!(
                "the script failed: {}",
                String::from_utf8_lossy(&st.stderr)
            ));
        }
        let mut map = BTreeMap::new();
        for entry in std::fs::read_dir(out).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            map.insert(
                name,
                lf(&String::from_utf8_lossy(
                    &std::fs::read(entry.path()).unwrap(),
                )),
            );
        }
        Ok(map)
    }

    pub fn read(format: &str, text: &str) -> Option<Result<BTreeMap<String, String>, String>> {
        let work = tempfile::TempDir::new().unwrap();
        let script = work.path().join("out.sh");
        std::fs::write(&script, text).unwrap();
        let out = work.path().join("got");
        let r = match format {
            "shell" => {
                // Source it and print each variable, so `export -p` quoting
                // does not stand between the script and the value.
                let mut map = BTreeMap::new();
                for (k, _, _) in FIXTURE {
                    let o = Command::new("sh")
                        .arg("-c")
                        .arg(". \"$1\"; eval \"printf %s \\\"\\${$2-__UNSET__}\\\"\"")
                        .arg("sh")
                        .arg(&script)
                        .arg(k)
                        .output()
                        .unwrap();
                    map.insert(k.to_string(), lf(&String::from_utf8_lossy(&o.stdout)));
                }
                Ok(map)
            }
            "gcp-secrets" => {
                let bin = stubs(
                    work.path(),
                    "gcloud",
                    "case \"$1 $2\" in\n\
                     \"secrets describe\") exit 1;;\n\
                     \"secrets create\") cat > \"$OUT/$3\";;\n\
                     \"secrets versions\") cat > \"$OUT/$4\";;\n\
                     esac\n",
                );
                run_script(&script, &bin, &out)
            }
            "azure-keyvault" => {
                let bin = stubs(
                    work.path(),
                    "az",
                    "while [ $# -gt 0 ]; do\n\
                     case \"$1\" in --name) n=$2;; --file) f=$2;; esac; shift\n\
                     done\n\
                     cp \"$f\" \"$OUT/$n\"\n",
                );
                run_script(&script, &bin, &out)
            }
            "heroku" => {
                let bin = stubs(
                    work.path(),
                    "heroku",
                    "case \"$2\" in *=*) k=${2%%=*}; v=${2#*=}; printf %s \"$v\" > \"$OUT/$k\";; esac\n",
                );
                run_script(&script, &bin, &out)
            }
            _ => return None,
        };
        Some(r)
    }
}

#[test]
fn convert_output_reads_back_for_every_format() {
    let p = Project::new().env(&fixture_env());
    let mut f = Failures::default();
    let formats = evnx::formats::FORMATS;
    assert!(formats.len() >= 14, "format table read wrong");

    for format in formats {
        let name = format.name;
        let args = convert_args(name);
        let out = run(p.path(), &args);

        // Refusing a value a format cannot hold is allowed, if it says so and
        // prints nothing broken.
        if out.code != Some(0) {
            let said = !out.stderr.trim().is_empty() || !out.stdout.trim().is_empty();
            f.check(
                said && !out.stdout.contains("PLAIN"),
                name,
                format!(
                    "failed ({:?}) without saying why, or printed output:\n{}",
                    out.code,
                    out.all()
                ),
            );
            f.add(
                name,
                format!(
                    "refused the fixture; if a value cannot be held, name it in the test:\n{}",
                    out.all()
                ),
            );
            continue;
        }
        let text = lf(&out.stdout);

        // The script formats are run; the rest are parsed.
        #[cfg(unix)]
        let got = scripts::read(name, &text).unwrap_or_else(|| read_back(name, &text, &p));
        #[cfg(not(unix))]
        let got = match name {
            "shell" | "gcp-secrets" | "azure-keyvault" | "heroku" => continue,
            _ => read_back(name, &text, &p),
        };

        match got {
            Err(e) => f.add(
                name,
                format!("output does not read back: {e}\n--- output ---\n{text}"),
            ),
            Ok(got) => {
                let want = expected_for(name);
                for (k, v) in &want {
                    match got.get(k) {
                        None => f.add(name, format!("{k} is missing from the output")),
                        Some(g) if g != v => {
                            f.add(name, format!("{k}: wrote {v:?}, reads back {g:?}"))
                        }
                        Some(_) => {}
                    }
                }
                for k in got.keys().filter(|k| !want.contains_key(*k)) {
                    f.add(name, format!("{k} is in the output but not in .env"));
                }
            }
        }
    }
    f.finish();
}

#[test]
fn kubernetes_base64_reads_back() {
    let p = Project::new().env(&fixture_env());
    let out = run(
        p.path(),
        &["convert", "--to", "kubernetes", "--name", "s", "--base64"],
    );
    out.assert_code(0);
    let doc: serde_yaml::Value = serde_yaml::from_str(&lf(&out.stdout)).unwrap();
    let data = doc["data"].as_mapping().expect("a data mapping");
    let mut f = Failures::default();
    for (k, _, v) in FIXTURE {
        match data.get(*k).and_then(|x| x.as_str()) {
            None => f.add(k, "missing from data"),
            Some(b) => match unbase64(b) {
                Ok(s) => f.check(s == *v, k, format!("wrote {v:?}, decodes to {s:?}")),
                Err(e) => f.add(k, e),
            },
        }
    }
    f.finish();
}

#[test]
fn convert_to_a_file_reads_back() {
    // `-o` goes through a different writer than stdout.
    let p = Project::new().env(&fixture_env());
    run(p.path(), &["convert", "--to", "json", "-o", "out.json"]).assert_code(0);
    let text = std::fs::read_to_string(p.join("out.json")).unwrap();
    assert_eq!(json_strings(&json_in(&text)).unwrap(), fixture_values());
}

// ── 3. migrate --print-only ─────────────────────────────────────────────────

/// Every destination that prints commands is a script `sh` can parse.
#[cfg(all(unix, feature = "migrate"))]
#[test]
fn migrate_print_only_scripts_parse_under_sh() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let p = Project::new().env(&fixture_env());
    let destinations: &[(&str, &[&str])] = &[
        ("aws-secrets-manager", &["--secret-name", "my/secret"]),
        (
            "doppler",
            &["--project", "myapp", "--doppler-config", "dev"],
        ),
        (
            "infisical",
            &["--project", "myapp", "--infisical-env", "dev"],
        ),
        ("gcp-secret-manager", &["--project", "my-project"]),
        ("azure-keyvault", &["--vault-name", "my-vault"]),
        ("vercel", &["--vercel-project", "prj_1"]),
        ("heroku", &["--heroku-app", "my-app"]),
        (
            "railway",
            &[
                "--railway-project",
                "p-1",
                "--railway-environment",
                "production",
                "--railway-service",
                "web",
            ],
        ),
    ];
    let mut f = Failures::default();
    for (dest, extra) in destinations {
        let mut args = vec![
            "migrate",
            "--from",
            "env-file",
            "--to",
            dest,
            "--print-only",
        ];
        args.extend_from_slice(extra);
        let mut out = run(p.path(), &args);
        // A destination may refuse a value its tool cannot take, if it says
        // so and prints nothing (infisical: an empty value, one starting `@`).
        // The refusal is the invariant; then leave those keys out.
        if out.code == Some(2) && out.all().contains("Nothing was printed") {
            f.check(
                out.stdout.trim().is_empty() || !out.stdout.contains("PLAIN"),
                dest,
                "a refusal printed commands",
            );
            args.extend_from_slice(&["--exclude", "EMPTY,AT"]);
            out = run(p.path(), &args);
        }
        if out.code != Some(0) {
            f.add(dest, format!("exited {:?}:\n{}", out.code, out.all()));
            continue;
        }
        // The script is the lines that are not evnx's own commentary: stdout,
        // which is what `> run.sh` captures.
        let script = lf(&out.stdout);
        let mut sh = Command::new("sh")
            .arg("-n")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        sh.stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let res = sh.wait_with_output().unwrap();
        f.check(
            res.status.success(),
            dest,
            format!(
                "sh -n rejects the printed script: {}\n--- script ---\n{script}",
                String::from_utf8_lossy(&res.stderr)
            ),
        );
    }
    f.finish();
}

// ── 4. Round trips ──────────────────────────────────────────────────────────

#[test]
fn diff_of_a_file_with_itself_is_clean() {
    let p = Project::new().env(&fixture_env()).example(&fixture_env());
    for extra in [&[][..], &["--values"][..]] {
        let mut args = vec!["diff", "--env", ".env", "--example", ".env"];
        args.extend_from_slice(extra);
        let out = run(p.path(), &args);
        assert_eq!(out.code, Some(0), "{extra:?}\n{}", out.all());
    }
    // And against a template that is the same file.
    let out = run(p.path(), &["diff", "--values", "--fail-on", "different"]);
    assert_eq!(out.code, Some(0), "{}", out.all());
}

#[test]
fn sync_reverse_copies_the_template_values_exactly() {
    let p = Project::new()
        .env("OTHER=1\n")
        .example(&format!("OTHER=1\n{}", fixture_env()));
    run(p.path(), &["sync", "--direction", "reverse", "--yes"]).assert_code(0);
    let got = parsed(&p, ".env").unwrap();
    let mut f = Failures::default();
    for (k, v) in fixture_values() {
        // By design a template value that says "nothing here" (empty, `null`)
        // reaches .env as a placeholder to fill in, not as itself.
        if k == "EMPTY" || k == "NUL" {
            f.check(
                got.get(&k).is_some_and(|g| !g.is_empty() && *g != v),
                &k,
                format!("expected a placeholder, sync wrote {:?}", got.get(&k)),
            );
            continue;
        }
        f.check(
            got.get(&k) == Some(&v),
            &k,
            format!("template has {v:?}, sync wrote {:?}", got.get(&k)),
        );
    }
    f.finish();
}

#[test]
fn sync_forward_leaves_a_template_that_parses() {
    let p = Project::new()
        .env(&format!("OTHER=1\n{}", fixture_env()))
        .example("OTHER=1\n");
    // The template gets the names with placeholders, never the values.
    run(p.path(), &["sync"]).assert_code(0);
    let tpl = parsed(&p, ".env.example").unwrap();
    for (k, _, _) in FIXTURE {
        assert!(
            tpl.contains_key(*k),
            "{k} was not added to the template: {tpl:?}"
        );
    }
    let again = run(p.path(), &["sync", "--check"]);
    assert_eq!(
        again.code,
        Some(0),
        "a second sync still sees a difference:\n{}",
        again.all()
    );
}

#[test]
fn add_custom_values_read_back() {
    let values: &[(&str, &str)] = &[
        ("PLAIN_V", "plain"),
        ("SPACED", "two words"),
        ("HASHED", "a#b"),
        ("DOLLARED", "cost $5"),
        ("EQUALS", "a=b=c"),
        ("UNICODE", "héllo ✓"),
        ("QUOTED", "say \"hi\""),
        ("APOS", "it's"),
    ];
    let p = Project::new().env("EXISTING=1\n");
    let mut f = Failures::default();
    for (k, v) in values {
        let arg = format!("{k}={v}");
        let out = run(p.path(), &["add", "custom", "--yes", &arg]);
        if out.code != Some(0) {
            f.add(
                k,
                format!("add custom exited {:?}:\n{}", out.code, out.all()),
            );
        }
    }
    match parsed(&p, ".env") {
        Err(e) => f.add("add", e),
        Ok(got) => {
            f.check(
                got.get("EXISTING").map(String::as_str) == Some("1"),
                "EXISTING",
                "was lost",
            );
            for (k, v) in values {
                f.check(
                    got.get(*k).map(String::as_str) == Some(*v),
                    k,
                    format!("added {v:?}, reads back {:?}", got.get(*k)),
                );
            }
        }
    }
    f.finish();
}

#[test]
fn remove_leaves_the_rest_intact() {
    let p = Project::new().env(&fixture_env());
    let mut want = fixture_values();
    for key in ["DQUOTES", "MULTI", "EMPTY"] {
        run(p.path(), &["remove", key, "--yes", "--no-backup"]).assert_code(0);
        want.remove(key);
    }
    assert_eq!(parsed(&p, ".env").unwrap(), want);
}

#[test]
fn a_generated_project_survives_add_sync_and_remove() {
    let p = Project::new();
    run(p.path(), &["init", "--yes", "--with", "nextjs,postgresql"]).assert_code(0);
    run(
        p.path(),
        &[
            "add",
            "custom",
            "--yes",
            "FEATURE_FLAG=on",
            "BANNER=hello world",
        ],
    )
    .assert_code(0);
    run(p.path(), &["sync"]).assert_code(0);
    run(p.path(), &["sync", "--direction", "reverse", "--yes"]).assert_code(0);
    run(
        p.path(),
        &["remove", "FEATURE_FLAG", "--yes", "--no-backup"],
    )
    .assert_code(0);

    let mut f = Failures::default();
    for file in [".env", ".env.example"] {
        match parsed(&p, file) {
            Err(e) => f.add(file, e),
            Ok(m) => f.check(
                !m.contains_key("FEATURE_FLAG") || file == ".env.example",
                file,
                "FEATURE_FLAG survived remove",
            ),
        }
    }
    let d = run(p.path(), &["diff", "--env", ".env", "--example", ".env"]);
    f.check(
        d.code == Some(0),
        "diff",
        format!("a file differs from itself:\n{}", d.all()),
    );
    let v = run(p.path(), &["validate", "--format", "json"]);
    for issue in json_in(&v.stdout)["issues"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let ty = issue["type"].as_str().unwrap_or("?");
        f.check(
            issue["severity"] != "error" || ty == "placeholder_value",
            "validate",
            format!("{ty}: {}", issue["message"]),
        );
    }
    f.finish();
}
