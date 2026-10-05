# Releasing evnx

Nine channels fire from one tag. This is the order, and the traps.

> ⚠️ **Written 2026-10-05**, after four releases that each hit something nobody
> had written down. Every warning here is something that actually happened.

---

## What a tag sets off

Pushing `vX.Y.Z` to **`main`** starts four workflows, and a fifth chains off one
of them:

| Workflow | Fires on | Publishes |
|---|---|---|
| `release.yml` | `v*` tag | GitHub Release (16 assets) → Homebrew tap, Scoop bucket, **winget** |
| `python-publish.yml` | `v*` tag | PyPI |
| `publish-action.yml` | `v*` tag | GitHub Marketplace action |
| `publish-image.yml` | `v*` tag + push to `main` | ghcr.io |
| `npm-publish.yml` | **`release.yml` completing** | npm — 5 platform packages, then `@evnx/cli` |

**crates.io is not in that list.** It is fully manual: `cargo publish` from the
tag. No workflow exists, and nothing reminds you.

⛔ **Never tag from a feature branch.** Three of those fire on `v*` regardless of
which branch the tag points at, so a stray tag publishes a feature branch to five
registries. Tag only from `main`, after the PR is merged.

---

## Before the tag

### 1 · The server, if the release needs one

⚠️ **Deploy `api.evnx.dev` first.** A CLI command that 404s against production is
worse than one that does not exist. 0.8.0 needed `GET /auth/devices`; 0.9.0 needed
the organisation endpoints and migrations 010–012.

```bash
curl -s https://api.evnx.dev/health     # the build sha must be the one you expect
```

### 2 · Documentation, if a command is new

⚠️ **The binary prints documentation URLs, and they are compiled in.**
`src/docs.rs` holds one `CommandDoc` per subcommand, and five tests hold them to
the host and path shape — but **no test can tell whether the page exists.**

So the order is: publish the guide, verify the URL, *then* add the `docs.rs`
entry.

```bash
curl -s -o /dev/null -w '%{http_code}\n' https://docs.evnx.dev/cli/commands/<name>
```

ⓘ `draft: true` in a guide's frontmatter means **unlisted, not unreachable** —
`getGuide()` does not filter drafts, only `getAllGuides()` does. So a draft guide
answers 200 while being absent from the sidebar, the index and the sitemap. Good
enough for the binary, not good enough for a release.

⚠️ **Internal links in guides are written `/guides/...`, never `/cli/...`.**
`DOCS_MODE` rewrites the prefix at render time. Writing the rendered form produces
links that are dead on the live site, and `pnpm check:links` is what catches it.

### 3 · The version, in both places

```bash
# evnx/Cargo.toml  →  version = "X.Y.Z"
cargo check          # updates Cargo.lock's own entry
```

Nothing else in the repo carries the version. The packaging artefacts all derive
it from the tag.

### 4 · CHANGELOG, and say why the number

In `0.x`, features are a **minor** bump and fixes are a patch. A change to the
output of a command that already shipped is a compatibility change, which is also
a minor bump — 0.8.0 was 0.8.0 rather than 0.7.1 for exactly that reason, and the
entry says so. Write that reasoning down; somebody will ask.

### 5 · README, if the surface changed

The README enumerates commands. A new top-level command that is not there is a
command most people never find.

### 6 · Sync the winget-pkgs fork

⚠️ **`wingetcreate` fails if the fork is behind**, and it has been tens of
thousands of commits behind before. Sync it on GitHub before tagging.

### 7 · The gates

```bash
cargo fmt --check
cargo clippy --all-features --all-targets -- -D warnings
cargo clippy --all-targets -- -D warnings      # default features too
cargo test --all-features
cargo build --release --all-features && ./target/release/evnx --version
```

⚠️ **Lint both feature sets.** CI has linted only `--all-features` before, which
is how unused imports under default features survived.

---

## The tag

```bash
git checkout main && git pull origin main
git tag -a vX.Y.Z -m "evnx vX.Y.Z"
git push origin vX.Y.Z
```

---

## After the tag — what to actually check

⚠️ **A green workflow run is not evidence that anything published.** Each of
these has produced a green run while publishing nothing:

| Channel | Check | The trap |
|---|---|---|
| GitHub Release | 16 assets + checksums | |
| **npm** | `npm view @evnx/cli version` | `npm-publish.yml` fires on `release.yml` *completing* — including when it **fails**, where it correctly skips and still reports green. A green npm run means nothing; read the registry. |
| **crates.io** | `cargo publish` — **manual** | Nothing automates it and nothing reminds you. evnx-crypto 0.1.1 and 0.1.2 exist as git tags and npm releases only, because this step was skipped twice. |
| PyPI | install the wheel and run the binary | |
| Homebrew / Scoop | formula and manifest bumped | |
| ghcr.io | new digest, not the old one | |
| **winget** | a PR against microsoft/winget-pkgs | Validation has blocked this package twice. See below. |

⚠️ **Checking crates.io needs a `User-Agent`.** Their API refuses an anonymous
request and answers with HTML, so a naive `curl | python3 -c 'json.load(...)'`
dies with a decode error that reads exactly like the crate is missing. It is not:

```bash
curl -s -H "User-Agent: evnx-release-check (urwithajit9)" \
  https://crates.io/api/v1/crates/evnx \
  | python3 -c "import json,sys; print(json.load(sys.stdin)['crate']['max_version'])"
```

Cost ten confusing minutes during 0.9.0, on a release that had in fact published
correctly.

⚠️ **Do not probe a release with `npx -y @evnx/cli@X.Y.Z --version` on a machine
that has evnx installed.** A package-manager install earlier in `PATH` answers
instead, and the output looks like a broken release. Probe the artefact:

```bash
cd "$(mktemp -d)" && npm install @evnx/cli@X.Y.Z >/dev/null && ./node_modules/@evnx/cli/bin/evnx --version
```

### Winget validation

⚠️ **It has blocked this package twice**, as `Trojan:Win32/Sprisky.U!cl` — a
cloud-delivered ML verdict, not a signature match, so it is **not stable across
releases in either direction**. A clean pass is not proof the problem is gone.

The root cause is that the Windows binary is **unsigned**, which is what Defender's
heuristics target. Clearance goes through
`microsoft.com/en-us/wdsi/filesubmission` — upload the **.exe**, not the ZIP.

⚠️ Do not read a merged PR's final labels as its history: `Validation-Defender-Error`
is *removed* once cleared, so a blocked-then-cleared PR looks identical to one that
sailed through. Read `/issues/{n}/events` instead.

---

## Known, and deliberately not fixed

| | |
|---|---|
| **The Marketplace action's pinned-version install is broken.** `urwithajit9/evnx-action` downloads `releases/download/<version>/evnx-linux-x86_64`, and **no release has ever had an asset by that name** — the real one is `evnx-x86_64-unknown-linux-musl.tar.gz`, a tarball. `version: latest` (the default) works, because it goes through `dotenv.space/install.sh`. Pinning a version writes a 404 page to `/usr/local/bin/evnx`. Verified 2026-10-05 against the v0.8.0 asset list. **Fix belongs in the `evnx-action` repo.** |
| **`publish-action.yml`'s version bump is a no-op.** The pattern `evnx-vX.Y.Z` appears nowhere in the action's `action.yml`. Harmless — the action resolves the version at runtime — and the step now says so in the log instead of passing silently. |
