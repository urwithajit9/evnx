// ============================================================================
// formats/github.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct GitHubActionsConverter {
    pub separator: String,
}

impl Default for GitHubActionsConverter {
    fn default() -> Self {
        Self {
            separator: "---".to_string(),
        }
    }
}

impl Converter for GitHubActionsConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let pairs = options.prepare(vars, self)?;

        let mut output = String::new();
        output.push_str("Paste these into Settings → Secrets and variables → Actions:\n\n");

        let count = pairs.len();
        for (i, (key, value)) in pairs.iter().enumerate() {
            output.push_str(&format!("Name: {}\n", key));
            output.push_str(&format!("Value: {}\n", value));

            if i < count - 1 {
                output.push_str(&format!("{}\n", self.separator));
            }
        }

        output.push_str(&format!(
            "\n({} secrets total — paste each one individually)\n",
            count
        ));

        Ok(output)
    }

    fn name(&self) -> &str {
        "github-actions"
    }

    fn description(&self) -> &str {
        "GitHub Actions secrets (text to paste into the web form)"
    }

    /// GitHub stores secret names in uppercase: `A_B` and `a_b` are one secret.
    fn names_ignore_case(&self) -> bool {
        true
    }
}
