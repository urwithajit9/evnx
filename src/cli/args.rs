//! Argument groups shared by many commands, so a flag is spelled, typed and
//! described the same way everywhere. Flatten one into a command with
//! `#[command(flatten)]`.

use crate::core::resolve::Flags;
use clap::{Args, ValueEnum};
use std::fmt;
use std::path::PathBuf;

/// A set of output formats that has a JSON member, for `--json`.
pub trait FormatSet: ValueEnum + Clone + Default + Send + Sync + 'static {
    /// The value `--json` stands for.
    fn json() -> Self;
    /// How this format is written (`core::output`).
    fn mode(&self) -> crate::core::output::Mode;
}

macro_rules! format_enum {
    ($(#[$m:meta])* $name:ident { $first:ident => $ftext:literal $(, $variant:ident => $text:literal)* $(,)? }) => {
        $(#[$m])*
        #[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub enum $name {
            #[default]
            #[value(name = $ftext)]
            $first,
            $(
                #[value(name = $text)]
                $variant,
            )*
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self { Self::$first => $ftext, $(Self::$variant => $text),* })
            }
        }

        impl FormatSet for $name {
            fn json() -> Self {
                Self::Json
            }
            fn mode(&self) -> crate::core::output::Mode {
                match self {
                    Self::$first => crate::core::output::Mode::of(stringify!($first)),
                    $(Self::$variant => crate::core::output::Mode::of(stringify!($variant)),)*
                }
            }
        }
    };
}

format_enum!(
    /// `pretty` or `json`.
    Format {
        Pretty => "pretty",
        Json => "json",
    }
);
format_enum!(
    /// What `evnx scan` can print.
    ScanFormat {
        Pretty => "pretty",
        Json => "json",
        Sarif => "sarif",
        Github => "github",
    }
);
format_enum!(
    /// What `evnx validate` can print.
    ValidateFormat {
        Pretty => "pretty",
        Json => "json",
        Github => "github",
    }
);
format_enum!(
    /// What `evnx diff` can print.
    DiffFormat {
        Pretty => "pretty",
        Json => "json",
        Patch => "patch",
    }
);

/// `--format` and its shorthand `--json`.
#[derive(Args, Debug, Clone)]
pub struct OutputArgs<F: FormatSet = Format> {
    /// Output format.
    #[arg(long, value_enum, default_value_t = F::default(), help_heading = "Output")]
    pub format: F,

    /// Same as --format json.
    #[arg(long, conflicts_with = "format", help_heading = "Output")]
    pub json: bool,
}

impl<F: FormatSet> OutputArgs<F> {
    /// The format to print: `json` when `--json` was given, else `--format`.
    pub fn resolved(&self) -> F {
        if self.json {
            F::json()
        } else {
            self.format.clone()
        }
    }
}

/// `-y, --yes`: do not ask.
#[derive(Args, Debug, Clone, Copy, Default)]
pub struct ConfirmArgs {
    /// Do not ask for confirmation.
    #[arg(short = 'y', long, help_heading = "Confirmation")]
    pub yes: bool,
}

/// `-n, --dry-run`: show what would change, write nothing.
#[derive(Args, Debug, Clone, Copy, Default)]
pub struct DryRunArgs {
    /// Show what would change without writing anything.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

/// The env file, chosen by path or by name, and the template to compare it with.
#[derive(Args, Debug, Clone, Default)]
pub struct FileArgs {
    /// Env file to use. Defaults to `.env`.
    #[arg(long, value_name = "FILE", help_heading = "Files")]
    pub env: Option<String>,

    /// Use `.env.<NAME>` instead of `.env`.
    #[arg(
        long,
        value_name = "NAME",
        conflicts_with = "env",
        help_heading = "Files"
    )]
    pub env_name: Option<String>,

    /// Template to compare with. Defaults to `.env.example`.
    #[arg(long, value_name = "FILE", help_heading = "Files")]
    pub example: Option<String>,
}

impl FileArgs {
    /// The flags as `core::resolve` takes them.
    pub fn flags(&self) -> Flags<'_> {
        Flags {
            env: self.env.as_deref(),
            env_name: self.env_name.as_deref(),
            example: self.example.as_deref(),
        }
    }
}

/// The env file, chosen by path or by name, for commands with no template.
#[derive(Args, Debug, Clone, Default)]
pub struct EnvArgs {
    /// Env file to use. Defaults to `.env`.
    #[arg(long, value_name = "FILE", help_heading = "Files")]
    pub env: Option<String>,

    /// Use `.env.<NAME>` instead of `.env`.
    #[arg(
        long,
        value_name = "NAME",
        conflicts_with = "env",
        help_heading = "Files"
    )]
    pub env_name: Option<String>,
}

impl EnvArgs {
    /// The flags as `core::resolve` takes them (no template).
    pub fn flags(&self) -> Flags<'_> {
        Flags {
            env: self.env.as_deref(),
            env_name: self.env_name.as_deref(),
            example: None,
        }
    }
}

/// `-C, --dir`: the project folder.
#[derive(Args, Debug, Clone, Default)]
pub struct DirArgs {
    /// Project folder to work in. Defaults to the current folder.
    #[arg(short = 'C', long, value_name = "DIR", help_heading = "Files")]
    pub dir: Option<PathBuf>,
}

/// What `evnx convert --from` can read: the two formats `--to` can write that
/// have a plain reader. More wait for `evnx add import`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConvertFrom {
    /// A `.env` file.
    #[default]
    Dotenv,
    /// A flat `{"KEY": "value"}` object.
    Json,
}

/// What `evnx convert --to` can write (and, from 0.14.2, `cloud export --to`).
///
/// Fifteen formats, spelled as [`crate::formats::FORMATS`] names them, except
/// that GitHub Actions is `github`: the one word every command uses for it.
/// The other short spellings `convert` has always taken (`yml`, `k8s`, `tf`,
/// …) stay as hidden aliases; `github-actions` and `gh-actions` do not.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertTarget {
    Json,
    #[value(alias = "yml")]
    Yaml,
    #[value(aliases = ["bash", "export"])]
    Shell,
    #[value(alias = "env")]
    Dotenv,
    #[value(aliases = ["aws", "aws-secrets-manager"])]
    AwsSecrets,
    #[value(aliases = ["gcp", "gcp-secret-manager"])]
    GcpSecrets,
    #[value(aliases = ["azure", "azure-key-vault"])]
    AzureKeyvault,
    Github,
    #[value(aliases = ["docker", "compose"])]
    DockerCompose,
    #[value(aliases = ["k8s", "kubectl"])]
    Kubernetes,
    #[value(aliases = ["tfvars", "tf"])]
    Terraform,
    Doppler,
    Heroku,
    Vercel,
    Railway,
}

impl ConvertTarget {
    /// The name [`crate::formats::find`] knows it by.
    pub fn format_name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Shell => "shell",
            Self::Dotenv => "dotenv",
            Self::AwsSecrets => "aws-secrets",
            Self::GcpSecrets => "gcp-secrets",
            Self::AzureKeyvault => "azure-keyvault",
            Self::Github => "github-actions",
            Self::DockerCompose => "docker-compose",
            Self::Kubernetes => "kubernetes",
            Self::Terraform => "terraform",
            Self::Doppler => "doppler",
            Self::Heroku => "heroku",
            Self::Vercel => "vercel",
            Self::Railway => "railway",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The enum and the format table are one list: a format added to one and
    /// not the other fails here.
    #[test]
    fn convert_target_matches_the_format_table() {
        let from_enum: Vec<&str> = ConvertTarget::value_variants()
            .iter()
            .map(|t| t.format_name())
            .collect();
        let table: Vec<&str> = crate::formats::FORMATS.iter().map(|f| f.name).collect();
        assert_eq!(from_enum, table);
    }

    /// Every alias the table lists still parses, except the two GitHub ones
    /// that were removed.
    #[test]
    fn convert_target_keeps_the_table_aliases() {
        for f in crate::formats::FORMATS {
            for alias in f.aliases {
                if ["github", "gh-actions"].contains(alias) && f.name == "github-actions" {
                    continue;
                }
                assert!(
                    ConvertTarget::from_str(alias, true).is_ok(),
                    "{alias} no longer parses"
                );
            }
        }
        assert!(ConvertTarget::from_str("github-actions", true).is_err());
        assert!(ConvertTarget::from_str("gh-actions", true).is_err());
        assert_eq!(
            ConvertTarget::from_str("github", true)
                .unwrap()
                .format_name(),
            "github-actions"
        );
    }
}
