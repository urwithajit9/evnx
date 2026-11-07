//! Custom scan patterns — the matcher behind `--pattern` and `[[scan.patterns]]`.
//!
//! The built-in detectors know the formats evnx ships with. This module is how a
//! project adds the ones only it knows about: an internal token format, a
//! vendor's key shape, a naming convention that means "this is live".
//!
//! # Why a set, and not a loop
//!
//! Scanning is `O(files × lines × patterns)` if each pattern is tried in turn,
//! and the pattern count is the one factor a project controls — so a design that
//! walks the list is a design that punishes teams for declaring rules.
//!
//! [`regex::RegexSet`] compiles every pattern into **one** automaton that reports
//! which of them match in a single pass, without locating any of them. Only the
//! rules that pass report get their own [`regex::Regex`] run to find the actual
//! span. On the overwhelmingly common path — a line with no secret in it — the
//! cost is one pass regardless of how many patterns are declared.
//!
//! That is the same shape as GitHub's scanner (Hyperscan) and gitleaks' keyword
//! prefilter, and it is why the per-rule `Regex` is compiled a second time rather
//! than reused from the set: `RegexSet` deliberately cannot tell you *where* it
//! matched, and that is exactly what makes it fast.
//!
//! Measured, not assumed — 400 TypeScript files, 60,000 lines, 6.3 MB, no
//! findings, mean of three release runs. The right-hand column is this same code
//! with the `set.matches` prefilter removed so every rule is tried in turn:
//!
//! ```text
//! patterns   with the set   looping over every rule
//!        0         55 ms                       n/a
//!        1         56 ms                     56 ms
//!       10         59 ms                     72 ms
//!       50         58 ms                    136 ms
//!      200         68 ms                    369 ms
//! ```
//!
//! The loop is linear in the rule count, at roughly 1.5 ms per pattern over this
//! corpus. The set is close to flat: 200 rules cost 13 ms more than one, and the
//! whole scan stays within 25% of the no-patterns baseline. A team that declares
//! its formats should not pay for having done so.
//!
//! The literal prefilter gitleaks builds by hand — Aho-Corasick over each rule's
//! keywords — is already inside the `regex` crate, which extracts literal
//! prefixes and runs memchr/Teddy before the automaton. Declaring
//! `ACME-[A-Z0-9]{32}` gets that treatment for free; hand-rolling a second layer
//! on top would be slower, not faster.
//!
//! # Why a user-supplied pattern cannot hang the scanner
//!
//! `regex` is finite-automaton based and has no backtracking, so it matches in
//! time linear in the input **for every expression it accepts** — there is no
//! catastrophic-backtracking input to find. A pattern can still be expensive to
//! *compile*, which is what [`PATTERN_SIZE_LIMIT`] bounds. The practical effect
//! is that `[scan.patterns]` in a committed `.evnx.toml` is not a denial-of-
//! service vector against everyone who clones the repository.

use super::{Confidence, Severity};
use crate::core::config::PatternRule;
use anyhow::{anyhow, bail, Context, Result};
use regex::{Regex, RegexBuilder, RegexSet, RegexSetBuilder};

/// Ceiling on the compiled size of a single pattern.
///
/// `regex` trades memory for speed when it expands bounded repetitions, so
/// `[A-Z0-9]{1000000}` is a legal expression that costs a great deal to compile.
/// 1 MiB is far above any real credential format and far below anything that
/// would be felt.
pub(crate) const PATTERN_SIZE_LIMIT: usize = 1 << 20;

/// One declared rule, compiled and ready to match.
#[derive(Debug)]
struct CompiledRule {
    id: String,
    name: String,
    confidence: Confidence,
    severity: Severity,
    /// Matches below this Shannon entropy are dropped.
    entropy: Option<f64>,
    url: Option<String>,
    regex: Regex,
    /// The expression as written, so a match lying inside its own text — the
    /// declaration in `.evnx.toml` — can be set aside (D-36).
    source: String,
    /// One of `secretscan::rules::builtin_rules`, unchanged (I-058).
    builtin: bool,
    /// C7: lowercased; the rule is tried only on text holding one of them.
    keywords: Vec<String>,
    /// C7: the capture group that is the secret; unset is the whole match.
    secret_group: Option<usize>,
    /// C7: a secret any of these matches is not reported.
    allow: Option<RegexSet>,
    /// C7: nothing on a line any of these matches is reported.
    allow_line: Option<RegexSet>,
}

/// What a custom pattern found.
///
/// `value` is the **matched span**, not the whole line or the whole value — so a
/// key embedded in a longer string is reported as the key, and masking in
/// `runner::truncate_value` has the right thing to mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    /// The rule's stable id (C2).
    pub id: String,
    /// The rule's name, as it appears in output.
    pub name: String,
    pub confidence: Confidence,
    pub severity: Severity,
    /// Where to go and revoke it, when the rule says.
    pub url: Option<String>,
    /// The matched span.
    pub value: String,
}

/// Every custom pattern this run was given, compiled once.
#[derive(Debug)]
pub struct PatternSet {
    /// The single-pass filter over the rules **without** keywords.
    set: RegexSet,
    /// The set's index `i` is rule `in_set[i]`.
    in_set: Vec<usize>,
    /// C7: one pass over every keyword, lowercased. Pattern `i` belongs to
    /// rule `by_keyword[i]`. A rule with keywords is tried only where one
    /// appears, which is what keeps a pack of hundreds of rules fast: one
    /// `RegexSet` of them all outgrows its cache and crawls.
    keywords: Option<aho_corasick::AhoCorasick>,
    by_keyword: Vec<usize>,
    rules: Vec<CompiledRule>,
}

impl PatternSet {
    /// A set with no rules, which never matches and costs nothing.
    ///
    /// This is what every project that has not declared a pattern gets, and it
    /// is why the custom detector is not registered at all in that case.
    pub fn empty() -> Self {
        Self {
            set: RegexSet::empty(),
            in_set: Vec::new(),
            keywords: None,
            by_keyword: Vec::new(),
            rules: Vec::new(),
        }
    }

    /// Compile the declared rules.
    ///
    /// ⚠️ **Every failure here must reach the caller**, not be logged and
    /// skipped. `evnx scan --pattern '('` that reported "no secrets found"
    /// because the pattern silently did not compile would be the worst outcome
    /// this command has: a clean result that was never actually checked.
    /// `scan::run` turns the error into exit 2 — *no verdict* — which is a
    /// different answer from exit 0.
    pub fn compile(rules: &[PatternRule]) -> Result<Self> {
        if rules.is_empty() {
            return Ok(Self::empty());
        }

        let mut compiled = Vec::with_capacity(rules.len());
        let mut sources = Vec::with_capacity(rules.len());
        let builtins = crate::core::secretscan::rules::builtin_rules();

        for (i, rule) in rules.iter().enumerate() {
            let name = rule.name.trim();
            if name.is_empty() {
                bail!(
                    "custom pattern #{} has no name — give it a `name` in [[scan.patterns]]",
                    i + 1
                );
            }

            let source = rule.regex.trim();
            if source.is_empty() {
                bail!("'{name}' has an empty regular expression");
            }

            // Unset means "you wrote this rule deliberately, so it counts" —
            // high, which is what a blocking `--severity high` gate reports on.
            // A project that wants a softer rule says so; a typo in that value
            // is an error rather than a silent downgrade, because quietly
            // demoting a rule to `low` would drop it out of exactly that gate.
            let confidence = match rule.confidence.as_deref() {
                Some(level) => level
                    .parse::<Confidence>()
                    .with_context(|| format!("'{name}'"))?,
                None => Confidence::High,
            };

            // A rule that re-rates a built-in by name keeps the built-in's
            // identity and risk: changing how sure it is must not change which
            // rule it is, nor drop it to the default severity.
            let shipped = builtins.iter().find(|b| b.name == name);
            let id = match rule.id.as_deref().or(shipped.and_then(|b| b.id.as_deref())) {
                Some(id) => id.trim().to_string(),
                None => slug(name),
            };
            if id.is_empty()
                || !id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                bail!("'{name}' has id '{id}': use lowercase letters, digits and '-'");
            }
            if let Some(other) = compiled.iter().find(|r: &&CompiledRule| r.id == id) {
                bail!(
                    "'{name}' and '{}' have the same id '{id}': each rule needs its own",
                    other.name
                );
            }
            // Unset: medium, the middle of the scale, for a rule evnx knows
            // nothing about the risk of.
            let severity = match rule
                .severity
                .as_deref()
                .or(shipped.and_then(|b| b.severity.as_deref()))
            {
                Some(level) => level
                    .parse::<Severity>()
                    .with_context(|| format!("'{name}'"))?,
                None => Severity::Medium,
            };

            let regex = RegexBuilder::new(source)
                .size_limit(PATTERN_SIZE_LIMIT)
                .build()
                .map_err(|e| {
                    anyhow!("'{name}' is not a valid regular expression\n  {source}\n\n{e}")
                })?;

            // A pattern that matches the empty string matches at every position
            // of every line, so every value in the project becomes a finding.
            // That is not a strict scan, it is an unreadable one — and the
            // findings would carry empty previews.
            if regex.is_match("") {
                bail!(
                    "'{name}' matches the empty string, so it would flag every line \
                     in the project\n  {source}"
                );
            }

            let allow_set = |list: &Option<Vec<String>>, what: &str| -> Result<Option<RegexSet>> {
                match list.as_deref() {
                    None | Some([]) => Ok(None),
                    Some(list) => RegexSetBuilder::new(list)
                        .size_limit(PATTERN_SIZE_LIMIT.saturating_mul(list.len()))
                        .build()
                        .map(Some)
                        .map_err(|e| {
                            anyhow!(
                                "'{name}' has an {what} expression that does not compile\n\n{e}"
                            )
                        }),
                }
            };
            let allow = allow_set(&rule.allow, "allow")?;
            let allow_line = allow_set(&rule.allow_line, "allow_line")?;
            if let Some(g) = rule.secret_group {
                if g == 0 || g >= regex.captures_len() {
                    bail!(
                        "'{name}' has secret_group = {g}, and its regex has {} capture group(s)",
                        regex.captures_len() - 1
                    );
                }
            }

            compiled.push(CompiledRule {
                secret_group: rule.secret_group,
                allow,
                allow_line,
                id,
                name: name.to_string(),
                confidence,
                severity,
                entropy: rule.entropy,
                url: rule.url.clone(),
                regex,
                source: source.to_string(),
                builtin: builtins.contains(rule),
                keywords: rule
                    .keywords
                    .iter()
                    .flatten()
                    .map(|k| k.trim().to_ascii_lowercase())
                    .filter(|k| !k.is_empty())
                    .collect(),
            });
            if compiled.last().is_some_and(|r| r.keywords.is_empty()) {
                sources.push(source.to_string());
            }
        }
        let in_set: Vec<usize> = (0..compiled.len())
            .filter(|&i| compiled[i].keywords.is_empty())
            .collect();
        let mut words = Vec::new();
        let mut by_keyword = Vec::new();
        for (i, r) in compiled.iter().enumerate() {
            for k in &r.keywords {
                words.push(k.clone());
                by_keyword.push(i);
            }
        }
        let keywords = if words.is_empty() {
            None
        } else {
            Some(aho_corasick::AhoCorasick::new(&words).context("compiling the rule keywords")?)
        };

        // The set is built from the same sources, so its indices line up with
        // `compiled`. Its size limit scales with the rule count: the per-pattern
        // ceiling has already been enforced above, and applying that same number
        // to the combined automaton would make a set of modest patterns fail for
        // no reason other than that there are several of them.
        // Every rule may have keywords (a gitleaks pack): then the set is empty.
        let set = if sources.is_empty() {
            RegexSet::empty()
        } else {
            RegexSetBuilder::new(&sources)
                .size_limit(PATTERN_SIZE_LIMIT.saturating_mul(sources.len()))
                .build()
                .context("compiling the custom pattern set")?
        };

        Ok(Self {
            set,
            in_set,
            keywords,
            by_keyword,
            rules: compiled,
        })
    }

    /// No rules declared.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// How many rules are active, for verbose output.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// `(built-in, custom)`: rules evnx ships, unchanged, and everything else —
    /// `--pattern`, `[[scan.patterns]]`, and a built-in a project re-rated
    /// (I-058).
    pub fn counts(&self) -> (usize, usize) {
        let builtin = self.rules.iter().filter(|r| r.builtin).count();
        (builtin, self.rules.len() - builtin)
    }

    /// Every rule's name, in the order they are tried.
    pub fn names(&self) -> Vec<&str> {
        self.rules.iter().map(|r| r.name.as_str()).collect()
    }

    /// Every distinct match in `haystack`, in the order the rules were declared.
    ///
    /// Repeated matches of the same rule on the same text are reported once: a
    /// line that mentions the same token twice is one leak, not two.
    pub fn find_all(&self, haystack: &str) -> Vec<PatternMatch> {
        if self.rules.is_empty() || haystack.is_empty() {
            return Vec::new();
        }

        // The single pass. Everything below runs only for text that already
        // contains something.
        let mut tried: Vec<usize> = self
            .set
            .matches(haystack)
            .iter()
            .map(|i| self.in_set[i])
            .collect();
        if let Some(ac) = &self.keywords {
            let lower = haystack.to_ascii_lowercase();
            tried.extend(
                ac.find_overlapping_iter(&lower)
                    .map(|m| self.by_keyword[m.pattern().as_usize()]),
            );
            // Declaration order, each rule once.
            tried.sort_unstable();
            tried.dedup();
        }
        if tried.is_empty() {
            return Vec::new();
        }

        let mut out: Vec<PatternMatch> = Vec::new();
        for index in tried {
            let rule = &self.rules[index];
            if rule
                .allow_line
                .as_ref()
                .is_some_and(|a| a.is_match(haystack))
            {
                continue;
            }
            for caps in rule.regex.captures_iter(haystack) {
                let found = caps.get(0).expect("group 0 is the match");
                // C7: `secret_group` names the part that is the secret, as
                // gitleaks' rules do; it is what is judged and reported.
                let value = match rule.secret_group {
                    Some(g) => match caps.get(g) {
                        Some(m) => m.as_str(),
                        None => continue,
                    },
                    None => found.as_str(),
                };
                if value.is_empty() {
                    continue;
                }
                if rule.allow.as_ref().is_some_and(|a| a.is_match(value)) {
                    continue;
                }
                // D-36: a pattern of literal text matches its own declaration,
                // `regex = "ACME-INTERNAL-TOKEN"` in `.evnx.toml`. A match lying
                // inside the rule's own source text, where that text is the
                // value of a `regex =` assignment, is the declaration, not a
                // secret. ⚠️ Only there: a literal pattern's every real match
                // equals its source too. Every other rule still reads the line.
                let (start, end) = (found.start(), found.end());
                if haystack
                    .match_indices(rule.source.as_str())
                    .any(|(at, text)| {
                        at <= start && end <= at + text.len() && declares(&haystack[..at])
                    })
                {
                    continue;
                }
                if out.iter().any(|m| m.name == rule.name && m.value == value) {
                    continue;
                }
                if rule
                    .entropy
                    .is_some_and(|min| super::heuristic::calculate_entropy(value) < min)
                {
                    continue;
                }
                out.push(PatternMatch {
                    id: rule.id.clone(),
                    name: rule.name.clone(),
                    confidence: rule.confidence,
                    severity: rule.severity,
                    url: rule.url.clone(),
                    value: value.to_string(),
                });
            }
        }
        out
    }

    /// The single best match in `haystack`, for a `.env` value.
    ///
    /// One value is one finding — the same rule the built-in detectors follow in
    /// `runner::best` — so when several rules claim the same value the most
    /// confident wins, and ties keep the order they were declared in.
    pub fn strongest(&self, haystack: &str) -> Option<PatternMatch> {
        // ⚠️ Confidence first, then **longest match**. Before the built-ins
        // became rules, collisions were resolved by the source order of an `if`
        // chain — implicit, and it went wrong: a permissive `sk-` rule for
        // OpenAI was checked before Anthropic's, so `sk-ant-api03-…` was
        // reported as an OpenAI key and sent you to the wrong dashboard to
        // revoke it.
        //
        // Longest-match is how a lexer settles the same question, and it needs
        // no configuration: `sk-ant-api03-…` matches more characters under
        // Anthropic's rule than under a looser one, so the specific rule wins
        // whatever order they were declared in. Ties beyond that fall to
        // declaration order, which is the local-first order `merge` produces.
        Self::strongest_of(self.find_all(haystack))
    }

    /// [`PatternSet::strongest`] over matches already found — for a caller
    /// that sets some aside first, as the rule detector does with stand-ins
    /// (I-050).
    pub fn strongest_of(matches: impl IntoIterator<Item = PatternMatch>) -> Option<PatternMatch> {
        matches.into_iter().reduce(|best, next| {
            // ⚠️ `url.is_some()` is part of the rank, and omitting it regressed
            // a real case the suite caught: a user's `--pattern` that happens to
            // match an AWS key claimed the finding and reported it as "Custom
            // pattern 1" with **no link to IAM**. Custom rules default to high
            // confidence, so confidence alone could not separate them, and
            // `merge` puts local rules first — so declaration order handed it to
            // the one that could not say where to revoke.
            //
            // Confidence stays primary: a high-confidence match must not lose to
            // a low-confidence one merely for carrying a URL.
            let rank = |m: &PatternMatch| (m.confidence, m.url.is_some(), m.value.len());
            if rank(&next) > rank(&best) {
                next
            } else {
                best
            }
        })
    }
}

/// Does `before` end a `regex = "` / `regex = '` assignment — the text in front
/// of a rule's own expression in `[[scan.patterns]]` (D-36)?
fn declares(before: &str) -> bool {
    let Some(rest) = before.strip_suffix(['"', '\'']) else {
        return false;
    };
    rest.trim_end()
        .strip_suffix('=')
        .is_some_and(|key| key.trim_end().ends_with("regex"))
}

impl Default for PatternSet {
    fn default() -> Self {
        Self::empty()
    }
}

/// Combine `--pattern` flags with `[[scan.patterns]]` from `.evnx.toml`.
///
/// Additive, like [`crate::core::config::extend`]: the flag is this run's rule
/// and the config file is the project's standing set, and a flag silently
/// dropping the project's rules would be a surprising way to *narrow* a scan.
///
/// Flags come first so an ad-hoc rule outranks a configured one on the same
/// value, and duplicates by expression are dropped — passing `--pattern` for
/// something already declared should not report it twice.
pub fn merge(
    flags: Vec<String>,
    configured: Option<Vec<PatternRule>>,
    builtins: bool,
    disable: Option<Vec<String>>,
) -> Vec<PatternRule> {
    let mut out: Vec<PatternRule> = flags
        .into_iter()
        .enumerate()
        .map(|(i, regex)| PatternRule::anonymous(i + 1, regex))
        .collect();

    for rule in configured.into_iter().flatten() {
        if !out.iter().any(|existing| existing.regex == rule.regex) {
            out.push(rule);
        }
    }

    // ⚠️ Built-ins come **last**, and that is what makes a project able to
    // re-rate one. A rule declared in `[[scan.patterns]]` with the same name
    // wins, because `dedup_by_name` below keeps the first of each name and the
    // project's rules were pushed first.
    if builtins {
        let off: Vec<String> = disable
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.to_lowercase())
            .collect();
        for rule in crate::core::secretscan::rules::builtin_rules() {
            if is_named(&rule, &off) {
                continue;
            }
            out.push(rule);
        }
    }

    dedup_by_name(out)
}

/// Whether `off` (lowercased `[scan] disable` entries) names `rule`, by its
/// name or, since C2, its id.
pub fn is_named(rule: &PatternRule, off: &[String]) -> bool {
    off.iter()
        .any(|d| *d == rule.name.to_lowercase() || rule.id.as_deref() == Some(d.as_str()))
}

/// Keep the first rule of each name.
///
/// Names reach SARIF as rule ids, so two rules sharing one would collapse two
/// distinct findings into one alert. Order decides the winner, and the order is
/// `--pattern` → `[[scan.patterns]]` → built-ins, so the more local declaration
/// always takes precedence.
fn dedup_by_name(rules: Vec<PatternRule>) -> Vec<PatternRule> {
    let mut seen = std::collections::HashSet::new();
    rules
        .into_iter()
        .filter(|r| seen.insert(r.name.to_lowercase()))
        .collect()
}

/// `name`, lowercased, with every run of other characters as one `-`: the id a
/// rule gets when it declares none. `Custom pattern 1` is `custom-pattern-1`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, regex: &str) -> PatternRule {
        let mut r = PatternRule::anonymous(1, regex.to_string());
        r.name = name.to_string();
        r
    }

    #[test]
    fn empty_set_never_matches() {
        let set = PatternSet::empty();
        assert!(set.is_empty());
        assert!(set
            .find_all("ACME-ABCDEFGHIJKLMNOPQRSTUVWXYZ012345")
            .is_empty());
    }

    #[test]
    fn finds_a_declared_format() {
        let set = PatternSet::compile(&[rule("Acme API key", r"ACME-[A-Z0-9]{32}")]).unwrap();
        let found = set
            .strongest("ACME-ABCDEFGHIJKLMNOPQRSTUVWXYZ012345")
            .unwrap();
        assert_eq!(found.name, "Acme API key");
        assert_eq!(found.value, "ACME-ABCDEFGHIJKLMNOPQRSTUVWXYZ012345");
        assert_eq!(found.confidence, Confidence::High);
    }

    #[test]
    fn reports_the_span_not_the_whole_haystack() {
        let set = PatternSet::compile(&[rule("Acme", r"ACME-[A-Z0-9]{8}")]).unwrap();
        let found = set
            .strongest("Authorization: Bearer ACME-ABCD1234 trailing")
            .unwrap();
        assert_eq!(found.value, "ACME-ABCD1234");
    }

    #[test]
    fn an_invalid_expression_is_an_error_not_a_clean_scan() {
        let err = PatternSet::compile(&[rule("broken", "(")]).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("broken"), "{text}");
        assert!(text.contains("not a valid regular expression"), "{text}");
    }

    #[test]
    fn an_empty_matching_expression_is_refused() {
        for source in [".*", "a?", "(?:)"] {
            let err = PatternSet::compile(&[rule("catch-all", source)]).unwrap_err();
            assert!(
                format!("{err:#}").contains("matches the empty string"),
                "{source} should be refused"
            );
        }
    }

    #[test]
    fn an_unknown_confidence_is_an_error_not_a_silent_downgrade() {
        let mut r = rule("typo", "ACME-[0-9]{8}");
        r.confidence = Some("hgih".to_string());
        let err = PatternSet::compile(&[r]).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("typo"), "{text}");
        assert!(text.contains("unknown confidence"), "{text}");
    }

    /// C2: a typo in `severity` is refused the same way, and a missing one is
    /// medium, or a re-rated built-in's own.
    #[test]
    fn severity_is_parsed_defaulted_and_inherited() {
        let mut r = rule("typo", "ACME-[0-9]{8}");
        r.severity = Some("crit".to_string());
        let err = format!("{:#}", PatternSet::compile(&[r]).unwrap_err());
        assert!(err.contains("unknown severity"), "{err}");

        let set = PatternSet::compile(&[rule("mine", "ACME-[0-9]{8}")]).unwrap();
        let m = &set.find_all("ACME-12345678")[0];
        assert_eq!((m.id.as_str(), m.severity), ("mine", Severity::Medium));

        // A project re-rating a built-in by name keeps its id and severity.
        let mut stripe = rule("Stripe Secret Key (LIVE)", "sk_live_[0-9a-zA-Z]{24,}");
        stripe.confidence = Some("medium".to_string());
        let set = PatternSet::compile(&[stripe]).unwrap();
        let m = &set.find_all(&format!("sk_live_{}", "a".repeat(24)))[0];
        assert_eq!(m.id, "stripe-live-secret-key");
        assert_eq!(
            (m.confidence, m.severity),
            (Confidence::Medium, Severity::Critical)
        );
    }

    /// C2: ids are kebab-case and unique within a set.
    #[test]
    fn ids_are_checked() {
        let mut a = rule("a", "AAA-[0-9]{8}");
        a.id = Some("same".into());
        let mut b = rule("b", "BBB-[0-9]{8}");
        b.id = Some("same".into());
        let err = format!("{:#}", PatternSet::compile(&[a, b]).unwrap_err());
        assert!(err.contains("same id 'same'"), "{err}");
        let mut c = rule("c", "CCC-[0-9]{8}");
        c.id = Some("Not Kebab".into());
        assert!(PatternSet::compile(&[c]).is_err());
    }

    /// C2: `entropy` drops a match below the floor.
    #[test]
    fn an_entropy_floor_drops_low_entropy_matches() {
        let mut r = rule("tok", "tok_[a-z0-9]{12}");
        r.entropy = Some(3.0);
        let set = PatternSet::compile(&[r]).unwrap();
        assert!(set.find_all("tok_aaaaaaaaaaaa").is_empty());
        assert_eq!(set.find_all("tok_k3x9q2m7v1zp").len(), 1);
    }

    /// C7: the secret group is what is reported and judged; `allow` drops a
    /// secret, `allow_line` a whole line.
    #[test]
    fn the_secret_group_is_reported_and_allow_lists_drop_matches() {
        let mut r = rule("kv", r"key\s*=\s*([a-z0-9]{8})");
        r.secret_group = Some(1);
        r.allow = Some(vec!["(?i)example".into()]);
        r.allow_line = Some(vec![r"#\s*fixture".into()]);
        let set = PatternSet::compile(std::slice::from_ref(&r)).unwrap();
        let found = set.find_all("key = k3x9q2m7");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value, "k3x9q2m7");
        assert!(set.find_all("key = example1").is_empty());
        assert!(set.find_all("key = k3x9q2m7 # fixture").is_empty());

        r.secret_group = Some(2);
        assert!(PatternSet::compile(&[r]).is_err(), "no group 2");
    }

    /// C7: a rule with keywords is tried only where one appears, any case;
    /// one without stays in the set and is tried everywhere.
    #[test]
    fn keywords_gate_a_rule_and_leave_the_others_alone() {
        let mut gated = rule("gated", "[0-9]{8}");
        gated.keywords = Some(vec!["Acme".into()]);
        // Only keyword rules: no set at all, and it still compiles.
        assert!(PatternSet::compile(std::slice::from_ref(&gated)).is_ok());
        let open = rule("open", "zz-[0-9]{4}");
        let set = PatternSet::compile(&[gated, open]).unwrap();
        assert!(set.find_all("id 12345678").is_empty());
        let found: Vec<String> = set
            .find_all("ACME_ID=12345678 zz-1234")
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(found, ["gated", "open"]);
    }

    #[test]
    fn declared_confidence_is_honoured() {
        let mut r = rule("soft", "ACME-[0-9]{8}");
        r.confidence = Some("medium".to_string());
        let set = PatternSet::compile(&[r]).unwrap();
        assert_eq!(
            set.strongest("ACME-12345678").unwrap().confidence,
            Confidence::Medium
        );
    }

    #[test]
    fn several_rules_match_in_one_pass_and_keep_declaration_order() {
        let set = PatternSet::compile(&[
            rule("first", r"ACME-[0-9]{4}"),
            rule("second", r"BETA-[0-9]{4}"),
        ])
        .unwrap();
        let found = set.find_all("ACME-1234 and BETA-5678");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "first");
        assert_eq!(found[1].name, "second");
    }

    #[test]
    fn the_same_token_twice_on_a_line_is_one_finding() {
        let set = PatternSet::compile(&[rule("Acme", r"ACME-[0-9]{4}")]).unwrap();
        assert_eq!(set.find_all("ACME-1234 ACME-1234").len(), 1);
        assert_eq!(set.find_all("ACME-1234 ACME-5678").len(), 2);
    }

    #[test]
    fn ties_on_confidence_keep_the_first_rule() {
        let set = PatternSet::compile(&[
            rule("first", r"ACME-[0-9]{4}"),
            rule("second", r"ACME-\d{4}"),
        ])
        .unwrap();
        assert_eq!(set.strongest("ACME-1234").unwrap().name, "first");
    }

    #[test]
    fn the_more_confident_rule_wins_a_value() {
        let mut soft = rule("soft", r"ACME-[0-9]{4}");
        soft.confidence = Some("low".to_string());
        let set = PatternSet::compile(&[soft, rule("strict", r"ACME-\d{4}")]).unwrap();
        let found = set.strongest("ACME-1234").unwrap();
        assert_eq!(found.name, "strict");
        assert_eq!(found.confidence, Confidence::High);
    }

    #[test]
    fn a_pattern_too_large_to_compile_is_refused_rather_than_accepted() {
        // Bounded repetition is expanded at compile time, so this is a legal
        // expression that costs far more than any credential format would.
        let err = PatternSet::compile(&[rule("huge", r"(?s)[\s\S]{1,1000000}")]).unwrap_err();
        assert!(format!("{err:#}").contains("huge"));
    }

    #[test]
    fn merge_puts_flags_first_and_appends_the_project_rules() {
        let merged = merge(
            vec![r"ACME-[0-9]{4}".to_string()],
            Some(vec![rule("Project rule", r"BETA-[0-9]{4}")]),
            false,
            None,
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "Custom pattern 1");
        assert_eq!(merged[1].name, "Project rule");
    }

    #[test]
    fn merge_drops_a_flag_that_repeats_a_configured_expression() {
        let merged = merge(
            vec![r"BETA-[0-9]{4}".to_string()],
            Some(vec![rule("Project rule", r"BETA-[0-9]{4}")]),
            false,
            None,
        );
        assert_eq!(
            merged.len(),
            1,
            "the same expression should not report twice"
        );
        assert_eq!(merged[0].name, "Custom pattern 1");
    }

    #[test]
    fn flags_are_numbered_in_the_order_given() {
        let merged = merge(
            vec!["ACME-[0-9]{4}".to_string(), "BETA-[0-9]{4}".to_string()],
            None,
            false,
            None,
        );
        assert_eq!(merged[0].name, "Custom pattern 1");
        assert_eq!(merged[1].name, "Custom pattern 2");
    }
}
