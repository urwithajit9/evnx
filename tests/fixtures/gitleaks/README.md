# gitleaks' default rules, pinned

`gitleaks.toml` is gitleaks' `config/gitleaks.toml` at **v8.30.1** (commit
`83d9cd684c87d95d656c1458ef04895a7f1cbd8e`), unmodified:

```
e163e53b9e7e8a8511e77271e2b323ed057759542a6d988258afe3a1fa329caf  gitleaks.toml
```

`LICENSE` is gitleaks' MIT licence from the same tag. `tests/scan_packs.rs` converts
this file with `evnx scan rules add --format gitleaks` and asserts the exact counts (221
converted, 1 skipped), so a change to the importer, or a re-pin, shows up as a test
diff. It holds detection rules, not secrets.

To re-pin, download the new tag's `config/gitleaks.toml` and `LICENSE` here, update the
tag, commit and sha256 above, and update the counts in the test.
