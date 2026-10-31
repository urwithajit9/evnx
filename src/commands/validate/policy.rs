//! What `validate` decides about a value, with no IO: whether it is a
//! placeholder, and what `--fix` would do to it. `checks` and `fixer` both
//! depend on this, and not on each other (B2 P1).

use super::types::IssueType;

/// `validate`'s placeholder question: [`crate::core::classify::is_placeholder`]
/// at the **Strict** level, with the variable's name (C-05, DEC-G13).
///
/// ⚠️ Strict, because `--fix` overwrites what this calls a placeholder. The
/// list (anchored, never `contains`) moved to `core::classify` unchanged; what
/// is new is the name: `API_KEY=dev` is a placeholder, `ENVIRONMENT=dev` is not
/// (DEC-G11), and `*****` or AWS's documentation key always is (DEC-G10).
pub fn is_placeholder(name: &str, value: &str) -> bool {
    crate::core::classify::is_placeholder(name, value, crate::core::classify::Strictness::Strict)
}

// ─────────────────────────────────────────────────────────────
// Fix Action Types
// ─────────────────────────────────────────────────────────────

/// Whether `--fix` may replace a weak secret that holds a **real value**.
///
/// ⛔ A `bool` parameter here would read `suggest_fix(k, v, &ty, false)` at the
/// call site, where `false` says nothing about what it permits. This is the one
/// decision in the module that destroys data when it goes the wrong way, so it
/// is named at every call site rather than positional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeakSecretPolicy {
    /// Placeholders only — a value nobody has filled in yet. The default.
    PlaceholdersOnly,
    /// `--rotate-weak-secrets`: replace a real credential too, because the
    /// person running it said to.
    Rotate,
}

#[derive(Debug, Clone)]
pub enum FixAction {
    GenerateSecret,
    /// A missing variable evnx may generate a value for (DEC-VA6): added with
    /// a new secret in one run, rather than a placeholder and a second run.
    AddSecret,
    ReplacePlaceholder(String),
    FixBoolean(String),
    AddMissing(String),
    Skip,
}

impl FixAction {
    /// Would applying this actually change anything for the better?
    ///
    /// Drives `Issue::auto_fixable`, so the report never offers `--fix` for
    /// something `--fix` will decline to touch.
    pub fn is_actionable(&self) -> bool {
        !matches!(self, FixAction::Skip)
    }
}

// ─────────────────────────────────────────────────────────────
// Fix Suggestion Logic
// ─────────────────────────────────────────────────────────────

pub fn suggest_fix(
    key: &str,
    value: &str,
    issue_type: &IssueType,
    weak_secrets: WeakSecretPolicy,
) -> FixAction {
    match issue_type {
        IssueType::PlaceholderValue => {
            // ⛔ I-260 (DEC-VA4): generated only for a secret nothing outside
            // the app has to hold — its own signing key, or a password it
            // chooses. This generated for **every** secret-shaped name, so
            // `STRIPE_KEY=changeme` became 64 random hex characters that then
            // passed validate: a fake provider key, reported as fixed.
            if crate::core::classify::may_generate(key) {
                FixAction::GenerateSecret
            } else if crate::core::classify::is_secret_name(
                key,
                crate::core::classify::Strictness::Strict,
            ) {
                // A key someone else issues. The finding stays, and does not
                // offer `--fix`; the person fetches the value.
                FixAction::Skip
            } else if key.contains("PORT") {
                // `8080` is a **working default**, not a stand-in — a port is
                // not a credential and the value is immediately usable.
                FixAction::ReplacePlaceholder("8080".to_string())
            } else {
                // ⛔ **URL and EMAIL used to be replaced here**, with
                // `https://example.com` and `user@example.com`. Both are
                // removed, for the reason the `DB_NAME` note below already
                // gives: substituting one placeholder for a *less informative*
                // one is a no-op that reports itself as a repair.
                //
                // It was worse than a no-op for a URL. `DATABASE_URL` is matched
                // by `contains("URL")`, so a postgres connection string was
                // replaced with an **https** one — a value that is wrong in its
                // scheme, its host and its credentials, written over the only
                // line that said what the database was:
                //
                // ```text
                // DATABASE_URL=postgres://u:pw@db/todos → https://example.com
                // ```
                //
                // ⚠️ That particular case reached a real value because
                // `is_placeholder` matched `todo` as a substring, which is
                // fixed. But `https://example.com` would still be the wrong
                // repair for a genuine `your_database_url`, so the branch goes.
                // ⚠️ Not `your_value_here`. For a secret, a URL, an email or a
                // port there is a right answer evnx can supply; for `DB_NAME`
                // there is not, and substituting one placeholder for a *less
                // informative* one is a no-op that reports itself as a repair:
                //
                //   • DB_NAME: "your_db_name_value" → your_value_here
                //   ✗  DB_NAME looks like a placeholder
                //
                // It also discards the only hint the line carried about what
                // the variable is for.
                FixAction::Skip
            }
        }
        IssueType::BooleanTrap => {
            let fixed = if value.eq_ignore_ascii_case("true") {
                "true"
            } else {
                "false"
            };
            FixAction::FixBoolean(fixed.to_string())
        }
        // ⛔ **Only when the value is a placeholder.**
        //
        // Until 2026-10-07 this was an unconditional `GenerateSecret`, so
        // `--fix` replaced *working credentials* with random hex:
        //
        // ```text
        // DATABASE_PASSWORD="p@ss word#1"   →   DATABASE_PASSWORD=ad0da713…c508
        //   • DATABASE_PASSWORD: "p@ss word#1" → ad0da713…
        //   ✓ All checks passed
        // ```
        //
        // The check behind it is a *heuristic about length and wording*. It
        // cannot tell "this is a placeholder nobody has filled in" from "this
        // is the real password, and it is short" — and the second is the
        // common case, because a short password is exactly what trips it.
        //
        // Generating over a placeholder is a repair. Generating over a value
        // is destruction of the one thing on the machine that cannot be
        // recomputed, performed by a command whose name says *validate*.
        //
        // The finding still fires; it simply stops claiming to be fixable, and
        // keeps the advice it already carried ("Run: openssl rand -hex 32"),
        // which the person can act on with the old value still in front of
        // them.
        //
        // ⛔ And only for a secret evnx may generate at all (I-260): the
        // `--rotate-weak-secrets` flag included (DEC-VA5). A provider always
        // holds its key, so a value invented here never matches it.
        IssueType::WeakSecret => {
            if crate::core::classify::may_generate(key)
                && (is_placeholder(key, value) || weak_secrets == WeakSecretPolicy::Rotate)
            {
                FixAction::GenerateSecret
            } else {
                FixAction::Skip
            }
        }
        // A missing variable has no value. One evnx may generate is added
        // with a new secret (DEC-VA6); for the rest, the caller that knows the
        // template uses [`fill_for_missing`] with it.
        IssueType::MissingVariable if crate::core::classify::may_generate(key) => {
            FixAction::AddSecret
        }
        IssueType::MissingVariable => FixAction::AddMissing(fill_for_missing(key, None)),
        _ => FixAction::Skip,
    }
}

/// What `--fix` writes for a variable the file lacks (I-037, D-25).
///
/// - A secret-shaped name gets the generic placeholder: evnx cannot invent a
///   Stripe key, and a template's value for a secret is never copied — a
///   sample key that does not look like a placeholder would then pass. (A
///   secret evnx may generate never gets here: it is `FixAction::AddSecret`.)
/// - Anything else gets the template's own value, whatever it is: `8080`,
///   `localhost`, a documentation URL, or nothing at all for `OPTIONAL_X=`.
///   The template is committed, so copying it reveals nothing, and it is what
///   `cp .env.example .env` would have given. It replaced `your_value_here`
///   for every name, which threw away the one hint the template carried.
/// - A name the template does not have (a contract requires it) gets
///   `your_value_here`.
///
/// Whether the result still needs a person is [`is_placeholder`]'s
/// call, made by the report — not this function's.
pub fn fill_for_missing(key: &str, template_value: Option<&str>) -> String {
    if crate::core::classify::is_secret_name(key, crate::core::classify::Strictness::Strict) {
        return "CHANGE_ME_SECURE_32_CHARS_MIN".to_string();
    }
    template_value.unwrap_or("your_value_here").to_string()
}
