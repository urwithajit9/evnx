//! Removing `[vars.NAME]` from `.evnx.toml`, format-preserving (C-R1 … C-R4).
//!
//! `toml_edit`, never deserialize-and-rewrite: the file holds comments and
//! settings evnx does not model. Both shapes go the same way — the table
//! `[vars.NAME]` and the inline `NAME = { … }` under `[vars]` — because both
//! are the `vars` item's table-like view.

use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};
use toml_edit::DocumentMut;

/// The names `[vars]` declares, or none. Malformed TOML is an error (C-R4).
pub fn declared(text: &str, label: &str) -> Result<Vec<String>> {
    let doc = parse(text, label)?;
    Ok(doc
        .get("vars")
        .and_then(|v| v.as_table_like())
        .map(|t| t.iter().map(|(k, _)| k.to_string()).collect())
        .unwrap_or_default())
}

/// `text` without the `[vars]` entries in `names`. Everything else — other
/// entries, `[defaults]`, `[[scan.patterns]]`, comments, order — is kept. The
/// comment directly above a `[vars.NAME]` header goes with it (C-1).
pub fn remove(text: &str, names: &BTreeSet<String>, label: &str) -> Result<String> {
    let mut doc = parse(text, label)?;
    let before = declared(text, label)?;
    if let Some(vars) = doc.get_mut("vars").and_then(|v| v.as_table_like_mut()) {
        for n in names {
            vars.remove(n);
        }
    }
    let out = doc.to_string();
    // The same post-condition the env files have (S-3): it still parses, and
    // declares exactly the old names minus the removed ones.
    let expected: Vec<String> = before.into_iter().filter(|n| !names.contains(n)).collect();
    match declared(&out, label) {
        Ok(after) if after == expected && out.parse::<toml::Table>().is_ok() => Ok(out),
        _ => bail!(
            "removing from {label} would change more than the entries asked for, so nothing was written"
        ),
    }
}

fn parse(text: &str, label: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>()
        .with_context(|| format!("{label} is not valid TOML, so nothing was changed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn c1_c3_c8_table_form_and_everything_else_kept() {
        let text = "\
# project contract
[defaults]
env_name = \"staging\"

[vars.KEEP]
environments = [\"production\"]

# the weak one
[vars.X]
description = \"drop me\"
required = true

[vars.ALSO]
format = \"port\"

[[scan.patterns]]
name = \"mine\"
regex = \"abc\"
";
        let out = remove(text, &n(&["X"]), "t").unwrap();
        assert_eq!(
            out,
            "\
# project contract
[defaults]
env_name = \"staging\"

[vars.KEEP]
environments = [\"production\"]

[vars.ALSO]
format = \"port\"

[[scan.patterns]]
name = \"mine\"
regex = \"abc\"
"
        );
    }

    #[test]
    fn c2_inline_form() {
        let text = "[vars]\nX = { format = \"port\" }\nY = { required = false } # keep\n";
        let out = remove(text, &n(&["X"]), "t").unwrap();
        assert_eq!(out, "[vars]\nY = { required = false } # keep\n");
        let text = "vars = { X = { format = \"port\" }, Y = {} }\n";
        let out = remove(text, &n(&["X"]), "t").unwrap();
        assert_eq!(declared(&out, "t").unwrap(), vec!["Y"]);
    }

    #[test]
    fn c7_malformed_is_refused() {
        assert!(remove("[vars.X\nrequired = true\n", &n(&["X"]), "t").is_err());
    }

    #[test]
    fn a_name_not_declared_changes_nothing() {
        let text = "[vars.A]\n";
        assert_eq!(remove(text, &n(&["B"]), "t").unwrap(), text);
    }
}
