// ============================================================================
// formats/dotenv.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
use indexmap::IndexMap;

/// A `.env` file, written with the quoting the parser reads back exactly
/// (`core::envfile::quote`): `convert --to json` then `convert --from json
/// --to dotenv` returns the same values.
pub struct DotenvConverter;

impl Converter for DotenvConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let mut output = String::new();
        for (key, value) in options.prepare(vars, self)? {
            output.push_str(&crate::core::envfile::line(&key, &value));
            output.push('\n');
        }
        Ok(output)
    }

    fn name(&self) -> &str {
        "dotenv"
    }

    fn description(&self) -> &str {
        "Plain .env file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_one_quoted_line_per_variable() {
        let mut vars = IndexMap::new();
        vars.insert("A".to_string(), "plain".to_string());
        vars.insert("B".to_string(), "two words # not a comment".to_string());
        vars.insert("C".to_string(), String::new());
        let out = DotenvConverter
            .convert(&vars, &ConvertOptions::default())
            .unwrap();
        assert_eq!(out, "A=plain\nB=\"two words # not a comment\"\nC=\n");
    }
}
