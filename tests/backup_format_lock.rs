//! Backup file format lock (P0-4, unit U0).
//!
//! A backup is written today and restored months later, by whatever version
//! the user has then. These fixtures were made by released binaries and must
//! keep opening, byte for byte, in every later version:
//!
//! | File | Made by |
//! |---|---|
//! | `tests/golden/backup/golden-0.10.0.backup` | evnx 0.10.0 (review package, backup-restore BR10) |
//! | `tests/golden/backup/golden-0.10.2.backup` | evnx 0.10.2, `805fa2a` |
//!
//! Both encrypt `golden.env` with the key file `golden.key`.
//!
//! `golden-0.10.10-short-key.backup` (evnx 0.10.10 release, key file
//! `golden-short.key`, 3 characters) encrypts the same file with a secret a new
//! backup now refuses (I-147); `restore_integration.rs` checks it still opens
//! (DEC-B2).
//!
//! ⚠️ **Test material only.** `golden.key` is a published test passphrase
//! (`demo-golden-passphrase-NOT-A-SECRET`) and `golden.env` holds synthetic
//! values. Neither protects anything.
//!
//! Before U2 changes how backup and restore write files (I-150, I-151), this is
//! the guard that the change did not alter the format.
#![cfg(feature = "backup")]

mod support;

use support::{run, Project};

fn golden(name: &str) -> String {
    std::fs::read_to_string(support::snapshot::golden_path(&format!("backup/{name}")))
        .expect("read backup fixture")
}

#[track_caller]
fn restores_byte_for_byte(backup: &str) {
    let p = Project::new()
        .file("golden.bak", &golden(backup))
        .file("golden.key", &golden("golden.key"));
    run(
        p.path(),
        &[
            "restore",
            "golden.bak",
            "--key-file",
            "golden.key",
            "--output",
            "golden.out",
        ],
    )
    .assert_code(0);
    assert_eq!(p.read("golden.out"), golden("golden.env"), "{backup}");
}

#[test]
fn backup_made_by_0_10_0_restores() {
    restores_byte_for_byte("golden-0.10.0.backup");
}

#[test]
fn backup_made_by_0_10_2_restores() {
    restores_byte_for_byte("golden-0.10.2.backup");
}

/// A fresh backup of the same content restores to the same bytes, so a
/// writer change cannot pass while producing files only it can read.
#[test]
fn backup_written_today_round_trips() {
    let p = Project::new()
        .env(&golden("golden.env"))
        .file("golden.key", &golden("golden.key"));
    run(
        p.path(),
        &[
            "backup",
            "--key-file",
            "golden.key",
            "--output",
            "today.bak",
        ],
    )
    .assert_code(0);
    run(
        p.path(),
        &[
            "restore",
            "today.bak",
            "--key-file",
            "golden.key",
            "--output",
            "today.out",
        ],
    )
    .assert_code(0);
    assert_eq!(p.read("today.out"), golden("golden.env"));
}
