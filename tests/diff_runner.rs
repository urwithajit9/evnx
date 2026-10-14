//! The `evnx diff` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-diff-demo.sh` (copied from the review
//! package, `local/diff/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/diff/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`14-diff.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(target_os = "linux")]

mod support;

demo_scenes!(
    "diff",
    d0 => "D0",
    d1 => "D1",
    d2 => "D2",
    d3 => "D3",
    d4 => "D4",
    d5 => "D5",
    d6 => "D6",
    d7 => "D7",
    d8 => "D8",
    d9 => "D9",
    d10 => "D10",
    d11 => "D11",
);
