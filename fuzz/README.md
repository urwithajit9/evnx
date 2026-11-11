# Fuzz targets

Local [cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html) targets for
the three places evnx reads bytes it did not write. They are **not** part of CI
or of any normal build: this directory is its own cargo workspace (`[workspace]`
in `Cargo.toml`), so `cargo build` and `cargo test` at the repository root never
see it, and the root `Cargo.toml` excludes it from the published crate.

| Target | What it feeds | Property |
|---|---|---|
| `env_parser` | arbitrary text to the `.env` parser, under every `ParserConfig` the first byte selects | no panic, no hang |
| `scan_engine` | one in-memory file to the scan engine; the first byte picks `.env`, plain text, JSON, YAML, TOML or a Jupyter notebook | no panic, no hang |
| `crypto_decrypt` | arbitrary bytes to `evnx-crypto`'s decrypt, unwrap and decode functions | every call returns an error, none panics, none succeeds |

## Setup, once

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

## Run

From the repository root (the paths below are relative to it):

```sh
mkdir -p fuzz/corpus/env_parser fuzz/corpus/scan_engine fuzz/corpus/crypto_decrypt
cargo +nightly fuzz run env_parser      fuzz/corpus/env_parser      fuzz/seeds/env_parser      -- -max_total_time=300
cargo +nightly fuzz run scan_engine     fuzz/corpus/scan_engine     fuzz/seeds/scan_engine     -- -max_total_time=300
cargo +nightly fuzz run crypto_decrypt  fuzz/corpus/crypto_decrypt  fuzz/seeds/crypto_decrypt  -- -max_total_time=300
```

- The first directory is where libFuzzer keeps what it finds (`fuzz/corpus/` is
  git-ignored); the second is the small committed seed set, read but not
  written. A crash is saved to `fuzz/artifacts/<target>/crash-<hash>`.
- `-max_total_time=300` stops after five minutes. Drop it to run until you stop it.
- `-rss_limit_mb=4096` raises libFuzzer's memory limit if a target reports an
  out-of-memory it should not have; `-timeout=10` is the per-input hang limit.
- `crypto_decrypt` derives one Argon2id key at start-up (about a second); each
  iteration after that is fast.

Without a nightly toolchain, the same build works on stable with
`RUSTC_BOOTSTRAP=1 cargo fuzz run --sanitizer none <target> ...`. That is a
workaround for machines that cannot install nightly, and it drops AddressSanitizer,
so memory errors in unsafe dependencies would go unnoticed; panics, hangs and
out-of-memory are still found.

## When a target finds something

1. Reproduce: `cargo +nightly fuzz run <target> fuzz/artifacts/<target>/crash-<hash>`.
2. Minimise: `cargo +nightly fuzz tmin <target> fuzz/artifacts/<target>/crash-<hash>`.
3. Fix the code, and put the minimised input in an ordinary unit test next to it
   (as a string or byte literal), not only in the corpus: the corpus is not
   committed, and a regression test has to run in CI.
4. Commit the fix and its test together.

## Checking they still build

```sh
cd fuzz && cargo check --bins
```

This works on stable and is all that is needed to confirm a refactor did not
break a target's imports.
