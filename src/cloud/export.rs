//! `evnx cloud export` — every version, decrypted, in a format that is not ours.
//!
//! # This is the one command whose purpose is to write plaintext to disk
//!
//! Everything else in `cloud/` works to avoid that. `cloud pull` writes one file
//! and asks before overwriting; `cloud run` exists so a CI job never writes one
//! at all. This command writes **every secret of every version** into a
//! directory, and that is not an oversight — it is the whole feature.
//!
//! The question it answers is *what happens when we want to leave?*, and an
//! escape hatch nobody can demonstrate is not believed. A team evaluating a
//! secrets vault asks it before they adopt, not after.
//!
//! ⚠️ **It is not [`super::auth::download_data`].** That writes account metadata
//! and contains no secrets by construction — the server has never held them.
//! This decrypts. The two were deliberately renamed apart so no one would reach
//! for the wrong one.
//!
//! # Why the default writes bytes rather than a format
//!
//! `push` encrypts the file verbatim — comments, ordering, blank lines and
//! quoting all survive. So with no `--to`, export writes those bytes back
//! unchanged and you get your file, not a reconstruction of it. `--to` opts into
//! a conversion, and a conversion is lossy about everything that is not a
//! key/value pair.
//!
//! # What it refuses, and when
//!
//! An unusable `--to`, and a non-empty output directory, are both refused
//! **before** the master password is asked for and before a single blob is
//! fetched. A refusal that arrives after minutes of decryption teaches people to
//! pass `--force` by reflex.

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
#[cfg(test)]
use indexmap::IndexMap;
use serde_json::json;
use std::path::{Path, PathBuf};

use super::auth;
use super::client::Client;
use super::config::CloudConfig;
use super::sync::{self, VersionSummary};
use super::vault::{self, VaultRef};

/// Where an export lands when `--output` is not given.
const DEFAULT_OUTPUT_DIR: &str = "evnx-export";

/// Export a vault's versions to a directory.
#[allow(clippy::too_many_arguments)]
pub fn export(
    server_override: Option<&str>,
    vault_target: Option<String>,
    output: Option<PathBuf>,
    all_versions: bool,
    to: Option<String>,
    force: bool,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    // ⚠️ First, before anything that costs a round trip or a password. A
    // mistyped `--to` is the cheapest possible failure and should stay that way.
    //
    // This only became possible when the dispatch stopped calling
    // `std::process::exit(2)` — see `formats::get_converter`.
    let converter = match &to {
        Some(name) => Some(crate::formats::get_converter(name).map_err(|_| {
            anyhow!(
                "unknown format: {name}\n\
                 \x20 Run `evnx convert --help` for the full list, or omit --to to \n\
                 \x20 export the files exactly as they were pushed.",
                name = name
            )
        })?),
        None => None,
    };

    let vault_target = sync::resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    vault::require_session(&client, &server)?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let root = output.unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT_DIR));
    let dir = root.join(directory_name(&vault_ref));

    // ⚠️ Also before the password. Same reason.
    refuse_non_empty(&dir, force)?;

    let mut versions = sync::list_versions(&client, &vault_ref)?;
    if versions.is_empty() {
        return Err(anyhow!(
            "{} has no versions to export. Push one with `evnx cloud push`.",
            vault_ref.label()
        ));
    }
    versions.sort_by_key(|v| v.version_num);

    let latest = versions
        .last()
        .map(|v| v.version_num)
        .expect("non-empty, checked above");

    let wanted: Vec<&VersionSummary> = if all_versions {
        versions.iter().collect()
    } else {
        versions
            .iter()
            .filter(|v| v.version_num == latest)
            .collect()
    };

    println!();
    println!(
        "  exporting {} version(s) of {} to {}",
        wanted.len().to_string().bold(),
        vault_ref.label().bold(),
        dir.display().to_string().bold()
    );
    if !all_versions && versions.len() > 1 {
        println!(
            "  {} is the latest of {}. Pass {} for all of them.",
            format!("v{latest}").dimmed(),
            versions.len(),
            "--all-versions".cyan()
        );
    }
    println!();

    let password = auth::read_password(password_stdin, "Master password")?;
    let master_key = auth::derive_master_key_for_account(&client, &password)?;

    // Opened **once** and reused for every version. The key is per vault, not
    // per version, so unwrapping it in the loop would be one needless round trip
    // per version and would make a 40-version export forty times as chatty.
    let vault_key = sync::unwrap_vault_key(&client, &vault_ref, &master_key)?;

    create_dir_private(&dir)?;

    let extension = converter
        .as_ref()
        .map(|c| crate::formats::extension_for(c.name()))
        .unwrap_or("env");

    let mut written = Vec::new();
    for summary in &wanted {
        let n = summary.version_num;
        let blob = sync::fetch_blob(&client, &vault_ref, n)?;

        // ⚠️ `n` here is the number the blob was fetched under, and it goes into
        // the AAD. Pairing one version's ciphertext with another's AAD fails
        // rather than decrypting to something plausible — which is exactly the
        // property a loop like this needs.
        let plaintext = sync::decrypt_version(&blob, &vault_ref, n, &vault_key)?;

        let bytes = match &converter {
            None => plaintext,
            Some(c) => {
                let vars = super::run::parse_env(&plaintext).with_context(|| {
                    format!(
                        "version {n} of {} could not be parsed into variables, so it \
                         cannot be converted. Export without --to to get the file as \
                         it was pushed.",
                        vault_ref.label()
                    )
                })?;
                let converted = c
                    .convert(&vars, &Default::default())
                    .with_context(|| format!("converting version {n} to {}", c.name()))?;
                newline_terminated(converted).into_bytes()
            }
        };

        let name = format!("v{n}.{extension}");
        let path = dir.join(&name);
        super::creds::write_atomic_secure(&path, &bytes)
            .with_context(|| format!("writing {}", path.display()))?;

        println!(
            "  {} {}  {} variable(s), {} bytes",
            "✓".green(),
            name.bold(),
            summary.key_count,
            bytes.len()
        );
        written.push((summary, name, bytes.len()));
    }

    let manifest = manifest(&vault_ref, &server, latest, extension, &to, &written);
    let manifest_path = dir.join("manifest.json");
    super::creds::write_atomic_secure(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)
            .context("formatting the manifest")?
            .as_bytes(),
    )
    .with_context(|| format!("writing {}", manifest_path.display()))?;
    println!("  {} manifest.json", "✓".green());

    println!();
    println!(
        "  {}",
        "⚠ These files hold your secrets in plain text."
            .yellow()
            .bold()
    );
    println!(
        "  {}",
        "  Written 0600 in a 0700 directory, but that protects them only from".yellow()
    );
    println!(
        "  {}",
        "  other users on this machine — not from backup, sync or a commit.".yellow()
    );
    println!();
    println!(
        "  {}",
        "It is a copy taken now. It does not stay in sync, and a stale export of".dimmed()
    );
    println!(
        "  {}",
        "rotated credentials is its own hazard. For CI, use `evnx cloud run`.".dimmed()
    );
    if verbose {
        println!();
        println!("  server:  {server}");
        println!("  vault:   {}", vault_ref.id);
        println!("  format:  {}", to.as_deref().unwrap_or("env (verbatim)"));
    }
    println!();
    Ok(())
}

/// End a converted file with a newline, the way a text file should end.
///
/// ⚠️ **This exists because `convert` disagrees with itself.** Writing to stdout
/// goes through `println!`, so the output ends in a newline; writing the same
/// content with `-o <file>` goes through `fs::write`, so it does not. The two
/// produce different bytes for the same conversion, which is a defect in
/// `convert` rather than something to copy — fixing it there changes the output
/// of a released command, so it is filed separately.
///
/// Export follows the stdout form: it is the one people see, the one they
/// compare against, and the one that makes a well-formed file. A JSON or YAML
/// file with no final newline is the shape every diff tool complains about.
///
/// The verbatim path — no `--to` — never comes through here. Those bytes are
/// what was pushed and must not gain a byte they did not have.
fn newline_terminated(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// The per-vault directory name, safe to put on a filesystem.
///
/// `label()` is `name/environment`, and a slash would make a nested directory
/// nobody asked for. Anything outside `[A-Za-z0-9._-]` becomes `-` so a vault
/// named with a space, a colon or a non-ASCII character still exports.
fn directory_name(vault_ref: &VaultRef) -> String {
    let raw = vault_ref.label();
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();

    // ⚠️ Two shapes must not survive, and both are reachable from a vault name.
    //
    // `""` — a name of nothing but separators — joins onto the root as nothing
    // at all, so the export lands in the root directory rather than a
    // subdirectory of it.
    //
    // `"."` or `".."` — a name of nothing but dots, which `..` with an empty
    // environment produces exactly — is a *relative path component*, and
    // `root.join("..")` is the root's parent. A vault name is user-supplied and
    // the server does not constrain it, so this is a path the caller never
    // asked to write to.
    //
    // Dots are kept everywhere else: `api.v2-production` is a fine directory,
    // and so is `..-..-etc-passwd`, which is a literal name with no slash in it
    // and therefore cannot traverse anywhere.
    let is_only_dots = !trimmed.is_empty() && trimmed.chars().all(|c| c == '.');
    if trimmed.is_empty() || is_only_dots {
        vault_ref.id.clone()
    } else {
        trimmed
    }
}

/// Refuse to write into a directory that already holds something.
///
/// ⚠️ Checked **before** the password prompt and before the first blob, so a
/// refusal costs nothing. Overwriting an earlier export silently is the failure
/// worth avoiding: two exports of the same vault at different times look
/// identical on disk, and the older one may be the one someone still needs.
fn refuse_non_empty(dir: &Path, force: bool) -> Result<()> {
    if force || !dir.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .peekable();
    if entries.peek().is_none() {
        return Ok(());
    }
    Err(anyhow!(
        "{} already has files in it.\n\
         \x20 Pass --force to overwrite them, or -o to export somewhere else.\n\
         \x20 Refusing before anything was decrypted.",
        dir.display()
    ))
}

/// Create the export directory, owner-only where the platform expresses that.
///
/// 0700 rather than the umask default: every file inside is plaintext secrets,
/// and a directory others cannot list is one less way to find them.
fn create_dir_private(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(dir)
            .with_context(|| format!("reading permissions of {}", dir.display()))?
            .permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(dir, perms)
            .with_context(|| format!("setting permissions on {}", dir.display()))?;
    }

    Ok(())
}

/// Describe what was written, so the directory is not a pile of numbered files.
///
/// ⚠️ The first question anyone asks of an export is *which one was current?*
/// `latest_version` is here for that, and every entry carries `is_latest` so the
/// answer survives someone reading a single record out of context.
fn manifest(
    vault_ref: &VaultRef,
    server: &str,
    latest: i32,
    extension: &str,
    format: &Option<String>,
    written: &[(&&VersionSummary, String, usize)],
) -> serde_json::Value {
    let versions: Vec<serde_json::Value> = written
        .iter()
        .map(|(s, name, bytes)| {
            json!({
                "version": s.version_num,
                "file": name,
                "is_latest": s.version_num == latest,
                "pushed_at": s.pushed_at,
                "pushed_by": s.pushed_by,
                "key_count": s.key_count,
                "key_names": s.key_names,
                "blob_size_bytes": s.blob_size_bytes,
                "exported_bytes": bytes,
            })
        })
        .collect();

    json!({
        "exported_at": chrono::Local::now().to_rfc3339(),
        "exported_by": format!("evnx {}", env!("CARGO_PKG_VERSION")),
        "server": server,
        "vault": {
            "id": vault_ref.id,
            "name": vault_ref.name,
            "environment": vault_ref.environment,
        },
        "format": format.as_deref().unwrap_or("env"),
        "file_extension": extension,
        "latest_version": latest,
        "versions_exported": versions.len(),
        "versions": versions,
    })
}

/// Convert already-parsed variables, for the byte-equality test below.
///
/// Exists so a test can assert that export's conversion step and `convert`'s
/// produce the same bytes without needing a server. The production path calls
/// the same two functions in the same order.
#[cfg(test)]
fn convert_for_test(vars: &IndexMap<String, String>, format: &str) -> Result<String> {
    let c = crate::formats::get_converter(format).map_err(|e| anyhow!("{e}"))?;
    c.convert(vars, &Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(name: &str, env: &str) -> VaultRef {
        VaultRef {
            id: "11111111-2222-3333-4444-555555555555".into(),
            name: name.into(),
            environment: env.into(),
        }
    }

    #[test]
    fn the_directory_name_flattens_the_slash_in_a_label() {
        assert_eq!(
            directory_name(&vault("api", "production")),
            "api-production"
        );
    }

    #[test]
    fn awkward_characters_cannot_escape_the_output_directory() {
        // A vault name is user-supplied and the server does not constrain it.
        // The result must always be one path component that stays put.
        for (name, env) in [
            ("../../etc", "passwd"),
            ("..", ""),
            (".", ""),
            ("....", ""),
            ("/", "/"),
            ("a/b", "c/d"),
        ] {
            let v = vault(name, env);
            let d = directory_name(&v);
            assert!(!d.contains('/'), "{name:?}/{env:?}: a slash survived: {d}");
            assert!(
                !d.contains('\\'),
                "{name:?}/{env:?}: a backslash survived: {d}"
            );
            assert!(!d.is_empty(), "{name:?}/{env:?}: empty component");
            assert!(
                d.chars().any(|c| c != '.'),
                "{name:?}/{env:?}: {d:?} is a relative path component, so \
                 `root.join(..)` would leave the output directory"
            );
            // The decisive check: joining it must stay underneath the root.
            let root = Path::new("/tmp/evnx-export");
            let joined = root.join(&d);
            assert!(
                joined.starts_with(root) && joined != root,
                "{name:?}/{env:?}: {} escaped {}",
                joined.display(),
                root.display()
            );
        }
    }

    /// The far-fetched one that the loop above would not have found on its own:
    /// a vault literally named `..` with no environment sanitises to `..`.
    #[test]
    fn a_vault_named_dot_dot_does_not_become_the_parent_directory() {
        let v = vault("..", "");
        assert_eq!(directory_name(&v), v.id);
    }

    #[test]
    fn a_vault_addressed_by_id_still_gets_a_directory() {
        // `name` and `environment` are empty when a vault-scoped token resolved
        // it, so `label()` returns the id and the sanitiser must keep it usable.
        let v = vault("", "");
        assert_eq!(directory_name(&v), v.id);
    }

    #[test]
    fn a_name_of_nothing_but_separators_falls_back_to_the_id() {
        let v = vault("///", "///");
        assert_eq!(
            directory_name(&v),
            v.id,
            "an empty component would export into the parent directory"
        );
    }

    #[test]
    fn a_missing_directory_is_not_a_refusal() {
        let tmp = tempfile::tempdir().unwrap();
        let fresh = tmp.path().join("not-there-yet");
        assert!(refuse_non_empty(&fresh, false).is_ok());
    }

    #[test]
    fn an_empty_directory_is_not_a_refusal() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(refuse_non_empty(tmp.path(), false).is_ok());
    }

    #[test]
    fn a_directory_with_a_previous_export_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("v1.env"), b"A=1").unwrap();
        let err = refuse_non_empty(tmp.path(), false).unwrap_err();
        assert!(
            err.to_string().contains("--force"),
            "the refusal must say how to proceed: {err}"
        );
        assert!(
            err.to_string().contains("before anything was decrypted"),
            "the refusal must say nothing was decrypted: {err}"
        );
    }

    #[test]
    fn force_overrides_the_refusal() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("v1.env"), b"A=1").unwrap();
        assert!(refuse_non_empty(tmp.path(), true).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn the_export_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("api-production");
        create_dir_private(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "an export directory holds plaintext secrets"
        );
    }

    /// ⚠️ **The test that must not be skipped.**
    ///
    /// Export reaches the 14 formats through the same `get_converter` that
    /// `convert` uses, and the whole point of this command is that the file you
    /// get out is the file you would have got any other way. If these two paths
    /// ever drift, an export is quietly wrong and nobody finds out until they
    /// have already left.
    #[test]
    fn export_and_convert_produce_identical_bytes() {
        let content = "# a comment\n\
                       DATABASE_URL=postgres://user:pass@localhost:5432/db\n\
                       API_KEY=\"sk-live-abc123\"\n\
                       EMPTY=\n\
                       SPACED=two words\n\
                       MULTILINE=\"line one\nline two\"\n";

        // The path `convert` takes: its own parser, then the converter.
        let via_convert = crate::core::parser::Parser::new(Default::default())
            .parse_content(content)
            .unwrap();

        // The path `export` takes: `run::parse_env` on decrypted bytes, then the
        // same converter.
        let via_export = super::super::run::parse_env(content.as_bytes()).unwrap();

        assert_eq!(
            via_convert, via_export,
            "the two parse paths disagree before a converter is even reached"
        );

        for format in [
            "json",
            "yaml",
            "shell",
            "aws-secrets",
            "gcp-secrets",
            "azure-keyvault",
            "github-actions",
            "docker-compose",
            "kubernetes",
            "terraform",
            "doppler",
            "heroku",
            "vercel",
            "railway",
        ] {
            let a = convert_for_test(&via_convert, format).unwrap();
            let b = convert_for_test(&via_export, format).unwrap();
            assert_eq!(a, b, "{format} differs between the two paths");
        }
    }

    /// A multiline value survives the round trip into a format that can hold it.
    ///
    /// Same class as D21: a second parser is how a multiline value gets lost.
    #[test]
    fn a_multiline_value_survives_the_conversion() {
        let vars = super::super::run::parse_env(b"KEY=\"line one\nline two\"\n").unwrap();
        let json = convert_for_test(&vars, "json").unwrap();
        assert!(
            json.contains("line one\\nline two"),
            "the newline was lost: {json}"
        );
    }

    #[test]
    fn a_converted_file_ends_with_a_newline() {
        assert_eq!(newline_terminated("{}".into()), "{}\n");
    }

    #[test]
    fn a_newline_is_not_doubled() {
        assert_eq!(newline_terminated("{}\n".into()), "{}\n");
    }

    /// An export's content equals `convert`'s, and ends with exactly one newline.
    ///
    /// ⚠️ Stated as two claims on purpose, because `convert` cannot be matched
    /// byte for byte without copying its bugs. It prints with
    /// `println!("{}", content)`, so a format whose converter already ends in a
    /// newline — `yaml` does — reaches stdout with **two**, and one of those is
    /// a trailing blank line nobody wants in a file. Both quirks are filed
    /// against `convert` rather than reproduced here.
    ///
    /// What must hold is that the *content* agrees. If these two ever diverge
    /// by more than trailing newlines, an export is quietly wrong and the
    /// person relying on it has already left.
    #[test]
    fn a_converted_export_agrees_with_convert_and_ends_in_one_newline() {
        let vars = super::super::run::parse_env(b"A=1\nB=2\n").unwrap();
        for format in [
            "json",
            "yaml",
            "shell",
            "terraform",
            "docker-compose",
            "kubernetes",
            "doppler",
            "vercel",
            "railway",
        ] {
            let from_converter = convert_for_test(&vars, format).unwrap();
            let exported = newline_terminated(from_converter.clone());

            assert_eq!(
                exported.trim_end_matches('\n'),
                from_converter.trim_end_matches('\n'),
                "{format}: the exported content differs from the converter's"
            );
            assert!(
                exported.ends_with('\n') && !exported.ends_with("\n\n"),
                "{format}: expected exactly one trailing newline, got {:?}",
                &exported[exported.len().saturating_sub(4)..]
            );
        }
    }

    #[test]
    fn the_manifest_names_which_version_was_current() {
        let v = vault("api", "production");
        let s1 = VersionSummary {
            version_num: 1,
            key_count: 2,
            key_names: vec!["A".into(), "B".into()],
            blob_size_bytes: 100,
            pushed_by: "user-1".into(),
            pushed_at: "2026-09-01T00:00:00Z".into(),
        };
        let s2 = VersionSummary {
            version_num: 2,
            key_count: 3,
            key_names: vec!["A".into(), "B".into(), "C".into()],
            blob_size_bytes: 150,
            pushed_by: "user-1".into(),
            pushed_at: "2026-09-02T00:00:00Z".into(),
        };
        let r1 = &s1;
        let r2 = &s2;
        let written = vec![
            (&r1, "v1.env".to_string(), 40),
            (&r2, "v2.env".to_string(), 60),
        ];

        let m = manifest(&v, "https://api.evnx.dev", 2, "env", &None, &written);

        assert_eq!(m["latest_version"], 2);
        assert_eq!(m["versions_exported"], 2);
        assert_eq!(m["versions"][0]["is_latest"], false);
        assert_eq!(m["versions"][1]["is_latest"], true);
        assert_eq!(m["versions"][1]["file"], "v2.env");
        assert_eq!(m["format"], "env");
        assert_eq!(m["vault"]["id"], v.id);
    }

    #[test]
    fn the_manifest_records_the_format_when_one_was_asked_for() {
        let v = vault("api", "production");
        let s = VersionSummary {
            version_num: 7,
            key_count: 1,
            key_names: vec!["A".into()],
            blob_size_bytes: 10,
            pushed_by: "u".into(),
            pushed_at: "2026-09-01T00:00:00Z".into(),
        };
        let r = &s;
        let written = vec![(&r, "v7.json".to_string(), 12)];
        let m = manifest(
            &v,
            "https://api.evnx.dev",
            7,
            "json",
            &Some("json".to_string()),
            &written,
        );
        assert_eq!(m["format"], "json");
        assert_eq!(m["file_extension"], "json");
    }

    /// The manifest must not become a second copy of the secrets.
    ///
    /// It carries key *names*, which the server already holds and the dashboard
    /// already shows. Values are what the `.env` files beside it are for.
    #[test]
    fn the_manifest_holds_no_values() {
        let v = vault("api", "production");
        let s = VersionSummary {
            version_num: 1,
            key_count: 1,
            key_names: vec!["DATABASE_URL".into()],
            blob_size_bytes: 10,
            pushed_by: "u".into(),
            pushed_at: "2026-09-01T00:00:00Z".into(),
        };
        let r = &s;
        let written = vec![(&r, "v1.env".to_string(), 12)];
        let m = manifest(&v, "https://api.evnx.dev", 1, "env", &None, &written);
        let text = serde_json::to_string(&m).unwrap();
        assert!(text.contains("DATABASE_URL"), "names are expected");
        assert!(
            !text.contains("postgres://"),
            "a value reached the manifest: {text}"
        );
    }
}
