//! Reading variables from JSON: the one reader `validate --input-format json`
//! and `convert --from json` share.
//!
//! The shape is a flat `{"KEY": "value"}` object, or `evnx parse --format json
//! --values`'s output. Names stay in the order the file has them. Every line
//! number is 0: JSON has no line to point at. An error names a key, never a
//! value.

use anyhow::{Context, Result};
use indexmap::IndexMap;

use super::parser::{is_env_key, Document, Entry, Span};

/// The variables in `text`, as a [`Document`] with one entry per name.
pub fn document_from_json(text: &str) -> Result<Document> {
    use serde_json::Value;
    // `IndexMap`, not `serde_json::Map`: that one sorts its keys, and a round
    // trip through JSON must not reorder a file.
    let object: IndexMap<String, Value> = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(_)) => serde_json::from_str(text).context("it is not valid JSON")?,
        Ok(_) => anyhow::bail!("expected an object of variables"),
        Err(e) => return Err(anyhow::Error::new(e).context("it is not valid JSON")),
    };
    let pairs: Vec<(String, String)> = if object.contains_key("schema_version") {
        let entries = object
            .get("entries")
            .and_then(Value::as_array)
            .context("`entries` is not a list")?;
        entries
            .iter()
            .map(|e| {
                let key = e
                    .get("key")
                    .and_then(Value::as_str)
                    .context("an entry has no `key`")?;
                let value = e.get("value").and_then(Value::as_str).with_context(|| {
                    format!("{key} has no value: write it with `evnx parse --format json --values`")
                })?;
                Ok((key.to_string(), value.to_string()))
            })
            .collect::<Result<_>>()?
    } else {
        object
            .iter()
            .map(|(k, v)| {
                let v = v
                    .as_str()
                    .with_context(|| format!("{k}'s value is not a string"))?;
                Ok((k.clone(), v.to_string()))
            })
            .collect::<Result<_>>()?
    };
    let mut doc = Document::default();
    for (key, value) in pairs {
        if !is_env_key(&key) {
            anyhow::bail!("'{key}' is not a variable name: use letters, digits, _, ., -, [ and ]");
        }
        doc.entries.push(Entry {
            key,
            value: value.into(),
            span: Span::line(0),
            raw: String::new().into(),
            exported: false,
            quote: None,
        });
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_order_of_the_file() {
        let doc = document_from_json(r#"{"Z": "1", "A": "2", "M": ""}"#).unwrap();
        let keys: Vec<&str> = doc.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["Z", "A", "M"]);
    }

    #[test]
    fn a_number_is_refused_by_key_and_never_by_value() {
        let err = document_from_json(r#"{"PORT": 8080}"#).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("PORT"), "{text}");
        assert!(!text.contains("8080"), "{text}");
    }

    #[test]
    fn a_nested_value_and_a_non_object_are_errors() {
        assert!(document_from_json(r#"{"A": {"B": "c"}}"#).is_err());
        assert!(document_from_json(r#"["A"]"#).is_err());
        assert!(document_from_json("not json").is_err());
    }
}
