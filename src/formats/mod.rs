pub mod aws;
pub mod docker;
pub mod github;
pub mod json;
pub mod kubernetes;
/// One quoting rule per output format, in one place — see the module docs for
/// why each converter escaping inline was the wrong shape.
pub mod quoting;
pub mod shell;
pub mod terraform;
pub mod yaml;

// Cloud providers
pub mod azure;
pub mod doppler;
pub mod gcp;
pub mod heroku;
pub mod railway;
pub mod vercel;

// Re-export converters
pub use aws::AwsSecretsConverter;
pub use docker::DockerComposeConverter;
pub use github::GitHubActionsConverter;
pub use json::JsonConverter;
pub use kubernetes::KubernetesSecretConverter;
pub use shell::ShellExportConverter;
pub use terraform::TerraformConverter;
pub use yaml::YamlConverter;

pub use azure::AzureKeyVaultConverter;
pub use doppler::DopplerConverter;
pub use gcp::GcpSecretConverter;
pub use heroku::HerokuConfigConverter;
pub use railway::RailwayConverter;
pub use vercel::VercelEnvConverter;

use crate::core::converter::Converter;
use anyhow::{anyhow, bail, Result};

/// One output format: every list of formats evnx shows is built from these.
///
/// ⚠️ The list was written five times — the dispatch, the unknown-format help,
/// the selector's labels, the extension table and `--help` — and the selector's
/// copy said `aws-secrets` printed "CLI commands" (it prints JSON) and
/// `kubernetes` was "base64 encoded" (it is `stringData` unless `--base64`),
/// I-087. `--help` is static text in `cli.rs`; a test holds it to this table.
#[derive(Debug)]
pub struct Format {
    /// The canonical name: what [`Converter::name`] returns.
    pub name: &'static str,
    /// Other spellings `--to` accepts.
    pub aliases: &'static [&'static str],
    /// The heading it is listed under in the unknown-format help.
    pub group: &'static str,
    /// What the selector shows beside the name.
    pub label: &'static str,
    /// The file extension `cloud export` gives it, without the dot.
    pub extension: &'static str,
    /// Why the output is experimental, said on stderr when it is chosen
    /// (DEC-CV4). `None` for a shape checked against its target (17a).
    pub experimental: Option<&'static str>,
}

/// The 14 formats, in the order every list shows them.
pub const FORMATS: &[Format] = &[
    Format {
        name: "json",
        aliases: &[],
        group: "Generic",
        label: "Generic JSON key-value object",
        extension: "json",
        experimental: None,
    },
    Format {
        name: "yaml",
        aliases: &["yml"],
        group: "Generic",
        label: "Generic YAML key-value format",
        extension: "yaml",
        experimental: None,
    },
    Format {
        name: "shell",
        aliases: &["bash", "export"],
        group: "Generic",
        label: "Shell export script (bash/zsh)",
        extension: "sh",
        experimental: None,
    },
    Format {
        name: "aws-secrets",
        aliases: &["aws", "aws-secrets-manager"],
        group: "Cloud providers",
        label: "AWS Secrets Manager JSON (for create-secret --secret-string)",
        extension: "json",
        experimental: None,
    },
    Format {
        name: "gcp-secrets",
        aliases: &["gcp", "gcp-secret-manager"],
        group: "Cloud providers",
        label: "GCP Secret Manager script (gcloud; needs --project)",
        extension: "sh",
        experimental: None,
    },
    Format {
        name: "azure-keyvault",
        aliases: &["azure", "azure-key-vault"],
        group: "Cloud providers",
        label: "Azure Key Vault script (az; needs --vault-name)",
        extension: "sh",
        experimental: None,
    },
    Format {
        name: "github-actions",
        aliases: &["github", "gh-actions"],
        group: "CI/CD",
        label: "GitHub Actions secrets (text to paste into the web form)",
        extension: "txt",
        experimental: None,
    },
    Format {
        name: "docker-compose",
        aliases: &["docker", "compose"],
        group: "Containers",
        label: "Docker Compose YAML environment",
        extension: "yaml",
        experimental: None,
    },
    Format {
        name: "kubernetes",
        aliases: &["k8s", "kubectl"],
        group: "Containers",
        label: "Kubernetes Secret YAML (stringData, or data with --base64; needs --name)",
        extension: "yaml",
        experimental: None,
    },
    Format {
        name: "terraform",
        aliases: &["tfvars", "tf"],
        group: "Infrastructure",
        label: "Terraform .tfvars file",
        extension: "tfvars",
        experimental: None,
    },
    Format {
        name: "doppler",
        aliases: &[],
        group: "Secret managers",
        label: "Doppler secrets JSON (experimental)",
        extension: "json",
        experimental: Some(
            "doppler output is experimental: `doppler secrets upload` is documented to take \
             flat JSON (what --to json prints), not this shape.",
        ),
    },
    Format {
        name: "heroku",
        aliases: &[],
        group: "Secret managers",
        label: "Heroku config vars script (heroku; needs --heroku-app)",
        extension: "sh",
        experimental: None,
    },
    Format {
        name: "vercel",
        aliases: &[],
        group: "Secret managers",
        label: "Vercel environment variables JSON (experimental)",
        extension: "json",
        experimental: Some(
            "vercel output is experimental: Vercel's API takes {key, value, type, target} \
             objects, not this map.",
        ),
    },
    Format {
        name: "railway",
        aliases: &[],
        group: "Secret managers",
        label: "Railway variables JSON (experimental)",
        extension: "json",
        experimental: Some(
            "railway output is experimental: it is the JSON --to json prints, and Railway \
             documents no import that takes it.",
        ),
    },
];

/// The format `name` names — canonical or alias, any case.
pub fn find(name: &str) -> Option<&'static Format> {
    let wanted = name.to_lowercase();
    FORMATS
        .iter()
        .find(|f| f.name == wanted || f.aliases.contains(&wanted.as_str()))
}

/// The canonical names, in order: what an error lists as available.
pub fn names() -> Vec<&'static str> {
    FORMATS.iter().map(|f| f.name).collect()
}

/// A converter with every target name at its default.
fn default_converter(name: &str) -> Box<dyn Converter> {
    match name {
        "json" => Box::new(JsonConverter),
        "yaml" => Box::new(YamlConverter),
        "shell" => Box::new(ShellExportConverter),
        "aws-secrets" => Box::new(AwsSecretsConverter),
        "gcp-secrets" => Box::new(GcpSecretConverter::default()),
        "azure-keyvault" => Box::new(AzureKeyVaultConverter::default()),
        "github-actions" => Box::new(GitHubActionsConverter::default()),
        "docker-compose" => Box::new(DockerComposeConverter),
        "kubernetes" => Box::new(KubernetesSecretConverter::default()),
        "terraform" => Box::new(TerraformConverter),
        "doppler" => Box::new(DopplerConverter),
        "heroku" => Box::new(HerokuConfigConverter::default()),
        "vercel" => Box::new(VercelEnvConverter),
        "railway" => Box::new(RailwayConverter),
        // `FORMATS` and this match are kept in step by a test below.
        other => unreachable!("{other} is in FORMATS but has no converter"),
    }
}

/// Pick a converter by format name or alias, case-insensitively.
///
/// This is the single place that maps a user-supplied string to an output
/// format. `convert` reaches it through `--to`; `cloud export` reaches it
/// through the same flag, and the two must not drift — a format that exists for
/// one and not the other is a bug a reader would never predict.
///
/// ⚠️ The names a format puts in its output (a GCP project, a Key Vault, a
/// Heroku app, a Secret's name) are at their placeholders here. `convert`
/// builds through [`converter_for`], which requires them (I-084, DEC-CV1);
/// `cloud export` still converts with these (recorded for R-org-vault-cloud).
///
/// # ⚠️ It returns an error; it does not exit
///
/// This lived in `commands/convert.rs` and ended an unrecognised format with
/// `std::process::exit(2)`. That made it unusable from anywhere else: a long
/// command would die mid-work with no chance to clean up, and no test could
/// ever cover the unknown-format arm, because exiting takes the test harness
/// with it. `convert` still exits 2, at its own call site, with byte-identical
/// output — the decision simply moved to where the process belongs.
///
/// [`format_help`] prints the supported list, so a caller can offer the same
/// guidance `convert` does.
pub fn get_converter(format: &str) -> Result<Box<dyn Converter>> {
    find(format)
        .map(|f| default_converter(f.name))
        .ok_or_else(|| anyhow!("unknown format: {}", format.to_lowercase()))
}

/// The names a format writes into its output, from `convert`'s flags.
#[derive(Debug, Clone, Default)]
pub struct Targets {
    /// `--project`: the Google Cloud project (gcp-secrets).
    pub project: Option<String>,
    /// `--vault-name`: the Azure Key Vault (azure-keyvault).
    pub vault_name: Option<String>,
    /// `--heroku-app`: the Heroku app (heroku).
    pub heroku_app: Option<String>,
    /// `--name`: the Kubernetes Secret's name (kubernetes).
    pub name: Option<String>,
    /// `--namespace`: the Kubernetes namespace, optional (kubernetes).
    pub namespace: Option<String>,
}

impl Targets {
    /// Each flag, its value, the format it belongs to, and its name rule.
    fn flags(
        &self,
    ) -> [(
        &'static str,
        Option<&str>,
        &'static str,
        crate::core::args::Name,
    ); 5] {
        use crate::core::args::Name;
        [
            (
                "--project",
                self.project.as_deref(),
                "gcp-secrets",
                Name::GcpProject,
            ),
            (
                "--vault-name",
                self.vault_name.as_deref(),
                "azure-keyvault",
                Name::AzureVault,
            ),
            (
                "--heroku-app",
                self.heroku_app.as_deref(),
                "heroku",
                Name::HerokuApp,
            ),
            ("--name", self.name.as_deref(), "kubernetes", Name::K8sName),
            (
                "--namespace",
                self.namespace.as_deref(),
                "kubernetes",
                Name::K8sNamespace,
            ),
        ]
    }

    /// Refuse a name flag that is malformed, or meant for another format.
    ///
    /// Checked before the input is read, so a typo costs nothing.
    pub fn check(&self, format: Option<&Format>) -> Result<()> {
        for (flag, value, belongs, rule) in self.flags() {
            let Some(value) = value else { continue };
            crate::core::args::check(flag, value, rule)?;
            if let Some(f) = format {
                if f.name != belongs {
                    bail!(
                        "{flag} is for --to {belongs}; --to {} does not use it. Nothing was \
                         printed.",
                        f.name
                    );
                }
            }
        }
        Ok(())
    }
}

/// The converter for `format`, with the names `targets` gives it (I-084).
///
/// ⚠️ A format that writes a place into its output requires the flag that
/// names it (DEC-CV1): `my-project`, `my-keyvault`, `YOUR_APP_NAME` and
/// `app-secrets` were printed for the reader to find and edit by hand, and a
/// script run unedited went to a project nobody owns or failed halfway.
pub fn converter_for(format: &Format, targets: &Targets) -> Result<Box<dyn Converter>> {
    targets.check(Some(format))?;
    let need = |value: &Option<String>, flag: &str, placeholder: &str, what: &str| {
        value.clone().ok_or_else(|| {
            anyhow!(
                "--to {name} needs {flag}: {what}. Nothing was printed.\n\n\
                 \x20 evnx convert --to {name} {flag} <{placeholder}>",
                name = format.name
            )
        })
    };
    Ok(match format.name {
        "gcp-secrets" => Box::new(GcpSecretConverter {
            project_id: need(
                &targets.project,
                "--project",
                "PROJECT",
                "the Google Cloud project the secrets go to",
            )?,
        }),
        "azure-keyvault" => Box::new(AzureKeyVaultConverter {
            vault_name: need(
                &targets.vault_name,
                "--vault-name",
                "NAME",
                "the Key Vault the secrets go to",
            )?,
        }),
        "heroku" => Box::new(HerokuConfigConverter::new(need(
            &targets.heroku_app,
            "--heroku-app",
            "APP",
            "the Heroku app the config vars go to",
        )?)),
        "kubernetes" => Box::new(KubernetesSecretConverter {
            secret_name: need(
                &targets.name,
                "--name",
                "NAME",
                "the name of the Secret (--namespace places it, optionally)",
            )?,
            namespace: targets.namespace.clone(),
        }),
        other => default_converter(other),
    })
}

/// Print the supported formats and aliases to stderr.
///
/// Built from [`FORMATS`], so a format added there shows up here without a
/// second edit. It writes to stderr because its only callers are reporting a
/// mistake.
pub fn format_help() {
    use colored::Colorize;

    let mut groups: Vec<&str> = Vec::new();
    for f in FORMATS {
        if !groups.contains(&f.group) {
            groups.push(f.group);
        }
    }
    for group in groups {
        let names: Vec<String> = FORMATS
            .iter()
            .filter(|f| f.group == group)
            .map(|f| match f.experimental {
                Some(_) => format!("{} (experimental)", f.name),
                None => f.name.to_string(),
            })
            .collect();
        eprintln!("  {}", format!("{group}:").yellow());
        eprintln!("    {}", names.join(", "));
        eprintln!();
    }
    eprintln!("  {}", "Aliases:".dimmed());
    eprintln!("    k8s → kubernetes, tf → terraform, yml → yaml");
    eprintln!("    gh-actions → github-actions, compose → docker-compose");
}

/// The file extension a format's output belongs in, without the dot.
///
/// Only `cloud export` needs this — `convert` writes wherever `--output` says.
/// It comes from [`FORMATS`], so adding a format means filling in its row
/// rather than discovering the omission when an export writes `.txt`.
///
/// ⚠️ Several formats emit a **shell script** that uploads the secrets rather
/// than a data file — `gcp-secrets`, `azure-keyvault`, `heroku` — so they get
/// `sh`. `github-actions` emits prose to paste into a web form, which is not a
/// file format at all; `txt` is the honest answer rather than a misleading one.
pub fn extension_for(format_name: &str) -> &'static str {
    FORMATS
        .iter()
        .find(|f| f.name == format_name)
        .map_or("txt", |f| f.extension)
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    /// The arm that could not be tested before, because it exited the process.
    #[test]
    fn an_unknown_format_is_an_error_rather_than_an_exit() {
        // `Box<dyn Converter>` is not `Debug`, so `unwrap_err` is unavailable.
        let Err(err) = get_converter("definitely-not-a-format") else {
            panic!("an unrecognised format must not resolve to a converter");
        };
        assert!(
            err.to_string().contains("definitely-not-a-format"),
            "the message should name what was asked for: {err}"
        );
    }

    #[test]
    fn every_name_and_alias_resolves_to_the_format_it_promises() {
        let cases = [
            ("json", "json"),
            ("yaml", "yaml"),
            ("yml", "yaml"),
            ("shell", "shell"),
            ("bash", "shell"),
            ("export", "shell"),
            ("aws", "aws-secrets"),
            ("aws-secrets", "aws-secrets"),
            ("aws-secrets-manager", "aws-secrets"),
            ("gcp", "gcp-secrets"),
            ("gcp-secrets", "gcp-secrets"),
            ("gcp-secret-manager", "gcp-secrets"),
            ("azure", "azure-keyvault"),
            ("azure-keyvault", "azure-keyvault"),
            ("azure-key-vault", "azure-keyvault"),
            ("github", "github-actions"),
            ("github-actions", "github-actions"),
            ("gh-actions", "github-actions"),
            ("docker", "docker-compose"),
            ("docker-compose", "docker-compose"),
            ("compose", "docker-compose"),
            ("kubernetes", "kubernetes"),
            ("k8s", "kubernetes"),
            ("kubectl", "kubernetes"),
            ("terraform", "terraform"),
            ("tfvars", "terraform"),
            ("tf", "terraform"),
            ("doppler", "doppler"),
            ("heroku", "heroku"),
            ("vercel", "vercel"),
            ("railway", "railway"),
        ];
        for (asked, expected) in cases {
            let c = get_converter(asked).unwrap_or_else(|e| panic!("{asked}: {e}"));
            assert_eq!(c.name(), expected, "{asked} resolved to the wrong format");
        }
    }

    /// Matching is case-insensitive, and was before the move.
    #[test]
    fn case_does_not_matter() {
        assert_eq!(get_converter("JSON").unwrap().name(), "json");
        assert_eq!(get_converter("K8s").unwrap().name(), "kubernetes");
    }

    /// Every format the dispatch can return has a deliberate extension.
    ///
    /// Without this, adding a format silently exports it as `.txt` and nobody
    /// finds out until a file will not open.
    #[test]
    fn every_format_has_an_extension_chosen_on_purpose() {
        let canonical = [
            "json",
            "yaml",
            "shell",
            "aws-secrets",
            "gcp-secrets",
            "azure-keyvault",
            "docker-compose",
            "kubernetes",
            "terraform",
            "doppler",
            "heroku",
            "vercel",
            "railway",
        ];
        // `github-actions` is excluded on purpose and asserted separately
        // below — its output is prose, so `txt` is the right answer, not a
        // fall-through.
        for name in canonical {
            // Round-trips: the name the dispatch reports is the name the
            // extension table is keyed on.
            let c = get_converter(name).unwrap();
            assert_eq!(c.name(), name, "{name} is not its own canonical name");
            assert_ne!(
                extension_for(name),
                "txt",
                "{name} fell through to the default extension — add it to the table"
            );
        }
    }

    /// `github-actions` is the one deliberate `txt`, so the test above excludes
    /// it by asserting the arm exists rather than by skipping it.
    #[test]
    fn github_actions_is_prose_and_says_so() {
        assert_eq!(extension_for("github-actions"), "txt");
    }

    /// Every row of the table builds the converter it names.
    #[test]
    fn every_format_in_the_table_has_its_converter() {
        assert_eq!(FORMATS.len(), 14);
        for f in FORMATS {
            assert_eq!(default_converter(f.name).name(), f.name);
        }
    }

    /// I-087: the selector's labels say what the output is.
    #[test]
    fn t_i087_labels_describe_the_output() {
        let label = |n: &str| find(n).unwrap().label;
        assert!(!label("aws-secrets").contains("CLI commands"));
        assert!(label("aws-secrets").contains("JSON"));
        assert!(!label("kubernetes").contains("base64 encoded"));
        assert!(label("kubernetes").contains("stringData"));
        assert!(label("github-actions").contains("web form"));
        for n in ["doppler", "vercel", "railway"] {
            assert!(find(n).unwrap().experimental.is_some(), "{n}");
            assert!(label(n).contains("experimental"), "{n}");
        }
    }

    /// DEC-CV1: each named target requires its flag, and a flag for another
    /// format is refused.
    #[test]
    fn t_i084_converter_for_requires_the_names() {
        let none = Targets::default();
        for (n, flag) in [
            ("gcp-secrets", "--project"),
            ("azure-keyvault", "--vault-name"),
            ("heroku", "--heroku-app"),
            ("kubernetes", "--name"),
        ] {
            let Err(e) = converter_for(find(n).unwrap(), &none) else {
                panic!("{n} built without {flag}");
            };
            assert!(e.to_string().contains(flag), "{e}");
        }
        let gcp = Targets {
            project: Some("acme-prod".into()),
            ..Default::default()
        };
        assert!(converter_for(find("gcp").unwrap(), &gcp).is_ok());
        let Err(e) = converter_for(find("json").unwrap(), &gcp) else {
            panic!("--project accepted for json");
        };
        assert!(
            e.to_string().contains("--project is for --to gcp-secrets"),
            "{e}"
        );
    }
}
