// ============================================================================
// Doppler Format
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct DopplerConverter;

impl Converter for DopplerConverter {
    /// A flat JSON object of strings: the input `doppler secrets upload`
    /// takes (RM-65, I-097). It was `{"KEY": {"value", "computed"}}`, which is
    /// not an upload shape.
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let flat: IndexMap<String, String> = options.prepare(vars, self)?.into_iter().collect();
        Ok(serde_json::to_string_pretty(&flat)?)
    }

    fn name(&self) -> &str {
        "doppler"
    }

    fn description(&self) -> &str {
        "Doppler secrets JSON (for doppler secrets upload)"
    }
}
