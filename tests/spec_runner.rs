//! The `evnx spec` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-spec-demo.sh` (copied from the review
//! package, `local/spec/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/spec/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`23-spec.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(target_os = "linux")]

mod support;

demo_scenes!(
    "spec",
    sp0 => "SP0",
    sp1 => "SP1",
    sp2 => "SP2",
    sp3 => "SP3",
    sp4 => "SP4",
    sp5 => "SP5",
    sp6 => "SP6",
    sp7 => "SP7",
    sp8 => "SP8",
    sp9 => "SP9",
    sp10 => "SP10",
);
