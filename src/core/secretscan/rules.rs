//! The rules evnx ships, as data: the built-in provider patterns, and the
//! build-time prefixes that publish a variable (C1, RM-45). Moved from
//! `utils::patterns`, unchanged.

use crate::core::config::PatternRule;

/// The detectors evnx ships, as ordinary [`PatternRule`]s.
///
/// ⚠️ These were a hand-written `if` chain of `Regex::is_match` calls, and a
/// duplicate `SecretPattern` table that nothing ever read. Two consequences:
///
/// * **Built-ins were weaker than user rules.** `[[scan.patterns]]` goes through
///   `PatternSet` and is matched against whole lines; the chain only ever saw
///   tokens longer than 20 characters. `-----BEGIN RSA PRIVATE KEY-----` is a
///   phrase, never one token, so the private-key detector could not fire in any
///   file — including the `.pem` files private keys live in.
/// * **Collisions were resolved by source order.** Anthropic was checked after
///   OpenAI, so a permissive OpenAI pattern silently claimed `sk-ant-…` keys and
///   named the wrong provider to revoke at.
///
/// As data they share one engine, one precedence rule, and the config surface
/// that lets a project disable or re-rate any of them.
///
/// Two checks are deliberately **not** here, because neither is a pattern:
/// the AWS secret key (gated on the variable's *name* plus entropy, since its
/// shape is just 40 base64 characters) and the generic high-entropy fallback.
/// Both live in [`detect_heuristic`](super::heuristic::detect_heuristic) and stay token-scoped — running an entropy
/// threshold over whole lines would flag every minified bundle.
pub fn builtin_rules() -> Vec<PatternRule> {
    BUILTIN.rules.clone()
}

/// `src/assets/rules.toml` as it is compiled in (C2, RM-50).
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    /// The file's format version, shared with rule files and packs (C6).
    version: u32,
    rules: Vec<PatternRule>,
}

/// Parsed once per process. ⚠️ `expect`, not an error path: the file is part
/// of the binary, so a bad one is a build defect the loader test catches.
static BUILTIN: std::sync::LazyLock<RuleFile> = std::sync::LazyLock::new(|| {
    toml::from_str(include_str!("../../assets/rules.toml"))
        .expect("src/assets/rules.toml parses; rules::tests checks it")
});

// ── C6: rule files ────────────────────────────────────────────────────────

/// The largest rule file evnx reads, in bytes, and the most rules in one: a
/// rule file is regexes and data, never code, and is capped by name.
pub const MAX_RULE_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_RULES_PER_FILE: usize = 5000;

/// The project's rule folder, under its root (S3): committed, so the team and
/// CI share it with no download.
pub const PROJECT_RULES_DIR: &str = ".evnx/rules";

/// A rule and where it came from: `core`, a file's path, or `.evnx.toml`.
#[derive(Debug, Clone)]
pub struct Sourced {
    pub rule: PatternRule,
    pub source: String,
}

/// What [`load`] found beyond the core.
#[derive(Debug, Default)]
pub struct Loaded {
    /// In merge order: project, then personal, then each `--rules` file.
    pub rules: Vec<Sourced>,
    /// Rules per source, for the scan header.
    pub sources: Vec<(String, usize)>,
    /// Problems with personal rules, which warn rather than stop (S3).
    pub warnings: Vec<String>,
    /// How many of `rules` are personal: the scan header always names them.
    pub personal: usize,
}

/// The personal rule folder: `EVNX_CONFIG_DIR/rules`, else the platform's
/// config folder (`~/.config/evnx/rules` on Linux). Never committed.
pub fn personal_dir() -> Option<std::path::PathBuf> {
    match std::env::var_os("EVNX_CONFIG_DIR") {
        Some(d) if !d.is_empty() => Some(std::path::PathBuf::from(d).join("rules")),
        _ => dirs::config_dir().map(|d| d.join("evnx").join("rules")),
    }
}

/// Read the rule files that add to the core (C6, S3): `.evnx/rules/*.toml`
/// under `project`, then `personal`, then each of `extra` (`--rules`).
///
/// ⛔ Add-only (S5): a rule whose id is the core's, or another file's, is an
/// error naming both, never an override. Every rule needs an id. A project
/// or `--rules` file that cannot be read stops the scan; a personal one is a
/// warning, because it is one person's and the scan should still run.
pub fn load(
    project: &std::path::Path,
    personal: Option<&std::path::Path>,
    extra: &[std::path::PathBuf],
) -> anyhow::Result<Loaded> {
    use std::collections::HashMap;
    let mut seen: HashMap<String, String> = builtin_rules()
        .into_iter()
        .filter_map(|r| r.id)
        .map(|id| (id, "evnx's own rules".to_string()))
        .collect();
    let mut out = Loaded::default();

    let mut take = |path: &std::path::Path, label: String, personal: bool, out: &mut Loaded| {
        let rules = read_rule_file(path, &label)?;
        let n = rules.len();
        for rule in rules {
            let id = rule
                .id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("{label}: rule '{}' needs an id", rule.name))?;
            if let Some(other) = seen.get(&id) {
                anyhow::bail!(
                    "rule id '{id}' is in both {other} and {label}: ids must be unique, \
                     and a rule file adds to evnx's rules, it cannot replace one"
                );
            }
            seen.insert(id, label.clone());
            out.rules.push(Sourced {
                rule,
                source: label.clone(),
            });
        }
        if personal {
            out.personal += n;
        }
        out.sources.push((label, n));
        Ok::<(), anyhow::Error>(())
    };

    // ⛔ S4, before any file is read: a pack whose bytes no longer match
    // its lock is refused by name, in either folder. Tampering is never a
    // personal warning.
    super::packs::verify(&project.join(PROJECT_RULES_DIR), PROJECT_RULES_DIR)?;
    if let Some(dir) = personal {
        super::packs::verify(dir, &dir.display().to_string())?;
    }
    for f in toml_files(&project.join(PROJECT_RULES_DIR))? {
        // Named as it is in the project, wherever the scan was run from.
        let label = format!(
            "{PROJECT_RULES_DIR}/{}",
            f.file_name().unwrap_or_default().to_string_lossy()
        );
        take(&f, label, false, &mut out)?;
    }
    if let Some(dir) = personal {
        match toml_files(dir) {
            Ok(files) => {
                for f in files {
                    let label = f.display().to_string();
                    if let Err(e) = take(&f, label, true, &mut out) {
                        // A duplicate id is a conflict, not one person's
                        // broken file: still an error.
                        if format!("{e}").contains("ids must be unique") {
                            return Err(e);
                        }
                        out.warnings.push(format!("personal rules skipped: {e:#}"));
                    }
                }
            }
            Err(e) => out.warnings.push(format!("personal rules skipped: {e:#}")),
        }
    }
    for f in extra {
        let label = crate::utils::path::label(&f.to_string_lossy());
        take(f, label, false, &mut out)?;
    }
    Ok(out)
}

/// Every `*.toml` in `dir`, sorted. A missing folder has none; any other
/// failure to list it is an error.
pub(crate) fn toml_files(dir: &std::path::Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    use anyhow::Context;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.is_file())
        .collect();
    files.sort();
    Ok(files)
}

/// One rule file, in the same schema as `rules.toml`, within the caps.
pub fn read_rule_file(path: &std::path::Path, label: &str) -> anyhow::Result<Vec<PatternRule>> {
    use anyhow::Context;
    let size = std::fs::metadata(path)
        .with_context(|| format!("reading {label}"))?
        .len();
    if size > MAX_RULE_FILE_BYTES {
        anyhow::bail!(
            "{label} is {size} bytes; a rule file may be at most {MAX_RULE_FILE_BYTES} \
             (2 MiB), so it was not read"
        );
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {label}"))?;
    parse_rule_text(&text, label)
}

/// A rule file's text, checked as [`read_rule_file`] checks a file (C7: a
/// download is checked before it is written).
pub fn parse_rule_text(text: &str, label: &str) -> anyhow::Result<Vec<PatternRule>> {
    use anyhow::Context;
    if text.len() as u64 > MAX_RULE_FILE_BYTES {
        anyhow::bail!(
            "{label} is {} bytes; a rule file may be at most {MAX_RULE_FILE_BYTES} (2 MiB)",
            text.len()
        );
    }
    check_categories(text, label)?;
    let file: RuleFile =
        toml::from_str(text).with_context(|| format!("{label} is not a valid rule file"))?;
    if file.version != 1 {
        anyhow::bail!("{label} has version {}; this evnx reads 1", file.version);
    }
    if file.rules.len() > MAX_RULES_PER_FILE {
        anyhow::bail!(
            "{label} has {} rules; a rule file may hold at most {MAX_RULES_PER_FILE}",
            file.rules.len()
        );
    }
    // RM-92: a `names` expression that does not compile is a load error naming
    // the file and the rule.
    for r in &file.rules {
        for n in r.names.iter().flatten() {
            if let Err(e) = regex::Regex::new(n) {
                let id = r.id.as_deref().unwrap_or(&r.name);
                anyhow::bail!("{label}: rule '{id}' has a `names` expression '{n}' that does not compile\n\n{e}");
            }
        }
    }
    Ok(file.rules)
}

/// RM-90: an unknown `category` is a load error naming the file, the rule and
/// the allowed list, so a typo never silently lands in another group.
fn check_categories(text: &str, label: &str) -> anyhow::Result<()> {
    let Ok(doc) = text.parse::<toml::Table>() else {
        return Ok(()); // the typed parse reports the syntax error
    };
    let Some(rules) = doc.get("rules").and_then(|r| r.as_array()) else {
        return Ok(());
    };
    for rule in rules {
        let Some(value) = rule.get("category") else {
            continue;
        };
        let id = rule
            .get("id")
            .or_else(|| rule.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed)");
        let ok = value
            .as_str()
            .is_some_and(|s| s.parse::<crate::core::config::Category>().is_ok());
        if !ok {
            let shown = value.as_str().map_or(value.to_string(), str::to_string);
            anyhow::bail!(
                "{label}: rule '{id}' has unknown category '{shown}'; allowed: {}",
                crate::core::config::Category::allowed()
            );
        }
    }
    Ok(())
}

/// Build-time prefixes that inline a variable into the **client bundle**.
///
/// Each entry is `(prefix, what inlines it)`. The second field is there so a
/// finding can name the thing the reader has to go and change, rather than
/// describing evnx's own rule.
///
/// ⚠️ This is deliberately a short, conservative list of prefixes whose *whole
/// purpose* is public exposure. It is not a framework-detection system — that is
/// a larger piece of work (G1.2) and it is not needed to answer the question
/// this list exists for: "is this name one the bundler publishes?"
///
/// Longest first, so a future entry cannot be shadowed by a shorter one that
/// happens to be its suffix-prefix.
pub const PUBLIC_PREFIXES: &[(&str, &str)] = &[
    ("NEXT_PUBLIC_", "Next.js"),
    ("NUXT_PUBLIC_", "Nuxt"),
    ("EXPO_PUBLIC_", "Expo"),
    ("REACT_APP_", "Create React App"),
    ("STORYBOOK_", "Storybook"),
    ("GATSBY_", "Gatsby"),
    ("VUE_APP_", "Vue CLI"),
    ("VITE_", "Vite"),
    ("PUBLIC_", "SvelteKit and Astro"),
];

/// The build-time prefix on `name`, and what inlines it.
///
/// ⚠️ `PUBLIC_` is in the list even though `PUBLIC_KEY` is a perfectly ordinary
/// name for something meant to be public. That is safe because this only ever
/// *escalates a finding that already exists*: it never creates one. Verified
/// against the real detectors — `PUBLIC_KEY=<base64 public key>` and
/// `NEXT_PUBLIC_API_URL=https://…` both produce no finding at all, so there is
/// nothing for this to escalate.
#[must_use]
pub fn public_prefix(name: &str) -> Option<(&'static str, &'static str)> {
    let upper = name.to_uppercase();
    PUBLIC_PREFIXES
        .iter()
        .find(|(prefix, _)| upper.starts_with(prefix))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C2: `rules.toml` loads, every rule carries an id, a confidence and a
    /// severity that parse, no id repeats, and the whole set compiles.
    #[test]
    fn rules_toml_loads_with_unique_ids_and_compiles() {
        use crate::core::secretscan::{Confidence, PatternSet, Severity};
        let rules = builtin_rules();
        assert!(rules.len() >= 13);
        let mut ids = std::collections::HashSet::new();
        for r in &rules {
            let id =
                r.id.as_deref()
                    .unwrap_or_else(|| panic!("{} has no id", r.name));
            assert!(ids.insert(id), "duplicate id {id}");
            r.confidence
                .as_deref()
                .unwrap()
                .parse::<Confidence>()
                .unwrap();
            r.severity.as_deref().unwrap().parse::<Severity>().unwrap();
        }
        PatternSet::compile(&rules).expect("the built-in rules compile");
    }

    /// RM-90: every built-in names its provider and a category.
    #[test]
    fn every_builtin_rule_has_a_provider_and_category() {
        for r in builtin_rules() {
            let id = r.id.as_deref().unwrap_or(&r.name);
            let provider = r.provider.as_deref().unwrap_or("");
            assert!(!provider.trim().is_empty(), "{id} has no provider");
            assert!(r.category.is_some(), "{id} has no category");
        }
    }

    /// RM-90: an unknown category is a load error naming file, rule, list.
    #[test]
    fn an_unknown_category_is_rejected_with_file_rule_and_list() {
        let text = "version = 1\n[[rules]]\nid = \"x-key\"\nname = \"X\"\nregex = 'x'\ncategory = \"cloud-ish\"\n";
        let e = parse_rule_text(text, "my-rules.toml")
            .unwrap_err()
            .to_string();
        assert!(e.contains("my-rules.toml") && e.contains("x-key"), "{e}");
        assert!(
            e.contains("cloud-ish") && e.contains("keys-certificates"),
            "{e}"
        );
    }

    /// RM-92: an invalid `names` regex is a load error naming file and rule.
    #[test]
    fn an_invalid_names_regex_is_rejected_with_file_and_rule() {
        let text = "version = 1\n[[rules]]\nid = \"x-key\"\nname = \"X\"\nregex = 'x'\nnames = ['(unclosed']\n";
        let err = parse_rule_text(text, "my.toml").unwrap_err().to_string();
        assert!(err.contains("my.toml") && err.contains("x-key"), "{err}");
    }

    /// RM-90: provider and category are optional in user rule files.
    #[test]
    fn provider_and_category_are_optional_in_rule_files() {
        let text = "version = 1\n[[rules]]\nid = \"x-key\"\nname = \"X\"\nregex = 'x'\n";
        let rules = parse_rule_text(text, "f.toml").unwrap();
        assert!(rules[0].provider.is_none() && rules[0].category.is_none());
    }

    /// C2, S7: the case that split the two fields. A Google API key's shape
    /// is certain, and it is usually a browser key meant to be public.
    #[test]
    fn a_google_api_key_is_high_confidence_and_low_severity() {
        let g = builtin_rules()
            .into_iter()
            .find(|r| r.id.as_deref() == Some("google-api-key"))
            .unwrap();
        assert_eq!(
            (g.confidence.as_deref(), g.severity.as_deref()),
            (Some("high"), Some("low"))
        );
    }

    /// ⚠️ Rewritten when the provider patterns became [`builtin_rules`]. It used
    /// to call `detect_secret` directly, which no longer knows about them — and
    /// asserting against a function production does not use is how a suite keeps
    /// passing while the product breaks. This compiles the built-ins the same way
    /// `evnx scan` does.
    #[test]
    fn the_connection_string_rule_catches_a_password_and_only_a_password() {
        use crate::core::secretscan::PatternSet;
        let set = PatternSet::compile(&builtin_rules()).expect("built-ins must compile");
        let hit = |v: &str| {
            set.strongest(v)
                .map(|m| m.name == "Credentials in a connection string")
                .unwrap_or(false)
        };

        // ⚠️ The case that motivated the rule: a DATABASE_URL nothing else flags.
        assert!(hit("postgresql://admin:s3cr3t@db.example.com:5432/main"));
        assert!(hit("mongodb+srv://u:p@cluster0.mongodb.net/test"));
        assert!(hit("amqp://guest:guest@rabbit:5672/"));
        assert!(hit("mysql://root:toor@127.0.0.1:3306/app"));
        // A password may contain `@`. The rule delimits it differently from a
        // strict URL parser, which does not matter — it still matches.
        assert!(hit("postgres://u:p@ss@host/db"));

        // No credentials at all.
        assert!(!hit("redis://localhost:6379/0"));
        assert!(!hit("postgresql://localhost:5432/db"));
        assert!(!hit("https://api.example.com/v1"));
        assert!(!hit("not-a-url-at-all"));
        assert!(!hit("mailto:someone@example.com"));
        assert!(!hit("postgres://u:p"));

        // A username with no password is a name, not a secret.
        assert!(!hit("postgresql://reader@db.example.com/main"));

        // ⚠️ An `@` past the authority belongs to the path or query. Matching it
        // would report a credential in an ordinary URL — which is why the
        // character classes exclude `/`, `?` and `#`.
        assert!(!hit("https://api.example.com/users/me@example.com"));
        assert!(!hit("https://api.example.com/?to=a@b.com"));
    }

    #[test]
    fn builtin_rules_recognise_the_providers_they_name() {
        use crate::core::secretscan::PatternSet;
        let set = PatternSet::compile(&builtin_rules()).expect("built-ins must compile");

        for (value, expected) in [
            ("AKIA4OZRMFJ3VREALKEY", "AWS Access Key"),
            (
                "sk_live_51H1234567890abcdefghijk",
                "Stripe Secret Key (LIVE)",
            ),
            (
                "sk_test_51H1234567890abcdefghijk",
                "Stripe Secret Key (test)",
            ),
            ("ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8", "GitHub Token"),
            ("-----BEGIN PRIVATE KEY-----", "Private Key"),
            ("-----BEGIN RSA PRIVATE KEY-----", "Private Key"),
        ] {
            let m = set
                .strongest(value)
                .unwrap_or_else(|| panic!("{value} must match a built-in rule"));
            assert_eq!(m.name, expected, "for {value}");
        }

        // A placeholder and an ordinary value must match nothing.
        for value in ["localhost", "postgresql://localhost:5432/db"] {
            assert!(set.strongest(value).is_none(), "{value} is not a secret");
        }
    }
}
