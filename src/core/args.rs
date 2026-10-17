//! Names a user passes on the command line, checked before use (A-17, U6).
//!
//! `migrate` pasted `--heroku-app`, `--vault-name`, `--secret-name` and the rest
//! into the commands it prints, unvalidated and unquoted, so
//! `--heroku-app 'x;echo PWNED'` printed a second command (I-102). `--repo` was
//! not checked either: `notarepo` was accepted (I-107).
//!
//! A name that cannot be valid for its destination is refused, naming the flag
//! and the rule, before anything is printed or sent; `main` reports it as exit
//! 2. Names that pass are still printed through `core::shell::word`.
//!
//! ⚠️ Each rule is the destination's own, not a stricter invention: refusing a
//! name the platform accepts would break a working migration. Where the rule is
//! not published, [`Name::Slug`] allows the characters any platform's
//! identifiers use and refuses only whitespace and shell syntax.
//!
//! Also the one rule for a value chosen from a fixed list — a format, a level,
//! an issue type: [`one_of`].

use anyhow::{bail, Result};

/// The kinds of name evnx checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Name {
    /// `owner/repo` on GitHub.
    GitHubRepo,
    /// A Heroku app name.
    HerokuApp,
    /// An Azure Key Vault name.
    AzureVault,
    /// An AWS Secrets Manager secret name.
    AwsSecretName,
    /// A project, config, environment or profile identifier.
    Slug,
    /// An evnx organisation slug (the server's rule, migration 010).
    OrgSlug,
}

impl Name {
    fn rule(self) -> &'static str {
        match self {
            Name::GitHubRepo => {
                "`owner/repo`: an owner of letters, digits and dashes, then a repository \
                 of letters, digits, `.`, `_` and `-`"
            }
            Name::HerokuApp => {
                "3–30 lowercase letters, digits and dashes, starting with a letter and \
                 not ending with a dash"
            }
            Name::AzureVault => {
                "3–24 letters, digits and dashes, starting with a letter, ending with a \
                 letter or digit, no `--`"
            }
            Name::AwsSecretName => "1–512 characters from letters, digits and `/_+=.@-`",
            Name::Slug => "letters, digits and `._-@+=:/`, no spaces or shell characters",
            Name::OrgSlug => {
                "1–63 lowercase letters, digits and hyphens, not starting or ending with a hyphen"
            }
        }
    }

    fn accepts(self, s: &str) -> bool {
        let n = s.chars().count();
        match self {
            Name::GitHubRepo => {
                let Some((owner, repo)) = s.split_once('/') else {
                    return false;
                };
                (1..=39).contains(&owner.len())
                    && !owner.starts_with('-')
                    && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                    && (1..=100).contains(&repo.len())
                    && repo != "."
                    && repo != ".."
                    && repo
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            }
            Name::HerokuApp => {
                (3..=30).contains(&n)
                    && s.starts_with(|c: char| c.is_ascii_lowercase())
                    && !s.ends_with('-')
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            }
            Name::AzureVault => {
                (3..=24).contains(&n)
                    && s.starts_with(|c: char| c.is_ascii_alphabetic())
                    && s.ends_with(|c: char| c.is_ascii_alphanumeric())
                    && !s.contains("--")
                    && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            }
            Name::AwsSecretName => {
                (1..=512).contains(&n)
                    && s.chars().all(|c| {
                        c.is_ascii_alphanumeric()
                            || matches!(c, '/' | '_' | '+' | '=' | '.' | '@' | '-')
                    })
            }
            Name::OrgSlug => {
                (1..=63).contains(&n)
                    && !s.starts_with('-')
                    && !s.ends_with('-')
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            }
            Name::Slug => {
                (1..=256).contains(&n)
                    && s.chars().all(|c| {
                        c.is_ascii_alphanumeric()
                            || matches!(c, '.' | '_' | '-' | '@' | '+' | '=' | ':' | '/')
                    })
            }
        }
    }
}

/// Refuse `value` for `flag` unless it is a valid `kind` name.
pub fn check(flag: &str, value: &str, kind: Name) -> Result<()> {
    if kind.accepts(value) {
        return Ok(());
    }
    bail!(
        "{flag} {value:?} is not a valid name here.\n\n\
         Expected {}. Nothing was printed or sent.",
        kind.rule()
    )
}

/// Refuse `value` for `flag` unless it looks like an email address: one `@`
/// with something on both sides, no whitespace (I-216). ⚠️ No stricter than the
/// server, which asks only for an `@` — `bob@localhost` is accepted there, so it
/// is here.
pub fn check_email(flag: &str, value: &str) -> Result<()> {
    let v = value.trim();
    let ok = !v.chars().any(char::is_whitespace)
        && v.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        });
    if ok {
        return Ok(());
    }
    bail!("{flag} {value:?} is not an email address. Nothing was sent.")
}

/// [`check`] for an optional flag.
pub fn check_opt(flag: &str, value: Option<&str>, kind: Name) -> Result<()> {
    value.map_or(Ok(()), |v| check(flag, v, kind))
}

/// A value from a fixed list — a format, a level, an issue type — or an error
/// naming every accepted one.
///
/// `accepted` pairs each spelling with what it means, so aliases (`text` for
/// `pretty`) cost nothing; `shown` is what the error lists. Matching ignores
/// surrounding spaces and case, and the error repeats the value as compared:
///
/// ```text
/// unknown format 'xml' — expected one of: pretty, json, github-actions
/// ```
///
/// ⚠️ One helper because the rule was written four times (validate, scan twice,
/// doctor), and validate's copy matched case-sensitively, so `--format JSON`
/// worked on scan and was refused by validate. A list check that accepts a typo
/// silently is worse than none: `validate --ignore placeholder_vlaue` suppressed
/// nothing and said nothing (I-038).
pub fn one_of<T: Copy>(
    what: &str,
    value: &str,
    accepted: &[(&str, T)],
    shown: &[&str],
) -> Result<T> {
    let wanted = value.trim().to_lowercase();
    match accepted.iter().find(|(name, _)| *name == wanted) {
        Some((_, meant)) => Ok(*meant),
        None => bail!(
            "unknown {what} '{wanted}' — expected one of: {}",
            shown.join(", ")
        ),
    }
}

/// The longest span, in days, any evnx day count may name: ten years
/// (DEC-AU2, DEC-CL7). The server enforces the same cap on token lifetimes.
pub const MAX_DAYS: i64 = 3650;

/// A day count from the command line: a whole number from 1 to [`MAX_DAYS`].
///
/// For clap's `value_parser`, so every flag that takes days gives the same one
/// message. ⚠️ `--expires-in-days 0` and `=-5` used to mint a token that was
/// already expired, and `-5` drew clap's "unexpected argument" tip, which
/// suggests the value might be valid (I-205, I-210).
pub fn parse_days(raw: &str) -> Result<i64, String> {
    match raw.trim().parse::<i64>() {
        Ok(n) if (1..=MAX_DAYS).contains(&n) => Ok(n),
        _ => Err(format!(
            "{raw:?} is not a number of days: use a whole number from 1 to {MAX_DAYS}"
        )),
    }
}

/// `now` minus `days`, or `None` if `days` is outside 1–[`MAX_DAYS`].
///
/// ⚠️ Checked arithmetic. `chrono::Duration::days` **panics** out of range, and
/// `cloud audit --last 9223372036854775807d` died with exit 101 on exactly that
/// (I-231).
pub fn days_before(
    now: chrono::DateTime<chrono::Utc>,
    days: i64,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if !(1..=MAX_DAYS).contains(&days) {
        return None;
    }
    chrono::Duration::try_days(days).and_then(|d| now.checked_sub_signed(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_i102_shell_syntax_never_passes() {
        for bad in [
            "x;y", "a b", "$(id)", "`id`", "a|b", "a&b", "a'b", "a\"b", "a\nb", "",
        ] {
            for kind in [
                Name::GitHubRepo,
                Name::HerokuApp,
                Name::AzureVault,
                Name::AwsSecretName,
                Name::Slug,
            ] {
                assert!(!kind.accepts(bad), "{kind:?} accepted {bad:?}");
            }
        }
    }

    #[test]
    fn t_i038_one_of_matches_any_case_and_lists_every_name() {
        let names = [("pretty", 1), ("text", 1), ("json", 2)];
        assert_eq!(
            one_of("format", " JSON ", &names, &["pretty", "json"]).unwrap(),
            2
        );
        assert_eq!(
            one_of("format", "text", &names, &["pretty", "json"]).unwrap(),
            1
        );
        let err = one_of("format", "XML", &names, &["pretty", "json"])
            .unwrap_err()
            .to_string();
        assert_eq!(err, "unknown format 'xml' — expected one of: pretty, json");
    }

    #[test]
    fn t_i205_days_are_1_to_3650() {
        assert_eq!(parse_days("1"), Ok(1));
        assert_eq!(parse_days(" 3650 "), Ok(3650));
        for bad in ["0", "-5", "3651", "9223372036854775807", "abc", "", "1.5"] {
            let err = parse_days(bad).unwrap_err();
            assert!(err.contains("1 to 3650"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn t_i231_days_before_never_panics() {
        let now = chrono::Utc::now();
        assert!(days_before(now, 7).unwrap() < now);
        for bad in [0, -1, 3651, i64::MAX, i64::MIN] {
            assert_eq!(days_before(now, bad), None, "{bad}");
        }
    }

    #[test]
    fn t_i107_repo_needs_owner_and_name() {
        assert!(Name::GitHubRepo.accepts("octo-org/hello.world_1"));
        for bad in ["notarepo", "/r", "o/", "o/r/x", "-o/r", "o/..", "o r/x"] {
            assert!(!Name::GitHubRepo.accepts(bad), "{bad}");
        }
    }

    #[test]
    fn t_i215_org_slug_is_the_servers_rule() {
        for ok in ["acme", "acme-2", "a"] {
            assert!(Name::OrgSlug.accepts(ok), "{ok}");
        }
        for bad in ["Bad Slug!", "-acme", "acme-", "ACME", "", &"a".repeat(64)] {
            assert!(!Name::OrgSlug.accepts(bad), "{bad}");
        }
    }

    #[test]
    fn t_i216_email_needs_an_at_with_text_both_sides() {
        // ⚠️ No stricter than the server, which asks only for an `@`.
        for ok in ["bob@example.com", "bob@localhost", " bob@example.com "] {
            assert!(check_email("<email>", ok).is_ok(), "{ok}");
        }
        for bad in [
            "not-an-email",
            "@example.com",
            "bob@",
            "b ob@x.io",
            "a@b@c.io",
            "",
        ] {
            assert!(check_email("<email>", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn platform_rules_hold_at_the_edges() {
        assert!(Name::HerokuApp.accepts("abc"));
        assert!(!Name::HerokuApp.accepts("ab"));
        assert!(!Name::HerokuApp.accepts("My-App"));
        assert!(!Name::HerokuApp.accepts("app-"));
        assert!(Name::AzureVault.accepts("Prod-Vault-7"));
        assert!(!Name::AzureVault.accepts("a--b"));
        assert!(!Name::AzureVault.accepts("1vault"));
        assert!(!Name::AzureVault.accepts(&"a".repeat(25)));
        assert!(Name::AwsSecretName.accepts("prod/my_app+v1=x.y@z-1"));
        assert!(Name::Slug.accepts("prj_AbC123"));
    }
}
