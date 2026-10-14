//! The `evnx migrate` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-migrate-demo.sh` (copied from the review
//! package, `local/migrate/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/migrate/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`18-migrate.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(all(target_os = "linux", feature = "migrate"))]

mod support;

demo_scenes!(
    "migrate",
    m0 => "M0",
    m1 => "M1",
    m2 => "M2",
    m3 => "M3",
    m4 => "M4",
    m5 => "M5",
    m6 => "M6",
    m7 => "M7",
    m8 => "M8",
    m9 => "M9",
    m10 => "M10",
    m11 => "M11",
    m12 => "M12",
);
