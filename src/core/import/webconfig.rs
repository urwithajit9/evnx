//! `web.config`: the `<appSettings>` of an ASP.NET application.
//!
//! Only `<configuration><appSettings>` is read, and in it only what .NET reads:
//!
//! * `<add key="…" value="…"/>` sets a key;
//! * `<remove key="…"/>` takes one away and `<clear/>` takes all away, in the
//!   order they come, so a later `<add>` puts a key back;
//! * a key given twice counts last, in its first place (as in JSON and YAML).
//!   Keys are matched without regard to ASCII case, as .NET does.
//!
//! The XML reader is `quick-xml`. Entities (`&amp;`, `&lt;`, `&quot;`,
//! `&#x41;`) are decoded and an attribute value is normalised as XML says (a
//! literal line break or tab is a space; `&#10;` is a line break). Comments,
//! CDATA and text are skipped, so a commented-out `<add>` is not imported.
//! `<connectionStrings>`, `<location>` and everything else are ignored. An
//! `<appSettings configSource="…">` keeps its entries in another file, which
//! is refused with a message saying so.
//!
//! ⛔ Errors name a line and a column and a fixed reason, never text from the
//! file; a value is only ever put in a [`Secret`].

use anyhow::{Result, bail};
use indexmap::IndexMap;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::Found;
use super::text::normalise;
use crate::core::secret::Secret;

/// One `<add>` that is in force.
struct Entry {
    key: String,
    value: std::result::Result<Secret, String>,
}

pub fn read(text: &str, section: Option<&str>) -> Result<Vec<Found>> {
    if section.is_some() {
        bail!("--section does not apply to a web.config: only <appSettings> is read");
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    let mut path: Vec<String> = Vec::new();
    let mut seen_root = false;
    let mut seen_settings = false;
    // Lower-cased key → the entry in force.
    let mut entries: IndexMap<String, Entry> = IndexMap::new();
    let mut unnamed = 0usize;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| invalid(text, reader.error_position(), reason(&e)))?;
        let at = reader.buffer_position();
        match event {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                let name = event_name(e);
                if path.is_empty() {
                    if seen_root {
                        return Err(invalid(text, at, "more than one top-level element"));
                    }
                    root(&name)?;
                    seen_root = true;
                } else if in_settings(&path) {
                    element(e, &name, &mut entries, &mut unnamed, text, at)?;
                } else if path.len() == 1 && name == "appSettings" {
                    settings_start(e, text, at)?;
                    seen_settings = true;
                }
                if matches!(event, Event::Start(_)) {
                    path.push(name);
                }
            }
            Event::End(_) => {
                path.pop();
            }
            // Text, CDATA, comments, the declaration, processing instructions,
            // a DOCTYPE and entity references in text: none of them is a setting.
            _ => {}
        }
    }
    if !seen_root {
        bail!("the web.config has no <configuration> element");
    }
    if !seen_settings {
        bail!(
            "the web.config has no <appSettings> in <configuration>: only appSettings entries \
             are imported"
        );
    }
    Ok(entries
        .into_values()
        .map(|e| Found {
            source: format!("appSettings.{}", e.key),
            name: normalise(&e.key),
            value: e.value,
        })
        .collect())
}

fn event_name(e: &BytesStart) -> String {
    e.local_name().as_ref().to_string()
}

fn root(name: &str) -> Result<()> {
    if name != "configuration" {
        bail!("expected <configuration> at the top of the web.config");
    }
    Ok(())
}

/// Directly inside `<configuration><appSettings>`.
fn in_settings(path: &[String]) -> bool {
    path.len() == 2 && path[1] == "appSettings"
}

fn settings_start(e: &BytesStart, text: &str, at: u64) -> Result<()> {
    if attribute(e, "configSource", text, at)?.is_some() {
        bail!(
            "<appSettings> takes its entries from the file named by configSource: \
             import that file instead"
        );
    }
    Ok(())
}

/// One child of `<appSettings>`.
fn element(
    e: &BytesStart,
    name: &str,
    entries: &mut IndexMap<String, Entry>,
    unnamed: &mut usize,
    text: &str,
    at: u64,
) -> Result<()> {
    match name {
        "clear" => entries.clear(),
        "remove" => {
            if let Some(key) = attribute(e, "key", text, at)? {
                entries.shift_remove(&key.to_ascii_lowercase());
            }
        }
        "add" => {
            let Some(key) = attribute(e, "key", text, at)? else {
                // Listed, so it is not lost without a word; no name to give it.
                *unnamed += 1;
                entries.insert(
                    format!("\0{unnamed}"),
                    Entry {
                        key: format!("add[{unnamed}]"),
                        value: Err("has no key attribute".to_string()),
                    },
                );
                return Ok(());
            };
            let value = match attribute(e, "value", text, at)? {
                Some(v) => Ok(Secret::new(v)),
                None => Err("has no value attribute".to_string()),
            };
            let lower = key.to_ascii_lowercase();
            match entries.get_mut(&lower) {
                Some(slot) => {
                    slot.key = key;
                    slot.value = value;
                }
                None => {
                    entries.insert(lower, Entry { key, value });
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The decoded value of attribute `name`, if it is there. Every attribute is
/// checked, so one given twice is an error, as XML says.
fn attribute(e: &BytesStart, name: &str, text: &str, at: u64) -> Result<Option<String>> {
    let mut found = None;
    for attr in e.attributes().with_checks(true) {
        let attr =
            attr.map_err(|_| invalid(text, at, "an attribute is malformed or given twice"))?;
        if attr.key.local_name().as_ref() != name {
            continue;
        }
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| {
                invalid(
                    text,
                    at,
                    "an entity or character reference is unknown or malformed",
                )
            })?;
        found = Some(value.into_owned());
    }
    Ok(found)
}

/// A fixed reason for each kind of error: quick-xml's own messages can quote
/// the text (an entity name, a tag name).
fn reason(e: &quick_xml::Error) -> &'static str {
    use quick_xml::Error as E;
    match e {
        E::Syntax(_) => "a syntax error",
        E::IllFormed(_) => "a tag is not closed, or closed without being opened",
        E::InvalidAttr(_) => "an attribute is malformed",
        E::Escape(_) => "an entity or character reference is unknown or malformed",
        _ => "the file cannot be read as XML",
    }
}

/// `not valid XML (line L, column C): reason`, from a byte offset.
fn invalid(text: &str, offset: u64, reason: &str) -> anyhow::Error {
    let offset = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(text.len());
    let mut cut = offset;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let before = &text[..cut];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    anyhow::anyhow!("not valid XML (line {line}, column {column}): {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str) -> Vec<(String, String, std::result::Result<String, String>)> {
        read(text, None)
            .unwrap()
            .into_iter()
            .map(|f| (f.source, f.name, f.value.map(|v| v.expose().to_string())))
            .collect()
    }

    fn wrap(inner: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?>\n<configuration>\n<appSettings>\n{inner}\n</appSettings>\n</configuration>\n"
        )
    }

    #[test]
    fn add_entries_in_order_with_entities_decoded() {
        let got = rows(&wrap(
            r#"<add key="Db:Host" value="a&amp;b &lt;c&gt; &quot;q&quot; &apos;s&apos; &#x41;&#66;" />
               <add key='ApiKey' value='k'/>"#,
        ));
        assert_eq!(
            got,
            [
                (
                    "appSettings.Db:Host".to_string(),
                    "DB_HOST".to_string(),
                    Ok("a&b <c> \"q\" 's' AB".to_string())
                ),
                ("appSettings.ApiKey".into(), "APIKEY".into(), Ok("k".into())),
            ]
        );
    }

    #[test]
    fn clear_and_remove_apply_in_order_and_the_last_add_counts() {
        let got = rows(&wrap(
            r#"<add key="A" value="1"/><add key="B" value="2"/><remove key="a"/>
               <add key="C" value="3"/><add key="c" value="4"/><clear/>
               <add key="D" value="5"/><add key="E" value="6"/><add key="D" value="7"/>"#,
        ));
        let names: Vec<_> = got.iter().map(|r| (r.1.as_str(), r.2.clone())).collect();
        assert_eq!(names, [("D", Ok("7".into())), ("E", Ok("6".into()))]);
    }

    #[test]
    fn an_attribute_given_twice_is_an_error_without_the_value() {
        let e = read(&wrap(r#"<add key="K" value="hunter2" value="b"/>"#), None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("given twice"), "{e}");
        assert!(!e.contains("hunter2"), "{e}");
    }

    #[test]
    fn a_line_break_written_as_a_reference_survives() {
        let got = rows(&wrap("<add key=\"K\" value=\"l1&#10;l2\tx\ny\"/>"));
        assert_eq!(got[0].2, Ok("l1\nl2 x y".into()));
    }
}
