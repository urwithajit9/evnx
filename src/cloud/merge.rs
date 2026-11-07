//! Several vaults into one set of variables (`--vault` given more than once,
//! RM-59, decision X4).
//!
//! `cloud run` and `cloud pull` both accept a repeatable `--vault`. The vaults
//! are fetched, unlocked with **one** master-password prompt, decrypted one by
//! one and merged **left to right**: the first vault's keys come first, in its
//! own order, then each later vault's new keys in theirs.
//!
//! A key that two or more vaults hold is an **error** unless `--allow-override`
//! is given, and the error is raised before a child is spawned or a file is
//! written. With the flag the later vault wins, and each overlap is named
//! (key and winning vault — never a value).
//!
//! Single-vault runs never come through here.

use anyhow::{anyhow, Result};
use colored::Colorize;
use indexmap::IndexMap;
use zeroize::Zeroize;

use super::client::Client;
use super::vault::VaultRef;
use super::{auth, sync};

/// A key held by more than one of the merged vaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Overlap {
    pub key: String,
    /// Every vault holding the key, in the order given. The last one wins when
    /// overriding is allowed.
    pub vaults: Vec<String>,
}

impl Overlap {
    pub fn winner(&self) -> &str {
        self.vaults.last().map(String::as_str).unwrap_or("")
    }

    /// `K is in both a and b` / `K is in a, b and c`.
    fn describe(&self) -> String {
        match self.vaults.as_slice() {
            [a, b] => format!("{} is in both {a} and {b}", self.key),
            [init @ .., last] => format!("{} is in {} and {last}", self.key, init.join(", ")),
            [] => self.key.clone(),
        }
    }
}

/// Merge `(label, variables)` pairs left to right.
///
/// Returns the merged map and every overlap found. Without `allow_override` an
/// overlap is an error naming each key and the vaults it is in; with it the
/// later value replaces the earlier one (keeping the key's first position).
/// The error carries key and vault names only.
pub(crate) fn merge_vaults(
    vaults: Vec<(String, IndexMap<String, String>)>,
    allow_override: bool,
) -> Result<(IndexMap<String, String>, Vec<Overlap>)> {
    let mut merged: IndexMap<String, String> = IndexMap::new();
    let mut holders: IndexMap<String, Vec<String>> = IndexMap::new();
    for (label, vars) in vaults {
        for (key, value) in vars {
            holders.entry(key.clone()).or_default().push(label.clone());
            merged.insert(key, value);
        }
    }
    let overlaps: Vec<Overlap> = holders
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(key, vaults)| Overlap { key, vaults })
        .collect();

    if !overlaps.is_empty() && !allow_override {
        let lines: Vec<String> = overlaps
            .iter()
            .map(|o| format!("  {}", o.describe()))
            .collect();
        return Err(anyhow!(
            "{} variable(s) are in more than one of the given vaults:\n{}\n\
             \x20 Nothing was run or written. Pass --allow-override to let the later vault win, \
             or drop the key from one of them.",
            overlaps.len(),
            lines.join("\n")
        ));
    }
    Ok((merged, overlaps))
}

/// One vault that went into a merge, for messages.
pub(crate) struct Source {
    pub vault_ref: VaultRef,
    pub version: i32,
}

pub(crate) struct Merged {
    pub vars: IndexMap<String, String>,
    pub sources: Vec<Source>,
}

impl Merged {
    /// `acme/shared v3 + acme/prod v7`.
    pub fn describe(&self) -> String {
        self.sources
            .iter()
            .map(|s| format!("{} v{}", s.vault_ref.label(), s.version))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    pub fn is_production(&self) -> bool {
        self.sources
            .iter()
            .any(|s| s.vault_ref.environment == "production")
    }

    /// The merged variables as `.env` text.
    pub fn to_env_text(&self) -> zeroize::Zeroizing<Vec<u8>> {
        let mut text = String::new();
        for (k, v) in &self.vars {
            text.push_str(&crate::core::envfile::line(k, v));
            text.push('\n');
        }
        let bytes = text.as_bytes().to_vec();
        text.zeroize();
        zeroize::Zeroizing::new(bytes)
    }
}

/// Fetch every vault's latest version, unlock once, decrypt and merge.
///
/// Blobs are all fetched before the password is asked for, as in the single
/// vault path: a vault with no versions answers before any typing.
pub(crate) fn fetch_merged(
    client: &Client,
    refs: Vec<VaultRef>,
    password_stdin: bool,
    allow_override: bool,
) -> Result<Merged> {
    let mut fetched = Vec::with_capacity(refs.len());
    for vault_ref in refs {
        let version = sync::current_version(client, &vault_ref)?;
        if version == 0 {
            return Err(anyhow!(
                "{} has no versions yet. Push one with `evnx cloud push --vault {}`.",
                vault_ref.label(),
                vault_ref.label()
            ));
        }
        let blob = sync::fetch_blob(client, &vault_ref, version)?;
        fetched.push((vault_ref, version, blob));
    }

    let password = auth::read_master_password(password_stdin)?;
    let unlocked = auth::unlock(client, &password)?;

    let mut parsed = Vec::with_capacity(fetched.len());
    let mut sources = Vec::with_capacity(fetched.len());
    for (vault_ref, version, blob) in fetched {
        let vault_key = sync::unwrap_vault_key(client, &vault_ref, &unlocked)?;
        let mut plaintext = sync::decrypt_version(&blob, &vault_ref, version, &vault_key)?;
        let vars = super::run::parse_env(&plaintext);
        plaintext.zeroize();
        parsed.push((vault_ref.label(), vars?));
        sources.push(Source { vault_ref, version });
    }

    let (vars, overlaps) = merge_vaults(parsed, allow_override)?;
    if !ui_quiet() {
        for o in &overlaps {
            eprintln!(
                "  {} {} is in {}; {} wins",
                "·".cyan(),
                o.key,
                o.vaults.join(", "),
                o.winner()
            );
        }
    }
    Ok(Merged { vars, sources })
}

fn ui_quiet() -> bool {
    crate::utils::ui::is_quiet()
}

/// Refuse what only makes sense for one vault. Called before any request.
pub(crate) fn check_usage(
    vault_count: usize,
    version: Option<i32>,
    allow_override: bool,
) -> Result<()> {
    if vault_count > 1 && version.is_some() {
        return Err(anyhow!(
            "--version selects a version of one vault, so it cannot be combined with more than \
             one --vault. Pull each vault on its own to choose its version."
        ));
    }
    if vault_count <= 1 && allow_override {
        crate::utils::ui::warning("--allow-override has no effect without more than one --vault");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn two() -> Vec<(String, IndexMap<String, String>)> {
        vec![
            ("acme/shared".into(), vars(&[("A", "1"), ("B", "2")])),
            ("acme/prod".into(), vars(&[("C", "3"), ("D", "4")])),
        ]
    }

    #[test]
    fn disjoint_vaults_merge_first_vault_first() {
        let (m, o) = merge_vaults(two(), false).unwrap();
        assert!(o.is_empty());
        assert_eq!(m.keys().cloned().collect::<Vec<_>>(), ["A", "B", "C", "D"]);
    }

    #[test]
    fn an_overlap_is_refused_naming_key_and_both_vaults_but_no_value() {
        let v = vec![
            (
                "acme/shared".into(),
                vars(&[("DATABASE_URL", "s3cr3t-one")]),
            ),
            ("acme/prod".into(), vars(&[("DATABASE_URL", "s3cr3t-two")])),
        ];
        let err = merge_vaults(v, false).unwrap_err().to_string();
        assert!(
            err.contains("DATABASE_URL is in both acme/shared and acme/prod"),
            "{err}"
        );
        assert!(err.contains("--allow-override"), "{err}");
        assert!(!err.contains("s3cr3t"), "{err}");
    }

    #[test]
    fn allow_override_lets_the_later_vault_win_and_reports_it() {
        let v = vec![
            ("a".into(), vars(&[("K", "old"), ("X", "x")])),
            ("b".into(), vars(&[("K", "new"), ("Y", "y")])),
            ("c".into(), vars(&[("K", "newest")])),
        ];
        let (m, o) = merge_vaults(v, true).unwrap();
        assert_eq!(m["K"], "newest");
        assert_eq!(m.keys().cloned().collect::<Vec<_>>(), ["K", "X", "Y"]);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].key, "K");
        assert_eq!(o[0].vaults, ["a", "b", "c"]);
        assert_eq!(o[0].winner(), "c");
    }

    #[test]
    fn three_way_overlap_reads_as_a_list() {
        let o = Overlap {
            key: "K".into(),
            vaults: vec!["a".into(), "b".into(), "c".into()],
        };
        assert_eq!(o.describe(), "K is in a, b and c");
    }

    #[test]
    fn version_with_several_vaults_is_a_usage_error() {
        assert!(check_usage(2, Some(1), false).is_err());
        assert!(check_usage(1, Some(1), false).is_ok());
        assert!(check_usage(2, None, true).is_ok());
    }
}
