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
        let transformed: IndexMap<String, String> =
            options.prepare(vars, self)?.into_iter().collect();

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
