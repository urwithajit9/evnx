//! "Rotate at the source": what to replace after someone loses access to a vault.
//!
//! Revoking stops a person reading what is pushed from now on. It cannot recall
//! what they could already read, so every credential they could see should be
//! treated as theirs and replaced where it was issued. This module turns the
//! **names** of the variables they could read into that list, grouped by who
//! issued each credential, with the place to replace it when evnx knows one.
//!
//! ⚠️ Names only. Nothing here receives a value: the caller passes names it has
//! already read (`sync::read_key_names`, or the server's recorded `key_names`).
//!
//! Who issued a name comes from tables evnx already ships, not a new one:
//!
//! 1. `schema.json`'s `scan` section names the rule that reports a variable
//!    (`STRIPE_SECRET_KEY` → `stripe-live-secret-key`), and that rule in
//!    `rules.toml` names its `provider` and `url`;
//! 2. a rule's `names` patterns (Clerk and UploadThing reuse Stripe's prefix, so
//!    their rules are tied to the variable name);
//! 3. the `add service` catalogue (`schema.json` `services`), whose display name
//!    is the provider; its `url` is the one the rules give that provider, when
//!    they agree on one.
//!
//! A name none of these place is still listed, as "provider not known".

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::core::secretscan::rules::{builtin_rules, public_prefix};
use crate::schema::component::{Kind, declaring};
use crate::schema::loader::{ScanView, scan_view};

/// Where a set of names came from, which decides what the checklist can claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameSource {
    /// Read from each version after decrypting it during the re-key.
    Decrypted,
    /// The `key_names` the server recorded at each push (`--no-rekey` opens
    /// nothing, so they are not checked against the encrypted contents).
    ServerRecorded,
}

/// The names one vault's versions hold, as one revoke collected them.
#[derive(Debug, Clone)]
pub(crate) struct VaultNames {
    /// `name/environment`.
    pub label: String,
    pub source: NameSource,
    pub names: BTreeSet<String>,
    /// How many versions the names come from.
    pub versions: usize,
    /// Versions whose contents could not be read as `.env` text for names.
    pub unreadable: Vec<i32>,
}

impl VaultNames {
    pub(crate) fn new(label: &str, source: NameSource) -> Self {
        VaultNames {
            label: label.to_string(),
            source,
            names: BTreeSet::new(),
            versions: 0,
            unreadable: Vec::new(),
        }
    }
}

/// Who issued a credential, and where to replace it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Issuer {
    pub provider: Option<String>,
    pub url: Option<String>,
}

/// One line of the checklist, as `--json` prints it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RotateEntry {
    pub name: String,
    pub provider: Option<String>,
    pub url: Option<String>,
    /// The vaults (`name/environment`) the name was read from.
    pub vaults: Vec<String>,
}

/// The whole checklist: credentials to replace, and the other names.
#[derive(Debug, Clone, Default)]
pub(crate) struct Checklist {
    /// Names that look like credentials, sorted by provider then name.
    pub rotate: Vec<RotateEntry>,
    /// Names that do not look like credentials by name (`PORT`, `NODE_ENV`).
    pub other: Vec<String>,
    /// One line per vault saying what the names cover.
    pub notes: Vec<String>,
}

/// Who issued the credential a variable called `name` holds, if evnx can say.
pub(crate) fn issuer(name: &str) -> Issuer {
    let rules = builtin_rules();
    let by_id = |id: &str| rules.iter().find(|r| r.id.as_deref() == Some(id));

    // 1. The catalogue's scan view names the rule that reports this variable.
    if let Some(ScanView::Rule(id)) = scan_view(name) {
        // The AWS secret key is a check, not a rule (`rules::CHECK_IDS`): its
        // issuer is the one the AWS access-key rule names.
        let rule = match by_id(id) {
            Some(r) => Some(r),
            None if id == "aws-secret-access-key" => by_id("aws-access-key"),
            None => None,
        };
        if let Some(r) = rule {
            let provider = r.provider.clone().filter(|p| p != "Generic");
            return Issuer {
                url: provider.as_ref().and(r.url.clone()),
                provider,
            };
        }
    }

    // 2. A rule tied to the variable's name.
    for r in &rules {
        let Some(patterns) = &r.names else { continue };
        let hit = patterns
            .iter()
            .filter_map(|p| regex::Regex::new(p).ok())
            .any(|re| re.is_match(name));
        if hit && r.provider.as_deref().is_some_and(|p| p != "Generic") {
            return Issuer {
                provider: r.provider.clone(),
                url: r.url.clone(),
            };
        }
    }

    // 3. The service catalogue. A framework (`SECRET_KEY` under Django) is the
    // app's own secret, not an issuer, so only services and infrastructure.
    let component = declaring(name)
        .ok()
        .flatten()
        .filter(|c| matches!(c.kind, Kind::Service | Kind::Infrastructure));
    if let Some(c) = component {
        let urls: BTreeSet<&str> = rules
            .iter()
            .filter(|r| {
                r.provider
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case(&c.display_name))
            })
            .filter_map(|r| r.url.as_deref())
            .collect();
        let url = match urls.len() {
            1 => urls.into_iter().next().map(str::to_string),
            _ => None,
        };
        return Issuer {
            provider: Some(c.display_name),
            url,
        };
    }

    Issuer {
        provider: None,
        url: None,
    }
}

/// Whether a variable called `name` holds a credential, by its name: a name
/// the scan treats as secret, or one a provider rule reports. A name behind a
/// browser-exposed prefix (`NEXT_PUBLIC_`) is published by design.
pub(crate) fn is_credential_name(name: &str) -> bool {
    if public_prefix(name).is_some() {
        return false;
    }
    match scan_view(name) {
        Some(ScanView::Public) => false,
        Some(ScanView::Rule(_)) => true,
        _ => crate::core::classify::is_sensitive_key(name),
    }
}

/// Build the checklist from the names each revoked vault held.
pub(crate) fn build(vaults: &[VaultNames]) -> Checklist {
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for v in vaults {
        for n in &v.names {
            seen.entry(n.clone()).or_default().push(v.label.clone());
        }
    }

    let mut rotate = Vec::new();
    let mut other = Vec::new();
    for (name, labels) in seen {
        if !is_credential_name(&name) {
            other.push(name);
            continue;
        }
        let who = issuer(&name);
        rotate.push(RotateEntry {
            name,
            provider: who.provider,
            url: who.url,
            vaults: labels,
        });
    }
    // Known providers first, alphabetically; "not known" last.
    rotate.sort_by(|a, b| {
        (a.provider.is_none(), &a.provider, &a.url, &a.name).cmp(&(
            b.provider.is_none(),
            &b.provider,
            &b.url,
            &b.name,
        ))
    });

    let notes = vaults.iter().map(note).collect();
    Checklist {
        rotate,
        other,
        notes,
    }
}

/// What the names for one vault cover, said plainly.
fn note(v: &VaultNames) -> String {
    let mut s = match v.source {
        NameSource::Decrypted => format!(
            "{}: names read from its {} version(s). A version deleted earlier may have held more.",
            v.label, v.versions
        ),
        NameSource::ServerRecorded => format!(
            "{}: names as the server recorded them at each push ({} version(s)); \
             --no-rekey opens nothing, so they were not checked against the contents. \
             A version deleted earlier may have held more.",
            v.label, v.versions
        ),
    };
    if !v.unreadable.is_empty() {
        let list: Vec<String> = v.unreadable.iter().map(|n| n.to_string()).collect();
        s.push_str(&format!(
            " Version(s) {} could not be read for names; check them with \
             `evnx cloud pull --vault {} --version <N>`.",
            list.join(", "),
            v.label
        ));
    }
    s
}

/// Print the checklist for a person: names and places, never a value.
///
/// `several` adds the vaults each name came from.
pub(crate) fn print(list: &Checklist, several: bool) {
    use colored::Colorize;

    println!();
    println!("  {}", "Rotate at the source".bold());
    if list.rotate.is_empty() {
        println!("  None of the names they could read look like credentials.");
    } else {
        println!("  They could read these credentials, and revoking cannot recall them.");
        println!("  Replace each one where it was issued:");
        let mut last: Option<(&Option<String>, &Option<String>)> = None;
        for e in &list.rotate {
            if last != Some((&e.provider, &e.url)) {
                let head = match (&e.provider, &e.url) {
                    (Some(p), Some(u)) => format!("{p}  {u}"),
                    (Some(p), None) => format!("{p}  (where it was issued)"),
                    (None, _) => "Provider not known  (where it was issued)".to_string(),
                };
                println!("    {}", head.cyan());
                last = Some((&e.provider, &e.url));
            }
            if several {
                println!(
                    "      {}  {}",
                    e.name,
                    format!("({})", e.vaults.join(", ")).dimmed()
                );
            } else {
                println!("      {}", e.name);
            }
        }
    }
    if !list.other.is_empty() {
        println!(
            "  {} {}",
            "Also readable, not credentials by name:".dimmed(),
            list.other.join(", ")
        );
    }
    for n in &list.notes {
        println!("  {}", n.dimmed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_are_placed_with_their_provider_and_url() {
        let s = issuer("STRIPE_SECRET_KEY");
        assert_eq!(s.provider.as_deref(), Some("Stripe"));
        assert_eq!(
            s.url.as_deref(),
            Some("https://dashboard.stripe.com/apikeys")
        );

        let a = issuer("AWS_SECRET_ACCESS_KEY");
        assert_eq!(a.provider.as_deref(), Some("AWS"));
        assert_eq!(a.url.as_deref(), Some("https://console.aws.amazon.com/iam"));

        // A connection string: the place to rotate is the database itself.
        let d = issuer("DATABASE_URL");
        assert_eq!(d.provider, None);
        assert_eq!(d.url, None);

        // Tied to the name by the rule's `names`.
        assert_eq!(
            issuer("CLERK_SECRET_KEY").provider.as_deref(),
            Some("Clerk")
        );
    }

    #[test]
    fn credentials_and_other_names_are_told_apart() {
        for n in [
            "STRIPE_SECRET_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "DATABASE_URL",
            "JWT_SECRET",
        ] {
            assert!(is_credential_name(n), "{n}");
        }
        for n in ["PORT", "NODE_ENV", "NEXT_PUBLIC_APP_URL"] {
            assert!(!is_credential_name(n), "{n}");
        }
    }

    #[test]
    fn the_checklist_groups_by_provider_and_puts_unknown_last() {
        let mut a = VaultNames::new("app/production", NameSource::Decrypted);
        a.versions = 2;
        for n in [
            "STRIPE_SECRET_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "DATABASE_URL",
            "PORT",
        ] {
            a.names.insert(n.into());
        }
        let mut b = VaultNames::new("api/staging", NameSource::Decrypted);
        b.versions = 1;
        b.names.insert("STRIPE_SECRET_KEY".into());
        let list = build(&[a, b]);
        let names: Vec<&str> = list.rotate.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["AWS_SECRET_ACCESS_KEY", "STRIPE_SECRET_KEY", "DATABASE_URL"]
        );
        assert_eq!(
            list.rotate[1].vaults,
            ["app/production".to_string(), "api/staging".to_string()]
        );
        assert_eq!(list.other, ["PORT"]);
        assert!(list.notes[0].contains("2 version(s)"), "{:?}", list.notes);
        assert!(
            list.notes[0].contains("deleted earlier"),
            "{:?}",
            list.notes
        );
    }

    #[test]
    fn server_recorded_names_say_they_were_not_checked() {
        let mut v = VaultNames::new("app/production", NameSource::ServerRecorded);
        v.unreadable.push(3);
        let n = note(&v);
        assert!(n.contains("not checked against the contents"), "{n}");
        assert!(n.contains("Version(s) 3 could not be read"), "{n}");
    }
}
