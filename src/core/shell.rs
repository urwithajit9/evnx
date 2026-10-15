//! Shell words: one quoting rule for every command evnx prints (A-16, U6).
//!
//! `migrate` printed commands with five hand-written escapes. Single quotes with
//! `'\''` were right (heroku, doppler, infisical, railway); aws escaped `'` as
//! `\'`, which **ends** a single-quoted string (I-094); vercel and azure used
//! double quotes, so `$5` came out empty and `$(…)` ran when the user ran the
//! command (I-101); names were printed with no quoting at all, so `;` in a name
//! started a second command (I-102).
//!
//! Two functions, one rule: *the shell hands the program exactly this string.*
//!
//! - [`quote`] for values: always single-quoted, so a reader sees at once
//!   where a value starts and ends.
//! - [`word`] for names and flags: bare when every character is safe, quoted
//!   otherwise, so ordinary commands stay readable.

/// `value` single-quoted; the shell reads back exactly `value`.
///
/// Inside single quotes nothing is special — no `$`, backtick, backslash, `!`
/// or `#`. A single quote itself is written as close, escaped quote, reopen:
/// `'\''`. ⚠️ Not `\'` inside the quotes: that is two characters there, and the
/// next `'` closes the string (I-094).
#[must_use]
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// `s` as one shell word: bare when it is non-empty and every character is in
/// `A-Za-z0-9` or `_ . / : @ % + , = -`, otherwise [`quote`]d.
#[must_use]
pub fn word(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '.' | '/' | ':' | '@' | '%' | '+' | ',' | '=' | '-')
        });
    if safe {
        s.to_string()
    } else {
        quote(s)
    }
}

/// `KEY=value` as one shell word, for tools that take assignments
/// (`heroku config:set`, `doppler secrets set`).
#[must_use]
pub fn assignment(key: &str, value: &str) -> String {
    format!("{}={}", word(key), quote(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SHARE-11: what `sh` hands a program is exactly the input.
    #[test]
    fn t_share_11_sh_reads_back_every_value() {
        let values = [
            "it's",
            "$5 off",
            "$(touch /nonexistent/evnx-pwned)",
            "`id`",
            "back\\slash",
            "two words",
            "say \"hi\"",
            "#hash",
            "!bang",
            "*",
            "",
            "line1\nline2",
            "'",
            "''",
            "日本語",
        ];
        for v in values {
            for printed in [quote(v), word(v)] {
                let out = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("printf '%s' {printed}"))
                    .output()
                    .expect("sh");
                assert_eq!(
                    String::from_utf8_lossy(&out.stdout),
                    v,
                    "{v:?} printed as {printed}"
                );
            }
        }
    }

    #[test]
    fn plain_names_stay_bare() {
        for s in ["my-app", "prod/myapp/config", "v1.2", "DB_URL"] {
            assert_eq!(word(s), s);
        }
        assert_eq!(word("x;y"), "'x;y'");
        assert_eq!(assignment("A", "it's"), "A='it'\\''s'");
    }
}
