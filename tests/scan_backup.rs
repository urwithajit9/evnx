//! `evnx scan` and files named like a backup (RM-84, I-293).
//!
//! An `evnx backup` file is ciphertext, so scanning it only produces a false
//! high-entropy finding. It used to be skipped by **name**, so a plaintext
//! `cp .env .env.backup` was never scanned either: real secrets, exit 0. The
//! content decides now, as it does for `doctor` (I-160).

mod support;

use support::{run, Project};

const STRIPE: &str = "STRIPE_SECRET_KEY=sk_live_51H8xQ2eZvKYlo2CpR4mN7bV9\n";

#[test]
fn rm84_a_plaintext_copy_named_like_a_backup_is_scanned() {
    for name in [".env.backup", ".env.production.backup", ".env.backup.2"] {
        let p = Project::new().file(".env", "SAFE=1\n").file(name, STRIPE);
        let out = run(p.path(), &["scan"]);
        out.assert_code(1);
        assert!(
            out.stdout.contains(name),
            "{name} was not scanned: {}",
            out.stdout
        );
    }
}

#[cfg(feature = "backup")]
#[test]
fn rm84_a_real_evnx_backup_is_still_skipped() {
    let p = Project::new().file(".env", "SAFE=1\n");
    support::run_with(
        p.path(),
        &["backup", ".env", "--output", ".env.backup"],
        &[("EVNX_PASSWORD", "correct horse battery staple")],
        None,
    )
    .assert_code(0);
    assert!(p.exists(".env.backup"));

    let out = run(p.path(), &["scan"]);
    out.assert_code(0);
    assert!(!out.stdout.contains(".env.backup"), "{}", out.stdout);
}
