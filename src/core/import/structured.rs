//! JSON and YAML: flat or nested files, and the shapes people already have
//! variables in.
//!
//! | Shape | Where the variables are |
//! |---|---|
//! | Kubernetes Secret | `data` (base64, decoded) and `stringData`; a ConfigMap's `data` |
//! | docker-compose | `services.<name>.environment`, a map or a list of `NAME=value` |
//! | GitHub Actions | `env:` at the top, in a job, or in a step |
//! | anything else | every scalar, nested keys joined with `__` |
//!
//! `--section` picks a top-level key (generic), a service (compose) or a job
//! (GitHub Actions); its name is not part of the variable names.
//!
//! A parse error names a line and column and nothing else: a parser message
//! can quote the text it choked on.
//!
//! Both formats are read into [`V`], a small value type that keeps the order
//! of a file's keys (`serde_json::Value` sorts them), so variables are listed
//! and written in the order the file has them. YAML comes from `core::yaml`
//! (its rules: `yes`/`on` are text, numbers keep the text they were written
//! as, the merge key `<<` is not applied).

use anyhow::{Result, bail};
use base64::Engine;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

use super::Found;
use super::text::normalise;
use crate::core::secret::Secret;

/// A parsed value, with objects as ordered lists.
#[derive(Debug, Clone)]
enum V {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<V>),
    Obj(Vec<(String, V)>),
}

impl V {
    fn obj(&self) -> Option<&[(String, V)]> {
        match self {
            V::Obj(m) => Some(m),
            _ => None,
        }
    }

    fn str(&self) -> Option<&str> {
        match self {
            V::Str(s) => Some(s),
            _ => None,
        }
    }

    fn arr(&self) -> Option<&[V]> {
        match self {
            V::Arr(a) => Some(a),
            _ => None,
        }
    }
}

/// The entry of an ordered object.
fn get<'a>(map: &'a [(String, V)], key: &str) -> Option<&'a V> {
    map.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn keys(map: &[(String, V)]) -> String {
    map.iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl V {
    /// A YAML value; a number keeps the text it was written as (`1.10`).
    fn from_yaml(y: crate::core::yaml::Value) -> V {
        use crate::core::yaml::Value as Y;
        match y {
            Y::Null => V::Null,
            Y::Bool(b) => V::Bool(b),
            Y::Number(n) => V::Num(n),
            Y::String(s) => V::Str(s),
            Y::Seq(items) => V::Arr(items.into_iter().map(V::from_yaml).collect()),
            Y::Map(m) => V::Obj(m.into_iter().map(|(k, v)| (k, V::from_yaml(v))).collect()),
        }
    }
}

impl<'de> Deserialize<'de> for V {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<V, D::Error> {
        struct Vis;
        impl<'de> Visitor<'de> for Vis {
            type Value = V;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON or YAML value")
            }
            fn visit_bool<E>(self, b: bool) -> std::result::Result<V, E> {
                Ok(V::Bool(b))
            }
            fn visit_i64<E>(self, n: i64) -> std::result::Result<V, E> {
                Ok(V::Num(n.to_string()))
            }
            fn visit_u64<E>(self, n: u64) -> std::result::Result<V, E> {
                Ok(V::Num(n.to_string()))
            }
            fn visit_f64<E>(self, n: f64) -> std::result::Result<V, E> {
                Ok(V::Num(n.to_string()))
            }
            fn visit_str<E>(self, s: &str) -> std::result::Result<V, E> {
                Ok(V::Str(s.to_string()))
            }
            fn visit_string<E>(self, s: String) -> std::result::Result<V, E> {
                Ok(V::Str(s))
            }
            fn visit_none<E>(self) -> std::result::Result<V, E> {
                Ok(V::Null)
            }
            fn visit_unit<E>(self) -> std::result::Result<V, E> {
                Ok(V::Null)
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> std::result::Result<V, D2::Error> {
                V::deserialize(d)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<V, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = a.next_element()? {
                    out.push(v);
                }
                Ok(V::Arr(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<V, A::Error> {
                let mut out: Vec<(String, V)> = Vec::new();
                while let Some((k, v)) = a.next_entry::<String, V>()? {
                    // A key given twice: the last one counts, in the first one's place.
                    match out.iter_mut().find(|(ek, _)| *ek == k) {
                        Some(slot) => slot.1 = v,
                        None => out.push((k, v)),
                    }
                }
                Ok(V::Obj(out))
            }
        }
        d.deserialize_any(Vis)
    }
}

pub fn json(text: &str, section: Option<&str>) -> Result<Vec<Found>> {
    let value: V = serde_json::from_str(text).map_err(|e| {
        anyhow::anyhow!("not valid JSON (line {}, column {})", e.line(), e.column())
    })?;
    document(&value, section)
}

/// Every document of a multi-document file, through `core::yaml`. A key
/// given twice counts last, as in JSON; a parse error is a position only.
fn yaml_documents(text: &str) -> Result<Vec<V>> {
    let docs = crate::core::yaml::documents(text, crate::core::yaml::Duplicates::LastWins)?;
    Ok(docs
        .into_iter()
        .filter(|d| !d.is_null())
        .map(V::from_yaml)
        .collect())
}

pub fn yaml(text: &str, section: Option<&str>) -> Result<Vec<Found>> {
    let docs = yaml_documents(text)?;
    if docs.is_empty() {
        bail!("the YAML file has no content");
    }
    let mut out = Vec::new();
    for d in &docs {
        out.extend(document(d, section)?);
    }
    Ok(out)
}

fn ok(source: String, name: String, value: String) -> Found {
    Found {
        source,
        name,
        value: Ok(Secret::new(value)),
    }
}

fn refuse(source: String, name: String, reason: &str) -> Found {
    Found {
        source,
        name,
        value: Err(reason.to_string()),
    }
}

/// A scalar as the text a variable holds, or why it cannot be.
fn scalar(v: &V) -> std::result::Result<String, &'static str> {
    match v {
        V::Str(s) => Ok(s.clone()),
        V::Num(n) => Ok(n.clone()),
        V::Bool(b) => Ok(b.to_string()),
        V::Null => Err("has no value (null)"),
        V::Arr(_) => Err("is a list: a variable holds one value"),
        V::Obj(_) => Err("is a nested object"),
    }
}

fn document(doc: &V, section: Option<&str>) -> Result<Vec<Found>> {
    let Some(root) = doc.obj() else {
        bail!("expected an object at the top of the file, with the variables in it");
    };
    let kind = get(root, "kind").and_then(V::str);
    if matches!(kind, Some("Secret" | "ConfigMap")) && get(root, "apiVersion").is_some() {
        if section.is_some() {
            bail!("--section does not apply to a Kubernetes manifest: it has no sections");
        }
        return kubernetes(root, kind == Some("Secret"));
    }
    // A compose file has a service with `environment`; a workflow has jobs
    // with `steps` or `runs-on`. Anything else with a `services` or `jobs`
    // key is plain configuration.
    let any = |key: &str, field: &[&str]| {
        get(root, key).and_then(V::obj).is_some_and(|m| {
            m.iter().any(|(_, def)| {
                def.obj()
                    .is_some_and(|d| field.iter().any(|f| get(d, f).is_some()))
            })
        })
    };
    if any("services", &["environment"]) {
        return compose(root, section);
    }
    if any("jobs", &["steps", "runs-on", "uses"]) {
        return actions(root, section);
    }
    // The `environment:` block `evnx convert --to docker-compose` writes, on
    // its own: nothing else in the file.
    if let [(key, env)] = root {
        if key == "environment" && matches!(env, V::Obj(_) | V::Arr(_)) {
            if section.is_some() {
                bail!("--section does not apply to an environment block: it has no sections");
            }
            let mut out = Vec::new();
            compose_environment("environment", env, &mut out);
            return Ok(out);
        }
    }
    generic(root, section)
}

// ── Kubernetes ──────────────────────────────────────────────────────────

fn kubernetes(root: &[(String, V)], secret: bool) -> Result<Vec<Found>> {
    let mut out = Vec::new();
    if let Some(data) = get(root, "data").and_then(V::obj) {
        for (key, v) in data {
            let source = format!("data.{key}");
            let name = normalise(key);
            let Some(s) = v.str() else {
                out.push(refuse(source, name, "is not a string"));
                continue;
            };
            if !secret {
                out.push(ok(source, name, s.to_string()));
                continue;
            }
            let packed: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            match base64::engine::general_purpose::STANDARD.decode(packed) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) if !text.contains('\0') => out.push(ok(source, name, text)),
                    _ => out.push(refuse(source, name, "is binary (not UTF-8 text)")),
                },
                Err(_) => out.push(refuse(source, name, "is not valid base64")),
            }
        }
    }
    if let Some(data) = get(root, "stringData").and_then(V::obj) {
        for (key, v) in data {
            let source = format!("stringData.{key}");
            let name = normalise(key);
            match scalar(v) {
                Ok(s) => out.push(ok(source, name, s)),
                Err(r) => out.push(refuse(source, name, r)),
            }
        }
    }
    if let Some(bin) = get(root, "binaryData").and_then(V::obj) {
        for (key, _) in bin {
            out.push(refuse(
                format!("binaryData.{key}"),
                normalise(key),
                "is binary",
            ));
        }
    }
    Ok(out)
}

// ── docker-compose ──────────────────────────────────────────────────────

const FROM_SHELL: &str = "has no value (compose takes it from the shell)";

fn compose(root: &[(String, V)], section: Option<&str>) -> Result<Vec<Found>> {
    let services = get(root, "services").and_then(V::obj).expect("checked");
    if let Some(s) = section {
        if get(services, s).is_none() {
            bail!("no service {s} in the file (services: {})", keys(services));
        }
    }
    let mut out = Vec::new();
    for (svc, def) in services {
        if section.is_some_and(|s| s != svc) {
            continue;
        }
        if let Some(env) = def.obj().and_then(|d| get(d, "environment")) {
            compose_environment(&format!("services.{svc}.environment"), env, &mut out);
        }
    }
    Ok(out)
}

/// One `environment:`, a map or a list of `NAME=value`. Compose reads `$$` as
/// a literal `$`, so that is what is imported.
fn compose_environment(base: &str, env: &V, out: &mut Vec<Found>) {
    let literal = |v: String| v.replace("$$", "$");
    match env {
        V::Obj(map) => {
            for (k, v) in map {
                let source = format!("{base}.{k}");
                match scalar(v) {
                    Ok(s) => out.push(ok(source, normalise(k), literal(s))),
                    Err(_) if matches!(v, V::Null) => {
                        out.push(refuse(source, normalise(k), FROM_SHELL))
                    }
                    Err(r) => out.push(refuse(source, normalise(k), r)),
                }
            }
        }
        V::Arr(list) => {
            for (i, item) in list.iter().enumerate() {
                let Some(text) = item.str() else {
                    out.push(refuse(
                        format!("{base}[{i}]"),
                        String::new(),
                        "is not a NAME=value string",
                    ));
                    continue;
                };
                match text.split_once('=') {
                    Some((k, v)) => out.push(ok(
                        format!("{base}.{k}"),
                        normalise(k),
                        literal(v.to_string()),
                    )),
                    None => out.push(refuse(
                        format!("{base}.{text}"),
                        normalise(text),
                        FROM_SHELL,
                    )),
                }
            }
        }
        _ => {}
    }
}

// ── GitHub Actions ──────────────────────────────────────────────────────

fn env_map(out: &mut Vec<Found>, at: &str, env: Option<&V>) {
    let Some(map) = env.and_then(V::obj) else {
        return;
    };
    for (k, v) in map {
        let source = format!("{at}.{k}");
        match scalar(v) {
            Ok(s) if s.contains("${{") => out.push(refuse(
                source,
                normalise(k),
                "is a workflow expression, not a value",
            )),
            Ok(s) => out.push(ok(source, normalise(k), s)),
            Err(r) => out.push(refuse(source, normalise(k), r)),
        }
    }
}

fn actions(root: &[(String, V)], section: Option<&str>) -> Result<Vec<Found>> {
    let jobs = get(root, "jobs").and_then(V::obj).expect("checked");
    if let Some(s) = section {
        if get(jobs, s).is_none() {
            bail!("no job {s} in the file (jobs: {})", keys(jobs));
        }
    }
    let mut out = Vec::new();
    if section.is_none() {
        env_map(&mut out, "env", get(root, "env"));
    }
    for (id, job) in jobs {
        if section.is_some_and(|s| s != id) {
            continue;
        }
        let Some(job) = job.obj() else { continue };
        env_map(&mut out, &format!("jobs.{id}.env"), get(job, "env"));
        if let Some(steps) = get(job, "steps").and_then(V::arr) {
            for (i, step) in steps.iter().enumerate() {
                let env = step.obj().and_then(|s| get(s, "env"));
                env_map(&mut out, &format!("jobs.{id}.steps[{i}].env"), env);
            }
        }
    }
    Ok(out)
}

// ── anything else ───────────────────────────────────────────────────────

fn generic(root: &[(String, V)], section: Option<&str>) -> Result<Vec<Found>> {
    let start: &[(String, V)] = match section {
        Some(s) => match get(root, s) {
            Some(V::Obj(m)) => m,
            Some(_) => bail!("{s} is not an object: --section takes a key that holds variables"),
            None => bail!("no key {s} at the top of the file (keys: {})", keys(root)),
        },
        None => root,
    };
    let mut out = Vec::new();
    flatten(start, &mut Vec::new(), &mut out);
    Ok(out)
}

fn flatten(map: &[(String, V)], path: &mut Vec<String>, out: &mut Vec<Found>) {
    for (k, v) in map {
        path.push(k.clone());
        match v {
            V::Obj(inner) => flatten(inner, path, out),
            other => {
                let source = path.join(".");
                let name = path
                    .iter()
                    .map(|p| normalise(p))
                    .collect::<Vec<_>>()
                    .join("__");
                match scalar(other) {
                    Ok(s) => out.push(ok(source, name, s)),
                    Err(r) => out.push(refuse(source, name, r)),
                }
            }
        }
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(f: Vec<Found>) -> Vec<(String, String, std::result::Result<String, String>)> {
        f.into_iter()
            .map(|f| (f.source, f.name, f.value.map(|v| v.expose().to_string())))
            .collect()
    }

    fn good(
        source: &str,
        name: &str,
        value: &str,
    ) -> (String, String, std::result::Result<String, String>) {
        (source.into(), name.into(), Ok(value.into()))
    }

    #[test]
    fn nested_keys_join_with_two_underscores_and_scalars_are_text() {
        let got = rows(
            json(
                r#"{"db":{"host":"h","port":5432,"tls":true},"api-key":"k"}"#,
                None,
            )
            .unwrap(),
        );
        assert_eq!(
            got,
            [
                good("db.host", "DB__HOST", "h"),
                good("db.port", "DB__PORT", "5432"),
                good("db.tls", "DB__TLS", "true"),
                good("api-key", "API_KEY", "k"),
            ]
        );
    }

    #[test]
    fn arrays_and_null_are_refused_naming_the_key() {
        let got = rows(json(r#"{"hosts":["a","b"],"gone":null,"ok":"1"}"#, None).unwrap());
        assert!(got[0].2.as_ref().unwrap_err().contains("list"));
        assert!(got[1].2.as_ref().unwrap_err().contains("null"));
        assert_eq!(got[2], good("ok", "OK", "1"));
    }

    #[test]
    fn a_section_picks_one_top_level_key_and_drops_its_name() {
        let text = r#"{"dev":{"db":{"host":"d"}},"prod":{"db":{"host":"p"}}}"#;
        assert_eq!(
            rows(json(text, Some("prod")).unwrap()),
            [good("db.host", "DB__HOST", "p")]
        );
        let e = json(text, Some("x")).unwrap_err().to_string();
        assert!(e.contains("keys: dev, prod"), "{e}");
    }

    #[test]
    fn a_json_error_is_a_position_never_the_text() {
        let e = json("{\"A\": \"hunter2-secret\" oops}", None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("line 1"), "{e}");
        assert!(!e.contains("hunter2"), "{e}");
        let e = yaml("a: 'hunter2-secret\n  b: [", None)
            .unwrap_err()
            .to_string();
        assert!(!e.contains("hunter2"), "{e}");
    }

    #[test]
    fn a_kubernetes_secret_decodes_data_and_reads_string_data() {
        let text = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: x\ntype: Opaque\n\
                    data:\n  db-pass: aHVudGVyMg==\n  blob: //4A\nstringData:\n  PLAIN: text\n  NUM: 7\n";
        let got = rows(yaml(text, None).unwrap());
        assert_eq!(got[0], good("data.db-pass", "DB_PASS", "hunter2"));
        assert!(got[1].2.as_ref().unwrap_err().contains("binary"));
        assert_eq!(got[2], good("stringData.PLAIN", "PLAIN", "text"));
        assert_eq!(got[3], good("stringData.NUM", "NUM", "7"));
        assert!(yaml(text, Some("x")).is_err());
    }

    #[test]
    fn a_config_map_is_read_as_plain_text_and_documents_are_all_read() {
        let text = "apiVersion: v1\nkind: ConfigMap\ndata:\n  LEVEL: info\n---\n\
                    apiVersion: v1\nkind: Secret\ndata:\n  K: dg==\n";
        let got = rows(yaml(text, None).unwrap());
        assert_eq!(
            got,
            [
                good("data.LEVEL", "LEVEL", "info"),
                good("data.K", "K", "v")
            ]
        );
    }

    #[test]
    fn compose_reads_a_map_and_a_list_and_a_service_can_be_picked() {
        let text = "services:\n  web:\n    image: x\n    environment:\n      PORT: 8080\n      DEBUG: true\n      TOKEN:\n  db:\n    environment:\n      - POSTGRES_USER=app\n      - PGDATA\n";
        let all = rows(yaml(text, None).unwrap());
        assert_eq!(
            all[0],
            good("services.web.environment.PORT", "PORT", "8080")
        );
        assert_eq!(
            all[1],
            good("services.web.environment.DEBUG", "DEBUG", "true")
        );
        assert!(all[2].2.is_err());
        assert_eq!(
            all[3],
            good(
                "services.db.environment.POSTGRES_USER",
                "POSTGRES_USER",
                "app"
            )
        );
        assert!(all[4].2.is_err());
        let web = rows(yaml(text, Some("web")).unwrap());
        assert_eq!(web.len(), 3);
        let e = yaml(text, Some("cache")).unwrap_err().to_string();
        assert!(e.contains("services: web, db"), "{e}");
    }

    #[test]
    fn compose_reads_a_bare_environment_block_and_unescapes_dollars() {
        let got = rows(yaml("environment:\n  - \"A=$$HOME x\"\n  - \"B=2\"\n", None).unwrap());
        assert_eq!(
            got,
            [
                good("environment.A", "A", "$HOME x"),
                good("environment.B", "B", "2")
            ]
        );
    }

    #[test]
    fn github_actions_env_at_every_level_and_expressions_are_refused() {
        let text = "name: ci\non: push\nenv:\n  TOP: t\njobs:\n  build:\n    env:\n      JOB: j\n      TOKEN: ${{ secrets.TOKEN }}\n    steps:\n      - run: x\n        env:\n          STEP: s\n";
        let all = rows(yaml(text, None).unwrap());
        assert_eq!(all[0], good("env.TOP", "TOP", "t"));
        assert_eq!(all[1], good("jobs.build.env.JOB", "JOB", "j"));
        assert!(all[2].2.as_ref().unwrap_err().contains("expression"));
        assert_eq!(all[3], good("jobs.build.steps[0].env.STEP", "STEP", "s"));
        let one = rows(yaml(text, Some("build")).unwrap());
        assert_eq!(one.len(), 3);
    }

    #[test]
    fn plain_configuration_with_a_services_key_is_not_taken_for_compose() {
        let got = rows(json(r#"{"services":{"stripe":{"key":"k"}}}"#, None).unwrap());
        assert_eq!(
            got,
            [good("services.stripe.key", "SERVICES__STRIPE__KEY", "k")]
        );
    }

    #[test]
    fn json_keys_keep_the_files_order() {
        let got = rows(json(r#"{"b":"1","a":"2","c":"3"}"#, None).unwrap());
        let names: Vec<&str> = got.iter().map(|r| r.1.as_str()).collect();
        assert_eq!(names, ["B", "A", "C"]);
    }

    #[test]
    fn keys_keep_the_files_order_and_a_repeated_key_counts_last() {
        let got = rows(yaml("z: 1\na: 2\nz: 3\n", None).unwrap());
        assert_eq!(got, [good("z", "Z", "3"), good("a", "A", "2")]);
    }

    #[test]
    fn yaml_flat_and_nested() {
        let got = rows(yaml("A: 1\nb:\n  c: \"x y\"\n", None).unwrap());
        assert_eq!(got, [good("A", "A", "1"), good("b.c", "B__C", "x y")]);
        assert!(yaml("", None).is_err());
        assert!(yaml("- a\n- b\n", None).is_err());
    }
}
