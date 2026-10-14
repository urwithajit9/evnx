//! The `evnx backup` training demo as a behaviour lock (P0-3, unit U0).
//!
//! Each scene of `tests/demos/run-backup-demo.sh` (copied from the review
//! package, `local/backup-restore/tutorial/`) runs as its own test and must print
//! exactly `tests/golden/demos/backup/<scene>.txt`. Lines marked `KNOWN` show a
//! defect logged in the review (`21-backup-restore.md`) as it is today; a fix flips them
//! on purpose. See `tests/support/demo.rs` for how to regenerate.
#![cfg(all(target_os = "linux", feature = "backup"))]

mod support;

demo_scenes!(
    "backup",
    br0 => "BR0",
    br1 => "BR1",
    br2 => "BR2",
    br3 => "BR3",
    br4 => "BR4",
    br5 => "BR5",
    br6 => "BR6",
    br7 => "BR7",
    br8 => "BR8",
    br9 => "BR9",
    br10 => "BR10",
    br11 => "BR11",
    br12 => "BR12",
);
