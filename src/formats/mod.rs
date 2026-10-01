pub mod aws;
pub mod docker;
pub mod github;
pub mod json;
pub mod kubernetes;
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
use anyhow::{anyhow, Result};

/// Pick a converter by format name or alias, case-insensitively.
///
/// This is the single place that maps a user-supplied string to an output
/// format. `convert` reaches it through `--to`; `cloud export` reaches it
/// through the same flag, and the two must not drift — a format that exists for
/// one and not the other is a bug a reader would never predict.
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
    match format.to_lowercase().as_str() {
        // Generic formats
        "json" => Ok(Box::new(JsonConverter)),
        "yaml" | "yml" => Ok(Box::new(YamlConverter)),
        "shell" | "bash" | "export" => Ok(Box::new(ShellExportConverter)),

        // Cloud providers
        "aws" | "aws-secrets" | "aws-secrets-manager" => Ok(Box::new(AwsSecretsConverter)),
        "gcp" | "gcp-secrets" | "gcp-secret-manager" => Ok(Box::new(GcpSecretConverter::default())),
        "azure" | "azure-keyvault" | "azure-key-vault" => {
            Ok(Box::new(AzureKeyVaultConverter::default()))
        }

        // CI/CD platforms
        "github" | "github-actions" | "gh-actions" => {
            Ok(Box::new(GitHubActionsConverter::default()))
        }

        // Container platforms
        "docker" | "docker-compose" | "compose" => Ok(Box::new(DockerComposeConverter)),
        "kubernetes" | "k8s" | "kubectl" => Ok(Box::new(KubernetesSecretConverter::default())),

        // Infrastructure as Code
        "terraform" | "tfvars" | "tf" => Ok(Box::new(TerraformConverter)),

        // Secret management platforms
        "doppler" => Ok(Box::new(DopplerConverter)),
        "heroku" => Ok(Box::new(HerokuConfigConverter::default())),
        "vercel" => Ok(Box::new(VercelEnvConverter)),
        "railway" => Ok(Box::new(RailwayConverter)),

        unknown => Err(anyhow!("unknown format: {unknown}")),
    }
}

/// Print the supported formats and aliases to stderr.
///
/// Kept beside [`get_converter`] so a format added to one is visible from the
/// other. It writes to stderr because its only callers are reporting a mistake.
pub fn format_help() {
    use colored::Colorize;

    eprintln!("  {}", "Generic:".yellow());
    eprintln!("    json, yaml, shell");
    eprintln!();
    eprintln!("  {}", "Cloud providers:".yellow());
    eprintln!("    aws-secrets, gcp-secrets, azure-keyvault");
    eprintln!();
    eprintln!("  {}", "CI/CD:".yellow());
    eprintln!("    github-actions");
    eprintln!();
    eprintln!("  {}", "Containers:".yellow());
    eprintln!("    docker-compose, kubernetes");
    eprintln!();
    eprintln!("  {}", "Infrastructure:".yellow());
    eprintln!("    terraform");
    eprintln!();
    eprintln!("  {}", "Secret managers:".yellow());
    eprintln!("    doppler, heroku, vercel, railway");
    eprintln!();
    eprintln!("  {}", "Aliases:".dimmed());
    eprintln!("    k8s → kubernetes, tf → terraform, yml → yaml");
    eprintln!("    gh-actions → github-actions, compose → docker-compose");
}

/// The file extension a format's output belongs in, without the dot.
///
/// Only `cloud export` needs this — `convert` writes wherever `--output` says.
/// It is kept here, beside [`get_converter`], so adding a format means editing
/// one file rather than discovering the omission when an export writes `.txt`.
///
/// The names are the canonical ones from [`Converter::name`], not the aliases.
/// ⚠️ Several formats emit a **shell script** that uploads the secrets rather
/// than a data file — `gcp-secrets`, `azure-keyvault`, `heroku` — so they get
/// `sh`. `github-actions` emits prose to paste into a web form, which is not a
/// file format at all; `txt` is the honest answer rather than a misleading one.
pub fn extension_for(format_name: &str) -> &'static str {
    match format_name {
        "json" | "aws-secrets" | "doppler" | "vercel" | "railway" => "json",
        "yaml" | "docker-compose" | "kubernetes" => "yaml",
        "shell" | "gcp-secrets" | "azure-keyvault" | "heroku" => "sh",
        "terraform" => "tfvars",
        "github-actions" => "txt",
        // Unreachable for anything `get_converter` returns — the test below
        // holds that. A format added without an extension lands here rather
        // than failing the export.
        _ => "txt",
    }
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
}
