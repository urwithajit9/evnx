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

/// [`check`] for an optional flag.
pub fn check_opt(flag: &str, value: Option<&str>, kind: Name) -> Result<()> {
    value.map_or(Ok(()), |v| check(flag, v, kind))
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
    fn t_i107_repo_needs_owner_and_name() {
        assert!(Name::GitHubRepo.accepts("octo-org/hello.world_1"));
        for bad in ["notarepo", "/r", "o/", "o/r/x", "-o/r", "o/..", "o r/x"] {
            assert!(!Name::GitHubRepo.accepts(bad), "{bad}");
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
