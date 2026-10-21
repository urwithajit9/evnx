//! The `evnx scan` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-scan-demo.sh` (copied from the review
//! package, `local/scan/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/scan/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`11-scan.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(target_os = "linux")]

mod support;

demo_scenes!(
    "scan",
    s0 => "S0",
    s1 => "S1",
    s2 => "S2",
    s3 => "S3",
    s4 => "S4",
    s5 => "S5",
    s6 => "S6",
    s7 => "S7",
    s8 => "S8",
    s9 => "S9",
    s10 => "S10",
    s11 => "S11",
    s12 => "S12",
);
