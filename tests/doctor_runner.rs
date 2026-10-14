//! The `evnx doctor` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-doctor-demo.sh` (copied from the review
//! package, `local/doctor/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/doctor/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`22-doctor.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(target_os = "linux")]

mod support;

demo_scenes!(
    "doctor",
    dr0 => "DR0",
    dr1 => "DR1",
    dr2 => "DR2",
    dr3 => "DR3",
    dr4 => "DR4",
    dr5 => "DR5",
    dr6 => "DR6",
    dr7 => "DR7",
    dr8 => "DR8",
    dr9 => "DR9",
    dr10 => "DR10",
);
