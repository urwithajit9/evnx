//! Names in files read as text (SCAN-NAMES).
//!
//! A `.env` is judged by name and value; any other file read as text was
//! judged by value alone. So `ENV DB_PASSWORD=<random>` in a Dockerfile,
//! `export DB_PASSWORD="<random>"` in a shell script and `DB_PASSWORD = "<random>"`
//! in a Python module scanned clean unless the random value happened to look
//! like a key. This finds those assignments, line by line, and hands each
//! `(name, value)` to the same check a `.env` line gets
//! ([`DetectorRegistry::scan_kv`](super::DetectorRegistry::scan_kv): the name
//! list, the placeholder, empty and reference filters).
//!
//! # What is an assignment
//!
//! Only a literal. Where the right-hand side is not a literal string, the
//! secret is wherever it points and not on this line, so it is skipped:
//!
//! * Dockerfile `ENV`/`ARG` (pair and `ENV KEY value` forms, continuation
//!   lines); `ARG X` with no value is nothing.
//! * Shell: `[export|readonly|declare -x] NAME=value`, a bare `NAME=value`
//!   (also as a prefix to a command). A value with `$`, `$(`, `${` or a
//!   backtick inside double quotes or bare is an expansion and is skipped; a
//!   single-quoted one is literal.
//! * Python: `NAME = "literal"` or `NAME: str = "literal"`, alone on its line.
//!   `os.environ[...]`, `os.getenv(...)`, `settings.X`, an f-string or a
//!   concatenation is not a string literal and is skipped.
//!
//! ⚠️ Comments are not read here (the value rules still read them). A name
//! check on `# DB_PASSWORD = "..."` in a script mostly flags documentation.

/// How a file's assignments are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Dockerfile,
    Shell,
    Python,
}

impl Shape {
    /// The shape of the file at `path`, by its name, or for an extensionless
    /// script by the `#!` line `first_line`. `None`: names are not checked.
    #[must_use]
    pub fn of(path: &str, first_line: &str) -> Option<Shape> {
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path)
            .to_ascii_lowercase();
        if name == "dockerfile"
            || name == "containerfile"
            || name.starts_with("dockerfile.")
            || name.ends_with(".dockerfile")
        {
            return Some(Shape::Dockerfile);
        }
        match name.rsplit_once('.').map(|(_, e)| e) {
            Some("sh" | "bash" | "zsh") => return Some(Shape::Shell),
            Some("py") => return Some(Shape::Python),
            Some(_) => return None,
            None => {}
        }
        let shebang = first_line.trim_start_matches('\u{feff}');
        let interpreter = shebang.strip_prefix("#!")?;
        let mut words = interpreter.split_whitespace();
        let mut prog = words.next()?.rsplit('/').next()?;
        if prog == "env" {
            prog = words.find(|w| !w.starts_with('-'))?;
        }
        matches!(prog, "sh" | "bash" | "zsh" | "dash" | "ksh").then_some(Shape::Shell)
    }
}

/// Reads assignments from successive lines of one file.
pub struct Assignments {
    shape: Shape,
    /// The previous Dockerfile line was an `ENV`/`ARG` pair list ending in `\`.
    continued: bool,
}

impl Assignments {
    #[must_use]
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            continued: false,
        }
    }

    /// The `(name, value)` pairs on `line`, which must be fed in order.
    pub fn line(&mut self, line: &str) -> Vec<(String, String)> {
        match self.shape {
            Shape::Dockerfile => self.dockerfile(line),
            Shape::Shell => shell(line),
            Shape::Python => python(line).into_iter().collect(),
        }
    }

    fn dockerfile(&mut self, line: &str) -> Vec<(String, String)> {
        let t = line.trim();
        let continues = t.ends_with('\\');
        let body = t.strip_suffix('\\').unwrap_or(t).trim();
        let was_continued = std::mem::replace(&mut self.continued, false);
        if body.starts_with('#') {
            // A comment line inside a continued instruction does not end it.
            self.continued = was_continued;
            return Vec::new();
        }
        if was_continued {
            self.continued = continues;
            return pairs(body);
        }
        let Some((instr, rest)) = body.split_once(char::is_whitespace) else {
            return Vec::new();
        };
        let is_env = instr.eq_ignore_ascii_case("ENV");
        if !is_env && !instr.eq_ignore_ascii_case("ARG") {
            return Vec::new();
        }
        let rest = rest.trim();
        if starts_with_assignment(rest) {
            self.continued = continues;
            return pairs(rest);
        }
        // `ENV KEY value with spaces`: the rest of the line is the value.
        // `ARG KEY` has no value and is nothing.
        if !is_env {
            return Vec::new();
        }
        let Some((key, value)) = rest.split_once(char::is_whitespace) else {
            return Vec::new();
        };
        let value = value.trim();
        let value = ['"', '\'']
            .iter()
            .find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q))
            .unwrap_or(value);
        if is_ident(key) && !value.is_empty() && !is_expansion(value) {
            vec![(key.to_string(), value.to_string())]
        } else {
            Vec::new()
        }
    }
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(f) if f.is_ascii_alphabetic() || f == '_')
        && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `NAME=` at the start of `s`.
fn starts_with_assignment(s: &str) -> bool {
    s.split_once('=').is_some_and(|(k, _)| is_ident(k))
}

/// `$`, `$(`, `${` or a backtick: the shell or Docker expands it.
fn is_expansion(v: &str) -> bool {
    v.contains('$') || v.contains('`')
}

/// Successive `NAME=value` words at the start of `s`, stopping quietly at the
/// first word that is not one (`FOO=bar cmd args`).
fn pairs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while let Some((key, after)) = rest.split_once('=') {
        if !is_ident(key) {
            break;
        }
        let (value, remainder, literal) = read_value(after);
        // `NAME=` (empty), an expansion and a non-literal are not values.
        if literal && !value.is_empty() && !is_expansion_unless_single(after, &value) {
            out.push((key.to_string(), value));
        }
        rest = remainder.trim_start();
    }
    out
}

/// A single-quoted value is literal whatever it holds.
fn is_expansion_unless_single(after: &str, value: &str) -> bool {
    !after.starts_with('\'') && is_expansion(value)
}

/// One shell/Docker word value: quoted or bare. Returns the value, what
/// follows it, and whether it ended cleanly (an unclosed quote does not).
fn read_value(s: &str) -> (String, &str, bool) {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, q @ ('"' | '\''))) => {
            let mut value = String::new();
            let mut escaped = false;
            for (i, c) in chars {
                if escaped {
                    value.push(c);
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    return (value, &s[i + 1..], true);
                } else {
                    value.push(c);
                }
            }
            (value, "", false)
        }
        _ => {
            let end = s.find(char::is_whitespace).unwrap_or(s.len());
            let word = s[..end].trim_end_matches(';');
            (word.to_string(), &s[end..], true)
        }
    }
}

fn shell(line: &str) -> Vec<(String, String)> {
    let t = line.trim_start();
    if t.starts_with('#') {
        return Vec::new();
    }
    let mut rest = t;
    // `export`, `readonly`, `declare -x`, `typeset -x`, `local`: then words.
    for kw in ["export", "readonly", "declare", "typeset", "local"] {
        if let Some(after) = t
            .strip_prefix(kw)
            .filter(|a| a.starts_with(char::is_whitespace))
        {
            rest = after.trim_start();
            while rest.starts_with('-') {
                rest = rest
                    .split_once(char::is_whitespace)
                    .map_or("", |(_, r)| r.trim_start());
            }
            break;
        }
    }
    pairs(rest)
}

/// `NAME = "literal"` or `NAME: type = 'literal'`, and nothing else on the
/// line but a comment.
fn python(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    let end = t
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(t.len());
    let (name, mut rest) = t.split_at(end);
    if !is_ident(name) {
        return None;
    }
    rest = rest.trim_start();
    if let Some(annotated) = rest.strip_prefix(':') {
        // The annotation is a name or a subscript, never a string.
        let (annotation, after) = annotated.split_once('=')?;
        if annotation.contains(['"', '\'']) {
            return None;
        }
        rest = after;
    } else {
        rest = rest.strip_prefix('=')?;
    }
    if rest.starts_with('=') {
        return None;
    }
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(['r', 'R', 'u', 'U']).unwrap_or(rest);
    let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let body = &rest[1..];
    if body.starts_with(quote) {
        // `""` is empty, and `"""` opens a docstring-style block.
        return None;
    }
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            let after = body[i + 1..].trim();
            return (after.is_empty() || after.starts_with('#'))
                .then(|| (name.to_string(), body[..i].to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(shape: Shape, text: &str) -> Vec<(String, String)> {
        let mut a = Assignments::new(shape);
        text.lines().flat_map(|l| a.line(l)).collect()
    }

    fn kv(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn shapes_come_from_the_file_name_or_the_shebang() {
        assert_eq!(Shape::of("./Dockerfile", ""), Some(Shape::Dockerfile));
        assert_eq!(Shape::of("a/Dockerfile.prod", ""), Some(Shape::Dockerfile));
        assert_eq!(Shape::of("api.dockerfile", ""), Some(Shape::Dockerfile));
        assert_eq!(Shape::of("x\\Containerfile", ""), Some(Shape::Dockerfile));
        assert_eq!(Shape::of("run.sh", ""), Some(Shape::Shell));
        assert_eq!(Shape::of("run.zsh", ""), Some(Shape::Shell));
        assert_eq!(
            Shape::of("deploy", "#!/usr/bin/env bash"),
            Some(Shape::Shell)
        );
        assert_eq!(Shape::of("deploy", "#!/bin/sh -e"), Some(Shape::Shell));
        assert_eq!(Shape::of("deploy", "#!/usr/bin/python3"), None);
        assert_eq!(Shape::of("settings.py", ""), Some(Shape::Python));
        assert_eq!(Shape::of("notes.md", "#!/bin/sh"), None);
        assert_eq!(Shape::of("<stdin>", ""), None);
    }

    #[test]
    fn dockerfile_env_and_arg_in_every_form() {
        let text = "FROM x\nENV DB_PASSWORD=abc123\nARG TOKEN_X=\"quoted val\"\n\
                    env KEY value with spaces\nENV A=1 B=2\nARG NPM_TOKEN\nARG\nRUN x=1\n";
        assert_eq!(
            all(Shape::Dockerfile, text),
            vec![
                kv("DB_PASSWORD", "abc123"),
                kv("TOKEN_X", "quoted val"),
                kv("KEY", "value with spaces"),
                kv("A", "1"),
                kv("B", "2"),
            ]
        );
    }

    #[test]
    fn dockerfile_continuation_lines_are_read() {
        let text = "ENV A=1 \\\n    DB_PASSWORD=abc123 \\\n    # note\n    C=3\nRUN B=2\n";
        assert_eq!(
            all(Shape::Dockerfile, text),
            vec![kv("A", "1"), kv("DB_PASSWORD", "abc123"), kv("C", "3")]
        );
    }

    #[test]
    fn dockerfile_references_are_not_values() {
        let text =
            "ENV P=${DB_PASSWORD}\nENV Q=$OTHER\nENV R $THIRD\nARG S=$(cmd)\nENV T=\nENV U=\"\"\n";
        assert!(all(Shape::Dockerfile, text).is_empty());
    }

    #[test]
    fn shell_assignments_and_command_prefixes() {
        let text = "#!/bin/bash\nexport DB_PASSWORD=\"abc 123\"\nAPI_KEY=abc123\n\
                    readonly T='$notexpanded'\nexport A=1 B=2\ndeclare -x D=4\n\
                    FOO=bar psql -h db\n  indented=1\n# C=3\n";
        assert_eq!(
            all(Shape::Shell, text),
            vec![
                kv("DB_PASSWORD", "abc 123"),
                kv("API_KEY", "abc123"),
                kv("T", "$notexpanded"),
                kv("A", "1"),
                kv("B", "2"),
                kv("D", "4"),
                kv("FOO", "bar"),
                kv("indented", "1"),
            ]
        );
    }

    #[test]
    fn shell_expansions_and_empties_are_not_values() {
        let text = "export A=$B\nexport C=\"${D}\"\nE=$(vault read x)\nF=`cmd`\nG=\"\"\nH=\n\
                    export I\nread -s PASSWORD\necho \"x=y\"\nif [ a=b ]; then :; fi\n";
        assert!(
            all(Shape::Shell, text).is_empty(),
            "{:?}",
            all(Shape::Shell, text)
        );
    }

    #[test]
    fn python_string_literal_assignments_only() {
        let text =
            "DB_PASSWORD = \"abc\"  # note\nTOKEN: str = 'xyz'\n    nested = \"in a block\"\n";
        assert_eq!(
            all(Shape::Python, text),
            vec![
                kv("DB_PASSWORD", "abc"),
                kv("TOKEN", "xyz"),
                kv("nested", "in a block")
            ]
        );
    }

    #[test]
    fn python_expressions_are_not_literals() {
        let text =
            "password = os.environ[\"X\"]\ntoken = os.getenv(\"Y\")\nsecret = settings.SECRET\n\
                    a = f\"{x}\"\nb = \"a\" + c\nc = \"\"\nd = \"\"\"doc\"\"\"\nif p == \"abc\":\n\
                    def f(password: str):\ne: str\nf = (\"x\")\nreturn password != \"\"\n";
        assert!(
            all(Shape::Python, text).is_empty(),
            "{:?}",
            all(Shape::Python, text)
        );
    }
}
