//! `evnx scan rules add | update | remove` (C7; S2, S4, S6).
//!
//! The only code that fetches a rule file. A scan never does: it reads what
//! these commands wrote, and checks it against `rules.lock`.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::core::secretscan::packs::{self, Entry, Index, Lock};
use crate::core::secretscan::rules::{self, PROJECT_RULES_DIR};
use crate::core::secretscan::PatternSet;
use crate::utils::ui;

/// The rule folder `add` writes to: the project's, or the personal one.
fn rules_dir(project: &Path, personal: bool) -> Result<PathBuf> {
    if personal {
        rules::personal_dir()
            .ok_or_else(|| anyhow::anyhow!("no personal config folder; set EVNX_CONFIG_DIR"))
    } else {
        Ok(project.join(PROJECT_RULES_DIR))
    }
}

fn shown(dir: &Path, personal: bool) -> String {
    if personal {
        dir.display().to_string()
    } else {
        PROJECT_RULES_DIR.to_string()
    }
}

/// A rule file over https, or plain http to this machine (tests, a local
/// mirror). Capped at the rule-file limit.
#[cfg(feature = "net")]
fn fetch(url: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let parsed = reqwest::Url::parse(url).with_context(|| format!("'{url}' is not a URL"))?;
    let loopback = matches!(
        parsed.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]") | Some("::1")
    );
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
        bail!("rule files are downloaded over https only, and {url} is not");
    }
    let timeout = std::time::Duration::from_secs(30);
    let resp = crate::core::http::client(timeout, None)?
        .get(url)
        .send()
        .map_err(|e| {
            anyhow::anyhow!(
                "downloading {url}: {}",
                crate::core::http::transport_reason(&e, timeout)
            )
        })?;
    if !resp.status().is_success() {
        bail!("downloading {url}: the server answered {}", resp.status());
    }
    let mut bytes = Vec::new();
    resp.take(rules::MAX_RULE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("downloading {url}"))?;
    if bytes.len() as u64 > rules::MAX_RULE_FILE_BYTES {
        bail!("{url} is larger than 2 MiB, the most a rule file may be");
    }
    Ok(bytes)
}

#[cfg(not(feature = "net"))]
fn fetch(url: &str) -> Result<Vec<u8>> {
    bail!(
        "this evnx was built without network support (the `net` feature), so it \
         cannot download {url}. Download the file yourself and add it by path: \
         `evnx scan rules add ./file.toml`"
    )
}

/// The catalogue's entry for `name`, and its bytes, which must hash to the
/// index's sha256.
fn from_catalogue(name: &str) -> Result<(Vec<u8>, packs::IndexEntry)> {
    let index_url = packs::catalogue_url();
    let index = Index::parse(&String::from_utf8_lossy(&fetch(&index_url)?))?;
    let Some(entry) = index.packs.iter().find(|p| p.name == name).cloned() else {
        let names: Vec<&str> = index.packs.iter().map(|p| p.name.as_str()).collect();
        bail!(
            "the catalogue has no pack '{name}'. It has: {}",
            if names.is_empty() {
                "none yet".to_string()
            } else {
                names.join(", ")
            }
        );
    };
    let bytes = fetch(&packs::resolve(&index_url, &entry.path))?;
    if packs::sha256_hex(&bytes) != entry.sha256 {
        bail!(
            "the download of '{name}' does not match the sha256 the catalogue lists; \
             nothing was written"
        );
    }
    Ok((bytes, entry))
}

/// How `source` is reached: a URL, an existing path, else a catalogue name.
fn kind_of(source: &str) -> &'static str {
    if source.contains("://") {
        "url"
    } else if Path::new(source).exists() {
        "path"
    } else {
        "catalogue"
    }
}

/// `source`'s bytes as they will be stored, the name they are stored under,
/// and their rule count, converted from gitleaks when `format` says so.
fn obtain(kind: &str, source: &str, format: &str) -> Result<(String, String, usize)> {
    let (bytes, name) = match kind {
        "catalogue" => {
            if !packs::valid_name(source) {
                bail!("'{source}' is not a file, a URL or a catalogue pack name");
            }
            let (bytes, _) = from_catalogue(source)?;
            (bytes, source.to_string())
        }
        _ => {
            let bytes = if kind == "url" {
                fetch(source)?
            } else {
                std::fs::read(source).with_context(|| format!("reading {source}"))?
            };
            let stem = source
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(source)
                .trim_end_matches(".toml")
                .to_lowercase();
            if !packs::valid_name(&stem) {
                bail!(
                    "a pack is named after its file, and '{stem}' is not a valid name \
                     (lowercase letters, digits and '-'); rename the file"
                );
            }
            (bytes, stem)
        }
    };
    let text = String::from_utf8(bytes).context("a rule file must be UTF-8 text")?;
    if format != "gitleaks" {
        let n = rules::parse_rule_text(&text, source)?.len();
        return Ok((text, name, n));
    }
    let c = packs::from_gitleaks(&text, &name)?;
    let today = chrono::Local::now().format("%Y-%m-%d");
    let mut header = vec![
        format!("Converted from {source} (gitleaks format) by `evnx scan rules add`, {today}."),
        "⛔ Do not edit: a scan refuses this file once it differs from rules.lock.".into(),
        format!("`evnx scan rules update {name}` fetches and converts it again."),
        "Check the source's licence before committing it.".into(),
        format!(
            "{} rules converted, {} skipped. {} rule(s) lost an allowlist tied to paths or",
            c.rules.len(),
            c.skipped.len(),
            c.narrowed
        ),
        "commits, which evnx cannot express, so they may report more than in gitleaks.".into(),
    ];
    for (id, why) in &c.skipped {
        header.push(format!("skipped {id}: {why}"));
    }
    if !c.skipped.is_empty() {
        eprintln!("  {} gitleaks rule(s) not converted:", c.skipped.len());
        for (id, why) in &c.skipped {
            eprintln!("    {id}: {why}");
        }
    }
    if c.narrowed > 0 {
        ui::warning(format!(
            "{} rule(s) had an allowlist tied to paths or commits, which is not carried \
             over; they may report more than in gitleaks",
            c.narrowed
        ));
    }
    let n = c.rules.len();
    Ok((packs::to_rule_file(&c.rules, &header)?, name, n))
}

/// The pack's rules compile, and no id clashes with evnx's own or another
/// file's in either folder (S5). `except` is the pack being replaced.
fn check(text: &str, label: &str, project: &Path, except: &Path) -> Result<Vec<String>> {
    let rules = rules::parse_rule_text(text, label)?;
    let mut taken: std::collections::HashMap<String, String> = rules::builtin_rules()
        .into_iter()
        .filter_map(|r| r.id)
        .map(|id| (id, "evnx's own rules".to_string()))
        .collect();
    let dirs = [Some(project.join(PROJECT_RULES_DIR)), rules::personal_dir()];
    for dir in dirs.into_iter().flatten() {
        for f in rules::toml_files(&dir).unwrap_or_default() {
            if f == except {
                continue;
            }
            let shown = f.display().to_string();
            for r in rules::read_rule_file(&f, &shown).unwrap_or_default() {
                if let Some(id) = r.id {
                    taken.insert(id, shown.clone());
                }
            }
        }
    }
    let mut ids = Vec::with_capacity(rules.len());
    for r in &rules {
        let id =
            r.id.clone()
                .ok_or_else(|| anyhow::anyhow!("{label}: rule '{}' needs an id", r.name))?;
        if let Some(other) = taken.get(&id) {
            bail!("rule id '{id}' in {label} is already in {other}; ids must be unique");
        }
        ids.push(id);
    }
    PatternSet::compile(&rules)
        .with_context(|| format!("{label} has a rule that does not compile"))?;
    Ok(ids)
}

pub fn add(project: &Path, source: &str, format: &str, personal: bool) -> Result<()> {
    let dir = rules_dir(project, personal)?;
    let lock_path = packs::lock_path(&dir);
    let mut lock = Lock::load(&lock_path)?;
    let kind = kind_of(source);
    let (text, name, n) = obtain(kind, source, format)?;
    let file = dir.join(format!("{name}.toml"));
    let shown_file = format!("{}/{name}.toml", shown(&dir, personal));
    if lock.get(&name).is_some() {
        bail!("the pack '{name}' is already added; `evnx scan rules update {name}` refetches it");
    }
    if file.exists() {
        bail!("{shown_file} already exists and is not a pack; rename one of them");
    }
    check(&text, &shown_file, project, &file)?;
    crate::core::safe_write::create_dirs(&dir)?;
    crate::core::safe_write::write_shared(&file, text.as_bytes())?;
    lock.packs.push(Entry {
        name: name.clone(),
        kind: kind.to_string(),
        source: source.to_string(),
        format: format.to_string(),
        sha256: packs::sha256_hex(text.as_bytes()),
        rules: n,
        added: chrono::Local::now().format("%Y-%m-%d").to_string(),
    });
    lock.packs.sort_by(|a, b| a.name.cmp(&b.name));
    lock.save(&lock_path)?;
    ui::success(format!(
        "added {name}: {n} rule(s) in {shown_file}, recorded in {}",
        if personal {
            lock_path.display().to_string()
        } else {
            ".evnx/rules.lock".to_string()
        }
    ));
    if !personal {
        ui::info("Commit .evnx/rules/ and .evnx/rules.lock so the team and CI use it too.");
    }
    Ok(())
}

pub fn update(project: &Path, name: Option<&str>, personal: bool, yes: bool) -> Result<()> {
    let dir = rules_dir(project, personal)?;
    let lock_path = packs::lock_path(&dir);
    let mut lock = Lock::load(&lock_path)?;
    let names: Vec<String> = match name {
        Some(n) => {
            if lock.get(n).is_none() {
                bail!("no pack '{n}' has been added here");
            }
            vec![n.to_string()]
        }
        None => lock.packs.iter().map(|e| e.name.clone()).collect(),
    };
    if names.is_empty() {
        ui::info("no packs added here; nothing to update");
        return Ok(());
    }
    for name in names {
        let entry = lock.get(&name).cloned().expect("listed above");
        let (text, _, n) = obtain(&entry.kind, &entry.source, &entry.format)?;
        let sha = packs::sha256_hex(text.as_bytes());
        let file = dir.join(format!("{name}.toml"));
        let shown_file = format!("{}/{name}.toml", shown(&dir, personal));
        let on_disk = std::fs::read(&file).ok();
        if sha == entry.sha256 && on_disk.as_deref().map(packs::sha256_hex) == Some(sha.clone()) {
            ui::success(format!("{name} is up to date ({n} rules)"));
            continue;
        }
        let new_ids = check(&text, &shown_file, project, &file)?;
        // What was there, read leniently: the file may be the tampered one.
        let old_ids: Vec<String> = on_disk
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|t| rules::parse_rule_text(&t, &shown_file).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| r.id)
            .collect();
        let added: Vec<&String> = new_ids.iter().filter(|i| !old_ids.contains(i)).collect();
        let removed: Vec<&String> = old_ids.iter().filter(|i| !new_ids.contains(i)).collect();
        println!("  {name}: {} → {} rules", entry.rules, n);
        if !added.is_empty() {
            println!(
                "    + {}",
                added
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !removed.is_empty() {
            println!(
                "    - {}",
                removed
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if sha == entry.sha256 {
            println!("    (the file on disk had changed; this restores what was added)");
        }
        if !crate::utils::prompt::confirm(
            &format!("Replace {shown_file}?"),
            yes,
            &format!("evnx scan rules update {name} --yes"),
        )? {
            ui::info(format!("{name} left as it was"));
            continue;
        }
        crate::core::safe_write::write_shared(&file, text.as_bytes())?;
        if let Some(e) = lock.packs.iter_mut().find(|e| e.name == name) {
            e.sha256 = sha;
            e.rules = n;
        }
        lock.save(&lock_path)?;
        ui::success(format!("updated {name}: {n} rule(s)"));
    }
    Ok(())
}

pub fn remove(project: &Path, name: &str, personal: bool) -> Result<()> {
    let dir = rules_dir(project, personal)?;
    let lock_path = packs::lock_path(&dir);
    let mut lock = Lock::load(&lock_path)?;
    let shown_file = format!("{}/{name}.toml", shown(&dir, personal));
    if lock.get(name).is_none() {
        bail!(
            "no pack '{name}' has been added here. A hand-written rule file is not a pack; \
             delete {shown_file} yourself"
        );
    }
    match std::fs::remove_file(dir.join(format!("{name}.toml"))) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing {shown_file}")),
    }
    lock.packs.retain(|e| e.name != name);
    lock.save(&lock_path)?;
    ui::success(format!("removed {name}"));
    Ok(())
}
