//! The `evnx template` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-template-demo.sh` (copied from the review
//! package, `local/template/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/template/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`20-template.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(target_os = "linux")]

mod support;

demo_scenes!(
    "template",
    tp0 => "TP0",
    tp1 => "TP1",
    tp2 => "TP2",
    tp3 => "TP3",
    tp4 => "TP4",
    tp5 => "TP5",
    tp6 => "TP6",
    tp7 => "TP7",
    tp8 => "TP8",
    tp9 => "TP9",
    tp10 => "TP10",
    tp11 => "TP11",
    tp12 => "TP12",
);
