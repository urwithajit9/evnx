//! `evnx commands --json`, read from the binary's real stdout (T-SHARE-46).
//!
//! Three test files parsed it with their own copy of the same walk over
//! `commands[].path`. This is the one copy; each entry keeps its raw JSON for
//! the fields a test needs beyond the path.

use super::{run, Project};
use serde_json::Value;

/// One entry of `commands`: a command path and its raw JSON.
pub struct Entry {
    pub path: Vec<String>,
    pub json: Value,
}

impl Entry {
    /// The path as typed: `vault share`.
    pub fn name(&self) -> String {
        self.path.join(" ")
    }

    /// The command's arguments, as emitted.
    pub fn args(&self) -> &[Value] {
        self.json["args"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// The whole payload, from a run in an empty folder.
pub fn payload() -> Value {
    let p = Project::new();
    let out = run(p.path(), &["commands", "--json"]);
    out.assert_code(0);
    serde_json::from_str(&out.stdout).expect("commands --json is JSON")
}

/// Every entry of `commands`, in the order the binary emits them.
pub fn entries() -> Vec<Entry> {
    payload()["commands"]
        .as_array()
        .expect("`commands` is an array")
        .iter()
        .map(|c| Entry {
            path: c["path"]
                .as_array()
                .expect("`path` is an array")
                .iter()
                .map(|s| s.as_str().expect("path segments are strings").to_string())
                .collect(),
            json: c.clone(),
        })
        .collect()
}
