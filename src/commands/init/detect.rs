//! Read the project, so `init` stops asking about a stack already on disk.
//!
//! # What this is, and what it is not
//!
//! A **heuristic that proposes**. Detection never applies itself: a `stripe`
//! dependency in a test fixture would otherwise add payment variables to a
//! project that has no payments, and a dependency table is wrong the week a
//! framework renames its package. Everything here is shown and confirmed.
//!
//! That is also why the table is small and obvious rather than clever. It maps
//! package names to the component ids in [`crate::schema::component`], and the
//! answer is a list of those ids — which C's resolver already knows how to turn
//! into variables. Detection contributes recognition, nothing else.
//!
//! # Signals, in descending order of how much they can be trusted
//!
//! 1. **Dependency manifests** — `package.json`, `pyproject.toml`,
//!    `requirements.txt`, `Cargo.toml`, `go.mod`, `composer.json`, `Gemfile`.
//!    A declared dependency is a strong signal: the project compiles against it.
//! 2. **`docker-compose.yml`** — the backing services, which is exactly what
//!    `init` currently makes people click through.
//! 3. **Marker files** — a `Dockerfile`, `.github/workflows/`, `vercel.json`.
//!
//! ⚠️ Source scanning is deliberately absent. A naive `env::var("…")` sweep
//! found 6 of 21 variables on this workspace's own server, because config there
//! goes through `required_var!` macros — and every project past its first week
//! wraps env access somehow. That is Proposal B's problem, with its own
//! false-positive budget, and it does not belong in the one-keystroke path.

use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// One component the project appears to use, and why we think so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    /// A component id from [`crate::schema::component`].
    pub component: String,
    /// The file that said so — shown to the user, because a proposal they cannot
    /// check is a proposal they have to take on faith.
    pub evidence: String,
    /// The exact token matched: `next`, `postgres:16`.
    pub matched: String,
}

/// Package name → component id.
///
/// ⚠️ A maintenance treadmill, and a known one. It is wrong the week a framework
/// renames its package, which is the cost of the fastest path being one
/// keystroke. Wrong here means *missing*, not *incorrect*: an unrecognised
/// dependency proposes nothing, and the user reaches for `--with`.
///
/// Ordered by ecosystem so an entry is easy to find and easy to add.
fn package_table() -> BTreeMap<&'static str, &'static str> {
    [
        // ── JavaScript / TypeScript ──
        ("next", "nextjs"),
        ("nuxt", "nuxt"),
        ("vite", "vite"),
        ("express", "express_fastify"),
        ("fastify", "express_fastify"),
        ("@nestjs/core", "nest"),
        ("@sveltejs/kit", "sveltekit"),
        // ── Python ──
        ("django", "django"),
        ("fastapi", "fastapi_flask"),
        ("flask", "fastapi_flask"),
        // ── Rust ──
        ("axum", "axum_actix"),
        ("actix-web", "axum_actix"),
        // ── Go ──
        ("github.com/gin-gonic/gin", "gin_echo"),
        ("github.com/labstack/echo/v4", "gin_echo"),
        // ── Ruby / PHP / JVM ──
        ("rails", "rails"),
        ("laravel/framework", "laravel"),
        ("org.springframework.boot", "spring_boot"),
        // ── Databases and caches ──
        ("pg", "postgresql"),
        ("psycopg2", "postgresql"),
        ("psycopg2-binary", "postgresql"),
        ("asyncpg", "postgresql"),
        ("mysql2", "mysql_mariadb"),
        ("mongodb", "mongodb"),
        ("mongoose", "mongodb"),
        ("redis", "redis"),
        ("ioredis", "redis"),
        ("@elastic/elasticsearch", "elasticsearch"),
        ("@supabase/supabase-js", "supabase"),
        ("firebase", "firebase"),
        // ── Messaging ──
        ("kafkajs", "kafka"),
        ("amqplib", "rabbitmq"),
        ("pika", "rabbitmq"),
        // ── Auth, payments, storage ──
        ("@clerk/nextjs", "clerk"),
        ("@clerk/clerk-sdk-node", "clerk"),
        ("@auth0/auth0-react", "auth0"),
        ("stripe", "stripe"),
        ("@lemonsqueezy/lemonsqueezy.js", "lemon_squeezy"),
        ("razorpay", "razorpay"),
        ("@aws-sdk/client-s3", "aws_s3"),
        ("boto3", "aws_s3"),
        ("cloudinary", "cloudinary"),
        ("uploadthing", "uploadthing"),
        // ── Observability ──
        ("@sentry/node", "sentry"),
        ("@sentry/nextjs", "sentry"),
        ("sentry-sdk", "sentry"),
        ("@datadog/browser-logs", "datadog"),
        // ── AI ──
        ("openai", "openai_api"),
        ("@anthropic-ai/sdk", "anthropic"),
        ("anthropic", "anthropic"),
        ("@huggingface/inference", "huggingface"),
        // ── Email / SMS ──
        ("resend", "resend_email"),
        ("@sendgrid/mail", "sendgrid"),
        ("twilio", "twilio"),
    ]
    .into_iter()
    .collect()
}

/// Docker image name → component id, matched on the part before any tag.
fn image_table() -> BTreeMap<&'static str, &'static str> {
    [
        ("postgres", "postgresql"),
        ("postgis/postgis", "postgresql"),
        ("mysql", "mysql_mariadb"),
        ("mariadb", "mysql_mariadb"),
        ("mongo", "mongodb"),
        ("redis", "redis"),
        // Valkey is a drop-in Redis replacement and shares its variables.
        ("valkey/valkey", "redis"),
        ("elasticsearch", "elasticsearch"),
        (
            "docker.elastic.co/elasticsearch/elasticsearch",
            "elasticsearch",
        ),
        ("rabbitmq", "rabbitmq"),
        ("confluentinc/cp-kafka", "kafka"),
        ("apache/kafka", "kafka"),
    ]
    .into_iter()
    .collect()
}

/// Everything the project appears to use, deduplicated, in detection order.
pub fn detect(root: &Path) -> Result<Vec<Detected>> {
    let mut found: Vec<Detected> = Vec::new();

    let push = |component: &str, evidence: &str, matched: &str, out: &mut Vec<Detected>| {
        if out.iter().any(|d| d.component == component) {
            return;
        }
        out.push(Detected {
            component: component.to_string(),
            evidence: evidence.to_string(),
            matched: matched.to_string(),
        });
    };

    for (dep, evidence) in dependencies(root) {
        if let Some(component) = package_table().get(dep.as_str()) {
            push(component, &evidence, &dep, &mut found);
        }
    }

    for (image, evidence) in compose_images(root) {
        // Match the repository part, so `postgres:16` and `postgres` agree.
        let repo = image.split(':').next().unwrap_or(&image).to_string();
        if let Some(component) = image_table().get(repo.as_str()) {
            push(component, &evidence, &image, &mut found);
        }
    }

    for (marker, component) in [
        (".github/workflows", "github_actions"),
        ("vercel.json", "vercel"),
        ("Dockerfile", "docker"),
    ] {
        if root.join(marker).exists() {
            push(component, marker, marker, &mut found);
        }
    }

    Ok(found)
}

/// Declared dependency names, paired with the file that declared them.
///
/// Every manifest is read if present: a repository with both `package.json` and
/// `pyproject.toml` is one project with two halves, not a contradiction.
fn dependencies(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();

    // package.json — dependencies and devDependencies
    if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(json) = serde_json::from_str::<Value>(&text) {
            for field in ["dependencies", "devDependencies"] {
                if let Some(map) = json.get(field).and_then(Value::as_object) {
                    out.extend(map.keys().map(|k| (k.clone(), "package.json".into())));
                }
            }
        }
    }

    // Cargo.toml — dependencies table keys
    if let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) {
        if let Ok(doc) = toml::from_str::<toml::Value>(&text) {
            if let Some(map) = doc.get("dependencies").and_then(toml::Value::as_table) {
                out.extend(map.keys().map(|k| (k.clone(), "Cargo.toml".into())));
            }
        }
    }

    // requirements.txt — one name per line, before any version specifier
    if let Ok(text) = std::fs::read_to_string(root.join("requirements.txt")) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let name = line
                .split(['=', '>', '<', '~', '!', '[', ';', ' '])
                .next()
                .unwrap_or(line)
                .trim()
                .to_lowercase();
            if !name.is_empty() {
                out.push((name, "requirements.txt".into()));
            }
        }
    }

    // pyproject.toml — PEP 621 and Poetry both
    if let Ok(text) = std::fs::read_to_string(root.join("pyproject.toml")) {
        if let Ok(doc) = toml::from_str::<toml::Value>(&text) {
            if let Some(list) = doc
                .get("project")
                .and_then(|p| p.get("dependencies"))
                .and_then(toml::Value::as_array)
            {
                for entry in list.iter().filter_map(toml::Value::as_str) {
                    let name = entry
                        .split(['=', '>', '<', '~', '!', '[', ';', ' '])
                        .next()
                        .unwrap_or(entry)
                        .trim()
                        .to_lowercase();
                    out.push((name, "pyproject.toml".into()));
                }
            }
            if let Some(map) = doc
                .get("tool")
                .and_then(|t| t.get("poetry"))
                .and_then(|p| p.get("dependencies"))
                .and_then(toml::Value::as_table)
            {
                out.extend(
                    map.keys()
                        .map(|k| (k.to_lowercase(), "pyproject.toml".into())),
                );
            }
        }
    }

    // go.mod — the module path on each require line
    if let Ok(text) = std::fs::read_to_string(root.join("go.mod")) {
        for line in text.lines() {
            let line = line.trim().trim_start_matches("require ").trim();
            if let Some(path) = line.split_whitespace().next() {
                if path.contains('/') {
                    out.push((path.to_string(), "go.mod".into()));
                }
            }
        }
    }

    // composer.json / Gemfile
    if let Ok(text) = std::fs::read_to_string(root.join("composer.json")) {
        if let Ok(json) = serde_json::from_str::<Value>(&text) {
            if let Some(map) = json.get("require").and_then(Value::as_object) {
                out.extend(map.keys().map(|k| (k.clone(), "composer.json".into())));
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(root.join("Gemfile")) {
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("gem ") {
                let name = rest.trim_matches(|c: char| c == '\'' || c == '"' || c == ',');
                let name = name.split(['\'', '"', ',']).next().unwrap_or(name);
                out.push((name.to_string(), "Gemfile".into()));
            }
        }
    }

    out
}

/// Images under `services:` in a compose file.
///
/// ⚠️ **`services:`, never the document root.** A top-level key scan returns the
/// `volumes:` entries too — on this workspace's own compose file that is
/// `postgres_data`, `valkey_data`, `localstack_data`, which look enough like
/// service names to be proposed and are not services at all.
fn compose_images(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();

    for name in ["docker-compose.yml", "docker-compose.yaml", "compose.yml"] {
        let Ok(text) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        let Some(services) = doc.get("services").and_then(|s| s.as_mapping()) else {
            continue;
        };

        for (key, spec) in services {
            // `image:` when given; otherwise the service's own name, which is
            // how people write `postgres:` with a build or a default.
            let image = spec
                .get("image")
                .and_then(|i| i.as_str())
                .map(str::to_string)
                .or_else(|| key.as_str().map(str::to_string));
            if let Some(image) = image {
                out.push((image, name.to_string()));
            }
        }
        break;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn project(files: &[(&str, &str)]) -> TempDir {
        let d = TempDir::new().unwrap();
        for (name, body) in files {
            let path = d.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, body).unwrap();
        }
        d
    }

    /// Owned, so a caller can write `ids(&detect(..)?)` without the borrow
    /// outliving the temporary.
    fn ids(found: &[Detected]) -> Vec<String> {
        found.iter().map(|d| d.component.clone()).collect()
    }

    #[test]
    fn a_node_project_is_read_from_package_json() {
        let d = project(&[(
            "package.json",
            r#"{"dependencies":{"next":"16","stripe":"14","@clerk/nextjs":"5"}}"#,
        )]);
        let found = detect(d.path()).unwrap();
        for want in ["nextjs", "stripe", "clerk"] {
            assert!(
                ids(&found).iter().any(|c| c == want),
                "{want} missing: {:?}",
                ids(&found)
            );
        }
    }

    #[test]
    fn dev_dependencies_count_too() {
        let d = project(&[("package.json", r#"{"devDependencies":{"vite":"5"}}"#)]);
        assert!(ids(&detect(d.path()).unwrap()).iter().any(|c| c == "vite"));
    }

    /// ⚠️ The compose trap. A top-level key scan returns `volumes:` entries as
    /// well — `postgres_data` reads enough like a service to be proposed, and is
    /// not one.
    #[test]
    fn compose_reads_services_and_not_volumes() {
        let d = project(&[(
            "docker-compose.yml",
            "services:\n  postgres:\n    image: postgres:16\n  cache:\n    image: valkey/valkey:9-alpine\nvolumes:\n  postgres_data:\n  mysql_data:\n",
        )]);
        let found = detect(d.path()).unwrap();
        assert!(
            ids(&found).iter().any(|c| c == "postgresql"),
            "{:?}",
            ids(&found)
        );
        assert!(
            ids(&found).iter().any(|c| c == "redis"),
            "valkey is redis-compatible"
        );
        assert!(
            !ids(&found).iter().any(|c| c == "mysql_mariadb"),
            "mysql_data is a volume, not a service: {:?}",
            ids(&found)
        );
    }

    /// A service with no `image:` is named by its key, which is how people write
    /// `postgres:` with a build section.
    #[test]
    fn a_service_without_an_image_is_named_by_its_key() {
        let d = project(&[("docker-compose.yml", "services:\n  redis:\n    build: .\n")]);
        assert!(ids(&detect(d.path()).unwrap()).iter().any(|c| c == "redis"));
    }

    #[test]
    fn python_is_read_from_both_manifest_shapes() {
        let reqs = project(&[(
            "requirements.txt",
            "Django>=5.0\n# a comment\npsycopg2-binary==2.9\n",
        )]);
        let found = ids(&detect(reqs.path()).unwrap());
        assert!(found.iter().any(|c| c == "django"), "{found:?}");
        assert!(found.iter().any(|c| c == "postgresql"), "{found:?}");

        let pyproject = project(&[(
            "pyproject.toml",
            "[project]\nname = \"x\"\ndependencies = [\"fastapi>=0.110\", \"redis\"]\n",
        )]);
        let found = ids(&detect(pyproject.path()).unwrap());
        assert!(found.iter().any(|c| c == "fastapi_flask"), "{found:?}");
        assert!(found.iter().any(|c| c == "redis"), "{found:?}");
    }

    #[test]
    fn marker_files_are_signals_too() {
        let d = project(&[
            (".github/workflows/ci.yml", "on: push\n"),
            ("Dockerfile", "FROM rust:1\n"),
        ]);
        let found = ids(&detect(d.path()).unwrap());
        assert!(found.iter().any(|c| c == "github_actions"), "{found:?}");
        assert!(found.iter().any(|c| c == "docker"), "{found:?}");
    }

    #[test]
    fn an_empty_project_detects_nothing() {
        let d = project(&[]);
        assert!(detect(d.path()).unwrap().is_empty());
    }

    /// Two manifests naming the same thing propose it once.
    #[test]
    fn a_component_is_proposed_at_most_once() {
        let d = project(&[
            (
                "package.json",
                r#"{"dependencies":{"redis":"4","ioredis":"5"}}"#,
            ),
            (
                "docker-compose.yml",
                "services:\n  redis:\n    image: redis:7\n",
            ),
        ]);
        let found = ids(&detect(d.path()).unwrap());
        assert_eq!(
            found.iter().filter(|c| *c == "redis").count(),
            1,
            "{found:?}"
        );
    }

    /// ⚠️ Every id this proposes must exist in the catalogue, or `--with` would
    /// reject what `init` just offered.
    #[test]
    fn every_detectable_component_is_in_the_catalogue() {
        let known: Vec<String> = crate::schema::component::catalogue()
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();

        for (pkg, component) in package_table() {
            assert!(
                known.iter().any(|c| c == component),
                "package '{pkg}' maps to '{component}', which is not a component"
            );
        }
        for (image, component) in image_table() {
            assert!(
                known.iter().any(|c| c == component),
                "image '{image}' maps to '{component}', which is not a component"
            );
        }
    }

    /// Garbage in a manifest must not panic — a half-written `package.json` is a
    /// normal thing to find mid-edit.
    #[test]
    fn a_malformed_manifest_is_ignored_not_fatal() {
        let d = project(&[
            ("package.json", "{ not json at all"),
            ("docker-compose.yml", ":::: not yaml"),
            ("Cargo.toml", "[[[["),
        ]);
        assert!(detect(d.path()).is_ok());
    }
}
