//! The `evnx convert` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-convert-demo.sh` (copied from the review
//! package, `local/convert/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/convert/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`16-convert.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(target_os = "linux")]

mod support;

demo_scenes!(
    "convert",
    c0 => "C0",
    c1 => "C1",
    c2 => "C2",
    c3 => "C3",
    c4 => "C4",
    c5 => "C5",
    c6 => "C6",
    c7 => "C7",
    c8 => "C8",
    c9 => "C9",
    c10 => "C10",
    c11 => "C11",
);
