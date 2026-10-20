//! github.rs — Migrate secrets to GitHub Actions
//!
//! Requires the `migrate` feature flag (pulls in `reqwest`, `indicatif`, etc.)
//!
//! # Bugs fixed vs. original migrate.rs
//!
//! - B1/B2/B3: GitHub API URLs had stray spaces (`"repos/  {}"`) — fixed.
//! - B4: `encrypt_for_github` was a base64-only placeholder. **Fixed in v0.5.0.**
//!
//! ⚠️ This header previously claimed B4 was already fixed — "This module provides
//! the correct libsodium sealed-box implementation using the `crypto_box` crate"
//! — and it was not. The real implementation sat commented out, `crypto_box` was
//! a declared dependency nothing called, and the live code path base64-encoded
//! the plaintext and sent that as `encrypted_value`. The note asking someone to
//! "add crypto_box to Cargo.toml to activate" was also stale: it was already
//! there.
//!
//! The lesson worth keeping: the two tests guarding this asserted `is_ok()` and
//! "not empty", both of which the placeholder satisfied. A ciphertext test has to
//! decrypt.

use crate::core::http;
use crate::utils::prompt;
use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use indexmap::IndexMap;

use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

use super::super::destination::{
    DestinationKind, MigrationDestination, MigrationOptions, MigrationResult, Out,
};

// ─── How evnx talks to the GitHub REST API (DEC-MG3) ─────────────────────────

/// The REST API version every request names.
///
/// ⚠️ Not the newer `2026-03-10`: GitHub Enterprise Server, which
/// `GITHUB_API_URL` reaches, may not know it, and none of its breaking changes
/// touch these three endpoints. `2022-11-28` is what GitHub assumes when no
/// version is sent, supported until 2028-03-10 — naming it means a change of
/// default cannot move evnx without a release.
const API_VERSION: &str = "2022-11-28";

/// Per request, and to connect. ⚠️ Each request used to build its own
/// `Client::new()`, which has no timeout: an unanswered connection hung the
/// upload for as long as the operating system allowed (I-112).
const TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Between two uploads. GitHub asks for at least a second between writes, and
/// counts a secret upload against a secondary limit of 80 a minute; at 100 ms
/// a repository with more than 80 secrets ran into it and the rest failed.
const PAUSE: Duration = Duration::from_secs(1);

/// GitHub's largest page of the secrets list.
const PER_PAGE: usize = 100;

/// A repository holds at most 100 secrets; past this many pages the server is
/// not GitHub, or not answering as GitHub does.
const MAX_PAGES: usize = 20;

/// The largest value GitHub stores in one secret.
const MAX_VALUE_BYTES: usize = 48 * 1024;

/// [`PAUSE`], unless a **debug** build is told otherwise by
/// `EVNX_GITHUB_PAUSE_MS` — so the tests do not wait a second per secret. A
/// release binary always waits (as `update` treats `EVNX_UPDATE_URL`, DEC-SF10).
fn pause() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("EVNX_GITHUB_PAUSE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return Duration::from_millis(ms);
    }
    PAUSE
}

/// What GitHub would refuse in `secrets`, one line each, checked before any
/// request (I-267). A refusal used to cost one failed upload per name, after
/// the others had gone.
///
/// The character rules — letters, digits and `_`, no leading digit — hold for
/// every name `migrate` hands over (`mod.rs` checks them for all
/// destinations). What is left is GitHub's own: the `GITHUB_` prefix is
/// reserved, in any case, and a value is at most 48 KB.
pub fn refused_by_github(secrets: &IndexMap<String, String>) -> Vec<String> {
    let mut refused = Vec::new();
    for (name, value) in secrets {
        if name.to_ascii_uppercase().starts_with("GITHUB_") {
            refused.push(format!("{name}: names starting with GITHUB_ are reserved"));
        } else if value.len() > MAX_VALUE_BYTES {
            refused.push(format!(
                "{name}: {} bytes; a GitHub secret holds at most 48 KB",
                value.len()
            ));
        }
    }
    refused
}

/// One repository's secrets endpoints, through one client.
struct Api<'a> {
    client: &'a Client,
    base: &'a str,
    repo: &'a str,
    token: &'a str,
}

impl Api<'_> {
    /// `…/repos/{repo}/actions/secrets{path}`, with the token, the media type
    /// GitHub documents, and the API version. The user agent is the shared
    /// client's (`evnx/<version>`).
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(
                method,
                format!("{}/repos/{}/actions/secrets{}", self.base, self.repo, path),
            )
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
    }

    /// Send, wording a request that never completed as `core::http` does.
    fn send(&self, request: RequestBuilder) -> Result<Response> {
        request.send().map_err(|e| {
            anyhow!(
                "could not reach the GitHub API: {}",
                http::transport_reason(&e, TIMEOUT)
            )
        })
    }
}

// ─── Destination struct ───────────────────────────────────────────────────────

/// Where the GitHub API lives when nothing says otherwise.
const DEFAULT_API_BASE: &str = "https://api.github.com";

pub struct GitHubDestination {
    /// `owner/repo`
    pub repository: String,
    /// GitHub Personal Access Token (PAT) with `secrets:write` scope.
    pub token: String,
    /// API root, without a trailing slash.
    ///
    /// ⚠️ Not only a test seam, though it is that too — this was hardcoded to
    /// `api.github.com` in three `format!` calls, which made the one destination
    /// that actually uploads the one destination untestable without a live token.
    ///
    /// It also makes **GitHub Enterprise Server** work, where the API lives at
    /// `https://github.example.com/api/v3`.
    api_base: String,
}

impl GitHubDestination {
    /// Construct from explicit values — used when CLI flags provide everything.
    ///
    /// The API base comes from `GITHUB_API_URL`, which GitHub Actions sets in
    /// every job (to `https://api.github.com` on github.com, and to the
    /// appropriate root on Enterprise), falling back to the public API.
    pub fn new(repository: String, token: String) -> Self {
        let api_base = std::env::var("GITHUB_API_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
        Self::with_api_base(repository, token, api_base)
    }

    /// Construct against a named API root. Used by the tests, and by anyone
    /// pointing evnx at an Enterprise instance explicitly.
    pub fn with_api_base(repository: String, token: String, api_base: String) -> Self {
        Self {
            repository,
            token,
            api_base: api_base.trim_end_matches('/').to_string(),
        }
    }
}

// ─── Trait implementation ─────────────────────────────────────────────────────

impl MigrationDestination for GitHubDestination {
    fn name(&self) -> &str {
        "GitHub Actions"
    }

    // The only destination that actually transfers anything: it PUTs each
    // secret to api.github.com. Every other one prints commands, which is why
    // `EmitsCommands` is the trait's default.
    fn kind(&self) -> DestinationKind {
        DestinationKind::Uploads
    }

    /// GitHub stores secret names in uppercase: `db_url` is `DB_URL` (I-104).
    fn names_ignore_case(&self) -> bool {
        true
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        // ── What GitHub would refuse, before anything else (I-267) ────────────
        let refused = refused_by_github(secrets);
        if !refused.is_empty() {
            anyhow::bail!(
                "GitHub would refuse {}:\n  {}\n\n\
                 Rename them in the .env, add a prefix with --add-prefix, or leave \
                 them out with --exclude. Nothing was uploaded.",
                if refused.len() == 1 {
                    "this secret"
                } else {
                    "these secrets"
                },
                refused.join("\n  ")
            );
        }

        opts.out.status(format!(
            "\n{} Migrating to GitHub Actions Secrets…",
            crate::utils::ui::glyph::INFO.cyan()
        ));

        // ── Ask first, or refuse (I-109) ──────────────────────────────────────
        // ⚠️ Before any request. Without a terminal the confirmation below
        // cannot be shown, and there was no `--yes`, so an upload could not run
        // in CI at all — it failed with dialoguer's "not a terminal" after
        // already listing the repository's secrets.
        if !opts.dry_run && !opts.yes {
            prompt::require_terminal_or(&format!(
                "Add --yes to upload without asking:\n  \
                 evnx migrate --to github-actions --repo {} --yes",
                self.repository
            ))?;
        }

        // ── One client for the whole run (DEC-MG3) ────────────────────────────
        let client =
            http::client(TIMEOUT, Some(CONNECT_TIMEOUT)).context("building the HTTP client")?;
        let api = Api {
            client: &client,
            base: &self.api_base,
            repo: &self.repository,
            token: &self.token,
        };

        // ── Fetch existing secrets (skip in dry-run to avoid network calls) ──
        let existing: HashSet<String> = if !opts.dry_run {
            fetch_existing_secrets(&api, opts.verbose)?
        } else {
            HashSet::new()
        };

        // ── Classify each secret ──────────────────────────────────────────────
        let mut to_upload: Vec<(String, String)> = Vec::new();
        let mut to_skip: Vec<String> = Vec::new();
        let mut conflicts: Vec<String> = Vec::new();
        let mut undecided: Vec<&str> = Vec::new();

        for (key, value) in secrets {
            // ⚠️ Uppercase on both sides: GitHub stores `db_url` as `DB_URL`, so
            // an exact comparison called an existing secret new and replaced it
            // under `--skip-existing` (I-266).
            if existing.contains(&key.to_ascii_uppercase()) {
                if opts.skip_existing {
                    to_skip.push(key.clone());
                } else if opts.yes && !opts.overwrite {
                    // `--yes` answers "Proceed?", never "replace this secret?".
                    undecided.push(key);
                } else if opts.overwrite
                    || prompt::yes_no()
                        .with_prompt(format!("Overwrite existing secret '{}'?", key))
                        .default(false)
                        .interact_opt()?
                        .unwrap_or(false)
                {
                    to_upload.push((key.clone(), value.clone()));
                    conflicts.push(key.clone());
                } else {
                    to_skip.push(key.clone());
                }
            } else {
                to_upload.push((key.clone(), value.clone()));
            }
        }

        if !undecided.is_empty() {
            anyhow::bail!(
                "{} already exist{} in {}: {}.\n\n\
                 --yes does not replace secrets. Add --overwrite to replace them \
                 or --skip-existing to leave them. Nothing was uploaded.",
                undecided.len(),
                if undecided.len() == 1 { "s" } else { "" },
                self.repository,
                undecided.join(", ")
            );
        }

        // ── Print plan ────────────────────────────────────────────────────────
        println!("\n{}", "Migration plan:".bold());
        println!("  • {} to upload", to_upload.len());
        if !to_skip.is_empty() {
            println!("  • {} to skip (already exist)", to_skip.len());
        }
        if !conflicts.is_empty() {
            println!("  • {} conflicts (will overwrite)", conflicts.len());
        }

        if opts.dry_run {
            println!("\n{} Dry-run — no changes made.", "·".cyan());
            return Ok(MigrationResult {
                uploaded: 0,
                // ⚠️ `to_upload` counts too. A dry run skips the "which secrets
                // already exist" request, so `existing` is empty and everything
                // lands in `to_upload` — leaving this at 0 and the summary
                // reading "0 secret(s) previewed" directly under a preview of N.
                // F12 fixed the wording for the eight command-emitting
                // destinations; this is the ninth.
                skipped: to_upload.len() + to_skip.len(),
                ..Default::default()
            });
        }

        // ── Confirm ───────────────────────────────────────────────────────────
        // Answered on Enter (N-09); Esc cancels like "no".
        if !opts.yes
            && !prompt::yes_no()
                .with_prompt("Proceed with migration?")
                .default(true)
                .interact_opt()?
                .unwrap_or(false)
        {
            println!("{} Cancelled. Nothing was uploaded.", "·".cyan());
            return Ok(MigrationResult::default());
        }

        // ── Upload with progress bar ──────────────────────────────────────────
        let pb = ProgressBar::new(to_upload.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );

        let mut result = MigrationResult {
            skipped: to_skip.len(),
            ..Default::default()
        };

        // One key for the whole run. It belongs to the repository, not to the
        // secret, so fetching it per secret only doubled the request count.
        let public_key = fetch_public_key(&api)?;

        for (i, (key, value)) in to_upload.iter().enumerate() {
            if i > 0 {
                std::thread::sleep(pause());
            }
            pb.set_message(key.clone());
            match upload_secret(&api, &public_key, key, value, opts.verbose) {
                Ok(()) => {
                    result.uploaded += 1;
                    pb.println(format!("  {} {}", "✓".green(), key));
                }
                Err(e) => {
                    result.failed += 1;
                    result.errors.push(format!("{}: {}", key, e));
                    pb.println(format!("  {} {} — {}", "✗".red(), key, e));
                }
            }
            pb.inc(1);
        }

        pb.finish_with_message("Done");
        Ok(result)
    }

    fn print_next_steps(&self, out: Out) {
        out.status(format!("\n{}", "Next steps:".bold()));
        out.status("  1. Update your GitHub Actions workflows to reference these secrets.");
        out.status("  2. Revoke / delete your local .env file or encrypt it with `evnx backup`.");
        out.status("  3. Regenerate .env.example if secret names changed.");
    }
}
// ─── GitHub REST API helpers ──────────────────────────────────────────────────

/// GET /repos/{owner}/{repo}/actions/secrets, every page, as uppercase names.
///
/// ⚠️ One request with no `per_page` read GitHub's default page of **30**
/// names, so a repository with more looked as if the rest did not exist, and
/// `--skip-existing` replaced them (I-266). Now `per_page=100` — GitHub's
/// largest, and a repository's limit — and every page until one comes back
/// short or `total_count` is reached.
fn fetch_existing_secrets(api: &Api, verbose: bool) -> Result<HashSet<String>> {
    if verbose {
        println!("  Fetching existing secrets from {}", api.repo);
    }

    #[derive(Deserialize)]
    struct SecretsPage {
        #[serde(default)]
        total_count: Option<usize>,
        secrets: Vec<SecretItem>,
    }
    #[derive(Deserialize)]
    struct SecretItem {
        name: String,
    }

    let mut names = HashSet::new();
    for page_number in 1..=MAX_PAGES {
        let resp = api.send(api.request(
            Method::GET,
            &format!("?per_page={PER_PAGE}&page={page_number}"),
        ))?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "GitHub API error {}: {}",
                resp.status(),
                resp.text().unwrap_or_default()
            ));
        }
        let page: SecretsPage = resp
            .json()
            .context("reading the list of secrets from GitHub")?;
        let short = page.secrets.len() < PER_PAGE;
        names.extend(
            page.secrets
                .into_iter()
                .map(|s| s.name.to_ascii_uppercase()),
        );
        if short || page.total_count.is_some_and(|total| names.len() >= total) {
            return Ok(names);
        }
    }
    Err(anyhow!(
        "GitHub listed more than {} secrets for {}, and kept going. Nothing was uploaded.",
        MAX_PAGES * PER_PAGE,
        api.repo
    ))
}

/// GET /repos/{owner}/{repo}/actions/secrets/public-key
fn fetch_public_key(api: &Api) -> Result<PublicKey> {
    let resp = api.send(api.request(Method::GET, "/public-key"))?;

    if !resp.status().is_success() {
        return Err(anyhow!("Failed to get public key: {}", resp.status()));
    }
    resp.json().context("Failed to parse public key response")
}

#[derive(Deserialize)]
struct PublicKey {
    key_id: String,
    key: String,
}

/// PUT /repos/{owner}/{repo}/actions/secrets/{secret_name}
fn upload_secret(api: &Api, pk: &PublicKey, key: &str, value: &str, verbose: bool) -> Result<()> {
    if verbose {
        println!("  Uploading '{}'", key);
    }

    // ⚠️ The key is fetched once by the caller and passed in. This used to call
    // fetch_public_key() itself, so uploading N secrets made 2N requests. The
    // pause between uploads is GitHub's ask for writes (see `PAUSE`).
    let encrypted = encrypt_for_github(&pk.key, value)?;

    #[derive(Serialize)]
    struct Payload {
        encrypted_value: String,
        key_id: String,
    }

    let resp = api.send(api.request(Method::PUT, &format!("/{key}")).json(&Payload {
        encrypted_value: encrypted,
        key_id: pk.key_id.clone(),
    }))?;

    if resp.status().is_success() || resp.status() == reqwest::StatusCode::CREATED {
        Ok(())
    } else {
        Err(anyhow!(
            "Upload failed {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ))
    }
}

// ─── Encryption ───────────────────────────────────────────────────────────────

/// Encrypt `value` with the repository's public key as a libsodium **sealed box**,
/// which is the only format the GitHub Actions secrets API accepts.
///
/// A sealed box is `ephemeral_public_key || XSalsa20-Poly1305(plaintext)`, and its
/// nonce is **derived** — `blake2b(ephemeral_pk || recipient_pk)`, truncated to 24
/// bytes — not random. `crypto_box`'s `PublicKey::seal` does that for us.
///
/// # ⚠️ This was a stub until v0.5.0, and the stub shipped
///
/// The body of this function was:
///
/// ```text
/// eprintln!("WARNING: Using placeholder encryption …");
/// Ok(general_purpose::STANDARD.encode(value.as_bytes()))
/// ```
///
/// It base64-encoded the plaintext and sent that as `encrypted_value`. The module
/// header claimed the opposite — "B4: … This module provides the correct libsodium
/// sealed-box implementation" — and `crypto_box` was a declared dependency that
/// nothing called. `evnx migrate --to github` therefore transmitted every secret in
/// a trivially reversible form, labelled as encrypted, and whatever GitHub stored
/// could not be decrypted back to the right value.
///
/// Two details the commented-out "fix" in that stub also got wrong, recorded so the
/// same shape is not reintroduced:
///
/// 1. It used `SalsaBox::generate_nonce(&mut OsRng)` — a **random** nonce. A sealed
///    box derives its nonce from the two public keys, so a random one cannot be
///    reconstructed by the recipient and never opens.
/// 2. `crypto_box`'s `seal` is behind the crate's `seal` feature, which was not
///    enabled, so the suggested code would not have compiled as written.
fn encrypt_for_github(public_key_b64: &str, value: &str) -> Result<String> {
    use base64::{engine::general_purpose, Engine as _};
    use crypto_box::{aead::OsRng, PublicKey};

    let pk_bytes = general_purpose::STANDARD
        .decode(public_key_b64)
        .context("Failed to base64-decode GitHub public key")?;

    let recipient = PublicKey::from_slice(&pk_bytes).map_err(|_| {
        anyhow!(
            "Invalid GitHub public key: expected 32 bytes, got {}",
            pk_bytes.len()
        )
    })?;

    let sealed = recipient
        .seal(&mut OsRng, value.as_bytes())
        .map_err(|e| anyhow!("Failed to seal secret for GitHub: {e:?}"))?;

    Ok(general_purpose::STANDARD.encode(&sealed))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose, Engine as _};
    use crypto_box::{aead::OsRng, SecretKey};

    /// The test that matters: seal with the public key, then **open it with the
    /// private key** and get the original back.
    ///
    /// ⚠️ Nothing weaker would have caught the stub. The two tests that existed
    /// before v0.5.0 asserted only `is_ok()` and "not empty" — both of which the
    /// base64-of-plaintext placeholder satisfied, which is how it survived to
    /// ship. An assertion about the *shape* of a ciphertext is not an assertion
    /// that it is one.
    #[test]
    fn sealed_secret_round_trips_through_the_private_key() {
        let sk = SecretKey::generate(&mut OsRng);
        let pk_b64 = general_purpose::STANDARD.encode(sk.public_key().as_bytes());

        let sealed_b64 = encrypt_for_github(&pk_b64, "sk_live_SUPER_SECRET").unwrap();
        let sealed = general_purpose::STANDARD.decode(&sealed_b64).unwrap();

        let opened = sk
            .unseal(&sealed)
            .expect("GitHub would fail to open this exactly as we just did");
        assert_eq!(opened, b"sk_live_SUPER_SECRET");
    }

    /// The regression test for the stub, stated as what it got wrong: the value
    /// GitHub receives must not decode back to the secret.
    #[test]
    fn sealed_secret_is_not_merely_base64_of_the_plaintext() {
        let sk = SecretKey::generate(&mut OsRng);
        let pk_b64 = general_purpose::STANDARD.encode(sk.public_key().as_bytes());

        let sealed_b64 = encrypt_for_github(&pk_b64, "sk_live_SUPER_SECRET").unwrap();
        let raw = general_purpose::STANDARD.decode(&sealed_b64).unwrap();

        assert_ne!(raw.as_slice(), b"sk_live_SUPER_SECRET");
        assert!(
            !String::from_utf8_lossy(&raw).contains("sk_live"),
            "the plaintext is recoverable from what we send to GitHub"
        );
    }

    /// `ephemeral_public_key (32) || ciphertext || Poly1305 tag (16)`.
    #[test]
    fn sealed_secret_has_the_libsodium_sealed_box_layout() {
        let sk = SecretKey::generate(&mut OsRng);
        let pk_b64 = general_purpose::STANDARD.encode(sk.public_key().as_bytes());

        let plaintext = "0123456789";
        let sealed = general_purpose::STANDARD
            .decode(encrypt_for_github(&pk_b64, plaintext).unwrap())
            .unwrap();

        assert_eq!(sealed.len(), 32 + plaintext.len() + 16);
    }

    /// Two seals of the same value differ — the ephemeral key is fresh each time.
    #[test]
    fn sealing_is_not_deterministic() {
        let sk = SecretKey::generate(&mut OsRng);
        let pk_b64 = general_purpose::STANDARD.encode(sk.public_key().as_bytes());
        let a = encrypt_for_github(&pk_b64, "same").unwrap();
        let b = encrypt_for_github(&pk_b64, "same").unwrap();
        assert_ne!(a, b);
    }

    // ─── HTTP paths, against a local mock ───────────────────────────────────
    //
    // ⚠️ None of this was reachable before v0.5.0. The API root was hardcoded in
    // three format! calls, so the only destination that actually uploads was the
    // only one that could not be tested without a live token and a real repo —
    // which is exactly how the placeholder encryption survived to ship.

    fn test_client() -> Client {
        http::client(TIMEOUT, Some(CONNECT_TIMEOUT)).unwrap()
    }

    fn api<'a>(client: &'a Client, base: &'a str, token: &'a str) -> Api<'a> {
        Api {
            client,
            base,
            repo: "owner/repo",
            token,
        }
    }

    fn key_response(sk: &SecretKey) -> String {
        format!(
            r#"{{"key_id":"kid-1","key":"{}"}}"#,
            general_purpose::STANDARD.encode(sk.public_key().as_bytes())
        )
    }

    #[test]
    fn fetch_public_key_hits_the_documented_path() {
        let mut server = mockito::Server::new();
        let sk = SecretKey::generate(&mut OsRng);
        let m = server
            .mock("GET", "/repos/owner/repo/actions/secrets/public-key")
            .match_header("authorization", "Bearer tok")
            .with_status(200)
            .with_body(key_response(&sk))
            .create();

        let (client, url) = (test_client(), server.url());
        let pk = fetch_public_key(&api(&client, &url, "tok")).unwrap();
        assert_eq!(pk.key_id, "kid-1");
        m.assert();
    }

    #[test]
    fn fetch_existing_secrets_lists_names() {
        let mut server = mockito::Server::new();
        let m = server
            .mock("GET", "/repos/owner/repo/actions/secrets")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("per_page".into(), "100".into()),
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
            ]))
            .with_status(200)
            .with_body(r#"{"secrets":[{"name":"API_KEY"},{"name":"db_url"}]}"#)
            .create();

        let (client, url) = (test_client(), server.url());
        let names = fetch_existing_secrets(&api(&client, &url, "tok"), false).unwrap();
        assert_eq!(
            names,
            HashSet::from(["API_KEY".to_string(), "DB_URL".to_string()])
        );
        m.assert();
    }

    /// The upload path end to end: what lands on the wire is a sealed box the
    /// repository's private key opens — not the plaintext, and not base64 of it.
    #[test]
    fn upload_sends_a_sealed_box_the_private_key_can_open() {
        use std::sync::{Arc, Mutex};

        let mut server = mockito::Server::new();
        let sk = SecretKey::generate(&mut OsRng);
        let pk = fetch_public_key_from_json(&key_response(&sk));

        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured = Arc::clone(&seen);

        let m = server
            .mock("PUT", "/repos/owner/repo/actions/secrets/API_KEY")
            .match_header("authorization", "Bearer tok")
            .with_status(201)
            .match_request(move |req| {
                let body: serde_json::Value = serde_json::from_slice(req.body().unwrap()).unwrap();
                assert_eq!(body["key_id"], "kid-1");
                *captured.lock().unwrap() =
                    Some(body["encrypted_value"].as_str().unwrap().to_string());
                true
            })
            .create();

        let (client, url) = (test_client(), server.url());
        upload_secret(
            &api(&client, &url, "tok"),
            &pk,
            "API_KEY",
            "sk_live_SUPER_SECRET",
            false,
        )
        .unwrap();
        m.assert();

        let sent = seen.lock().unwrap().clone().expect("no body captured");
        let sealed = general_purpose::STANDARD.decode(&sent).unwrap();
        assert_eq!(sk.unseal(&sealed).unwrap(), b"sk_live_SUPER_SECRET");
    }

    #[test]
    fn an_api_error_is_reported_with_its_status() {
        let mut server = mockito::Server::new();
        server
            .mock("GET", "/repos/owner/repo/actions/secrets")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"message":"Bad credentials"}"#)
            .create();

        let (client, url) = (test_client(), server.url());
        let err = fetch_existing_secrets(&api(&client, &url, "bad"), false).unwrap_err();
        assert!(err.to_string().contains("401"), "{err}");
    }

    /// I-267: GitHub's own rules, checked before any request.
    #[test]
    fn t_i267_reserved_names_and_oversized_values_are_named() {
        let mut s = IndexMap::new();
        s.insert("github_token_copy".to_string(), "x".to_string());
        s.insert("BIG".to_string(), "x".repeat(MAX_VALUE_BYTES + 1));
        s.insert("OK".to_string(), "x".repeat(MAX_VALUE_BYTES));
        let refused = refused_by_github(&s);
        assert_eq!(refused.len(), 2, "{refused:?}");
        assert!(refused[0].starts_with("github_token_copy:"), "{refused:?}");
        assert!(refused[1].starts_with("BIG:"), "{refused:?}");
    }

    /// GITHUB_API_URL is what makes evnx work against Enterprise Server, and what
    /// GitHub Actions sets in every job.
    #[test]
    fn a_trailing_slash_on_the_api_base_does_not_double_up() {
        let d = GitHubDestination::with_api_base(
            "o/r".into(),
            "t".into(),
            "https://github.example.com/api/v3/".into(),
        );
        assert_eq!(d.api_base, "https://github.example.com/api/v3");
    }

    fn fetch_public_key_from_json(json: &str) -> PublicKey {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_public_key_that_is_not_base64_is_refused() {
        assert!(encrypt_for_github("!!!not_base64!!!", "value").is_err());
    }

    /// A 32-byte key is required; anything else is refused rather than padded.
    #[test]
    fn a_public_key_of_the_wrong_length_is_refused() {
        let short = general_purpose::STANDARD.encode([0u8; 16]);
        let err = encrypt_for_github(&short, "value").unwrap_err();
        assert!(err.to_string().contains("32 bytes"), "{err}");
    }
}
