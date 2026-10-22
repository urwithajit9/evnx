//! The `evnx init` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-init-demo.sh` (copied from the review
//! package, `local/init/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/init/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`01-init.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(target_os = "linux")]

mod support;

demo_scenes!(
    "init",
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
    s13 => "S13",
    s14 => "S14",
    s15 => "S15",
    s16 => "S16",
    s17 => "S17",
    s18 => "S18",
    s19 => "S19",
    s20 => "S20",
    s21 => "S21",
    s22 => "S22",
    s23 => "S23",
    s24 => "S24",
    s25 => "S25",
    s26 => "S26",
);
