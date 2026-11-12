//! The `.env` parser on arbitrary text, under every configuration evnx uses.
//!
//! It must never panic and never hang. A bad line is a `Problem` in the
//! document, not a crash.
#![no_main]

use evnx::core::parser::{is_env_key, is_valid_key, strip_export, Parser, ParserConfig};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Invalid UTF-8 is replaced, not skipped, so the parser still sees the
    // bytes around it.
    let text = String::from_utf8_lossy(data);

    // The first byte picks the configuration, so one corpus covers them all.
    let flags = data.first().copied().unwrap_or(0);
    let config = ParserConfig {
        allow_expansion: flags & 1 != 0,
        strict: flags & 2 != 0,
        allow_inline_comments: flags & 4 != 0,
        max_expansion_depth: usize::from(flags >> 5),
        ..ParserConfig::default()
    };
    let parser = Parser::new(config);

    let doc = parser.parse_document(&text);
    let _ = doc.into_result();
    let _ = parser.parse_content(&text);
    let _ = parser.parse_content_located(&text);
    let _ = Parser::default().parse_content(&text);

    for line in text.lines().take(64) {
        let _ = strip_export(line);
        if let Some((key, _)) = line.split_once('=') {
            let _ = is_valid_key(key);
            let _ = is_env_key(key);
        }
    }
});
