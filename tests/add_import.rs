//! `evnx add import <FILE|->`: many variables from one file.
//!
//! One fixture per format and per known shape; the rules (names, collisions,
//! arrays, existing names kept, placeholders in the template); and the value
//! hygiene: no value in a dry run, in `--json`, in a refusal or in a warning.
//! The last group is the round trip: `convert --to X`, then `add import`,
//! gives the same `.env`.

mod support;

use evnx::core::Parser;
use support::{Project, run, run_with};

fn parsed(p: &Project, file: &str) -> Vec<(String, String)> {
    Parser::default()
        .parse_content(&p.read(file))
        .unwrap_or_else(|e| panic!("{file} does not parse: {e}\n{}", p.read(file)))
        .into_iter()
        .map(|(k, v)| (k, v.expose().to_string()))
        .collect()
}

fn value(p: &Project, name: &str) -> Option<String> {
    parsed(p, ".env")
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
}

fn import(p: &Project, file: &str) -> support::Output {
    run(p.path(), &["add", "import", file, "--yes"])
}

fn base() -> Project {
    Project::new().env("KEEP=1\n").example("KEEP=\n")
}

// ── one fixture per format ───────────────────────────────────────────────

#[test]
fn env_like_file() {
    let p = base().file(
        "vars.env",
        "# c\nexport Db_Host=\"h 1\"\nPORT=8080\nMULTI=\"l1\\nl2\"\n",
    );
    import(&p, "vars.env").assert_code(0);
    assert_eq!(value(&p, "DB_HOST").as_deref(), Some("h 1"));
    assert_eq!(value(&p, "PORT").as_deref(), Some("8080"));
    assert_eq!(value(&p, "MULTI").as_deref(), Some("l1\nl2"));
    assert_eq!(value(&p, "KEEP").as_deref(), Some("1"));
}

#[test]
fn env_like_dollars_are_literal_and_said() {
    let p = base().file("vars.env", "REF='${OTHER}/x'\n");
    let out = import(&p, "vars.env");
    out.assert_code(0);
    out.assert_contains("written as literal text");
    assert_eq!(value(&p, "REF").as_deref(), Some("${OTHER}/x"));
}

#[test]
fn properties_file() {
    let p = base().file(
        "app.properties",
        "# c\ndb.host=localhost\ndb.port : 5432\nlong=a\\\n  b\nname=caf\\u00e9\n",
    );
    import(&p, "app.properties").assert_code(0);
    assert_eq!(value(&p, "DB_HOST").as_deref(), Some("localhost"));
    assert_eq!(value(&p, "DB_PORT").as_deref(), Some("5432"));
    assert_eq!(value(&p, "LONG").as_deref(), Some("ab"));
    assert_eq!(value(&p, "NAME").as_deref(), Some("café"));
}

#[test]
fn ini_file_prefixes_the_section_and_section_picks_one() {
    let text = "[db]\nhost = localhost\n[cache]\nurl = redis://x\n";
    let p = base().file("a.ini", text);
    import(&p, "a.ini").assert_code(0);
    assert_eq!(value(&p, "DB_HOST").as_deref(), Some("localhost"));
    assert_eq!(value(&p, "CACHE_URL").as_deref(), Some("redis://x"));

    let q = base().file("a.ini", text);
    run(
        q.path(),
        &["add", "import", "a.ini", "--section", "db", "--yes"],
    )
    .assert_code(0);
    assert_eq!(value(&q, "HOST").as_deref(), Some("localhost"));
    assert_eq!(value(&q, "CACHE_URL"), None);
    run(
        q.path(),
        &["add", "import", "a.ini", "--section", "nope", "--yes"],
    )
    .assert_code(2)
    .assert_contains("sections: db, cache");
}

#[test]
fn json_flat_and_nested() {
    let p = base().file(
        "s.json",
        r#"{"api-key":"k1","db":{"host":"h","port":5432,"tls":true},"cert":"a\nb"}"#,
    );
    import(&p, "s.json").assert_code(0);
    assert_eq!(value(&p, "API_KEY").as_deref(), Some("k1"));
    assert_eq!(value(&p, "DB__HOST").as_deref(), Some("h"));
    assert_eq!(value(&p, "DB__PORT").as_deref(), Some("5432"));
    assert_eq!(value(&p, "DB__TLS").as_deref(), Some("true"));
    assert_eq!(value(&p, "CERT").as_deref(), Some("a\nb"));
}

#[test]
fn yaml_flat_and_nested() {
    let p = base().file("s.yaml", "API_KEY: k1\ndb:\n  host: h\n  pass: \"p w\"\n");
    import(&p, "s.yaml").assert_code(0);
    assert_eq!(value(&p, "API_KEY").as_deref(), Some("k1"));
    assert_eq!(value(&p, "DB__HOST").as_deref(), Some("h"));
    assert_eq!(value(&p, "DB__PASS").as_deref(), Some("p w"));
}

#[test]
fn kubernetes_secret_decodes_data_and_reads_string_data() {
    let p = base().file(
        "secret.yaml",
        "apiVersion: v1\nkind: Secret\nmetadata:\n  name: x\ndata:\n  db-pass: aHVudGVyMg==\nstringData:\n  PLAIN: text\n",
    );
    import(&p, "secret.yaml").assert_code(0);
    assert_eq!(value(&p, "DB_PASS").as_deref(), Some("hunter2"));
    assert_eq!(value(&p, "PLAIN").as_deref(), Some("text"));
}

#[test]
fn kubernetes_binary_data_is_refused_and_nothing_is_written() {
    let p = base().file(
        "secret.yaml",
        "apiVersion: v1\nkind: Secret\ndata:\n  ok: dg==\n  blob: //4A\n",
    );
    let out = import(&p, "secret.yaml");
    out.assert_code(2);
    out.assert_contains("data.blob is binary");
    assert_eq!(p.read(".env"), "KEEP=1\n");
}

#[test]
fn compose_environment_map_list_and_section() {
    let text = "services:\n  web:\n    image: x\n    environment:\n      PORT: 8080\n      MODE: prod\n  db:\n    environment:\n      - POSTGRES_USER=app\n      - POSTGRES_DB=main\n";
    let p = base().file("compose.yaml", text);
    import(&p, "compose.yaml").assert_code(0);
    assert_eq!(value(&p, "PORT").as_deref(), Some("8080"));
    assert_eq!(value(&p, "POSTGRES_USER").as_deref(), Some("app"));

    let q = base().file("compose.yaml", text);
    run(
        q.path(),
        &["add", "import", "compose.yaml", "--section", "db", "--yes"],
    )
    .assert_code(0);
    assert_eq!(value(&q, "POSTGRES_DB").as_deref(), Some("main"));
    assert_eq!(value(&q, "PORT"), None);
}

#[test]
fn compose_variable_without_a_value_is_refused() {
    let p = base().file(
        "compose.yaml",
        "services:\n  web:\n    environment:\n      - FROM_SHELL\n      - A=1\n",
    );
    let out = import(&p, "compose.yaml");
    out.assert_code(2);
    out.assert_contains("has no value (compose takes it from the shell)");
}

#[test]
fn github_actions_env_at_every_level() {
    let text = "name: ci\non: push\nenv:\n  TOP: t\njobs:\n  build:\n    runs-on: x\n    env:\n      JOB: j\n    steps:\n      - run: echo\n        env:\n          STEP: s\n";
    let p = base().file("ci.yml", text);
    import(&p, "ci.yml").assert_code(0);
    for (n, v) in [("TOP", "t"), ("JOB", "j"), ("STEP", "s")] {
        assert_eq!(value(&p, n).as_deref(), Some(v), "{n}");
    }
}

#[test]
fn github_actions_expression_is_refused() {
    let p = base().file(
        "ci.yml",
        "jobs:\n  b:\n    runs-on: x\n    env:\n      TOKEN: ${{ secrets.TOKEN }}\n",
    );
    let out = import(&p, "ci.yml");
    out.assert_code(2);
    out.assert_contains("workflow expression");
}

#[test]
fn csv_with_and_without_a_header() {
    let p = base().file("v.csv", "name,value\nA,1\nB,\"x,\"\"y\"\"\"\n");
    import(&p, "v.csv").assert_code(0);
    assert_eq!(value(&p, "A").as_deref(), Some("1"));
    assert_eq!(value(&p, "B").as_deref(), Some("x,\"y\""));
    let q = base().file("v.csv", "C,3\nD,4\n");
    import(&q, "v.csv").assert_code(0);
    assert_eq!(value(&q, "D").as_deref(), Some("4"));
}

// ── web.config ───────────────────────────────────────────────────────────

fn web_config(inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<configuration>\n  \
         <connectionStrings>\n    <add name=\"Main\" connectionString=\"Server=x\"/>\n  \
         </connectionStrings>\n  <appSettings>\n{inner}\n  </appSettings>\n</configuration>\n"
    )
}

#[test]
fn web_config_reads_app_settings_with_entities_decoded() {
    let p = base().file(
        "web.config",
        &web_config(
            r#"    <add key="Db:Password" value="p&amp;ss &lt;w&gt; &quot;q&quot; &#x41;&#66;" />
    <add key="ApiUrl" value='https://x.test/?a=1&amp;b=2'/>"#,
        ),
    );
    import(&p, "web.config").assert_code(0);
    assert_eq!(
        value(&p, "DB_PASSWORD").as_deref(),
        Some("p&ss <w> \"q\" AB")
    );
    assert_eq!(
        value(&p, "APIURL").as_deref(),
        Some("https://x.test/?a=1&b=2")
    );
    // connectionStrings are not appSettings.
    assert_eq!(value(&p, "MAIN"), None);
    assert!(!p.read(".env").contains("Server=x"));
}

#[test]
fn web_config_skips_comments_and_cdata() {
    let p = base().file(
        "web.config",
        &web_config(
            r#"    <!-- <add key="Old" value="gone"/> -->
    <![CDATA[ <add key="Raw" value="no"/> ]]>
    <add key="Live" value="yes"/>"#,
        ),
    );
    import(&p, "web.config").assert_code(0);
    assert_eq!(value(&p, "LIVE").as_deref(), Some("yes"));
    assert_eq!(value(&p, "OLD"), None);
    assert_eq!(value(&p, "RAW"), None);
}

#[test]
fn web_config_applies_clear_and_remove_in_order_and_the_last_add_counts() {
    let p = base().file(
        "web.config",
        &web_config(
            r#"    <add key="Inherited" value="1"/>
    <clear/>
    <add key="A" value="1"/>
    <add key="B" value="2"/>
    <remove key="b"/>
    <add key="C" value="first"/>
    <add key="C" value="second"/>"#,
        ),
    );
    let out = run(p.path(), &["add", "import", "web.config", "--dry-run"]);
    out.assert_code(0);
    out.assert_contains("appSettings.A");
    out.assert_contains("appSettings.C");
    assert!(!out.all().contains("INHERITED"), "{}", out.all());
    assert!(!out.all().contains("appSettings.B"), "{}", out.all());
    import(&p, "web.config").assert_code(0);
    assert_eq!(value(&p, "A").as_deref(), Some("1"));
    assert_eq!(value(&p, "B"), None);
    assert_eq!(value(&p, "C").as_deref(), Some("second"));
    assert_eq!(value(&p, "INHERITED"), None);
}

#[test]
fn web_config_with_a_bom_or_in_utf16_is_read() {
    let text = web_config(r#"    <add key="K" value="v&#233;"/>"#);
    let p = base().file("web.config", &format!("\u{feff}{text}"));
    import(&p, "web.config").assert_code(0);
    assert_eq!(value(&p, "K").as_deref(), Some("vé"));

    let q = base();
    let utf16 = text.replace("utf-8", "utf-16");
    let mut bytes = vec![0xFF, 0xFE];
    for unit in utf16.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(q.path().join("Web.config"), bytes).unwrap();
    import(&q, "Web.config").assert_code(0);
    assert_eq!(value(&q, "K").as_deref(), Some("vé"));
}

#[test]
fn web_config_without_app_settings_or_with_config_source_says_why() {
    let p = base().file("web.config", "<configuration><system.web/></configuration>");
    let out = import(&p, "web.config");
    out.assert_code(2);
    out.assert_contains("no <appSettings>");
    let q = base().file(
        "web.config",
        "<configuration><appSettings configSource=\"secrets.config\"/></configuration>",
    );
    import(&q, "web.config")
        .assert_code(2)
        .assert_contains("configSource");
    assert_eq!(q.read(".env"), "KEEP=1\n");
}

#[test]
fn web_config_that_is_malformed_names_a_line_and_no_value() {
    for (i, broken) in [
        // An unclosed tag.
        format!("<configuration><appSettings><add key=\"K\" value=\"{CANARY}\"></appSettings></configuration>"),
        // An unknown entity next to the value.
        format!("<configuration>\n<appSettings>\n<add key=\"K\" value=\"{CANARY}&{CANARY};\"/>\n</appSettings></configuration>"),
        // An attribute without quotes.
        format!("<configuration><appSettings><add key=K value={CANARY}/></appSettings></configuration>"),
        // Not XML at all.
        format!("{CANARY}=1\n"),
    ]
    .iter()
    .enumerate()
    {
        let p = base().file("web.config", broken);
        let out = import(&p, "web.config");
        out.assert_code(2);
        assert!(!out.all().contains(CANARY), "case {i}: {}", out.all());
        assert!(!out.all().contains("canary"), "case {i}: {}", out.all());
        assert_eq!(p.read(".env"), "KEEP=1\n", "case {i}");
    }
    let p = base().file(
        "web.config",
        &format!("<configuration>\n<appSettings>\n<add key=\"K\" value=\"{CANARY}&{CANARY};\"/>\n</appSettings></configuration>"),
    );
    import(&p, "web.config").assert_contains("not valid XML (line 3,");
}

#[test]
fn web_config_refusals_and_dry_runs_print_no_value() {
    let p = base().file(
        "web.config",
        &web_config(&format!(
            "    <add key=\"Api.Key\" value=\"{CANARY}\"/>\n    <add key=\"api-key\" value=\"{CANARY}-2\"/>\n    <add value=\"{CANARY}-3\"/>\n    <add key=\"NoValue\"/>"
        )),
    );
    for args in [
        &["add", "import", "web.config", "--dry-run"][..],
        &["add", "import", "web.config", "--yes"],
        &["add", "import", "web.config", "--dry-run", "--json"],
    ] {
        let out = run(p.path(), args);
        out.assert_code(2);
        assert!(!out.all().contains(CANARY), "{args:?}\n{}", out.all());
    }
    let out = import(&p, "web.config");
    // Api.Key and api-key make one name; the nameless add and the add with no
    // value are refused, each named.
    out.assert_contains("API_KEY is made by appSettings.Api.Key and appSettings.api-key");
    out.assert_contains("has no key attribute");
    out.assert_contains("has no value attribute");
    assert_eq!(p.read(".env"), "KEEP=1\n");
}

// ── the rules ────────────────────────────────────────────────────────────

#[test]
fn two_sources_one_name_is_refused_listing_both() {
    let p = base().file("s.json", r#"{"db.host":"a","db-host":"b","ok":"1"}"#);
    let out = import(&p, "s.json");
    out.assert_code(2);
    out.assert_contains("DB_HOST is made by db.host and db-host");
    assert_eq!(p.read(".env"), "KEEP=1\n");
}

#[test]
fn arrays_and_null_are_refused_naming_the_key() {
    let p = base().file("s.json", r#"{"hosts":["a","b"],"gone":null,"ok":"1"}"#);
    let out = import(&p, "s.json");
    out.assert_code(2);
    out.assert_contains("hosts is a list");
    out.assert_contains("gone has no value");
    assert_eq!(p.read(".env"), "KEEP=1\n");
    // --exclude leaves them out and the rest goes in.
    run(
        p.path(),
        &[
            "add",
            "import",
            "s.json",
            "--exclude",
            "HOSTS,GONE",
            "--yes",
        ],
    )
    .assert_code(0);
    assert_eq!(value(&p, "OK").as_deref(), Some("1"));
}

#[test]
fn a_name_that_cannot_be_valid_is_refused() {
    let p = base().file("s.json", r#"{"1st":"a"}"#);
    run(p.path(), &["add", "import", "s.json", "--yes"])
        .assert_code(2)
        .assert_contains("not a valid variable name");
}

#[test]
fn an_existing_name_is_kept_and_reported_without_either_value() {
    let p = Project::new()
        .env("API_URL=old-canary-value\n")
        .example("API_URL=\n")
        .file("s.json", r#"{"api-url":"new-canary-value","port":"1"}"#);
    let out = import(&p, "s.json");
    out.assert_code(0);
    assert_eq!(value(&p, "API_URL").as_deref(), Some("old-canary-value"));
    assert_eq!(value(&p, "PORT").as_deref(), Some("1"));
    out.assert_contains("API_URL");
    out.assert_contains("existing value was kept");
    assert!(!out.all().contains("canary"), "{}", out.all());
}

#[test]
fn a_template_that_lists_the_name_does_not_stop_the_value_reaching_env() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\nAPI_URL=\n")
        .file("s.json", r#"{"API_URL":"https://x"}"#);
    import(&p, "s.json").assert_code(0);
    assert_eq!(value(&p, "API_URL").as_deref(), Some("https://x"));
}

#[test]
fn the_template_gets_placeholders_only() {
    const SECRET: &str = "sk_live_canary_0123456789abcdef";
    let p = base().file(
        "s.json",
        &format!(r#"{{"stripe-key":"{SECRET}","port":"9999"}}"#),
    );
    import(&p, "s.json").assert_code(0);
    let example = p.read(".env.example");
    assert!(example.contains("STRIPE_KEY="), "{example}");
    assert!(
        !example.contains(SECRET) && !example.contains("9999"),
        "{example}"
    );
    assert_eq!(value(&p, "STRIPE_KEY").as_deref(), Some(SECRET));
}

#[test]
fn only_and_exclude_filter_by_name() {
    let text = r#"{"db_host":"h","db_pass":"p","api_key":"k"}"#;
    let p = base().file("s.json", text);
    run(
        p.path(),
        &["add", "import", "s.json", "--only", "DB_*", "--yes"],
    )
    .assert_code(0);
    assert_eq!(value(&p, "DB_HOST").as_deref(), Some("h"));
    assert_eq!(value(&p, "API_KEY"), None);
    let q = base().file("s.json", text);
    run(
        q.path(),
        &["add", "import", "s.json", "--exclude", "DB_*", "--yes"],
    )
    .assert_code(0);
    assert_eq!(value(&q, "API_KEY").as_deref(), Some("k"));
    assert_eq!(value(&q, "DB_HOST"), None);
}

#[test]
fn env_name_writes_that_file() {
    let p = Project::new()
        .env("A=1\n")
        .file(".env.staging", "")
        .file("s.json", r#"{"port":"1"}"#);
    run(
        p.path(),
        &["add", "import", "s.json", "--env-name", "staging", "--yes"],
    )
    .assert_code(0);
    assert!(p.read(".env.staging").contains("PORT=1"));
    assert_eq!(p.read(".env"), "A=1\n");
}

#[test]
fn a_missing_env_file_is_created() {
    let p = Project::new().file("s.json", r#"{"port":"1"}"#);
    import(&p, "s.json").assert_code(0);
    assert!(p.read(".env").contains("PORT=1"));
}

// ── stdin, format, terminal ──────────────────────────────────────────────

#[test]
fn standard_input_needs_from() {
    let p = base();
    run_with(
        p.path(),
        &["add", "import", "-", "--yes"],
        &[],
        Some(b"{\"a\":\"1\"}"),
    )
    .assert_code(2)
    .assert_contains("--from");
    run_with(
        p.path(),
        &["add", "import", "-", "--from", "json", "--yes"],
        &[],
        Some(b"{\"a\":\"1\"}"),
    )
    .assert_code(0);
    assert_eq!(value(&p, "A").as_deref(), Some("1"));
}

#[test]
fn an_unknown_extension_needs_from() {
    let p = base().file("vars.txt", "A=1\n");
    run(p.path(), &["add", "import", "vars.txt", "--yes"])
        .assert_code(2)
        .assert_contains("--from");
    run(
        p.path(),
        &["add", "import", "vars.txt", "--from", "env", "--yes"],
    )
    .assert_code(0);
}

#[test]
fn a_missing_file_and_a_bad_file_are_exit_2() {
    let p = base().file("bad.json", "{ nope");
    run(p.path(), &["add", "import", "nope.json", "--yes"]).assert_code(2);
    let out = run(p.path(), &["add", "import", "bad.json", "--yes"]);
    out.assert_code(2);
    out.assert_contains("not valid JSON (line 1");
}

#[test]
fn without_a_terminal_and_without_yes_it_exits_4_and_writes_nothing() {
    let p = base().file("s.json", r#"{"port":"1"}"#);
    run(p.path(), &["add", "import", "s.json"]).assert_code(4);
    assert_eq!(p.read(".env"), "KEEP=1\n");
}

// ── dry run, JSON, and no value anywhere ─────────────────────────────────

const CANARY: &str = "import-canary-4d7a91c3";

fn canary_project() -> Project {
    base().file(
        "s.json",
        &format!(r#"{{"db":{{"pass":"{CANARY}"}},"api-key":"{CANARY}-2","list":[1]}}"#),
    )
}

#[test]
fn a_dry_run_prints_the_mapping_and_no_value() {
    let p = canary_project();
    let out = run(p.path(), &["add", "import", "s.json", "--dry-run"]);
    // `list` is an array: refused, and the table says so.
    out.assert_code(2);
    out.assert_contains("db.pass");
    out.assert_contains("DB__PASS");
    out.assert_contains("refused");
    assert!(!out.all().contains(CANARY), "{}", out.all());

    let ok = run(
        p.path(),
        &["add", "import", "s.json", "--exclude", "LIST", "--dry-run"],
    );
    ok.assert_code(0);
    ok.assert_contains("source key");
    ok.assert_contains("API_KEY");
    ok.assert_contains("new");
    assert!(!ok.all().contains(CANARY), "{}", ok.all());
    assert_eq!(p.read(".env"), "KEEP=1\n");
}

#[test]
fn json_has_the_mapping_and_no_value() {
    let p = canary_project();
    for args in [
        vec![
            "add",
            "import",
            "s.json",
            "--exclude",
            "LIST",
            "--dry-run",
            "--json",
        ],
        vec![
            "add",
            "import",
            "s.json",
            "--exclude",
            "LIST",
            "--yes",
            "--json",
        ],
    ] {
        let out = run(p.path(), &args);
        out.assert_code(0);
        assert!(!out.all().contains(CANARY), "{args:?}\n{}", out.all());
        let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["run"]["tool"], "evnx");
        assert_eq!(v["format"], "json");
        let names: Vec<&str> = v["mapping"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["DB__PASS", "API_KEY"]);
        assert_eq!(v["mapping"][0]["source"], "db.pass");
        assert!(!v["env_added"].as_array().unwrap().is_empty());
    }
}

#[test]
fn no_run_prints_a_value_not_even_a_refusal_or_a_conflict() {
    let p = Project::new()
        .env("API_KEY=old\n")
        .file(
            "s.json",
            &format!(r#"{{"api-key":"{CANARY}","db":{{"pass":"{CANARY}"}},"x.y":"{CANARY}","x-y":"{CANARY}"}}"#),
        );
    for args in [
        vec!["add", "import", "s.json", "--yes"],
        vec!["add", "import", "s.json", "--dry-run"],
        vec!["add", "import", "s.json", "--exclude", "X_Y", "--yes"],
        vec![
            "add",
            "import",
            "s.json",
            "--exclude",
            "X_Y",
            "--json",
            "--yes",
        ],
    ] {
        let out = run(p.path(), &args);
        assert!(!out.all().contains(CANARY), "{args:?}\n{}", out.all());
    }
}

#[test]
fn a_tracked_source_file_with_secrets_gets_the_untrack_warning() {
    let p = Project::new()
        .git()
        .env("A=1\n")
        .file(".gitignore", ".env\n")
        .file(
            "s.json",
            r#"{"stripe-secret-key":"sk_live_51H1234567890abcdefghijk","port":"1"}"#,
        );
    p.track("s.json");
    let out = import(&p, "s.json");
    out.assert_code(0);
    out.assert_contains("git rm --cached");
    out.assert_contains("rotate");
    assert!(!out.all().contains("sk_live_51H"), "{}", out.all());

    // Untracked: no warning.
    let q = Project::new()
        .git()
        .env("A=1\n")
        .file(".gitignore", ".env\ns.json\n")
        .file(
            "s.json",
            r#"{"stripe-secret-key":"sk_live_51H1234567890abcdefghijk"}"#,
        );
    let out = import(&q, "s.json");
    out.assert_code(0);
    assert!(!out.all().contains("git rm --cached"), "{}", out.all());
}

// ── round trip: convert --to X, then add import ──────────────────────────

const TRICKY: &str = concat!(
    "PLAIN=value\n",
    "EMPTY=\n",
    "SPACED=\"two words here\"\n",
    "HASH=\"a # not a comment\"\n",
    "EQUALS=https://x.test/?a=b&c=d\n",
    "DOLLAR='$HOME and ${NOT_EXPANDED}'\n",
    "DOUBLE_QUOTE=\"say \\\"hi\\\"\"\n",
    "SINGLE_QUOTE=\"it's\"\n",
    "BACKSLASH='C:\\dir\\file'\n",
    "NEWLINE=\"line one\\nline two\"\n",
    "UNICODE=caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \u{1f600}\n",
    "PADDED=\"  leading and trailing  \"\n",
    "TAB=\"a\\tb\"\n",
);

fn round_trip(to: &[&str], file: &str) {
    let source = Project::new().env(TRICKY);
    let mut args = vec!["convert", "--to"];
    args.extend_from_slice(to);
    let converted = run(source.path(), &args);
    converted.assert_code(0);
    let target = Project::new().env("").file(file, &converted.stdout);
    let out = run(target.path(), &["add", "import", file, "--yes"]);
    out.assert_code(0);
    let want = Parser::default()
        .parse_content(TRICKY)
        .unwrap()
        .into_iter()
        .map(|(k, v)| (k, v.expose().to_string()))
        .collect::<Vec<_>>();
    assert_eq!(
        parsed(&target, ".env"),
        want,
        "round trip through {to:?} changed something:\n{}\n--- imported .env:\n{}",
        converted.stdout,
        target.read(".env")
    );
}

#[test]
fn round_trip_json() {
    round_trip(&["json"], "out.json");
}

#[test]
fn round_trip_yaml() {
    round_trip(&["yaml"], "out.yaml");
}

#[test]
fn round_trip_kubernetes_string_data() {
    round_trip(&["kubernetes", "--name", "x"], "secret.yaml");
}

#[test]
fn round_trip_kubernetes_base64_data() {
    round_trip(&["kubernetes", "--name", "x", "--base64"], "secret.yaml");
}

#[test]
fn round_trip_docker_compose() {
    round_trip(&["docker-compose"], "compose.yaml");
}
