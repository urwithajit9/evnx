//! Which variable names each output can hold (D12, 0.13.0).
//!
//! Since 0.13.0 the parser reads the names Docker Compose reads: `my.key`,
//! `my-key`, `1FOO`, `a[0]`, Unicode letters ([`crate::core::parser::is_env_key`]).
//! A process receives them; a shell, a GitHub secret or a Vercel project does
//! not. So the rule lives per target, here, and every `convert` format and
//! `migrate` destination checks it in one place: [`ConvertOptions::prepare`],
//! against the name the target will actually get (after `--prefix`,
//! `--transform` and the format's own renaming).
//!
//! ⚠️ **A table, not code in each format.** The roadmap (v0.15.0, RM-64
//! `--check`) asks for per-target name limits as catalogue data; writing D12's
//! refusals as rows from the start means that release adds columns rather than
//! moving checks out of fifteen converters.
//!
//! ⚠️ **Every rule accepts every name evnx read before 0.13.0**
//! (`[A-Za-z_][A-Za-z0-9_]*`, after each target's own renaming). These rows
//! refuse only what D12 newly lets in; tightening a target (Doppler wants
//! uppercase) would be a separate, announced change.
//!
//! [`ConvertOptions::prepare`]: crate::core::converter::ConvertOptions::prepare

/// What a target accepts as a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRule {
    /// Anything the parser accepts: JSON and YAML keys, Docker Compose
    /// `environment:` (measured, Compose v5.7.0), a JSON secret blob.
    Any,
    /// `[A-Za-z_][A-Za-z0-9_]*`: a name a shell can `export`, and the rule the
    /// providers below document for their own names.
    Shell,
    /// A Kubernetes Secret data key: `[-._a-zA-Z0-9]+`.
    Kubernetes,
    /// A Terraform variable name: a letter or `_`, then letters, digits, `_`
    /// and `-`.
    Terraform,
    /// A GCP Secret Manager secret ID: `[A-Za-z0-9_-]+`.
    GcpSecretId,
    /// An Azure Key Vault secret name: `[0-9A-Za-z-]+`. `_` is accepted
    /// because both Azure outputs write it as `-` (`convert` renames before
    /// this check, `migrate` after it).
    AzureSecretName,
}

/// One row: the target's names as `convert` and `migrate` spell them, its
/// rule, and where the rule comes from.
pub struct TargetKeyRule {
    pub names: &'static [&'static str],
    pub rule: KeyRule,
    pub source: &'static str,
}

/// The catalogue. A target not listed is [`KeyRule::Any`].
pub const TARGET_KEY_RULES: &[TargetKeyRule] = &[
    TargetKeyRule {
        names: &["shell"],
        rule: KeyRule::Shell,
        source: "an `export` script is sourced by a shell, which holds only shell names",
    },
    TargetKeyRule {
        names: &["github-actions", "GitHub Actions"],
        rule: KeyRule::Shell,
        source: "GitHub: secret names are letters, digits and _, not starting with a digit",
    },
    TargetKeyRule {
        names: &["vercel", "Vercel"],
        rule: KeyRule::Shell,
        source: "Vercel REST API env_key_invalid_characters: letters, digits and _ only",
    },
    TargetKeyRule {
        names: &["heroku", "Heroku"],
        rule: KeyRule::Shell,
        source: "Heroku config var policy: alphanumerics and _, not starting with a digit, no -",
    },
    TargetKeyRule {
        names: &["doppler", "Doppler"],
        rule: KeyRule::Shell,
        source: "Doppler: secret names are letters, numbers and _, not starting with a number",
    },
    // ⚠️ Not verified against a published rule (2026-11-08): neither provider
    // documents one. Refused rather than sent, so a name the target may not
    // take fails here by name instead of there.
    TargetKeyRule {
        names: &["railway", "Railway"],
        rule: KeyRule::Shell,
        source: "no published rule found; refused as a shell name until checked",
    },
    TargetKeyRule {
        names: &["Infisical"],
        rule: KeyRule::Shell,
        source: "no published rule found; refused as a shell name until checked",
    },
    TargetKeyRule {
        names: &["kubernetes"],
        rule: KeyRule::Kubernetes,
        source: "Kubernetes: a Secret data key matches [-._a-zA-Z0-9]+",
    },
    TargetKeyRule {
        names: &["terraform"],
        rule: KeyRule::Terraform,
        source: "Terraform: a variable name starts with a letter or _ and holds letters, digits, _ and -",
    },
    TargetKeyRule {
        names: &["gcp-secrets", "GCP Secret Manager"],
        rule: KeyRule::GcpSecretId,
        source: "GCP Secret Manager: a secret ID holds letters, digits, _ and -",
    },
    TargetKeyRule {
        names: &["azure-keyvault", "Azure Key Vault"],
        rule: KeyRule::AzureSecretName,
        source: "Azure Key Vault: a secret name holds letters, digits and -",
    },
];

/// The rule for the target `convert` or `migrate` calls `name`.
pub fn for_target(name: &str) -> KeyRule {
    TARGET_KEY_RULES
        .iter()
        .find(|r| r.names.contains(&name))
        .map_or(KeyRule::Any, |r| r.rule)
}

impl KeyRule {
    /// Does the target accept `name`? `name` is the one it will get.
    pub fn accepts(self, name: &str) -> bool {
        let ascii = |extra: &[char]| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
        };
        match self {
            KeyRule::Any => crate::core::parser::is_env_key(name),
            KeyRule::Shell => crate::core::parser::is_valid_key(name),
            KeyRule::Kubernetes => ascii(&['-', '.', '_']),
            KeyRule::Terraform => {
                ascii(&['_', '-'])
                    && name
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            }
            KeyRule::GcpSecretId => ascii(&['_', '-']),
            KeyRule::AzureSecretName => ascii(&['-', '_']),
        }
    }

    /// What the target accepts, for a refusal.
    pub fn describe(self) -> &'static str {
        match self {
            KeyRule::Any => "letters, digits, _, ., -, [ and ]",
            KeyRule::Shell => "letters, digits and _, not starting with a digit",
            KeyRule::Kubernetes => "ASCII letters, digits, -, . and _",
            KeyRule::Terraform => "ASCII letters, digits, _ and -, starting with a letter or _",
            KeyRule::GcpSecretId => "ASCII letters, digits, _ and -",
            KeyRule::AzureSecretName => "ASCII letters, digits, - and _ (written as -)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name evnx read before 0.13.0 still passes every rule, after the
    /// target's own renaming: these rows refuse only what D12 let in.
    #[test]
    fn every_pre_d12_name_passes_every_rule() {
        let renamed = |rule: KeyRule, k: &str| match rule {
            KeyRule::GcpSecretId => k.to_lowercase().replace('_', "-"),
            KeyRule::Terraform => k.to_lowercase(),
            _ => k.to_string(),
        };
        for row in TARGET_KEY_RULES {
            for k in ["A", "_A", "DATABASE_URL", "a_b2", "__X__"] {
                assert!(
                    row.rule.accepts(&renamed(row.rule, k)),
                    "{:?} {k}",
                    row.names
                );
            }
        }
    }

    #[test]
    fn d12_names_are_refused_where_the_target_cannot_hold_them() {
        assert!(!for_target("shell").accepts("my.key"));
        assert!(!for_target("GitHub Actions").accepts("1FOO"));
        assert!(for_target("kubernetes").accepts("my.key"));
        assert!(!for_target("kubernetes").accepts("a[0]"));
        assert!(for_target("json").accepts("café"));
        assert!(for_target("docker-compose").accepts("a[0]"));
        assert!(!for_target("terraform").accepts("my.key"));
        assert!(for_target("terraform").accepts("my-key"));
        assert!(!for_target("gcp-secrets").accepts("my.key"));
        assert!(!for_target("azure-keyvault").accepts("my.key"));
    }
}
