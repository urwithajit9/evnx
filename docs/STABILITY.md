# Stability

What you can rely on in evnx, what may still change before 1.0, and what changed
in this release. evnx follows [Semantic Versioning](https://semver.org); before 1.0 a
minor release (0.14 to 0.15) may change behaviour, and the CHANGELOG says so under
"Breaking".

## Stable

These do not change without a CHANGELOG "Breaking" entry, and are tested.

- **Exit codes.** `0` ran and found nothing, `1` ran and found something, `2` could not
  run, `4` declined (a question answered no, or needed with no terminal and no `--yes`).
  `restore` adds `5` (written aside), the account and cloud commands `10` to `15`, and a
  closed pipe is `141`. `run` and `cloud run` exit with the command's own code, or `128+n`
  when signal `n` killed it. One table: [exit-codes.md](exit-codes.md), also in every
  `--help` and in `evnx commands --json`. A code means the same everywhere: `2` is never a
  finding and `1` is never a failure to run.
- **JSON output.** Every `--format json` document starts with a `run` header (`tool`,
  `version`, `schema_version`, `ruleset`, `features`). Changes that add fields keep the shape and
  raise `schema_version`; a change that renames or removes one is a "Breaking" entry.
  `evnx commands --json` (`schema` 1) lists every command, flag group and exit code.
- **Flag spellings after the 0.15.0 vocabulary change.** The same idea has the same
  spelling in every command: `--env` (a file), `--env-name` (a name), `--example`,
  `-C/--dir`, `--format` with `--json` as its shorthand, `--yes`, `-n/--dry-run`,
  `--show-values`. See "Breaking: one flag vocabulary" in the CHANGELOG for the table.
- **Reading commands do not print values unless asked.** `parse` needs `--values`,
  `diff` needs `--show-values`, `get` needs `--reveal` for a name that looks secret,
  and a terminal needs `--show-values` for `parse --values` and `restore --output -`.
  `run` hands values to the command in its environment, never in its arguments.
  (`migrate` prints the commands you will run, and those carry the values by design.)
- **`run` keeps the file's value over your environment's by default** (`--prefer vault`).
  `--prefer env` is the opt-in for the other way round.
- **The scan baseline** (`.evnx/baseline.json`) stays `schema_version` 1. A field added
  to it is optional (an entry's `reason` is the first), and an evnx that does not know a
  field ignores it, so a team on mixed versions can share one committed baseline.
- **Files that can hold a secret** are created owner-only (mode 600; an owner-only ACL
  on Windows), and a file evnx replaces is kept as `.bak`.

## May change before 1.0

- Human-readable text: wording, spacing, colour and the order of lines. Parse the JSON,
  not the pretty output.
- Anything marked experimental in `--help` (`convert --to railway`) and the `.evnx.toml`
  keys not documented in the README.
- Which commands need an optional feature (`net`, `migrate`, `backup`, `cloud`);
  `evnx commands` says what a given build has.
- `cloud`, `vault`, `org` and `platform` follow the server they talk to.

## How a removal is announced

1. The CHANGELOG "Breaking" section names it, in the release that makes it.
2. A removed flag spelling is not silently accepted and not silently ignored: typing it exits
   `2` with `'<old>' was removed from 'evnx <command>' in <version>; use '<new>'`, and
   nothing runs. `evnx commands --json` lists each under `removed`.
3. A deprecated spelling (for example `org accept TOKEN`) keeps working with a warning
   first; the CHANGELOG entry says until when.

## Changed defaults in 0.16.0

The headlines of the CHANGELOG's "Breaking" section for the 0.16.0 release (others are
added there as they land).

- **A "no" at a confirmation prompt now exits 4.**

## Changed defaults in 0.15.0

The headlines of the CHANGELOG's "Breaking" section for this release. Each entry there
says who is hit and what to pass to get the old behaviour.

- **One flag vocabulary: the old spellings are removed**
- **`--env-name` pairs the environment with its own template.**
- **`--env-name` takes a name, not a path.**
- **Outside a git repository, only the working folder's `.evnx.toml` applies.**
- **`platform --env-name` is now `--target`.**
- **Generated contracts mark a name required only when the template gives it no default.**
- **Templates never turn debugging on.**
- **`template` no longer replaces an existing output without `--force`**
- **`remove` refuses to leave dangling references.**
- **`cloud pull` and `platform pull` write the pulled file 600**
- **`add` and `init --from-source` write no dated `# Added by evnx … on <date>` line**
- **`convert --to yaml` quotes every value**
- **Exit codes follow one table**
- **`run` and `cloud run` refuse more names that start a program or load code**
- **`auth token create` expires in 90 days and needs a scope**
- **`vault share` gives the `viewer` role by default**
- **`vault delete` needs the full vault name**
- **`org seats --set` and `--unlimited` ask first**
- **`org accept TOKEN` is deprecated**
- **`cloud push` asks before removing most of the keys**
- **`platform push --prune` stops when it would remove most of an environment**
- **`platform` warns about a readable token file and names an overridden API**
- **Windows: `.env`, backups and `credentials.json` become owner-only**
- **`diff A B` compares two files, and the left file is printed first**
- **`migrate --to vercel` prints `--force` only with `--overwrite`**
- **`validate` runs the URL, port and email checks by default, as warnings**
- **`EVNX_AUTO_FIX` is no longer read**
- **`parse --format json` is `run.schema_version` 2**
- **`restore --output -` writes the decrypted text to stdout**
- **The install URL is `https://evnx.dev/install.sh` everywhere**
