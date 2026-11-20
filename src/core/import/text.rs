//! The line-based readers: `.env`, `.properties`, INI and CSV.
//!
//! Errors name a line, never what is on it.

use anyhow::{Result, bail};

use super::Found;
use crate::core::secret::Secret;

/// A name or a key part as an environment variable spells it: ASCII letters
/// and digits upper-cased, `_` kept, every other run of characters one `_`.
/// `db.host`, `db-host` and `DB HOST` all become `DB_HOST`.
pub fn normalise(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    let mut pending = false;
    for c in part.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            if pending && !out.is_empty() {
                out.push('_');
            }
            pending = false;
            out.push(c.to_ascii_uppercase());
        } else {
            pending = true;
        }
    }
    // A key that began with punctuation keeps one leading `_`: `.hidden`.
    if out.is_empty() && !part.is_empty() {
        return String::new();
    }
    out
}

fn found(source: impl Into<String>, name: String, value: String) -> Found {
    Found {
        source: source.into(),
        name,
        value: Ok(Secret::new(value)),
    }
}

fn no_section(format: &str, section: Option<&str>) -> Result<()> {
    if let Some(s) = section {
        bail!("--section {s} does not apply to {format}: it has no sections");
    }
    Ok(())
}

// ── .env ────────────────────────────────────────────────────────────────

/// `NAME=value` lines, read by evnx's own parser with no expansion.
pub fn env(text: &str, section: Option<&str>) -> Result<Vec<Found>> {
    no_section("an env file", section)?;
    let doc = crate::core::parser::Parser::new(crate::core::parser::ParserConfig {
        allow_expansion: false,
        ..Default::default()
    })
    .parse_document(text);
    if let Some(p) = doc.problems.first() {
        bail!(
            "the file has a line evnx cannot read, so nothing was imported:\n  {}\n  \
             `evnx validate --env <file>` lists every problem.",
            p.describe()
        );
    }
    // The last assignment of a name counts, as everywhere else.
    let mut last: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    for e in &doc.entries {
        last.insert(e.key.clone(), e.value.expose().replace('\u{fdd0}', ""));
    }
    Ok(last
        .into_iter()
        .map(|(k, v)| found(k.clone(), normalise(&k), v))
        .collect())
}

// ── .properties ─────────────────────────────────────────────────────────

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) if hex.len() == 4 => out.push(ch),
                    _ => {
                        out.push_str("\\u");
                        out.push_str(&hex);
                    }
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Java `.properties`: `key=value`, `key:value` or `key value`, `#` and `!`
/// comments, a trailing `\` continues the line, `\n`, `\t` and `\uXXXX`.
pub fn properties(text: &str, section: Option<&str>) -> Result<Vec<Found>> {
    no_section(".properties", section)?;
    let mut logical: Vec<(usize, String)> = Vec::new();
    let mut pending: Option<(usize, String)> = None;
    for (i, raw) in text.lines().enumerate() {
        let line = match pending.take() {
            Some((n, mut acc)) => {
                acc.push_str(raw.trim_start());
                (n, acc)
            }
            None => {
                let t = raw.trim_start();
                if t.is_empty() || t.starts_with('#') || t.starts_with('!') {
                    continue;
                }
                (i + 1, t.to_string())
            }
        };
        let slashes = line.1.chars().rev().take_while(|c| *c == '\\').count();
        if slashes % 2 == 1 {
            let mut l = line;
            l.1.pop();
            pending = Some(l);
        } else {
            logical.push(line);
        }
    }
    if let Some(l) = pending {
        logical.push(l);
    }

    let mut out = Vec::new();
    for (n, line) in logical {
        let mut key_end = line.len();
        let mut escaped = false;
        for (i, c) in line.char_indices() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if matches!(c, '=' | ':') || c.is_whitespace() {
                key_end = i;
                break;
            }
        }
        let key = unescape(&line[..key_end]);
        let mut rest = line[key_end..].trim_start();
        if let Some(r) = rest.strip_prefix(['=', ':']) {
            rest = r.trim_start();
        }
        let source = if key.is_empty() {
            format!("line {n}")
        } else {
            key.clone()
        };
        out.push(found(source, normalise(&key), unescape(rest)));
    }
    Ok(out)
}

// ── INI ─────────────────────────────────────────────────────────────────

/// INI: `[section]` then `key = value` or `key: value`; `;` and `#` start a
/// comment line. `[db] host` becomes `DB_HOST`. With a section, only it is
/// read, and its name is left off.
pub fn ini(text: &str, wanted: Option<&str>) -> Result<Vec<Found>> {
    let mut sections: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[') {
            let Some(name) = inner.strip_suffix(']') else {
                bail!("line {} opens a section and does not close it", i + 1);
            };
            let name = name.trim().to_string();
            if !sections.contains(&name) {
                sections.push(name.clone());
            }
            current = Some(name);
            continue;
        }
        let Some(at) = line.find(['=', ':']) else {
            bail!("line {} has no = or : (INI wants key = value)", i + 1);
        };
        let key = line[..at].trim();
        let mut value = line[at + 1..].trim();
        if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            value = &value[1..value.len() - 1];
        }
        match (wanted, &current) {
            (Some(w), Some(c)) if c.eq_ignore_ascii_case(w) => out.push(found(
                format!("{c}.{key}"),
                normalise(key),
                value.to_string(),
            )),
            (Some(_), _) => {}
            (None, Some(c)) => out.push(found(
                format!("{c}.{key}"),
                format!("{}_{}", normalise(c), normalise(key)),
                value.to_string(),
            )),
            (None, None) => out.push(found(key.to_string(), normalise(key), value.to_string())),
        }
    }
    if let Some(w) = wanted {
        if !sections.iter().any(|s| s.eq_ignore_ascii_case(w)) {
            bail!(
                "no section [{w}] in the file (sections: {})",
                if sections.is_empty() {
                    "none".to_string()
                } else {
                    sections.join(", ")
                }
            );
        }
    }
    Ok(out)
}

// ── CSV ─────────────────────────────────────────────────────────────────

/// Records of RFC 4180 CSV: quoted fields, `""` for a quote, line breaks
/// inside quotes. `(first line of the record, fields)`.
fn records(text: &str) -> Result<Vec<(usize, Vec<String>)>> {
    let mut out = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut quoted = false;
    let mut line = 1usize;
    let mut start = 1usize;
    let mut chars = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .chars()
        .peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                '\n' => {
                    line += 1;
                    field.push('\n');
                }
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !quoted => {
                in_quotes = true;
                quoted = true;
            }
            ',' => {
                fields.push(std::mem::take(&mut field));
                quoted = false;
            }
            '\r' => {}
            '\n' => {
                fields.push(std::mem::take(&mut field));
                quoted = false;
                out.push((start, std::mem::take(&mut fields)));
                line += 1;
                start = line;
                any = false;
            }
            c => field.push(c),
        }
    }
    if in_quotes {
        bail!("line {start}: a quoted field is never closed");
    }
    if any {
        fields.push(field);
        out.push((start, fields));
    }
    Ok(out)
}

/// Two columns, name and value. A first row of `name,value` (or `key,value`)
/// is a header and is skipped.
pub fn csv(text: &str, section: Option<&str>) -> Result<Vec<Found>> {
    no_section("CSV", section)?;
    let mut out = Vec::new();
    for (i, (line, fields)) in records(text)?.into_iter().enumerate() {
        if fields.len() == 1 && fields[0].trim().is_empty() {
            continue;
        }
        if fields.len() != 2 {
            bail!(
                "line {line} has {} columns; a CSV import takes two: name,value",
                fields.len()
            );
        }
        if i == 0
            && matches!(
                fields[0].trim().to_lowercase().as_str(),
                "name" | "key" | "variable"
            )
            && fields[1].trim().eq_ignore_ascii_case("value")
        {
            continue;
        }
        let key = fields[0].trim();
        out.push(found(
            if key.is_empty() {
                format!("line {line}")
            } else {
                key.to_string()
            },
            normalise(key),
            fields[1].clone(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(f: Vec<Found>) -> Vec<(String, String, String)> {
        f.into_iter()
            .map(|f| {
                (
                    f.source,
                    f.name,
                    f.value.map(|v| v.expose().to_string()).unwrap_or_default(),
                )
            })
            .collect()
    }

    #[test]
    fn normalise_makes_upper_snake() {
        for (given, want) in [
            ("db.host", "DB_HOST"),
            ("db-host", "DB_HOST"),
            ("DB HOST", "DB_HOST"),
            ("Db..host", "DB_HOST"),
            ("api_key", "API_KEY"),
            ("a__b", "A__B"),
            ("port", "PORT"),
            ("1st", "1ST"),
            ("", ""),
            ("...", ""),
        ] {
            assert_eq!(normalise(given), want, "{given}");
        }
    }

    #[test]
    fn env_reads_quotes_and_keeps_dollars_literal() {
        let got = pairs(
            env(
                "A=1\nexport b=\"x y\"\nC='${NOT}'\nA=2\n# note\nD=\"l1\\nl2\"\n",
                None,
            )
            .unwrap(),
        );
        assert_eq!(
            got,
            [
                ("A".into(), "A".into(), "2".into()),
                ("b".into(), "B".into(), "x y".into()),
                ("C".into(), "C".into(), "${NOT}".into()),
                ("D".into(), "D".into(), "l1\nl2".into()),
            ]
        );
    }

    #[test]
    fn env_with_an_unreadable_line_names_the_line_not_the_text() {
        let e = env("A=1\nhunter2 oops\n", None).unwrap_err().to_string();
        assert!(e.contains("Line 2"), "{e}");
        assert!(!e.contains("hunter2"), "{e}");
    }

    #[test]
    fn properties_cover_separators_comments_continuations_and_escapes() {
        let text = "# c\n! c\ndb.host=localhost\ndb.port : 5432\nname   value with spaces\n\
                    long=a\\\n   b\nuni=caf\\u00e9\nnl=a\\nb\nempty=\nkey\\ with\\ space=v\n";
        let got = pairs(properties(text, None).unwrap());
        assert_eq!(
            got,
            [
                ("db.host".into(), "DB_HOST".into(), "localhost".into()),
                ("db.port".into(), "DB_PORT".into(), "5432".into()),
                ("name".into(), "NAME".into(), "value with spaces".into()),
                ("long".into(), "LONG".into(), "ab".into()),
                ("uni".into(), "UNI".into(), "café".into()),
                ("nl".into(), "NL".into(), "a\nb".into()),
                ("empty".into(), "EMPTY".into(), String::new()),
                ("key with space".into(), "KEY_WITH_SPACE".into(), "v".into()),
            ]
        );
    }

    #[test]
    fn ini_prefixes_the_section_and_a_section_flag_picks_one() {
        let text = "top=1\n; c\n[db]\nhost = localhost\nport: 5432\n[Cache]\nurl=\"redis://x\"\n";
        let got = pairs(ini(text, None).unwrap());
        assert_eq!(
            got,
            [
                ("top".into(), "TOP".into(), "1".into()),
                ("db.host".into(), "DB_HOST".into(), "localhost".into()),
                ("db.port".into(), "DB_PORT".into(), "5432".into()),
                ("Cache.url".into(), "CACHE_URL".into(), "redis://x".into()),
            ]
        );
        let one = pairs(ini(text, Some("DB")).unwrap());
        assert_eq!(
            one,
            [
                ("db.host".into(), "HOST".into(), "localhost".into()),
                ("db.port".into(), "PORT".into(), "5432".into()),
            ]
        );
        let e = ini(text, Some("nope")).unwrap_err().to_string();
        assert!(e.contains("sections: db, Cache"), "{e}");
        assert!(ini("[open\n", None).is_err());
        assert!(ini("no separator\n", None).is_err());
    }

    #[test]
    fn csv_reads_two_columns_a_header_and_quoted_fields() {
        let text = "name,value\nA,1\n\"B\",\"x,\"\"y\"\"\"\nC,\"l1\nl2\"\n\n";
        let got = pairs(csv(text, None).unwrap());
        assert_eq!(
            got,
            [
                ("A".into(), "A".into(), "1".into()),
                ("B".into(), "B".into(), "x,\"y\"".into()),
                ("C".into(), "C".into(), "l1\nl2".into()),
            ]
        );
        // No header: the first row is data.
        assert_eq!(csv("A,1\r\nB,2\r\n", None).unwrap().len(), 2);
        let e = csv("A,1\nB,2,3\n", None).unwrap_err().to_string();
        assert!(e.contains("line 2 has 3 columns"), "{e}");
        assert!(csv("A,\"open\n", None).is_err());
    }

    #[test]
    fn sections_are_refused_where_there_are_none() {
        assert!(env("A=1\n", Some("x")).is_err());
        assert!(csv("A,1\n", Some("x")).is_err());
        assert!(properties("a=1\n", Some("x")).is_err());
    }
}
