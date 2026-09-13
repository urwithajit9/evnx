//! Cloud sync — zero-knowledge encrypted `.env` storage.
//!
//! Gated behind the `cloud` feature, which is **not** in `default` and **not** in
//! `full`. `cargo install evnx` and `--features full` are unchanged for existing
//! users; nobody gets network-capable code they did not ask for.
//!
//! # The zero-knowledge boundary runs through this module
//!
//! Encryption and decryption happen here, on the user's machine. The server
//! receives ciphertext and never holds anything that can open it — not the master
//! key, not a vault key, not the private key in usable form. Any change here that
//! would send plaintext or key material to the server breaks the guarantee the
//! whole product rests on. Treat that as the review question for every PR that
//! touches this directory.
//!
//! # Layout, filled in over PRs 2–8
//!
//! ```text
//! cloud/
//! ├── mod.rs      this file — subcommand dispatch
//! ├── status.rs   `evnx cloud status`
//! ├── config.rs   ~/.config/evnx/                                    PR 2
//! ├── creds.rs    credential store, file backend at mode 0600        PR 2
//! ├── client.rs   HTTP, typed errors, refresh-on-401, --server       PR 3
//! ├── auth.rs     register / login / logout / status                 PR 4–5
//! ├── vault.rs    create / list / delete                             PR 6
//! └── sync.rs     push / pull / history                              PR 7–8
//! ```
//!
//! # Why blocking HTTP, not async
//!
//! `main()` is synchronous, and the `migrate` feature already speaks HTTP through
//! `reqwest::blocking` with `rustls-tls`. Adding an async runtime for this feature
//! alone would pull in tokio, thread a runtime handle through every command, and
//! re-open the cross-compilation trouble the migrate feature already worked
//! through — the commented-out TLS attempts in `Cargo.toml` are that history. A
//! cloud command makes a handful of requests, so blocking is the right shape.
//!
//! # Crypto comes from crates.io, never a path dependency
//!
//! `evnx-crypto = "0.1"` is the same published crate evnx-server links against, so
//! both ends of the wire agree byte for byte. A path dependency would also make
//! `cargo install evnx --features cloud` impossible, since `cargo publish` rejects
//! path deps without a published version.

pub mod status;

use crate::cli::CloudCommands;
use anyhow::Result;

/// Dispatch an `evnx cloud …` subcommand.
pub fn run(command: CloudCommands, verbose: bool) -> Result<()> {
    match command {
        CloudCommands::Status => status::run(verbose),
    }
}
