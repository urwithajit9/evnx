//! What differs between the local file and a Vercel environment, as a plan.
//!
//! A Vercel variable is a **key plus a set of targets**, and one key can have
//! several entries with different targets, so the first job is to pick, for the
//! chosen environment, the entry whose `target` contains it ([`select`]).
//!
//! The plan never holds a value it shows: rows carry keys, kinds and actions.
//! The values it does hold (a Config value read from Vercel, for comparing or
//! pulling) stay in [`SecretString`]s and are never formatted.

use indexmap::IndexMap;
use serde_json::json;

use super::api::{Kind, RemoteVar};
use crate::platform::secret::SecretString;

/// A Vercel environment: one of the three built in, or a project's custom one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Environment {
    /// `production`.
    Production,
    /// `preview`.
    Preview,
    /// `development` (what `vercel env pull` reads).
    Development,
    /// A custom environment, by slug. Its id is looked up when the session opens.
    Custom(CustomEnv),
}

/// A custom environment (`--env-name <slug>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomEnv {
    /// The slug the person typed, and Vercel lists.
    pub slug: String,
    /// Vercel's id for it (`env_…`): what an entry's `customEnvironmentIds`
    /// holds. `None` until [`Session::open`](super::Session::open) has looked it up.
    pub id: Option<String>,
}

impl Environment {
    /// The name: Vercel's in `target` for a built-in one, the slug otherwise.
    pub fn name(&self) -> &str {
        match self {
            Environment::Production => "production",
            Environment::Preview => "preview",
            Environment::Development => "development",
            Environment::Custom(c) => &c.slug,
        }
    }

    /// Whether this is a custom environment.
    pub fn is_custom(&self) -> bool {
        matches!(self, Environment::Custom(_))
    }

    /// The custom environment's id, once looked up.
    pub fn custom_id(&self) -> Option<&str> {
        match self {
            Environment::Custom(c) => c.id.as_deref(),
            _ => None,
        }
    }

    /// Whether `var` applies to this environment: its `target` names it, or
    /// (for a custom one) its `customEnvironmentIds` holds the id.
    pub fn contains(&self, var: &RemoteVar) -> bool {
        match self {
            Environment::Custom(c) => {
                c.id.as_deref()
                    .is_some_and(|id| var.custom_environment_ids.iter().any(|x| x == id))
            }
            built_in => var.target.iter().any(|t| t == built_in.name()),
        }
    }

    /// `--env-name`: `production`, `preview`, `development`, or a custom
    /// environment's slug. The slug becomes part of a file name (`.env.<slug>`),
    /// so it is limited to the characters a slug has.
    pub fn parse(s: &str) -> Result<Environment, String> {
        match s {
            "production" => return Ok(Environment::Production),
            "preview" => return Ok(Environment::Preview),
            "development" => return Ok(Environment::Development),
            _ => {}
        }
        if matches!(
            s.to_ascii_lowercase().as_str(),
            "production" | "preview" | "development"
        ) {
            return Err(format!(
                "'{s}': the built-in environments are lowercase (production, preview, development)"
            ));
        }
        let valid = !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && !s.starts_with(['.', '-']);
        if valid {
            Ok(Environment::Custom(CustomEnv {
                slug: s.to_string(),
                id: None,
            }))
        } else {
            Err(format!(
                "'{s}' is not an environment name: use production, preview, development, \
                 or the slug of a custom environment (letters, digits, - _ .)"
            ))
        }
    }
}

/// A variable of the chosen environment, and what is known of its value.
#[derive(Debug)]
pub struct Remote {
    /// The list entry (no value).
    pub var: RemoteVar,
    /// Config or Secret. A Config entry whose read returned no value counts as a
    /// Secret: Vercel will not reveal it.
    pub kind: Kind,
    /// The value, when it was read and Vercel returned one.
    pub value: Option<SecretString>,
    /// Whether the value was asked for. A diff reads only what it compares.
    pub read: bool,
    /// The entry is the preview variable for **all branches**, standing in for
    /// the `--git-branch` one the key does not have. It is read, never changed
    /// or deleted by a branch run.
    pub inherited: bool,
}

/// The entries of one environment, one per key.
#[derive(Debug, Default)]
pub struct Selection {
    /// One entry per key, in the order the list gave them.
    pub entries: Vec<RemoteVar>,
    /// Entries scoped to a preview branch that was not asked for, skipped.
    pub skipped_branch: usize,
    /// Keys with more than one entry for this environment; the newest was used.
    pub ambiguous: Vec<String>,
    /// Entries of Vercel's own system variables, skipped.
    pub skipped_system: usize,
    /// Ids of entries chosen only because the key has no entry for the branch:
    /// the all-branches preview entry (`--git-branch`).
    pub inherited: std::collections::HashSet<String>,
}

/// Pick, for each key, the entry that applies to `env`.
///
/// With `branch` (preview only), a key's entry for that branch wins; a key with
/// none falls back to its all-branches preview entry, which is marked
/// [`Selection::inherited`]. Without `branch`, branch-scoped entries are skipped.
pub fn select(vars: Vec<RemoteVar>, env: &Environment, branch: Option<&str>) -> Selection {
    let mut sel = Selection::default();
    // Per key: the entries for the branch (or for no branch), and the fallbacks.
    let mut by_key: IndexMap<String, (Vec<RemoteVar>, Vec<RemoteVar>)> = IndexMap::new();
    for v in vars {
        if v.raw_type == "system" {
            sel.skipped_system += 1;
            continue;
        }
        if !env.contains(&v) {
            continue;
        }
        // 0: this branch's entry (or the only kind there is), 1: all branches.
        let class = match (&v.git_branch, branch) {
            (None, None) => 0,
            (None, Some(_)) => 1,
            (Some(b), Some(want)) if b == want => 0,
            (Some(_), _) => {
                sel.skipped_branch += 1;
                continue;
            }
        };
        let slot = by_key.entry(v.key.clone()).or_default();
        if class == 0 {
            slot.0.push(v);
        } else {
            slot.1.push(v);
        }
    }
    for (key, (mut exact, mut fallback)) in by_key {
        let newest = |list: &mut Vec<RemoteVar>| {
            list.sort_by_key(|e| e.updated_at.as_u64().unwrap_or(0));
            list.pop()
        };
        if exact.len() > 1 {
            sel.ambiguous.push(key);
        }
        if let Some(e) = newest(&mut exact) {
            sel.entries.push(e);
        } else if let Some(e) = newest(&mut fallback) {
            sel.inherited.insert(e.id.clone());
            sel.entries.push(e);
        }
    }
    sel
}

// ── Type of a new variable ───────────────────────────────────────────────────

/// Prefixes that frameworks inline into browser bundles: such a variable is
/// public by design and must be Config (Vercel refuses a Secret there).
const PUBLIC_PREFIXES: &[&str] = &[
    "NEXT_PUBLIC_",
    "NUXT_PUBLIC_",
    "VITE_",
    "REACT_APP_",
    "VUE_APP_",
    "GATSBY_",
    "EXPO_PUBLIC_",
    "PUBLIC_",
];

/// Whether the key has a public framework prefix.
pub fn has_public_prefix(key: &str) -> bool {
    PUBLIC_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// The type evnx would give a variable it created: a Secret when the key or the
/// value looks like a credential (`core::classify`), Config otherwise; a public
/// prefix is always Config.
pub fn suggested_kind(key: &str, value: &str) -> Kind {
    if has_public_prefix(key) {
        Kind::Config
    } else if crate::core::classify::holds_secret(key, value) {
        Kind::Secret
    } else {
        Kind::Config
    }
}

// ── The diff plan ────────────────────────────────────────────────────────────

/// Why a variable on both sides is an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Config on Vercel, and the values differ.
    ValueDiffers,
    /// Secret on Vercel: whether the value changed cannot be known.
    WriteOnly,
}

/// What a row asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Local only: would be created.
    Add,
    /// On both sides, and different or not comparable.
    Update(Why),
    /// On both sides, and the same.
    Unchanged,
    /// A Secret on both sides: it exists, and its value cannot be compared. Not
    /// a difference (F1); `diff --strict` keeps it an [`Action::Update`].
    Present,
    /// On Vercel only.
    RemoteOnly,
    /// The type evnx would give it differs from the one Vercel has.
    TypeDiffers,
}

impl Action {
    /// The word in the table, and in JSON.
    pub fn word(self) -> &'static str {
        match self {
            Action::Add => "add",
            Action::Update(_) => "update",
            Action::Unchanged => "unchanged",
            Action::Present => "present",
            Action::RemoteOnly => "remote only",
            Action::TypeDiffers => "type differs",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Action::Add => "+",
            Action::Update(_) => "~",
            Action::Unchanged | Action::Present => "=",
            Action::RemoteOnly | Action::TypeDiffers => "!",
        }
    }
}

/// One line of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The variable's name.
    pub key: String,
    /// The type column: Vercel's, or for an add the one evnx would use.
    pub kind: Kind,
    /// What happens.
    pub action: Action,
    /// What Vercel has, when the row is a type difference.
    pub vercel_kind: Option<Kind>,
}

impl Row {
    /// The parenthesised remark.
    pub fn note(&self) -> Option<String> {
        match self.action {
            Action::Update(Why::ValueDiffers) => Some("value differs".into()),
            Action::Update(Why::WriteOnly) => Some("value cannot be compared: write-only".into()),
            Action::RemoteOnly => Some("not in the local file".into()),
            Action::TypeDiffers => {
                let have = self.vercel_kind.map_or("?", Kind::word);
                Some(if have == "secret" {
                    "Vercel has secret; a public prefix needs config; not changed".to_string()
                } else {
                    format!("Vercel has {have}; this looks like a secret; not changed")
                })
            }
            Action::Present => Some("write-only, not compared".into()),
            Action::Add | Action::Unchanged => None,
        }
    }
}

/// The plan: local against Vercel, one row per key.
#[derive(Debug, Default)]
pub struct Plan {
    /// Rows: the local file's keys in file order, then keys only Vercel has.
    pub rows: Vec<Row>,
}

/// Counts for the summary line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// `+`.
    pub add: usize,
    /// `~`.
    pub update: usize,
    /// `=`.
    pub unchanged: usize,
    /// `=` a Secret on both sides, not compared.
    pub present: usize,
    /// `!` remote only.
    pub remote_only: usize,
    /// `!` type differs.
    pub type_differs: usize,
}

impl Plan {
    /// Count the rows.
    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for r in &self.rows {
            match r.action {
                Action::Add => c.add += 1,
                Action::Update(_) => c.update += 1,
                Action::Unchanged => c.unchanged += 1,
                Action::Present => c.present += 1,
                Action::RemoteOnly => c.remote_only += 1,
                Action::TypeDiffers => c.type_differs += 1,
            }
        }
        c
    }

    /// Whether anything differs (everything but `=`). A Secret that is present
    /// on both sides is not a difference.
    pub fn differs(&self) -> bool {
        self.rows
            .iter()
            .any(|r| !matches!(r.action, Action::Unchanged | Action::Present))
    }

    /// The plan as `diff` reports it: a Secret on both sides, which cannot be
    /// compared, becomes [`Action::Present`] and no longer counts as a
    /// difference. `diff --strict` skips this and keeps it an update.
    pub fn presented(mut self) -> Plan {
        for r in &mut self.rows {
            if r.action == Action::Update(Why::WriteOnly) {
                r.action = Action::Present;
            }
        }
        self
    }
}

/// Compare `local` with the environment's variables.
///
/// A Secret on both sides is always an update: evnx cannot tell whether it
/// changed. A type difference is reported only where it matters: Vercel holds a
/// readable Config for something that looks like a credential, or a Secret under
/// a public prefix. A Secret where evnx would have chosen Config is left alone;
/// over-protecting is not a problem to flag.
pub fn diff(local: &IndexMap<String, String>, remote: &[Remote]) -> Plan {
    let by_key: IndexMap<&str, &Remote> = remote.iter().map(|r| (r.var.key.as_str(), r)).collect();
    let mut rows = Vec::new();
    for (key, value) in local {
        let suggested = suggested_kind(key, value);
        let Some(r) = by_key.get(key.as_str()) else {
            rows.push(Row {
                key: key.clone(),
                kind: suggested,
                action: Action::Add,
                vercel_kind: None,
            });
            continue;
        };
        let type_differs = match r.kind {
            Kind::Config => suggested == Kind::Secret,
            Kind::Secret => has_public_prefix(key),
        };
        let action = if type_differs {
            Action::TypeDiffers
        } else {
            match (r.kind, &r.value) {
                (Kind::Config, Some(v)) if v.expose() == value => Action::Unchanged,
                (Kind::Config, Some(_)) => Action::Update(Why::ValueDiffers),
                _ => Action::Update(Why::WriteOnly),
            }
        };
        rows.push(Row {
            key: key.clone(),
            kind: if type_differs { suggested } else { r.kind },
            action,
            vercel_kind: Some(r.kind),
        });
    }
    for r in remote {
        if !local.contains_key(&r.var.key) {
            rows.push(Row {
                key: r.var.key.clone(),
                kind: r.kind,
                action: Action::RemoteOnly,
                vercel_kind: Some(r.kind),
            });
        }
    }
    Plan { rows }
}

// ── Rendering ────────────────────────────────────────────────────────────────

/// The table and summary, as text. Keys, types and actions only.
pub fn render(plan: &Plan) -> String {
    use std::fmt::Write;
    let width = plan
        .rows
        .iter()
        .map(|r| r.key.len())
        .max()
        .unwrap_or(0)
        .max(8);
    let mut out = String::new();
    for r in &plan.rows {
        let note = r.note().map(|n| format!("  ({n})")).unwrap_or_default();
        let _ = writeln!(
            out,
            "  {} {:width$}  {:6}  {}{}",
            r.action.symbol(),
            r.key,
            r.kind.word(),
            r.action.word(),
            note
        );
    }
    out
}

/// `1 add · 2 update · 1 unchanged · values never shown`
pub fn summary(c: Counts) -> String {
    let mut parts = Vec::new();
    for (n, w) in [
        (c.add, "add"),
        (c.update, "update"),
        (c.unchanged, "unchanged"),
        (c.present, "present (not compared)"),
        (c.remote_only, "remote only"),
        (c.type_differs, "type differs"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {w}"));
        }
    }
    if parts.is_empty() {
        parts.push("nothing to compare".into());
    }
    parts.push("values never shown".into());
    parts.join(" · ")
}

/// The plan as JSON: keys, kinds and actions, never a value.
pub fn to_json(
    plan: &Plan,
    project: &str,
    team: Option<&str>,
    environment: &Environment,
    file: &str,
    hidden_production: u64,
    strict: bool,
) -> serde_json::Value {
    let c = plan.counts();
    json!({
        "platform": "vercel",
        "project": project,
        "team": team,
        "environment": environment.name(),
        "file": file,
        "differs": plan.differs(),
        "strict": strict,
        "hidden_production_env_count": hidden_production,
        "summary": {
            "add": c.add,
            "update": c.update,
            "unchanged": c.unchanged,
            "present": c.present,
            "remote_only": c.remote_only,
            "type_differs": c.type_differs,
        },
        "entries": plan.rows.iter().map(|r| json!({
            "key": r.key,
            "type": r.kind.word(),
            // Additive: a Secret that is present on both sides is
            // `unchanged` with `compare: "write_only"`, so a consumer that only
            // reads `action` sees "no difference", and one that wants to tell
            // "compared and equal" from "not compared" reads `compare`.
            "action": match r.action {
                Action::Present => "unchanged".to_string(),
                a => a.word().replace(' ', "_"),
            },
            "compare": match r.action {
                Action::Update(Why::ValueDiffers) => Some("value_differs"),
                Action::Update(Why::WriteOnly) | Action::Present => Some("write_only"),
                _ => None,
            },
            "vercel_type": r.vercel_kind.map(Kind::word),
        })).collect::<Vec<_>>(),
    })
}

// ── The pull plan ────────────────────────────────────────────────────────────

/// What pull does with one variable.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PullOp {
    /// Config, not in the local file: append it.
    Add,
    /// Config, local value differs: replace it.
    Update,
    /// Config, local value is the same.
    Same,
    /// Secret, the local file has a value: keep it.
    KeepLocal,
    /// Secret, not in the local file: write `KEY=` and say so.
    WriteEmpty,
}

/// One variable's fate.
#[derive(Debug)]
pub struct PullChange {
    /// The name.
    pub key: String,
    /// Its type on Vercel.
    pub kind: Kind,
    /// What pull does.
    pub op: PullOp,
    /// The value to write, for `Add` and `Update`.
    pub value: Option<SecretString>,
}

/// Work out what pull does. `remote` has been read in full. Local-only keys are
/// not listed: they are never touched.
pub fn pull_plan(local: &IndexMap<String, String>, remote: Vec<Remote>) -> Vec<PullChange> {
    remote
        .into_iter()
        .map(|r| {
            let key = r.var.key.clone();
            let local_value = local.get(&key);
            let (op, value) = match (r.kind, r.value, local_value) {
                (Kind::Config, Some(v), Some(l)) if v.expose() == l => (PullOp::Same, None),
                (Kind::Config, Some(v), Some(_)) => (PullOp::Update, Some(v)),
                (Kind::Config, Some(v), None) => (PullOp::Add, Some(v)),
                (_, _, Some(_)) => (PullOp::KeepLocal, None),
                (_, _, None) => (PullOp::WriteEmpty, None),
            };
            PullChange {
                key,
                kind: r.kind,
                op,
                value,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn var(id: &str, key: &str, kind: Kind, target: &[&str], at: u64) -> RemoteVar {
        RemoteVar {
            id: id.into(),
            key: key.into(),
            kind,
            raw_type: match kind {
                Kind::Config => "encrypted".into(),
                Kind::Secret => "sensitive".into(),
            },
            target: target.iter().map(|s| (*s).to_string()).collect(),
            git_branch: None,
            updated_at: Value::from(at),
            custom_environment_ids: vec![],
        }
    }

    fn remote(key: &str, kind: Kind, value: Option<&str>) -> Remote {
        Remote {
            var: var("id", key, kind, &["development"], 1),
            kind,
            value: value.map(SecretString::new),
            read: true,
            inherited: false,
        }
    }

    fn local(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn the_entry_whose_target_has_the_environment_is_picked() {
        let vars = vec![
            var("a", "T1", Kind::Config, &["development", "preview"], 1),
            var("b", "T1", Kind::Secret, &["production"], 2),
            var("c", "ONLY_PROD", Kind::Config, &["production"], 3),
        ];
        let dev = select(vars, &Environment::Development, None);
        assert_eq!(dev.entries.len(), 1);
        assert_eq!(dev.entries[0].id, "a");
    }

    #[test]
    fn production_gets_its_own_entry_of_the_same_key() {
        let vars = vec![
            var("a", "T1", Kind::Config, &["development", "preview"], 1),
            var("b", "T1", Kind::Secret, &["production"], 2),
        ];
        let prod = select(vars, &Environment::Production, None);
        assert_eq!(prod.entries.len(), 1);
        assert_eq!(
            (prod.entries[0].id.as_str(), prod.entries[0].kind),
            ("b", Kind::Secret)
        );
    }

    #[test]
    fn branch_scoped_and_system_entries_are_skipped_and_counted() {
        let mut branch = var("b", "K", Kind::Config, &["preview"], 1);
        branch.git_branch = Some("feature".into());
        let mut system = var("s", "VERCEL_URL", Kind::Config, &["preview"], 1);
        system.raw_type = "system".into();
        let sel = select(
            vec![branch, system, var("c", "K", Kind::Config, &["preview"], 2)],
            &Environment::Preview,
            None,
        );
        assert_eq!(
            (sel.entries.len(), sel.skipped_branch, sel.skipped_system),
            (1, 1, 1)
        );
        assert_eq!(sel.entries[0].id, "c");
    }

    #[test]
    fn a_git_branch_picks_its_entry_and_falls_back_to_the_all_branches_one() {
        let mut on_branch = var("b1", "K", Kind::Config, &["preview"], 5);
        on_branch.git_branch = Some("feature".into());
        let mut other_branch = var("b2", "K", Kind::Config, &["preview"], 6);
        other_branch.git_branch = Some("other".into());
        let all = var("a1", "K", Kind::Config, &["preview"], 1);
        let all_only = var("a2", "J", Kind::Config, &["preview"], 1);
        let mut other_only = var("b3", "L", Kind::Config, &["preview"], 1);
        other_only.git_branch = Some("other".into());
        let sel = select(
            vec![on_branch, other_branch, all, all_only, other_only],
            &Environment::Preview,
            Some("feature"),
        );
        let ids: Vec<&str> = sel.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["b1", "a2"]);
        assert_eq!(sel.skipped_branch, 2);
        assert!(sel.inherited.contains("a2") && !sel.inherited.contains("b1"));
    }

    #[test]
    fn a_custom_environment_is_found_by_id_not_by_target() {
        let mut staging = var("s1", "K", Kind::Config, &[], 1);
        staging.custom_environment_ids = vec!["env_stg".into()];
        let mut shared = var("s2", "J", Kind::Config, &["development"], 1);
        shared.custom_environment_ids = vec!["env_stg".into(), "env_other".into()];
        let other = var("o1", "L", Kind::Config, &["production"], 1);
        let env = Environment::Custom(CustomEnv {
            slug: "staging".into(),
            id: Some("env_stg".into()),
        });
        let sel = select(vec![staging, shared, other], &env, None);
        let ids: Vec<&str> = sel.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["s1", "s2"]);
        // Until the id is known nothing matches (and nothing is guessed).
        let unresolved = Environment::parse("staging").unwrap();
        assert!(select(sel.entries, &unresolved, None).entries.is_empty());
    }

    #[test]
    fn env_name_accepts_the_built_ins_and_slugs_and_nothing_path_like() {
        assert_eq!(Environment::parse("preview").unwrap(), Environment::Preview);
        assert!(Environment::parse("staging").unwrap().is_custom());
        assert!(Environment::parse("qa-2.eu_west").unwrap().is_custom());
        for bad in [
            "",
            "Production",
            "../x",
            "a/b",
            ".hidden",
            "-x",
            "a b",
            &"x".repeat(65),
        ] {
            assert!(Environment::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn two_entries_for_one_environment_use_the_newest_and_say_so() {
        let sel = select(
            vec![
                var("old", "K", Kind::Config, &["development"], 5),
                var("new", "K", Kind::Config, &["development"], 9),
            ],
            &Environment::Development,
            None,
        );
        assert_eq!(sel.entries[0].id, "new");
        assert_eq!(sel.ambiguous, ["K"]);
    }

    #[test]
    fn every_action_is_produced() {
        let l = local(&[
            ("NEW_ONE", "1"),
            ("DATABASE_URL", "postgres://x"),
            ("API_URL", "http://b"),
            ("SAME", "x"),
            ("AUTH_SECRET", "hunter2"),
        ]);
        let r = vec![
            remote("DATABASE_URL", Kind::Secret, None),
            remote("API_URL", Kind::Config, Some("http://a")),
            remote("SAME", Kind::Config, Some("x")),
            remote("AUTH_SECRET", Kind::Config, Some("hunter2")),
            remote("REDIS_URL", Kind::Secret, None),
        ];
        let p = diff(&l, &r);
        let act = |k: &str| p.rows.iter().find(|r| r.key == k).unwrap().action;
        assert_eq!(act("NEW_ONE"), Action::Add);
        assert_eq!(act("DATABASE_URL"), Action::Update(Why::WriteOnly));
        assert_eq!(act("API_URL"), Action::Update(Why::ValueDiffers));
        assert_eq!(act("SAME"), Action::Unchanged);
        assert_eq!(act("AUTH_SECRET"), Action::TypeDiffers);
        assert_eq!(act("REDIS_URL"), Action::RemoteOnly);
        assert!(p.differs());
        assert_eq!(
            p.counts(),
            Counts {
                add: 1,
                update: 2,
                unchanged: 1,
                present: 0,
                remote_only: 1,
                type_differs: 1
            }
        );
    }

    #[test]
    fn a_secret_under_a_public_prefix_is_a_type_difference() {
        let p = diff(
            &local(&[("NEXT_PUBLIC_X", "1")]),
            &[remote("NEXT_PUBLIC_X", Kind::Secret, None)],
        );
        assert_eq!(p.rows[0].action, Action::TypeDiffers);
        assert_eq!(p.rows[0].kind, Kind::Config);
    }

    #[test]
    fn a_public_prefix_is_config_even_when_the_value_looks_secret() {
        assert_eq!(
            suggested_kind("NEXT_PUBLIC_API_KEY", "sk_live_abcdefghijklmnop"),
            Kind::Config
        );
        assert_eq!(suggested_kind("STRIPE_SECRET_KEY", "x"), Kind::Secret);
        assert_eq!(suggested_kind("LOG_LEVEL", "debug"), Kind::Config);
    }

    #[test]
    fn a_secret_on_both_sides_is_present_not_a_difference_unless_strict() {
        let p = diff(&local(&[("DB", "x")]), &[remote("DB", Kind::Secret, None)]);
        // As `diff --strict` sees it.
        assert!(p.differs());
        let p = p.presented();
        assert_eq!(p.rows[0].action, Action::Present);
        assert!(!p.differs());
        assert_eq!(p.counts().present, 1);
        assert_eq!(p.counts().update, 0);
        let text = format!("{}{}", render(&p), summary(p.counts()));
        assert!(
            text.contains("= DB") && text.contains("present  (write-only, not compared)"),
            "{text}"
        );
        let j = to_json(&p, "prj", None, &Environment::Development, ".env", 0, false);
        assert_eq!(j["entries"][0]["action"], "unchanged");
        assert_eq!(j["entries"][0]["compare"], "write_only");
        assert_eq!(j["summary"]["present"], 1);
        assert_eq!(j["strict"], false);
        // A real difference next to it still differs.
        let p = diff(
            &local(&[("DB", "x"), ("NEW", "1")]),
            &[remote("DB", Kind::Secret, None)],
        )
        .presented();
        assert!(p.differs());
    }

    #[test]
    fn nothing_differing_is_clean() {
        let p = diff(
            &local(&[("A", "1")]),
            &[remote("A", Kind::Config, Some("1"))],
        );
        assert!(!p.differs());
    }

    #[test]
    fn the_rendering_holds_no_value() {
        let l = local(&[("API_URL", "very-secret-local"), ("DB", "very-secret-db")]);
        let r = vec![
            remote("API_URL", Kind::Config, Some("very-secret-remote")),
            remote("DB", Kind::Secret, None),
        ];
        let p = diff(&l, &r);
        let text = format!("{}{}", render(&p), summary(p.counts()));
        assert!(!text.contains("very-secret"), "{text}");
        let j = to_json(
            &p,
            "prj",
            Some("team"),
            &Environment::Development,
            ".env",
            0,
            false,
        )
        .to_string();
        assert!(!j.contains("very-secret"), "{j}");
    }

    #[test]
    fn pull_keeps_a_local_secret_value_and_writes_a_missing_one_empty() {
        let l = local(&[
            ("S_LOCAL", "mine"),
            ("C_SAME", "x"),
            ("C_OLD", "old"),
            ("LOCAL_ONLY", "z"),
        ]);
        let ops = pull_plan(
            &l,
            vec![
                remote("S_LOCAL", Kind::Secret, None),
                remote("S_MISSING", Kind::Secret, None),
                remote("C_SAME", Kind::Config, Some("x")),
                remote("C_OLD", Kind::Config, Some("new")),
                remote("C_NEW", Kind::Config, Some("n")),
            ],
        );
        let op = |k: &str| ops.iter().find(|c| c.key == k).unwrap().op;
        assert_eq!(op("S_LOCAL"), PullOp::KeepLocal);
        assert_eq!(op("S_MISSING"), PullOp::WriteEmpty);
        assert_eq!(op("C_SAME"), PullOp::Same);
        assert_eq!(op("C_OLD"), PullOp::Update);
        assert_eq!(op("C_NEW"), PullOp::Add);
        assert!(ops.iter().all(|c| c.key != "LOCAL_ONLY"));
    }

    #[test]
    fn a_config_that_vercel_would_not_reveal_is_treated_as_write_only() {
        let ops = pull_plan(&local(&[]), vec![remote("K", Kind::Secret, None)]);
        assert_eq!(ops[0].op, PullOp::WriteEmpty);
    }
}
