//! `evnx doctor`: can an AI coding agent read `.env`? (RM-25)
//!
//! - Claude Code reads any file unless `permissions.deny` blocks it, so its
//!   rule is required whatever `.gitignore` says. `--fix` prints the rule and
//!   never rewrites `.claude/settings.json`.
//! - Cursor, Aider, Gemini CLI and Windsurf honour `.gitignore`, and each has
//!   its own ignore file: `.env` is reachable only when neither excludes it.
//!   `--fix` appends `.env*` to the agent's file, once.
//! - A tool the project does not use is never configured.

mod support;

use serde_json::Value;
use support::{run, Project};

/// The `ai_agents` check from `doctor --format json`.
fn agents(p: &Project) -> Value {
    let out = run(p.path(), &["doctor", "--format", "json"]);
    let doc: Value = serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}):\n{}", out.all()));
    doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "ai_agents")
        .cloned()
        .unwrap_or_else(|| panic!("no ai_agents check:\n{}", out.stdout))
}

fn details(c: &Value) -> &str {
    c["details"].as_str().unwrap_or("")
}

/// A project whose `.env` git ignores.
fn ignored() -> Project {
    Project::new()
        .git()
        .env("A=1\n")
        .example("A=\n")
        .file(".gitignore", ".env*\n!.env.example\n")
}

/// A project whose `.env` git does not ignore, in a way `--fix` leaves alone:
/// the user's later `!.env` (I-156). Without it, `--fix` repairs `.gitignore`
/// first and the agents' own files are then not needed.
fn exposed() -> Project {
    Project::new()
        .git()
        .env("A=1\n")
        .example("A=\n")
        .file(".gitignore", ".env*\n!.env\n")
}

#[test]
fn rm25_no_agent_configuration_passes_and_nothing_is_created() {
    let p = ignored();
    let c = agents(&p);
    assert_eq!(c["severity"], "ok", "{c}");
    run(p.path(), &["doctor", "--fix"]);
    for f in [
        ".cursorignore",
        ".aiderignore",
        ".geminiignore",
        ".codeiumignore",
        ".claude",
    ] {
        assert!(!p.path().join(f).exists(), "{f} was created");
    }
}

#[test]
fn rm25_claude_code_needs_a_deny_rule_even_when_gitignored() {
    let p = ignored().file("CLAUDE.md", "# notes\n");
    let c = agents(&p);
    assert_eq!(c["severity"], "warning", "{c}");
    assert!(details(&c).contains("Claude Code can read .env"), "{c}");
    assert!(
        details(&c).contains(r#""Read(./.env)", "Read(./.env.*)""#),
        "{c}"
    );
    assert_eq!(c["fixable"], false, "{c}");

    // --fix never writes the team's settings file.
    run(p.path(), &["doctor", "--fix"]);
    assert!(!p.path().join(".claude/settings.json").exists());

    // The documented rule, in the shared or the local file, satisfies it.
    for file in [".claude/settings.json", ".claude/settings.local.json"] {
        let p = ignored().file(
            file,
            r#"{"permissions":{"deny":["Read(./.env)","Read(./.env.*)"]}}"#,
        );
        let c = agents(&p);
        assert_eq!(c["severity"], "ok", "{file}: {c}");
        assert!(details(&c).contains("Claude Code: permissions.deny covers .env"));
    }
    // One rule for both.
    let p = ignored().file(
        ".claude/settings.json",
        r#"{"permissions":{"deny":["Read(**/.env*)"]}}"#,
    );
    assert_eq!(agents(&p)["severity"], "ok");
}

#[test]
fn rm25_gitignore_is_enough_for_agents_that_honour_it() {
    for marker in [
        ".cursorrules",
        ".aider.conf.yml",
        "GEMINI.md",
        ".windsurfrules",
    ] {
        let p = ignored().file(marker, "x\n");
        let c = agents(&p);
        assert_eq!(c["severity"], "ok", "{marker}: {c}");
    }
}

/// One case per format: warned, fixed into its own file, idempotent.
#[test]
fn rm25_each_ignore_file_agent_is_warned_and_fixed() {
    for (marker, name, file) in [
        (".cursorrules", "Cursor", ".cursorignore"),
        (".aider.conf.yml", "Aider", ".aiderignore"),
        ("GEMINI.md", "Gemini CLI", ".geminiignore"),
        (".windsurfrules", "Windsurf", ".codeiumignore"),
    ] {
        let p = exposed().file(marker, "x\n");
        let c = agents(&p);
        assert_eq!(c["severity"], "warning", "{name}: {c}");
        assert!(
            details(&c).contains(&format!("{name} can read .env")),
            "{c}"
        );
        assert_eq!(c["fixable"], true, "{name}: {c}");

        let out = run(p.path(), &["doctor", "--fix"]);
        assert!(
            out.stderr.contains(&format!("Fixed: {file}: added .env*")),
            "{name}: {}",
            out.stderr
        );
        assert!(p.read(file).lines().any(|l| l == ".env*"), "{file}");
        assert_eq!(agents(&p)["severity"], "ok", "{name} after --fix");

        let before = p.read(file);
        let again = run(p.path(), &["doctor", "--fix"]);
        assert!(!again.stderr.contains(&format!("Fixed: {file}")), "{name}");
        assert_eq!(p.read(file), before, "{name}: --fix is idempotent");
    }
}

#[test]
fn rm25_the_report_names_every_agent_it_checked() {
    let p = exposed()
        .file("CLAUDE.md", "x\n")
        .file(".cursorignore", ".env*\n")
        .file("GEMINI.md", "x\n")
        .file(".github/copilot-instructions.md", "x\n");
    let d = details(&agents(&p)).to_string();
    assert!(d.contains("Claude Code can read .env"), "{d}");
    assert!(d.contains("✓ Cursor"), "{d}");
    assert!(d.contains("Gemini CLI can read .env"), "{d}");
    assert!(
        d.contains("Copilot: content exclusion is an organisation setting"),
        "{d}"
    );
}

#[test]
fn rm25_a_broken_claude_settings_file_is_named() {
    let p = ignored().file(".claude/settings.json", "{ not json");
    let c = agents(&p);
    assert_eq!(c["ran"], false, "{c}");
    assert!(
        details(&c).contains(".claude/settings.json is not valid JSON"),
        "{c}"
    );
}
