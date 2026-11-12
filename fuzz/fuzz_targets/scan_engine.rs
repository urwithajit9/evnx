//! The scan engine on one in-memory file, down each path `evnx scan` takes:
//! `.env`, plain text, and the structured readers (JSON, YAML, TOML) and the
//! Jupyter notebook reader.
//!
//! It must never panic and never hang, whatever the bytes. The first byte picks
//! the path; the rest is the file.
#![no_main]

use evnx::core::secretscan::engine::{self, Context, Format, Kind};
use evnx::core::secretscan::DetectorRegistry;
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

fn registry() -> &'static DetectorRegistry {
    static REGISTRY: OnceLock<DetectorRegistry> = OnceLock::new();
    REGISTRY.get_or_init(DetectorRegistry::new)
}

fn scan(text: &str, kind: Kind, path: &str) {
    let ctx = Context {
        path,
        kind,
        document: None,
        leaves: None,
        registry: registry(),
    };
    let _ = engine::scan_text(text, &ctx);
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(rest);

    match mode % 6 {
        0 => scan(&text, Kind::Dotenv, ".env"),
        1 => scan(&text, Kind::Text, "notes.txt"),
        2 => scan(&text, Kind::Structured(Format::Json), "config.json"),
        3 => scan(&text, Kind::Structured(Format::Yaml), "config.yaml"),
        4 => scan(&text, Kind::Structured(Format::Toml), "config.toml"),
        _ => {
            // A notebook is read cell by cell, each chunk as text, as the
            // runner does; a file that is not a notebook is read as text.
            match engine::notebook_chunks(&text) {
                Some(chunks) => {
                    for chunk in chunks {
                        scan(&chunk.text, Kind::Text, "analysis.ipynb");
                    }
                }
                None => scan(&text, Kind::Text, "analysis.ipynb"),
            }
        }
    }

    // The leaf walk is reachable on its own too (a caller may pass `leaves`).
    for format in [Format::Json, Format::Yaml, Format::Toml] {
        let _ = engine::leaves(&text, format);
    }
});
