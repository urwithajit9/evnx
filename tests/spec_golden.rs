//! The `[vars]` block `evnx spec init` writes, pinned (P0-4, unit U0).
//!
//! `.evnx.toml` is a file people commit and other commands read, so the block
//! is a contract. The command is the `--help` example (D-127, fixed in
//! 0.10.1).
//!
//! ⚠️ The first lines of the golden file are the banner, which 0.10.2 prints
//! on stdout. That is I-176: U1 moves it to stderr (DEC-SP1) and regenerates
//! this file on purpose. Everything from the `# ── Variable contract` line
//! down must stay byte for byte through U1 and U3.

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
