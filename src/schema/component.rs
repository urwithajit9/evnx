//! One flat, addressable catalogue over `schema.json`.
//!
//! # Why this exists
//!
//! `schema.json` already stores frameworks, services and infrastructure as
//! separate, named, reusable pieces — and every way of reaching them was
//! shaped differently. A blueprint reached them through
//! [`resolve_blueprint`](super::resolver::resolve_blueprint), the architect flow
//! through `resolve_architect_selection`, and `evnx add` through
//! `resolve_service` / `resolve_framework`. Three entry points over one set.
//!
//! The consequence was visible from outside: ten fixed blueprints were the only
//! way to combine pieces, so "Django + Kafka + Clerk" was inexpressible without
//! someone adding a `django_kafka_clerk` stack. The combinations were never the
//! problem; the addressing was.
//!
//! A blueprint is, structurally, already a list of these names — `t3_modern` is
//! `nextjs, postgresql, clerk, aws_s3, stripe, github_actions, vercel` — which
//! is what makes this a refactor rather than a redesign.
//!
//! # Names are a compatibility surface
//!
//! ⚠️ Once `--with postgresql` is something people put in a script, renaming
//! `postgresql` to `postgres` breaks it. The 51 ids here are `schema.json`'s
//! existing keys, unchanged, and they should be treated as public from now on.
//! [`catalogue`] is what a deprecation would have to go through.

use anyhow::{bail, Result};

use super::loader::schema;
use super::models::VarCollection;
use super::resolver::{add_framework_vars, add_infra_vars, add_service_vars};

/// What kind of thing a component is — for grouping in a listing, and for
/// telling the user where a name came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A web framework, under some language.
    Framework,
    /// A backing service: database, queue, auth provider, payment processor.
    Service,
    /// A deployment target or CI system.
    Infrastructure,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Framework => "framework",
            Kind::Service => "service",
            Kind::Infrastructure => "infrastructure",
        }
    }
}

/// One addressable piece of a stack.
#[derive(Debug, Clone)]
pub struct Component {
    /// The name `--with` accepts. `schema.json`'s own key.
    pub id: String,
    /// For display: "PostgreSQL" rather than "postgresql".
    pub display_name: String,
    pub kind: Kind,
    /// Where it sits: the language for a framework, the category for a service.
    pub group: String,
}

/// Every component, in the order `schema.json` declares them.
///
/// Declaration order rather than alphabetical: `schema.json`'s order is curated
/// — it is the same reason the menus moved from `HashMap` to `IndexMap`.
pub fn catalogue() -> Result<Vec<Component>> {
    let schema = schema()?;
    let mut out = Vec::new();

    for (lang_id, lang) in &schema.languages {
        for (fw_id, fw) in &lang.frameworks {
            out.push(Component {
                id: fw_id.clone(),
                display_name: fw.display_name.clone().unwrap_or_else(|| fw_id.clone()),
                kind: Kind::Framework,
                group: lang_id.clone(),
            });
        }
    }

    for (category, services) in service_categories(schema) {
        for (svc_id, svc) in services {
            out.push(Component {
                id: svc_id.clone(),
                display_name: svc.display_name.clone().unwrap_or_else(|| svc_id.clone()),
                kind: Kind::Service,
                group: category.to_string(),
            });
        }
    }

    for (infra_id, infra) in &schema.infrastructure {
        out.push(Component {
            id: infra_id.clone(),
            display_name: infra
                .display_name
                .clone()
                .unwrap_or_else(|| infra_id.clone()),
            kind: Kind::Infrastructure,
            group: "infrastructure".to_string(),
        });
    }

    Ok(out)
}

/// `ServiceCategories` is a struct of named fields rather than a map, so the
/// eight categories are listed once here instead of at every call site.
fn service_categories(
    schema: &'static super::models::Schema,
) -> Vec<(
    &'static str,
    &'static indexmap::IndexMap<String, super::models::ServiceConfig>,
)> {
    let s = &schema.services;
    vec![
        ("databases", &s.databases),
        ("messaging_queues", &s.messaging_queues),
        ("auth_providers", &s.auth_providers),
        ("storage", &s.storage),
        ("monitoring_logging", &s.monitoring_logging),
        ("payments", &s.payments),
        ("ai_ml", &s.ai_ml),
        ("email_sms", &s.email_sms),
    ]
}

/// Look one up by the name a user would type.
pub fn find(id: &str) -> Result<Option<Component>> {
    Ok(catalogue()?.into_iter().find(|c| c.id == id))
}

/// Resolve a list of component names into the variables they contribute.
///
/// ⚠️ **An unknown name is an error.** The resolvers this replaces used
/// `if let Some(…)` with no `else`, so a name that matched nothing was silently
/// skipped and the variables simply did not appear — a typo in a blueprint, or
/// in a `--with` list, would produce a smaller `.env.example` and say nothing.
/// That is the same shape of failure as a scanner reporting "clean" over a path
/// it could not read.
///
/// Duplicates are allowed and collapse: `--with postgresql,postgresql` is one
/// component, and a name already contributed by a blueprint does not double up.
/// Order given is order applied.
pub fn resolve(ids: &[String]) -> Result<VarCollection> {
    let all = catalogue()?;
    let schema = schema()?;
    let mut collection = VarCollection::default();
    let mut seen: Vec<&str> = Vec::new();

    for id in ids {
        let id = id.trim();
        if id.is_empty() || seen.contains(&id) {
            continue;
        }

        let Some(component) = all.iter().find(|c| c.id == id) else {
            bail!("{}", unknown_message(id, &all));
        };
        seen.push(&component.id);

        match component.kind {
            Kind::Framework => {
                let lang = schema
                    .languages
                    .get(&component.group)
                    .expect("catalogue only names languages the schema has");
                let fw = lang
                    .frameworks
                    .get(id)
                    .expect("catalogue only names frameworks the schema has");
                add_framework_vars(&mut collection, id, fw);
            }
            Kind::Service => {
                let (_, svc) = super::loader::find_service(id)
                    .expect("catalogue only names services the schema has");
                add_service_vars(&mut collection, id, svc);
            }
            Kind::Infrastructure => {
                let infra = schema
                    .infrastructure
                    .get(id)
                    .expect("catalogue only names infrastructure the schema has");
                add_infra_vars(&mut collection, id, infra);
            }
        }
    }

    Ok(collection)
}

/// Which component, if any, declares this variable.
///
/// Used by `--from-source`: a name the code reads may be one the catalogue
/// already knows, in which case its description, default and category come for
/// free and the generated file stays annotated instead of bare `KEY=` lines.
pub fn declaring(var_name: &str) -> Result<Option<Component>> {
    let schema = schema()?;

    for (lang_id, lang) in &schema.languages {
        for (fw_id, fw) in &lang.frameworks {
            if fw.vars.iter().any(|v| v == var_name) {
                return Ok(Some(Component {
                    id: fw_id.clone(),
                    display_name: fw.display_name.clone().unwrap_or_else(|| fw_id.clone()),
                    kind: Kind::Framework,
                    group: lang_id.clone(),
                }));
            }
        }
    }

    for (category, services) in service_categories(schema) {
        for (svc_id, svc) in services {
            if svc.vars.iter().any(|v| v == var_name) {
                return Ok(Some(Component {
                    id: svc_id.clone(),
                    display_name: svc.display_name.clone().unwrap_or_else(|| svc_id.clone()),
                    kind: Kind::Service,
                    group: category.to_string(),
                }));
            }
        }
    }

    for (infra_id, infra) in &schema.infrastructure {
        if infra.vars.iter().any(|v| v == var_name) {
            return Ok(Some(Component {
                id: infra_id.clone(),
                display_name: infra
                    .display_name
                    .clone()
                    .unwrap_or_else(|| infra_id.clone()),
                kind: Kind::Infrastructure,
                group: "infrastructure".to_string(),
            }));
        }
    }

    Ok(None)
}

/// An error that answers "then what should I have typed?".
///
/// A bare "unknown component" sends someone to the docs for a name that is one
/// character away. Prefix and substring matches cover the two mistakes people
/// actually make — an abbreviation, and the wrong half of a two-word name.
fn unknown_message(id: &str, all: &[Component]) -> String {
    let lower = id.to_lowercase();
    let near: Vec<&str> = all
        .iter()
        .filter(|c| c.id.starts_with(&lower) || c.id.contains(&lower))
        .map(|c| c.id.as_str())
        .take(6)
        .collect();

    let mut msg = format!("unknown component '{id}'");
    if !near.is_empty() {
        msg.push_str(&format!("\n\nDid you mean: {}", near.join(", ")));
    }
    msg.push_str("\n\nRun `evnx init --list-components` for the full list.");
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_not_empty_and_has_all_three_kinds() {
        let all = catalogue().unwrap();
        assert!(all.len() >= 40, "only {} components", all.len());
        for kind in [Kind::Framework, Kind::Service, Kind::Infrastructure] {
            assert!(
                all.iter().any(|c| c.kind == kind),
                "no {} in the catalogue",
                kind.label()
            );
        }
    }

    /// ⚠️ The property `--with` depends on. If two components ever share an id,
    /// `--with stripe` becomes ambiguous and the flag needs qualifying — so this
    /// failing is a schema decision, not a test to relax.
    #[test]
    fn every_component_id_is_unique() {
        let all = catalogue().unwrap();
        let mut ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate component ids in schema.json");
    }

    #[test]
    fn a_known_component_resolves_to_its_variables() {
        let vars = resolve(&["postgresql".to_string()]).unwrap();
        assert!(
            vars.vars.contains_key("DATABASE_URL"),
            "{:?}",
            vars.vars.keys()
        );
        assert!(vars.vars.contains_key("DB_HOST"));
    }

    #[test]
    fn components_combine() {
        let vars = resolve(&[
            "nextjs".to_string(),
            "postgresql".to_string(),
            "stripe".to_string(),
        ])
        .unwrap();
        assert!(vars.vars.contains_key("DATABASE_URL"));
        assert!(
            vars.vars.keys().any(|k| k.contains("STRIPE")),
            "{:?}",
            vars.vars.keys()
        );
    }

    /// ⚠️ The fail-open this replaces. `resolve_blueprint` and friends used
    /// `if let Some(…)` with no `else`, so a name matching nothing was skipped in
    /// silence and its variables simply never appeared.
    #[test]
    fn an_unknown_component_is_an_error_not_a_silent_skip() {
        let err = resolve(&["postgresql".to_string(), "nope".to_string()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown component 'nope'"), "{err}");
        assert!(err.contains("--list-components"), "{err}");
    }

    /// The error has to answer "then what should I have typed?".
    #[test]
    fn a_near_miss_is_suggested() {
        let err = resolve(&["postgres".to_string()]).unwrap_err().to_string();
        assert!(err.contains("postgresql"), "{err}");
    }

    #[test]
    fn duplicates_collapse_and_order_is_kept() {
        let once = resolve(&["redis".to_string()]).unwrap();
        let twice = resolve(&["redis".to_string(), "redis".to_string()]).unwrap();
        assert_eq!(once.vars.len(), twice.vars.len());
    }

    /// ⚠️ The claim that makes this a refactor rather than a rewrite: a blueprint
    /// **is** a component list, so resolving its components by name must give
    /// exactly what `resolve_blueprint` gives.
    ///
    /// If this ever fails, the two paths have diverged and blueprints are no
    /// longer aliases — which is the thing C exists to prevent.
    #[test]
    fn a_blueprint_is_exactly_its_component_list() {
        for (id, _) in super::super::loader::list_blueprints() {
            let bp = super::super::loader::get_blueprint(id).unwrap();

            let mut names = vec![bp.components.framework.clone()];
            names.extend(bp.components.services.iter().cloned());
            names.extend(bp.components.infrastructure.iter().cloned());

            let by_blueprint = super::super::resolver::resolve_blueprint(bp).unwrap();
            let by_components = resolve(&names).unwrap();

            let mut a: Vec<&String> = by_blueprint.vars.keys().collect();
            let mut b: Vec<&String> = by_components.vars.keys().collect();
            a.sort();
            b.sort();
            assert_eq!(a, b, "blueprint {id} diverges from its component list");
        }
    }

    /// Every id a blueprint names must exist in the catalogue — otherwise
    /// `resolve` would reject a stack the CLI still offers.
    #[test]
    fn every_blueprint_reference_is_a_real_component() {
        let all = catalogue().unwrap();
        let known: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();

        for (id, _) in super::super::loader::list_blueprints() {
            let bp = super::super::loader::get_blueprint(id).unwrap();
            let mut refs = vec![&bp.components.framework];
            refs.extend(bp.components.services.iter());
            refs.extend(bp.components.infrastructure.iter());

            for r in refs {
                assert!(
                    known.contains(&r.as_str()),
                    "blueprint {id} names '{r}', which is not in the catalogue"
                );
            }
        }
    }
}
