use super::expand::LITERAL_DOLLAR;
use super::lines::closing_quote_index;
use super::*;

// ─── D16: a value containing `$` can be written ──────────────────────────
//
// Expansion used to run in every quoting form and there was no escape, so a
// generated secret containing `${...}` made the file fail to parse and one
// containing `$NAME`, where NAME was also a key, was silently replaced — with
// another of your secrets, and with no error.

fn parse_one(content: &str) -> IndexMap<String, String> {
    Parser::default().parse_content(content).expect("parses")
}

// ─────────────────────────────────────────────────────────────────────────
// P2: `#` inside an unquoted value
// ─────────────────────────────────────────────────────────────────────────

/// ⛔ Every `#` used to end the value, so four realistic values were
/// silently destroyed.
///
/// ⚠️ The expectations below are **Docker Compose's**, measured with
/// `docker compose config` rather than assumed. Node dotenv 17 disagrees —
/// it truncates at any `#`, turning `COLOR=#ff0000` into an empty string —
/// and the divergence is deliberate: the failure mode of that rule is a
/// destroyed password or URL, and `evnx convert --to docker` would otherwise
/// emit a file Compose reads differently from the `.env` it came from.
#[test]
fn a_hash_without_preceding_whitespace_stays_in_the_value() {
    let vars = parse_one(
        "COLOR=#ff0000\n\
         PW=Tr0ub4dor#3\n\
         URL=https://x/#/route\n\
         TAG=v1.0#rc1\n\
         TIGHT=a#b\n",
    );
    assert_eq!(vars["COLOR"], "#ff0000");
    assert_eq!(vars["PW"], "Tr0ub4dor#3");
    assert_eq!(vars["URL"], "https://x/#/route");
    assert_eq!(vars["TAG"], "v1.0#rc1");
    assert_eq!(vars["TIGHT"], "a#b");
}

/// The complement: a `#` that *is* preceded by whitespace is still a
/// comment, which is the one case every implementation agrees on.
#[test]
fn a_hash_after_whitespace_is_still_a_comment() {
    let vars = parse_one("A=v # comment\nB=v\t# comment\nC=v   #c\n");
    assert_eq!(vars["A"], "v");
    assert_eq!(vars["B"], "v");
    assert_eq!(vars["C"], "v");
}

/// ⓘ A `#` at the start of a value is literal, so there is no way to write
/// an empty value followed by a comment. That is Compose's behaviour
/// verbatim — `LEAD= #ff0000` gives `#ff0000`, not an empty string.
#[test]
fn a_leading_hash_is_part_of_the_value() {
    let vars = parse_one("LEAD= #ff0000\nEMPTY= # just a comment\n");
    assert_eq!(vars["LEAD"], "#ff0000");
    assert_eq!(vars["EMPTY"], "# just a comment");
}

/// A quoted value was never affected and must stay that way.
#[test]
fn a_quoted_value_keeps_its_hash_and_drops_a_trailing_comment() {
    let vars = parse_one("Q=\"keep # this\" # drop this\nS='also # kept'\n");
    assert_eq!(vars["Q"], "keep # this");
    assert_eq!(vars["S"], "also # kept");
}

/// ⚠️ The whitespace test indexes bytes. A multi-byte character before a
/// `#` must not be mistaken for whitespace, and must not panic.
#[test]
fn a_multibyte_character_before_a_hash_is_not_whitespace() {
    let vars = parse_one("A=日本#x\nB=café#1\nC=日本 #c\n");
    assert_eq!(vars["A"], "日本#x");
    assert_eq!(vars["B"], "café#1");
    assert_eq!(vars["C"], "日本", "a real comment after a multi-byte value");
}

// ─────────────────────────────────────────────────────────────────────────
// S4: a comment after a closing quote
// ─────────────────────────────────────────────────────────────────────────

/// ⛔ **The serious half of this defect: variables vanished, exit 0.**
///
/// `opens_multiline` asked whether the *line* ended with the quote. For
/// `A="x" # c` it does not, so the value was treated as opening a multiline
/// one — and the parser then scanned forward for a closing quote it had
/// already passed. Whether that failed loudly depended entirely on what
/// came next:
///
/// ```text
/// A="x" # c            A="x" # c
/// B=plain              B="y"
///   → exit 2             → ONE variable, exit 0
/// ```
///
/// The second shape is the common one — most `.env` files have more than
/// one quoted value — and `convert`, `cloud push`, `cloud run` and
/// `cloud export` all read through this parser. Five lines became three and
/// nothing said so.
#[test]
fn a_comment_after_a_closing_quote_loses_no_variables() {
    let vars = parse_one(
        "A=\"a\" # c\n\
         B=plain\n\
         C=\"c\"\n\
         D=plain\n\
         E=\"e\"\n",
    );
    assert_eq!(vars.len(), 5, "variables were swallowed: {vars:?}");
    assert_eq!(vars["A"], "a");
    assert_eq!(vars["C"], "c");
    assert_eq!(vars["E"], "e");
}

/// Every quoting form, and the escape handling that distinguishes them.
#[test]
fn every_quote_style_accepts_a_trailing_comment() {
    let vars = parse_one(
        "A=\"double\" # c\n\
         B='single' # c\n\
         C=`backtick` # c\n\
         D=\"has \\\"escaped\\\" quotes\" # c\n\
         E=\"ends with a backslash\\\\\" # c\n",
    );
    assert_eq!(vars["A"], "double");
    assert_eq!(vars["B"], "single");
    assert_eq!(vars["C"], "backtick");
    assert_eq!(vars["D"], "has \"escaped\" quotes");
    assert_eq!(vars["E"], "ends with a backslash\\");
}

/// ⚠️ A `#` **inside** the quotes is part of the value, not a comment. The
/// closing quote is what ends it, so this must not change.
#[test]
fn a_hash_inside_the_quotes_stays_in_the_value() {
    let vars = parse_one("A=\"keep # this\" # drop this\nB='also # kept'\n");
    assert_eq!(vars["A"], "keep # this");
    assert_eq!(vars["B"], "also # kept");
}

/// A genuinely unterminated quote must still be an error — the fix must not
/// turn the loud failure into a silent one in the other direction.
#[test]
fn a_genuinely_unterminated_quote_is_still_an_error() {
    for content in ["A=\"no close\n", "A='no close\n", "A=\"\n"] {
        let r = Parser::new(ParserConfig {
            allow_multiline: false,
            ..Default::default()
        })
        .parse_content(content);
        assert!(r.is_err(), "accepted an unterminated quote: {content:?}");
    }
}

/// Multiline values still work — the quote simply has to close *somewhere*,
/// and when it does not on the opening line the value continues.
#[test]
fn a_real_multiline_value_still_spans_lines() {
    let vars = parse_one("A=\"line1\nline2\"\nB=after\n");
    assert_eq!(vars["A"], "line1\nline2");
    assert_eq!(vars["B"], "after");
}

/// Text after the closing quote that is not a comment is named rather than
/// discarded: silently dropping it would hide a missing quote or a stray
/// paste. It is counted, not quoted (B2 P0.3): it is often the rest of a secret.
#[test]
fn unexpected_text_after_the_closing_quote_is_reported() {
    let err = Parser::default()
        .parse_content("A=\"value\" junk here\n")
        .expect_err("should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("unexpected text after the closing \" (9 characters)"),
        "unhelpful error: {msg}"
    );
    assert!(!msg.contains("junk"), "the stray text leaked: {msg}");
}

/// The primitive, directly: where the quote closes, independent of what the
/// line ends with.
#[test]
fn closing_quote_index_finds_the_close_not_the_line_end() {
    assert_eq!(closing_quote_index("\"abc\" # c", '"'), Some(4));
    assert_eq!(closing_quote_index("\"abc\"", '"'), Some(4));
    assert_eq!(closing_quote_index("\"abc", '"'), None);
    assert_eq!(closing_quote_index("\"", '"'), None);
    // An escaped quote does not close a double-quoted value. The string is
    // `"a\"b"` — `"`(0) `a`(1) `\`(2) `"`(3) `b`(4) `"`(5) — so the close
    // is the one at 5, and the escaped one at 3 is skipped.
    assert_eq!(closing_quote_index("\"a\\\"b\"", '"'), Some(5));
    // ...but `'` and backtick are literal, so the first one does.
    assert_eq!(closing_quote_index("'a\\'b'", '\''), Some(3));
}

/// The headline: single quotes are now literal for `$`, as they already were
/// for backslashes. One rule instead of two.
#[test]
fn d21_unterminated_quote_is_rejected_so_the_fallback_is_live() {
    let content = "A=\"sk_live_51H8xQ2eZvKYlo2CabcdefghijklmnopQ\nB=other\n";
    let r = Parser::new(ParserConfig {
        allow_expansion: false,
        ..Default::default()
    })
    .parse_content_located(content);
    assert!(
        r.is_err(),
        "the parser accepted an unterminated quote — the scan fallback would be dead code"
    );
}

#[test]
fn single_quotes_do_not_expand() {
    let vars = parse_one("B=world\nA='cost $B'\nC=`cost $B`\n");
    assert_eq!(vars["A"], "cost $B");
    assert_eq!(vars["C"], "cost $B", "backticks too");
}

/// ⚠️ The failure that had no workaround. A token containing `${...}` used to
/// make the **whole file** fail to parse, with no way to write it.
#[test]
fn a_braced_reference_in_single_quotes_is_not_an_undefined_variable() {
    let vars = parse_one("TOKEN='pa${SOMETHING}ss'\n");
    assert_eq!(vars["TOKEN"], "pa${SOMETHING}ss");
}

/// ⚠️ The quieter failure, and the worse one: no error, and the value that
/// replaced it is another of your secrets.
#[test]
fn a_name_that_collides_with_a_key_is_no_longer_substituted() {
    let vars = parse_one("DB_PASS=hunter2\nAPI_KEY='prefix$DB_PASS'\n");
    assert_eq!(vars["API_KEY"], "prefix$DB_PASS");
    assert!(
        !vars["API_KEY"].contains("hunter2"),
        "leaked another secret"
    );
}

#[test]
fn double_quoted_and_unquoted_still_expand() {
    let vars = parse_one("B=world\nA=\"cost $B\"\nC=cost $B\n");
    assert_eq!(vars["A"], "cost world");
    assert_eq!(vars["C"], "cost world");
}

/// `\$` is a literal dollar in a double-quoted value, so expansion can be kept
/// for the rest of the string.
#[test]
fn an_escaped_dollar_is_literal() {
    let vars = parse_one("B=world\nA=\"\\$B costs $B\"\n");
    assert_eq!(vars["A"], "$B costs world");
}

/// ⚠️ The case the marker exists for, and the reason `\$` could not simply emit
/// a `$`.
///
/// `unescape_double` turns `\\` into `\` and would have turned `\$` into `$`,
/// making "literal dollar" and "literal backslash before a real expansion"
/// indistinguishable by the time expansion ran. This asserts the second still
/// works, which a naive fix would have broken.
#[test]
fn a_backslash_before_a_real_expansion_still_expands() {
    let vars = parse_one("B=world\nA=\"path\\\\$B\"\n");
    assert_eq!(vars["A"], "path\\world");
}

/// The internal marker must never reach a caller, by any path.
#[test]
fn the_literal_dollar_marker_never_escapes() {
    for content in [
        "B=w\nA=\"\\$B\"\n",
        "A='\\$B'\n",
        "A=\"\\$\"\n",
        "B=w\nA=\"\\$B and $B and \\$B\"\n",
    ] {
        for (k, v) in parse_one(content) {
            assert!(
                !v.contains(LITERAL_DOLLAR),
                "{k} leaked the marker from {content:?}: {v:?}"
            );
        }
    }
}

/// ⚠️ The path `expand_all` does not cover. With expansion disabled nothing
/// consumes the markers, so the strip has to be unconditional.
#[test]
fn the_marker_is_stripped_even_with_expansion_disabled() {
    let parser = Parser::new(ParserConfig {
        allow_expansion: false,
        ..Default::default()
    });
    let vars = parser
        .parse_content("B=world\nA=\"\\$B\"\n")
        .expect("parses");
    assert!(!vars["A"].contains(LITERAL_DOLLAR), "{:?}", vars["A"]);
    assert_eq!(vars["A"], "$B");
}

// ─── D17: expansion errors name a real line ──────────────────────────────

fn expect_err(content: &str) -> String {
    Parser::default()
        .parse_content(content)
        .expect_err("should fail")
        .to_string()
}

/// ⚠️ It said **line 0** for every one of these — not a line in any file, so it
/// pointed at nothing, in exactly the case where it matters: a large `.env` that
/// suddenly will not parse.
#[test]
fn an_undefined_variable_names_the_line_it_was_written_on() {
    let err = expect_err("A=one\nB=two\nC=hello ${NOPE}\n");
    assert!(err.contains("line 3"), "{err}");
    assert!(!err.contains("line 0"), "{err}");
}

/// A multiline value is reported at the line where it **opened**, matching how
/// an unterminated string is reported — one rule for "where is this value".
#[test]
fn a_multiline_value_reports_the_line_it_opened_on() {
    let err = expect_err("A=one\nC=\"hello\n${NOPE}\"\n");
    assert!(err.contains("line 2"), "{err}");
}

/// The circular-expansion error shared the same hardcoded zero.
#[test]
fn a_circular_reference_names_a_line_too() {
    let err = expect_err("A=${B}\nB=${A}\n");
    assert!(err.contains("line 1"), "{err}");
    assert!(!err.contains("line 0"), "{err}");
}

/// A multiline single-quoted value is literal in every respect — escapes and
/// expansion alike.
#[test]
fn a_multiline_single_quoted_value_is_fully_literal() {
    let vars = parse_one("B=world\nA='line $B\nsecond \\t line'\n");
    assert_eq!(vars["A"], "line $B\nsecond \\t line");
}

// ── Basic parsing ─────────────────────────────────────────────────────────

#[test]
fn test_basic_key_value() {
    let p = Parser::default();
    let vars = p.parse_content("KEY1=value1\nKEY2=value2").unwrap();
    assert_eq!(vars["KEY1"], "value1");
    assert_eq!(vars["KEY2"], "value2");
}

#[test]
fn test_empty_lines_and_comments_skipped() {
    let p = Parser::default();
    let vars = p.parse_content("# comment\n\nKEY=val\n# another").unwrap();
    assert_eq!(vars.len(), 1);
    assert_eq!(vars["KEY"], "val");
}

#[test]
fn test_empty_value() {
    let p = Parser::default();
    let vars = p.parse_content("KEY=").unwrap();
    assert_eq!(vars["KEY"], "");
}

#[test]
fn test_whitespace_around_equals() {
    let p = Parser::default();
    let vars = p.parse_content("  KEY1  =  value1  ").unwrap();
    assert_eq!(vars["KEY1"], "value1");
}

// ── export prefix ─────────────────────────────────────────────────────────

#[test]
fn test_export_prefix() {
    let p = Parser::default();
    let vars = p
        .parse_content("export KEY1=value1\nexport KEY2=value2")
        .unwrap();
    assert_eq!(vars["KEY1"], "value1");
    assert_eq!(vars["KEY2"], "value2");
}

// ── Quote styles ──────────────────────────────────────────────────────────

#[test]
fn test_double_quoted() {
    let p = Parser::default();
    let vars = p.parse_content(r#"KEY="hello world""#).unwrap();
    assert_eq!(vars["KEY"], "hello world");
}

#[test]
fn test_single_quoted() {
    let p = Parser::default();
    let vars = p.parse_content("KEY='hello world'").unwrap();
    assert_eq!(vars["KEY"], "hello world");
}

#[test]
fn test_backtick_quoted() {
    let p = Parser::default();
    let vars = p.parse_content("KEY=`hello world`").unwrap();
    assert_eq!(vars["KEY"], "hello world");
}

#[test]
fn test_empty_double_quoted() {
    let p = Parser::default();
    let vars = p.parse_content(r#"KEY="""#).unwrap();
    assert_eq!(vars["KEY"], "");
}

// ── Escape sequences ──────────────────────────────────────────────────────

#[test]
fn test_escape_newline_tab() {
    let p = Parser::default();
    let vars = p.parse_content(r#"KEY="line1\nline2\ttab""#).unwrap();
    assert_eq!(vars["KEY"], "line1\nline2\ttab");
}

#[test]
fn test_escape_quote_and_backslash() {
    let p = Parser::default();
    let vars = p.parse_content(r#"KEY="He said \"hi\"\\path""#).unwrap();
    assert_eq!(vars["KEY"], r#"He said "hi"\path"#);
}

#[test]
fn test_escape_single_quote_in_double() {
    let p = Parser::default();
    let vars = p.parse_content(r#"KEY="it\'s a test""#).unwrap();
    assert_eq!(vars["KEY"], "it's a test");
}

#[test]
fn test_single_quoted_no_escaping() {
    // Backslashes inside single quotes are literal.
    let p = Parser::default();
    let vars = p.parse_content(r"KEY='no\nescape'").unwrap();
    assert_eq!(vars["KEY"], r"no\nescape");
}

// ── Inline comments ───────────────────────────────────────────────────────

#[test]
fn test_inline_comment_stripped() {
    let p = Parser::default();
    let vars = p.parse_content("PORT=8080 # web server").unwrap();
    assert_eq!(vars["PORT"], "8080");
}

#[test]
fn test_inline_comment_disabled() {
    let p = Parser::new(ParserConfig {
        allow_inline_comments: false,
        ..Default::default()
    });
    let vars = p.parse_content("PORT=8080 # web server").unwrap();
    assert_eq!(vars["PORT"], "8080 # web server");
}

#[test]
fn test_hash_inside_double_quotes_preserved() {
    // # inside a quoted string must NOT be treated as a comment.
    let p = Parser::default();
    let vars = p.parse_content(r#"KEY="value#notacomment""#).unwrap();
    assert_eq!(vars["KEY"], "value#notacomment");
}

// ── Multiline values ──────────────────────────────────────────────────────

#[test]
fn test_multiline_double_quoted() {
    let p = Parser::default();
    let content = "KEY=\"line one\nline two\nline three\"";
    let vars = p.parse_content(content).unwrap();
    assert_eq!(vars["KEY"], "line one\nline two\nline three");
}

/// B5. An escaped `\"` inside a value that spans lines used to end the value,
/// because the closing-quote check was a bare `ends_with('"')` and the final
/// character of `"line1 \"q\"` *is* a quote. The next line then became a
/// stray entry: `Invalid format at line 2: missing '=' separator`.
#[test]
fn escaped_quote_does_not_end_a_multiline_value() {
    let p = Parser::default();
    let vars = p
        .parse_content("B=\"line1 \\\"q\\\"\nline2\"\nAFTER=ok\n")
        .unwrap();
    assert_eq!(vars["B"], "line1 \"q\"\nline2");
    // Parsing must continue past it — the original symptom was the *next*
    // line being misread, not the value itself.
    assert_eq!(vars["AFTER"], "ok");
}

/// The same value, on one line and across two, must mean the same thing.
///
/// ⚠️ It did not. The multiline branch inserted its accumulated text directly
/// and never reached `parse_value`, so `"x\ty"` produced a tab on one line and
/// a literal backslash-`t` across two — and `\"` was inexpressible in a
/// multiline value at all, which is the other half of B5.
#[test]
fn escape_sequences_mean_the_same_across_lines() {
    let p = Parser::default();

    let one = p.parse_content("V=\"x\\ty and \\\"q\\\"\"\n").unwrap();
    let two = p.parse_content("V=\"x\\ty and \\\"q\\\"\nz\"\n").unwrap();

    assert_eq!(one["V"], "x\ty and \"q\"");
    assert_eq!(two["V"], "x\ty and \"q\"\nz");
    // The spanning value is the single-line one plus a newline and `z`.
    assert_eq!(two["V"], format!("{}\nz", one["V"]));
}

/// Single quotes are literal by definition, so a backslash inside them
/// survives verbatim whether or not the value spans lines. This is the escape
/// hatch the README points at for content that must not be unescaped.
#[test]
fn single_quoted_multiline_stays_literal() {
    let p = Parser::default();
    let vars = p.parse_content("S='a\\nb\nsecond'\n").unwrap();
    // A literal backslash and `n`, then a real newline from the line break.
    assert_eq!(vars["S"], "a\\nb\nsecond");
}

/// A PEM key is the common case and contains no backslashes, so the escape
/// change above must leave it byte-identical.
#[test]
fn pem_key_is_unaffected_by_escape_handling() {
    let p = Parser::default();
    let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADAN\n-----END PRIVATE KEY-----";
    let vars = p.parse_content(&format!("P=\"{pem}\"\nNEXT=1\n")).unwrap();
    assert_eq!(vars["P"], pem);
    assert_eq!(vars["NEXT"], "1");
}

#[test]
fn test_multiline_disabled_returns_error() {
    let p = Parser::new(ParserConfig {
        allow_multiline: false,
        ..Default::default()
    });
    // Without multiline support an unclosed quote is an error.
    let result = p.parse_content("KEY=\"unclosed");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ParseError::UnterminatedString { .. }
    ));
}

#[test]
fn test_unterminated_string_eof() {
    // File ends while still inside a multiline value.
    let p = Parser::default();
    let result = p.parse_content("KEY=\"starts but never ends");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ParseError::UnterminatedString { .. }
    ));
}

// ── Key validation ────────────────────────────────────────────────────────

#[test]
fn test_key_starting_with_digit_rejected() {
    let p = Parser::default();
    let result = p.parse_content("1KEY=value");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ParseError::InvalidKey { .. }));
}

#[test]
fn test_key_with_hyphen_rejected() {
    let p = Parser::default();
    let result = p.parse_content("MY-KEY=value");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ParseError::InvalidKey { .. }));
}

#[test]
fn test_key_with_space_rejected() {
    let p = Parser::default();
    let result = p.parse_content("MY KEY=value");
    assert!(result.is_err());
}

#[test]
fn test_mixed_case_key_accepted_by_default() {
    let p = Parser::default();
    let vars = p.parse_content("MyKey=value").unwrap();
    assert_eq!(vars["MyKey"], "value");
}

#[test]
fn test_strict_mode_rejects_lowercase() {
    let p = Parser::new(ParserConfig {
        strict: true,
        ..Default::default()
    });
    let result = p.parse_content("lowercase=value");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ParseError::InvalidKey { .. }));
}

#[test]
fn test_strict_mode_accepts_uppercase() {
    let p = Parser::new(ParserConfig {
        strict: true,
        ..Default::default()
    });
    let vars = p.parse_content("UPPER_CASE=value").unwrap();
    assert_eq!(vars["UPPER_CASE"], "value");
}

// ── Variable expansion ────────────────────────────────────────────────────

#[test]
fn test_expansion_brace_syntax() {
    let p = Parser::default();
    let vars = p
        .parse_content("BASE=http://localhost\nURL=${BASE}/api")
        .unwrap();
    assert_eq!(vars["URL"], "http://localhost/api");
}

/// T-SHARE-22 / T-024: this test sat commented out while the bug it
/// describes shipped (I-141). Re-enabled with the fix.
#[test]
fn test_expansion_bare_syntax() {
    let p = Parser::default();
    let vars = p
        .parse_content("BASE=http://localhost\nURL=$BASE/api")
        .unwrap();
    assert_eq!(vars["URL"], "http://localhost/api");
}

/// T-SHARE-21 (I-141): every character that ends a bare `$NAME` stays in
/// the value. On 0.10.1 all of these but end-of-string lost it.
#[test]
fn bare_reference_keeps_the_character_that_ends_it() {
    let p = Parser::default();
    let cases = [
        ("$B/data", "app/data"),
        ("$B:8080", "app:8080"),
        ("$B.example.com", "app.example.com"),
        ("a$B!", "aapp!"),
        ("$B-x", "app-x"),
        ("$B,x", "app,x"),
        ("$B;x", "app;x"),
        ("($B)", "(app)"),
        ("$B", "app"),
        ("$B$C", "appcee"),
        ("${B}x", "appx"),
        ("$BX", "$BX"),
        ("$NOPE/bin", "$NOPE/bin"),
        ("$5/x", "$5/x"),
        ("$/x", "$/x"),
        ("x$", "x$"),
    ];
    for (raw, want) in cases {
        let vars = p
            .parse_content(&format!("B=app\nC=cee\nV={raw}\n"))
            .unwrap_or_else(|e| panic!("{raw}: {e}"));
        assert_eq!(vars["V"], want, "V={raw}");
    }
}

/// T-SHARE-21: a space ends a reference too, inside double quotes (an
/// unquoted value would lose its trailing part to comment and space rules,
/// which is a separate matter).
#[test]
fn bare_reference_keeps_a_following_space_in_double_quotes() {
    let p = Parser::default();
    let vars = p
        .parse_content("B=app\nV=\"$B rest\"\nQ=\"$B'\"\n")
        .unwrap();
    assert_eq!(vars["V"], "app rest");
    assert_eq!(vars["Q"], "app'");
}

/// T-SHARE-21: an escaped dollar is not a reference, whatever follows.
#[test]
fn escaped_dollar_is_not_a_reference() {
    let p = Parser::default();
    let vars = p.parse_content("B=app\nV=\"\\$B/x\"\n").unwrap();
    assert_eq!(vars["V"], "$B/x");
}

/// T-SHARE-22: chained bare references, each with a terminator.
#[test]
fn bare_references_chain() {
    let p = Parser::default();
    let vars = p
        .parse_content("HOST=localhost\nBASE=http://$HOST:8080\nAPI=$BASE/api\n")
        .unwrap();
    assert_eq!(vars["API"], "http://localhost:8080/api");
}

/// T-SHARE-22: a cycle through a bare reference is still caught.
#[test]
fn bare_reference_cycle_is_an_error() {
    let p = Parser::default();
    let err = p.parse_content("A=$B/x\nB=$A/y\n").unwrap_err();
    assert!(
        matches!(err, ParseError::CircularExpansion { .. }),
        "{err:?}"
    );
}

#[test]
fn test_expansion_chained() {
    let p = Parser::default();
    let content = "BASE=http://localhost\nAPI=${BASE}/api\nFULL=${API}/v1";
    let vars = p.parse_content(content).unwrap();
    assert_eq!(vars["FULL"], "http://localhost/api/v1");
}

#[test]
fn test_expansion_disabled() {
    let p = Parser::new(ParserConfig {
        allow_expansion: false,
        ..Default::default()
    });
    let vars = p.parse_content("KEY=${OTHER}").unwrap();
    assert_eq!(vars["KEY"], "${OTHER}");
}

#[test]
fn test_undefined_brace_var_errors() {
    let p = Parser::default();
    let result = p.parse_content("KEY=${UNDEFINED}");
    assert!(result.is_err());
    match result.unwrap_err() {
        ParseError::UndefinedVariable { var, .. } => assert_eq!(var, "UNDEFINED"),
        e => panic!("expected UndefinedVariable, got {e:?}"),
    }
}

#[test]
fn test_undefined_bare_var_kept_literal() {
    // Bare $VAR references to undefined variables are kept as-is
    // (shell variables like $HOME are common in .env files).
    let p = Parser::default();
    let vars = p.parse_content("KEY=$UNDEFINED_BARE").unwrap();
    assert_eq!(vars["KEY"], "$UNDEFINED_BARE");
}

#[test]
fn test_circular_expansion_detected() {
    let p = Parser::default();
    let result = p.parse_content("A=${B}\nB=${A}");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ParseError::CircularExpansion { .. }
    ));
}

#[test]
fn test_expansion_depth_limit() {
    let p = Parser::new(ParserConfig {
        max_expansion_depth: 2,
        ..Default::default()
    });
    // Three levels of nesting exceeds depth 2.
    let content = "A=base\nB=${A}\nC=${B}\nD=${C}";
    let result = p.parse_content(content);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ParseError::ExpansionDepthExceeded { .. }
    ));
}
// New test demonstrating order preservation:
#[test]
fn test_key_order_preserved() {
    let p = Parser::default();
    let content = "Z=last\nA=first\nM=middle";
    let vars = p.parse_content(content).unwrap();

    let keys: Vec<_> = vars.keys().collect();
    assert_eq!(keys, vec!["Z", "A", "M"]); // Insertion order preserved
}

// ── Real-world integration ────────────────────────────────────────────────

#[test]
fn test_real_world_dotenv() {
    let p = Parser::default();
    let content = r#"
# Database
DATABASE_URL=postgresql://user:pass@localhost:5432/mydb

# Django settings
SECRET_KEY="django-insecure-abc123"
DEBUG=True
ALLOWED_HOSTS=localhost,127.0.0.1 # dev only

# AWS
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
AWS_REGION=us-east-1

# Computed
API_BASE=http://localhost:8000
API_V1=${API_BASE}/api/v1

# export style
export LEGACY_KEY=legacy_value
"#;
    let vars = p.parse_content(content).unwrap();

    assert_eq!(
        vars["DATABASE_URL"],
        "postgresql://user:pass@localhost:5432/mydb"
    );
    assert_eq!(vars["SECRET_KEY"], "django-insecure-abc123");
    assert_eq!(vars["DEBUG"], "True");
    assert_eq!(vars["ALLOWED_HOSTS"], "localhost,127.0.0.1");
    assert_eq!(vars["AWS_REGION"], "us-east-1");
    assert_eq!(vars["API_V1"], "http://localhost:8000/api/v1");
    assert_eq!(vars["LEGACY_KEY"], "legacy_value");
    assert_eq!(vars.len(), 10);
}

// ─── B1: `Document` ─────────────────────────────────────────────────────────

/// The strict reading of a document must be exactly what the parser returned
/// before `Document` existed, error included. `legacy.rs` is that parser,
/// frozen; this compares the two over every combination of up to three lines
/// drawn from fragments chosen to hit each branch, under each setting that
/// changes a branch.
#[test]
fn b1_the_document_reads_exactly_as_the_old_parser_did() {
    const FRAGMENTS: &[&str] = &[
        "",
        "   ",
        "# comment",
        "  # A=1",
        "A=1",
        "A=2",
        "B=x # c",
        "export C=c",
        // ⚠️ Not `exportD=d`: the old parser read it as `D`, and B2 P0.4 reads
        // it as `exportD` on purpose. `p0_4_*` and edge_22 pin the new reading.
        "a_lower=v",
        "1BAD=v",
        "my-key=v",
        "no separator",
        "=nokey",
        "E=\"q\"",
        "E='lit $A'",
        "F=`tick`",
        "G=\"open",
        "close\"",
        "H='open",
        "end'",
        "I=\"esc \\\" still open",
        "J=\"a\" trailing",
        "K=${A}",
        "L=${NOPE}",
        "L=fine",
        "M=$A/x",
        "N=\\$A",
        "O=\"\\$A\"",
        "P=${P}",
        "Q=${R}",
        "R=${Q}",
        "COLOR=#ff0000",
        "S=\"x\\ty\"",
    ];
    let configs = [
        ParserConfig::default(),
        ParserConfig {
            allow_expansion: false,
            ..Default::default()
        },
        ParserConfig {
            strict: true,
            ..Default::default()
        },
        ParserConfig {
            allow_multiline: false,
            ..Default::default()
        },
        ParserConfig {
            allow_inline_comments: false,
            trim_values: false,
            ..Default::default()
        },
    ];

    let mut files: Vec<String> = Vec::new();
    for a in FRAGMENTS {
        files.push(format!("{a}\n"));
        for b in FRAGMENTS {
            files.push(format!("{a}\n{b}\n"));
            for c in FRAGMENTS {
                files.push(format!("{a}\n{b}\n{c}"));
            }
        }
    }
    // Every `.env`-like fixture the repository holds.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut dirs = vec![root.join("tests")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                dirs.push(path);
            } else if name.starts_with(".env") || name.ends_with(".env") {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    files.push(text);
                }
            }
        }
    }

    let show = |r: ParseResult<(IndexMap<String, String>, IndexMap<String, usize>)>| match r {
        Ok(ok) => format!("{ok:?}"),
        Err(e) => format!("Err({e:?})"),
    };
    let mut differ = Vec::new();
    for config in &configs {
        let p = Parser::new(config.clone());
        for f in &files {
            // ⚠️ The one intended difference (v0.12.0 B2 P0.1): a leading BOM
            // is skipped, where the old parser read it into the first key.
            let old = show(p.legacy_parse_content_located(f.strip_prefix('\u{feff}').unwrap_or(f)));
            let new = show(p.parse_content_located(f));
            if old != new {
                differ.push(format!("{config:?}\n{f:?}\n  old {old}\n  new {new}"));
            }
        }
    }
    assert!(
        differ.is_empty(),
        "{} of {} readings differ; first:\n{}",
        differ.len(),
        files.len() * configs.len(),
        differ[0]
    );
}

#[test]
fn b1_every_problem_is_reported_with_its_line() {
    let doc = Parser::default().parse_document("A=1\nnot a line\nB=2\n1BAD=x\nC=\"x\" junk\nD=4\n");
    let lines: Vec<usize> = doc.problems.iter().map(|p| p.span.line).collect();
    assert_eq!(lines, [2, 4, 5]);
    assert!(matches!(
        doc.problems[1].error,
        ParseError::InvalidKey { .. }
    ));
    assert_eq!(doc.problems[0].raw, "not a line");
    // Reading carried on around them.
    let keys: Vec<&str> = doc.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, ["A", "B", "D"]);
}

#[test]
fn b1_an_unterminated_quote_spans_to_the_end_of_the_file() {
    let doc = Parser::default().parse_document("A=1\nB=\"open\nC=3\nD=4\n");
    assert_eq!(doc.entries.len(), 1);
    assert_eq!(doc.problems.len(), 1);
    let p = &doc.problems[0];
    assert!(matches!(
        p.error,
        ParseError::UnterminatedString { line: 2 }
    ));
    assert_eq!(
        p.span,
        Span {
            line: 2,
            end_line: 4
        }
    );
    assert_eq!(p.raw, "B=\"open\nC=3\nD=4");
}

#[test]
fn b1_duplicates_are_kept_and_the_last_one_counts() {
    let doc = Parser::default().parse_document("K=sk_live_x\nOTHER=1\nK=\n");
    let ks: Vec<(&str, &str, usize)> = doc
        .entries
        .iter()
        .map(|e| (e.key.as_str(), e.value.as_str(), e.span.line))
        .collect();
    assert_eq!(ks, [("K", "sk_live_x", 1), ("OTHER", "1", 2), ("K", "", 3)]);
    // First position, last value and line: what the parser always returned.
    assert_eq!(format!("{:?}", doc.vars()), r#"{"K": "", "OTHER": "1"}"#);
    assert_eq!(format!("{:?}", doc.lines()), r#"{"K": 3, "OTHER": 2}"#);
}

#[test]
fn b1_entries_carry_their_form_and_comments_are_kept() {
    let doc = Parser::default()
        .parse_document("# top\nexport A='x'\nB=\"l1\nl2\"\n  #   indented\nC=plain\n");
    let a = &doc.entries[0];
    assert!(a.exported);
    assert_eq!((a.quote, a.raw.as_str()), (Some('\''), "export A='x'"));
    let b = &doc.entries[1];
    assert_eq!(
        b.span,
        Span {
            line: 3,
            end_line: 4
        }
    );
    assert_eq!(
        (b.value.as_str(), b.raw.as_str()),
        ("l1\nl2", "B=\"l1\nl2\"")
    );
    assert_eq!(doc.entries[2].quote, None);
    let comments: Vec<(usize, &str)> = doc
        .comments
        .iter()
        .map(|c| (c.line, c.text.as_str()))
        .collect();
    assert_eq!(comments, [(1, " top"), (5, "   indented")]);
}

#[test]
fn b1_an_expansion_error_keeps_its_entry_and_names_its_line() {
    let doc = Parser::default().parse_document("A=1\nB=${NOPE}\nC=${A}\nD=${ALSO_NOPE}\n");
    let lines: Vec<usize> = doc.problems.iter().map(|p| p.span.line).collect();
    assert_eq!(lines, [2, 4]);
    assert_eq!(doc.vars()["B"], "${NOPE}");
    assert_eq!(doc.vars()["C"], "1");
}

/// A leading UTF-8 BOM is skipped and recorded (B2 P0.1). It used to be read
/// into the first key, and the file was refused with an invalid key whose
/// fault was invisible. Only a leading one: a BOM further down is still text.
#[test]
fn p0_1_a_leading_bom_is_skipped_and_recorded() {
    let doc = Parser::default().parse_document("\u{feff}K=v\nL=w\n");
    assert!(doc.bom);
    assert!(doc.is_clean(), "{:?}", doc.problems);
    assert_eq!(doc.entries[0].key, "K");
    assert_eq!(doc.entries[0].span, Span::line(1));
    assert_eq!(
        Parser::default().parse_content("\u{feff}K=v\n").unwrap()["K"],
        "v"
    );

    assert!(!Parser::default().parse_document("K=v\n").bom);
    let later = Parser::default().parse_document("K=v\n\u{feff}L=w\n");
    assert!(!later.bom);
    assert_eq!(
        later.problems.len(),
        1,
        "a BOM past the start is still refused"
    );
}

/// `${VAR:-d}` and its five siblings read as Docker Compose reads them (B2 P0.2).
/// Every one was an undefined-variable error before, so no working file changes.
/// The values below were measured with Compose v5.3.1; the conformance fixture
/// `edge_21_default_family` pins the rest.
#[test]
fn p0_2_default_and_alternative_forms_read_as_compose_does() {
    let vars = Parser::default()
        .parse_content("X=1\nE=\nB=bee\nA=${E:-d}\nC=${E-d}\nD=${E:+alt}\nF=${X+alt}\nG=${NOPE:-$B}/x\nH='${NOPE:-d}'\n")
        .unwrap();
    assert_eq!(vars["A"], "d", "`:-` treats empty as unset");
    assert_eq!(vars["C"], "", "`-` does not");
    assert_eq!(vars["D"], "");
    assert_eq!(vars["F"], "alt");
    assert_eq!(vars["G"], "bee/x", "the default is expanded");
    assert_eq!(vars["H"], "${NOPE:-d}", "single quotes stay literal");

    let off = Parser::new(ParserConfig {
        allow_expansion: false,
        ..ParserConfig::default()
    });
    assert_eq!(
        off.parse_content("A=${NOPE:-d}").unwrap()["A"],
        "${NOPE:-d}"
    );
}

/// `${VAR:?message}` refuses the file with the message, as Compose does, and a
/// `${` inside a default stays an error (Compose reads it; evnx does not yet).
#[test]
fn p0_2_a_required_variable_names_itself_and_its_message() {
    let err = Parser::default()
        .parse_content("X=1\nR=${NOPE:?must be set}\n")
        .unwrap_err();
    assert!(
        matches!(&err, ParseError::RequiredVariable { line: 2, var, message }
            if var == "NOPE" && message == "must be set"),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "Required variable at line 2: ${NOPE} must be set"
    );
    let empty = Parser::default().parse_content("E=\nR=${E:?}").unwrap_err();
    assert!(
        empty.to_string().ends_with("${E} is not set or is empty"),
        "{empty}"
    );
    assert_eq!(
        Parser::default().parse_content("E=\nR=${E?}").unwrap()["R"],
        ""
    );

    assert!(matches!(
        Parser::default()
            .parse_content("B=1\nA=${NOPE:-${B}}")
            .unwrap_err(),
        ParseError::UndefinedVariable { .. }
    ));
}

/// `export` is a keyword only when whitespace follows it (B2 P0.4). It was
/// stripped as a bare prefix, so `exporter=1` became `er=1`, silently. Measured
/// with Compose v5.3.1: it keeps `exporter`, `exports_dir` and `exportD`.
#[test]
fn p0_4_export_is_a_keyword_only_before_whitespace() {
    let doc = Parser::default().parse_document("exporter=1\nexports_dir=/x\nexport\tTAB=3\n");
    let keys: Vec<(&str, bool)> = doc
        .entries
        .iter()
        .map(|e| (e.key.as_str(), e.exported))
        .collect();
    assert_eq!(
        keys,
        [("exporter", false), ("exports_dir", false), ("TAB", true)]
    );
    assert!(doc.is_clean(), "{:?}", doc.problems);
    assert_eq!(line_opens_multiline("exporter=\"a"), Some('"'));
}

/// `export =v` and a bare `export` say what is wrong rather than `Invalid key: ''`.
#[test]
fn p0_4_export_with_no_key_says_so() {
    for content in ["export =v\n", "export\n", "export   \n"] {
        let err = Parser::default().parse_content(content).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Invalid format at line 1: `export` is followed by no key",
            "{content:?}"
        );
    }
}

/// A carriage return that ends no CRLF line is found, wherever it is (B2 P0.5).
/// The reading does not change; `validate` and `doctor` warn from this.
#[test]
fn p0_5_a_lone_carriage_return_is_found_and_crlf_is_not() {
    let keys = |content: &str| -> Vec<String> {
        Parser::default()
            .parse_document(content)
            .lone_carriage_returns()
            .map(|e| e.key.clone())
            .collect()
    };
    assert_eq!(keys("K=v\rL=w\r"), ["K"]);
    assert_eq!(keys("A=1\nQ=\"x\ry\"\n"), ["Q"]);
    assert!(
        keys("A=1\r\nB=\"x\r\ny\"\r\n").is_empty(),
        "CRLF is a line ending"
    );
    assert!(
        keys("E=\"a\\rb\"\n").is_empty(),
        "an escaped \\r is text, not a CR"
    );
}

/// A NUL byte in a value is found (B2 P0.6). The reading does not change;
/// `validate` and `doctor` report it as an error.
#[test]
fn p0_6_a_nul_byte_in_a_value_is_found() {
    let doc = Parser::default().parse_document("A=1\nK=a\0b\nQ=\"x\0y\"\n");
    let keys: Vec<&str> = doc.nul_bytes().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, ["K", "Q"]);
    assert_eq!(
        Parser::default()
            .parse_document("A=1\n")
            .nul_bytes()
            .count(),
        0
    );
}
