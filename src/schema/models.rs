// src/schema/models.rs

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Collections whose iteration order reaches the user — menus, and the
/// non-interactive `--yes` default.
///
/// ⚠️ These must **not** be `HashMap`. `std`'s hasher is seeded per process, so a
/// `HashMap` here reordered the blueprint menu on every launch and made
/// `evnx init --yes` — which takes the first entry — pick a different stack each
/// run: six invocations in one empty directory produced Laravel, Next.js,
/// Laravel, Go, Rust and MERN. `IndexMap` keeps `schema.json`'s own declaration
/// order, which is curated (`t3_modern` first), so the menu is both stable and
/// deliberately ordered rather than merely alphabetical.
type Ordered<T> = IndexMap<String, T>;

/// Root schema containing all configuration data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub languages: Ordered<LanguageConfig>,
    pub services: ServiceCategories,
    pub infrastructure: Ordered<InfrastructureConfig>,
    pub stacks: Ordered<StackBlueprint>,
}

/// Language with its available frameworks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageConfig {
    /// Human-readable name for UI
    #[serde(default)]
    pub display_name: Option<String>,
    pub frameworks: Ordered<FrameworkConfig>,
}

/// Framework configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkConfig {
    /// Human-readable name for UI
    #[serde(default)]
    pub display_name: Option<String>,
    /// Environment variable names this framework uses
    pub vars: Vec<String>,
    /// Optional: default values for some vars
    #[serde(default)]
    pub defaults: HashMap<String, String>,
    /// Optional: descriptions for vars
    #[serde(default)]
    pub descriptions: HashMap<String, String>,
    /// Optional: category mapping for vars
    #[serde(default)]
    pub categories: HashMap<String, String>,
}

/// Service categories (databases, auth, storage, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceCategories {
    #[serde(default)]
    pub databases: Ordered<ServiceConfig>,
    #[serde(default)]
    pub messaging_queues: Ordered<ServiceConfig>,
    #[serde(default)]
    pub auth_providers: Ordered<ServiceConfig>,
    #[serde(default)]
    pub storage: Ordered<ServiceConfig>,
    #[serde(default)]
    pub monitoring_logging: Ordered<ServiceConfig>,
    #[serde(default)]
    pub payments: Ordered<ServiceConfig>,
    #[serde(default)]
    pub ai_ml: Ordered<ServiceConfig>,
    #[serde(default)]
    pub email_sms: Ordered<ServiceConfig>,
}

/// Individual service configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Human-readable name for UI
    #[serde(default)]
    pub display_name: Option<String>,
    pub vars: Vec<String>,
    #[serde(default)]
    pub defaults: HashMap<String, String>,
    #[serde(default)]
    pub descriptions: HashMap<String, String>,
    /// Optional: category for grouping in output
    #[serde(default)]
    pub category: Option<String>,
    /// Optional: which vars are required
    #[serde(default)]
    pub required: Vec<String>,
}

/// Infrastructure/Deployment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfrastructureConfig {
    /// Human-readable name for UI
    #[serde(default)]
    pub display_name: Option<String>,
    pub vars: Vec<String>,
    #[serde(default)]
    pub defaults: HashMap<String, String>,
    #[serde(default)]
    pub descriptions: HashMap<String, String>,
    /// Optional: category for grouping
    #[serde(default)]
    pub category: Option<String>,
    /// Sub-categories like "oidc", "deployment_vars", etc.
    #[serde(default, flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Pre-combined stack blueprint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackBlueprint {
    /// Display name for UI
    pub name: String,
    /// Description shown in selection
    pub description: String,
    /// Components that make up this stack
    pub components: StackComponents,
    /// Optional: tags for filtering
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Components referenced by a blueprint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackComponents {
    /// Language key (e.g., "javascript_typescript")
    pub language: String,
    /// Framework key (e.g., "nextjs")
    pub framework: String,
    /// Service keys (e.g., ["postgresql", "redis"])
    #[serde(default)]
    pub services: Vec<String>,
    /// Infrastructure keys (e.g., ["vercel", "github_actions"])
    #[serde(default)]
    pub infrastructure: Vec<String>,
}

/// Collected variables with metadata, ready for template generation
#[derive(Debug, Clone, Default)]
pub struct VarCollection {
    /// Map of var_name → metadata
    pub vars: HashMap<String, VarMetadata>,
}

#[derive(Debug, Clone)]
pub struct VarMetadata {
    pub example_value: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub required: bool,
    pub source: VarSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VarSource {
    Framework(String),
    Service(String),
    Infrastructure(String),
    BlueprintOverride,
}
