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
use super::models::{VarCollection, VarSource};
use super::resolver::{add_framework_vars, add_infra_vars, add_service_vars};
use crate::utils::string::suggest;

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

/// Names people type for a component whose id is something else.
///
/// The ids are a compatibility surface (see the module docs) and stay as they
/// are; these are accepted *in addition*, resolve to exactly the same component,
/// and never appear in output. Several ids are two products sharing one set of
/// variables (`express_fastify`), so either half is a natural thing to type.
///
/// ⚠️ Every entry is a name evnx has to keep accepting. Add one only when it is
/// unambiguous, and never one that is also a real id (a test enforces both).
/// A table here rather than a field in `schema.json`: the schema's types are
/// public library API, and a new field would change them (DEC-SP12, I-001).
pub const ALIASES: &[(&str, &str)] = &[
    ("postgres", "postgresql"),
    ("pg", "postgresql"),
    ("mysql", "mysql_mariadb"),
    ("mariadb", "mysql_mariadb"),
    ("mongo", "mongodb"),
    ("express", "express_fastify"),
    ("fastify", "express_fastify"),
    ("fastapi", "fastapi_flask"),
    ("flask", "fastapi_flask"),
    ("axum", "axum_actix"),
    ("actix", "axum_actix"),
    ("gin", "gin_echo"),
    ("echo", "gin_echo"),
    ("next", "nextjs"),
    ("k8s", "kubernetes"),
];

/// The prefix a framework's browser bundle needs, by framework id.
///
/// A browser framework only exposes variables with its own prefix to client
/// code. Laravel is here because its Vite integration exposes `VITE_*` to the
/// frontend (decision 2026-10-13). Server frameworks are absent: they read
/// every variable directly.
pub const CLIENT_PREFIX: &[(&str, &str)] = &[
    ("vite", "VITE_"),
    ("nextjs", "NEXT_PUBLIC_"),
    ("sveltekit", "PUBLIC_"),
    ("nuxt", "NUXT_PUBLIC_"),
    ("laravel", "VITE_"),
];

/// Service variables their vendor designs to be shipped to the browser.
///
/// ⚠️ Only these get a client-prefixed copy (I-002). Each is public by design:
/// a project URL, a publishable or anon key, an OAuth client id, a DSN, the
/// Firebase web config. Secrets never appear here, and a test fails if a name
/// that looks like one is added. The reasoning for each entry, and for what was
/// left out, is in the review package (`PROPOSAL-H-client-variables.md`).
pub const CLIENT_SAFE: &[&str] = &[
    "SUPABASE_URL",
    "SUPABASE_ANON_KEY",
    "FIREBASE_API_KEY",
    "FIREBASE_PROJECT_ID",
    "FIREBASE_AUTH_DOMAIN",
    "FIREBASE_STORAGE_BUCKET",
    "AUTH0_DOMAIN",
    "AUTH0_CLIENT_ID",
    "AUTH0_AUDIENCE",
    "CLERK_PUBLISHABLE_KEY",
    "GOOGLE_CLIENT_ID",
    "SENTRY_DSN",
    "STRIPE_PUBLIC_KEY",
    "RAZORPAY_KEY_ID",
    "CLOUDINARY_CLOUD_NAME",
];

/// Drop a service's browser name for a framework the stack does not use
/// (I-002b, DQ-23).
///
/// ⚠️ Clerk lists `NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY` and Vercel
/// `NEXT_PUBLIC_VERCEL_ENV` in the catalogue, so a Vite, Nuxt or Express stack
/// got a Next.js name its code can never read, beside the `VITE_` copy it can.
/// A service or infrastructure variable carrying a framework's prefix
/// ([`CLIENT_PREFIX`]) is now kept only when a framework with that prefix is in
/// `frameworks`; on another browser framework [`add_client_copies`] gives the
/// public value under that framework's own prefix. A framework's own variables
/// are never touched.
pub(crate) fn browser_names_for(collection: &mut VarCollection, frameworks: &[&str]) {
    collection.vars.retain(|name, meta| {
        if matches!(meta.source, VarSource::Framework(_)) {
            return true;
        }
        let Some((_, prefix)) = CLIENT_PREFIX.iter().find(|(_, p)| name.starts_with(p)) else {
            return true;
        };
        CLIENT_PREFIX
            .iter()
            .any(|(id, p)| p == prefix && frameworks.contains(id))
    });
}

/// Add the client-prefixed copy of each browser-safe variable in `collection`,
/// for a framework that needs one (I-002, phase A of DQ-2).
///
/// ⚠️ Additive only. A Vite + Supabase stack used to get `SUPABASE_URL` and not
/// the `VITE_SUPABASE_URL` the browser can read. The original stays (server code
/// reads it); the copy is added beside it with the same placeholder and
/// category; a name that already exists is left alone (Clerk already carries
/// `NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY`). Called by both resolvers, so `--with`,
/// a blueprint and the Architect menu produce the same file.
pub(crate) fn add_client_copies(collection: &mut VarCollection, framework_id: &str) {
    let Some((_, prefix)) = CLIENT_PREFIX.iter().find(|(id, _)| *id == framework_id) else {
        return;
    };
    let framework = schema()
        .ok()
        .and_then(|s| {
            s.languages
                .values()
                .find_map(|l| l.frameworks.get(framework_id))
                .and_then(|f| f.display_name.clone())
        })
        .unwrap_or_else(|| framework_id.to_string());

    let copies: Vec<(String, super::models::VarMetadata)> = CLIENT_SAFE
        .iter()
        .filter_map(|name| {
            let meta = collection.vars.get(*name)?;
            let copy = format!("{prefix}{name}");
            if collection.vars.contains_key(&copy) {
                return None;
            }
            let mut meta = meta.clone();
            meta.description = Some(format!(
                "Client-side copy of {name} (exposed to the browser by {framework})"
            ));
            Some((copy, meta))
        })
        .collect();
    collection.vars.extend(copies);
}

/// The catalogue id for a name: the alias target if `name` is an alias, the
/// name itself otherwise. Matching is exact; case is left to the suggestions.
pub fn canonical(name: &str) -> &str {
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, id)| *id)
        .unwrap_or(name)
}

/// Look one up by the name a user would type (an id or an alias).
pub fn find(id: &str) -> Result<Option<Component>> {
    let id = canonical(id);
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
        // Canonical first, so `postgres,postgresql` collapses to one component.
        let id = canonical(id.trim());
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

    // After every component, so the result does not depend on the order the
    // framework and its services were named in.
    let frameworks: Vec<&str> = seen
        .iter()
        .copied()
        .filter(|id| all.iter().any(|c| c.id == *id && c.kind == Kind::Framework))
        .collect();
    browser_names_for(&mut collection, &frameworks);
    for framework in frameworks {
        add_client_copies(&mut collection, framework);
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
    // Prefix and substring, then edit distance for the commonest typos — a
    // transposition (`supabse`), a dropped letter (`strpe`), a doubled one
    // (`stripee`) — with aliases suggesting the id they stand for (I-001). The
    // same rule now answers an unknown blueprint (I-235).
    let ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
    let near = suggest(&lower, &ids, ALIASES);

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
        // A prefix (was `postgres`, which is now an alias and resolves).
        let err = resolve(&["postgre".to_string()]).unwrap_err().to_string();
        assert!(err.contains("Did you mean: postgresql"), "{err}");
    }

    /// T-INIT-09 (I-001): typos a prefix match cannot see.
    #[test]
    fn a_typo_is_suggested() {
        for (typo, want) in [
            ("supabse", "supabase"), // missing letter / swap
            ("strpe", "stripe"),     // dropped letter
            ("stripee", "stripe"),   // doubled letter
            ("postgrsql", "postgresql"),
            ("rdeis", "redis"),          // adjacent swap
            ("postgress", "postgresql"), // typo of an alias suggests its id
        ] {
            let err = resolve(&[typo.to_string()]).unwrap_err().to_string();
            assert!(
                err.contains(&format!("Did you mean: {want}")),
                "{typo}: {err}"
            );
        }
        for nothing_close in ["zzzz", "kafkaesque-thing"] {
            let err = resolve(&[nothing_close.to_string()])
                .unwrap_err()
                .to_string();
            assert!(!err.contains("Did you mean"), "{nothing_close}: {err}");
        }
    }

    /// Aliases resolve to exactly what their id resolves to, collapse with it,
    /// and can never shadow or point past a real id.
    #[test]
    fn aliases_are_the_same_component() {
        let ids: Vec<String> = catalogue().unwrap().into_iter().map(|c| c.id).collect();
        for (alias, id) in ALIASES {
            assert!(
                !ids.iter().any(|i| i == alias),
                "alias {alias} is also an id"
            );
            assert!(
                ids.iter().any(|i| i == id),
                "alias {alias} points at unknown {id}"
            );
            let via_alias = resolve(&[alias.to_string()]).unwrap();
            let via_id = resolve(&[id.to_string()]).unwrap();
            // `vars` is a HashMap: compare as sets, not in iteration order.
            let names = |c: &VarCollection| {
                let mut v: Vec<_> = c.vars.keys().cloned().collect();
                v.sort();
                v
            };
            assert_eq!(names(&via_alias), names(&via_id), "{alias}");
            assert_eq!(find(alias).unwrap().unwrap().id, *id);
        }
        let both = resolve(&["postgres".to_string(), "postgresql".to_string()]).unwrap();
        let one = resolve(&["postgresql".to_string()]).unwrap();
        assert_eq!(both.vars.len(), one.vars.len());
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

    /// T-INIT-24: nothing that looks like a secret is browser-safe, every entry
    /// is a real catalogue variable, and every prefixed framework exists.
    #[test]
    fn client_safe_list_holds_no_secrets_and_no_strangers() {
        for name in CLIENT_SAFE {
            for word in ["SECRET", "PASSWORD", "PRIVATE", "SERVICE_ROLE", "TOKEN"] {
                assert!(!name.contains(word), "{name} looks like a secret");
            }
            assert!(
                declaring(name).unwrap().is_some(),
                "{name} is not a catalogue variable"
            );
        }
        let frameworks: Vec<String> = catalogue()
            .unwrap()
            .into_iter()
            .filter(|c| c.kind == Kind::Framework)
            .map(|c| c.id)
            .collect();
        for (id, prefix) in CLIENT_PREFIX {
            assert!(frameworks.iter().any(|f| f == id), "unknown framework {id}");
            assert!(prefix.ends_with('_'), "{prefix}");
        }
    }

    /// DQ-2, phase A: adding a framework only ever adds variables. For every
    /// prefixed framework and every service, the combination keeps everything
    /// each part had on its own, and the copies carry the original's value.
    #[test]
    fn client_copies_are_additive_only() {
        let keys = |ids: &[&str]| -> std::collections::BTreeSet<String> {
            let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
            resolve(&ids).unwrap().vars.keys().cloned().collect()
        };
        let services: Vec<String> = catalogue()
            .unwrap()
            .into_iter()
            .filter(|c| c.kind == Kind::Service)
            .map(|c| c.id)
            .collect();
        for (framework, prefix) in CLIENT_PREFIX {
            for service in &services {
                let both = resolve(&[framework.to_string(), service.clone()]).unwrap();
                let names: std::collections::BTreeSet<String> = both.vars.keys().cloned().collect();
                for k in keys(&[framework]).union(&keys(&[service])) {
                    assert!(names.contains(k), "{framework}+{service} lost {k}");
                }
                for name in CLIENT_SAFE {
                    let copy = format!("{prefix}{name}");
                    // Only the copies this adds; a name the catalogue already
                    // had (Clerk's `NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY`) is left alone.
                    let added = both.vars.get(&copy).filter(|m| {
                        m.description
                            .as_deref()
                            .is_some_and(|d| d.starts_with("Client-side copy of"))
                    });
                    if let Some(meta) = added {
                        let original = both.vars.get(*name).expect("copy without original");
                        assert_eq!(meta.example_value, original.example_value, "{copy}");
                    }
                }
            }
        }
    }

    /// The copies do not depend on the order the components were named in.
    #[test]
    fn client_copies_do_not_depend_on_order() {
        let names = |ids: &[&str]| -> Vec<String> {
            let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
            let mut v: Vec<String> = resolve(&ids).unwrap().vars.keys().cloned().collect();
            v.sort();
            v
        };
        assert_eq!(names(&["vite", "supabase"]), names(&["supabase", "vite"]));
        assert!(names(&["supabase", "vite"]).contains(&"VITE_SUPABASE_URL".to_string()));
    }
}
