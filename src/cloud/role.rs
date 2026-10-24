//! How cloud commands read a role someone typed (A-303, DEC-VL10).
//!
//! `vault share`, `vault role`, `org invite` and `org role` had three copies with
//! three rules: `org` did not trim, `vault share` refused `owner` without saying
//! why, and each worded the refusal its own way.

use anyhow::{anyhow, Result};

/// What `vault share` and `vault role` can grant.
pub(crate) const VAULT_ROLES: &[&str] = &["viewer", "developer", "admin"];

/// What `org invite` and `org role` can grant.
pub(crate) const ORG_ROLES: &[&str] = &["member", "admin"];

/// Why a vault's `owner` cannot be granted.
pub(crate) const VAULT_OWNER: &str = "A vault has one owner, set once by whoever created it.";

/// Why an organisation's `owner` cannot be granted.
///
/// ⚠️ Refused client-side as well as by the server and by migration 010's CHECK.
/// Three layers, because an organisation has exactly one owner and an invitation
/// is a link somebody can forward.
pub(crate) const ORG_OWNER: &str = "An organisation has exactly one owner, set\n\
     \x20 when it is created — handing it out through an invitation or a role\n\
     \x20 change would leave two, and nothing could then remove either.";

/// `raw` as one of `assignable`: trimmed, lower-cased.
///
/// `owner` is never assignable; refusing it says why (`owner_reason`), because
/// "not one of these" reads as if `owner` were a typo.
pub(crate) fn parse_role(raw: &str, assignable: &[&str], owner_reason: &str) -> Result<String> {
    let role = raw.trim().to_lowercase();
    if role == "owner" {
        return Err(anyhow!("`owner` cannot be granted. {owner_reason}"));
    }
    if !assignable.contains(&role.as_str()) {
        return Err(anyhow!(
            "role must be one of: {}. Got `{role}`.",
            assignable.join(", ")
        ));
    }
    Ok(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One table for both kinds of role.
    #[test]
    fn t_dec_vl10_one_role_parser_for_vaults_and_orgs() {
        for (raw, roles, want) in [
            ("viewer", VAULT_ROLES, "viewer"),
            (" Admin ", VAULT_ROLES, "admin"),
            ("DEVELOPER", VAULT_ROLES, "developer"),
            ("member", ORG_ROLES, "member"),
            (" ADMIN ", ORG_ROLES, "admin"),
        ] {
            assert_eq!(parse_role(raw, roles, "").unwrap(), want, "{raw:?}");
        }

        for (raw, roles) in [
            ("member", VAULT_ROLES),
            ("developer", ORG_ROLES),
            ("viewer", ORG_ROLES),
            ("", VAULT_ROLES),
            ("", ORG_ROLES),
        ] {
            let e = parse_role(raw, roles, "").unwrap_err().to_string();
            assert!(e.starts_with("role must be one of: "), "{raw:?}: {e}");
        }

        let e = parse_role(" Owner ", VAULT_ROLES, VAULT_OWNER)
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "`owner` cannot be granted. A vault has one owner, set once by whoever created it."
        );
        let e = parse_role("owner", ORG_ROLES, ORG_OWNER)
            .unwrap_err()
            .to_string();
        assert!(e.contains("exactly one owner"), "{e}");
    }
}
