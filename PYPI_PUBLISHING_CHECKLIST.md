# evnx → PyPI Publishing Checklist
## Complete step-by-step guide for first-time publish

---

## FILES TO ADD TO YOUR REPO

| File | Destination in repo | Action |
|------|---------------------|--------|
| `pyproject.toml` | repo root (next to Cargo.toml) | CREATE |
| `README.md` | repo root | UPDATE/CREATE |
| `.github/workflows/python-publish.yml` | `.github/workflows/` | CREATE |

---

## STEP 1 — Update Cargo.toml (one-time check)

Open your Cargo.toml and confirm it has a [[bin]] section:

```toml
[[bin]]
name = "evnx"          # ← this name becomes the CLI command after pip install
path = "src/main.rs"
```

Also confirm the [package] block has these fields (PyPI pulls some from here):
```toml
[package]
name    = "evnx"
version = "0.1.0"      # ← maturin reads this and puts it on PyPI
edition = "2021"
description = "..."
license = "MIT"
repository = "https://github.com/urwithajit9/evnx"
readme = "README.md"
```

---

## STEP 2 — Edit pyproject.toml

Open the pyproject.toml file and update:
  - authors email field (search for: your-email@example.com)
  - description — one sentence about what evnx actually does
  - classifiers — adjust "Development Status" when you hit stable (4 = Beta, 5 = Production)

---

## STEP 3 — Add the PyPI API token to GitHub Secrets

1. Go to: https://pypi.org → Account Settings → API tokens → Add API token
2. Name it something like "evnx-github-actions"
3. Scope: "Entire account" for first publish, then lock it to the project after
4. Copy the token (starts with "pypi-")

5. Go to: https://github.com/urwithajit9/evnx/settings/secrets/actions
6. Click "New repository secret"
7. Name:  PYPI_API_TOKEN
8. Value: paste the pypi- token
9. Click "Add secret"

---

## STEP 4 — Test the build locally before pushing

```bash
# Install maturin (one-time)
pip install maturin

# Create a virtual env to test cleanly
python3 -m venv .venv
source .venv/bin/activate        # Windows: .venv\Scripts\activate

# Build and install locally — this is exactly what pip install does
maturin develop --release

# Verify the binary is on PATH
evnx --help
evnx --version

# If that works, build a real wheel file
maturin build --release
ls target/wheels/                # you'll see a .whl file here

# Deactivate when done
deactivate
```

If `maturin develop` works and `evnx --help` runs correctly, you're ready.

---

## STEP 5 — Test on Test PyPI first (strongly recommended)

Test PyPI is a sandbox — mistakes here don't affect the real PyPI.

```bash
# Get a Test PyPI account at: https://test.pypi.org
# Get a Test PyPI token the same way as Step 3

# Build the wheel
maturin build --release

# Upload to Test PyPI manually
pip install twine
twine upload --repository testpypi target/wheels/*
# Enter: __token__ as username
# Enter: your test.pypi.org token as password

# Install from Test PyPI to verify
pip install --index-url https://test.pypi.org/simple/ evnx
evnx --help
```

---

## STEP 6 — First real publish (manual — then switch to tag-based)

When you're confident everything works:

```bash
maturin build --release
twine upload target/wheels/*
# Username: __token__
# Password: your PyPI API token (pypi-...)
```

OR just push a tag and let GitHub Actions do it automatically (Step 7).

---

## STEP 7 — Automated release via git tag (the production workflow)

From this point on, every release to PyPI is just:

```bash
# 1. Update version in Cargo.toml  (e.g. "0.1.0" → "0.2.0")
# 2. Commit the version bump
git add Cargo.toml
git commit -m "chore: bump version to 0.2.0"
git push

# 3. Tag it — this triggers python-publish.yml AND release.yml simultaneously
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions will:
  - Build wheels for Linux x86_64, Linux aarch64, Linux armv7,
    macOS Intel, macOS Apple Silicon, Windows x86_64
  - Build the source distribution (sdist)
  - Validate all wheels with twine
  - Upload everything to PyPI
  - release.yml (existing) runs in parallel and posts the GitHub Release

Total time: ~8-12 minutes. No manual steps.

---

## STEP 8 — Verify the publish

After the Actions workflow turns green:

```bash
# Wait ~2 minutes for PyPI to index
pip install evnx          # should pull the new version
evnx --version            # should match the tag you pushed

# Check the PyPI page
# https://pypi.org/project/evnx
```

---

## WHAT USERS WILL SEE

After publishing, anyone can install with:

```bash
# Standard Python install
pip install evnx

# Isolated CLI install (recommended for tools)
pipx install evnx

# Pin to exact version in CI
pip install evnx==0.2.0
```

The `evnx` command is immediately available on their PATH — no Rust required.

---

## TROUBLESHOOTING

### "pyproject.toml: bindings = 'bin' not found"
→ Make sure maturin >= 1.0 is installed: `pip install --upgrade maturin`

### "error: binary name 'evnx' not found in Cargo.toml"
→ Add [[bin]] name = "evnx" to Cargo.toml (see Step 1)

### GitHub Actions: "Error: PYPI_API_TOKEN not found"
→ Re-check Step 3: the secret name must be exactly PYPI_API_TOKEN

### "twine check failed: missing long description"
→ Your README.md is missing or not listed in pyproject.toml [project] readme field

### Build passes locally but fails in CI on aarch64
→ Normal for cross-compilation issues. Check the maturin-action logs.
   Usually fixed by using manylinux: auto in the workflow (already set).

---

## VERSION SYNC RULE (important)

Cargo.toml version and pyproject.toml version must ALWAYS match.
maturin enforces this and will fail the build if they differ.

Recommended: only set version in Cargo.toml, and in pyproject.toml use:
  dynamic = ["version"]   ← maturin reads it from Cargo.toml automatically