// ============================================================================
// formats/dotenv.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use crate::core::secret::Vars;
use anyhow::Result;

/// A `.env` file, written with the quoting the parser reads back exactly
/// (`core::envfile::quote`): `convert --to json` then `convert --from json
/// --to dotenv` returns the same values.
pub struct DotenvConverter;

impl Converter for DotenvConverter {
    fn convert(&self, vars: &Vars, options: &ConvertOptions) -> Result<String> {
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
    use crate::core::secret::Vars;

    #[test]
    fn writes_one_quoted_line_per_variable() {
        let mut vars = Vars::new();
        vars.insert("A".to_string(), "plain".into());
        vars.insert("B".to_string(), "two words # not a comment".into());
        vars.insert("C".to_string(), "".into());
        let out = DotenvConverter
            .convert(&vars, &ConvertOptions::default())
            .unwrap();
        assert_eq!(out, "A=plain\nB=\"two words # not a comment\"\nC=\n");
    }
}
