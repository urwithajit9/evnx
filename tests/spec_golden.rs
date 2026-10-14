//! The `[vars]` block `evnx spec init` writes, pinned (P0-4, unit U0).
//!
//! `.evnx.toml` is a file people commit and other commands read, so the block
//! is a contract. The command is the `--help` example (D-127, fixed in
//! 0.10.1).
//!
//! Until 0.10.2 the first lines here were the banner, printed on stdout
//! (I-176). Since DEC-SP1 / DEC-G1 it is on stderr, and this file holds only
//! TOML; `tests/spec_integration.rs` parses it as such.

mod support;

use support::snapshot::{check_golden, normalise};
use support::{run, Project};

#[test]
fn spec_init_stdout_block_is_stable() {
    let p = Project::new();
    let out = run(
        p.path(),
        &[
            "spec",
            "init",
            "--with",
            "django,postgres,redis",
            "--stdout",
        ],
    );
    out.assert_code(0);
    check_golden(
        "spec/init-django-postgres-redis.stdout",
        &normalise(&out.stdout, &[p.path()]),
    );
}
