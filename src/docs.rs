// src/docs.rs
//
// The documentation URLs this binary prints — in `--help` via cli.rs
// `after_help`, and in command output via `ui::print_docs_hint`.
//
// ⚠️ This file is NOT a single source of truth, and the header here used to say
// it was: "change it here — it propagates automatically". Nothing propagated.
// `after_help` feeds Clap's derive attribute, so it has to be a `&'static str`
// literal with no allocation, which means every URL below spells out the host
// in full. When the guides moved to docs.evnx.dev, all 33 of them kept pointing
// at the old one — and kept *working*, because a 301 is not a broken link, so
// nothing failed and nothing warned. They shipped that way in two releases.
//
// BASE_URL is the host and prefix every URL below must start with, and the
// tests at the bottom are what actually hold them to it.

pub const BASE_URL: &str = "https://docs.evnx.dev/cli";

/// A documentation entry for one evnx command.
///
/// All fields are `&'static str` so they can be used directly
/// in Clap's `#[command(after_help = …)]` derive attribute
/// without allocations or LazyLock.
pub struct CommandDoc {
    /// The subcommand name, e.g. "init"
    pub command: &'static str,
    /// Full URL to the command's guide page
    pub url: &'static str,
    /// One-liner shown alongside the URL in terminal output
    pub description: &'static str,
    /// Pre-formatted string for Clap's after_help (static, no allocation)
    pub after_help: &'static str,
}

impl CommandDoc {
    /// Compact one-line hint for end of command output.
    ///
    /// Output: `  Docs: https://docs.evnx.dev/cli/commands/init`
    pub fn hint_line(&self) -> String {
        format!("  Docs: {}", self.url)
    }
}

// ── Registry ──────────────────────────────────────────────────────────────────
//
// One entry per subcommand. `after_help` is a plain string literal so it can
// be used directly in #[command(after_help = docs::INIT.after_help)].

pub const INIT: CommandDoc = CommandDoc {
    command: "init",
    url: "https://docs.evnx.dev/cli/commands/init",
    description: "Project setup, stacks, and service presets",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/init",
};

pub const SPEC: CommandDoc = CommandDoc {
    command: "spec",
    url: "https://docs.evnx.dev/cli/commands/spec",
    description: "Declare what each variable is: required, format, secret",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/spec",
};

pub const ADD: CommandDoc = CommandDoc {
    command: "add",
    url: "https://docs.evnx.dev/cli/commands/add",
    description: "Adding variables interactively or from blueprints",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/add",
};

pub const REMOVE: CommandDoc = CommandDoc {
    command: "remove",
    url: "https://docs.evnx.dev/cli/commands/remove",
    description: "Removing a variable from the env file, template and contract",
    after_help: "Examples:
  evnx remove OLD_TOKEN                     from .env only
  evnx remove OLD_TOKEN --all --dry-run     every file, the template and .evnx.toml: preview
  evnx rm 'TEST_*' --env-name staging --yes from .env.staging, without asking
  evnx remove OLD_TOKEN --check             CI: exit 1 if it is still in any env file, the template or .evnx.toml

Full guide: https://docs.evnx.dev/cli/commands/remove",
};

pub const VALIDATE: CommandDoc = CommandDoc {
    command: "validate",
    url: "https://docs.evnx.dev/cli/commands/validate",
    description: "Validation rules, CI flags, and strict mode",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/validate",
};

pub const SCAN: CommandDoc = CommandDoc {
    command: "scan",
    url: "https://docs.evnx.dev/cli/commands/scan",
    description: "Secret detection patterns and entropy analysis",
    // ⚠️ The rule names are the ones `[scan] disable` takes (I-056). A test in
    // `core::secretscan::rules` fails when a built-in is missing here.
    after_help: "\
What a folder walk reads:\n\
\x20 every file, except .git, node_modules, target, dist and build, the\n\
\x20 .env.example family, evnx's own files in .evnx/ (rule files, rules.lock,\n\
\x20 the baseline, backups.json, platform state), editor swap files, binary\n\
\x20 files (except PKCS#12 keystores, found by their content), and files over\n\
\x20 1 MiB (--max-file-size). A file named on the command line is always read.\n\
\x20 In a git repository, findings in files git ignores and does not track\n\
\x20 are summarised, not counted (--include-ignored counts them).\n\
\x20 Lockfiles (package-lock.json, pnpm-lock.yaml, yarn.lock, Cargo.lock, go.sum\n\
\x20 and the like) get the key-format rules only, not name or entropy checks.\n\
\n\
Encoded values:\n\
\x20 a value of 16 to 4096 characters that is base64, hex or percent-encoded\n\
\x20 text is decoded once and checked against the rules below only (not names\n\
\x20 or entropy). The finding shows \"encoded\" in JSON.\n\
\n\
Context:\n\
\x20 a value on a line with an Authorization or Bearer header, an HTTP client\n\
\x20 call or a connection string is shown one level higher; a file under a\n\
\x20 test, tests, mock, spec or fixtures folder one level lower (name, entropy\n\
\x20 and medium findings only). Nothing is dropped, and the exit code and\n\
\x20 --confidence use the level before the hint.\n\
\n\
Accepting a finding:\n\
\x20 evnx:allow <rule> reason=used-in-tests|false-positive|fix-later, then\n\
\x20 free text, on the line or the one above; --update-baseline --reason CODE\n\
\x20 does the same for the baseline. The summary counts by reason.\n\
\n\
Rules, by the names [scan] disable takes (or by id: evnx scan -v --format json):\n\
\x20 AWS Access Key, AWS Temporary Access Key (STS), GitHub Fine-grained PAT,\n\
\x20 Slack Token, Google API Key, JSON Web Token, Stripe Secret Key (LIVE),\n\
\x20 Clerk Secret Key (LIVE), UploadThing Secret Key, Stripe Secret Key (test),\n\
\x20 GitHub Token, OpenAI API Key, Anthropic API Key,\n\
\x20 OpenSSH Private Key, Private Key, Credentials in a connection string,\n\
\x20 Stripe Restricted Key (LIVE), Azure Storage Account Key, Azure SAS Token,\n\
\x20 Twilio API Key, SendGrid API Key, Mailgun API Key, DigitalOcean Token,\n\
\x20 npm Access Token, PyPI API Token, Docker Hub Access Token,\n\
\x20 Shopify Access Token, HashiCorp Vault Token, GitLab Personal Access Token,\n\
\x20 Hugging Face Token, Heroku API Key, Sentry DSN, Sentry Auth Token,\n\
\x20 Linear API Key, Discord Bot Token, Telegram Bot Token, Slack Webhook URL,\n\
\x20 Google OAuth Client Secret, GitLab Runner Authentication Token, GitLab\n\
\x20 Deploy Token, GitLab Pipeline Trigger Token, Groq API Key, xAI API Key,\n\
\x20 Replicate API Token, Perplexity API Key, Stripe Webhook Signing Secret,\n\
\x20 Databricks Personal Access Token, Atlassian API Token, Bitbucket App\n\
\x20 Password, HCP Terraform API Token, Pulumi Access Token, Doppler Token,\n\
\x20 Supabase Personal Access Token, Supabase Secret Key, age Secret Key, New\n\
\x20 Relic User API Key, Grafana Service Account Token, Grafana Cloud Access\n\
\x20 Policy Token, Postman API Key, Notion Integration Token, GitLab CI/CD\n\
\x20 Job Token, Fly.io Access Token, Azure DevOps Personal Access Token,\n\
\x20 GitLab Feed Token, GitLab Incoming Email Token, GitLab Agent for\n\
\x20 Kubernetes Token, GitLab Workspace Token, GitLab SCIM Token, GitLab\n\
\x20 Feature Flags Client Token, GitLab OAuth Application Secret;\n\
\x20 and the heuristics: Sensitive config key, High-entropy string,\n\
\x20 AWS Secret Access Key.\n\
\n\
A short value under a secret name (PASSWORD=admin, 4 to 7 characters) is\n\
reported as Sensitive config key, medium severity and low confidence, only\n\
when --confidence low (or [scan] confidence = \"low\") is given.\n\
\n\
A policy the repository cannot lower:\n\
\x20 evnx scan --policy ci/scan-policy.toml        (or EVNX_POLICY)\n\
\x20 required_rules = [\"AWS Access Key\"]    detectors .evnx.toml may not disable\n\
\x20 max_severity_floor = \"medium\"          highest --severity or [scan] severity\n\
\x20 allow_disable = false | [\"Slack Token\"]   what .evnx.toml may disable\n\
\x20 A conflict exits 2 before any file is read, naming the key and the policy.\n\
\x20 --strict makes an unknown key in .evnx.toml exit 2; --exit-only prints\n\
\x20 nothing and answers with the exit code: 0 clean, 1 findings, 2 could not run.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/scan",
};

pub const DIFF: CommandDoc = CommandDoc {
    command: "diff",
    url: "https://docs.evnx.dev/cli/commands/diff",
    description: "Comparing .env vs .env.example",
    // ⚠️ The rule for what is compared lives here, beside the exit codes,
    // because it decides the exit code (I-078, owner 2026-10-21).
    after_help: "\
What is compared:\n\
\x20 .env against a template (.env.example, [defaults] example, or --example)\n\
\x20     key names only: a filled-in value differs from its placeholder by design.\n\
\x20     --values compares the values too.\n\
\x20 two environments (--env-name production --against staging, or two files)\n\
\x20     key names, and which values differ (values hidden unless --show-values).\n\
\x20     With two files the left one is listed first.\n\
\n\
Examples:\n\
\x20 evnx diff                                  # keys .env lacks, or the template lacks\n\
\x20 evnx diff .env.staging .env.production     # two files: keys and values, left first\n\
\x20 evnx diff --fail-on missing                # fail only when .env lacks a template key\n\
\x20 evnx diff --env-name production --against staging\n\
\x20 evnx diff --format json | jq -e '.summary.missing == 0'\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/diff",
};

pub const CONVERT: CommandDoc = CommandDoc {
    command: "convert",
    url: "https://docs.evnx.dev/cli/commands/convert",
    description: "All 15 output formats and filtering options",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/convert",
};

pub const SYNC: CommandDoc = CommandDoc {
    command: "sync",
    url: "https://docs.evnx.dev/cli/commands/sync",
    description: "Keeping .env and .env.example in sync",
    after_help: "Examples:
  evnx sync --check --direction forward   CI gate: does the template have everything .env has?
  evnx sync --prune --dry-run             preview names in the template that no env file has
  evnx sync --prune --yes                 remove them, without asking

Full guide: https://docs.evnx.dev/cli/commands/sync",
};

pub const ALIGN: CommandDoc = CommandDoc {
    command: "align",
    url: "https://docs.evnx.dev/cli/commands/align",
    description: "Giving every .env the template's sections and key order",
    after_help: "\
What it does:\n\
\x20 The template decides headings, comments, blank lines and key order. Each\n\
\x20 file keeps its own assignments, moved byte for byte: values, quoting,\n\
\x20 inline comments and multiline values are never rewritten.\n\
\x20 A key the file lacks gets a # KEY= placeholder: never the template's value,\n\
\x20 and never a blank KEY=, so what the app reads does not change. Keys the\n\
\x20 template lacks are kept, in order, under \"Environment-specific settings\".\n\
\n\
Which template: the one every command pairs with the file. --example, else\n\
.env.<name>.example for .env.<name> when it exists, else [defaults] example,\n\
else .env.example. Files named together may follow different templates.\n\
\n\
Comments only the file has are kept: one directly above an assignment moves\n\
with it, one followed by a blank line stays after the assignment above it.\n\
A comment the template also has is the template's. Duplicate keys, a line\n\
that does not parse, mixed line endings, or more copies of a template comment\n\
than the template has stop the run, and nothing is written.\n\
\n\
Examples:\n\
\x20 evnx align --env .env.development --env .env.test\n\
\x20 evnx align --example .env.example --env .env --dry-run\n\
\x20 evnx align --env-name production --check\n\
\n\
Every file is read and checked before any is written, and one already aligned\n\
is not touched. A file that changes is kept as .bak first, replacing the last\n\
one (--no-backup to skip); a .bak that .gitignore does not cover is refused,\n\
since it would hold the secrets. If a later file fails, the ones already\n\
replaced are put back. Another evnx run writing the same file stops this one.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/align",
};

pub const MIGRATE: CommandDoc = CommandDoc {
    command: "migrate",
    url: "https://docs.evnx.dev/cli/commands/migrate",
    description: "Migrating secrets to cloud managers",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/migrate",
};

pub const DOCTOR: CommandDoc = CommandDoc {
    command: "doctor",
    url: "https://docs.evnx.dev/cli/commands/doctor",
    description: "Diagnosing setup and gitignore issues",
    // The exit codes and the variable are here because nothing else said them
    // (D-118). EVNX_AUTO_FIX is no longer read: a CI image that exports it
    // gets one warning and no repair.
    after_help: "\
Env files: .env and .env.<name> (not .envrc), and plaintext copies such as\n\
.env.bak; each must be ignored by git and not tracked, valid dotenv, and\n\
private (no access for group or others). The template, meant to be\n\
committed: [defaults] example in .evnx.toml, else .env.example, .env.sample\n\
or .env.template, whichever is there.\n\
\n\
Environment:\n\
\x20 EVNX_OUTPUT_JSON=1  same as --format json; an explicit --format wins\n\
\n\
EVNX_AUTO_FIX is no longer read: it prints one warning and repairs nothing.\n\
Pass --fix.\n\
\n\
Examples:\n\
\x20 evnx doctor --fix                       # repair .gitignore and file modes\n\
\x20 evnx doctor --all                       # doctor, validate, sync --check and scan in one report\n\
\x20 evnx doctor --strict --format json      # a CI gate: warnings fail too\n\
\x20 evnx doctor --skip-without-env          # a pre-commit hook on a fresh clone\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/doctor",
};

pub const TEMPLATE: CommandDoc = CommandDoc {
    command: "template",
    url: "https://docs.evnx.dev/cli/commands/template",
    description: "Generating config files from templates",
    after_help: "\
References:\n\
\x20 {{VAR}}  ${VAR}  $VAR        replaced by the value from the env file\n\
\n\
Filters: {{VAR|filter}}, chained left to right: {{VAR|default:8080|int}}\n\
\x20 upper  lower                 case\n\
\x20 title                        Title Case For Each Word\n\
\x20 capitalize                   First character only\n\
\x20 bool                         true/yes/on/1/y or false/no/off/0/n; anything else is an error\n\
\x20 int                          a whole number; anything else is an error\n\
\x20 json  yaml  toml            a quoted, escaped string for that syntax\n\
\x20 sh                           one shell word, single-quoted\n\
\x20 default:VALUE                VALUE when VAR is empty or not defined\n\
An unknown filter is an error and nothing is written.\n\
\\${VAR} \\{{VAR}} \\$VAR are written as they are, without the backslash.\n\
$VAR is replaced only when VAR is defined, and never reported.\n\
A placeholder with no value is an error too (exit 2); --allow-unresolved\n\
writes it through as it is.\n\
\n\
Examples:\n\
\x20 evnx template config.tpl config.yml     # INPUT OUTPUT\n\
\x20 evnx template config.tpl -              # to stdout\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/template",
};

pub const BACKUP: CommandDoc = CommandDoc {
    command: "backup",
    url: "https://docs.evnx.dev/cli/commands/backup",
    description: "AES-256-GCM encrypted backups",
    after_help: "\
Passphrase: --key-file, then EVNX_PASSWORD, then a prompt (asked twice). A new\n\
backup needs at least 8 characters from any of them, and is told when the\n\
passphrase is weak (under 12 characters, digits only, a common word).\n\
\n\
The backup file is encrypted: committing it is as safe as its passphrase.\n\
--gitignore adds it to .gitignore; --no-gitignore leaves .gitignore alone.\n\
\n\
A backup is a file you keep, behind a passphrase you choose; it needs no account\n\
and no network. To share and version secrets instead, see `evnx cloud push`.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/backup",
};

pub const RESTORE: CommandDoc = CommandDoc {
    command: "restore",
    url: "https://docs.evnx.dev/cli/commands/restore",
    description: "Restoring from encrypted backups",
    after_help: "\
Replacing a file: an existing --output is never replaced silently. With a\n\
terminal evnx asks; without one, pass --yes. Either way the\n\
replaced file is kept beside it as <output>.bak. Content identical to the\n\
backup is left alone.\n\
\n\
Passphrase: --key-file, then EVNX_PASSWORD, then a prompt.\n\
\n\
--output - prints the decrypted text to stdout, with every status line on\n\
stderr; a terminal refuses it unless --show-values. Text that is not a .env\n\
goes to <output>.restored instead: a private file (0600) that holds plaintext\n\
secrets, printed with its full path.\n\
\n\
A backup file is not a vault. For evnx cloud, use `evnx cloud pull` (one\n\
version) or `evnx cloud export` (all of them).\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/restore",
};

pub const CLOUD: CommandDoc = CommandDoc {
    command: "cloud",
    url: "https://docs.evnx.dev/cli/commands/cloud",
    description: "Zero-knowledge encrypted .env sync",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/cloud",
};

pub const PLATFORM: CommandDoc = CommandDoc {
    command: "platform",
    url: "https://docs.evnx.dev/cli/commands/platform",
    description: "Compare and sync .env with Vercel's variables",
    after_help: "Examples:
  evnx platform diff --to vercel                         what differs from Vercel's development variables
  evnx platform diff --to vercel --target production   .env.production against production
  evnx platform pull --to vercel --dry-run               what a pull would change, nothing written
  evnx platform pull --to vercel --yes                   write Vercel's readable values, no question

Auth: VERCEL_TOKEN, or --token-file <path>. There is no --token flag.
Project: --project / --team, or VERCEL_PROJECT_ID / VERCEL_ORG_ID, or .vercel/project.json.
Config variables can be read back; Secrets cannot, so they are compared by presence only
and never pulled. Values are never printed.

Full guide: https://docs.evnx.dev/cli/commands/platform",
};

pub const AUTH: CommandDoc = CommandDoc {
    command: "auth",
    url: "https://docs.evnx.dev/cli/commands/auth",
    description: "Account registration and sign-in",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/auth",
};

pub const VAULT: CommandDoc = CommandDoc {
    command: "vault",
    url: "https://docs.evnx.dev/cli/commands/vault",
    description: "Creating and managing encrypted vaults",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/vault",
};

pub const ORG: CommandDoc = CommandDoc {
    command: "org",
    url: "https://docs.evnx.dev/cli/commands/org",
    description: "Organisations: a plan, a directory, and seats",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/org",
};

pub const PARSE: CommandDoc = CommandDoc {
    command: "parse",
    url: "https://docs.evnx.dev/cli/commands/parse",
    description: "What evnx reads from a .env: keys, lines and flags, never values",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/parse",
};

pub const CONFIG: CommandDoc = CommandDoc {
    command: "config",
    url: "https://docs.evnx.dev/cli/commands/config",
    description: "Every effective setting and where it came from",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/config",
};

pub const HISTORY: CommandDoc = CommandDoc {
    command: "history",
    url: "https://docs.evnx.dev/cli/commands/history",
    description: "The copies evnx kept before changing a file: which file, command and time",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/history",
};

pub const UNDO: CommandDoc = CommandDoc {
    command: "undo",
    url: "https://docs.evnx.dev/cli/commands/undo",
    description: "Put a file back as it was before evnx last changed it",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/undo",
};

pub const RUN: CommandDoc = CommandDoc {
    command: "run",
    url: "https://docs.evnx.dev/cli/commands/run",
    description: "Run a command with a .env's variables, secrets masked in its output",
    after_help: "\
Which value wins: by default the env file replaces a variable your environment\n\
already holds. --prefer env reverses that, so a local value is kept and the file\n\
only fills the names you did not set. Testing with a local value:\n\
\x20 DB_URL=postgres://localhost evnx run --prefer env -- npm test\n\
\n\
What masking does not cover: values the command transforms (encoded, split,\n\
hashed), files it writes, core dumps, /proc/<pid>/environ (readable by the same\n\
user), and programs that log their own environment. Masking pipes stdout and\n\
stderr, so the command sees no terminal: colour and progress output turn off,\n\
and Python holds its output in a buffer, so set PYTHONUNBUFFERED=1. --no-mask\n\
leaves the streams as they are.\n\
\n\
Names that start a program or load code (PATH, LD_*, EDITOR, PAGER, VISUAL,\n\
GIT_ASKPASS, SSH_ASKPASS, CLASSPATH, npm_config_*, GIT_CONFIG_*, ...) are\n\
refused unless --allow-unsafe-names. Proxy and certificate names (HTTPS_PROXY,\n\
SSL_CERT_FILE, ...) only produce a warning that names them.\n\
\n\
Signals and exit (Unix): without masking the command replaces evnx, so signals,\n\
the terminal and the exit status are the command's own. With masking evnx stays\n\
to filter the output and passes SIGTERM, SIGHUP, SIGQUIT and SIGINT on to the\n\
command (not to what it starts), then exits with its code, or 128+n when signal\n\
n killed it. Windows: no signal forwarding and no job object, so the command can\n\
outlive evnx when the console closes; its exit code is passed through.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/run",
};

pub const COMMANDS: CommandDoc = CommandDoc {
    command: "commands",
    url: "https://docs.evnx.dev/cli/commands/commands",
    description: "What this build can do, and which features it has",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/commands",
};

pub const COMPLETIONS: CommandDoc = CommandDoc {
    command: "completions",
    url: "https://docs.evnx.dev/cli/commands/completions",
    description: "Shell completion scripts, and where each shell reads them",
    after_help: "\
Install, for the shell you use (create the folder first if it does not exist),\n\
then open a new shell:\n\
\x20 bash        evnx completions bash > ~/.local/share/bash-completion/completions/evnx\n\
\x20 zsh         evnx completions zsh > ~/.zfunc/_evnx\n\
\x20             and in ~/.zshrc, before compinit: fpath=(~/.zfunc $fpath)\n\
\x20 fish        evnx completions fish > ~/.config/fish/completions/evnx.fish\n\
\x20 powershell  evnx completions powershell >> $PROFILE\n\
\x20 elvish      evnx completions elvish >> ~/.config/elvish/rc.elv\n\
\n\
The script lists this build's commands: generate it again after an upgrade.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/completions",
};

pub const SET: CommandDoc = CommandDoc {
    command: "set",
    url: "https://docs.evnx.dev/cli/commands/set",
    description: "Setting one variable in place",
    after_help: "Examples:
  evnx set PORT=8080                        change one line, keep the rest
  evnx set API_TOKEN                        typed with no echo, or read from stdin
  printf %s \"$TOKEN\" | evnx set API_TOKEN   from a pipe, nothing in shell history
  evnx set LOG_LEVEL=info --if-missing      only when the name is not there yet
  evnx set PORT=8080 --dry-run              show the change, write nothing
  evnx set TLS_KEY --from-file key.pem      a certificate or key from a file; never printed
  evnx set --fill                           ask for every empty or placeholder value
  evnx set --fill --dry-run                 list the names it would ask for; no terminal needed

Full guide: https://docs.evnx.dev/cli/commands/set",
};

pub const GET: CommandDoc = CommandDoc {
    command: "get",
    url: "https://docs.evnx.dev/cli/commands/get",
    description: "Printing one variable's value",
    after_help: "Examples:
  PORT=$(evnx get PORT)                     one value, no sourcing
  evnx get --exists DATABASE_URL            exit 0 when set, 1 when not; prints nothing
  evnx get API_TOKEN --reveal               a secret is printed only on request
  evnx get PORT --env-name staging          from .env.staging

Full guide: https://docs.evnx.dev/cli/commands/get",
};

pub const UPDATE: CommandDoc = CommandDoc {
    command: "update",
    url: "https://docs.evnx.dev/cli/commands/update",
    description: "How to upgrade, and whether a newer release exists",
    after_help: "\
Without --check the exit code is 0, however many copies --list finds. With\n\
--check, 1 means a newer release exists, and 2 that it could not tell.\n\
\n\
More than one evnx installed? `evnx update --list` shows each copy, its\n\
version, which one runs, and how to remove the rest. Only --check uses the\n\
network.\n\
\n\
-q prints only the upgrade command; with --check, the status line and the command.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/update",
};

/// Every `CommandDoc` in this file.
///
/// Hand-maintained, which is a risk — so
/// `the_list_above_covers_every_command_doc_in_this_file` counts the
/// declarations in this file's own source and fails if the two disagree.
///
/// Not test-only. `evnx surface` reads it so the documentation URL the tour
/// links to is the same string the binary prints in `--help`. Two lists
/// would be two things to keep in step, which is the failure this whole area
/// exists to stop.
pub const ALL: &[&CommandDoc] = &[
    &INIT,
    &SPEC,
    &ADD,
    &REMOVE,
    &SET,
    &GET,
    &VALIDATE,
    &SCAN,
    &DIFF,
    &CONVERT,
    &SYNC,
    &ALIGN,
    &MIGRATE,
    &DOCTOR,
    &TEMPLATE,
    &BACKUP,
    &RESTORE,
    &CLOUD,
    &PLATFORM,
    &AUTH,
    &VAULT,
    &ORG,
    &PARSE,
    &HISTORY,
    &CONFIG,
    &UNDO,
    &RUN,
    &COMMANDS,
    &COMPLETIONS,
    &UPDATE,
];

/// The documentation URL for a top-level command, if it has one.
///
/// `None` is a real answer, not a gap to paper over: a command that ships
/// before its guide has no `CommandDoc` yet. A caller that invented a URL from
/// the command name would produce a 404 for exactly that command. Today every
/// top-level command has an entry, and `tests/surface_integration.rs` checks
/// it (I-195).
pub fn url_for(command: &str) -> Option<&'static str> {
    ALL.iter().find(|d| d.command == command).map(|d| d.url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test P6 needed and did not have.
    ///
    /// A stale URL that 301s is the one kind of wrong nothing notices: it
    /// resolves, the reader lands on the right page, and the extra hop is
    /// invisible unless something is looking for it. Nothing was.
    #[test]
    fn every_url_points_at_the_docs_host() {
        for d in ALL {
            assert!(
                d.url.starts_with(BASE_URL),
                "{}: url is {}, which does not start with {BASE_URL}",
                d.command,
                d.url
            );
        }
    }

    /// `after_help` is what a reader sees in `--help`; `url` is what the hint
    /// line prints. A copy-paste leaving one on another command's guide sends
    /// people somewhere plausible and wrong, which is worse than a 404.
    #[test]
    fn after_help_links_to_the_same_page_as_url() {
        for d in ALL {
            assert!(
                d.after_help.contains(d.url),
                "{}: after_help does not contain its own url {}",
                d.command,
                d.url
            );
        }
    }

    /// A trailing slash is a 308 here, and the instinct to add one comes
    /// from `app.evnx.dev`, where it is the opposite: that host is a static
    /// export and 308s `/login` *to* `/login/`. Verified against the live
    /// host — `/cli/commands/init/` redirects back to `/cli/commands/init`.
    #[test]
    fn no_url_ends_in_a_slash() {
        for d in ALL {
            assert!(
                !d.url.ends_with('/'),
                "{}: {} ends in a slash, which docs.evnx.dev 308s away",
                d.command,
                d.url
            );
        }
    }

    /// The pre-split path must not come back in any printed field.
    #[test]
    fn the_old_guides_path_is_gone() {
        for d in ALL {
            for (field, text) in [
                ("url", d.url),
                ("after_help", d.after_help),
                ("description", d.description),
            ] {
                assert!(
                    !text.contains("/guides"),
                    "{}: {field} still references the pre-split /guides path",
                    d.command
                );
            }
        }
    }

    /// Reads this file's own source so a command added above cannot sit
    /// outside every test here by being left out of `ALL`.
    #[test]
    fn the_list_above_covers_every_command_doc_in_this_file() {
        let declared = include_str!("docs.rs")
            .lines()
            .filter(|l| l.starts_with("pub const ") && l.contains(": CommandDoc"))
            .count();
        assert_eq!(
            declared,
            ALL.len(),
            "{declared} CommandDoc consts are declared in docs.rs but ALL lists {} \
             — add the new one to ALL",
            ALL.len()
        );
    }
}
