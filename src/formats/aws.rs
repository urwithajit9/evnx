// ============================================================================
// formats/aws.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct AwsSecretsConverter;

impl Converter for AwsSecretsConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let filtered = options.filter_vars(vars);

        let transformed: IndexMap<String, String> = filtered
            .iter()
            .map(|(k, v)| {
                let key = options.transform_key(k);
                let value = options.transform_value(v);
                (key, value)
            })
            .collect();

        let json = serde_json::to_string_pretty(&transformed)?;
        Ok(json)
    }

    fn name(&self) -> &str {
        "aws-secrets"
    }

    fn description(&self) -> &str {
        "AWS Secrets Manager JSON format"
    }
}
