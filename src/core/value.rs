//! What a value means, decided once (R-template, 0.10.5).
//!
//! Boolean words were recognised in three places with three lists: template's
//! `|bool` (`true`/`yes`/`1`, everything else silently `false`, I-138), spec's
//! `format = "bool"` (`true`/`false`/`1`/`0`/`yes`/`no`) and spec's inference
//! (`true`/`false`/`yes`/`no`). This module holds the one list; spec moved onto
//! it in 0.10.6 (R-spec), which widened what `format = "bool"` accepts.

/// The words read as true, in any case (DEC-T7).
pub const TRUE_WORDS: [&str; 5] = ["true", "yes", "on", "1", "y"];

/// The words read as false, in any case (DEC-T7).
pub const FALSE_WORDS: [&str; 5] = ["false", "no", "off", "0", "n"];

/// `Some(true)` or `Some(false)` for a boolean word in any case; `None` for
/// anything else.
///
/// ⚠️ `None` is the point. An unknown word (`enabled`, `maybe`, a typo) is not
/// `false`: reading it as false turned a misspelt `DEBUG=ture` into a quiet
/// production setting. The value is matched exactly — no trimming, like `|int`
/// — so `" yes"` and the empty value are `None` too.
pub fn parse_bool(value: &str) -> Option<bool> {
    if TRUE_WORDS.iter().any(|w| value.eq_ignore_ascii_case(w)) {
        Some(true)
    } else if FALSE_WORDS.iter().any(|w| value.eq_ignore_ascii_case(w)) {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_in_any_case() {
        for w in TRUE_WORDS {
            for v in [w.to_string(), w.to_uppercase()] {
                assert_eq!(parse_bool(&v), Some(true), "{v}");
            }
        }
        for w in FALSE_WORDS {
            for v in [w.to_string(), w.to_uppercase()] {
                assert_eq!(parse_bool(&v), Some(false), "{v}");
            }
        }
        assert_eq!(parse_bool("On"), Some(true));
        assert_eq!(parse_bool("oFF"), Some(false));
    }

    #[test]
    fn anything_else_is_not_a_boolean() {
        for v in [
            "", " yes", "yes ", "enabled", "maybe", "ture", "2", "-1", "t", "f",
        ] {
            assert_eq!(parse_bool(v), None, "{v:?}");
        }
    }
}
