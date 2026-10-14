//! The `evnx validate` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-validate-demo.sh` (copied from the review
//! package, `local/validate/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/validate/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`04-validate.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(target_os = "linux")]

mod support;

demo_scenes!(
    "validate",
    v0 => "V0",
    v1 => "V1",
    v2 => "V2",
    v3 => "V3",
    v4 => "V4",
    v5 => "V5",
    v6 => "V6",
    v7 => "V7",
    v8 => "V8",
    v9 => "V9",
    v10 => "V10",
);
