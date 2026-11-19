//! WP-14: `diff A B`, and `migrate`'s names, Vercel `--overwrite`, dry runs
//! without target flags, and the argv warning.

mod support;

use support::{Project, run};

// ── diff A B ───────────────────────────────────────────────────────────────────

fn two_files() -> Project {
    Project::new()
        .file("left.env", "SAME=1\nCHANGED=left-value\nONLY_LEFT=x\n")
        .file("right.env", "SAME=1\nCHANGED=right-value\nONLY_RIGHT=y\n")
}

#[test]
fn wp14_diff_positional_two_files_compares_values() {
    let p = two_files();
    let out = run(p.path(), &["diff", "left.env", "right.env"]);
    out.assert_code(1)
        .assert_contains("Missing from ./left.env (present in ./right.env)")
        .assert_contains("ONLY_RIGHT")
        .assert_contains("Extra in ./left.env (not in ./right.env)")
        .assert_contains("ONLY_LEFT")
        .assert_contains("Different values")
        .assert_contains("CHANGED");
}

#[test]
fn wp14_diff_positional_one_file_vs_template_keys_only() {
    let p = Project::new()
        .file("staging.env", "A=1\nB=other\n")
        .example("A=\nB=\nC=\n");
    let out = run(p.path(), &["diff", "staging.env"]);
    out.assert_code(1).assert_contains("C");
    assert!(!out.stdout.contains("Different values"), "{}", out.stdout);
}

#[test]
fn wp14_diff_positional_conflicts_with_env_flags_exit_2() {
    let p = two_files().example("A=1\n");
    for extra in [
        &["--env", "left.env"][..],
        &["--env-name", "staging"],
        &["--example", "right.env"],
        &["--against", "staging"],
    ] {
        let mut args = vec!["diff", "left.env", "right.env"];
        args.extend_from_slice(extra);
        run(p.path(), &args).assert_code(2);
    }
    run(p.path(), &["diff", "left.env", "--env", "x.env"]).assert_code(2);
}

#[test]
fn wp14_diff_positional_right_without_left_is_impossible() {
    // `B` requires `A`: the only way to name one file is `A`.
    let p = two_files();
    run(p.path(), &["diff", "left.env", "right.env", "extra.env"]).assert_code(2);
}

#[test]
fn wp14_diff_identical_files_exit_0() {
    let p = Project::new().file("a.env", "A=1\n").file("b.env", "A=1\n");
    run(p.path(), &["diff", "a.env", "b.env"])
        .assert_code(0)
        .assert_contains("Files are identical");
}

#[test]
fn wp14_diff_different_values_prints_left_first() {
    let p = two_files();
    let out = run(
        p.path(),
        &["diff", "left.env", "right.env", "--show-values"],
    );
    out.assert_code(1);
    let left = out
        .stdout
        .find("./left.env: left-value")
        .expect(&out.stdout);
    let right = out
        .stdout
        .find("./right.env: right-value")
        .expect(&out.stdout);
    assert!(left < right, "the left file comes first:\n{}", out.stdout);
}

#[test]
fn wp14_diff_reverse_swaps_order() {
    let p = two_files();
    let out = run(
        p.path(),
        &[
            "diff",
            "left.env",
            "right.env",
            "--show-values",
            "--reverse",
        ],
    );
    out.assert_code(1);
    let left = out
        .stdout
        .find("./left.env: left-value")
        .expect(&out.stdout);
    let right = out
        .stdout
        .find("./right.env: right-value")
        .expect(&out.stdout);
    assert!(
        right < left,
        "--reverse puts the right file first:\n{}",
        out.stdout
    );
    out.assert_contains("Missing from ./right.env (present in ./left.env)");
}

#[test]
fn wp14_diff_json_unchanged_keys() {
    let p = two_files();
    let out = run(
        p.path(),
        &[
            "diff",
            "left.env",
            "right.env",
            "--format",
            "json",
            "--show-values",
        ],
    );
    out.assert_code(1);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let item = &json["different"][0];
    assert_eq!(item["key"], "CHANGED");
    // The names are kept: `env_value` is the left file, `example_value` the right.
    assert_eq!(item["env_value"], "left-value");
    assert_eq!(item["example_value"], "right-value");
    assert_eq!(json["compared"], "values");
}

// ── migrate ────────────────────────────────────────────────────────────────────

#[cfg(feature = "migrate")]
mod migrate {
    use super::*;
    use support::run_with;

    fn migrate(p: &Project, args: &[&str]) -> support::Output {
        let mut full = vec!["migrate", "--from", "env-file"];
        full.extend_from_slice(args);
        run(p.path(), &full)
    }

    fn project() -> Project {
        Project::new().env("A=1\nB=two words\n")
    }

    #[test]
    fn wp14_migrate_accepts_convert_names() {
        let p = project();
        for (name, extra, marker) in [
            (
                "aws",
                &["--secret-name", "prod/app"][..],
                "AWS Secrets Manager",
            ),
            (
                "aws-secrets",
                &["--secret-name", "prod/app"],
                "AWS Secrets Manager",
            ),
            ("gcp", &["--project", "my-project-1"], "GCP Secret Manager"),
            (
                "gcp-secrets",
                &["--project", "my-project-1"],
                "GCP Secret Manager",
            ),
            ("azure", &["--vault-name", "myvault"], "Azure Key Vault"),
            (
                "azure-key-vault",
                &["--vault-name", "myvault"],
                "Azure Key Vault",
            ),
        ] {
            let mut args = vec!["--to", name, "--dry-run"];
            args.extend_from_slice(extra);
            migrate(&p, &args).assert_code(0).assert_contains(marker);
        }
    }

    #[test]
    fn wp14_migrate_help_says_file_vs_provider() {
        let p = Project::new();
        run(p.path(), &["migrate", "--help"])
            .assert_code(0)
            .assert_contains("`convert` writes a file; `migrate` talks to a provider");
        // And the banner of a run says it, on stderr.
        let out = migrate(
            &project(),
            &["--to", "heroku", "--heroku-app", "my-app", "--dry-run"],
        );
        assert!(
            out.stderr
                .contains("convert writes a file; migrate talks to a provider"),
            "{}",
            out.stderr
        );
    }

    #[test]
    fn wp14_migrate_vercel_force_only_with_overwrite() {
        let p = project();
        let plain = migrate(&p, &["--to", "vercel"]);
        plain.assert_code(0);
        assert!(!plain.stdout.contains("--force"), "{}", plain.stdout);
        assert!(
            plain.stdout.contains("vercel env add A production --yes"),
            "{}",
            plain.stdout
        );

        let over = migrate(&p, &["--to", "vercel", "--overwrite"]);
        over.assert_code(0);
        let adds: Vec<&str> = over
            .stdout
            .lines()
            .filter(|l| l.contains("vercel env add"))
            .collect();
        assert_eq!(adds.len(), 2, "{}", over.stdout);
        assert!(
            adds.iter().all(|l| l.contains("--force --yes")),
            "{}",
            over.stdout
        );
    }

    #[test]
    fn wp14_migrate_vercel_skip_existing_refused() {
        migrate(&project(), &["--to", "vercel", "--skip-existing"])
            .assert_code(2)
            .assert_contains("--skip-existing cannot apply to `vercel`");
    }

    #[test]
    fn wp14_migrate_overwrite_still_refused_for_other_printing_destinations() {
        migrate(
            &project(),
            &["--to", "heroku", "--heroku-app", "my-app", "--overwrite"],
        )
        .assert_code(2)
        .assert_contains("--overwrite cannot apply to `heroku`");
    }

    #[test]
    fn wp14_migrate_vercel_points_to_platform_push() {
        let out = migrate(&project(), &["--to", "vercel"]);
        out.assert_code(0);
        assert!(out.stdout.contains("evnx platform push"), "{}", out.stdout);
        // Not among the commands: it is a status line, so `--print-only | sh`
        // never sees it.
        let piped = migrate(&project(), &["--to", "vercel", "--print-only"]);
        assert!(!piped.stdout.contains("platform push"), "{}", piped.stdout);
    }

    /// MIG-2: heroku, doppler, infisical and railway put the values in argv, and
    /// say so once, on stderr, whatever hides the status lines.
    #[test]
    fn wp14_migrate_four_argv_destinations_warn_once() {
        let p = project();
        let cases: [&[&str]; 4] = [
            &["--to", "heroku", "--heroku-app", "my-app"],
            &[
                "--to",
                "doppler",
                "--project",
                "p",
                "--doppler-config",
                "dev",
            ],
            &["--to", "infisical", "--project", "p"],
            &["--to", "railway"],
        ];
        for base in cases {
            for extra in [&[][..], &["--print-only"], &["-q"]] {
                let mut args = base.to_vec();
                args.extend_from_slice(extra);
                let out = migrate(&p, &args);
                out.assert_code(0);
                assert_eq!(
                    out.stderr
                        .matches("pass the values as command-line arguments")
                        .count(),
                    1,
                    "{args:?}:\nstderr: {}",
                    out.stderr
                );
                assert!(
                    !out.stdout.contains("command-line arguments"),
                    "{args:?}: the warning must not be among the commands:\n{}",
                    out.stdout
                );
            }
        }
    }

    #[test]
    fn wp14_migrate_stdin_destinations_do_not_warn() {
        let p = project();
        for args in [
            &["--to", "vercel"][..],
            &["--to", "gcp-secret-manager", "--project", "my-project-1"],
        ] {
            let out = migrate(&p, args);
            out.assert_code(0);
            assert!(
                !out.stderr.contains("command-line arguments"),
                "{}",
                out.stderr
            );
        }
    }

    /// D8: a dry run works without the target flag, shows a placeholder and says
    /// which flag a real run needs; a real run still refuses.
    #[test]
    fn wp14_migrate_dry_run_missing_identifier_names_flag() {
        let p = project();
        for (to, flag) in [
            ("heroku", "--heroku-app"),
            ("aws-secrets-manager", "--secret-name"),
            ("gcp-secret-manager", "--project"),
            ("azure-keyvault", "--vault-name"),
            ("github", "--repo"),
        ] {
            let dry = run_with(
                p.path(),
                &["migrate", "--from", "env-file", "--to", to, "--dry-run"],
                &[("GITHUB_TOKEN", "")],
                None,
            );
            dry.assert_code(0);
            // GitHub's preview does not print the repository; the others show it.
            if to != "github" {
                assert!(
                    dry.stdout.contains(&format!("<{flag} not set>")),
                    "{to}:\n{}",
                    dry.stdout
                );
            }
            assert!(
                dry.stdout.contains(&format!("A real run needs {flag}")),
                "{to}:\n{}",
                dry.stdout
            );
            let real = migrate(&p, &["--to", to, "--yes"]);
            real.assert_code(2).assert_contains(flag);
        }
    }

    #[test]
    fn wp14_migrate_dry_run_with_the_flag_has_no_placeholder() {
        let out = migrate(
            &project(),
            &["--to", "heroku", "--heroku-app", "my-app", "--dry-run"],
        );
        out.assert_code(0).assert_contains("my-app");
        assert!(!out.stdout.contains("not set"), "{}", out.stdout);
        assert!(!out.stdout.contains("A real run needs"), "{}", out.stdout);
    }

    /// A dry run names names; it prints no value.
    #[test]
    fn wp14_migrate_dry_run_prints_no_value() {
        let p = Project::new().env("PW=hunter2-secret-value\nTOKEN=tok-abc-123\n");
        for args in [
            &["--to", "aws-secrets-manager"][..],
            &["--to", "heroku"],
            &["--to", "gcp-secret-manager"],
            &["--to", "azure-keyvault"],
            &["--to", "vercel"],
            &["--to", "doppler"],
        ] {
            let mut full = args.to_vec();
            full.push("--dry-run");
            let out = migrate(&p, &full);
            out.assert_code(0);
            assert!(
                !out.all().contains("hunter2-secret-value") && !out.all().contains("tok-abc-123"),
                "{args:?} printed a value:\n{}",
                out.all()
            );
        }
    }
}
