// ============================================================================
// formats/kubernetes.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter, Only};
use crate::core::secret::Vars;
use anyhow::Result;
// use std::collections::HashMap;

/// What `--to kubernetes` writes (`--kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum KubeKind {
    /// One `Secret` holding every variable (the default).
    #[default]
    Secret,
    /// One `ConfigMap` holding every variable, as plain text.
    Configmap,
    /// Two documents: a `Secret` for the variables that look secret and a
    /// `ConfigMap` (`<name>-config`) for the rest.
    Split,
}

pub struct KubernetesSecretConverter {
    pub secret_name: String,
    /// `metadata.namespace`; none means the namespace `kubectl` applies to.
    pub namespace: Option<String>,
    /// `--kind`.
    pub kind: KubeKind,
}

/// ⚠️ `app-secrets` is a placeholder. `convert` requires `--name` (I-084,
/// DEC-CV1); only `cloud export` still builds this default.
impl Default for KubernetesSecretConverter {
    fn default() -> Self {
        Self {
            secret_name: "app-secrets".to_string(),
            namespace: None,
            kind: KubeKind::Secret,
        }
    }
}

impl KubernetesSecretConverter {
    /// One manifest. `entries` are the names and values to write.
    fn manifest(
        &self,
        kind: &str,
        name: &str,
        entries: &[(String, String)],
        base64: bool,
    ) -> String {
        let mut output = String::new();
        output.push_str("apiVersion: v1\n");
        output.push_str(&format!("kind: {kind}\n"));
        output.push_str("metadata:\n");
        output.push_str(&format!("  name: {name}\n"));
        if let Some(ns) = &self.namespace {
            output.push_str(&format!("  namespace: {ns}\n"));
        }
        if kind == "Secret" {
            output.push_str("type: Opaque\n");
            output.push_str(if base64 { "data:\n" } else { "stringData:\n" });
        } else {
            output.push_str("data:\n");
        }
        for (key, value) in entries {
            // ⛔ A quoted scalar, always. Unquoted, YAML types the value by its
            // shape: `PORT: 8080` is an integer and `BOOL: true` a boolean, both
            // of which kubectl rejects under `stringData:`; `a: b` becomes a
            // mapping; `#` starts a comment; and a value containing `\n---\n`
            // starts a second manifest.
            output.push_str(&format!(
                "  {}: {}\n",
                key,
                crate::formats::quoting::yaml_scalar(value.as_str())
            ));
        }
        output
    }
}

impl Converter for KubernetesSecretConverter {
    fn convert(&self, vars: &Vars, options: &ConvertOptions) -> Result<String> {
        match self.kind {
            KubeKind::Secret => {
                let entries = options.prepare(vars, self)?;
                Ok(self.manifest("Secret", &self.secret_name, &entries, options.base64))
            }
            KubeKind::Configmap => {
                if options.base64 {
                    anyhow::bail!(
                        "--base64 is for a Secret's `data:`; a ConfigMap holds plain text. \
                         Use --kind secret, or drop --base64."
                    );
                }
                let entries = options.prepare(vars, self)?;
                Ok(self.manifest("ConfigMap", &self.secret_name, &entries, false))
            }
            KubeKind::Split => {
                // The same classifier as `--only`; a user's own `--only` keeps
                // the one side it names. Values are encoded for the Secret
                // only, so `--base64` fills its `data:` and leaves the
                // ConfigMap readable.
                let plain = ConvertOptions {
                    base64: false,
                    ..options.clone()
                };
                let side = |only: Only| -> Result<Vec<(String, String)>> {
                    if options.only.is_some_and(|o| o != only) {
                        return Ok(Vec::new());
                    }
                    ConvertOptions {
                        only: Some(only),
                        ..plain.clone()
                    }
                    .prepare(vars, self)
                };
                let secrets: Vec<(String, String)> = side(Only::Secrets)?
                    .into_iter()
                    .map(|(k, v)| {
                        let v = options.transform_value(&v);
                        (k, v)
                    })
                    .collect();
                let public = side(Only::Public)?;
                let mut docs = Vec::new();
                if !secrets.is_empty() || public.is_empty() {
                    docs.push(self.manifest("Secret", &self.secret_name, &secrets, options.base64));
                }
                if !public.is_empty() {
                    docs.push(self.manifest(
                        "ConfigMap",
                        &format!("{}-config", self.secret_name),
                        &public,
                        false,
                    ));
                }
                Ok(docs.join("---\n"))
            }
        }
    }

    fn name(&self) -> &str {
        "kubernetes"
    }

    fn description(&self) -> &str {
        "Kubernetes Secret YAML"
    }
}
