# Demo runners (behaviour lock)

Copies of the training demos from the review package (`local/<cmd>/tutorial/`).
Each prints every command it runs, the output and `[exit N]`, then checks the
result: `ok:` when a check holds, `MISMATCH:` when it does not. Lines marked
`KNOWN I-nnn` show a logged defect as it is today.

| Run it | How |
|---|---|
| In CI, one test per scene | `cargo test --all-features --test sync_runner` (golden files in `tests/golden/demos/`) |
| Twice with one binary, or old vs new | `scripts/behaviour-lock.sh sync [old-evnx] [new-evnx]` |
| By hand, one scene | `EVNX=target/debug/evnx tests/demos/run-sync-demo.sh S5` |

When a fix changes a scene on purpose:

1. Change the runner line in this folder (flip the `KNOWN` check, name the decision id).
2. `UPDATE_GOLDEN=1 cargo test --all-features --test <cmd>_runner`
3. Read `git diff tests/golden/demos/<cmd>/`: every changed line must be the fix.
4. Make the same edit in the review package copy, so the two do not drift.

Differences from the package copies are marked `U0-EDIT` in each file.

Linux only (bash, GNU tools, git; python3 and nc for a few scenes). The
`backup` and `migrate` runners need their cargo features.
