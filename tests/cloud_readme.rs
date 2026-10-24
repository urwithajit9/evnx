//! The README's cloud part, checked against the CLI both ways (D-155 … D-158,
//! D-301, D-302, DEC-VL9, DEC-CL9).
//!
//! It named `cloud export --out` (the flag is `--output`), said `watch` reported
//! "someone else" pushing when it reported every push, and left out most of
//! `org`, `vault rekey`, `--no-rekey`, `cloud delete-version` and `unlink`. A
//! command added or renamed now fails here until the README says so — the same
//! guard `t_d150` keeps on the account section.
#![cfg(feature = "cloud")]

mod support;

use std::collections::BTreeSet;

fn readme_cloud_part() -> String {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    readme
        .split("## Cloud sync")
        .nth(1)
        .and_then(|s| s.split("\n## CI/CD Integration").next())
        .expect("README has a cloud sync part")
        .to_string()
}

/// The code in `text`: fenced blocks and inline spans. Prose such as "an evnx
/// cloud account" is not a command.
fn code_in(text: &str) -> Vec<&str> {
    let mut code = Vec::new();
    for (i, part) in text.split("```").enumerate() {
        if i % 2 == 1 {
            code.push(part);
        } else {
            code.extend(part.split('`').skip(1).step_by(2));
        }
    }
    code
}

/// `evnx vault|cloud|org <sub>` as written in the cloud part's code.
fn named_in(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for code in code_in(text) {
        for group in ["vault", "cloud", "org"] {
            let needle = format!("evnx {group} ");
            for (i, _) in code.match_indices(&needle) {
                let sub: String = code[i + needle.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                    .collect();
                if !sub.is_empty() && !sub.starts_with('-') {
                    found.insert(format!("{group} {sub}"));
                }
            }
        }
    }
    found
}

#[test]
fn t_d156_the_readme_names_every_vault_cloud_and_org_command_and_only_real_ones() {
    let real: BTreeSet<String> = support::surface::entries()
        .into_iter()
        .filter(|e| e.path.len() == 2 && ["vault", "cloud", "org"].contains(&e.path[0].as_str()))
        .map(|e| e.name())
        .collect();
    assert!(real.len() >= 36, "only {} found: {real:?}", real.len());
    let readme = named_in(&readme_cloud_part());

    let missing: Vec<_> = real.difference(&readme).collect();
    let unknown: Vec<_> = readme.difference(&real).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "README cloud part is out of step with the CLI.\n\
         not in the README: {missing:?}\nnot a real command: {unknown:?}"
    );
}

/// D-155, D-157, D-301, D-302: the statements that were wrong or missing.
#[test]
fn t_d155_the_readme_says_what_the_cli_does() {
    let text = readme_cloud_part();
    assert!(
        !text.contains("export --out "),
        "D-155: the flag is --output"
    );
    for needle in [
        "--output",                               // D-155
        "--include-mine",                         // D-157: watch skips your own pushes
        "--no-rekey",                             // D-156
        "--all-versions",                         // D-156
        "| viewer | developer | admin | owner |", // D-302: the role table
        "vault-scoped token",                     // D-301: what works by id
        "--yes",                                  // DEC-CL9: the production push prompt
    ] {
        assert!(text.contains(needle), "the README does not say {needle:?}");
    }
}
