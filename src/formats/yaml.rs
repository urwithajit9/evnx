// ============================================================================
// formats/yaml.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use crate::core::secret::Vars;
use anyhow::Result;
use indexmap::IndexMap;
pub struct YamlConverter;

/// `KEY: "value"`, one line per variable, every value double-quoted.
///
/// ⚠️ CONV-1: this went through `serde_yaml`, which follows YAML 1.2 and
/// quotes only what 1.2 would type (`'true'`, `'123'`). YAML 1.1 readers —
/// PyYAML's `safe_load`, Ruby's Psych, most CI tools — also type `yes`, `on`,
/// `No` as booleans and `2026-01-01` as a date, so `A=yes` came back as
/// `True`. A value in an env file is a string, so every one is written as a
/// quoted string ([`crate::formats::quoting::yaml_scalar`], the rule the
/// Kubernetes and Docker formats use).
impl Converter for YamlConverter {
    fn convert(&self, vars: &Vars, options: &ConvertOptions) -> Result<String> {
        let transformed: IndexMap<String, String> =
            options.prepare(vars, self)?.into_iter().collect();
        if transformed.is_empty() {
            return Ok("{}\n".to_string());
        }
        let mut out = String::new();
        for (key, value) in &transformed {
            out.push_str(&yaml_key(key));
            out.push_str(": ");
            out.push_str(&crate::formats::quoting::yaml_scalar(value));
            out.push('\n');
        }
        Ok(out)
    }

    fn name(&self) -> &str {
        "yaml"
    }

    fn description(&self) -> &str {
        "Generic YAML key-value format"
    }
}

/// A key as written: bare when it is a plain name YAML reads as a string,
/// quoted otherwise (a prefix or transform can make `on`, `1`, `a: b`).
fn yaml_key(key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && !matches!(
            key.to_ascii_lowercase().as_str(),
            "y" | "n" | "yes" | "no" | "on" | "off" | "true" | "false" | "null" | "~"
        );
    if plain {
        key.to_string()
    } else {
        crate::formats::quoting::yaml_scalar(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::secret::Vars;

    fn convert(pairs: &[(&str, &str)]) -> String {
        let vars: Vars = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), (*v).into()))
            .collect();
        YamlConverter
            .convert(&vars, &ConvertOptions::default())
            .unwrap()
    }

    /// CONV-1's reproduction: every value reads back as the same string.
    /// serde_yaml reads YAML 1.2; the 1.1 words are checked by their quotes.
    #[test]
    fn conv1_every_value_is_a_quoted_string() {
        let pairs = [
            ("A", "yes"),
            ("E", "on"),
            ("I", "No"),
            ("J", "2026-01-01"),
            ("B", "true"),
            ("K", "123"),
            ("TILDE", "~"),
            ("C", "a: b # c"),
            ("Q", "say \"hi\"\nthere"),
            ("EMPTY", ""),
        ];
        let out = convert(&pairs);
        for (k, v) in pairs {
            let line = out
                .lines()
                .find(|l| l.starts_with(&format!("{k}: ")))
                .unwrap_or_else(|| panic!("{k} missing:\n{out}"));
            assert!(line[k.len() + 2..].starts_with('"'), "{line}");
            let _ = v;
        }
        let back: IndexMap<String, serde_yaml::Value> = serde_yaml::from_str(&out).unwrap();
        for (k, v) in pairs {
            assert_eq!(back[k], serde_yaml::Value::String(v.to_string()), "{k}");
        }
    }

    #[test]
    fn a_key_yaml_would_type_is_quoted() {
        assert_eq!(yaml_key("on"), "\"on\"");
        assert_eq!(yaml_key("N"), "\"N\"", "YAML 1.1 reads a bare N as false");
        assert_eq!(yaml_key("1PORT"), "\"1PORT\"");
        assert_eq!(yaml_key("APP_PORT"), "APP_PORT");
        assert_eq!(convert(&[]), "{}\n");
    }
}
