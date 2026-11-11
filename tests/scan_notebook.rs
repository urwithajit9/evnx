//! `evnx scan` on Jupyter notebooks (F14-6, OW-4).
//!
//! A notebook is JSON, so a secret in a cell is behind string escapes. Cells
//! and outputs are read unescaped, and a finding is located by cell:
//! `nb.ipynb:LINE (cells[N].source)`, LINE being within that cell.
//!
//! ⚠️ Secret-shaped values are assembled at run time, so push protection never
//! sees one in the source.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn aws() -> String {
    format!("AKIA{}", "IOSFODNN7EXAMPLQ")
}

fn stripe() -> String {
    format!("sk_live_{}", "51Habc1234567890ABCDEFGH")
}

fn notebook(cells: serde_json::Value) -> String {
    serde_json::to_string_pretty(&json!({
        "cells": cells, "metadata": {}, "nbformat": 4, "nbformat_minor": 5
    }))
    .unwrap()
}

fn scan_text(body: &str) -> serde_json::Value {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join("nb.ipynb"), body).unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn locations(doc: &serde_json::Value) -> Vec<String> {
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["location"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_secret_in_a_code_cell_a_markdown_cell_and_an_output_is_found() {
    let (a, s) = (aws(), stripe());
    let doc = scan_text(&notebook(json!([
        {"cell_type": "code", "metadata": {}, "execution_count": 1, "outputs": [],
         "source": ["import os\n", format!("key = \"{a}\"\n")]},
        {"cell_type": "markdown", "metadata": {}, "source": format!("Use `{s}` to log in")},
        {"cell_type": "code", "metadata": {}, "execution_count": 2,
         "source": "print(1)",
         "outputs": [
            {"output_type": "stream", "name": "stdout", "text": ["ok\n", format!("AWS={a}\n")]}
         ]},
        {"cell_type": "code", "metadata": {}, "execution_count": 3, "source": "x",
         "outputs": [
            {"output_type": "execute_result", "execution_count": 3, "metadata": {},
             "data": {"text/plain": [format!("'{a}'")]}},
            {"output_type": "error", "ename": "E", "evalue": "v",
             "traceback": ["line one", format!("token {s}")]}
         ]},
    ])));
    let locs = locations(&doc);
    for want in [
        "./nb.ipynb:2 (cells[0].source)",
        "./nb.ipynb:1 (cells[1].source)",
        "./nb.ipynb:2 (cells[2].outputs[0].text)",
        "./nb.ipynb:1 (cells[3].outputs[0].data.text/plain)",
        "./nb.ipynb:2 (cells[3].outputs[1].traceback)",
    ] {
        assert!(
            locs.iter().any(|l| l.starts_with(want)),
            "{want} not in {locs:?}"
        );
    }
    assert_eq!(doc["summary"]["total"], 5, "{locs:?}");
    assert_eq!(doc["files"][0]["read_as"], "structured");
}

#[test]
fn a_secret_split_by_json_escaping_is_found_once() {
    // In the file: `\"` around the key, `\n` between lines and, for the PEM-less
    // case, the value in the middle of an array element boundary.
    let a = aws();
    let raw = format!(
        r#"{{"cells":[{{"cell_type":"code","metadata":{{}},"outputs":[],"source":["x = 1\n","token = \"{a}\"\n","y = 2"]}}],"metadata":{{}},"nbformat":4,"nbformat_minor":5}}"#
    );
    assert!(raw.contains("\\\""), "fixture must hold a JSON escape");
    let doc = scan_text(&raw);
    let locs = locations(&doc);
    assert_eq!(locs.len(), 1, "{locs:?}");
    assert!(
        locs[0].starts_with("./nb.ipynb:2 (cells[0].source)"),
        "{locs:?}"
    );
    // The shown value is not polluted by the escape.
    assert!(!doc["findings"][0]["preview"]
        .as_str()
        .unwrap_or("")
        .contains('\\'));
}

#[test]
fn a_value_in_a_cell_and_echoed_in_its_output_is_two_findings_not_four() {
    let a = aws();
    let doc = scan_text(&notebook(json!([
        {"cell_type": "code", "metadata": {}, "execution_count": 1,
         "source": format!("k = '{a}'\nprint(k)"),
         "outputs": [{"output_type": "stream", "name": "stdout", "text": format!("{a}\n")}]},
    ])));
    // One per place it is written, none from also reading the raw JSON.
    assert_eq!(doc["summary"]["total"], 2, "{:?}", locations(&doc));
}

#[test]
fn a_malformed_notebook_is_scanned_as_text() {
    let a = aws();
    let doc = scan_text(&format!("{{\"cells\": [ {{\"source\": \"k = {a}\"\n"));
    assert_eq!(doc["summary"]["total"], 1, "{:?}", locations(&doc));
    assert_eq!(doc["files"][0]["read_as"], "text");
}

#[test]
fn image_output_is_not_read_as_text() {
    let doc = scan_text(&notebook(json!([
        {"cell_type": "code", "metadata": {}, "execution_count": 1, "source": "plot()",
         "outputs": [{"output_type": "display_data", "metadata": {},
           "data": {"image/png": "A".repeat(200), "text/plain": "<Figure>"}}]},
    ])));
    assert_eq!(doc["summary"]["total"], 0);
}
