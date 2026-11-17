//! The one place evnx writes a JSON document to stdout (WP-9b).
//!
//! Every `--format json` / `--json` output goes through [`emit`], which puts a
//! `run` object first in the top-level object:
//!
//! ```json
//! { "run": { "tool": "evnx", "version": "0.15.0", "schema_version": 1,
//!            "ruleset": null, "features": ["backup", "cloud"] }, ... }
//! ```
//!
//! * `run` is additive: the keys a command already printed (including its own
//!   `schema_version` / `schema`, deprecated, still present) stay where they
//!   were, so existing `jq` paths keep working.
//! * `run.schema_version` is the documented schema number of THIS output. A
//!   command that had none starts at 1.
//! * `ruleset` is the scan rule-set id on `scan` and `null` everywhere else, so
//!   the header has one shape.
//! * There is no timestamp in `run`: the same input prints the same bytes.
//!
//! The one exception is [`emit_unwrapped`].
//!
//! ⛔ `tests/architecture.rs` forbids `serde_json::to_string*` / `to_writer*`
//! feeding stdout anywhere else.

use anyhow::{Context, Result};
use serde::Serialize;

/// The `run` object.
#[derive(Serialize)]
struct Run<'a> {
    tool: &'static str,
    version: &'static str,
    schema_version: u32,
    ruleset: Option<&'a str>,
    features: Vec<&'static str>,
}

/// `value`'s own fields with `run` in front. `flatten` keeps the order of the
/// inner value's keys, whatever `serde_json`'s map feature set is.
#[derive(Serialize)]
struct Document<'a, T: Serialize> {
    run: Run<'a>,
    #[serde(flatten)]
    value: &'a T,
}

/// The document as text, pretty (`pretty`) or on one line, without a newline.
pub fn render<T: Serialize>(
    value: &T,
    schema_version: u32,
    ruleset: Option<&str>,
    pretty: bool,
) -> Result<String> {
    let features = crate::core::build::features();
    let doc = Document {
        run: Run {
            tool: "evnx",
            version: env!("CARGO_PKG_VERSION"),
            schema_version,
            ruleset,
            features,
        },
        value,
    };
    if pretty {
        serde_json::to_string_pretty(&doc)
    } else {
        serde_json::to_string(&doc)
    }
    .context("serialising the JSON output")
}

fn write(mut text: String) -> Result<()> {
    text.push('\n');
    crate::utils::term::write_stdout(text.as_bytes()).context("writing to stdout")
}

/// Print `value` as one pretty JSON document, `run` first.
pub fn emit<T: Serialize>(value: &T, schema_version: u32, ruleset: Option<&str>) -> Result<()> {
    write(render(value, schema_version, ruleset, true)?)
}

/// Print `value` as pretty JSON with NO `run` header. Two outputs use it:
/// `scan rules list --json`, whose top level is an array (an array has no key
/// to put `run` under, and wrapping it would break every existing `jq '.[]'`),
/// and `scan --format sarif`, whose schema is SARIF's, not ours.
pub fn emit_unwrapped<T: Serialize>(value: &T) -> Result<()> {
    write(serde_json::to_string_pretty(value).context("serialising the JSON output")?)
}

/// Print a document [`render`] already made (it is also written to a file by
/// `-o`): `auth key-bundle` and `auth token create`. Adds the newline if the
/// text has none.
pub fn emit_rendered(text: &str) -> Result<()> {
    write(text.trim_end_matches('\n').to_string())
}

/// [`emit`] on one line, for the output whose `--compact` flag asks for it.
pub fn emit_compact<T: Serialize>(
    value: &T,
    schema_version: u32,
    ruleset: Option<&str>,
) -> Result<()> {
    write(render(value, schema_version, ruleset, false)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_comes_first_and_the_rest_keeps_its_order() {
        let v = serde_json::json!({ "b": 1, "a": 2 });
        let s = render(&v, 3, Some("r"), false).unwrap();
        assert!(s.starts_with(r#"{"run":{"tool":"evnx","version":""#), "{s}");
        assert!(
            s.contains(r#""schema_version":3,"ruleset":"r","features":["#),
            "{s}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["a"], 2);
        assert!(parsed["run"].get("timestamp").is_none());
    }
}
