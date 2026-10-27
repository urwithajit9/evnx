# Changelog

All notable changes to evnx are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

⚠️ **Breaking changes, and how to migrate:**
- **`template`: an unresolved placeholder is an error** (RM-14, DEC-T2, I-142).
  `{{X}}` with no value used to be written through literally, exit 0; now the
  run exits 2, writes nothing, and lists every name. *Migrate:* add
  `--allow-unresolved` where the pass-through was intended. `--strict` is now
  the default; it is still accepted, with a notice, and is removed in 1.0.
- **`spec`: a regex format is written `regex:<pattern>`** (RM-15, DEC-SP6,
  I-188). A name that is not a format used to be compiled as a regex, so a typo
  (`urll`) failed every value; it is now an error when the contract is read,
  with "did you mean". *Migrate:* prefix bare patterns:
  `format = "^sk_"` → `format = "regex:^sk_"`.
- **`diff --format json`: `values_redacted` is gone; `schema_version` is 2**
  (RM-16). *Migrate:* read `value_state` (`withheld`, `masked`, `shown`);
  `value_state == "masked"` is the old `values_redacted == true`.
- **`restore` writes back to the file the backup was made from** (RM-18, I-029).
  A backup of `.env.staging` used to restore over `.env`; without `--output` it
  now restores to `.env.staging`, in the current folder. A recorded name that is
  not `.env` or `.env.<name>` still restores to `.env`. *Migrate:* pass
  `--output .env` to keep the old destination.
- **A relative `[defaults] example` is read from the folder holding
  `.evnx.toml`** (I-332), the way `.gitignore` and `docker-compose.yml` read
  their paths. A root `.evnx.toml` naming `deploy/compose/.env.example` used to
  send a command run inside `deploy/compose` to
  `deploy/compose/deploy/compose/.env.example`. Nothing changes for a command
  run in the folder that holds the file. *Migrate:* a path written relative to
  a subfolder you run commands from should be rewritten relative to the
  `.evnx.toml`.

⚠️ **`init` and `add` honour `[defaults] env_name`.** A project whose
`.evnx.toml` sets `env_name = "production"` now has `init` and `add` write
`.env.production`, the file `validate`, `sync` and `diff` already read there,
instead of `.env`. Pass `--env-name` to choose another.

- **`install.sh` reports the copy it installed, and names the one that runs**
  (RM-20). Live on `main` (and `dotenv.space/install.sh`) on merge, not with
  this release:
  - "Installed version" is the version of the file just written, not of
    whichever `evnx` is first on `PATH`; another copy ahead on `PATH` is named,
    with the command that removes it (npm, pipx, pip, Homebrew, cargo).
  - A copy that Homebrew, npm or pip owns is never replaced: the script stops
    and prints that manager's upgrade command.
  - A fresh account installs into `~/.local/bin`, created if missing, instead of
    asking for `sudo`.
  - A folder not on `PATH` ends with the `export` line and exit 0, not
    "Installation failed". Another copy on `PATH` is then named as the one
    `evnx` still runs, and never offered for removal: the first version of
    this change told the user to `rm` what could be their only working copy.
  - `EVNX_VERSION=0.10.18` pins a release; `EVNX_INSTALL_DIR=…` picks the
    folder.
- **`cloud export --to` takes `convert`'s names and collision rule** (RM-07).
  `--to gcp-secrets`, `azure-keyvault`, `heroku` and `kubernetes` wrote
  `my-project`, `my-keyvault`, `YOUR_APP_NAME` and `app-secrets`, and two names
  the format merges were both written. They now need `--project`,
  `--vault-name`, `--heroku-app` or `--name` (`--namespace` optional), refused
  before any request when missing; a collision is refused unless
  `--allow-collisions` is given. The file is what `evnx convert` prints for the
  same input and flags.
- **`sync --check --format json` sorts `missing` and `template_only` again**
  (RM-11). 0.10.12 to 0.10.18 printed them in file order, unannounced, while
  the guide said "Keys are sorted".
- **`init` names a `.env` in a subfolder** (I-331). In a monorepo, `init` at the
  root now says that `deploy/compose/.env` exists, that commands run at the
  root read the root's file, and how to set up the subfolder
  (`evnx init -p deploy/compose`). Dependency, build and hidden folders are
  skipped.
- **`init --env-name NAME` and `add … --env-name NAME`** (RM-17, MEC-2, I-027,
  I-033). `init` creates or fills `.env.NAME` from the template; `add` writes
  `.env.NAME` and the template, and no other env file. One file per run: loop
  over names for several. A name is checked as on the read side (`../x` is
  refused). The hint for a missing `.env.NAME` — "Create it with `evnx init`",
  which wrote `.env` — now says `evnx init --env-name NAME` (C-2).
- **One "did you mean" for every `--env-name`** (RM-17): an abbreviation, then
  the closest spelling, so `prodution` suggests `production`.
- 🔒 **Azure secrets never reach a command line** (RM-06, I-086). `convert --to
  azure-keyvault` and `migrate --to azure-keyvault` printed `az keyvault secret
  set --value <secret>`, visible in `ps`, shell history and `set -x` logs. Each
  secret is now written to a private temp file through a quoted heredoc and
  read with `--file … --encoding utf-8`; the file is removed afterwards.
- **`doctor` warns when `.dockerignore` lets `.env` into an image** (RM-22). With
  a `Dockerfile`, a missing `.dockerignore`, or one that does not exclude
  `.env*`, is a warning; `--fix` adds `.env*` and the template exceptions.
- **`migrate` in CI names each secret** (RM-08, I-274): with no terminal, the
  per-secret `✓`/`✗` lines were swallowed by the hidden progress bar, so a log
  said "1 failed" and named nothing. A missing token now says "Set
  `GITHUB_TOKEN`, or pass `--github-token`" (I-273); `--dry-run` says "would
  print commands for" for the destinations evnx does not upload to; menus no
  longer end in `?:`.
- **A master-password prompt with no terminal names `--password-stdin`**
  (I-330, RM-01). `cloud push`, `pull`, `run` and `export`, `vault create`,
  `share`, `revoke` and `rekey`, and `auth login`, `register`,
  `rotate-master-password` and `undo-password-change`, run in CI without the
  flag, failed with "reading the master password: IO error: not a terminal".
  They now say "no terminal to ask for the master password: pipe it with
  --password-stdin" and show the command with the flag added. Exit code
  unchanged (2).
- **`init` writes `.evnx.toml` before printing anything** (RM-03). `init
  --blueprint t3_modern --yes | head -n 1` exited 141 with `.env`,
  `.env.example` and `.gitignore` written but no `.evnx.toml`. All four are now
  written first; the lines printed, and their order, are unchanged.
- **`sync` audits real values written through `allow_actual`** (RM-05, I-284).
  They were written with no `[AUDIT]` line while the run said "placeholder
  values" and the dry-run marked them `·`. The `[AUDIT]` line now prints, the
  message says "and real values for N keys in allow_actual", and the preview
  marks them `!`.
- **`doctor` reports every detected project type** (RM-23), not the first:
  `requirements.txt` beside `package.json` gets the Python and the Node.js
  checks.
- **`validate` suggests `--fix` only when a finding shown is fixable** (RM-09).
- **Hints and messages that named the wrong thing:**
  - `org remove`, `delete` and `leave` with no terminal name the member and
    `--org <slug>`; `remove` printed a literal `<member>` (RM-02).
  - `template --output -` refused with "Nothing was written to -."; it now says
    "Nothing was printed." (RM-13).
  - `scan`: one held-back finding says "lists it" (I-290);
    `scan.max_file_size` is announced in KiB, as the skip note says it (I-291);
    `scan.disable` counts "built-in detector(s)", since the names include
    heuristics (I-292).
  - `init --dry-run -p DIR` names every file with its folder (I-333).
- **Help texts that described old behaviour** (RM-21): `auth register` (three
  public keys, ML-KEM-768 included), `auth rotate-master-password` (`evnx vault
  rekey` exists), `auth download-data` (`cloud export` has shipped), `add custom`
  (`*_PWD` is hidden too), and `validate --rotate-weak-secrets`, whose example
  rendered as one flattened line.

## [0.10.18] - 2026-10-25

The patch for `evnx org`, `vault` and `cloud` (package R-org-vault-cloud,
#223). Sharing no longer rewrites an existing member; every command proves the
master password before using it, so `vault create` no longer seals a vault
under a typo; admins can share and members can leave; every confirmation names
`--yes` when there is no terminal; `-q` works for every cloud command; `cloud
watch` reports failed polls and skips your own pushes; pushing the default
`.env` into a production vault asks first; and the README covers all of
`vault`, `cloud` and `org`, checked against the CLI.

⚠️ **Scripts:**
- `cloud push` of the default `.env` into a production vault with no terminal
  now exits 2 unless `--yes` (or `--file`/`--env-name`).
- Under `-q`, cloud commands print far less: a script that read a status line
  ("✓ pushed …") from stdout under `-q` gets nothing.
- `vault create` with upper case exits 2 before the password prompt.
- `cloud history`'s `PUSHED BY` holds emails, and its width follows them.
- `cloud pull` into a git repository may print a `.gitignore` question or
  warning; `--no-gitignore` skips it.

ⓘ **Shipped separately, on evnx-server** (`d2663fa`, live since
2026-10-25): sharing with someone already in a vault is a 409 and writes
nothing — for every client, older CLIs and the app included — and a refused
vault name says "lower-case letters, digits and hyphens" once. This release
behaves the same against the server before and after it.

🔒 **Security:** `tokio` 1.49.0 → 1.53.3 (RUSTSEC-2026-0386, published
2026-10-13, informational "unsound": `AsyncFd::new` and related constructors
violate I/O safety, which can lead to a use-after-free). evnx never calls
`AsyncFd`; tokio is there for `reqwest`'s HTTP client. 1.53.3 needs `mio` 1.2,
`socket2` 0.6.3 and, through `mio`, `libc` 0.2.183, so those move too.

🔒 Sharing with someone already in a vault rewrote their role and key —
the owner's too, so an admin could demote the owner and then remove them
(I-313). Fixed on the server (above); this release also refuses it before the
password.

- **A role is read the same way by `vault` and `org`** (A-303, DEC-VL10).
  `vault share`, `vault role`, `org invite` and `org role` had three copies of
  the rule: `org` did not trim spaces, `vault share --role owner` was refused
  without saying why, and each worded the refusal its own way. One parser now:
  spaces trimmed, any case, `owner` refused with the reason ("A vault has one
  owner, set once by whoever created it." / the organisation's), anything else
  as "role must be one of: … Got `x`.". Only the refusals' wording changed;
  every role accepted before is accepted now.
- **`evnx vault members <id>` works with a vault-scoped API token** (I-309,
  A-301). It listed every vault before asking for the members, and such a token
  is refused the list by design, so it failed with a bare "Insufficient
  permissions". An id now needs no lookup, as in `cloud pull`; by name, the
  token is told to pass the id. A vault named by id is shown by its id in the
  header, with a token or a login. The vault commands now find a vault one way
  (`list_and_pick`), where six of them had their own copy.
- 🔒 **`evnx vault share` no longer rewrites an existing member** (I-313,
  I-314). Sharing with someone already in the vault replaced their role and
  their key and printed "shared": with your own address an owner became a
  developer and could no longer delete the vault, and an admin could demote the
  owner on the way to removing them. `share` now reads the member list first,
  before the key lookup and the password: yourself and the owner are refused;
  someone at another role is pointed at `evnx vault role` with the arguments
  that would do it; someone already at the role asked for gets "already … —
  nothing changed." and exit 0, as `vault role` does. evnx-server refuses the
  same requests with a 409 since SV-3, for older CLIs and the app.
- **The master password is checked before it is used** (I-316, A-302). Every
  command that asks for it (`vault create`, `share`, `revoke`, `rekey`,
  `cloud push`, `pull`, `run`, `export`) derived a key from whatever was typed
  and found out later, at a vault key, saying the password was "probably
  wrong". `vault create` opens no vault key, so it sealed a new vault under the
  typo: created, counted against the plan, never openable with the real
  password. Now the password opens your account's sealed keypair first, and a
  wrong one is refused right after the prompt with the sentence
  `rotate-master-password` already used ("that master password did not open
  your keypair … Nothing has been changed."). A vault whose own copy still will
  not open with a proven password was created with a typo; the message says so
  and how to get out of it. Opening a vault shared with you no longer fetches
  and opens your keypair a second time.

  ⚠️ **If you created a vault with a mistyped password and pushed to it with
  that same typo**, this release refuses the typo at the check. evnx 0.10.17
  still opens it with the typo: export its versions with 0.10.17, then delete
  the vault and create it again.
- **Admins can share from the CLI** (I-311, DEC-VL5). An admin holds a copy
  shared to them, and `vault share` refused it ("re-sharing is not supported")
  after the password, though the server allows admins to share. It is opened
  with your keypair, as `vault rekey` already did. A developer or viewer is told
  that sharing needs admin or owner, and an admin asking for `--role admin` that
  only the owner can make an admin — both before the password.
- **`evnx vault leave <vault>`** (I-312, I-318, DEC-VL6). A member could not
  leave a vault from the CLI, though the server allows it; `vault revoke --user
  <yourself>` said "ask an admin to remove you" — to the owner too. `leave`
  confirms (`-y` without a terminal), deletes your membership and says the key
  was not rotated. The owner cannot leave; revoking yourself points at `leave`,
  or tells the owner why not.
- **Revoking the owner is refused before any work** (I-317). An admin revoking
  the owner re-encrypted every version and only then heard the server refuse,
  leaving the staged blobs in storage.
- **A vault name is lower-case letters, digits and hyphens, checked before the
  password** (I-315, DEC-VL8). Upper case passed the CLI and came back from the
  server as "name: name: only alphanumeric and hyphens". The refusal suggests
  the lower-case name.
- **`--version` is 1 or more** for `cloud pull`, `run` and `delete-version`, and
  a version that does not exist is named, with `evnx cloud history` to look
  (I-303, DEC-CL11). `--version 0` went to the server and came back as a raw API
  path.
- **`cloud history` names who pushed by email** (I-319); it showed an
  8-character id for everyone but you. Someone who has left keeps the short id.
- **The push hints name the vault** (I-320). `vault create`, `history`, `diff`,
  `pull`, `run` and `export` said "Push one with `evnx cloud push`", which fails
  where no directory is bound; they say `evnx cloud push --vault <vault>`. By
  name, a vault-scoped token is told to pass the vault's id without naming
  `--vault`, which `vault members` does not take.
- **Every cloud confirmation names `--yes` when there is no terminal** (I-218).
  `vault delete`, `revoke` and `rekey`, `cloud delete-version`, `auth sessions
  revoke` and `revoke-others`, `auth token revoke` and
  `rotate-master-password` failed with "IO error: not a terminal: not a
  terminal"; `cloud pull` over a different file names `--force`. Nothing was or
  is sent before the answer.
- **`-q` for every cloud command** (I-225, DEC-CL3, A-44). It reached only the
  config banner and `cloud status`/`unlink`. Now `auth`, `org`, `vault`,
  `cloud`, `auth token`, `sessions`, `devices`, `totp` and
  `rotate-master-password` print nothing on success under `-q`; tables, a new
  token (alone on stdout), recovery codes, warnings and errors still print. A
  `crate::say!` macro is the shared output handle.
- **`cloud watch` reports failed polls and skips your own pushes** (I-229,
  DEC-CL5). Failures showed only with `-v`, so an expired login looked like a
  quiet vault. From the third failed poll in a row it says so on stderr, after
  ten it stops with exit 2, and it says when polling works again. Your own
  pushes are skipped unless `--include-mine`.
- **`cloud pull` keeps the file it writes out of git** (owner, 2026-10-17): the
  `.gitignore` step `template`, `convert` and `restore` use, with `--gitignore`
  and `--no-gitignore`. `--file -` writes no file and checks nothing.
- **Pushing the default `.env` into a production vault asks first** (I-234,
  DEC-CL9). A forgotten `--file` pushed development secrets into production with
  only a warning. On a terminal it asks; without one it is refused unless
  `--yes` (`-y`, new on `push`). A file named with `--file` or `--env-name` is
  not asked about. By id, a signed-in session looks the environment up for the
  question; a vault-scoped token cannot and is not asked.
- **README: the whole of `vault`, `cloud` and `org`** (D-155 … D-158, D-301,
  D-302, DEC-VL9): the role table, `rekey`, `--no-rekey`, `leave`,
  `delete-version`, `unlink`, every `org` command, which file push and pull use,
  what works by id with a vault-scoped token, and `cloud export --output` (it
  said `--out`). A test checks the commands against the CLI both ways.
- `org` subcommands' `--org <slug>` have help text (DEC-SF12).

## [0.10.17] - 2026-10-23

The patch for `evnx auth` (package R-auth, #221). `evnx auth status` says why a
paid organisation is not raising your plan; vault commands work right after the
verification link is opened, without signing in again; a refused API token is
reported as one, not as an expired session; the `login` and `totp` help point
CI at API tokens; the README covers all of `evnx auth`; and an error code this
build does not know keeps the server's sentence.

⚠️ **Scripts that read `evnx auth` output:** with a refused `EVNX_TOKEN`,
`evnx auth status` prints `status  API token refused` where it printed
`status  session expired`, and other commands print "the API token in
EVNX_TOKEN was refused …" where they printed "your session has expired …".
Exit codes are unchanged.

ⓘ **Shipped separately, on evnx-server:** a 429 carries the real time left and
a `Retry-After` header; malformed requests, unknown routes and wrong methods
answer with `{error, code}`; plan limits add `"reason": "PLAN_LIMIT"`; login
starts are limited to 30 per 15 minutes per address (wrong passwords still lock
it after 5). This release behaves the same against the server before and after.

🔒 **Security:** `ctutils` 0.4.2 → 0.4.5 (RUSTSEC-2026-0385, published
2026-10-23: non-constant-time mask generation and `u128` comparisons). It sits
under `ml-kem`, the post-quantum half of the key wrap `evnx vault share` uses,
so the fix is a timing side channel in work done on your own machine. No
change to what is encrypted or how; nothing to rotate.

⚠️ **Building from source now needs Rust 1.87** (`rust-version` was 1.85).
Every fixed `ctutils` release requires it. Only `cargo install evnx` on an older
toolchain is affected; the npm, Homebrew, Scoop, winget, PyPI and GitHub
Release binaries are built on stable as before.

- **`evnx auth status` says why a paid organisation is not raising your plan**
  (I-321). The server applies the plan of the organisation you hold a seat in,
  else your own, and paying for seats hands none out: an organisation's owner
  is created without one. A customer whose organisation pays for Team saw
  `plan (free)` and nothing else, which reads like a failed payment. Under the
  plan block, `status` now names each organisation on a higher plan where you
  hold no seat, and the next step: `evnx org assign <you> --org <slug>` for an
  owner or admin with a seat free; release one or raise the count when every
  seat is taken; who to ask, for a member; and that your one seat is elsewhere,
  with the `evnx org release` that frees it. `-q` leaves it out, and if
  `GET /orgs` fails the block is left out rather than failing `status`.
  app.evnx.dev shows the same notice.
- **No second sign-in after verifying your email** (I-301, DEC-AU301). The
  session's access token carries `email_verified` from when it was issued, so
  for up to 15 minutes after the link was opened every vault command said
  "your email address is not verified yet" while `evnx auth status` said
  `verified  yes`. When the server answers EMAIL_NOT_VERIFIED to a signed-in
  session, the CLI now refreshes the session once and retries the request; the
  refreshed token is issued from the database. Still unverified after that,
  the message is as before. API tokens have no refresh and are unchanged.
- **A refused API token says so** (I-302, DEC-AU302; the auth I-302 in the
  review's `issues.csv` — #219 reused the number for `init`). A revoked,
  expired or mistyped `EVNX_TOKEN` was reported as "your session has expired.
  Run `evnx auth login`", the wrong cause and a fix a CI job cannot run. It now
  reads "the API token in EVNX_TOKEN was refused", with `evnx auth token
  create` as the fix, and `evnx auth status` shows `status  API token refused`
  instead of `session expired`. The exit code is unchanged (2; `status` 0). An
  expired login still says to run `evnx auth login`.
- **`evnx auth login --help` and `evnx auth totp --help` say CI uses API
  tokens** (DEC-AU11). A second factor cannot be scripted, by design, and a
  pipeline wired to `login` found out only when it stopped for a code. Both
  now point unattended jobs at `evnx auth token create` and `EVNX_TOKEN`. Help
  text only; the one-line summaries in `evnx auth --help` are unchanged.
- **README: the whole of `evnx auth`, and where it keeps your login** (D-148,
  D-150, DEC-AU9). The README named 3 of the 19 auth commands and gave
  `~/.config/evnx/config.toml` as the only config path, which is right on Linux
  alone. A new "Your account" section lists all 19, the folder on Linux, macOS
  and Windows, `EVNX_CONFIG_DIR`, `config.toml` and `credentials.json` (mode
  0600), and the three environment variables. A test checks the table against
  the CLI in both directions, so a command added or renamed later fails it.
- **An error code this build does not know keeps the server's sentence**
  (I-306). It printed "unexpected response from …: HTTP 403" and dropped the
  sentence that said what was wrong — which is why the server could not give
  plan limits a code of their own without breaking every installed CLI. A
  refusal with an unrecognised code and a sentence now prints the sentence;
  one with no sentence (a proxy's error page) still names the status. The
  server marks plan limits with `"reason": "PLAN_LIMIT"` beside `code:
  FORBIDDEN` (evnx-server, SV-4), which every version of the CLI reads as
  before.

## [0.10.16] - 2026-10-22

The patch for `evnx init` (package R-init: #217, #218, #219). init now keeps
the `.evnx.toml` contract, writes into the template the project already uses,
catches an existing `.env` up, sets up a fresh clone under `--yes`, says which
env files it leaves alone, and has `--dry-run` and `--json`. Also: what
`--from-source` finds, browser names for the framework the stack uses,
`compose.yaml`, the target folder's own `.evnx.toml`, when init may ask, and
five new components.

⚠️ **Scripts and CI jobs that run `evnx init`:**

- **init writes `.evnx.toml`** (I-242, DQ-17, DQ-18, DQ-26, DQ-27). With none
  at or above the target, one is created in the target folder (below the
  repository root too, so a package's variables are not required of its
  siblings), with a `[vars]` entry for each variable — the entries
  `spec init --with` writes — and an empty `[vars]` for a blank init. A config
  that has `[vars]` gets the new names recorded, as `evnx add` records them; one
  without gets `[vars]` entries appended, its other sections kept byte for
  byte. Commit it with the template. Every evnx command in that folder then
  prints its `config .evnx.toml` line, `validate` checks against the contract,
  and `spec init` refuses the file (it has `[vars]`): use `spec init --merge`.
- **`init --yes` on a fresh clone succeeds** (DQ-31). With the template there
  and no stack named, it keeps the template, creates `.env` from it (0600) and
  exits 0; it exited 2 ("refusing to overwrite"). With `.env` there too it says
  "already set up". `--with` or `--blueprint` over an existing template still
  exits 2 unless `--force`.
- **The project's template** (I-295, DQ-28): `[defaults] example` in
  `.evnx.toml`, else `.env.example`, `.env.sample` or `.env.template`, else a new
  `.env.example` — the rule doctor follows. Beside a `.env.sample`, init wrote a
  second template, `.env.example`; now `--with x --yes` refuses over
  `.env.sample` like over `.env.example`, and `--force` replaces it. `add`
  appends to it too.
- **An existing `.env` catches up** (I-297, DQ-29): the names the new template
  has and `.env` lacks are appended below its values, with placeholders, through
  add's writer; a name `.env` has is never touched. It kept 2 names beside a
  10-variable template, while "Next steps" told the user to replace placeholders.
- **`CI` set means no questions** (DQ-30). With `CI` set (GitHub Actions,
  GitLab CI and most others set `CI=true`), init refuses its menu and its
  confirmations the way it does without a terminal: exit 2, naming the flags.
  Runs that pass `--yes`, `--with`, `--blueprint` or `--detect` are unchanged;
  `CI=false` and `CI=0` are not CI. A terminal on stdin with stderr redirected
  (`evnx init 2>log`) is refused the same way: it failed with `IO error: not a
  terminal: not a terminal` (I-299).
- **The target folder's `.evnx.toml` is read** (I-298). With `-p <dir>`, a
  malformed `.evnx.toml` or a bare `evnx.toml` there stops init before anything
  is written (exit 2), as it already did in the working folder. When the
  target's config is another file than the working folder's, its `config` line
  and its warnings are printed too.
- **`NEXT_PUBLIC_*` names only on Next.js** (I-002b, DQ-23). Clerk's
  `NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY` and Vercel's `NEXT_PUBLIC_VERCEL_ENV` are
  written only when the stack has Next.js. Another browser framework gets
  Clerk's key under its own prefix (`VITE_CLERK_PUBLISHABLE_KEY`, …), as before;
  a server stack gets neither. The same for `--detect`, `add service` and
  `add blueprint`: the `content_marketing` blueprint (Nuxt) has one variable
  fewer. A name already in your files is not removed.

### init

- **`--dry-run` and `--json`** (IN-16, DQ-21). `--dry-run` lists what the run
  would write — the template, `.env`, `.gitignore`, `.evnx.toml` and the notices
  — and writes nothing; it asks no confirmation (a menu is still shown when
  nothing was named), and refuses over an existing template as `--yes` does.
  `--json` prints the result, or with `--dry-run` the plan, as one document with
  `add --json`'s fields (`template`, `gitignore`, `contract`, `notices` are new
  in both); like add's, it needs `--yes` or `--dry-run`.
- **The env files init does not write are named** (I-296, DQ-32): `not changed:
  .env.production, .env.staging — check one with: evnx validate --env-name
  production`; `.env.local` is said to be read before `.env` by Next.js and
  Vite; a `[defaults] env_name` pointing at another file is a warning.
  `--env-name` for init is planned for 0.11.0.
- **`.envrc`** (I-174, DQ-33) stays ignored by `.env*`; when an untracked one is
  there, one line says so and how to commit it (`!.envrc`).
- **Five components** (IN-14, DQ-25): Astro (browser prefix `PUBLIC_`; `HOST`,
  `PORT` for `@astrojs/node`), Hono, Neon (`DATABASE_URL` pooled,
  `DATABASE_URL_UNPOOLED` direct), Cloudflare Turnstile (the site key is
  browser-safe and gets prefixed copies, the secret key does not; the
  descriptions name Cloudflare's test keys) and Fly.io (`FLY_API_TOKEN`).
  `--detect` reads `astro`, `hono`, `@neondatabase/serverless` and `fly.toml`.
  `--list-components` lists 56; each new entry is last in its group, so in the
  Architect services list the items after the databases move down one.
- "Next steps" says to commit the template and `.evnx.toml`.
- **A stack refused over an existing template** (I-302) now also says how to
  add its variables to the template instead, keeping what is there: `evnx add
  blueprint <id>` or `evnx add service <id>`. It named only `--force` and
  removing the file.
- **Under `CI`, the refusal gives `CI` as the reason** (I-303): `CI is set, so
  init does not ask`. At a real terminal it said "there is no terminal to show
  the menu on".
- `--from-source` with nothing new to add creates `.env` from the template when
  there is none, as its other paths do (N-06).
- **A `.env` git would still commit** gets the shared warning (E6): `'.env' is
  still not ignored: a later rule re-includes it`, or, for a tracked `.env`,
  `'.env' is tracked by git` with `git rm --cached .env`; it was told a rule
  re-included it.
- **`--from-source` finds more of what the code reads** (I-236, DQ-22):
  destructuring — `const { API_KEY, DB_URL: db, PORT = 3000 } = process.env`,
  and the same from `import.meta.env` and `Bun.env`, over several lines or with
  a TypeScript annotation; a rest element names nothing —, a member access split
  across lines (`process` ⏎ `.env.X`) and `process.env?.X`; `os.getenv("X")`
  (Python), `ENV.fetch("X")` (Ruby), `getenv("X")` (PHP), and `env!("X")`,
  `option_env!("X")` and `env::var_os("X")` (Rust). Each is reported at the line
  the name is on.
- **`--detect` reads `compose.yaml`** (I-257), the name Docker Compose prefers.
  It read `docker-compose.yml`, `docker-compose.yaml` and `compose.yml` only. The
  four names are now one list with validate's and doctor's (`core::project`).
- **An unknown blueprint gets the hint a component gets** (I-235):
  `--blueprint mern` suggests `mern_v2` (a prefix or substring, then a near
  spelling). A blueprint id given to `--with` says `t3_modern is a blueprint:
  use --blueprint t3_modern`; a component id given to `--blueprint` says
  `postgresql is a component: use --with postgresql`.
- The menu asks `How do you want to start?`, without a colon after the
  question mark.

### add

- **The project's template** (I-295, DQ-28): `add` appends to `[defaults]
  example`, or the `.env.example`, `.env.sample` or `.env.template` the folder
  has, and names it in its messages; it always wrote `.env.example`. A
  `.evnx.toml` in `--path` that does not parse stops `add` before it writes
  (it wrote, then said it could not update `[vars]`).
- **`--json`** gains `template`, `gitignore` (entries a new `.env` added),
  `contract` (`action`, `file`, `added`) and `notices`, the fields `init --json`
  prints (DQ-21).
- **A `.env` git would commit** gets the shared warning (E6): `'.env' may
  contain secrets and is not in .gitignore`, then `evnx doctor --fix`; it said
  `.env is not covered by .gitignore`.
- `add blueprint` with an unknown id prints the same "Did you mean" line as
  `init --blueprint` (I-235): the two commands share one message.
- `add service clerk`, `add service vercel` and `add blueprint` write
  `NEXT_PUBLIC_*` names only in a Next.js project (I-002b, above): `add service
  vercel` in a project without Next.js adds 3 variables, not 4.
- Its menus (`add` with no target, `add custom`'s category) no longer add a
  colon after a question: `Which framework?`, not `Which framework?:`.

### scan

- A variable the contract declares `secret = true` is not reported when its
  value is a stand-in: evnx's own `postgresql://user:password@localhost…`, a run
  of `x` after a key prefix, the placeholders the other rules already drop
  (`core::classify::is_stand_in`). With init declaring `DATABASE_URL` secret, a
  new project's `.env` was a high "Declared secret" and no longer scanned clean
  (N-20).

### Tests

- `backup_pty`'s two `t_br32` tests wait 60 s for the question restore asks
  after deriving the key: two debug-build Argon2 derivations came before it, and
  they timed out once on a macOS runner (I-300).

## [0.10.15] - 2026-10-22

The patch for `evnx doctor` (package R-doctor). Doctor now checks every env
file, plaintext copies included, and stops calling `.envrc` one. Syntax errors
and a template without `.env` are errors, and a check that cannot run is
reported by name instead of stopping the run. The template is the project's
own: `[defaults] example` in `.evnx.toml`, else `.env.example`, `.env.sample`
or `.env.template` (#215).

⚠️ **Scripts, CI jobs and hooks that run `evnx doctor`:**

- **A syntax error is an error** (I-161, DEC-D7): exit 1, the verdict `validate`
  gives. It was a warning, so a `.env` the parser rejects passed with exit 0. A
  file doctor cannot read — no permission, not UTF-8 — is an error that says
  which.
- **No env file is a verdict of its own** (I-162, DEC-D8). Beside a template
  (`.env.example`) a missing `.env` is an error, exit 1, with the command that
  creates it (`evnx sync --direction reverse`). In a project without a template
  it stays a warning. A folder with no env file, no template and no project
  files is exit 2, "nothing to diagnose here": an empty or wrong checkout used
  to be "healthy".
- **`--skip-without-env`** (DEC-D17): with no env file, say so on stderr and
  exit 0, as for `diff` and `sync`. **The `evnx-doctor` pre-commit hook** now
  runs `evnx doctor --skip-without-env`, so a fresh clone can commit; its
  description says what fails a commit (errors) and what does not (warnings,
  D-120).
- **A check that cannot run no longer stops the others** (I-163, DEC-D9). A
  directory named `requirements.txt` ended the run with `Error: Is a directory
  (os error 21)`: no check name, no path, no summary, and in JSON mode nothing on
  stdout. That check now reads `could not run: Is a directory
  (requirements.txt)`, the rest finish, and the run exits 2 with "No complete
  verdict". JSON gains `ran` on every check (`false` for one that could not
  run). A fix that cannot write — a read-only folder — is "not fixed", with the
  reason, exit 1; it aborted the run too.
- **`.envrc`, `.environment` and `.env-notes.md` are not env files** (I-173,
  DEC-D3): only `.env` and `.env.<name>` are. A committed direnv `.envrc` was a
  security error with exit 1. ⓘ `--fix` still writes `.env*`, which also
  matches `.envrc`; that is `init`'s decision (I-174).
- **Plaintext copies are checked** (I-160, DEC-D5, DEC-D18). `.env.backup`,
  `.env.bak`, `.env.old`, `.env.save` and `.env~` that hold plain text are
  errors until git ignores them, like the file they copy, and their permissions
  are checked. An `evnx backup` file is recognised by its content and skipped:
  every name containing `.backup` used to be skipped, so `cp .env .env.backup`
  was never checked.
- **Structured files named after `.env`** (`.env.json`, `.env.yaml`) are
  checked for ignore and permissions, but not as dotenv (DEC-D19).

### doctor

- **Every env file's permissions** (I-158, DEC-D4), each named:
  `.env.production: permissions 644 (recommended: 600)`. Only `.env` was
  checked, so `.env.production` at 644 gave "checks passed". `--fix` sets each
  readable one to 600, never a template. Private means no access for group or
  others (I-170): `700` and `500` pass; doctor accepted exactly 600 and 400.
- **Symlinked env files are seen** (I-159, DEC-D6). A symlinked `.env` was "not
  found" (no ignore or syntax check), while the permission check followed the
  link and `--fix` chmod'ed its target outside the project. It is now listed as
  `.env (symlink to …)`, checked like a file, and `--fix` changes a target only
  inside the project; outside, it prints the `chmod` to run.
- **The template is the project's, not always `.env.example`** (I-294). The
  `env_example` check uses `[defaults] example` from `.evnx.toml` — the rule
  `validate`, `sync`, `diff` and `spec init` follow — else the first of
  `.env.example`, `.env.sample`, `.env.template`. A project using `.env.sample`
  was told ".env.example not found". `doctor PATH` reads PATH's own
  `.evnx.toml`. The JSON check keeps its name and description; `details` names
  the file, and the verbose line says `.env.example is tracked in Git`.
- `EVNX_AUTO_FIX=1` says on stderr that it turned repair on (I-167, DEC-D12).
- `--help` has Environment, Examples and Exit codes sections (D-118, DEC-D12),
  and `--fix`'s help says what it changes (D-119).
- A skipped permission check keeps its description; the reason is in `details`
  (I-166).
- README and Getting Started describe what doctor checks; the Getting Started
  sample is real output (D-114, D-115, D-116, D-117).

### `--env-name`

- `.env.old` and `.env.save` are copies, not environments (DEC-D20): the
  "Available" list in an `--env-name` error and `validate`'s known environments
  no longer offer `old` or `save`. One copy rule with `doctor`.

### Internal

- `core::env_name::kind` classifies a file name (env, template, copy, other)
  for `doctor`, `--env-name` and `scan`; `TEMPLATE_SUFFIXES` and
  `STRUCTURED_EXTENSIONS` are the one list each (scan output unchanged).
  `core::envfile::is_evnx_backup` recognises a backup without the `backup`
  feature; `core::permissions::is_private_mode` is the one rule.
  `utils::fs::{has,set}_secure_permissions` are deprecated wrappers over
  `core::permissions` (I-170). `doctor`'s and `scan`'s exit codes are
  `core::verdict`'s. `core::gitignore::is_tracked` is public; doctor used its
  own `git ls-files`.
- One loop builds doctor's report for both renderers (I-169).

## [0.10.14] - 2026-10-21

The patch for `evnx scan` (package R-scan). In a git repository a folder walk
counts what git would commit, so a filled-in `.env` that `.gitignore` covers no
longer fails the scan. Placeholders and addresses are no longer reported as
secrets, and the advice under the findings names the real files.

⚠️ **Scripts, CI jobs and hooks that run `evnx scan`:**

- **Files git ignores and does not track are summarised, not counted**
  (I-285, DEC-SC1). In a git repository a folder walk still reads them, but
  their findings are one line — `git ignores ./.env — 4 findings there are not
  counted, because it is not committed` — and do not make the run exit 1. A
  tracked file is reported whatever `.gitignore` says now; a file git would
  take on the next `git add .` is reported. A file named on the command line,
  `--include-ignored`, and any folder outside a repository report everything,
  as before. In CI the checkout has no ignored files, so a gate there is
  unchanged. JSON gains `ignored_by_git` (per file) and
  `summary.ignored_by_git`, and each finding a `git` field (`tracked`,
  `unignored`, `ignored`) — all absent outside a repository. SARIF and
  `--format github` leave ignored files out.
- **Stand-ins are not findings** (I-050, N-20). A format match on a run of
  filler (`sk_live_xxxx…`, `ghp_0000…`), the documented AWS example key, or the
  `user:password@localhost` URL evnx itself writes into new projects is no
  longer reported, with or without `--ignore-placeholders`. `init --blueprint
  fastapi_ai` (or `rust_high_perf`, `django_enterprise`) then `scan` is clean.
- **A name-only finding needs a value that could be a secret** (I-287,
  DEC-SC2). `NEXTAUTH_URL=http://localhost:3000`, `AUTH0_DOMAIN=…auth0.com`,
  `TOKEN_URL=https://…/token`, `SECRET_MANAGER_REGION=us-east-1` and an email
  address are no longer reported for the `AUTH`/`TOKEN`/`SECRET` in their
  names. A URL with a password, a query string or a token-like path segment
  (a Slack or Discord webhook) still is.
- **`--exclude NAME` matches whole path components** (I-053). It was a
  substring of the whole path: `--exclude env` dropped `.env`, and `--exclude
  test` dropped `latest/` and `contest.py`. A value with `*`, `?` or `[` is a
  glob, as before.
- **`[scan] disable` silences the name-based finding too** (I-056). Disabling
  `GitHub Token` left `Sensitive config key: GITHUB_TOKEN`, high, for the same
  value. The heuristics can be disabled by name: `Sensitive config key`,
  `High-entropy string`, `AWS Secret Access Key`. `scan --help` lists every
  name `disable` takes.
- **`*.env` files are read as `.env`** (I-254): `app.env`, `production.env`.
  `[vars]` applies to them, and findings name the variable
  (`app.env:2 (APP_NAME)`).
- **Output is sorted** by path, then line, then rule (I-062), and a path given
  twice, or a file inside a folder also given, is scanned once (I-054).
- **`-q` keeps the line saying the project config weakens the scan** (I-060):
  `config .evnx.toml (scan.severity=high, …)`. Otherwise `-q` is findings only.

### scan

- **"Next steps" follow what git knows** (I-286). History rewriting is advised
  only for files that may have been committed, as `git filter-repo --path
  <file>` with the real file — it said `--path .env` whatever held the key.
  A file git would commit gets "keep it out of git"; an ignored file named on
  the command line gets "nothing was committed".
- New flags: `--include-ignored` (above); `--max-file-size SIZE` and
  `[scan] max_file_size` (`500K`, `5MB`, `2MiB`; default 1 MiB, DEC-SC4) for the
  walk's size limit, announced when a project sets it lower (I-061);
  `--no-ignore-placeholders` undoes `[scan] ignore_placeholders = true` for one
  run (I-057).
- "Nothing was scanned" names the files a walk skipped and why, and no longer
  advises `--include`, a flag that never existed (I-048, I-289).
- A private key's preview shows its header, `-----BEGIN RSA PRIVATE KEY-----`,
  not `----…----` (I-051); a connection-string finding says `a password in a
  postgres:// URL` instead of "matches a live key format" (I-052).
- A `[[scan.patterns]]` rule no longer reports its own `regex = "…"`
  declaration in `.evnx.toml` (D-36); the file is still scanned.
- `-v` counts built-in rules and custom patterns apart and names them (I-058):
  it called the 13 built-ins "13 custom pattern(s)".
- `--help`: `path` and `--exclude` have help text (DEC-SF12); `--severity
  high` is described correctly — it keeps long values under secret-like names
  too, so a committed `JWT_SECRET` still fails the gate (I-288, DEC-SC3); a new
  epilogue says what a walk reads, the rule names and the exit codes.

### Internal

- The rule engine (`PatternSet`, `DetectorRegistry`, `Confidence`) moved from
  `commands::scan` to `core::secretscan` (A-09); the old paths re-export it.
  `sync` no longer imports a command, and `core::classify` uses the same
  compiled rules instead of compiling them a second time.
- `core::gitignore::exposures` answers for many files at once, two git
  processes per repository. `core::classify::is_stand_in`,
  `core::placeholder::is_url_placeholder`, `core::args::parse_size` and
  `core::config::either` are new shared helpers.

## [0.10.13] - 2026-10-21

The patch for `evnx diff` (package R-diff). Against a template, `diff` compares
key names only; between two environments it compares values too.

It also updates the TLS certificate checks that the cloud commands,
`migrate --to github-actions` and `update` rely on: two advisories published
after 0.10.12 apply to every earlier prebuilt binary (see "Security —
dependency" below).

⚠️ **Scripts, CI jobs and hooks that run `evnx diff`:**

- **Against a template only key names are compared** (I-078, DEC-DF1). That
  means the default `.env.example`, `[defaults] example` or `--example`. A
  filled-in `.env` differs from its placeholders in every value,
  so `evnx diff` exited 1 on every real project. A `.env` with exactly the
  template's keys now exits 0 and says "have the same keys", not "Files are
  identical". `--values` compares values against a template as before.
  `--against NAME` compares two environments, values included, and is
  unchanged. `--format json` reports `"compared": "keys"` or `"values"`; with
  `keys`, `different` is empty. `--format patch` has no update section against
  a template (it replaced real values with placeholders), and `--interactive`
  asks no update questions.
- **`--fail-on missing,extra,different`** (I-078) chooses what exits 1. The
  default is all three; the report is the same either way. `--fail-on missing`
  fails only when `.env` lacks a key the template has. `different` against a
  template without `--values` is exit 2: it could never fail.
- **The `evnx-diff` pre-commit hook** runs `evnx diff --fail-on missing
  --skip-without-env` (D-39, DEC-DF3). Plain `evnx diff` blocked every commit on a
  working setup, and exited 2 in a fresh clone; the README said it "warns". It
  now blocks only when `.env` lacks a key `.env.example` declares, and skips a
  clone with no `.env`, saying so. `--skip-without-env` is new: with no `.env`
  it exits 0 and says it skipped; a missing template is still exit 2.
- **Flags that did nothing are exit 2** (I-075): `--with-stats` without
  `--format json`, `--interactive` without `--format patch`.
- **Without `--show-values` no value is shown, not even `***`** (DEC-DF2). A
  secret-named key printed `SECRET_KEY = ***`, and two `***`
  lines under a changed value, beside plain keys that printed their name only.
  In JSON its `example_value`/`env_value` are now `null` and `values_redacted`
  is `false` unless values were asked for.
- **JSON** gains `schema_version` (1), `compared`, `ignored` (the ignored keys
  that were in either file, I-077) and, per changed key, `value_state`:
  `withheld`, `masked` or `shown` (I-076b). `values_redacted` stays for one
  release; `value_state == "masked"` is the same test.

### diff

- The summary counts ignored keys (`1 key ignored`) (I-077). `-v` lists ignore
  names that matched no key; `--ignore-keys` is case-sensitive, and the help
  now says so and that it adds to `[diff] ignore_keys`.
- `-v` says which files were compared, with how many keys, whether values were
  compared and what fails the run (I-079). It printed "Running diff in verbose
  mode". `-q` also hides the new "values not compared" note.
- An error is no longer printed under a header announcing the comparison
  (I-080): both files are read before the header. With `--reverse` the header
  names the files in the order compared (`.env.example ↔ .env`).
- `--help` states what is compared, the exit codes and examples; `--format
  patch` says it is not `patch(1)` input, and `--interactive` that it writes
  nothing yet (D-44; I-073 is planned for 0.11.0).

### Shared

- `diff` takes its template from `Config::example_path()`, the rule `validate`,
  `sync` and `spec init` use (`--example`, `[defaults] example`,
  `.env.example`); the result is the same as its own copy.
- `core::verdict::skipped` prints the one-line skip notice for `sync` and
  `diff` (`sync`'s output is unchanged).
- `core::mask::disclose` withholds every value that was not asked for
  (`diff` is its only caller), and `core::mask::ValueState` names the three
  states a report prints.

### Security — dependency

- `rustls-webpki` 0.103.15 → 0.103.16 and `webpki-roots` 1.0.6 → 1.0.10
  (through `reqwest`, used by the cloud commands, `migrate --to github-actions`
  and `update`): RUSTSEC-2026-0382 (certificate verification could accept a
  distrusted intermediate) and RUSTSEC-2026-0381 (the bundled roots trusted the
  distrusted emSign intermediates). Both were published after 0.10.12 shipped and
  are in every prebuilt binary up to it, which are built with `--all-features`.

## [0.10.12] - 2026-10-20

The patch for `evnx sync` (package R-sync). `--check` gives one verdict whatever
the output format, looks both ways for a developer and one way for CI and the
pre-commit hook, and every run without a terminal finishes instead of failing on
a prompt.

⚠️ **Scripts, CI jobs and hooks that run `evnx sync`:**

- **`sync --check` without `--direction` is two-way** (DEC-S3, DEC-S17): a name
  only `.env.example` has — a stale entry, or a `.env` that has not caught up —
  is drift, exit 1. Naming a direction keeps the old one-way check, and that is
  the form for CI and hooks: `evnx sync --check --direction forward`. The
  shipped `evnx-sync` pre-commit hook now runs exactly that, so a teammate whose
  `.env` lacks a template name can still commit. `--format json` adds
  `template_only` and `summary.template_only` to two-way reports (I-119).
- **`--check --format json` is the same verdict as the pretty output** (I-117).
  It now includes the names `[vars]` requires in reverse, refuses a credential
  in `allow_actual` and a name `--naming-policy error` refuses (both exit 2, was
  1), and reports a missing `.env` in reverse as drift, exit 1 (was 2), listing
  every name.
- `--check` with an empty `.env` beside a populated template is **exit 2**, no
  verdict (I-127, DEC-S14); it passed with a warning. Plain `sync` still warns.
- **No terminal, or `CI` set** (to anything but empty, `0` or `false`): sync uses
  placeholder values on an existing template and in reverse too, and says so,
  instead of failing with "IO error: not a terminal" (I-124, DEC-S9, DEC-S18).
  Only `sync` reads `CI`. `--placeholder` is deprecated and does what `--force`
  does, with a notice.
- A misspelt key in a `--template-config` file (`allow_actaul`) and an invalid
  `[sync] naming_policy` in `.evnx.toml` are **exit 2**, naming the accepted
  values; both were ignored without a word (I-121, DEC-S11). The file's value is
  checked even when `--naming-policy` overrides it, as `[validate] fail_on` is.
- `--direction` has no default in the parser any more (it is still forward):
  `evnx::commands::sync::run` takes `Option<SyncDirection>`.

### sync

- Placeholders are chosen by whole words of the name (I-120, DEC-S10):
  `SUPPORT_EMAIL` got `8000`, `MONKEY_COUNT` `YOUR_KEY_HERE`, `BYPASS_CACHE` a
  password placeholder and `GHOST_MODE` `localhost`; now `YOUR_VALUE_HERE`.
  `DEBUG=false` stays `false` (it became `true`). A URL placeholder follows the
  value's scheme — `postgres`, `postgresql`, `mysql`, `mongodb`(`+srv`), `amqp`(`s`),
  `redis`, `rediss`, `http(s)` — where only `postgresql://`, `redis://` and
  `http(s)://` were recognised; `URI` and `DSN` names count as URLs. `add custom`
  writes the same placeholders.
- Reverse with no `.env`: the template's own "# Generated from .env" header is
  not copied into the new `.env` (I-129), and names the spec requires are added
  under "# Added by evnx sync" (I-275, DEC-S20) — they were left out, so the new
  file failed `validate`.
- Reverse lists a name that only `[vars]` requires as such, not as "in
  .env.example" (I-255).
- The naming policy also covers the names sync writes into a new
  `.env.example`; it skipped them, so `--naming-policy error` let a template
  with a lowercase name through on its first run.
- ⛔ **Creating `.env.example` with actual values no longer copies a refused
  credential** (I-277). The template was `.env`'s text rewritten line by line,
  so a multi-line value's continuation lines (a PEM private key's body) and a
  line written `export<TAB>KEY=…` were copied as they were — while the run
  said those keys were "kept out" and written as placeholders. The file is now
  built from the parsed values, as every other write to the template is; it no
  longer carries `.env`'s comments. Its advice to "re-create it with `evnx sync
  --force`" did nothing on the existing file; it now says to delete it first
  (I-278).
- With no `.env`, `--check` and `--skip-without-env` load `--template-config`
  too: a misspelt key or a missing file is exit 2 in CI as it is locally
  (I-279).
- A prompt needs stderr to be a terminal as well as stdin: `evnx sync 2>log`
  failed "IO error: not a terminal" (I-280). It uses placeholders and says so.
- `--check` and `--dry-run` say a missing file "would" be created rather than
  "creating" it (I-281). The empty-`.env` warning and the no-`.env` check name
  the files in the command they suggest (I-282). A new `.env` keeps a first
  line of the template's own that starts "# Generated from"; only the two
  lines sync writes are replaced; and choosing no names in reverse writes
  nothing (I-283).
- The `--check` verdict names the command that fixes each side, with
  `--env`/`--example` when they are not the defaults; reverse no longer says
  "commit the result" of writing a git-ignored `.env` (I-118).
- Messages name the files in play: `--env nope.env` said "File not found: .env",
  and "Created", "Updated", "Failed to parse" named `.env`/`.env.example`
  whatever was passed (I-125).
- "Added 1 variable", not "1 variables"; one warning for a readable `.env`,
  worded as everywhere else ("readable by other users (mode 644)" and the
  `chmod`), where reverse printed two, one in `0o644` form (I-126, I-276).
- An interactive comment is written above the variables it describes, in the
  same write; it was appended after them (I-126).
- The `--dry-run` preview names a symlinked file's target, as the write does.
- The "replace placeholders" reminder appears only when something added is one.

### Docs

- README: the sync section lists every flag, the two `--check` forms and what
  each is for (D-66, D-67); the `[sync]` config line says a bad value is an
  error and where placeholder rules go (D-71). Getting Started step 8 shows the
  template sync really writes (D-64). `--help` reworded for `--direction`,
  `--check`, `--force`, `--template-config` and `--naming-policy` (D-69).

### Shared code

- `core::placeholder`: the placeholder generator and `PlaceholderConfig`, moved
  from `commands::sync` so `add` no longer imports from another command
  (DEC-S19). `commands::sync::PlaceholderConfig` is re-exported. `init` and
  `validate --fix` keep their own placeholder shapes.
- `utils::term::can_prompt`: stdin and stderr are both terminals (I-280);
  `sync` uses it, the other prompts still check stdin only.
- `core::classify::has_word`, `core::envfile::only_in` (now also behind
  `diff`'s missing/extra lists, output unchanged), `utils::term::ci`,
  `utils::string::count` (moved from `add`), `core::permissions::warn_if_readable`
  and `readable_warning` (`safe_write::report` prints through it, unchanged;
  `report_link` is its symlink half), `NamingPolicy::from_config` on
  `core::args::one_of`.

## [0.10.11] - 2026-10-20

The patch for `evnx backup` and `evnx restore` (package R-backup-restore).
Neither can be pointed at a file it would destroy any more, a new backup needs
an 8-character passphrase from every source, a script can replace an existing
`.env` with `restore --yes`, and a backup that cannot be opened says why.

⚠️ **Scripts that run `evnx backup` or `evnx restore`:**

- **A key file or `EVNX_PASSWORD` under 8 characters is refused by `backup`**,
  exit 2, as the prompt always was (I-147, DEC-B2). `restore` takes any length,
  so a backup made earlier with a shorter one still opens.
- `backup --output` naming the source file — by path, symlink or hard link —
  is refused with exit 2: it turned the `.env` into ciphertext and left the
  plaintext only in `.env.1` (I-146). So is an output whose rotation would move
  the source (`backup out.1 --output out`), a directory at the output (exit 2,
  was 5) or in a rotation slot (DEC-B3). `restore --output` naming the backup
  itself is refused too.
- **`restore --yes`** (alias `--force`, `-y`) replaces an existing output
  without asking, for scripts (I-148, DEC-B1). Without it and without a terminal
  the run stops with exit 2 and a line naming `--yes` and `--output`, instead of
  "IO error: not a terminal". Whichever way the replacement is confirmed, the
  replaced file is kept as `<output>.bak` (`.bak.2`, … when one exists), with its
  mode (DEC-B11). An output that already holds exactly the backup's content is
  left alone: no question, no write, exit 0, "already up to date" (DEC-B12).
- `restore cloud://…` is an ordinary missing file, **exit 3**, with a line
  pointing at `evnx cloud pull`; it was exit 6, "evnx-cloud restore is not yet
  available in this build" (DEC-B4). Exit 6 is retired for `restore`.
- A backup that cannot be opened says why: not an evnx backup, truncated, or
  made by a newer evnx ("Upgrade evnx to restore it"). All of them read "Wrong
  password or corrupt backup" before; a wrong password and a tampered file still
  do, deliberately (I-149, DEC-B6). Still exit 2.
- With both `--key-file` and `EVNX_PASSWORD` set, a warning says the key file
  won and the variable was not used (I-155).
- `restore`'s key-file line is `backup`'s: "Using key file for decryption
  (non-interactive mode)", no longer a "less secure" warning; `EVNX_PASSWORD`
  keeps its warning. With no key file, no variable and no terminal, both
  commands name `--key-file` and `EVNX_PASSWORD` instead of "IO error: not a
  terminal".

### backup

- Rotating through a symlinked `--output` rotates the file it points to; it
  renamed the link itself to `.1`, so the output's name stopped existing.
- `--help` lists the exit codes `backup` produces: no 1, and 2 is "could not
  run", not only "source not found" (DEC-B9, D-101). `--verify` costs "a fraction
  of a second to about a second", not "~1 s" (D-103); `--keep` says which file
  the shift replaces (D-104).

### restore

- The `.restored` fallback no longer prints "✓ Restored 0 variable(s)" beside
  exit 5; a warning says where the content went (I-154, DEC-B5).
- In a git repository the restored file gets the `.gitignore` step `template`
  and `convert -o` have: asked with a terminal, a warning without one,
  `--gitignore` to add it, `--no-gitignore` to skip.
- `--help` describes `<BACKUP>` and `--output`, the overwrite rule and the exit
  codes, and says a backup file is not a vault (D-108, D-109, D-112, D-113).

### Docs

- README: the backup/restore section, which was two lines — the passphrase
  sources, rotation, replacing a file, a CI recipe, the exit codes, and a table
  of `backup`/`restore` against `cloud push`/`pull` (D-111, D-113). The module
  headers' `--env` example and exit-1 rows are gone (D-107, D-110).

### Shared code

- `utils::secret_source`: the passphrase from a key file, `EVNX_PASSWORD` or a
  prompt, with the floor, the notices and the no-terminal message, for `backup`
  and `restore` (DEC-B13). `read_key_file` moved here from `backup`. The cloud
  master-password prompts are not on it yet (B-WP3, 0.11.0).
- `zeroize::Zeroizing<String>` replaces the two private `ZeroizeOnDrop` guards;
  the restored plaintext is zeroized on the cancel and up-to-date paths too.
- One `KdfParams::V1` for encrypt and decrypt, where the Argon2id numbers were
  written twice. The 0.10.0 and 0.10.2 golden backups still open, and a 0.10.10
  backup made with a 3-character key joins them as a fixture.
- `core::env_name`: copies are not environments. `--env-name`'s "Available"
  list showed `.env.bak` (from `validate --fix`), `.env.1` and `.env.restored`
  as `bak`, `1`, `restored`, and `validate` counted them as known environments
  (I-272).

## [0.10.10] - 2026-10-20

The patch for `evnx migrate` (package R-migrate). Every destination was checked
against its tool's latest release on 2026-10-20: two printed commands their tool
rejects, gcp printed no values, and the GitHub upload replaced secrets that
`--skip-existing` promised to keep. Names that a rename or a case-blind
platform turns into one are refused, `--print-only` saves a runnable script, and
`--env-name` reads the environment every other command reads.

⚠️ **Scripts that run `evnx migrate`:**

- **`--to github-actions --skip-existing` (and `--yes` without `--overwrite`)
  replaced existing secrets** past the first 30 in GitHub's list, or spelled in
  another case (I-266). Every page is read now and names are compared as GitHub
  stores them, uppercase, so those secrets are skipped, or refused under `--yes`.
- `--to vercel --vercel-project P` printed `--project-id P`, which vercel does not
  have; it prints `--project P --force --yes` (I-268). `--force` replaces a
  variable that exists, as every other destination's command does.
- `--to railway` printed `railway --project P variables set …`, which does not
  parse; it prints `railway variable set --project P --environment E …`.
  `--railway-project` now needs the new `--railway-environment`, as Railway does;
  `--railway-service` is optional (I-269, DEC-MG1).
- `--to gcp-secret-manager` needs `--project`, as `convert --to gcp-secrets` does,
  and prints the value: `printf '%s' 'VALUE' | upsert_secret NAME`, the script
  convert prints. It printed `echo -n "${NAME}"`, which sent whatever your shell
  held under that name — usually nothing, so an empty secret — created only, so a
  second run failed, and dropped `--project` (I-100, DEC-MG2). Names stay as in
  the `.env`.
- `--to infisical` refuses a value that is empty or starts with `@`: the CLI
  rejects the first, and reads the second as a **file path** when signed in with
  `infisical login` (I-270, DEC-MG4).
- Two names that become one are exit 2, naming both: `--strip-prefix API_` on
  `API_X` and `X` printed one `X` and lost a value, and Azure Key Vault and GitHub
  ignore case, so `A_B` and `a_b` were one secret there. `--allow-collisions`
  keeps the last value (I-104, DEC-PL2).
- `--add-prefix` and `--strip-prefix` must keep names variable names (letters,
  digits, `_`, not starting with a digit): `--add-prefix 'X;touch PWNED;'` ran
  `touch` through the gcp commands, and a strip could leave an empty name
  (I-267). Names GitHub reserves (`GITHUB_…`) and values over 48 KB are refused
  before anything is uploaded.
- With `[defaults] env_name` in `.evnx.toml`, migrate reads `.env.<name>`, like
  every other command; it read `.env` (I-031). `--source-file` still wins.
- doppler, infisical and railway no longer need their CLI installed to print the
  commands (I-110).
- The load line names the file it read (`Loaded 3 secrets from .env.staging`),
  not `env-file`.

### migrate

- **`--print-only`** (I-111): stdout holds only the commands, so
  `evnx migrate … --print-only > upload.sh` is a script `sh` runs; the header,
  status lines and summary go to stderr. Without the flag the output is where it
  was. Not for github-actions, which prints no commands, nor with `--dry-run`.
- **`--env-name NAME`** reads `.env.NAME` (S-M1); a name with no file is an error
  listing the environments there are. Not with `--from environment`.
- `-q` also hides each destination's own lines ("Requires Heroku CLI",
  "Install: …", "Run this command…"), leaving the commands and the summary
  (I-105). `--from`'s help says what happens with no terminal: `env-file`.
- `--github-token` on the command line gets a warning: it is in your shell history
  and in `ps`. `GITHUB_TOKEN` is read when the flag is absent, without one (I-108).
- The GitHub upload uses evnx's one HTTP client: a 30 s timeout per request
  (there was none), user agent `evnx/<version>`, `X-GitHub-Api-Version:
  2022-11-28`, and one second between uploads, as GitHub asks for writes; at
  100 ms, more than 80 secrets ran into its secondary rate limit (I-112,
  DEC-MG3).
- `--help` says what migrate does — prints commands for eight destinations,
  uploads to one — with examples laid out as written, the flag each destination
  needs, and the exit codes (D-59, D-60). The "use `evnx convert`" gcp tip is
  gone: migrate prints that script itself.

### Docs

- README: what each destination prints or does and needs, `--print-only`,
  `--env-name`, collisions, the flags that only apply to github-actions, exit
  codes; `GITHUB_TOKEN` rather than `--github-token $GITHUB_TOKEN`; the feature
  table as `Cargo.toml` has it; migrate in the `--env-name` list; "migrate runs
  entirely offline" corrected for github-actions (D-54..D-58, D-61, D-63). The
  README's one-line migrate examples run in a test.
- GETTING_STARTED: a migrate step (D-62).

### Shared code

- `core::converter`: `ConvertOptions::prepare` takes any `Naming` — a
  converter, or a migrate destination — and gains `strip_prefix`,
  `prepare_with` (the refusal says what was not done) and `announce_kept`
  (convert's `--allow-collisions` warning). migrate's `filtering::apply_filters`,
  the third filter → strip → add pipeline and the only one with no collision
  check, is now a wrapper over it. convert's output is unchanged.
- `formats::gcp::{upsert_function, upsert_line}` and `formats::azure::secret_name`:
  one gcp script and one Key Vault name rule for convert and migrate.
- `core::http::client` for the GitHub upload; the architecture guard now flags
  reqwest's `Client::new()` as well as `Client::builder()`.

## [0.10.9] - 2026-10-20

The patch for `evnx convert` (package R-convert): the names a target needs are
flags instead of placeholders, the gcp script updates as well as creates, a
format's own renaming can no longer merge two variables in silence, and
filters use the same globs as `migrate`. And, in every command, a yes/no
question shows the key you press before Enter, and one Enter answers one
question (I-264, I-265).

⚠️ **Scripts that run `evnx convert`:**

- `gcp-secrets`, `azure-keyvault`, `heroku` and `kubernetes` need the place
  they write to: `--project`, `--vault-name`, `--heroku-app`, `--name`
  (`--namespace` is optional). Without it they exit 2 and print nothing; they
  printed `my-project`, `my-keyvault`, `YOUR_APP_NAME` and `app-secrets` to be
  edited by hand (I-084, DEC-CV1). A name flag for another format is exit 2 too.
- A filter that leaves nothing, or an empty `.env`, is exit 2 with stdout empty;
  it printed `{}` and exited 0. `--allow-empty` keeps the old output (I-089,
  DEC-CV3).
- Two names that become one name in the output are exit 2 for every format,
  not only after `--transform`: gcp-secrets lowercases and hyphenates and
  terraform lowercases, so `A_B` and `a_b` were two `gcloud secrets create a-b`
  or two `a_b =` lines; Azure Key Vault and GitHub ignore case, so they were one
  secret. `--allow-collisions` keeps the last value (I-249, DEC-PL2).
- `--include` and `--exclude` are `migrate`'s globs: `?` is one character, `*`
  works in the middle (`DB_*_URL`), and a comma list or a repeated flag gives
  several patterns. `P?RT` and `PORT,DEBUG` matched nothing before (I-090).
- An unknown `--transform` is exit 2 listing the modes; it was ignored (I-090).
- `--prefix` must be letters, digits and `_`, not starting with a digit (I-263).
- Errors no longer start with `Convert command failed:` (I-091).

⚠️ **Scripts that answer evnx's yes/no questions through a pty or `expect`:**
every question now waits for Enter after `y` or `n`. Twelve of them (in `sync`,
`restore`, `cloud`, `auth` and `vault`) took the first key alone (I-265).

### convert

- **The gcp script creates a secret or adds a version** (I-099). It ran
  `gcloud secrets create` alone, which fails for a secret that exists, so
  running the script again to update a value failed for every secret. Each
  value is still printed once and reaches gcloud on stdin.
- **`--prefix` reached the generated scripts unquoted** (I-263): `--prefix
  'X;touch PWNED;'` ran `touch` when the azure, heroku, shell or gcp script
  ran. The prefix is checked, and every name and flag value in a script goes
  through `core::shell::word`.
- No terminal and no `--to`: one error naming `--to` and the formats, in
  `migrate`'s words. It said "Format selection cancelled by user: IO error: not
  a terminal: not a terminal" under a header for a menu that never opened
  (I-083).
- A missing `--env` file gives validate's and diff's error, `.env does not
  exist` and how to create it, instead of three nested contexts (I-091).
- `--help` lost two developer notes it printed ("clap strips the markdown
  fence…", "This said … exactly backwards"), says 14 formats rather than 14+,
  names the flag each target needs, and lists the exit codes (I-082, D-47,
  D-49, D-50).
- The selector's labels say what each format prints: `aws-secrets` is JSON for
  `create-secret --secret-string` (it said "CLI commands"), `kubernetes` is
  `stringData` unless `--base64` (it said "base64 encoded"), `github-actions`
  is text for the web form (I-085, I-087).
- `doppler`, `vercel` and `railway` are marked experimental in `--help`, the
  selector and the docs, and say so on stderr when chosen: Doppler documents
  flat JSON for `doppler secrets upload` (what `--to json` prints), Vercel's
  API takes `{key, value, type, target}` objects, and Railway documents no
  import for this JSON (I-087, I-097, DEC-CV4). Their output is unchanged.
- `--output` in a git repository checks `.gitignore` for the file, as `template
  --output` does: a question with a terminal, a warning without one,
  `--gitignore` / `--no-gitignore` to decide without asking. The refusal to
  write over the input uses template's wording.
- `kubernetes` gains `--namespace`, written as `metadata.namespace`.

### migrate

- The gcp destination's tip names `--project`: it suggested `evnx convert --to
  gcp-secrets > upload.sh`, which now exits 2 without it.
- The "--to is required when there is no terminal" error is unchanged; it comes
  from the helper `convert` now shares.

### cloud export

- `--to gcp-secrets` writes the create-or-add-version script too (it uses the
  same converter). Placeholders (`my-project`, …) and unchecked collisions stay
  as they were for `cloud export`; whether it should require names and refuse
  collisions is left to its own patch (R-org-vault-cloud).

### Every command: yes/no questions

- **The key you press shows before Enter** (I-264). At "Generate .env files
  with these variables? [Y/n]", pressing `n` printed nothing: the cursor was
  hidden, the key was not echoed, and only the hint's capitals changed
  (`[Y/n]` ↔ `[y/N]`; under a No default, `n` changed nothing), so a keypress
  looked ignored until Enter. Now the line reads `[Y/n] n`, Backspace takes
  the answer back, and Enter answers; Enter alone still takes the default and
  Esc then Enter still cancels. The line left behind is unchanged
  (`… ? no`). Reported on `init`; every yes/no question in evnx asks this way.
- **One Enter answers one question** (I-265). Twelve questions — `sync`
  (two), `restore`, `cloud pull` (overwrite a file), `cloud delete-version`,
  `auth token revoke`, `auth sessions revoke` and `revoke-others`,
  `auth rotate-master-password`, `vault delete`, `vault revoke` and
  `vault rekey` — were answered on the first key, so the
  Enter typed after `y` answered the next question too: in `sync`, "Add a
  comment explaining these variables?" took its default and opened the
  Comment prompt. They now wait for Enter like init, add and the rest (N-09).

### Docs

- README and GETTING_STARTED: every target with the flag it needs, which
  targets are experimental, the glob rules, `--allow-empty`,
  `--allow-collisions`, `.gitignore` for `--output`, and that the AWS pipe
  example needs Linux or macOS and that evnx does not run `aws` (D-45, D-46,
  D-47, D-48, D-53). The README's one-line examples run in a test.

### Shared code

One definition each, where commands had their own:

- `ConvertOptions::prepare` (A-12): the one pipeline every format renders from
  — filter, prefix and transform, the format's own name rule
  (`Converter::target_name`, `names_ignore_case`), the collision rule
  (`Collisions`), base64. The 14 formats each repeated it inline.
- `core::glob` for convert's filters (A-13). Its own `glob_match` (`*` only at
  the ends, arguments reversed) is removed; `tests/architecture.rs` keeps one.
- `formats::FORMATS`: one table of the 14 formats (names, aliases, menu label,
  extension, experimental note) behind `get_converter`, `format_help`,
  `extension_for` and the selector; `formats::converter_for` and `Targets` for
  the name flags.
- `core::glob::nothing_left`: the "left nothing to …" wording, for `cloud run`
  (unchanged) and `convert`.
- `utils::prompt::choice_needed`: a choice with a flag and no terminal, for
  `migrate --to` (unchanged) and `convert --to`.
- `Parser::MISSING_ENV_HINT`: the missing `--env` hint, for validate and diff
  (unchanged) and convert.
- `core::safe_write::refuse_same_file`: `--output` naming an input, for
  template (unchanged) and convert. `GitignoreMode::from_flags` for both.
- `core::args::Name` gains `GcpProject`, `K8sName`, `K8sNamespace`,
  `VarPrefix`.

- `utils::prompt::yes_no` is evnx's own question (on `console`, which
  dialoguer already uses) rather than dialoguer's `Confirm`; every yes/no in
  evnx goes through it, and `tests/architecture.rs` keeps it that way.
  `utils::ui::prompt_yes_no` (deprecated in 0.10.5, no callers) is removed.

### Library

- `ConvertOptions` has `include` / `exclude` lists and `collisions` in place of
  `include_pattern` / `exclude_pattern`; `check_collisions` is replaced by
  `prepare` and `collisions_in`. `KeyTransform::parse` is new.
- `ConvertConfig` gains `targets`, `allow_empty`, `gitignore`; its builder
  takes `include` / `exclude` / `targets` / `allow_empty` /
  `allow_collisions` / `gitignore`. `ConvertConfig::validate_input` is removed
  (the parser reports a missing file).
- `KubernetesSecretConverter` gains `namespace`. `GitignoreMode` derives
  `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`.

## [0.10.8] - 2026-10-18

The patch for `evnx validate` (package R-validate): counts that count what they
say, one finding per problem, a `--fix` that tells a repair from a placeholder,
and the checks the review asked for — environment names, Docker files beside
the env file, and `.gitignore`.

⚠️ **Scripts that read validate's exit code or JSON:**

- In a git repository, an env file **already tracked by git** is an error
  (exit 1); one that `.gitignore` does not cover is a warning (exit 0). Outside
  a repository nothing changes (D-27, DEC-VA1).
- An unknown issue type in `--ignore` or `[validate] ignore` is exit 2, listing
  the valid types (I-038). It was accepted and ignored nothing.
- JSON: `fixed` and `summary.fixed_count` hold only completed repairs; the
  variables `--fix` could only add with a placeholder are in the new
  `needs_input` array and `summary.needs_input`, and keep their
  `placeholder_value` finding in `issues` (I-037). `dry_run` is new.
  `required_present` counts the declared variables that are present (I-034).
- `SECRET_KEY=changeme` is one finding (the placeholder), not also
  `weak_secret`, so error counts can drop (I-035).
- `--fix` no longer generates a value for a key a provider issues
  (`STRIPE_KEY`, `AWS_SECRET_ACCESS_KEY`, `VERCEL_TOKEN`, …), even with
  `--rotate-weak-secrets`; those findings are `auto_fixable: false` (I-260).

### validate

- **`required_present` counts declared variables that are there** (I-034). It
  was `min(variables in .env, variables in .env.example)`: two of eight missing
  plus one extra read 7 of 8, and a file of extras alone read as complete.
- **One problem, one finding** (I-035). A placeholder in a secret-shaped name
  is reported as a placeholder only; "too weak or predictable" is for a real
  value. `--ignore placeholder_value` still leaves the weak-secret finding.
- **`--fix` separates "Fixed" from "Needs your input"** (I-037). A variable it
  could only add with a placeholder is listed once, under "Needs your input",
  not counted as fixed and not repeated as a fresh error; the run exits 1 until
  it is filled in. A missing variable gets the template's value (`8080`,
  `localhost`, or empty for `OPTIONAL_X=`), and the generic placeholder only
  when its name looks like a secret (D-25). It was `your_value_here` for every
  name.
- **`--fix --dry-run`** shows what `--fix` would change and writes nothing — no
  `.env` change, no backup. The report and exit code are the file's as it
  stands; a secret it would generate is not shown (S1).
- **`--fail-on warning|error`**, and `[validate] fail_on` in `.evnx.toml`:
  `warning` makes warnings exit 1. The flag wins; `--exit-zero` still wins over
  both (S2).
- **An unknown `--ignore` type is an error** (I-038), and `--format` takes any
  case, as `scan` and `doctor` already did (`--format JSON`).
- **With a contract, `--strict` measures against `[vars]`** (I-039), as its help
  always said: the message names `[vars]` and suggests declaring the name, not
  adding it to `.env.example`. A variable whose `environments` exclude this
  file is reported too — "declared for production only, but set in
  .env.staging" (I-043).
- **`environments` names are checked** (I-042): a name that matches no
  `.env.<name>` file and no common name warns, with the one it was probably
  meant to be (`prodution` → `production`), and so does a file named close to a
  listed one (`.env.prod` against `"production"`). Exit code unchanged.
- **Docker files are looked for beside the env file**, then in each folder up to
  the repository root, then the current directory (I-046), and
  `compose.yaml` / `compose.yml` count (I-257). It looked only in the current
  directory, and only for `docker-compose.*`, `Dockerfile` and `Containerfile`.
- **`.gitignore`** (D-27, DEC-VA1): new findings `env_not_ignored` (warning)
  and `env_tracked` (error), from the same git check `doctor` uses.
- An empty template value that is missing from `.env` says how to make it
  optional: `evnx spec init`, then `required = false` (I-036, D-29).
- `--fix` writes `.env` atomically through the shared writer: the file keeps its
  mode, a symlinked `.env` is written through, and a file others can read gets
  the same `chmod` warning every other command gives. The backup is written at
  the original's mode from the start.
- `DEBUG=True`'s advice says "Use true or 1"; it said "true or 0" (I-261).
- **`--fix` invents only what nothing else holds** (I-260, DEC-VA4..VA6). It
  generated 64 random hex characters for every secret-shaped name holding a
  placeholder, so `STRIPE_KEY=changeme` became a fake Stripe key that then
  passed validate. It now generates only for the app's own secrets — an exact
  list: `SECRET_KEY`, `DJANGO_SECRET_KEY`, `FLASK_SECRET_KEY`,
  `SECRET_KEY_BASE`, `JWT_SECRET`, `NEXTAUTH_SECRET`, `AUTH_SECRET`,
  `BETTER_AUTH_SECRET`, `SESSION_SECRET`, `COOKIE_SECRET`, `APP_SECRET` — and
  for passwords (`*_PASSWORD`, `*_PASSWD`, `*_PWD`). Any other key keeps its
  placeholder and is reported, with "get it from where it is issued";
  `--rotate-weak-secrets` follows the same rule. A missing secret of that kind
  is generated in one run instead of a placeholder and a second `--fix`.

### add

- **`evnx add custom NAME --optional`** (I-036, DEC-VA2). With a `[vars]`
  contract it writes `required = false` — what `add custom` declares by
  default; the flag says so — and an `(optional)` comment in `.env.example`.
  Without a contract the variable is still added, and the run says how to make
  it optional. It conflicts with `--required`.

### doctor

- `docker_config` recognises `compose.yaml` and `compose.yml` (I-257,
  DEC-VA3).

### Docs

- README and GETTING_STARTED: `--strict` does **not** make warnings exit 1 —
  both said it did; `--fail-on warning` does. The GETTING_STARTED sample output
  is real output; the one it replaced was a format evnx never printed. Both
  list `--fix --dry-run` and `[validate] fail_on`.

### Shared code

One definition each, where commands had their own:

- `core::args::one_of`: a value from a fixed list, any case, or an error listing
  them — validate's `--format`, `--ignore` and `--fail-on`, scan's `--format`
  and `--severity`, doctor's `--format`. scan's and doctor's messages are
  unchanged.
- `core::project`: the Docker file list and the upward walk to the repository
  root, for validate and doctor (DEC-VA3).
- `core::safe_write::backup`: the numbered `.bak` that never overwrites an
  earlier one, at the original's mode. `utils::fs::backup_file` (never called;
  it wrote `.backup`) is removed.
- `core::classify::may_generate` and `APP_SECRETS`: which secrets evnx may
  generate (I-260), beside the name and placeholder rules it already holds.
- `core::env_name::abbreviation_of` and `meant`, and
  `core::spec::environment_problems`, for the environment-name check; the
  `--env-name` error's "did you mean" uses the same abbreviation rule as
  before.

### Library

- `commands::validate::run` takes `validate::Options`. `IssueType` gains
  `UnknownEnvironment`, `EnvNotIgnored`, `EnvTracked` and `IssueType::ALL`;
  `ValidationResult` gains `dry_run` and `needs_input`, `Summary` gains
  `needs_input`, `core::config::ValidatePolicy` gains `fail_on`.
  `validate::fixer::write_fixed_file` returns the backup path and the
  `safe_write::Written`; `fixer::fill_for_missing` and `FixAction::AddSecret`
  are new.

## [0.10.7] - 2026-10-17

The patch for the three commands about evnx itself — `commands`, `completions`
and `update` (package R-surface) — plus one HTTP client and one cargo command
where there were several.

⚠️ **Scripts that read these exit codes or this output:**

- `evnx update --check` exits **1** when a newer release exists (it was 0 either
  way) and 2 when it could not check. It gains `--json`.
- `evnx completions` takes its shell from a fixed list, so an unknown one is
  clap's usage error (exit 2, with the list) instead of evnx's "unsupported
  shell" line and the "No verdict" banner.
- `evnx update -q` prints only the upgrade command, and `update --check -q` the
  status line and the command. `--check`'s "asking api.github.com…" line is on
  stderr.

### commands (alias `surface`)

- **The closing line counts what it lists** (I-196):
  `20 commands (83 with sub-commands).` It said `83 commands` under a list of 20.
- **The warning for a build without `cloud` names no release** (I-197). It said
  `pip install evnx` lacked the cloud commands "up to and including 0.9.0",
  which is history and would be wrong on any later such build. It points at
  `evnx update` for the channels, and at `cargo install evnx --all-features`.
- `completions` and `update` have a guide entry, so `--json` fills their
  `docs_url` and their `--help` ends with the guide (I-195). Every top-level
  command has one.
- `surface` is the command's name before 0.10.0 and still works as an alias
  (D-138: no entry said so). `-q` has no effect, as before, and `--help` now
  says so (DEC-SF5).

### completions

- **`evnx completions bash | head -1` exits 141**, not 101 (I-185).
  clap_complete panicked on the closed pipe with a message the broken-pipe
  handler did not match.
- **Five shells, in any case: `bash`, `zsh`, `fish`, `powershell`, and `elvish`,
  which is new** (I-194, DEC-SF6). `--help` lists them, says where each shell
  reads the script, and links the guide. The scripts complete the shell names
  after `evnx completions` (they offered `<SHELL>`).

### update

- **The cargo upgrade keeps every feature** (I-199):
  `cargo install evnx --all-features --force`. It was `--features cloud
  --force`, which rebuilt an `--all-features` install without `migrate`,
  `backup` and `restore`.
- **A pip install is recognised by what pip leaves** (I-193): a virtual
  environment of any name (`pyvenv.cfg` beside `bin/` or `Scripts\`), or an
  `evnx-*.dist-info` record under the prefix's `site-packages`. `pip install
  --user` and system pip put the binary in `~/.local/bin` and `/usr/local/bin`,
  which read as the install script, so pip users were told to run the curl
  installer. With no such evidence, those two folders get both commands.
  Checked against the published 0.10.5 wheel.
- **`--check` answers with its exit code** (I-198, DEC-SF4): 0 for the latest
  release or newer, 1 when a newer one exists, 2 when it could not check.
  `--check --json` prints `schema` (1), `installed`, `latest`, `status`,
  `channel`, `command` and `release_url`. A release with no page link still
  answers.
- `--help` names the seven channels it detects — it said "nine install
  channels" and named none (D-140) — and lists the exit codes.
- The `--check` message for a build with no network support names
  `cargo install evnx --all-features` (it said `--features cloud`).
- Debug builds read `EVNX_UPDATE_URL`, for the tests; a release binary ignores
  it and always asks api.github.com.

### Every command: `--help` of a build without every feature

- The note at the end of `evnx --help` in a `cargo install evnx` build names
  what each feature adds (with `org`, which it left out) and gives one cargo
  line, `--all-features`, instead of one per feature.

### Release checks

- **`scripts/check-build-features.sh <evnx>`** reads the feature list from
  `evnx commands --json` and exits 1 naming what is missing (I-200). The PyPI
  wheel check uses it instead of grepping `--help` for four command names, which
  could not see `migrate` or `backup`; the npm check runs it on the global
  install; `docs/RELEASING.md` uses it for every channel.

### npm

- **A slow registry no longer fails a correct npm release, and re-running a
  failed one finishes it** (I-256). v0.10.6 uploaded all five platform packages,
  then gave up after 300 s on one that npm showed 8.5 minutes after its publish;
  the re-run stopped on that package, which npm was still processing, and the
  wrapper step could not be re-run at all. Both publishes now go through
  `scripts/npm-publish-packages.sh`: a version already in the registry is
  skipped, an authentication failure (npm's `404 Not Found - PUT` included)
  stops at once and names `NPM_TOKEN`, and any other failure waits for the
  version to appear. Each phase shares one deadline — 20 minutes for the five
  platform packages, 20 for the wrapper, 10 for `latest` — through
  `npm-await.sh`'s new `NPM_AWAIT_DEADLINE`, and the job stops after an hour.

### Docs

- README: sections for `evnx commands`, `evnx completions` and `evnx update`;
  *Verify* runs `evnx commands` (D-137, D-142); the cargo line for cloud support
  is `--all-features`, the one `update` prints (D-141).

### Shared code

One definition each, where commands had their own:

- `core::build`: the feature list (`commands`), and `CARGO_INSTALL` and
  `CARGO_UPGRADE`, the cargo command every message names. Five copies carried
  three flag sets; a test now fails on any other `cargo install evnx --…`.
- `core::http` (with `net`): the HTTP client builder and `transport_reason`, for
  `update --check` and every cloud command, whose copies differed only in the
  timeout they named. The private-CA fix (I-192, DEC-SF1, still open) will land
  there once.
- `utils::term::write_stdout()`: writes an answer and exits 141 on a closed
  pipe, for output that does not pass through `println!`.
- Tests: one `commands --json` reader (three files had their own), a
  closed-stdout runner, and the binary placed in a fake install location.

### Library

- `Commands::Completions { shell }` is a `clap_complete::Shell`, not a `String`.
- `commands::update::Channel::Script` is `Channel::ScriptOrPip`;
  `Channel::candidates()`, `Channel::id()` and `Channel::from_install()` are
  new. `commands::update::run` takes `json` and returns a `Verdict`.

## [0.10.6] - 2026-10-17

The `spec` patch: the review fixes left after 0.10.3 (package R-spec), the
`.evnx.toml` locator and writer that `spec`, `add` and `cloud link` each had a
copy of, and the npm install forms that never worked (I-253).

⚠️ **Some runs that passed on 0.10.5 now stop with exit 2**, and write nothing:

- any command, in a project with a bare `evnx.toml` (no leading dot) and no
  `.evnx.toml` beside it. That file was never read; now it says so. Rename it:
  `mv evnx.toml .evnx.toml`.
- `spec init` given `--env FILE`, `--example FILE` or `[defaults] example`
  naming a file that does not exist (it was read as empty), `--with` together
  with `--env`, `--example` or `--env-name` (they were ignored), or with
  nothing to write, `--with ""` (it was exit 0).

And `validate` exits 1 on an unknown key under `[vars.NAME]`.

### spec

#### Fixed

- **A generated contract passes the project it came from** (I-180). A `format`
  is written only when the sample value passes it; `PORT=0`, `REDIS_PORT=70000`,
  `DEBUG=2` and a 20-digit id each got one before, and the next `validate`
  failed four times. Each format left out is named on stderr, without the value.
  `--with django` no longer declares `DATABASE_URL` a `url`: the catalogue's
  sample is `sqlite:///db.sqlite3`, which has no host.
- **It writes the project's one `.evnx.toml`** (I-177). From a subfolder it
  loaded `../.evnx.toml` and then wrote a new `./.evnx.toml` that shadowed it.
  It now writes the config in force — at or above the folder — or, with none,
  creates one at the git root; the success line names it (`../.evnx.toml`).
  The `[vars]` guard reads that file too.
- **`--env-name` and `[defaults]`** (I-189): `--env-name staging` reads
  `.env.staging`, and `[defaults] env_name` and `[defaults] example` apply, as
  they do for `validate` and `sync`.
- **A file named by a flag must exist** (I-182): `--env`, `--example` and
  `[defaults] example` naming a missing file are exit 2. The default `.env` and
  `.env.example` may still be absent. `--with` with `--env`, `--example` or
  `--env-name` is refused, since `--with` reads no project file.
- **Nothing to write is exit 2** (I-186), so `spec init --with "$X" && git add
  .evnx.toml` does not carry on with no file.
- **The template's order, and names only in `.env`** (I-183). Entries follow
  `.env.example` instead of the alphabet; a name only in the environment file
  follows them as `required = false`, marked `# only in .env` (or the file's
  name). It used to be dropped without a word.
- **A URL with a password is `secret = true`** (I-184): `DATABASE_URL=postgres://u:p@db/app`.
- **`-q`** prints nothing on success (fixed in 0.10.3; the demo now checks it).

#### Changed

- **`secret` from a name follows `validate`'s rule** (I-190): `*_PWD` is now
  declared secret; `SENTRY_DSN`, `*_PRIVATE` and bare `TOKEN` no longer are.
  One rule, so the contract declares secret what `validate` demands a strong
  value for.
- **`format = "bool"` accepts `on`, `off`, `y` and `n`** as well, in any case —
  the one list template's `|bool` uses. A contract that checked these values
  now passes them.
- **A symlinked `.evnx.toml` is written through** to its target, which stays a
  link, with a `wrote through` notice — as `sync`, `backup` and `template` do.
- `spec init --help` lists the exit codes, and `--stdout` says only TOML goes
  to stdout.

### Every command: reading `.evnx.toml`

- **A key evnx does not know under `[vars.NAME]` is named** (I-187):
  `requird = false` left the variable required and `secrett = true` left it
  undeclared, silently. Every command now warns, with the key meant
  ("did you mean `required`?"), and `validate` reports it as an error,
  type `unknown_spec_key`, so it exits 1.
- **A bare `evnx.toml` is not ignored silently** (I-040): with no `.evnx.toml`
  beside it, exit 2 and the rename to make; beside one, a warning.

### add

- New variables are recorded with `spec init`'s inference: a format the
  catalogue's example value fails is left out, and a URL with a password, or
  a `*_PWD` name, is declared secret.
- An inline `vars = { … }` table is reported as one that cannot be extended,
  instead of as "no [vars] section".
- The config line names the file from where you are (`../.evnx.toml` from a
  subfolder), and a symlinked config is written through with a notice.

### cloud

- **`cloud link` from a subfolder with no config creates `.evnx.toml` at the
  git root**, where `spec init` writes, not in the subfolder. With a config at
  or above the folder, nothing changes.

### npm

- **`npx @evnx/cli` and `npm install --save-dev @evnx/cli` give an `evnx`
  command** (I-253). Only `npm install -g` worked, on every release since
  0.4.0: the five platform packages declared the `evnx` command too, and with
  two claims npm linked neither. The wrapper alone declares it now, and the
  release workflow checks all three install forms.

### Shared code

One definition each, where commands had their own:

- `core::config::project_file()`: which `.evnx.toml` a command records into —
  `spec init`, the `[vars]` writeback (`add`) and `cloud link`.
- `core::config::save()`: the one writer for it. It refuses text that does not
  parse and writes through `core::safe_write` (atomic, mode kept). All three
  wrote with `std::fs::write`, and only `spec init` checked the result parsed.
- `core::config::has_vars_section()`: one answer, from the parsed file
  (two textual copies, which missed an inline `vars = { … }`).
- `Config::example_path()`: the template path from `--example` and
  `[defaults] example`, for `validate`, `sync` and `spec init`.
- `spec::infer::guess()` and one `[vars.NAME]` renderer for `spec init` and the
  writeback; spec's secret-name list and bool words are gone for
  `core::classify` and `core::value`.
- `ui::short_path()` is public: a path as the person would type it from here.

## [0.10.5] - 2026-10-17

The `template` patch: the review fixes left after 0.10.3 (package R-template),
and code that several commands had their own copy of, moved into one place.

⚠️ **A template run that passed on 0.10.4 can now stop with exit 2**, and
write nothing, in three cases that used to put the wrong text into the
output silently: a filter evnx does not know (`{{PORT|innt}}`, or another
tool's `{{ item | quote }}`), two filters chained (`{{X|upper|json}}`), and
`|bool` on a value that is not one of `true yes on 1 y` / `false no off 0 n`
(`enabled`, `maybe`, empty). The error names each placeholder and its line.
Check a template before deploying with `evnx template … --dry-run`.

### template

#### Fixed

- **An unknown filter is an error.** `{{PORT|innt}}`, `{{ X | Upper }}` and
  `{{X|bogus}}` were written into the output unchanged with exit 0. Now the run
  stops before writing, names every one with its line, and suggests a close
  name ("did you mean `upper`?"). It applies to any `{{NAME|filter}}`, whether
  `NAME` is defined or not; `{{ .Values.x | quote }}` (Helm) is not an evnx
  reference and still passes through (DEC-T7).
- **Chained filters are an error**, `{{X|upper|json}}`, instead of being
  written through. Chaining arrives with the one substitution engine in 0.11.0;
  until then, one filter per placeholder. A filter after a default is a chain
  too: `{{PORT|default:8080|int}}` rendered `8080|int`. A pipe followed by
  anything else stays part of the default's text (`{{X|default:a|b}}` → `a|b`).
- **`|bool` reads `true yes on 1 y` and `false no off 0 n`**, in any case.
  Anything else (`enabled`, `maybe`, a typo, an empty value) is an error, as a
  bad `|int` already was; it was `false`, silently (I-138).
- **`|title` is title case:** `hello wORLD` → `Hello World` (Jinja's rule). It
  capitalised the first character only (I-137); that is now `|capitalize`.
- A bad `|bool` or `|int` value is not printed when its name looks secret
  (`DB_PASSWORD`).
- A missing output directory says how to create it (I-144).
- **An `--output` with `..` is resolved before it goes into `.gitignore`.**
  `--output sub/../out.yml --gitignore` added the line `sub/../out.yml`, and
  `--output ../x.conf` added `../x.conf`; git matches neither, and evnx said
  "Added". Now the first adds `out.yml`, and the second, outside the
  repository, gets the warning an absolute path already got (I-252).
- `--env-name production` names the file `.env.production`, as `.env` is named,
  not `./.env.production` (I-144). `sync` and `diff` print the same label,
  and so does the `# Synced from` line `sync` writes into `.env.example`.

#### Added

- **`--dry-run`**: render and say what would be written (the file, new or
  replaced, through which link, which directory `--mkdir` would create). It
  fails where the real run would, writes nothing, prints no value and leaves
  `.gitignore` alone (DEC-T9).
- **`--mkdir`**: create the output's missing directories, each one readable by
  you only (0700); directories that already exist keep their mode (I-144).
- `|capitalize`, the old `|title`.
- `--help` lists the reference styles and every filter (D-93).

#### Changed

- The question "add it to .gitignore?" is answered on Enter, like every other
  evnx question (N-09); it was answered by a raw line read of its own.

#### Docs

- The module documentation no longer shows `-i`/`-o`, which do not exist (D-88),
  or "configurable in future" for what `--strict` does (D-89). The README has an
  options table and a filter table (D-91); Getting Started has a template step
  (D-97); the 0.5.0 notes carry a correction: strictness is opt-in (D-94).

### Shared code

One definition each, where commands had their own (no output changes beyond
the label above):

- `utils::term::interactive()`: every "is there a terminal?" check (ten of them
  in add, init, migrate, sync and the shared prompts).
- `utils::protect`: template's `.gitignore` step, for the commands that write
  secrets to a path you name. `init`, `add`, `convert -o`, `restore` and
  `cloud pull --file` adopt it in their own patches.
- `utils::string::closest()`: the "did you mean" helper, from `init`'s component
  hints, now also used by template's filters.
- `core::value::parse_bool()`: the boolean words, for `|bool` now and `spec`'s
  `format = "bool"` in its own patch.
- `core::safe_write::{preview, create_dirs, create_dirs_private, is_stdout}`:
  what a write would do, without writing; directory creation (`init`,
  `cloud export`, `template --mkdir`); what `-` means for an output
  (`template --output -`, `cloud pull --file -`). `cloud export -o a/b/c`
  now makes a missing `a` and `a/b` 0700 as well, not only `c`.
- `ui::prompt_yes_no` and `utils::file_ops::ensure_dir` are deprecated: nothing
  in evnx calls them any more.

## [0.10.4] - 2026-10-17

The `add` patch: the review fixes left after 0.10.3 (package R-add), and one
0.10.3 regression. Only `add` changed, plus the `DATABASE_URL` that
`init --blueprint` writes for three blueprints.

⚠️ **If you ran `evnx add custom NAME=value` on 0.10.3 in a git repository that
had no `.env` yet**, that `.env` was created without a `.gitignore` entry (I-251,
below). Run `git status` and `git log --all -- .env`; if a `.env` holding a real
value was committed, rotate that value. `evnx doctor --fix` adds the ignore rule.

### add

#### Fixed

- **`add custom` no longer shows a secret on screen.** In the prompt loop, a
  secret-looking name (`STRIPE_SECRET_KEY`, `ADMIN_PWD`, …) now has its value
  read without echo, as the argument form already did; it was typed visibly.
  Both forms use one rule, the shared secret-name rule at its strict level
  (`*_SECRET`, `*_KEY`, `*_TOKEN`, `*_PASSWORD`, `*_PWD`, …): `ADMIN_PWD` is
  now asked for hidden in the argument form too, and `SENTRY_DSN`, a public
  value, no longer is (I-247).
- The confirm question names every file the run writes: "Add these 5 variables
  to .env.example and .env?" — it named only `.env.example` while `.env` got the
  names too (I-247).
- **Stacks with PostgreSQL write PostgreSQL's `DATABASE_URL`.** `fastapi_ai`,
  `rust_high_perf` and `django_enterprise` (in `init --blueprint` and
  `add blueprint`) wrote `DATABASE_URL=https://example.com`, the framework's
  generic placeholder, because the first value seen won. A service's own
  default now wins over a framework's, in either order (I-250).
- Django's `DATABASE_URL` default is `sqlite:///db.sqlite3`, Django's own
  out-of-the-box database; it was `https://example.com` (N-21).
- **`add custom` writes `.gitignore` before the `.env` it creates.** Since
  0.10.3 the values go to `.env` only, and in a folder with no `.env` that file
  was created without `.gitignore`, so in a git repository the real values were
  committable (a warning was printed). It is now ignored first, as every other
  `add` flow and `init` do, and the dry run says what it will create (I-251).

#### Changed

- "No browser framework found in this project …" is printed only with `-v`.
  In a backend project it appeared on every add of a service with public
  values (Stripe, Sentry, Auth0 …). When a framework is found, the line about
  the copies is unchanged (N-22).

#### Documentation

- README: `-q` is not yet silent in the cloud commands (until CG-06), and
  `--path` records names in the nearest `.evnx.toml` at or above the folder.

## [0.10.3] - 2026-10-17

⚠️ **Why 0.10.3.** By `docs/RELEASING.md` this would be a minor bump: it changes
the output and exit codes of shipped commands and removes a flag. It is a patch
by decision, the convention set in 0.10.1: each review release ships as a
`0.10.x`, and **0.11.0 stays reserved for the first stable release** after the
command-by-command review, which is not finished (the per-command docs pass and
the issues still marked `KNOWN` in the demo locks remain).

ⓘ **This release is the cross-command pass.** 0.10.1 and 0.10.2 fixed one
command each. The reviews of the other commands found the same defects again
and again — files written world-readable, secrets printed or committed, exit
codes that meant different things, values truncated at ` #` — so this release
fixes them once, in shared modules every command now goes through, and then
fixes each command's remaining critical issues. Every changed output is in a
reviewed demo golden, and 34 demo checks across 8 commands fail on the released
0.10.2 — each names the issue it fixes.

### ⚠️ Breaking, at a glance

Each is explained in its section below.

- `diff --format patch` and `diff --interactive` print `<withheld>` instead of
  values; add `--show-values` to a script that applies the patch.
- `convert -o FILE` refuses an existing FILE without `--force`, and creates it 0600.
- Exit codes: 0 clean, 1 findings, 2 no verdict (`backup` and `restore` keep
  their own documented codes) — `sync --naming-policy error` is now 2,
  `migrate --to github-actions` exits 1 when an upload failed, an unknown
  `--format` is 2.
- The banner and notices are on stderr; stdout carries only what a command
  produces. `-q` prints nothing on success.
- `spec init --force` is gone (exit 2, pointing at the new `--merge`).
- `validate` asks a strong value of `*_PWD`, `*_APIKEY`, `*CREDENTIALS` and
  `PASSWD` names.
- `scan` reads every file in a folder; a scan of a large repository may read and
  find more. `--extensions` restores a list, and says so.
- `add custom` writes placeholders, not your values, into `.env.example`.
- `restore` sends a file it cannot fully parse to `.restored`.
- Library: see *Library* below; `commands::validate::checks::is_secret_shaped`
  was removed and several `run` functions changed signature.

### ⛔ Security — `diff` printed secrets

- **`diff --format patch` and `diff --interactive` print `<withheld>`** in place
  of values unless `--show-values` is given (I-071, DEC-G7). They printed every
  real value from `.env` — database passwords, API secrets — with no flag. With
  `--show-values` they print the exact values (a patch has to apply), and a
  warning on stderr says the output holds secrets.
  ⚠️ A script that saves `diff --format patch` and applies it must now add
  `--show-values`.
- **`--show-values` masks credentials inside values** (I-076, DEC-G9). It masked
  by the variable's name only, so `DATABASE_URL=postgres://admin:s3cr3t@db/app`
  printed the password. A URL now keeps its structure with the password masked
  (`postgres://admin:***@db/app`), and a token-shaped value is masked whole
  even under a harmless name — the same built-in rules `scan` uses. In `pretty`
  and `json`; without `--show-values` nothing about a value is shown, as before.

### ⛔ Security — `sync`'s "actual values" choices copied credentials into `.env.example`

- **The two interactive "actual values" choices of `sync` now refuse
  credentials** (I-113, DEC-S1), as `allow_actual` already did: "Yes, add with
  actual values" for an existing template, and "Actual values copied from .env"
  for a new one. A key whose value holds a credential — by the detectors `evnx
  scan` uses — gets its placeholder instead and is named in a warning; the other
  keys keep their real values. One `y` used to commit `sk_live_…` or a database
  password. A new template keeps the `.env`'s comments and order.
- `[AUDIT] actual values written to .env.example for: …` goes to stderr every
  time real values are written, not only with `--verbose`.

### ⛔ Security — `convert -o` wrote every secret world-readable

- **`convert -o FILE` creates FILE with mode 0600**, not the umask's 0644 (I-081).
  The file holds every value from `.env`.
- ⚠️ **An existing FILE is refused unless `--force`** (exit 2, nothing written).
  It used to be replaced in silence. A script that reruns `convert -o` must add
  `--force`. With it, the file is replaced atomically, keeps its mode, and evnx
  prints `Replaced FILE`; if that kept mode lets others read it, a warning names
  the `chmod` command (DEC-G5, DEC-G6).
- The input is never the output: `-o .env`, or a symlink or hard link to it, is
  refused even with `--force`. It used to replace `.env` with its own conversion.
- A symlinked FILE is written through to its target, and the link stays.
- A missing output directory says so in one line.

### ⛔ Security — files holding secrets were created readable by everyone

Every file evnx writes now goes through one writer (`core::safe_write`): written
to a temp file beside it and renamed into place, so a crash leaves the old file
or the new one, never half of either.

- **`template`**: the rendered file is created 0600 (it was 0644 while holding
  real values, I-130). `--output` naming the template or the env file — by path,
  symlink or hard link — is refused and nothing is written (it used to replace
  the template with real values, I-131). An existing output is replaced with
  `Replaced FILE` (I-132). New `--mode 0640` sets a mode on request.
  `--output -` prints the rendered text on stdout and writes no file; it used to
  write a file named `-` (DEC-T8).
- **`init` and `add`** create `.env` 0600 and `.env.example` 0644 (I-237). `doctor`
  no longer warns about the `.env` that `init` just made.
- **`backup`**: rotation of older backups happens only after the new one is on
  disk; a failure before then leaves your previous backup where it was (I-150).
  An existing directory at `--output` is refused and no longer renamed aside
  (part of I-146).
- **`restore`** replaces its output atomically; a crash could leave a half-written
  `.env` (I-151).

### ⛔ Fixed — `doctor --fix` reported repairs it had not made

- **`doctor --fix` checks again after every fix** (I-156, DEC-D1). With
  `.gitignore` reading `.env*` then `!.env`, it showed ✓ and exited 0 while git
  still committed `.env`. Now a problem that is still there after the fix is
  "not fixed", the exit is 1, and doctor stops offering `--fix` for it. A later
  `!.env` is your line: doctor reports it and does not edit it.
- **A committed `.env` is reported as tracked by git** (I-157, DEC-D2), with
  `git rm --cached .env` and a pointer to `evnx scan` — its values are already in
  the repository history. It used to say "NOT in .gitignore", which is false
  when `.gitignore` covers it. `--fix` never runs git.
- `--fix` prints one `Fixed:` line per change, on stderr (I-165): the
  `.gitignore` lines it added, and `.env mode 644 → 600`.
- `doctor --fix` writes its `.gitignore` lines under the same evnx section as
  `init` (I-168).
- **`template --gitignore` asks git** whether the output is ignored (I-139,
  DEC-T11). `*.yml` already covering `out.yml` no longer adds a redundant line;
  `out.yml` then `!out.yml` is no longer called "already in .gitignore"; a tracked
  output gets `git rm --cached` instead of a `.gitignore` line.
- One `.gitignore` writer for `init`, `add`, `doctor` and `template`; a `.gitignore`
  with CRLF line endings keeps them.

### Changed — exit codes mean the same thing in every command

0 is "ran, nothing to report", 1 is "ran, and the answer is no", 2 is "could
not run — no verdict". ⓘ `backup` and `restore` keep their own specific codes
(2–6, listed in their guides), unchanged. Four places broke that:

- **`sync --naming-policy error` exits 2, not 1** (I-116, DEC-S13). Under
  `--check`, 1 means "out of sync", so a refused variable name read as drift.
  The error now ends with the same "No verdict" line as every other command.
- **`diff --format` and `sync --check --format` refuse an unknown value** with
  exit 2 and the valid values (I-074). `diff --format xml` used to print the
  pretty report and exit 1, so a typo in a CI step quietly broke a JSON
  contract.
- **`migrate --to github-actions` exits 1 when an upload failed** (I-106). It
  listed "✗ 1 failed" and exited 0, so a step that migrated half its secrets
  passed. A run that cannot start (no token, GitHub unreachable) is still 2.
- `diff` and `sync --check` errors end with "No verdict: evnx did not finish"
  like every other command; `sync --check`'s own "evnx could not check:" line
  is replaced by the shared `Error:` line.

### Added — `migrate --to github-actions` runs unattended

- **`--yes` (`-y`) uploads without asking "Proceed with migration?"** (I-109).
  Without a terminal and without `--yes`, the upload now stops before contacting
  GitHub, exit 2, and names `--yes`; it used to fail with "not a terminal" after
  listing the repository's secrets, so it could not run in CI at all.
- `--yes` never replaces a secret that already exists: with neither
  `--overwrite` nor `--skip-existing` the run lists them and uploads nothing
  (exit 2).
- At a terminal both questions are answered on Enter, like every other yes/no
  prompt, and Esc cancels: "Cancelled. Nothing was uploaded."

### ⛔ Security — `add custom` put your values in `.env.example`

- **`evnx add custom NAME=value` writes a placeholder into `.env.example` and
  the value only into `.env`** (I-245, DEC-U8-1; numbered I-240 in #182). The
  value went into both, so `add custom STRIPE_SECRET_KEY=sk_live_…` put a live
  key in the committed file.
  Every name gets a placeholder (the same ones `sync` writes, such as
  `YOUR_KEY_HERE`), not only secret-looking ones. ⚠️ If `.env` does not exist
  it is created, mode 0600, so the value is not lost.
- A name given without a value gets the same placeholder in both files
  (`FEATURE_FLAG=YOUR_VALUE_HERE`, `STRIPE_SECRET_KEY=YOUR_KEY_HERE`). `.env`
  got `your_value_here` while `.env.example` got the uppercase one.
- The prompt for a value says where it goes — `Value for NAME (.env only;
  .env.example gets YOUR_KEY_HERE)`. It said "Example/placeholder value", true
  only before the fix above.

### ⛔ Security — `scan` did not read a `.pem` file in a folder

- **A folder scan reads every file** (I-047, DEC-U8-2). It read only files with
  an extension on a built-in list, so `config/id_rsa.pem` — a private key — or a
  live key in `app.swift` was never opened, and the scan said "No secrets
  detected" and exited 0.
- Binary files and files over 1024 KiB in a folder are skipped, and the scan
  says so: `skipped N file(s) …` on stderr, each file and why with `-v`, and a
  new `skipped` list (and `summary.files_skipped`) in `--format json` — added
  keys only. A file named on the command line is
  read whatever its size or extension, as before.
- ⚠️ **The old behaviour is opt-in and announced**: `--extensions env,yml,json`,
  or `[scan] extensions = ["env"]` in `.evnx.toml`. A run limited this way says
  `scanning only files ending in …` and where the list came from, and the
  config key is shown as an override. A CI scan of a large repository may read
  more files than before and find more.

### ⚠️ Breaking — `spec init --force` is gone; `--merge` replaces it

- **`evnx spec init --merge` adds only the variables `[vars]` does not declare
  yet** (I-178, DEC-SP3), and keeps every existing entry and the rest of
  `.evnx.toml` byte for byte. `--force` appended a second `[vars]` "to be merged
  by hand"; with any repeated name TOML refused the file, and every command
  that reads the config exited 2 until someone did. ⚠️ `--force` now exits 2
  with a pointer to `--merge`.
- **`spec init` never writes a `.evnx.toml` that does not parse** (I-179). An
  inline `vars = { … }` slipped past the old check and the append broke the
  file; now it is seen, and a write that would not parse is refused with the
  file left as it was.

### Fixed — `sync --check` without a `.env`, and the `evnx-sync` hook

- **With no `.env`, `sync --check` checks `.env.example` against the spec**
  (DEC-S4): every required `[vars]` name must be declared in the template —
  exit 1 names the ones that are not. CI, which has no `.env`, can now use
  `sync --check` as a gate. With no spec either there is nothing to compare:
  exit 2, `nothing to compare`. `--format json` adds `"compared_with": "spec"`
  in that case only.
- **New `--skip-without-env`**: with no `.env` and nothing to compare, skip
  (exit 0) and say so on stderr. For pre-commit hooks; not for CI.
- **The `evnx-sync` pre-commit hook works** (I-128, DEC-S15). It ran
  `evnx sync --direction forward --placeholder`, which with no terminal failed
  "not a terminal" on any drift and "file not found" on a clone with no `.env`
  — blocking every new contributor. It now runs
  `evnx sync --check --skip-without-env`: drift blocks the commit with the
  command that fixes it; a fresh clone commits. New optional hook
  `evnx-sync-fix` (`evnx sync --force --skip-without-env`) writes the missing
  names with placeholder values.

### Fixed — `sync --direction reverse` keeps the template's values

- **Reverse copies `.env.example`'s own values into `.env`** (I-115, DEC-S2),
  and generates a placeholder only where the template has none — through
  `--template-config`, which reverse used to ignore. `REDIS_URL=redis://localhost:6379`
  came back as `…/0`, `API_PORT=8080` as `8000`, `LOG_LEVEL=info` as
  `YOUR_VALUE_HERE`.

### Fixed — `doctor`, `init --from-source` and `init --detect`

- **`doctor` gives a verdict on a Pipenv or `poetry.lock`-only project**
  (I-175). It read `pyproject.toml` unconditionally and aborted: "No such file
  or directory", exit 2. It now reads whichever manifest exists, and only under
  `--verbose`, where the answer is used.
- **`init --from-source` no longer calls a variable the code reads "named
  nowhere in the source"** (I-239). `process.env.X` and `import.meta.env.X`
  were found, listed as already in the template, and in the same report listed
  as unused.
- **`init --detect` finds the database an `sqlx` project uses** (I-240). `sqlx`
  names no database itself — its features do — so an Axum + sqlx project got no
  database at all. Feature `postgres` proposes PostgreSQL and `mysql` proposes
  MySQL / MariaDB, shown as `sqlx (postgres)`; `sqlite`, or no database feature,
  proposes none and says so in one line.
- **`init --from-source` no longer proposes values the framework fills itself**
  (I-246). `import.meta.env.DEV`, `MODE`, `PROD`, `SSR`, `BASE_URL` (Vite) and
  `SITE`, `ASSETS_PREFIX` (Astro) were written into `.env.example` as variables
  to set. They are never proposed now, and a template that already lists one is
  not called unused.

### Changed — one secret-name rule, at two levels

- **`validate` asks a strong value of `*_PWD`, `*_APIKEY`, `*CREDENTIALS` and
  `PASSWD`** (F6, DEC-G14). It had its own name rule, and `ADMIN_PWD=123`,
  `STRIPE_APIKEY=123`, `CREDENTIALS=123` and `PASSWD=123` validated clean while
  `diff` and `scan` already treated those names as secrets. ⚠️ A `.env` that
  passed `validate` may now report `weak_secret` for one of them (exit 1).
  `--fix` still replaces a real value only with `--rotate-weak-secrets`.
- Unchanged, on purpose: `validate` still ignores a setting whose name merely
  contains a credential word (`AUTH_ENABLED`, `QUOTA_FREE_TOKENS`, `API_KEY_ID`,
  `SECRET_KEY_ROTATION_DAYS`) and the shell's `PWD`; masking, `scan` and `sync`
  still hide all of them. Both levels now live in `core::classify`.

### Fixed — `convert` and `cloud pull`

- **`convert` refuses names that become one name after renaming** (I-088,
  DEC-U8-3). With `db_host=a` and `DB_HOST=b`, `--transform uppercase` printed
  `{"DB_HOST":"b"}` and exited 0 — one value silently gone. Now exit 2, both
  names listed, nothing written.
- **`cloud pull --file -` writes to stdout** (I-244, DEC-U8-4), with the status
  lines on stderr. It wrote the plaintext to a file named `-` in the working
  directory.

### ⛔ Security — `cloud run`, and what an API token may do

- **`cloud run` refuses vault variables that make a program run other code**
  (I-228, DEC-CL1): `PATH`, `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `BASH_ENV`,
  `PYTHONPATH`, `JAVA_TOOL_OPTIONS` and similar. Any name from the vault was
  injected, so anyone who could push to a vault could run code in every
  reader's process. Nothing runs; `--exclude` them, or pass
  `--allow-unsafe-names` if the vault is meant to set them.
- **`cloud run` no longer hands evnx's own settings to the child** (I-227):
  `EVNX_TOKEN`, `EVNX_PASSWORD`, `EVNX_SERVER` and the rest stay with evnx.
- **An API token can no longer delete a vault or a version, share, change a
  role, revoke or re-key** (I-220, DEC-VL1): refused before any request. The
  server refuses them too (evnx-server SV-1). Push, pull, run, list and members
  stay open to tokens — what CI needs.

### Fixed — `evnx org`, `vault` and `cloud`

- **`org delete --force` is the gate it says it is** (I-214, DEC-OR1): it sent
  `force: true` whenever seats were assigned, so `--yes` alone deleted an
  organisation with seat holders. Without `--force` that is now refused, naming
  the flag.
- `org create` checks the slug and name, and `org invite` the email, against the
  server's own rules before sending anything (I-215, I-216).
- `org remove`, `delete`, `leave` and `auth devices disavow` ask with the shared
  confirmation: piped `y` no longer confirms, and without a terminal they stop
  and name `--yes` (I-218).
- `org invites` shows `2026-10-20 09:00`, not the raw timestamp (I-219); every
  cloud command now formats times one way.
- `vault share` and `vault role` accept `--role Admin` (I-223).
- `cloud link <vault-id>` writes the vault's name, not `vault = "/"` (I-221).
- `cloud unlink` removes `.evnx.toml` when nothing else is left in it (I-224).
- `cloud status` reports an `EVNX_TOKEN` instead of "not signed in" (I-232).
- **`cloud export` refuses to write into a git repository** unless `-o` names the
  place or `--force` accepts it, and every export gets a `.gitignore` of `*`
  (I-230, DEC-CL6). The default `./evnx-export` is plaintext.

### Fixed — `evnx auth`

- **An API token can no longer change the master password, download the
  account's data or delete the account** (I-211, DEC-AU3). With `EVNX_TOKEN` set
  those three went to the server — the other account commands already refused
  locally. All of them now refuse before any request, with the same message.
- **`auth sessions revoke` needs at least 4 characters of a session id**
  (I-204, DEC-AU1). An empty target matched every session, so
  `evnx auth sessions revoke "$ID" -y` with `$ID` unset signed the machine out.
- **Revoking the session you are signed in with signs this machine out** (I-209):
  it said so but kept the dead login, so the next command failed instead.
- `auth status` says which credential it used (`using API token (from
  EVNX_TOKEN)` or `signed in on this machine`); `auth logout` with only
  `EVNX_TOKEN` says a token is ended with `evnx auth token revoke`, not "nothing
  to do" (I-212, DEC-AU10).
- Hints that named commands which do not run are fixed: `auth totp
  recovery-codes` (was `backup-codes`, I-202) and `auth sessions list` (was the
  group `auth sessions`, twice, I-208).
- `auth token list` lines its columns up by display width, so accented and CJK
  token names no longer push the columns out (I-207).

### ⛔ Security — cloud requests stay on the resource you named

- **Text you give a cloud command can no longer change which API resource it
  reaches** (I-217, DEC-OR2). It was pasted into request paths, so `evnx org
  uninvite "../members/u-bob"` sent `DELETE /orgs/<org>/members/u-bob` — removing
  a member instead of revoking an invite — and a value with a space, `?` or `#`
  became a query string. Every value is now encoded as one path segment, and
  `.`, `..` and empty values are refused before anything is sent: the URL
  standard collapses `..` (and `%2E%2E`) whatever the encoding.
- **`auth token create --expires-in-days` takes 1 to 3650** (I-205, I-210,
  DEC-AU2). `0` and `-5` created a token that had already expired. The server now
  enforces the same range.
- **`cloud audit --last` takes `1d` to `3650d` or `all`** (I-231, DEC-CL7). A huge
  value such as `9223372036854775807d` crashed the command (exit 101).
- `-y` now works as `--yes` on `auth rotate-master-password`, `org remove`,
  `org delete`, `org leave` and `cloud delete-version`, as it already did on every
  other command with `--yes` (I-213).

### ⛔ Security — the commands `migrate` prints ran parts of your values

- **vercel and azure put values in double quotes** (I-101). Running the printed
  commands emptied `$5`, replaced `$HOME` with your home folder, and **ran**
  `$(…)` and backticks inside a value. Every value is now single-quoted; vercel
  gets it through `printf '%s'` instead of `echo`, which also stops a trailing
  newline being stored.
- **aws escaped an apostrophe as `\'` inside single quotes** (I-094), which ends
  the string: the printed command was a syntax error, or ran the rest of the
  value. It is now written `'\''`.
- **Names are checked before anything is printed** (I-102, I-107).
  `--heroku-app`, `--vault-name`, `--secret-name`, `--aws-profile`,
  `--vercel-project`, `--project`, `--doppler-config`, `--infisical-env`,
  `--railway-project` and `--repo` were pasted into commands unvalidated, so
  `--heroku-app 'x;echo PWNED'` printed a second command. A name the platform
  cannot accept is refused (exit 2, naming the flag and the rule): Heroku app
  names 3–30 lowercase letters, digits and dashes; Key Vault names 3–24; AWS
  secret names `A-Za-z0-9/_+=.@-`; `--repo` must be `owner/repo`; the others
  allow letters, digits and `._-@+=:/`. Names that pass are still quoted.
- **`--from environment` needs `--include`** (I-103). It sent the whole process
  environment minus 14 system names — in CI, the runner's own tokens with it.
  Now it refuses without `--include`, and lists on stderr the names it will
  send.

### ⛔ Fixed — values written into `.env` files lost everything after ` #`

- **`sync` and `add custom` quote a value when it needs it** (I-114). They wrote
  `KEY=value` raw, so `APP_NAME="My App # prod"` came back as `My App` — the
  rest read as a comment — and `MSG=hello # world` as `hello`; leading and
  trailing spaces were lost too. One writer now serves `sync`, `add`, `init`'s
  schema output and `validate --fix`: plain values stay bare, a value with `$`
  or `\` goes in single quotes (literal in every dotenv reader), anything else
  in double quotes, and a line break is written as `\n` (`validate --fix` used
  to refuse one).
- **`restore` puts a valid `.env` back where it belongs** (I-145). Its "is this
  a `.env`?" check was a line heuristic that rejected `export A=1` and files with
  a PEM block, and wrote such backups to `.env.restored`. It now asks the parser
  every command reads `.env` with. `backup` no longer warns about those files.
  ⚠️ The other direction changed too: a file with a line the parser cannot read
  (up to a fifth of the lines used to be tolerated) now restores to `.restored`,
  the side that cannot overwrite a good `.env`.

### Changed — one placeholder rule, with the variable's name

`validate` and `scan` each kept their own list of placeholder values, and they
disagreed on 32 of 56 common values. Both now ask one rule, at one of two levels:
**Strict** for `validate` (a placeholder verdict lets `--fix` overwrite the
value) and **Lenient** for `scan` and display. The lists moved unchanged; what
changes for you:

- **`validate` reports more placeholders**: `API_KEY=dev`, `DB_PASSWORD=test`
  (a placeholder word under a secret name, DEC-G11), `*****` and AWS's
  documentation key `AKIAIOSFODNN7EXAMPLE` (DEC-G10). `ENVIRONMENT=development`
  and `MODE=test` are still real values.
- **`scan` treats `<set-me>`, `change-me`, `none`, `null` and an empty value as
  placeholders** (DEC-G10).
- `validate --fix` still never treats prefix forms such as `test_db_pass`,
  `example_value` or `abc123` as placeholders, so it never overwrites them
  (DEC-G13).

### Changed — symlinks and existing file modes

⚠️ **One rule now, in every command:** a symlink at the output is written
through to its target and the link stays, with `· wrote through LINK -> TARGET`;
an existing file keeps the mode you gave it (DEC-S6, DEC-S7, DEC-B7, DEC-G6).

- `sync` used to replace a symlinked `.env.example` with a plain file (I-122), and
  forced 0600 on a `.env` you had set to 0640 (I-123). A new `.env.example` from
  `sync` is now 0644 (it came out 0600 by accident).
- `backup` and `restore` refused a symlinked output; they now write through it.
- `restore` over an existing `.env` used to force it to 0600; it now keeps its
  mode. When a file that holds secrets is readable by other users, evnx warns
  and names `chmod 600 FILE` instead of changing it.

### Security — dependency

- `rustls` 0.23.45 → 0.23.46 (through `reqwest`, used by the cloud commands,
  `migrate --to github-actions` and `update`): RUSTSEC-2026-0346 (`Debug` output of
  client session values exposed TLS secrets, medium), -0347, -0348, -0349 (low).
  Landed in #170; this entry was missing there.

### Changed — what goes to stdout and what to stderr

⚠️ **Scripts that read stderr or stdout line by line should check this.**
Stdout carries what a command produces; notices about the run go to stderr. A
terminal shows both exactly as before.

- **The command banner is on stderr**, for every command (`  evnx sync  .env ↔
  .env.example`). `evnx spec init --stdout > spec.toml` now writes valid TOML;
  it used to start with the banner, which made the file unreadable by the next
  command (I-176). A script that captured stdout and looked for the banner must
  read stderr.
- **Warnings and errors printed by evnx (`! …`, `✗ …`) are on stderr**, with
  their detail lines: the permissions hint in `sync`, the `.gitignore` remedy and
  conflict list in `add`, the `.restored` fallback note in `restore`.
- `scan -q` no longer prints the banner (part of I-060).
- `cloud pull`, `push`, `run`, `history` and the other vault commands print
  "using <vault> from <file>" on stderr, and not at all under `-q` (I-226). It
  used to be the first line of stdout, ahead of the pulled file or the child's
  output.
- Not changed yet, and announced here one release ahead: success lines (`✓ …`),
  tips (`· …`) and "Next steps" stay on stdout in this release and are planned to
  move to stderr in a later one.

### Changed — `-q` is silent on success

`-q` now means what its help says: **nothing on success**; findings, warnings,
errors and data you asked for still print (I-007). A script that ran with `-q`
and read a success line must check the exit code instead.

- `diff` (no differences), `convert -o`, `scan` (clean), `doctor` (healthy),
  `backup`, `restore`, `cloud link` and `cloud unlink` print nothing on success.
  `backup` drops the "Important" reminder too (I-079, I-060, I-164, I-153).
- `doctor -q` with a finding prints the full report and exits 1, as before.
- `migrate -q` drops "Loaded N secrets" and "Next steps" (I-105). The
  destination's own intro lines stay until `--print-only` (I-111).
- `cloud status -q` and `auth status -q` print the answer only (server, account,
  status, token, vault): no heading, hints or Docs line.
- `restore -q --inspect` and `--dry-run` still print the backup information.
- The `-q` help text says this, instead of "Suppress non-essential output".

### Library

- New: `core::safe_write` (`write_private`, `write_shared`, `write_with_mode`,
  `same_file`, `report`, `Written`) and `core::permissions` (`is_private`,
  `make_private`, `mode`, `PRIVATE`, `SHARED`, `MODES_APPLY`) — the one way evnx
  writes files (C-04).
- `core::safe_write::stage_private` and `Staged::commit`: prepare a write, do
  something else (backup rotates), then make it visible.
- `commands::template::run_with` adds the output's file mode; `run` is unchanged.
- `utils::write_secure` is deprecated in favour of `core::safe_write::write_private`.
- New: `core::classify` (`is_secret_shaped`, `is_sensitive_key`, `holds_secret`,
  `url_password`) and `core::mask` (`mask`, `disclose`, `Disclosure`, `MASK`,
  `WITHHELD`) — one answer to "is this a secret" and one way to show it (C-05).
- New: `core::gitignore::{is_protected, exposure, Exposure, append_entries}`.
  `env_file_is_protected` now delegates to `is_protected`. Deprecated:
  `utils::git::{is_in_gitignore, add_to_gitignore}` and
  `utils::file_ops::append_if_missing` (text matching; no callers left).
- `core::classify::{is_placeholder, Strictness}`: the one placeholder rule.
  `utils::patterns::is_placeholder(value)` now asks it (Lenient, no key) and
  answers as before except for DEC-G10. ⚠️ `commands::validate::checks::is_placeholder`
  now takes the variable's name: `is_placeholder(name, value)`.
- `commands::convert::ConvertConfig` has a new public field `force` (and
  `ConfigBuilder::force`). Code that builds the struct literally must add it;
  the builder and `Default` are unaffected.
- `utils::ui::print_header_stderr` is deprecated: `print_header` writes to stderr
  now, and both are hidden by `-q`. New: `utils::ui::warning_detail`.
- `utils::ui::warning` and `utils::ui::error` write to stderr.
- New: `core::verdict` (`EXIT_OK`, `EXIT_FINDINGS`, `EXIT_ERROR`, `Verdict`,
  `finish`, `no_verdict`, `Stage`) — the exit-code contract (C-04).
  ⚠️ `commands::sync::run` and `commands::migrate::run` return
  `Result<Verdict>` instead of `Result<()>` and no longer exit the process;
  `sync::EXIT_*` are now aliases of the `core::verdict` constants.
- New: `utils::prompt::NoTerminal`, the typed error `require_terminal` and
  `require_terminal_or` return (same message as before).
- `commands::migrate::MigrateArgs` and `MigrationOptions` have a new public
  field `yes`. `MigrationOptions` literals with `..Default::default()` are
  unaffected; a full `MigrateArgs` literal must add it.
- New: `core::envfile` (`quote`, `line`, `is_dotenv`, `count_vars`) — the one
  way evnx writes a `.env` line and decides whether text is a `.env` (C-01).
  `utils::looks_like_dotenv` and `utils::count_dotenv_vars` now ask it.
- New: `core::shell` (`quote`, `word`, `assignment`), the one way evnx writes a
  shell word (A-16); `formats::quoting::shell_single` now asks it.
  `core::args` (`check`, `check_opt`, `Name`) checks names (A-17).
  `core::envsource` (`SKIP`, `is_skipped`, `require_include`, `read`) reads the
  environment (A-18); `migrate::sources::is_system_variable` asks it.
- New (U7, C-09): `cloud::client::api_path` builds every cloud request path;
  `Client::require_user_session` and `Client::require_credential` are the one
  credential guard (the five `require_login` copies delegate; `vault::require_session`
  is gone); `core::args::{MAX_DAYS, parse_days, days_before}`;
  `utils::prompt::confirm`; `core::envsource::{UNSAFE_NAMES, UNSAFE_PREFIXES,
  is_unsafe_name, is_evnx_own}`.
- New: `cloud::when` (one timestamp format), `core::args::{Name::OrgSlug,
  check_email}`, `core::gitignore::in_work_tree`, `cloud run --allow-unsafe-names`.
- U8: ⚠️ `commands::validate::checks::is_secret_shaped` is removed — use
  `core::classify::is_secret_name(key, Strictness::Strict)`; the Lenient level
  is what `is_sensitive_key` answers. `commands::scan::run` takes a trailing
  `extensions: Option<(Vec<String>, &str)>`, `commands::sync::run` a trailing
  `skip_without_env: bool`, and `commands::spec::run` a `merge: bool` before
  `force`. New: `scan::filters::{Collected, MAX_WALK_BYTES}`,
  `FileFilter::{with_extensions, extensions, collect}`,
  `ScanRunner::with_extensions`; `ScanResults::skipped` and
  `Summary::files_skipped`; `core::config::ScanPolicy::extensions`.

### Fixed

- **`scan` and `init --from-source` walk folders in name order.** They followed
  the order the filesystem returns, which differs between machines, so the same
  project printed a different `scanning …` line and findings order (including in
  JSON and SARIF) in CI than on a laptop, and `init --from-source` could report a
  variable at a different file (I-243).

### Tests

- The training demos for `sync`, `template`, `convert`, `diff`, `doctor`,
  `migrate`, `spec`, `scan`, `validate`, `init` and `backup`/`restore` run in CI
  scene by scene (`tests/<cmd>_runner.rs`, 140 scenes). Each scene must match
  `tests/golden/demos/<cmd>/<scene>.txt`, and a check marked `KNOWN` that flips
  fails the test until its runner line is changed on purpose. Linux only.
- `tests/backup_format_lock.rs`: backups made by 0.10.0 and 0.10.2 must keep
  restoring byte for byte. The key file there is a published test passphrase.
- `tests/spec_golden.rs` pins the `[vars]` block `spec init --stdout` prints;
  `spec_integration` parses that stdout as TOML (T-SP-20).
- `scripts/behaviour-lock.sh <cmd> [old] [new]` runs a demo twice, or with two
  binaries, and lists every line that moved.
- `tests/convert_output.rs` (8 tests) pins how `convert -o` writes: mode, refusal
  without `--force`, same file by path/symlink/hard link, write-through, `-q`.
- `tests/template_output.rs` (9), `tests/env_modes.rs` (4, T-INIT-27), a sync
  symlink test and backup/sync unit tests pin the writer's rules per command.
- `tests/doctor_gitignore.rs` (5) pins I-156, I-157, DEC-D1, DEC-D2 and JSON
  under `--fix`; three template tests pin I-139; `core::gitignore` gains unit
  tests against a real git repository and a CRLF file.
- `tests/diff_values.rs` (4) pins I-071 and I-076; `core::classify` and
  `core::mask` have unit tests (URL structure, no prefix, UTF-8).
- `tests/sync_pty.rs` (2) drives both "actual values" choices through a real
  terminal (I-113).
- `tests/architecture.rs` (T-SHARE-01): `core` imports no command, and no
  placeholder or mask function outside `core` decides on its own.
  `tests/validate_placeholders.rs` pins DEC-G11 end to end; `core::classify`
  unit tests pin DEC-G10, G11 and G13 at both levels.
- `tests/quiet_walk.rs` (T-SHARE-60) reads every command from `evnx commands
  --json`: each one has a `-q` success case that must print nothing, or a stated
  reason it has none. A new command fails the test until it is classified.
- `tests/exit_verdicts.rs` (6) pins I-116 and I-074; `tests/migrate_exit.rs`
  (2, Unix) drives `migrate --to github-actions` against a local mock of the
  GitHub API and pins I-106; `tests/architecture.rs` adds that `sync` and
  `migrate` never call `process::exit` themselves.
- `tests/migrate_exit.rs` adds I-109 (no terminal without and with `--yes`,
  existing secrets under `--yes`, Esc at the confirmation) and T-MIG-28
  (`--skip-existing`), all against the mock GitHub API.
- `tests/env_writer.rs` (5) pins I-114 for `sync` and `add custom` and I-145 for
  `restore`; `core::envfile` round-trips 24 awkward values (CRLF, PEM, `$NAME`
  next to a defined variable, mixed quotes) through the real parser;
  `tests/architecture.rs` refuses a raw `KEY=value` write in sync, add or schema.
- `tests/migrate_quoting.rs` (8, Unix) runs the printed commands of aws, azure,
  vercel, heroku, doppler, infisical and railway through `sh` with stub tools and
  checks each of 10 values arrives exactly and nothing runs (T-MIG-01..04, 07);
  names refused and accepted (T-MIG-09, 32); `--from environment` (T-MIG-17,
  18). `core::shell` unit test feeds 15 values through `sh` (T-SHARE-11).
- `tests/cloud_inputs.rs` (3): token lifetime, audit window and `-y` on every
  `--yes` (T-SHARE-52), each refused with no request. `api_path` is tested against
  what `reqwest::Url` really sends; `tests/architecture.rs` refuses a hand-built
  `/api/v1` path in `src/cloud` and a credential check outside `Client`.
- `tests/cloud_auth.rs` (5): T-SHARE-51 (8 account commands refuse a token with
  no request), I-204, I-209 (a real stored login is removed), DEC-AU10 for
  status and logout. `tests/quoted_commands.rs` (T-SHARE-49): every
  `` `evnx …` `` quoted in `src/` is a real command, not a bare group, with real
  flags — it catches I-202 and both I-208 on the old source and nothing else.
- `tests/add_custom_example.rs` (3): I-245 with and without `.env`, and a rerun is
  not a conflict; `tests/convert_output.rs` adds I-088; `cloud::sync` unit test for
  `--file -` (I-244, also checked against a local server).
- `tests/scan_walk.rs` (5): I-047 — a `.pem` and a `.swift` file are read,
  binary and over-size files are skipped and named, the list is opt-in from the
  flag and from `.evnx.toml` and announced either way, a named file is always
  read. The scan demo's `KNOWN I-047` line is flipped to `FIXED`.
- `tests/secret_names.rs` (3): F6 — validate checks the four credential names
  of U3's eight (red first), leaves the other four and `PWD` alone, and masking
  stays lenient; `core::classify` unit test for all eight at both levels; the
  one-definition guard now covers the secret-name rules (red first: it found
  both copies).
- `tests/init_integration.rs`: two more tests compared whole `.env.example`
  files including the minute-stamped `# Generated by` footer and failed on the
  Windows runner when the runs straddled a minute (T-074, missed by its first
  fix). Both now use `without_generated_stamp`.
- U8d: `tests/sync_reverse_values.rs` (5, I-115, T-SYNC-35),
  `tests/sync_check_without_env.rs` (10: DEC-S4, DEC-U8-5, and T-SYNC-51 reading
  the shipped `.pre-commit-hooks.yaml`), `tests/doctor_python.rs` (3, I-175),
  `tests/spec_merge.rs` (5, I-178, I-179), and an `init::source` unit test for
  I-239 (T-INIT-29), `init::detect` unit tests for I-240 (T-INIT-30) and an
  `init::source` unit test for I-246 — each red first. Demo scenes flipped from
  `KNOWN` to `FIXED`: sync S4 and S10, doctor DR10, spec SP3 and SP4.
- `tests/migrate_exit.rs`: `t_i109_esc_at_the_confirmation_uploads_nothing` sent
  Esc and Enter together and hung on a slow CI runner — the race #156 fixed
  elsewhere (the two bytes read as one escape sequence, the Enter lost). It now
  uses `cancel_yes_no`; 20 of 20 runs passed with every core loaded.
- `tests/cloud_org.rs` (5), `tests/cloud_vault.rs` (3), `tests/cloud_cloud.rs` (4):
  every Part D fix against a mock server, refusals with zero requests; the
  architecture guard now reads the path text, after six multi-line hand-built
  paths had passed it.

## [0.10.2] - 2026-10-14

⚠️ **Why 0.10.2.** As 0.10.1 explains, each reviewed command ships as its own
`0.10.x` and 0.11.0 is reserved for the first stable release. This one is
**`evnx add`**: every issue from its review, re-verified against the shipped
0.10.1 binary, plus one parser fix that every command benefits from.

ⓘ **Library API.** Unlike 0.10.1, this patch release changes evnx's public
library surface (see *Library* below). crates.io lists no crate that depends on
evnx, so nobody is known to be affected; it is stated so that nobody is
surprised.

### ⚠️ Breaking

**Yes/no questions in `evnx add` answer on Enter**, as in `evnx init` since
0.10.1. Type `y` or `n` and press Enter; Enter alone takes the default. Before,
the `y` answered at once and the Enter after it silently answered the next
question.

**`evnx add custom` asks less.** Per variable: name, value, and one `More
options? (description, category, required)`; an empty name finishes. `Add
another variable?` and the separate description and category questions are
gone, and the category list starts on `None`. Scripts should use the new
argument form instead (below).

### ⛔ Fixed — adding a variable again replaced your real value

`evnx add` appended every name every time. A second `add service redis` wrote
`REDIS_URL` twice into `.env.example` and `.env` and still said "Added 5
variables". Because dotenv uses the **last** assignment, the placeholder
appended to `.env` silently replaced the value you had set, so the app read
`redis://localhost` instead of your server. `evnx init --from-source` did the
same through the same code. Now each name is checked in each file:

- a name a file already has is **skipped** in that file;
- a name `.env.example` has with a **different value** is kept, listed, and
  not added to `.env` either;
- `.env` catches up with names `.env.example` already had.

The result says what happened: `Added 4 variables for PostgreSQL, skipped 2
already present`, or `Nothing to add`. Values in `.env` are never compared or
printed.

### Added

- **`evnx add custom NAME=value …`**: your own variables as arguments, with
  `--desc`, `--category` and `--required` for all of them. Works without a
  terminal under `--yes`, so scripts, CI and agents can use it. A name alone
  gets a placeholder; a secret-looking one (`*_SECRET`, `*_TOKEN`, `*_KEY`, …),
  in a terminal, is asked for without echo instead. Every name is checked
  before anything is written.
- **`evnx add --list`**: every id `add` takes, the same catalogue as `init
  --list-components`, plus the blueprints. Leave the id out
  (`evnx add blueprint`) to pick one in a terminal.
- **`--dry-run`** on every `add` subcommand: what would be added to each file,
  skipped and kept, and whether `.env` would be created. Writes nothing; needs
  no terminal.
- **`--json`** (with `--yes` or `--dry-run`): one document on stdout, with
  `added`, `skipped`, `conflicts`, `env_added`, `env_created`,
  `env_committable`, `written`, `dry_run`. Names only, and `.env.example`
  values for conflicts; nothing from `.env`.
- **`evnx add framework nextjs`** works: `--language` is optional (framework
  ids are unique). When given, it is checked, and a mismatch names the right
  language.
- **One id space**: `add service` and `add framework` take any catalogue id and
  say what it is (`vercel is an infra component; adding it.`). `evnx add infra`
  is a new alias of `add service`.
- **Browser copies in `add service`**: in a Vite, Next.js, SvelteKit, Nuxt or
  Laravel project (read from `package.json`, as `init --detect` reads it), the
  public values get their prefixed copy, as `init --with` writes them:
  `add service supabase` in a Vite project adds `VITE_SUPABASE_URL`.
- Esc cancels any `evnx add` confirmation or menu without writing anything
  (exit 0); at a yes/no question, Esc then Enter.

### Changed

- With no `.env`, `evnx add` creates one from `.env.example`, as `init` does:
  `.gitignore` first, asking in a terminal and not under `--yes`.
- `--yes` never shows the preview, in any `add` subcommand (`framework` and
  `blueprint` used to); `-q` leaves only warnings, conflicts and errors.
- The preview lists framework, service, infra in a fixed order (also `init`).

### Fixed

- **All commands: a bare `$NAME` in a `.env` value kept the character after
  it out.** `URL=$BASE/api` read as `http://localhostapi`, `$B:8080` as
  `app8080`, in `convert`, `template`, `validate`, `scan`, `diff`, `sync` and
  `migrate`. `${NAME}` was always correct.
- `evnx add custom`: a category with no description commented the variable out
  (`# [Application] NAME=value`) while reporting it added.
- A file `add` cannot write stops the run before anything is written; a
  read-only `.env` used to leave `.env.example` already appended. The error
  names the file.
- Without a terminal and without `--yes`, `add` says to add `--yes` (or, for
  `add custom`, to pass the variables as arguments) instead of `IO error: not a
  terminal: not a terminal`, and prints no preview first. Exit code still 2.
- "Next steps" no longer follows "Aborted.".
- Appended lines follow the file's line endings, so a CRLF `.env.example` no
  longer comes back mixed.
- Wording: `1 variable`, `1 conflict`; `-v` no longer mentions "TODO
  placeholders", which `add` stopped writing in 0.5.0.

### Library

- Removed: `commands::add::shared::{AppendMode, detect_conflicts,
  append_to_env_files}`. Use `commands::add::shared::{plan, apply}`.
- Changed: `commands::add::shared::Conflict` has no `existing_line`;
  `commands::add::run` takes `(Option<AddTarget>, add::Options)`; the
  subcommand handlers take a `shared::Mode`.
- Changed: `AddTarget` (re-exported as `evnx::AddTarget`): ids are
  `Option<String>`, `Framework.language` is `Option<String>`, and `Custom` has
  fields (`vars`, `desc`, `category`, `required`). `Commands::Add` gained
  `list`, `dry_run` and `json`.
- Added: `utils::prompt::{yes_no, require_terminal, require_terminal_or}`
  (moved from `init`, which re-exports them internally).

### Tests

`tests/support/` is a shared module for integration tests (temp projects, a
real isolated git repository, snapshot normalising, the pty driver).
`tests/add_golden.rs` pins what `add` writes byte for byte, and
`tests/add_runner.rs` runs the training demo's scenes in CI.

## [0.10.1] - 2026-10-13

⚠️ **Why 0.10.1 and not 0.11.0.** By the rule in `docs/RELEASING.md` this would
be a minor bump: it adds features (component aliases, browser variables, `-p`)
and changes the output of a shipped command (`init`). It is a patch by decision:
from 0.10.1 each reviewed command ships as its own `0.10.x` release (`init` here,
`add` next), and **0.11.0 is reserved as the first stable release** after the
command-by-command review. The breaking changes are few and listed first. The
library API only gained items (`schema::format_addition_from`,
`schema::component::{ALIASES, canonical, CLIENT_PREFIX, CLIENT_SAFE}`,
`utils::ui::{set_quiet, is_quiet, say}`, `utils::term`); no reachable public item
was removed or changed its signature, so a crate depending on `evnx = "0.10"`
still compiles.

ⓘ **This release is mainly `evnx init`.** Every issue found by re-verifying the
0.10.0 review against the shipped binary is fixed here, plus three behaviours
shared by every command: `-q`, a closed pipe, and Ctrl-C at a prompt. It also
carries the Windows fixes for `scan` and `doctor` listed under Fixed.

### ⚠️ Breaking

**`evnx init --from-source --force` is now an error** (exit 2, `the argument
'--from-source' cannot be used with '--force'`). `--force` never had an effect
there, because `--from-source` only appends. Drop the flag.

**The `evnx init` start menu pre-selects Blank**, where it pre-selected
Blueprint. Enter, Enter used to write a 27-variable T3 stack into whatever
directory you were in. Press Down once for Blueprint.

**Yes/no questions in `evnx init` answer on Enter.** Type `y` or `n` (or `yes` /
`no`) and press Enter; Enter alone takes the default. Before, the `y` answered
at once and the Enter typed after it answered the *next* question with its
default, unseen: `y⏎` at "Generate .env files?" silently answered "Overwrite
it?" with No, so the stack you asked for was never written.

**All commands: a closed stdout exits 141, quietly.** `evnx … | head` used to
end with `thread 'main' panicked … Broken pipe` and exit 101. ⓘ Only a closed
pipe is affected: SIGPIPE handling is unchanged, so a write to a closed network
socket is still reported as an error, and so is every other failure.

### ⛔ Security — `init` could leave `.env` without its `.gitignore` rule

`init` wrote `.env`, printed "Created .env from template", and only then
updated `.gitignore`. With stdout closed early (`evnx init … | head -n 3`) that
print panicked, `.gitignore` was never written, and a `.env` that git would
commit was left on disk: 4 to 33% of runs on 0.10.0. `init` now writes
`.gitignore` before any env file and prints only after every write. The lines
printed, and their order, are unchanged.

It also warns `.env is still not ignored` when a later line in `.gitignore`
(such as `!.env`) re-includes it. It checks with git, and never edits your file.

### Added

- **Browser variables.** For Vite, Next.js, SvelteKit, Nuxt and Laravel stacks,
  `init` adds a prefixed copy (`VITE_`, `NEXT_PUBLIC_`, `PUBLIC_`,
  `NUXT_PUBLIC_`) of each service value designed to be public: the Supabase URL
  and anon key, the Firebase web config, the Auth0 domain, client ID and
  audience, the Clerk publishable key, the Google OAuth client ID, the Sentry
  DSN, the Stripe publishable key, the Razorpay key ID and the Cloudinary cloud
  name. A Vite + Supabase stack now gets `VITE_SUPABASE_URL` next to
  `SUPABASE_URL`. Additive only: nothing is removed, and secrets are never
  copied. `evnx add blueprint` writes the same.
- **Component aliases**: `postgres`, `pg`, `mysql`, `mariadb`, `mongo`,
  `express`, `fastify`, `fastapi`, `flask`, `axum`, `actix`, `gin`, `echo`,
  `next`, `k8s`. They write exactly what the full id writes, and work in
  `init --with`, `add service`, `add framework` and `spec init --with` (the
  `spec --help` example `--with django,postgres,redis` now works).
- **Typo suggestions** for unknown components and blueprints: `supabse` →
  `Did you mean: supabase`, `t3modern` → `t3_modern`. Existing hints are
  unchanged.
- **`.env` from a template you keep.** Answer "no" to "Overwrite it?" and `init`
  offers to create `.env` as a copy of your `.env.example`. `--from-source` on a
  project with a template but no `.env` creates it too (asking first unless
  `--yes`), and writes the `.gitignore` rules. An existing `.env` is never
  replaced.
- `-p` as the short form of `init --path`, as `evnx add` has.
- `--detect` recognises more packages: `@clerk/react` (and the deprecated
  `@clerk/clerk-react`), `@clerk/react-router`, `@clerk/remix`, `@clerk/astro`,
  `@clerk/express`, `@clerk/backend`, `@sentry/react`, `@sentry/browser`,
  `@sentry/vue`, `@sentry/sveltekit`, `@supabase/ssr`. `@stripe/stripe-js` is
  deliberately not recognised: it is the browser SDK, and the Stripe component
  carries a secret key.
- Esc or `q` cancels any `init` prompt without writing anything (exit 0).

### Changed

- **`-q/--quiet` is silent on success, for every command.** Headers, success
  lines, tips, "Next steps" and the docs hint are suppressed. Errors, warnings,
  findings and data output (`convert`, `spec --stdout`) still print, and exit
  codes are unchanged. Without `-q`, output is unchanged.
- `init --from-source` recognises `import.meta.env.X` and `import.meta.env["X"]`
  (Vite, Astro, SvelteKit), `Deno.env.get("X")`, `Bun.env.X` and `Bun.env["X"]`,
  and scans `.mts`, `.cts`, `.vue`, `.svelte` and `.astro` files. Component
  files are read for direct access only, so a label in the markup is never
  proposed. Re-running it may append newly found names.
- `init --from-source` no longer follows directory symlinks (it says how many it
  skipped), and warns when it stops at 5,000 files, after which it calls
  nothing unused.
- Generated files have one `Authentication` section: `NEXTAUTH_SECRET` no longer
  sits under a separate `Auth` heading. No variable or value changes.
- Appends by `init --from-source` are stamped `# Added by evnx init
  --from-source` (they said `evnx add`). Library: `format_addition_from(vars,
  origin)` is new; `format_addition` is unchanged.

### Fixed

- **`init`: no "Next steps" after an abort**, and no "✓ Created .env.example"
  after you chose to keep your own template. Both told you to edit a `.env` that
  did not exist.
- **`init` without a terminal** says which flags to use (`--yes`, `--blueprint`,
  `--with`, `--detect`) instead of `IO error: not a terminal: not a terminal`.
  `--with` and `--blueprint` without `--yes` say to add `--yes`. Exit code
  still 2.
- Ctrl-C at a prompt no longer leaves the terminal cursor hidden (Unix).
- `init` wording: `Created .env.example (no variables yet)` instead of "empty";
  `1 component`; one colon after prompts; `PHP`; infrastructure shown by name
  (`AWS (Advanced)`) rather than id; `Nothing detected in this project` instead
  of an empty list.
- Removed an unreachable Spring Boot detection row: `pom.xml` and Gradle files
  are not read. Name the stack with `--with spring_boot`.

#### ⛔ `scan` emitted Windows path separators into machine-readable output

Three outputs that other programs parse carried `.\.env` instead of `./.env`
on Windows, and each one broke differently:

- **SARIF `artifactLocation.uri`** is specified as a URI, which uses `/`.
  GitHub code scanning could not associate the alert with a file in the
  repository, so it attached to nothing.
- **GitHub Actions annotations** (`::error file=…`) are matched against
  repository paths. `evnx-action` on a windows runner reported findings that
  annotated no file.
- **SARIF `partialFingerprints`** is the *identity* of a finding across runs —
  it exists so that inserting a line above a secret does not close one alert
  and open another. ⚠️ With a platform-dependent separator inside it, the same
  secret in the same file had a different identity depending on which runner
  scanned it, so a matrix build duplicated every alert and a dismissal made on
  one platform was lost on the other. That is precisely the failure the
  fingerprint was added to prevent.

ⓘ Normalisation is Windows-only. `\` is reserved in a Windows filename so the
rewrite is lossless there; on Unix it is an ordinary character and a file
really can be called `weird\name.env`, where rewriting it would report the
wrong path.

#### ⛔ `--exclude` glob patterns were inert on Windows

`--exclude "fixtures/**"` excluded nothing. `glob::Pattern` treats `/` as its
only separator, so it could never match the `.\fixtures\.env` the walker
produces, and the leading `.\` defeated the `./` prefix strip as well.

⚠️ This is the branch the previous release's exclusion fix missed. That one
corrected the hardcoded directory list and the plain-substring patterns and
left globs untouched — so `--exclude` appeared to work, and silently did not.

ⓘ The substring branch is narrowed in the same change. It previously matched
both the raw path and a normalised copy on *every* platform, which widened Unix
by accident: a file genuinely named `a\b` began matching `--exclude a/b`. For a
scanner an over-exclusion is a **silent miss** — the file is never read and the
output still says no secrets were detected.

#### ⚠️ `doctor` on Windows reports file permissions as skipped

Unchanged behaviour, now pinned by a test. File modes are a Unix concept, so
the check reports `info` / "Skipped: Windows platform" rather than a warning.
The assertion exists so the check cannot quietly disappear from the report
while the summary counts still add up.

#### ⛔ `scan`'s directory exclusions never fired on Windows

`.git`, `node_modules`, `target`, `dist` and `build` were scanned in full on
every Windows machine, and had been since the exclusions were written.

They were forward-slash substrings — `"node_modules/"` — tested with
`path.to_string_lossy().contains(…)`. On Windows that string is
`node_modules\pkg\.env`, so not one of the five ever matched. Directories are
now matched **by path component**, which lets the platform's own path parser
handle separators instead of comparing a `/` pattern against a `\` string.

ⓘ The direction matters: it over-scanned, so no secret was ever missed because
of this. What users got was a slow walk of `node_modules` and a flood of
findings from lockfile hashes and vendored fixtures — which is its own way to
teach someone the tool cries wolf.

Two smaller defects in the same comparison, both found while fixing it:

- **A directory whose name *ended with* an excluded one was also skipped** —
  `contains("dist/")` is true for `redist/`, `contains("target/")` for
  `my_target/`, `contains("build/")` for `prebuild/`. All silently excluded
  from every scan, on every platform. ⚠️ Note the shape: the extra text has to
  come *before* the name, so `my-node_modules-notes/` was never affected.
- **`.env.example` was a substring, not a file name**, so `.env.example.bak`
  and `my.env.example-notes` were excluded too. A copy of a template with a
  real key pasted into it is exactly what should be read.

Your own `--exclude` patterns are also now matched against the path with
separators normalised, as well as the path as written — so `--exclude
node_modules/`, the spelling `.gitignore` and every tutorial use, works on
Windows. ⓘ Trying both spellings can only *add* a match for a pattern you
typed on purpose, so no existing exclusion changes meaning.

⚠️ **CI now runs this module's tests on `windows-latest`.** Nothing in the repo
had ever run a test on Windows, which is why a Windows-only defect could ship;
a note in the tracker claimed otherwise. It is scoped to the module the fix
touches — the other 786 tests have still never executed there, and finding out
what they do is a separate task rather than something to discover inside a
bug-fix.

## [0.10.0] - 2026-10-10

### ⚠️ Four dependencies that were compiled for nothing

Each verified by building, not by reading the manifest:

- **`serde_json` in `[build-dependencies]`** — and there is no `build.rs`.
  Cargo reports the crate's build-script target as `NONE`, so this was fetched
  and compiled on every build of every install, for a script that does not
  exist. ⓘ The `[dependencies]` and `[dev-dependencies]` entries are real and
  untouched.
- **`dotenvy`** and **`console`** — zero code references. `console` is still
  in the tree through `dialoguer` and `indicatif`; only the redundant direct
  declaration is gone.
- **`rand`** — a direct optional dependency of the `backup` feature, and
  `cargo check --features backup` succeeds without it. The randomness that
  feature actually uses comes from `aes_gcm::aead::OsRng` and `getrandom`.

`chacha20`, `dotenvy` and `rand` leave the dependency tree. `src/utils/selection.rs`
is removed too — it was **0 bytes** and declared in no `mod`.

ⓘ Verified against all seven feature combinations — default, `net`, `migrate`,
`backup`, `cloud`, `full`, `--all-features` — before and after each removal,
one at a time so a failure would name its own cause.

### ⚠️ `rust-version = "1.85"` was a promise nothing checked

CI now has an MSRV job pinned to 1.85.0, running
`cargo check --all-features --all-targets`.

The floor held when measured — this does not fix a break, it stops one
happening quietly. A dependency bump can raise the real minimum at any time,
and without this the first person to find out is a user on 1.85 getting an
error from a crate they have never heard of.

ⓘ `--all-targets` so tests and benches are covered: a dev-dependency raising
the floor breaks `cargo test` for the same people.


⚠️ **Why 0.10.0 and not 0.9.1.** Three new commands (`cloud audit`, `cloud
diff`, `cloud watch`) and two changes to the output of commands that already
shipped — `diff --format json` lost its `*_redacted` fields, and `scan` now
reports credential classes it did not before. In `0.x` a feature is a minor
bump, and a change to the output of a shipped command is a compatibility
change, which is also a minor bump.

ⓘ **This is the release every security fix from the 2026-10-07 review has
been held for.** S1–S5 plus the external review's A1, B1, C5, S7, S8 and C3.
Anyone on 0.9.0 or earlier is running at least four known defects, two of
which leak credentials.

### ⛔ Security — `validate --fix` overwrote working credentials

A user read the source and reported six defects. Four were real. This is the
first of them.

`evnx validate --fix` replaced a **real** credential with random hex, printed
the one it replaced to stdout, wrote no backup, and reported `✓ All checks
passed`:

```text
DATABASE_PASSWORD="p@ss word#1"  →  DATABASE_PASSWORD=ad0da713…c508
  • DATABASE_PASSWORD: "p@ss word#1" → ad0da713…
  ✓ All checks passed                                            exit 0
```

It also rewrote **every** line in the file for a single repair, because it
looked each key up in the whole parsed map rather than in the set of fixes. So
quoting, trailing comments and `export ` prefixes were lost — and the first of
those is not cosmetic:

```text
QUOTED="has spaces and #hash"   →   QUOTED=has spaces and #hash

$ . ./.env
./.env: line 1: spaces: command not found
$ echo "[$QUOTED]"
[]
```

**What changed**

- `--fix` repairs a weak secret **only when the value is a placeholder**. The
  check judges length and wording; it cannot tell a placeholder nobody filled
  in from the real password, short because someone chose a short one — and the
  second is the common case, because shortness is what trips it. The finding
  still fires, with advice to rotate the credential where it is issued.
- **`--rotate-weak-secrets`** (new) does it deliberately, for when nothing
  outside the file holds the value.
- A line whose key was not repaired is **copied byte for byte**.
- A value evnx writes is quoted when it would not survive being read back, and
  a value containing a line break is refused rather than truncated.
- **`.env.bak`** is written before anything is touched, and never clobbers an
  earlier one (`.env.bak`, `.env.bak.2`, …).
- The replaced value is withheld for a credential-shaped variable that held a
  real value — in the struct, not the printer, because `--format json`
  serialises it. `old_value_withheld: true` distinguishes "withheld" from "the
  variable was added".

### ⛔ Security — `diff --format json` printed secret values

The defect the GitHub Action leaked through. The action is forced to use
`--format json`, because `diff`'s exit code is unusable for CI.

`DiffItem` held the raw value **and** a redacted copy, and serde serialised the
struct — so the human output picked the mask and the JSON output picked the
secret:

```console
$ evnx diff --format json | jq '.different[0]'
{
  "key": "DB_PASSWORD",
  "example_value": "example_pw",
  "env_value": "HUNTER2_live",      ← the live password
  "env_value_redacted": "HU***"
}
```

`--show-values` was inert in both directions: `pretty` masked regardless of it,
`json` leaked regardless of it.

Three more defects turned up while reproducing that one:

- **`diff` panicked on a non-ASCII secret.** The mask was
  `format!("{}***", &v[..v.len().min(2)])`, and byte 2 is not a character
  boundary when the value starts with a 3-byte character. `DB_PASSWORD=日本語`
  exited **101**.
- **`--show-values` printed sensitive values from the `missing` and `extra`
  sections completely unredacted.** The redaction lookup searched `different`
  for a mask — and a key present on only one side is never in `different`, so
  the lookup always missed and the raw fallback always won.
- **`diff` and `validate` disagreed about what a secret is.** `STRIPE_KEY`,
  `ENCRYPTION_KEY` and `SIGNING_KEY` were credential-shaped to `validate` and
  not sensitive to `diff`, because `is_sensitive_key` matched `API_KEY` and
  `PRIVATE_KEY` as substrings but had no `*_KEY` suffix rule.

**What changed**

- One function, `disclose`, decides what may be shown, and all three output
  sections call it. The report and the human output can no longer disagree,
  which is how they came to.
- The mask is a fixed `***` with **no prefix of the real value**. `sk***`
  identified the issuer; the key name beside it already said more. This also
  removes the slicing, so the panic is gone by construction.
- `is_sensitive_key` gained a `*_KEY` / `*_SECRET` / `*_PASSWORD` / `*_TOKEN`
  suffix rule. Deliberately broader than `validate`'s rule — masking a rotation
  interval costs a line of information, printing a signing key costs the key.
- ⓘ `--format patch` still emits real values, because a patch exists to be
  applied and a masked one would write `***` into the file. It is reachable
  only by asking for it: `pretty` is the default and the Action forces `json`.

### ⛔ Security — `scan` reported a clean bill of health on files it never read

A `.env` saved as UTF-16 — which is what a Windows editor produces when you
pick "Unicode" — scanned clean:

```console
$ evnx scan
  ✓  No secrets detected
     1 file scanned
$ echo $?
0
```

over a live Stripe key. Verified against the released 0.9.0 binary:

```text
                  before          after
UTF-8             found=1 exit=1  found=1 exit=1
UTF-16 + BOM      found=0 exit=0  found=1 exit=1
UTF-16LE no BOM   found=0 exit=0  found=1 exit=1
UTF-16BE no BOM   found=0 exit=0  found=1 exit=1
```

**Two different routes to the same false all-clear**, which is why fixing one
is not enough:

- **With a byte-order mark**, `read_to_string` fails — `0xFF` is not valid
  UTF-8 — and the error arm was `Err(_) => return Ok(())`, a comment-labelled
  "Skip binary/unreadable files".
- **Without one it succeeds.** UTF-16LE ASCII *is* valid UTF-8: every byte is
  either an ASCII character or `0x00`, and `0x00` is a legal UTF-8 code unit.
  The scanner received `"S\0T\0R\0I\0P\0E\0"` and genuinely found nothing
  in it.

And `1 file scanned` was a lie independent of either: the count came from
`files.len()`, the number of files *found*, set before any was opened.

**What changed**

- UTF-16 is **decoded and scanned**, at both endiannesses, with or without a
  BOM. A UTF-8 BOM is stripped rather than left to become part of the first
  key's name.
- A file this command selected and could not read is **reported, named, and
  given a reason**, counted in `summary.files_unreadable`, and **exits 1**.
  "I could not read it" must not share an exit code with "it is clean".
- `summary.files_scanned` counts files actually read.
- Editor swap files (`.swp`, `.swo`, `.swn`, `.swm`, `.swl`) are no longer
  selected. `.env*` matched `.env.swp`, which is binary — so without this, a
  `.env` left open in vim would fail CI. ⓘ **Not** `.env.bak` or `.env~`: those
  hold the same secrets as the file they copy, and `evnx validate --fix` writes
  the first one.

ⓘ This is the third appearance of one shape — `is_scannable` skipping
`.env.production` (task 23), `validate --fix` reporting "All checks passed" over
a credential it had destroyed, and this. A tool that reports success for work it
did not do is worse than one that fails.

### ⛔ A comment after a closing quote silently lost whole variables

```bash
KEY="value" # a trailing comment
```

Valid in every dotenv implementation there is, and documented on evnx.dev's own
syntax page. It was read as an **unclosed** quote, because `opens_multiline`
asked whether the *line* ended with a quote rather than where the quote closed.
The parser then scanned forward for a quote it had already passed.

**What happened next depended entirely on the following lines**, which is why
this was reported as a loud failure and is worse than that:

```text
A="x" # c          A="x" # c
B=plain            B="y"
  → exit 2           → ONE variable, exit 0
```

The second shape is the common one — most `.env` files have more than one
quoted value. The runaway value closed on the next quote it found, swallowing
every line in between, and the parse *succeeded*:

```console
$ cat .env
A="a" # c
B=plain
C="c"
D=plain
E="e"
$ evnx convert --to json | jq 'length'
3                                        # five variables in the file
$ echo $?
0
```

`convert`, `cloud push`, `cloud run` and `cloud export` all read through this
parser, so a push could upload a subset of a vault and a `cloud run` child
could start with part of its environment missing — in both cases silently.

ⓘ `scan` still found a secret in a swallowed line, because its detectors read
values and the secret had become part of one. It attributed it to the wrong
variable.

**What changed**

- `closing_quote_index` finds where the quote closes. `opens_multiline` and the
  value parser both use it, so the two cannot disagree about whether a value is
  complete.
- A `#` after a closing quote is a comment, in every quoting form. A `#` inside
  the quotes is still part of the value — the closing quote is what ends it.
- Anything else after a closing quote is **an error naming what it found**,
  rather than silently discarded: dropping it would hide a missing quote or a
  stray paste.
- A genuinely unterminated quote is still an error, and real multiline values
  still span lines.

⚠️ evnx.dev documented `KEY="value # here"   # kept` as yielding
`value # here`. It yielded this page's own annotation text appended to the
value. The documentation described the intent correctly and the code did not
implement it — the same shape as `diff`'s `is_sensitive_key`, which listed
`ADMIN_PWD` and `STRIPE_APIKEY` as redacted and matched neither.

### ⛔ `validate --fix` still destroyed real values, by a second route

The weak-secret gate added earlier in this release stopped `--fix` overwriting a
credential it judged *weak*. It left the **placeholder** path doing the same
damage, and that path reaches values the first one never would:

```console
$ cat .env
DB_PASSWORD=Tr0ub4dor<3horse
DATABASE_URL=postgres://u:pw@db/todos
MAIL_FROM=ops@acme.example.corp

$ evnx validate --fix
  • DB_PASSWORD: "Tr0ub4dor<3horse" → 3a7567fbe982f2bb…
  • DATABASE_URL: "postgres://u:pw@db/todos" → https://example.com
  • MAIL_FROM: "ops@acme.example.corp" → user@example.com
```

Three real values, replaced. The passphrase matched on `<`, the database URL on
`todo`, the sender address on `example` — every placeholder pattern was a
**substring** test, and `--fix` acts on its verdict by overwriting the value.
The old value was printed too, because the redaction asks the same broken
question.

**What changed**

- The patterns are **anchored**. `<…>` counts only when it wraps the whole
  value; `your_`, `change_me`, `todo`, `xxx` and the rest count as a prefix or
  as the whole value, never as a fragment inside one.
- A host under an RFC 2606 documentation domain (`example.com` and friends) is
  a placeholder — that is what those domains are for — so
  `https://api.example.com/v1` is caught while `postgres://u:p@db/example_db`
  is not.
- **`--fix` no longer replaces URLs or emails at all.** `https://example.com` is
  not anyone's database, and substituting one placeholder for a less
  informative one is the no-op this module already declined to do for `DB_NAME`.
  `PORT` keeps its `8080`, which is a working default rather than a stand-in.

ⓘ This is the same defect as the weak-secret one, found by an external review
that read the code without running it. Fixing one path and not the other is why
it survived.

### ⛔ The PyPI wheels still had no cloud commands — the fix never ran

`#111` set `--all-features` in `python-publish.yml` to put `auth`, `vault`,
`cloud` and `org` into the PyPI wheels. It could not work.

**`#` is not a comment inside `args: >-`.** A YAML folded block scalar is a
string, so the explanatory comment block was argument text. Parsing the file
returns:

```text
--release --out dist --find-interpreter --strip # ⚠️ --all-features, NOT --features full. # …
```

maturin-action writes that into a shell script on Linux, so everything from the
first `#` was dropped — taking `--strip` and `--all-features` with it.

And `pyproject.toml` carried a **second** switch, `features = ["full"]`, which
excludes `cloud` and applies on top of whatever the workflow passes. Fixing
either alone would have changed nothing.

**What changed**

- Every comment moved out of all three `args:` scalars, to above `with:`.
- `pyproject.toml` is `features = ["full", "cloud"]`.
- A release step installs the built wheel and **asserts the commands are
  there**. Verified non-vacuous: it passes on the corrected build and fails on
  exactly the build that shipped for five releases.

⚠️ Every release check before this read `evnx --version`, which is identical
with or without the feature. That is why five releases went green over a wheel
missing its headline feature.

### ⛔ Secrets written world-readable, and `--help` printing the token

Two small leaks from the external review, both reaching real credentials.

**`write_secure` applied `0600` only when it created the file.** `.mode()` on
`OpenOptions` is consulted on creation and ignored otherwise, so writing over an
existing file kept that file's permissions:

```text
new file                → 600
pre-existing file at    → 644 (before)
after write_secure      → 644   ⛔ secrets world-readable
via symlink → target is → 644   ⛔ wrote through the symlink
```

Restoring a backup over a `.env` that already exists is the normal case, not the
edge case — so the decrypted secrets were readable by every account on the
machine, under messages that said "written 0600".

`set_permissions` now runs **after** opening, which applies to the file that is
open rather than only to one being created. `O_NOFOLLOW` makes a symlink where a
`.env` was expected an error rather than a write to wherever it points.

**"0600" was printed on Windows, where nothing sets a mode.** The Windows branch
of `write_secure` is a plain `fs::write`, while `cloud/sync.rs` and
`cloud/export.rs` printed the literal string regardless of platform. Both now
say what actually happened — "mode 0600" on unix, "inherited from the parent
directory" elsewhere.

**`--help` printed the GitHub token.** clap prints the current value of an
`env =` fallback by default, so with `GITHUB_TOKEN` exported — which is how CI
runs — `evnx migrate --help` printed `[env: GITHUB_TOKEN=<the real token>]`.
It now prints `[env: GITHUB_TOKEN]`: the variable stays discoverable, its value
does not.

ⓘ `libc` is a new direct dependency, unix-only, for `O_NOFOLLOW`. It adds
nothing to the build — it was already compiled in through `console`,
`indicatif` and `dialoguer`.

### ⚠️ The cloud guarantee was overstated — corrected

The README said the server holds no key that can open your data "not with full
database access, not with a court order, **not after a breach**". The last one
does not hold.

`unwrap_vault_key` decides how to open a vault key from two fields the *server*
sends: both absent means "your own copy, under your master key", both present
means "shared with you, under the hybrid wrap". The hybrid wrap is not
authenticated to a sender and needs only the recipient's public keys, which the
server already holds. Nothing local records which vaults you own, so the client
cannot tell a genuine share from a fabricated one.

So an attacker with database write access can set those fields on a vault **you
created and never shared**, to a wrap of a key they chose, and read what you push
next.

**What still holds**, and is what the claim should have said: data already stored
stays closed. A database dump, a court order against stored data, a passive
observer — none of them yield plaintext, because the key never leaves your
machine. What a compromised server can influence is what *future* pushes are
encrypted under.

⚠️ `SECURITY.md` already carried this limit but scoped it to "the moment you
share". That was too narrow for the same reason: the client cannot tell. Both
documents now say the same thing, and cloud sync is labelled **beta**.

ⓘ No code change. This is the claim catching up with the code — the fix needs
signed wraps plus locally pinned identities, and a server change. The offline
commands are unaffected.

### ⛔ Converter output ran code, or did not parse

Every format emitter escaped values inline, and each got it differently wrong.
Measured against the shipped 0.9.0 converter with
`CMD_SUB='$(touch /tmp/evnx-pwned)'` in a `.env`:

```console
$ evnx convert --to shell > out.sh
$ sh -n out.sh
out.sh: 13: Syntax error: Unterminated quoted string
$ . ./out.sh                 # the file this command tells you to source
/tmp/evnx-pwned              ← created
$ echo "$DOLLAR"             # the value was pa$sw0rd
                             ← empty
```

- **shell** escaped `"` and nothing else, inside double quotes — so `$(…)`,
  backticks and `$VAR` stayed live, and a value ending in `\` swallowed the
  closing quote and broke every line after it.
- **Kubernetes** and **Compose** quoted nothing, so YAML typed the values by
  shape: `PORT: 8080` an integer and `BOOL: true` a boolean, both of which
  kubectl rejects under `stringData:`; `key: value` became a mapping; `#`
  started a comment; and `\n---\n` started a second manifest. The output did not
  parse as YAML at all.
- **Terraform** escaped `\` and `"` but left `${` and `%{`, HCL's interpolation
  and directive markers.
- **Azure** and **GCP** wrapped values in `'…'` by hand, so a single quote in a
  value closed the quoting and the rest became shell. Verified with a stub `az`
  on `PATH`: a `.env` value **injected and ran a command**. GCP additionally used
  `echo` without `-n`, appending a newline to every secret it pushed.
- **`template`** passed values to `Regex::replace_all` as a replacement string,
  where `$` names a capture group: `DB_PASSWORD=pa$sw0rd` rendered as `pa`.
  Substituted values were also re-scanned, so a value containing `{{OTHER}}`
  expanded or not depending on key order.

⚠️ `cloud export --to shell` reaches the same converters, and there the values
come from **other vault members** — so a hostile value is reachable by someone
who is not the person running the command.

**What changed**

One module, `formats::quoting`, with one function per format:
`shell_single` (POSIX single quotes, `'\''` for an embedded quote),
`yaml_scalar` (always a quoted scalar, so a value is always a string), and
`hcl_string` (escapes `${` and `%{` as `$${` and `%%{`). Every emitter calls it.
`template` substitutes in a single pass with `NoExpand`, so `$` is literal and a
substituted value is never re-scanned.

ⓘ The tests feed each format to its real consumer — `sh -n` and `sh` for the
shell script, a YAML parser for Kubernetes and Compose, stub `az` and `gcloud`
binaries that record exactly what they were handed. An escape list reviewed by
eye passes review; output handed to `sh` does not. Reverting all six emitters
fails all five.

ⓘ `heroku.rs` already escaped correctly and was the model.

### ⛔ A `#` inside a value truncated it, and `--inspect` printed private keys

**`#` is a comment only when whitespace precedes it.** It used to end the value
wherever it appeared, so four shapes people actually have were silently
destroyed:

```text
COLOR=#ff0000           →  ""
PW=Tr0ub4dor#3          →  "Tr0ub4dor"
URL=https://x/#/route   →  "https://x/"
TAG=v1.0#rc1            →  "v1.0"
```

⚠️ **The ecosystem is split on this, and following Compose is a deliberate
choice.** Measured rather than assumed — Node dotenv 17 via `dotenv.parse`,
Compose via `docker compose config`:

| | evnx (before) | Node dotenv 17 | Docker Compose |
|---|---|---|---|
| `PW=Tr0ub4dor#3` | `Tr0ub4dor` | `Tr0ub4dor` | `Tr0ub4dor#3` |
| `COLOR=#ff0000` | `` | `` | `#ff0000` |
| `A=v # comment` | `v` | `v` | `v` |

They agree only on the case that really is a comment. evnx now follows Compose,
for two reasons: the other rule's failure mode is silent destruction of a
password or a URL, and `evnx convert --to docker` would otherwise emit a file
Compose reads differently from the `.env` it came from.

ⓘ A `#` at the start of a value is literal, so there is no way to write an empty
value followed by a comment. That is Compose's behaviour verbatim.

**`restore --inspect` printed private-key bodies.** It had its own line
splitter, which took everything before the first `=` and, failing that, "the
whole line". A multiline value has no `=` on its continuation lines:

```text
Variables in this backup (names only — values never shown):
  APP
  TLS_KEY
  MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggS…SUPERSECRET1   ← the key
  -----END PRIVATE KEY-----"
  PORT
✓ 6 variable(s) found                                     ← there are 3
```

It now uses `core::parser`, which is the thing that knows what a multiline value
is. The comment above it said it was "kept deliberately simple to avoid false
negatives on edge cases"; the edge case it could not see was the one that leaks.

### ⛔ `scan` missed secrets a parsed `.env` cannot show it

A parsed `.env` is a map, and a map is smaller than the file. Three kinds of
secret fell into the gap, and each reported **0 findings, exit 0**:

```text
# STRIPE_SECRET_KEY=sk_live_…      a comment to the parser, a leak to everyone else
K=sk_live_…                        shadowed by the line below, so never scanned
K=
STRIPE_SECRET_KEY: sk_live_…       in .env.yaml, scanned by neither path
```

- **A commented-out assignment** was skipped by both the parser and the raw
  fallback. A key pasted into a file and then commented out is still a key that
  was committed.
- **A shadowed duplicate** survives only as the last value, so `K=sk_live_…`
  followed by `K=` left the real value unseen. Blanking it on the line below does
  not unpublish it.
- **`.env.yaml` and `.env.json`** were handed to the dotenv parser, which failed,
  and the raw fallback only looked at lines containing `=` — so `KEY: value` was
  scanned by neither.

**And the same test produced false positives.** Whether a file was "dotenv" was
decided by `path.contains(".env")`, which is true of every file inside a
directory called `.env` — and a Python virtualenv is routinely named exactly
that. Library code in `site-packages` became findings: `self.password = password`
in somebody else's package was reported as a secret.

**What changed**

- Dotenv-ness is decided by **file name**, not path, and a known structured
  extension (`.yaml`, `.json`, `.toml`, …) means the content detectors read it
  as text instead.
- After a successful parse, the lines the parse did not account for — comments
  and shadowed duplicates — are scanned too. ⓘ Only those lines, so a secret is
  never reported twice.
- The raw fallback no longer skips `#` lines.

Measured against 0.9.0: each of the three went 0 → 1 finding, the virtualenv
case went 2 → 0, and an ordinary file with one real secret still reports exactly
one.

### ⛔ The pre-commit hook blocked every commit, for everyone

`evnx-scan` ran with `pass_filenames: false` and `always_run: true`, so it
scanned the **whole working tree** on every commit — including the real,
gitignored `.env` that any working setup necessarily has. The scan finds the
secrets in it, exits 1, and the commit is refused.

Every commit. For anyone with a functioning local environment. Over a file that
was never going to be committed.

```text
staged: .gitignore, app.conf        (nothing secret)
old hook  →  exit 1   commit BLOCKED
new hook  →  exit 0   commit allowed
```

Both scan hooks now take the staged paths, which is what a pre-commit hook is
for. A gitignored `.env` is never staged, so the obstruction disappears — and a
secret that *is* staged still blocks the commit, verified.

ⓘ No `files:` filter. `scan`'s detectors are content-based and already find keys
in `.py`, `.yml` and `.md`, so narrowing to `.env*` would lose the case where
one is pasted into source.

⚠️ **Not changed: `.env.example` is still excluded**, and the review asked for
that to be removed. The reasoning given was "placeholders are already
suppressed" — they are not. An ordinary template produces five findings
(`sk_test_xxxx`, `AKIAIOSFODNN7EXAMPLE`, `ghp_xxxx`, `<your-api-key>`, and a
`user:password@` connection string), and `is_placeholder` recognises two of
seven such values. Removing the exclusion today would turn every project's hook
red on a correct template, which is worse than the miss it fixes. The
prerequisite is placeholder detection that understands template values; the
decision and its evidence are recorded in the test that guards the exclusion.

### ⛔ A branch name could publish to npm, and run commands while doing it

`npm-publish.yml` is triggered by `workflow_run: workflows: ["Release"]`, and
that filter matches a workflow **name**. A name is not unique, not immutable
and not confined to the default branch, so any workflow file in the repository
called `Release` fired it — in a job that GitHub documents as able to "access
secrets and write tokens, even if the previous workflow was not". That job
holds `NPM_TOKEN` and publishes six packages with provenance.

- **The trigger is now identified by file path.** `workflow_run.path` must be
  `.github/workflows/release.yml`; the upstream run must belong to this
  repository rather than a fork, and must have been a `push`.

- **A branch was indistinguishable from a tag.** `head_branch` carries a ref's
  short name and nothing else — it reads `v0.9.0` both for the tag `v0.9.0`
  and for a *branch* called `v0.9.0`, and the old `^v[0-9]` test could not
  tell them apart. The tag is now resolved through the refs API, where a
  branch 404s, and must still dereference to the commit the release run built,
  so a tag moved afterwards cannot publish binaries from one commit under a
  tag that names another.

- **The ref name was interpolated into a shell script.** `HEAD_BRANCH="${{ … }}"`
  substitutes into the script *text*, and git permits `"`, `$`, `;`, `|`, `&`
  and backticks in a ref name — only whitespace is excluded. The branch
  `v1.0.0";id>/tmp/pwned;"` is a legal ref, and it executed the `id` and then
  reported `version: 1.0.0`, so nothing in the log looked wrong. Every
  untrusted value now reaches the script as an environment variable, and no
  `${{ }}` remains inside any `run:` block in the file.

- **`workflow_dispatch` validated nothing.** It took a `run_id` and a `version`
  by hand and published whatever that run had uploaded. The run must now be a
  successful `release.yml` run in this repository, and the version must be
  semver — it is written into six `package.json` files.

⚠️ Every release tag in this repository is **annotated**, so the dereference
is the path that runs on an ordinary release rather than an edge case. The new
check was replayed against v0.6.0, v0.7.0, v0.8.0 and v0.9.0 using the live
API: all four resolve to exactly the commit their release run built, and all
four pass.

### ⚠️ The published crate carried the CI workflows

`cargo package` shipped all five `.github/workflows/*.yml` files inside the
`evnx` crate. They are not part of the library, the binary, or anything that
builds — but crates.io versions are immutable, so a workflow-only fix changed
the crate's contents and therefore could not be released by moving a tag. That
cost two version numbers before anyone noticed.

`exclude = [".github/**"]` drops them: **185 files → 180**, 708.0 KiB → 687.9
KiB compressed. Verified with a full `cargo package`, which runs the same
verification build `cargo publish` does.

⚠️ `exclude` rather than `include`, deliberately. `include` is a whitelist, so a
file added to the crate later would be silently left out of the package until
someone thought to list it. `exclude` fails in the harmless direction.

ⓘ Nothing excluded is reachable from a build — the crate's only `include_str!`
targets are `src/docs.rs`, `src/cloud/device.rs` and `src/assets/schema.json`,
all under `src/`. `tests/`, `docs/`, `scripts/`, `benches/`, `Dockerfile`,
`pyproject.toml` and `pypi-readme.md` still ship; `tests/` alone is 440 KB, so
there is more to trim here if it turns out to be worth trimming.

### ⚠️ Every GitHub Action ran from a tag anyone could move

All 59 `uses:` entries across the five workflows referenced a mutable tag, and
three referenced a **branch** — `dtolnay/rust-toolchain@stable`, the same at
`@master`, and `pypa/gh-action-pypi-publish@release/v1`. A branch moves on
every push. The highest-leverage of them sits in `release.yml`'s build job,
which compiles every binary for every one of the nine install channels.

All 59 are now pinned to a full commit SHA with the version beside them, and
`.github/dependabot.yml` keeps them current — pinning without that trades a
moving tag for being frozen on October 2026, security fixes included.

⚠️ `dtolnay/rust-toolchain` needed checking rather than pinning blind: it
selects the toolchain **by ref name**. Pinning `@stable` is safe because the
`action.yml` at that commit carries `default: stable`, so rustup still resolves
the current stable at run time — only the action's code is frozen. `@master`
has `toolchain` as **required with no default**, and is safe here only because
that one call site already passes `toolchain:` explicitly.

`ci.yml` had no `permissions:` block, so the repository default applied to a
workflow that only builds and tests; it is now `contents: read`.

### ⚠️ `install.sh` could skip checksum verification three ways

The installer verified downloads only when it happened to be able to, and said
nothing when it could not.

- **A missing checksum file was silently fine.** The whole check sat inside
  `if curl …; then`, so a 404 skipped verification and continued.
- ⛔ **macOS never verified a download, ever.** The check required `sha256sum`,
  which macOS does not ship — the tool there is `shasum -a 256` — and there was
  no `else`. Every macOS install since the script was written was unverified.
- ⛔ **On mismatch it asked "Continue anyway?"** via `read -p`. The documented
  way to run this script is `curl -sSL … | bash`, which puts the *script* on
  stdin, so `read` consumes the script's own remaining text rather than waiting
  for anybody. The prompt could not work in the path the README publishes.

All three now stop the install. ⓘ The checksum is still only as good as the
release that published it — this closes the gap between "we fetched a checksum"
and "we checked it", not the one above it.

⚠️ `set -e` became `set -euo pipefail`, and **two pipelines had to change for
it**, which is why this is not the one-line fix it looks like. musl's `ldd`
prints its version to stderr and exits 1 — that is what the existing `2>&1` was
for — so under `pipefail` the musl check inherited that failure, musl went
undetected, and Alpine users were handed the glibc binary. Reproduced, and
fixed with an explicit `|| true` inside the pipeline. A network failure during
the release lookup likewise aborted the installer with curl's exit code and no
message; those two substitutions now fall through to the existing
"that doesn't look like a version" error.

### ⛔ Security — a URL that looked like localhost sent your token elsewhere

`--server http://localhost:80@evil.com` was accepted, and the request went to
**evil.com** carrying an access token and a refresh token valid for thirty
days, in clear.

`localhost:80` there is *userinfo*, not a host — RFC 3986 is
`authority = [ userinfo "@" ] host [ ":" port ]`, so an `@` has to be resolved
before a colon means anything. `canonical_server` split on the last `:` and
read `localhost`, which passed the loopback test that allows plain `http://`
for local development. The guard was not missing; it was reading the wrong
half of the string.

⚠️ **Userinfo is now refused, not stripped.** This value is written to
`config.toml` and used as the credential-store key, so quietly accepting it
would store a password on disk as a side effect — and leave a URL whose
visible text disagrees with where the request goes.

Three more defects in the same five lines, all found while reproducing the
first and none of them reported:

- ⛔ **`http://[::1]` was refused.** `"[::1]".rsplit_once(':')` is
  `("[:", "1]")`, which trims to `:` — not a loopback address. With a port it
  happened to work, so IPv6 localhost worked or not depending on whether you
  typed one.
- **`http://localhost:abc` was accepted**, with `abc` taken as a port.
- **`http://a:b:c` was accepted**, with `a:b` taken as a host.

The authority is now parsed properly: userinfo refused, bracketed IPv6
literals understood with and without a port, and a port required to be a
number in range.

ⓘ The guard still does its job and still allows what it should: `http://evil.com`
and `http://localhost.evil.com` are refused, `http://localhost`,
`http://127.0.0.1:8080`, `http://[::1]` and `https://api.evnx.dev` are not.
13 cases checked against the built binary, and the five new tests were run
against the old parser first — all five fail there.

### ⚠️ `scan --ignore-placeholders` did nothing

The flag was wired end to end and reached the right line — and could never
fire. It asked `is_placeholder`, which answers a different question.

`is_placeholder` decides whether `validate --fix` may **overwrite** a value,
and its patterns were anchored deliberately so it could not destroy a real
credential. That makes it recognise a value that *is* a placeholder and never
one that *contains* a placeholder — but a scanner only ever sees values a
detector has already matched, like `sk_live_xxxxxxxxxxxxxxxxxxxxxxxx`. The
strict answer always won, so findings were identical with and without the
flag.

Scanning now uses its own predicate. The two questions are not the same one:

- *"Is this safe to overwrite?"* — a false positive destroys a working
  credential. Must stay strict.
- *"Did you ask to hide findings like this?"* — a false positive hides one
  line the reader explicitly asked to hide.

⛔ The strict predicate is unchanged, and a test now asserts it still refuses
to call these values safe. Merging the two back together re-opens the data
loss this release already fixed twice.

ⓘ Matching is on whole tokens, so `latest` is not `test` and
`exampleshire_road_token_4418` is not a placeholder — the first version used a
prefix match and claimed it was, caught by its own test before it shipped. A
run of four or more identical characters counts, because that is how a value
gets redacted by hand.

### ⛔ Security — `--compromised` wrote a file the leaked password opens

`evnx auth rotate-master-password --compromised` says the old password is in
someone else's hands. It then wrote a recovery bundle to disk and printed the
path. The bundle's own note explains the problem: *"nothing here opens without
the OLD master password"*.

So in the one case the flag exists for, the previous vault wraps, the previous
argon2 salt and the previous sealed private key — until that moment held only
on a server an attacker would have to breach separately — were written to the
local filesystem, openable with the password just declared compromised.

It also contradicted the flag's own help, which promised **no undo**. The
server-side undo was skipped; a local one was written anyway.

`--compromised` now writes no recovery file and says so, including that there
is no way back if the new password is lost.

⚠️ **And the flag's name over-promised, which is now stated rather than
implied.** It rotates the *password*. The identity keypair and every vault key
are re-wrapped under the new password, not replaced — that invariant is
asserted on every rotation and is not a bug — so anyone who already extracted
a vault key still has it. The help and the command's output now say this and
point at `evnx vault rekey`, which is what actually leaves an old key with
nothing to open.

### ⛔ `scan` missed whole classes of credential, and invented one

Six credentials in a `.env`, before: **four** findings, every one of them the
generic *"High-entropy string · low"*, and two missed entirely. After: six
findings, each named, each at its own severity.

**Five formats had no rule at all.**

| | | |
|---|---|---|
| AWS temporary key (`ASIA…`) | high | not found at all — and it is what every CI role assumption hands out, so it is the form most likely to reach a `.env` by accident |
| GitHub fine-grained PAT (`github_pat_…`) | high | the existing rule covers `ghp_`/`gho_`/`ghu_`/`ghs_` only, and GitHub now steers everyone to this one |
| Slack (`xox[abprs]-`, `xapp-`) | high | |
| Google API key (`AIza…`) | medium | ⓘ medium deliberately: a browser key for Maps or Places is meant to ship in a page and is restricted by referrer, not secrecy |
| JSON Web Token | medium | ⓘ also medium: plenty of JWTs are public ID tokens |

**And the variable name was being judged by two different rules.**

`scan` had a name check of its own with five hardcoded words, while
`is_sensitive_key` sat next to it carrying `PASSWD`, `PWD`, `PRIVATE_KEY`,
`AUTH`, `CREDENTIAL` and a `*_KEY` suffix rule — maintained for `diff`'s
redaction and never consulted here. Two lists for one question drift, and
these had:

- ⛔ `DB_PASSWD=hunter2hunter2hunter2` was **missed**. No provider prefix, and
  a repeating value scores far below the entropy fallback's threshold.
- ⛔ `QUOTA_FREE_TOKENS=2` was **reported as a medium-severity secret**,
  because the name contains `TOKENS`. That one was live in evnx-server's own
  `.env.example`.

There is now one rule. It uses the list that was already being maintained, and
it has the guards the old check lacked: a value must be at least 8 characters
and must not be a variable reference (`${…}`, `%…%`), a filesystem path, or a
bare number — each of those a false positive this produced on a real file.

⚠️ **`PUBLIC_KEY` is excluded, `NEXT_PUBLIC_TOKEN` is not**, and the
distinction is adjacency rather than the word. A public key is the one piece of
key material meant to be published; a token behind a prefix that says
"published" is the clearest finding there is. The first version of this
exclusion got that wrong in both directions and the existing `g11` tests caught
it — they encode exactly this pair.

### ⚠️ Breaking

**`evnx scan` now exits 1 when it could not read a file**, where it exited 0.

If your `.env` is UTF-16 it is now *scanned*, so a secret in it fails the build
as it always should have. If a selected file cannot be decoded at all — a
Latin-1 `.env`, say — the scan reports it and exits 1, because "I could not read
it" must not share an exit code with "it is clean".

A build that starts failing here was already not being scanned. ⓘ Editor swap
files (`.env.swp` and friends) are excluded rather than reported, so a `.env`
open in vim does not fail anything. If you need the old behaviour while you
re-save files as UTF-8, `--exit-zero` still applies.

**`scan`'s JSON gained `summary.files_unreadable` and a top-level `unreadable`
array.** Additive. ⚠️ But `summary.files_scanned` now counts files actually
read, where it previously counted files *found* — so a pipeline asserting
`summary.total == 0` should assert `summary.files_unreadable == 0` beside it.
The first says no secrets were found; only both together say there are none.

**`diff --format json` changed shape.** `DiffItem` now carries `example_value`
and `env_value` as nullable, holding only what is safe to print, plus
`values_redacted: bool`. The `example_value_redacted` and `env_value_redacted`
fields are **gone** — holding the raw value beside the mask was the defect,
since every consumer had to know to prefer the second and the JSON formatter did
not. Read `values_redacted` to tell a mask from a literal, and `null` to tell a
withheld value from an absent key.

**`validate --fix` on a project with a short real credential now exits 1 where
it exited 0.** The finding is no longer silently resolved — it was exiting 0 by
destroying the thing it was reporting. Pass `--rotate-weak-secrets` for the old
behaviour, or `--exit-zero` if the exit code is what your pipeline depends on.

**`FixApplied` gained `old_value_withheld`** in `--format json`, and
`old_value` is now `null` for a withheld value. Additive; a reader that ignores
unknown fields is unaffected.

---

## [0.9.0] - 2026-10-05

**One pull request, and the thing it adds is money.** `evnx org` creates an
organisation, invites people into it, and assigns seats — and a seat is what makes
a paid plan's limits apply to somebody. With the server side deployed, a team can
now buy a plan and see their limits change.

### ⛔ The one thing to understand before using it

**An organisation does not give anyone access to a vault.** It owns a plan and a
set of seats; a seat decides which plan's *limits* apply to its holder. That is
the whole feature.

The server has never been able to wrap a vault key — the zero-knowledge guarantee
is that it holds only ciphertext — so there is no mechanism by which membership
could produce access, and no column in the schema that could express it. Sharing
stays a deliberate act by someone who already holds the key:

```bash
evnx vault share app/production --with colleague@example.com --role developer
```

`evnx org invite` says this in its own output, not only in the guide. "I added
them to the org, why can't they see the vault?" is the first question this feature
will produce, and a guide is read once while output is read at the moment the
wrong assumption forms.

### ⚠️ This needs a server that has the organisation endpoints

`api.evnx.dev` has had them since 2026-10-05, with migrations 010–012 applied. A
self-hosted deployment needs evnx-server at that commit or later; against an older
one, `evnx org` returns 404 from every subcommand.

Nothing else in this release touches the server.

### Why this is 0.9.0 and not 0.8.1

Fifteen new subcommands under a new top-level `org`, behind the existing `cloud`
feature. In `0.x` semver puts new functionality in the minor position. Nothing
that already shipped changes behaviour, and no output format moves — so unlike
0.8.0, there is no compatibility note to read.

### Added

- **`evnx org` — organisations, seats, and what the plan is.** Fifteen
  subcommands:

  ```bash
  evnx org create "Acme Corp" --slug acme
  evnx org list · members · invite · invites · uninvite · accept
  evnx org seats · billing · assign · release · role · remove · leave · delete
  ```

  Every one takes `--org <slug>`. You can omit it when you belong to exactly one
  organisation; with several, omitting it is an error that **lists them rather
  than guessing** — picking wrong here bills the wrong company.

  ⚠️ **Organisation commands need a real sign-in.** An `evnx_tok_` API token is
  refused with 403: inviting people and assigning seats changes what the account
  is billed, so it takes a login rather than a CI credential. Same reasoning as
  `evnx auth token`.

- **A seat is a plan change for somebody who is not at the keyboard.**
  `evnx org assign` and `evnx org release` move another person's limits, and both
  name whose and to what. Without that they watch their quota move with no
  explanation, and the person who caused it never knew they had.

  ⚠️ Releasing a seat can leave someone **over** their new limit. Nothing is
  deleted and nothing becomes unreadable — a vault over the limit stays readable
  and exportable, and the next *push* to it is what gets refused. Over-quota and
  readable is the only humane state, and it was chosen deliberately rather than
  discovered.

- **`evnx org billing` — what the organisation is on, and until when.**

  ```
    acme — team plan
    seats 7 / 10
    renews 2026-11-03
  ```

  Read-only. Buying a plan, changing the seat count and updating a card happen at
  [app.evnx.dev/billing](https://app.evnx.dev/billing/) — a card form does not
  belong in a terminal, and the payment provider is the merchant of record for the
  transaction either way. What a terminal *is* the right place for is the question
  this answers: why are my limits what they are, and is that about to change?

  ⚠️ **"ends" and "renews" are the same date and the opposite event.** A cancelled
  subscription keeps running to the end of the period already paid for, so the
  provider reports it as *active* with a pending change attached. Reading the
  status alone would print `renews 2026-11-03` for a plan that ends on 2026-11-03
  — and the natural response to seeing that is to cancel again through your bank.
  Both states are distinguished:

  ```
    renews 2026-11-03                  ← it will be charged again
    ! this plan ENDS on 2026-11-03     ← it will not
  ```

  A failed payment prints *"your limits have not changed yet"*. The "yet" is
  load-bearing: without it people assume they are already locked out and stop
  working.

- **One seat per person, by construction.** A seat can be held in exactly one
  organisation at a time, and the database makes a second one unstorable.
  Directory membership is unconstrained, so a contractor at two companies is
  representable — two entries, one seat. Without that rule, "which plan applies to
  this account?" has no well-defined answer, and anyone with an enterprise
  organisation could raise the limits of every account they invited.

- **`evnx org seats --set` asks before it tries.** Once a subscription exists the
  seat count is what the invoice is computed from, so the server refuses the local
  route. The CLI checks first and names the next step rather than surfacing a
  rejection.

- **Roles have no peers.** You may only act on a rank you outrank, so an admin
  cannot create another admin, and `owner` cannot be granted at all — an
  organisation has exactly one, set at creation, never handed out by a forwardable
  invitation link.

### Documentation

- [`evnx org`](https://docs.evnx.dev/cli/commands/org) and
  [Billing](https://docs.evnx.dev/cli/reference/billing-and-plans) are published.
  Both were written during development and held unlisted until the commands they
  describe shipped — a published guide for a command nobody has is worse than no
  guide.

---

## [0.8.0] - 2026-10-03

**Nine pull requests, and the answer to "what happens when we want to leave?" is
now a command.** `evnx cloud export` writes every version of a vault to disk,
decrypted — the escape hatch that was previously a script you had to write
yourself. Alongside it: a vault key can be rotated without removing anyone, an
account can list where it has been signed in from, and `evnx scan` now finds a
secret pasted inside a multiline value, which it has been missing since the
beginning.

### Why this is 0.8.0 and not 0.7.1

Two changes alter the output of commands that already shipped, and in `0.x` semver
puts a compatibility change in the minor position:

- **A secret inside a multiline value is now a finding.** A repository with a live
  key inside a PEM block scanned clean, exit 0; it now fails
  `evnx scan --severity high` where it passed. Same class as the
  connection-string rule in 0.7.0. See *Fixed*.
- **`evnx convert` writes different bytes than it used to — by one.** Every one of
  the 29 format spellings produced different output on stdout than with
  `--output`, and 22 of them printed a trailing blank line. Both are fixed, which
  means a CI job diffing `convert` output will see a one-byte trailing-whitespace
  change once. No converted value changes. See *Fixed*.

⚠️ **`evnx auth devices` needs a server that has `GET /auth/devices`.** `api.evnx.dev`
has had it since 2026-10-02; a self-hosted deployment needs evnx-server at the
commit that adds it. Nothing else here requires a server change — `vault rekey`
reuses the re-key endpoint `vault revoke` already called, and `auth status` treats
a 404 from `/auth/usage` as "this server is older" rather than as a failure.

### Added

- **`evnx cloud export` — the way out.** Writes every version of a vault to disk,
  decrypted, with a manifest naming which one was current. The question a team asks
  before adopting a secrets vault is what happens when they want to leave, and an
  escape hatch nobody can demonstrate is not believed.

  ⚠️ **This is the one evnx command whose purpose is to put plaintext secrets on a
  filesystem.** `pull` writes one file and asks first; `run` writes none at all.
  Files land `0600` inside a `0700` directory, which protects them from other users
  on the machine and from nothing else — and the command says so rather than
  leaving it to the docs.

  With no `--to` the files are the exact bytes that were pushed: comments, ordering,
  quoting and multiline values all survive, because `push` encrypts the file
  verbatim. `--to` converts through the same fourteen formats `evnx convert` offers
  and keeps only keys and values. Three refusals land **before** the master password
  is asked for and before a single blob is fetched — an unusable `--to`, a non-empty
  output directory, and a vault with no versions — because a refusal that arrives
  after minutes of decryption teaches people to pass `--force` by reflex.

  Verified against a live server: three versions pushed and exported byte-identical,
  including a multiline PEM private key, and again through the hybrid
  X25519 + ML-KEM-768 wrap from a second account — the path a solo vault never
  exercises. (#101)

- **`evnx vault rekey` — rotate a key without removing anyone.** Rotating a vault key
  and removing a member are separate acts that happened to ship together. Removal
  needs rotation, or a former member keeps a usable key; rotation does not need
  removal, and the cases that call for it most have nobody leaving — a lost laptop
  with a signed-in CLI, a CI runner that held `EVNX_PASSWORD`, an account breach
  where the person stays on the team. Until now the only way to rotate was
  `vault revoke` followed by `vault share`: a worse audit trail, and a window in
  which that person genuinely had no access.

  ⚠️ **A member with no post-quantum sharing key refuses the rotation and is named**,
  rather than being skipped. The server requires the payload to cover every member
  exactly once, so a member left out is a member *removed* — and silently revoking
  someone mid-rotation is the precise failure `vault revoke` exists to make loud.

  The output says what a rotation does **not** do: anyone who could already read a
  version may still hold a copy, and no rotation recalls that. It also says nobody
  is signed out, because a rotation that looked like it logged the team out would be
  reached for less often than it should be. (#95, #96)

- **`evnx auth devices list` and `evnx auth devices disavow`.** A device is a network
  and browser the account has signed in from. The server identifies one by a keyed
  BLAKE3 digest of the client address and user agent, so repeat sign-ins collapse
  into one row without anything being stored that could name a machine.

  ⚠️ **There is no location in any of this and there cannot be.** A hash cannot be
  geolocated — no city, no country, no impossible travel. Those need a raw IP and
  evnx deliberately never stores one. The listing says so in as many words, because
  a device that looks new is very often a phone that reconnected, and the
  alternative is people acting on a threat that is not there. A test scans the
  module's own printed strings for location words, so a later copy edit cannot
  quietly introduce a claim the architecture cannot support.

  ⚠️ **Not the same as `evnx auth sessions`,** and the help text for both now says so.
  A session is a live credential you can revoke; a device is somewhere you have
  signed in from, which may have no session left at all. `disavow` confirms by
  default, because it ends every session on the account including the one running
  the command — and it does not stop at "done": revoking sessions does not protect a
  vault, so it points at `evnx auth rotate-master-password` as the next step. An API
  token is refused with an explanation, because a leaked CI token must not be able
  to enumerate where its owner signs in from. (#103)

- **`evnx auth download-data` — take your data with you.** GDPR Article 20, and the
  other half of `evnx auth delete-account`: being able to take a copy before erasing
  the account. Writes which vaults exist, who can reach them, each version by
  variable **name**, API tokens by name and scope, and this account's own activity.

  ⚠️ **The secrets are not in it and cannot be,** and the command says so twice — in
  `--help` and after a successful download. The server has never held a master
  password, a master key, a vault key or a plaintext value, so there is nothing on
  its side to decrypt with. An export that *did* contain secrets would mean the
  server could read them, so the absence is the product working. The output names
  `evnx cloud pull` as the way to get values.

  The file is written `0600`, permissions set before the bytes so there is no instant
  where it exists with real content under the process umask. No secret value is in
  it, but it carries every vault name, every co-member's email and the variable names
  of every version — the shape of a system without its contents, which is plenty for
  someone deciding what to attack. The body is taken as untyped JSON on purpose: a
  struct would silently drop any field the server adds later, turning "everything we
  hold" into "everything this binary knew about". (#97)

- **`evnx auth status` shows your plan and usage.** `status` already answered "is this
  credential good?"; it now also answers "how much of my plan am I using?" — which
  was previously answerable only by being refused.

  ⚠️ **`used == limit` renders as full, not as one remaining.** The server refuses at
  `count >= limit`, so three of three means the next create is already rejected, and
  a neutral "3 / 3" reads as one left to anyone used to progress bars. One slot
  remaining is warned in yellow — the last moment a warning is still actionable.
  Versions are listed per vault and only for vaults at or near their cap, because
  printing every count would bury the two that matter. (#98)

### Changed

- **`evnx auth export` is `evnx auth download-data`.** Renamed before it ever shipped,
  so no published binary has the old name and nothing breaks.

  ⚠️ **The collision this avoids was a safety problem, not a tidiness one.**
  `evnx cloud export` writes every secret, decrypted, to disk. `evnx auth export`
  writes account metadata and explicitly no secrets. Two commands named `export`
  with opposite risk profiles: someone who learns here that "an export holds no
  secrets" carries that belief to the one that writes plaintext credentials.
  `cloud export` keeps the word because that is what it means everywhere else —
  Doppler, Infisical and 1Password all use "export" for getting secrets out — so the
  metadata command is the one to move. `download-data` also matches the dashboard
  button, which already said "Download your data". The default filename moves with
  it: `evnx-account-data-<date>.json`. (#99)

- **Every documentation URL the binary prints now points at `docs.evnx.dev`.** The
  guides moved to their own host and all 33 URLs in `src/docs.rs` kept pointing at
  `evnx.dev/guides/*`, as did the SARIF `helpUri` that `scan` writes into GitHub
  code scanning results, the README line that crates.io, npm and PyPI all render,
  and the release-notes text.

  ⚠️ **Nothing was broken, which is why nothing caught it.** Every one of those URLs
  301s to the right page, so a reader never noticed and no check failed — the stale
  host was simply compiled into every published binary. `docs.rs` now carries tests
  that pin the host, reject a trailing slash (`docs.evnx.dev` 308s those away, the
  opposite of `app.evnx.dev`), require each `after_help` to link to its own command's
  guide, and read the file's own source so a command added later cannot sit outside
  all of them by being left out of the list.

### Fixed

- **`evnx scan` missed a secret inside a multiline value.** A `.env` holding a live
  Stripe key inside a PEM block reported "No secrets detected", exit 0. The identical
  bytes saved as `.txt` were caught.

  ⚠️ **The `.env` path was the only one that missed it, and it is the path that
  matters.** The published guide sells this command as a pre-commit and CI gate and
  lists "private keys accidentally pasted as values" as something it catches — and a
  private key pasted as a value *is* a multiline PEM block. The guide named the
  failing case as a feature.

  The cause was a second implementation: `scan` was the one command that re-read
  `.env` itself, splitting every line on `=`, and a continuation line has no `=` so
  it was skipped entirely. `core::parser` has handled multiline since the beginning
  and eleven other modules use it. `scan` now parses through the shared parser, so
  the duplicate is gone rather than patched. ⚠️ A file that does not parse falls back
  to the old raw scan rather than being skipped — the old path *does* find a secret
  in a file with an unterminated quote, and a half-pasted value in a broken `.env` is
  exactly where a secret hides.

  Characterised across 15 `.env` shapes before and after: exactly two lines of that
  table changed, both of them the bug. (#100)

- **`evnx convert` wrote different bytes to stdout than to `--output`.** Not for some
  formats — for all 29 spellings, every time, with the file always exactly one byte
  shorter. And 22 of the 29 printed a trailing blank line.

  One bug seen from two sides: a converter decides for itself whether to end in a
  newline, and each write path then applied a different rule on top. Neither column
  was a decision anyone made; both fell out of which function happened to do the
  writing. Normalised in one place, beside the trait whose output it normalises, and
  applied on both paths — add a format and it inherits this.

  ⚠️ **The golden test had enshrined the bug.** `tests/golden/convert.yaml` was
  generated from the buggy output and then asserted as correct, so a test was holding
  it in place. Worth remembering whenever a golden file is created: it locks in
  whatever was true that day, including what was wrong. (#102)

- **A vault named `..` could write outside `--output`.** Found while building
  `cloud export`: a vault named `..` with an empty environment sanitised to exactly
  `..`, and joining that onto the output path writes to its parent. A vault name is
  user-supplied and the server does not constrain it. (#101)

### Internal

- `rotate_vault_key` is extracted from `vault revoke`, which was 305 lines with the
  rotation inlined in its tail, so `rekey` adds no duplicate of it. `remove_user_id`
  is the only thing that makes a rotation a revocation, and the server already
  declared it optional — so a standalone re-key needed no server change at all. (#95)
- `get_converter` moves from `commands/convert.rs` into `formats` and returns an error
  instead of calling `std::process::exit(2)`. The exit is why it could not be reused:
  a library function cannot end the process under a caller mid-way through a long
  job, and no test could ever reach the unknown-format arm, because exiting takes the
  harness with it. `convert` still exits 2, at its own call site, with byte-identical
  output. (#101)
- `pull` splits into `fetch_blob` + `decrypt_version`, kept as two functions so
  pull's existing order survives — it fetches before prompting, so a version that
  does not exist answers 404 rather than wasting the typing. (#101)
- `core::parser` gains `EnvFile::lines` and `parse_content_located`, exposing the
  key-to-line map it already tracked internally. Additive: `parse_content` keeps its
  signature for all 51 existing call sites. (#100)

## [0.7.0] - 2026-09-30

**Thirteen pull requests, and a cloud account is now fully self-service.** A master
password can be changed and the change undone, an account can be deleted, and a
version can be removed from a vault's history — three things the server could
already do and nothing could reach.

### Why this is 0.7.0 and not 0.6.1

Three changes alter behaviour rather than only adding to it, and in `0.x` semver
puts a compatibility change in the minor position:

- **Single-quoted values no longer expand.** `'${FOO}'` was substituted; it is now
  literal, matching shell, dotenv, python-dotenv and godotenv. A file relying on
  expansion inside single quotes **changes meaning with no error**, because both
  forms parse. See *Changed*.
- **A password inside a connection string is now a finding.** A repository whose
  `DATABASE_URL` carries credentials now fails `evnx scan --severity high` where it
  passed. See *Added*.
- **`allow_actual` now does something.** It was parsed and read by nothing; it is
  implemented, and refuses the whole sync with exit 2 when a listed value looks
  like a credential. See *Added*.

One build-level change worth knowing: a new `net` feature gates the HTTP stack, so
`default = []` now means **no network code is compiled in at all** — `scan`,
`validate`, `sync` and `diff` are provably offline rather than offline by
convention. `migrate` and `cloud` both imply `net`, so no existing feature
selection changes meaning.

### Added

- **`evnx auth rotate-master-password` — change your master password.** The master
  key is derived from the password, so changing it re-derives the key and re-wraps
  every vault key under it, on your machine, in the same command. The server
  receives a new verifier and a set of blobs it cannot read.

  ⚠️ **The server cannot tell a correct rotation from random bytes.** That is the
  guarantee, not a gap — it can check a payload is complete, never that it is
  correct. So verification is mandatory rather than a flag: the re-sealed keypair
  is re-opened and its public keys asserted unchanged, every new wrap is unwrapped
  with the new master key and compared byte-for-byte with the key it came from, and
  the wraps and keys are checked to be the same length and in the same order — they
  are zipped, and an order that drifted would wrap each vault with its neighbour's
  key and still verify pairwise. All of it happens before a byte is sent, so a
  failure can say the account is untouched and mean it.

  Before acting it says how many keys move, how many shared-to-you vaults are
  unaffected, that every other device will be signed out, and how many API tokens
  exist. Tokens keep working — a token authenticates, it does not decrypt — but any
  CI holding the master password as a secret fails until that secret is updated.
  It does **not** re-key a vault, so a former member who kept a key is unaffected. (#92)

- **`evnx auth undo-password-change` — and it takes no session, deliberately.**
  Whoever needs it is whoever the change locked out, and a session is exactly what
  they do not have. Zero knowledge means the server can never be the recovery path,
  so the old wraps are written to a `0600` file in the config directory before the
  request: ciphertext, useless without the old password, and a complete offline
  undo that does not depend on the server keeping anything. The server's own
  snapshot window is the second way back. A wrong password and an address with
  nothing to undo answer identically, so the command cannot be used to discover who
  has an account. (#92)

- **`evnx auth delete-account`.** Prompts with the account named, takes the typed
  email back, asks for a 2FA code only when 2FA is enabled, and clears the local
  credential file afterwards. There is deliberately no `--yes`: a boolean flag that
  deletes an account is one shell-history recall away from doing it.
  `--confirm-stdin` covers the scripted case and still has to carry the address. A
  409 is surfaced as the server's own prose, naming the vaults that block the
  deletion, because reducing that to "conflict" strands someone with no idea what
  to do next. (#81)

- **`evnx cloud delete-version`.** The client half of the quota remedy — the
  server's quota refusal has started telling people to remove old versions, and
  nothing could. Shows the version, its key count, size and push date before
  asking, because "delete version 3" is not enough information to consent to.
  `--yes` skips the prompt. The latest version is refused here as well as on the
  server: `pull` and `run` fetch the latest, so removing it would change what every
  consumer of the vault gets without anyone asking for that. (#93)

- **`evnx update` — how to upgrade, and whether to.** evnx ships through nine
  channels and none of them ever told anyone a release had happened. It detects how
  the running binary was installed and prints that channel's upgrade command;
  `--check` asks GitHub Releases whether a newer release exists.

  Two deliberate limitations. **Nothing runs unless asked** — no command makes a
  network request on its own, which is what keeps evnx usable air-gapped and in CI
  with egress rules, and that is worth more than an upgrade nag. And **nothing
  self-replaces**: nine package managers ship evnx, and a binary that overwrites
  itself fights whichever one owns the file. An unrecognised install path prints
  every command rather than guessing, because `brew upgrade` against a binary
  Homebrew never installed fails unhelpfully. `--check` reads GitHub Releases
  because the tag is the only source true by construction — crates.io is published
  by hand, npm publishes on `workflow_run`, winget merges on Microsoft's schedule —
  and the output says so every time it reports a newer version. (#88)

- **`scan` detects a password embedded in a connection string.**
  `DATABASE_URL=postgresql://admin:s3cr3t@db.prod.example.com:5432/main` scanned
  clean, exit 0. The miss was structural: the provider patterns are all prefix
  formats and a connection string has no prefix, the sensitive-key heuristic keys
  off names containing `secret`/`token`/`key` and `DATABASE_URL` has none of them,
  and the entropy fallback scores a whole URL below threshold. A live database
  password in `DATABASE_URL` is among the most common things in a `.env`.

  The character classes exclude `/`, `?` and `#` so a match cannot run past the
  authority into a path — `https://host/users/me@example.com` is an ordinary URL —
  and a colon is required before the `@`, so `postgresql://reader@host/db` reads as
  a bare username. No `action_url`: the place to rotate a database password is the
  database. (#87)

- **`sync`'s `allow_actual` is implemented, and guarded.** It was documented with a
  schema-table row and read by nothing. Refusing it outright would hard-fail every
  config copied from that page, and the documented example uses it only for
  non-secret keys — `APP_NAME`, `ENVIRONMENT`, `LOG_LEVEL` — where the real value
  is the better example and `sync --reverse` then hands a teammate working
  defaults. So it works, and `check_allow_actual` refuses the whole sync before the
  first write, naming every offending key at once with exit 2, when a listed value
  looks like a credential.

  ⚠️ The guard was built on `DetectorRegistry::new()`, which is **not** the set
  `scan` uses: `new()` registers the two heuristics only, and the built-in patterns
  arrive via `with_patterns`, which only the `scan` command called. So it was
  relying on entropy and on sensitive names — enough for `sk_live_…`, not enough
  for a connection string. Found by adding the rule above and watching the guard
  fail to pick it up. It now uses `with_patterns(builtin_rules())`, which is what
  its doc comment already claimed. (#86)

### Fixed

- **A rejected login said "your session has expired".** `evnx auth login` with an
  address that has no account reported "your session has expired. Run
  `evnx auth login` to sign in again" — nonsense twice over, since there was no
  session and it names the command already running. It cannot say "no such
  account": `/srp/init` answers an unknown address with 200 and a fabricated salt
  and verifier, so `/srp/verify` rejects the proof with the same 401 a wrong
  password gets, and the client genuinely does not know. The message now names both
  possibilities, points at the likelier one, and says there is no password reset.

  The test that should have caught this had never reached the code: it asserted
  only `is_err()`, and the mock pinned the status at 200 while switching the body
  to an error document, so the client read an unparseable success. The status now
  comes from the same deterministic function of the exchange as the body, so the
  two cannot disagree. (#83)

- **One credential in a source file counted as two secrets.** A Stripe key in a
  `.py` reported twice — "High-entropy string" low and "Stripe Secret Key (LIVE)"
  high, same value, same line — so `--severity` gates and CI totals saw one high
  *and* one low. The same key in a `.json` gave four. A non-`.env` line is scanned
  by two paths that see different strings, and nothing reconciled them with each
  other. Now reconciled per line, by value, using containment rather than equality:
  the two paths disagree about where a value ends, so equality collapses the neatly
  quoted case and leaves `prefix-sk_live_…-suffix` duplicated, which is the shape
  real code is full of. Two different credentials on one line still stay two
  findings, and order is first-seen so output is stable between runs. (#85)

- **`scan` told you to rotate a key that is meant to be public.** The footer added
  in 0.6.0 said "revoke and reissue those keys — they have been readable by every
  visitor to your site, so treat them as public, not merely leaked",
  unconditionally. A Supabase **anon key** behind `NEXT_PUBLIC_` is designed to ship
  in the browser and is protected by row-level security, not by secrecy: it **is**
  public, and reissuing it is pointless work. The per-finding line already hedged
  for exactly this case — "if this is not meant to be public, rename it to …" — and
  the footer asserted over the top of it one line later.

  The footer now branches on the same discriminator the per-finding hedge uses: a
  named provider format carries an `action_url`, shape alone does not. With no
  recognised provider among the public-prefixed findings it says evnx cannot tell a
  publishable analytics key from an unrecognised secret, and asks the reader to
  check before rotating. With one, the wording is unchanged — a Stripe key behind
  `VITE_` is not ambiguous. Both branches still lead with renaming, which is right
  either way, and the unprefixed path is untouched. Found by running `scan` on a
  real Next.js project rather than a fixture. (#82)

- **`sync` produced a git diff on a project whose `.env` had not changed.**
  `generate_placeholder` iterated a `HashMap` and returned the first match, and
  more than one pattern usually matches a key — `STRIPE_SECRET_KEY` matches
  `SECRET`, `_KEY$` and `^STRIPE_` at once. Rust randomises `HashMap` iteration per
  process, so the winner changed between runs and was written into `.env.example`, a
  committed file. Eight identical runs produced four different answers. Order is now
  exact key match, then longest pattern, then declaration order; every rung is
  total, so the answer depends on the config file and the key and nothing else. The
  test runs the binary eight times rather than looping in-process, because the
  hasher is seeded once per process and the bug was invisible within a single
  run. (#86)

- **An uncompilable `sync` pattern was silently skipped**, so the key fell through
  to the default and the template looked fine while ignoring the project's rules.
  Now refused at load time, naming the pattern. `$schema` and `description` were
  silently dropped by serde, so an operator could not tell accepted from
  misspelled. `naming_convention` is documented in the sync config but read from
  `--naming-policy`, and now warns rather than vanishing. And
  `is_placeholder_value` ignored `config.patterns`, so a project whose convention
  is `<set-me>` had all of its own placeholders reported as real values in the
  dry-run preview. (#86)

- **Every expansion error reported "line 0"** — not a line in any file, in exactly
  the case where a line number is what you need: a large `.env` that suddenly will
  not parse. Expansion is a separate pass over the assembled map, run after the
  whole file is parsed, which is what makes a forward reference work; by then the
  line numbers were gone. The parser now records where each key was written and
  hands that to the pass. It reports the line of the key being expanded rather than
  of the missing variable, which frequently does not exist anywhere. The
  circular-expansion error shared the same hardcoded zero and is fixed with it. (#91)

- **The published Scoop manifest was invalid JSON on every release since
  `e96bdb4`.** `"find": "^([a-f0-9]+)\s"` — `\s` is not a JSON escape. The heredoc
  that writes the manifest is unquoted, which it must be for `${VERSION}` to
  substitute, so the shell ate one level of backslash and two arrived as one. It
  needs four. Confirmed in the wild, not reasoned about: the published 0.6.0
  manifest is rejected by both `jq` and `python -m json.tool`. Scoop's `parse_json`
  uses `ConvertFrom-Json -ErrorAction Stop` and returns `$null` on a throw, so on a
  strict parser there was no manifest at all; a lenient parser leaves
  `^([a-f0-9]+)s`, which does not match the real `.sha256` because the hex run is
  followed by a space. Both outcomes are broken.

  The escape is one character; the gate is the fix. A new step asserts valid JSON,
  the decoded `find` value, a `sha256:<64 hex>` hash and the version before
  anything is pushed — decoded values, because over-escaping also parses, it just
  means something else. This survived five releases because nothing between the
  heredoc and `git push` ever looked at the result. (#84)

- **A 404 while fetching a checksum published `"hash": "sha256:"`.** The release
  workflow declares no `defaults.run.shell`, so steps run under `bash -e` without
  `pipefail`, and the exit status of `curl | awk` is `awk`'s. Reproduced against a
  real 404: exit 0 without `pipefail`, 22 with it. Fixed in the Scoop job and in
  the Homebrew job, which had the same pattern across four assignments. (#84)

- **Nothing validated the Homebrew formula before pushing it.** Same shape as the
  Scoop gap, and worse: a formula is Ruby, so an invalid one means `brew install
  evnx` fails outright for every macOS user until the next release. Now checks
  `ruby -c`, that no `sha256` is empty, that every `sha256` is 64 hex characters,
  and that the version matches the tag. Verified by parsing the step out of the YAML
  and running it against four sabotages — an empty `sha256`, a six-character one, a
  wrong version, and invalid Ruby — each of which it caught. (#89)

### Changed

- **A secret containing a dollar sign can now be written.** A token containing
  `${...}` made the whole file fail to parse; one containing `$NAME`, where `NAME`
  was also a key, was **silently replaced** — substituting one of your secrets into
  another, with no error. Neither `\$` nor `$$` nor single quotes prevented it.

  Two halves. **Single quotes and backticks no longer expand**: they are literal in
  every respect now, matching shell, dotenv, python-dotenv and godotenv, and
  turning "literal for backslashes but not for dollars" into one rule people can
  hold. Whatever is between single quotes is the value. And **`\$` is a literal
  dollar in a double-quoted value**, so expansion can be kept for the rest of the
  string.

  ⚠️ **Breaking, and silently so**: a file relying on expansion inside single
  quotes changes meaning with no error, because both forms parse. Nothing in the
  suite asserted the old behaviour — all 1039 tests passed before the new ones were
  added — which reassures about internal reliance and warns that no test would have
  caught it either way. (#90)

## [0.6.0] - 2026-09-27

**The first release since 0.5.0 that contains new code.** 0.5.1 and 0.5.2 were
release engineering; ten pull requests have landed since.

### Security — a notice carried forward

⚠️ **Anyone who ran `evnx validate --fix` on v0.4.x or earlier should rotate the
secret it generated.** The value was derived from roughly 25 bits of *time*
rather than randomness — 48 of its 64 hexadecimal characters were always `0` —
and the output announced it as "Generated secure secret", so there was nothing to
suggest it needed replacing. Where such a value ended up as a session-signing key
or a JWT secret, it is guessable by anyone who knows approximately when it was
generated.

⚠️ **Anyone who ran `evnx migrate --to github` on a version before 0.5.0 should
rotate everything it uploaded, at the source.** Values were base64-encoded rather
than encrypted: the function was named `encrypt_for_github` and the payload went
into a field called `encrypted_value`, so both the code and the wire format
described encryption that had not happened.

Both were fixed in 0.5.0. This notice is repeated here because it was written
into `docs/releases/v0.5.0.md` and nothing published that file — the release body
was generated from a template that read no file at all, so the notice went unread
across three releases. That is fixed in this release too; see *Fixed* below.

Affected packages, all `< 0.5.0`: `crates.io/evnx`, `PyPI/evnx`, `npm/@evnx/cli`.
See the repository's security advisories for ranges and remediation.

### Why this is 0.6.0 and not 0.5.3

Two changes alter behaviour rather than only adding to it, and in `0.x` semver
puts a compatibility change in the minor position:

- Escape sequences in a **multiline** double-quoted value are now interpreted,
  matching what a single-line value always did. A multiline value containing a
  backslash changes meaning. See *Changed* below.
- A value recognised only by its shape, behind a build-time public prefix, is
  raised from `low` to `medium`. A `--severity medium` gate that passed can now
  fail. See *Added* below.

Neither is large, and no correct program depended on either — but both are the
kind of change someone should read a version number and expect.

### Added

- **A secret behind a build-time public prefix is now called what it is.** `scan`
  reported a Stripe key in `VITE_STRIPE_SECRET_KEY` correctly as high confidence,
  then told you to `git filter-repo` and force push — work that contains nothing,
  because the value is compiled into every bundle already served. Someone
  following it literally would rewrite history and believe they were done.

  A finding whose variable carries `NEXT_PUBLIC_`, `VITE_`, `REACT_APP_`,
  `EXPO_PUBLIC_`, `NUXT_PUBLIC_`, `GATSBY_`, `VUE_APP_`, `STORYBOOK_` or
  `PUBLIC_` now says which tool inlines it, names the prefix-stripped variable to
  rename it to, and leads the next steps with renaming rather than history
  rewriting. The prefix is in the JSON as `public_prefix`, so CI can tell
  "leaked" from "published". (#78)

  Severity is raised where it was wrong: a known provider format behind a public
  prefix is **high**, and a value recognised only by shape is raised to
  **medium** — enough to be seen, not enough to fail a `--severity high` gate on
  an analytics key that is meant to ship. ⚠️ Escalation only ever raises a
  finding that already exists, so `PUBLIC_KEY=<a public key>` and
  `NEXT_PUBLIC_API_URL=<a url>` are still not findings at all.

- **`.ipynb` files are scanned.** Identical JSON was scanned as `.json` and
  ignored as `.ipynb` — a filter, not a parser. Notebooks are where ML work keeps
  keys, and a key often appears twice: once in a `source` cell and again in an
  `outputs` cell where it was echoed. (#78)

- **`vault share`, `vault revoke` and `vault role` accept both `--with` and
  `--user`.** `share` took `--with` while the other two took `--user` for the
  same thing, so whichever you learned first was wrong half the time. Neither is
  deprecated. (#78)

- **`scripts/verify-claims.sh`** — documented claims about evnx are now checked by
  running it. Each claim has a stable id and a verdict of `CONFIRMED`, `REFUTED`
  or `SKIP`, and the exit status mirrors evnx's own contract: `0` all confirmed,
  `1` something refuted, `2` could not run. "No claims ran" is `2`, because a
  verification script that exits 0 when it verified nothing is the bug it exists
  to catch. (#74)

### Fixed

- **The GitHub Release body now carries this changelog.** `release.yml` generated
  the body from an inline template — installation instructions for the seven
  channels, and nothing about the release itself — so anything written *about* a
  version never reached the page people land on. The rotation notice above lived
  in `docs/releases/v0.5.0.md`, which no workflow reads, and went unread across
  three releases as a result.

  The section between `## [VERSION]` and the next heading is now lifted from
  `CHANGELOG.md` and placed **above** the installation instructions, so a reader
  who needs to rotate a credential does not have to scroll past seven package
  managers to learn it. A missing heading warns rather than failing the release,
  and the body falls back to installation instructions alone.

- **`evnx scan` no longer reports a clean result for a scan that examined nothing.**
  `dist/`, `build/`, `node_modules/` and `target/` are skipped on a directory
  walk, and naming a file inside one was silently dropped — so a CI gate pointed
  at a build artifact printed `✓ No secrets detected`, `0 files scanned` and exit
  `0`, on a file holding a live Stripe key. The extension allowlist did the same
  to anything it did not recognise: `evnx scan key.pem` scanned nothing and called
  it clean.

  Two changes, the second being the general one: **a path you name is always
  scanned** — the default exclusions and the allowlist exist to stop a directory
  walk reading a million vendored files, and were never a veto over an argument —
  and **zero files scanned is never success**, now exit `2`, because nothing was
  found *and* nothing was looked at. (#71)

- **The private-key detector could not fire outside a `.env` file.**
  `-----BEGIN RSA PRIVATE KEY-----` and its four siblings went undetected in every
  source and `.pem` file. `extract_tokens` splits on whitespace and keeps only
  tokens longer than 20 characters, and the header splits into `-----BEGIN`,
  `RSA`, `PRIVATE`, `KEY-----` — every piece too short to survive. Undetectable in
  exactly the files private keys live in.

  Fixed by making the built-in detectors **rules on the same engine as custom
  ones**, matched against whole lines through `PatternSet`. Custom
  `[[scan.patterns]]` were never affected, so a rule a user wrote was strictly
  stronger than one evnx shipped — that was the real defect. A dead
  `get_patterns()` table of ~20 detectors, which nothing called while a
  hand-written `Regex::is_match` chain did the work, is gone with it. (#72)

- **`evnx migrate`'s missing-file hint named `--env`, a flag it does not have.**
  It is `--source-file`. (#70)

- **The crate no longer ships environment files.** `.env.backup` and
  `.envx/placeholders.json` were committed in March 2026 and shipped to crates.io
  in 0.5.0, 0.5.1 and 0.5.2, because this repository's own `.gitignore` used the
  narrow `.env`/`.env.local` convention that `evnx init` had already moved away
  from. The repo now uses the entries `evnx init` writes, and CI fails if
  `cargo package --list` contains either path. Published versions cannot be
  altered; this stops it recurring. (#73)

- **npm publishing downloaded every artifact in the release run, not just the
  binaries.** `docker/build-push-action` uploads a build record of its own — a
  `.dockerbuild` blob that is not a plain zip — and unzipping it killed the 0.5.2
  npm publish with `ADM-ZIP: Invalid or unsupported zip format`. Two earlier
  failed tags had hidden it, because the docker job only produced one once the
  release got that far. Filtered to `^evnx-`. (#69)

- **npm publishing now reads the registry back instead of trusting the publish
  step.** `npm publish` returning 0 means the tarball was accepted — not that the
  version resolves, and not that the `latest` dist-tag moved. The smoke test also
  gave up after 90 seconds, so 0.5.2 published correctly to all six packages and
  went red anyway.

  The registry is now polled with a 300-second budget and the install runs once,
  after it is known to be there. Three things are newly checked: that `latest`
  actually points at the new version (the test installed a pinned version, so a
  stale tag was invisible), that **all five platform packages** resolve — they are
  `optionalDependencies`, which npm skips without failing, so a missing one gave a
  clean install and a binary absent at runtime on that architecture — and that
  `evnx --version` prints what was published. A failed upstream release is also a
  failure here rather than a silent skip, which used to leave a green check beside
  a red release. (#75)

- **An escaped `\"` no longer ends a multiline value.** `Parser::is_closed_quote`
  tested `ends_with('"')`, so for `B="line1 \"q\"` the final character *was* a
  quote — the trailing one of an escaped `\"` — and the parser concluded the value
  had closed, reporting `Invalid format at line 2: missing '=' separator` on the
  continuation line. The check now requires the run of backslashes before the
  closing quote to be of even length. Single-quoted and backtick values are
  literal and were never affected.

  The same bug had a second site: the continuation branch used
  `strip_suffix(ml_quote)`, so a value could survive its opening line and then be
  cut short by a continuation line ending in `\"`. Both now go through one shared
  function, along with `evnx doctor` — see below. (#77)

- **`evnx doctor` no longer reports the continuation lines of a multiline value as
  `invalid syntax`.** It checked `.env` syntax line by line, with no notion of a
  value spanning lines, so a PEM key produced two or three complaints about a file
  that `validate`, `convert` and `scan` all accepted. It now tracks the open quote
  using the parser's own helpers, so the two cannot disagree about where a value
  ends.

  `doctor` also now reports a quoted value whose quote never closes. The
  line-based check could not represent that at all, so it called such a file clean
  while `validate` failed on it — the same disagreement in the opposite direction. (#77)

### Changed

- ⚠️ **Escape sequences in a multiline double-quoted value are now interpreted,
  matching the single-line behaviour.** The accumulated value was inserted raw, so
  `"x\ty"` produced a tab on one line but a literal backslash and `t` across two —
  the same quote character with different escape rules depending on the line count,
  which also made `\"` inexpressible in a multiline value at all.

  **This changes existing values.** A multiline double-quoted value containing a
  backslash now resolves it. PEM keys, certificates and base64 blobs contain no
  backslashes and are unaffected; a multiline JSON value with `\n` or `\"` inside
  it is not. Use single quotes for literal content. (#77)

### Documentation

- **The README said multiline and array values "will not parse correctly".** Two
  of its three examples parse fine, and multiline values have worked all along —
  the claim came from a verification step that read an exit code without reading
  the message: it wrote a two-line value, ran `evnx validate --env .env.ml`, got
  exit `2` and recorded a parse error. The `2` was `.env.example does not exist`;
  validate never parsed the file.

  The claim had spread into the planning material, where it became a roadmap item
  proposing "parser support for quoted multi-line values" — work to build a
  feature that already existed. It also meant the README contradicted the docs
  site, where `concepts-migrate-intro.mdx` correctly said multi-line values are
  handled. Rewritten from what the binary actually does. (#76)

## [0.5.2] - 2026-09-25

**Still release engineering. No source has changed since 0.5.0.**

### Why this exists

0.5.1's fix for the aarch64-musl build introduced a second one. The build step
branched with a shell conditional:

```sh
if [ "$TARGET" = "aarch64-unknown-linux-musl" ]; then cross build ...; else cargo build ...; fi
```

The Windows runner defaults to PowerShell, which parsed it as a syntax error:

```
ParserError: Missing '(' after 'if' in if statement.
```

The `Package` step immediately below has carried `shell: bash` for exactly this
reason, so the precedent was already in the file.

### Fixed

- The build is now **two steps with `if:` expressions** rather than one step with
  a shell conditional. A GitHub `if:` needs no shell, so it cannot break this way
  on any runner.
- **`fail-fast: false`** on the target matrix. The Windows failure cancelled the
  other six builds, so a tag that broke on one platform said nothing about the
  rest — and `release.yml` can only be exercised by tagging. Information per
  attempt is worth more here than runner minutes.

### Version history, since three patches look alarming

| Version | crates.io / PyPI | npm, Homebrew, Scoop, winget, GHCR |
|---|---|---|
| 0.5.0 | published | blocked — aarch64-musl had no C toolchain |
| 0.5.1 | published | blocked — the fix for that broke Windows |
| 0.5.2 | published | **the one to use** |

⚠️ **0.5.0, 0.5.1 and 0.5.2 are the same code.** If you already have 0.5.0 or
0.5.1 from crates.io or PyPI, nothing is wrong with it and there is nothing to
do. The other five channels go straight from 0.4.0 to 0.5.2.

Each version exists because crates.io and PyPI publish from workflows separate
from the binary build, so they succeeded while the binary build failed — and a
published version number can never be reused.

## [0.5.1] - 2026-09-25

**A release-engineering fix. No code changed between 0.5.0 and 0.5.1.**

### Why this exists

v0.5.0's tag build failed on `aarch64-unknown-linux-musl`:

```
error occurred in cc-rs: failed to find tool "aarch64-linux-musl-gcc"
```

`musl-tools` provides `musl-gcc` for x86_64 only. It does not provide the
aarch64 musl cross compiler, and `ring` — pulled in by rustls via reqwest, so
present in every `--all-features` build — runs a build script that needs a C
compiler for the target. The target was new in v0.5.0 and had never built.

Every other job in `release.yml` declares `needs: build`, so one broken target
took the GitHub Release, the container image, Homebrew, Scoop, winget and npm
down with it. **crates.io and PyPI publish from separate workflows and were
unaffected** — which is why 0.5.0 exists on those two and nowhere else.

⚠️ **If you installed 0.5.0 from crates.io or PyPI, nothing is wrong with it.**
0.5.1 is the same code. Every other channel goes straight from 0.4.0 to 0.5.1.

### Fixed

- `aarch64-unknown-linux-musl` now builds through `cross`, which runs in a
  container that already carries the toolchain — less brittle than fetching one
  from a third-party host.
- **CI now compiles the release targets.** It previously built none of the
  seven, so a target that could not compile was discoverable only by pushing a
  tag — at which point it blocks five channels. The two musl targets are built
  on every PR, since they are the ones that cross-compile with a C toolchain.

## [0.5.0] - 2026-09-25

The command-review release. Every command was run against a real build and
compared with its published guide; where the two disagreed, one of them was
wrong and got fixed. 31 pull requests (#29–#59).

**Full write-up: [docs/releases/v0.5.0.md](docs/releases/v0.5.0.md).** Entries
below are one line each; the detail, including how each was found, is there.

### Security

⚠️ **`evnx validate --fix` generated predictable secrets and called them
secure.** `generate_secure_secret` returned `SystemTime::now().as_nanos()`
XORed with a constant, under a comment reading *"replace with crypto RNG in
production"*. 48 of the 64 hex characters were always `0`, and three runs
seconds apart differed only in their last few — roughly 25 bits, of *time*
rather than randomness, searchable by anyone who knew the day. Now 32 bytes
from the OS CSPRNG via `getrandom`.

**If `evnx validate --fix` ever generated a `SECRET_KEY` for you, rotate it.**

⚠️ **Five commands reported success while doing nothing, or the wrong thing.**
These are the reason to upgrade rather than wait.

- **`evnx migrate --to github` never encrypted anything.** It base64-encoded the
  plaintext and posted it as `encrypted_value`. Now a real libsodium sealed box.
  **Rotate any secret pushed to GitHub by an earlier version.**
- **`evnx sync` wrote real values into `.env.example`** when creating one, then
  advised committing it. Placeholders are now the default.
- **`evnx scan` never read `.env.production`** — or any dotted variant. A
  directory with `.env` and `.env.production` scanned only the first and reported
  a completed scan.
- **`evnx scan <unwalkable path>` printed "✓ No secrets detected"** and exited 0.
- **`evnx migrate` with no `--to` guessed.** With no terminal it silently picked
  the **first** destination in the list and exited 0, printing a plan for
  somewhere the user had never chosen.

### Fixed — from the DevRel review of 2026-09-24

Every command re-checked against every guide, each claim verified independently
before acting. Write-up and verification in `evnx-devrel-review/`.

- **`evnx init` wrote a `.env.example` that could not be appended to.** No
  trailing newline, so the next line fused onto the final comment and the
  variable silently disappeared. `evnx add` was immune, which is why it
  survived — evnx's own documented next step hid it.
- **`evnx validate --fix` exited 1 after fixing everything.** It counted the
  issues found on entry and never recounted, so `validate --fix && deploy`
  never reached `deploy`, while the next plain `validate` exited 0.
- **`--validate-formats` rejected every non-HTTP URL.** `postgresql://`,
  `redis://` and `amqp://` were all "not a valid URL" — including the
  `DATABASE_URL` that `evnx init --with postgresql` had just written. There
  were two URL definitions in one binary; there is now one.
- **`doctor` called `export FOO=bar` invalid** while the parser accepted it,
  and reported only the first bad line. Both fixed — plus a third disagreement
  the new cross-check found on its first run: the parser refused a leading
  underscore, which POSIX allows.
- **Findings came out in a different order every run** — `HashSet::difference`
  with Rust's randomised hasher. `sync` had the same bug, and there it *writes*
  that order into `.env.example`, so two developers syncing one file produced
  two different files.
- **A key written twice vanished in silence.** Last-wins is kept; the silence
  is not. `validate` now names the lines and says which one takes effect.
- **SARIF put the variable name inside the `ruleId`**, so every new variable was
  a new rule to GitHub and no dismissal ever stuck. Adds `rules[]` with help
  links and `partialFingerprints`, so an alert survives a line moving above it.
- **`completions` answered an unusable argument with 1**, bypassing the
  centralised exit-code mapping. Now 2, like everything else.
- **`convert --help` was backwards about kubernetes** — it respects `--base64`,
  emitting `stringData:` without it.
- **`cloud status` promised a token renewal it had no way to verify**,
  contradicting `evnx auth status` on the same machine.
- **`init --detect` conflicted with `--yes`**, which it already implies.
- **Fourteen flags had no help text at all**, including `--format` on `validate`
  and `scan` — the two flags that make evnx usable in CI, neither naming `json`,
  `sarif` or `github`. The prose already existed, accurately, in the guides.
- **`backup` and `restore` disagreed about how a passphrase arrives.** `restore`
  honoured `EVNX_PASSWORD`; `backup` ignored it, so a scheduled backup had no way
  to supply one except writing it to disk — while the restore it fed needed no
  file at all. `backup` now accepts it, and each command accepts the other's flag
  spelling (`--key-file` / `--password-file`).
- **One key file could produce two different passwords.** `backup --key-file`
  Base64-encodes a binary key file before Argon2id; `restore --password-file`
  read the same file as UTF-8 and stripped a trailing newline. A binary key file
  therefore wrote a backup that could not be opened with the file that wrote it,
  and the only symptom was a failed decryption. Both sides now use one reader.
- **`evnx scan` claimed a value matched a live key format when nothing had read
  the value.** Every high-confidence finding carried "matches a live key format,
  not a placeholder", including ones reached purely by the variable's *name* —
  so `NEXTAUTH_SECRET=dev-not-a-real-secret` was reported as a live key. That
  line is what tells someone to drop everything and rotate; attaching it to a
  name match teaches people to ignore it. Name-based findings now say so.
- **`migrate --dry-run` reported a migration that had not happened.** The summary
  said `✓ 0 command(s) generated` while printing the commands, then followed it
  with "verify secrets were set" and "redeploy your project". It now reports what
  it previewed and points at the run that would apply it.
- **`convert --help` printed its examples as unusable Markdown** — a fenced block
  in a doc comment, which clap renders verbatim, so the fence markers and the
  shell line-continuations landed inline on one collapsed line. The one place in
  the CLI where copying an example could not work.

### Fixed — validate's blind spots

⚠️ **`evnx validate` said nothing about the credentials `evnx init` had just
generated.** Two independent gaps, and together they meant a fresh
`evnx init --with nextjs,postgresql` followed by the documented
`cp .env.example .env` produced a `DB_PASSWORD` and a `NEXTAUTH_SECRET` still
holding placeholders — and `validate` passed it without a word. A forgotten
password reaching production is the single thing this command exists to prevent.

- **The placeholder list did not know evnx's own placeholders.** `init` writes
  `your_<name>_value`; the list held `your_key_here`, `your_secret_here` and
  `your_token_here` — the suffix is `_here`, not `_value`. Now any `your_*`.
- **The weak-secret check only ever looked at one variable.** It was
  `env_vars.get("SECRET_KEY")` — a single hardcoded lookup — so `JWT_SECRET=123`,
  `SESSION_SECRET=weak` and `DB_PASSWORD=dev` all validated clean. Now every
  credential-shaped name. The length floor is split by role: 32 characters for a
  framework signing key, 16 for everything else, because an AWS access key **ID**
  is 20 characters by specification and is not weak.
- **A generated secret could be called weak by the command that generated it.**
  `1234` and `abcd` are ordinary hex, so roughly one run in 500 produced a
  64-character secret containing one. A uniformly hex or base64 value is now
  exempt from the word list.
- **`validate --fix` reported a placeholder as a completed repair.** Adding a
  missing key can only insert `your_value_here`; the key exists, the value does
  not. It now says so rather than printing the fix directly above the finding
  that contradicts it.
- **`--fix` swapped one placeholder for a vaguer one and called it a fix.**
  Recognising `your_*` meant `--fix` began "repairing"
  `DB_NAME=your_db_name_value` into `DB_NAME=your_value_here` — discarding the
  only hint the line carried, and reporting it directly above the finding it had
  not resolved. evnx can invent a secret, a URL, an email and a port; it cannot
  invent a database name, and no longer offers to.
- **`--fix` repaired at most one weak secret per run, and only if it was called
  `SECRET_KEY`.** The fixer carried the same hardcoded lookup as the check.
  `DB_PASSWORD` and `API_TOKEN` are now treated as credentials too, so `--fix`
  generates real values for them instead of declining.

### Fixed — the generated `.env` is now readable by evnx

⚠️ **`evnx init` told you to run `evnx validate`, and doing so reported every
variable missing.** `init` wrote `.env` with every assignment commented out as
`# TODO: DB_HOST=localhost`, while `.env.example` held the same variables
active — so the parser saw nothing in the file `init` had just created:

```
3. Run 'evnx validate' to check configuration     <-- init's own next step

✗  Missing required variable: DATABASE_URL
✗  Missing required variable: DB_HOST
... all six                                        exit 1
```

It also made `init`'s first instruction — *"Edit .env and replace placeholder
values"* — untrue, since there were no values in `.env` to replace. `.env` and
`.env.example` now carry the same assignments, and `validate` reports what
actually needs attention: `DB_NAME looks like a placeholder`.

- **`evnx add` reported variables the parser could not see.** Same cause:
  `add service postgresql` printed "Added 6 variables", wrote six `# TODO:`
  comments, and `validate` — run immediately after — called all six missing. Two
  commands contradicting each other in consecutive steps.
- **`evnx sync` no longer just says "up to date" when the source is empty.** A
  blank `.env` beside a populated `.env.example` is more likely a truncated file
  or the wrong directory than a steady state, so it now says so and points at
  `--direction reverse`. Still exit `0` — a notice, not a failure.
- `evnx add` writing into an empty `.env` no longer opens the file with three
  blank lines.
- **A build missing features now says which, and how to get them.** `backup`,
  `restore`, `migrate`, `auth`, `vault` and `cloud` are compiled out of the
  command list when their feature is off, so `evnx backup` answered with clap's
  bare *"unrecognized subcommand"*. Each module contained a "feature not enabled"
  hint written for exactly this case, and all of them were unreachable — there
  was no command left to dispatch to. `evnx --help` now names what is missing.
  Only `cargo install evnx` is affected: every prebuilt binary ships
  `--all-features`.
- **A real provider key is no longer called weak.** The weak-word list was a
  plain substring match, so `abcd` occurring inside a 62-character SendGrid key
  reported *"too weak or predictable"* — with the advice *"Run: openssl rand
  -hex 32"*, which would replace a working credential with a random string. A
  weak word now has to make up at least a fifth of the value to count. This
  replaced a charset heuristic added in the same release, which was fragile in
  two ways found within a day of each other: it knew the separators `+/=-_` but
  not `.`, and it required a digit, so dotted tokens and digit-free keys both
  fell through.

### Fixed — reported issues

- **A file that is absent is no longer reported as a file that failed to parse**
  ([#12](https://github.com/urwithajit9/evnx/issues/12)). `evnx diff` with no
  `.env.example` said *"Failed to parse .env.example"*, sending you after a
  syntax error in a file that does not exist; `validate` printed the OS error
  twice on top of that. Both now name the file and say how to create it. A file
  that really is malformed still reports the line — which `diff` had been
  dropping, because it printed its error without the cause chain.
- **`evnx migrate` no longer reports a migration that did not happen**
  ([#13](https://github.com/urwithajit9/evnx/issues/13)). An empty source exited
  `0` with no docs link, and named the source *kind* (`env-file`) rather than the
  file — so it could not distinguish an empty file from the wrong directory.
  Both it and a filter that matches nothing are now errors, as they already were
  in `evnx cloud run`.
- **`--skip-existing` and `--overwrite` are refused where they cannot work.**
  Eight of nine destinations only print commands, so they cannot know what
  already exists at the destination; they accepted both flags in silence.
  `--to github-actions` is the one that uploads, and honours them — its dry run
  now also counts what it previewed instead of reporting zero.
- **`evnx add custom` can no longer write a file evnx cannot read**
  ([#10](https://github.com/urwithajit9/evnx/issues/10), item 4). The variable
  name came straight from the prompt unvalidated, so entering `NODE_VERSION=22`
  wrote `# TODO: NODE_VERSION=22=22`. The parser had always refused such a key;
  it was never asked. Entering a name with `=` now suggests the name you meant.
- **`scan`'s machine formats made the claim its terminal output had stopped
  making.** `--format json` returned identical fields for a name match and a
  value match, and `--format github` still said *"matches"* and *"Rotate this
  credential"* for a finding reached purely by the variable's name. JSON gains
  `matched_by: "value" | "name"` (additive — no existing consumer breaks) and the
  annotation now says the value went unchecked.
- Generated `.env` files no longer carry two leading spaces on their
  `# (required)` comments — the one ragged edge in otherwise column-aligned
  output.

⚠️ **Behaviour changes worth knowing before you upgrade CI:**

- `evnx validate` now flags unfilled `your_*` placeholders and weak secrets in
  any credential-shaped variable. A `.env` that passed before may now fail —
  which is the point, but it is a change.
- `evnx migrate` exits `2` instead of `0` when there is nothing to migrate.

### Added

- **`evnx spec`** — a `[vars]` contract saying what each variable *is*: required,
  secret, its format, which environments need it. Read by `validate`, `scan` and
  `sync`. `evnx spec init` infers it from the files you already have.
- **`evnx cloud run -- <cmd>`** — decrypt a vault into a subprocess. Nothing on
  disk, nothing in shell history, nothing in `ps`. `--include`/`--exclude` narrow
  what the child sees; the child's exit code comes back unchanged.
- **`evnx vault share`, `members`, `role`, `revoke`** — team access, wrapped with
  hybrid X25519 + ML-KEM-768. Revocation re-keys the vault.
- **`evnx scan --pattern` and `[[scan.patterns]]`** — your own secret formats,
  matched in one `RegexSet` pass so declaring rules stays close to free.
- **`.evnx.toml` is now read.** The loader existed and nothing called it.
  Precedence is flag > config > default; lists combine; settings that weaken
  scanning are announced on every run.
- **`--env-name production`** across `validate`, `scan`, `diff`, `sync`,
  `convert`, `backup` and `template`. `diff --env-name production --against
  staging` compares two real environments.
- **`evnx sync --check`** — a CI gate with 0/1/2 exit codes, and `--format json`.
- **`evnx doctor --fix`, `--path`, `--strict`** — the auto-fix had worked since
  doctor shipped, reachable only via `EVNX_AUTO_FIX=1`.
- **`evnx init --detect`, `--from-source`, `--with`, `--list-components`** —
  `--from-source` builds the template from what the code actually reads.
- `evnx scan --severity`, a `summary` object in `scan --format json`, and golden
  files pinning every machine-readable output.

### Fixed

- `evnx migrate` no longer claims `✓ N uploaded` for the eight destinations that
  only print commands. `--dry-run` runs headless.
- `evnx add` warns when the `.env` it wrote to is not covered by `.gitignore`.
- `evnx template` refuses unresolved placeholders under `--strict`; `{{ VAR }}`
  with spaces and `|default:` both work.
- `evnx init` and `doctor` cover every env file, not just `.env`.
- `evnx add blueprint <unknown>` lists the blueprints instead of saying "run
  `evnx init`" — an interactive prompt, and so unusable in the scripts where
  `--yes` is passed. `evnx init --blueprint <unknown>` already listed them.
- `evnx add` no longer writes an orphaned `  # (required)` line detached from
  the variable it describes, and the files it writes end with a newline.
- Benchmarks compile again; `install.sh` no longer depends on JSON formatting.
- CI lints `--all-targets`, so test code is checked too. It was not before, and
  an `Assert` whose result was dropped — asserting nothing — sat unnoticed.

### Changed

- **`evnx validate --pattern` was removed** — its implementation was a discarded
  argument. Use `--env-name`.
- **`evnx scan` exit codes are now 0 / 1 / 2.** `2` means the scan could not be
  completed, and is not a louder `1`. `--exit-zero` suppresses `1` only.
- ⚠️ **An error is now exit `2` in every command, not `1`.** `scan`, `diff`,
  `doctor` and `sync --check` already used `2` for "could not run, so there is
  no verdict". Everything else inherited Rust's default, which numbers any error
  `1` — the code `validate` uses for *invalid*, `diff` for *differences found*
  and `sync --check` for *drifted*. So `evnx validate --env ./missing.env`
  reported **validation failed** for a file it never opened, and a typo'd
  `--env-name` did the same in `validate`, `diff`, `sync`, `convert`, `template`
  and `backup`. A malformed `.evnx.toml` did it in every command at once.
  **Findings are unchanged:** a real validation error is still `1`, differences
  still `1`, a secret still `1`, success still `0`.
- `evnx doctor` gained `--format`; `EVNX_OUTPUT_JSON=1` still works and the flag
  wins when both are given.

---

## [0.4.0] - 2026-09-15

> ⚠️ **Corrected 2026-10-08.** "Mathematically unable to read it" below is
> overstated, and the sentence is left as published rather than rewritten.
> A server that can write its own database can choose the key your *next* push
> is encrypted under — the client decides how to unwrap from fields the server
> sends. What is already stored still stays closed. See `SECURITY.md`.

Zero-knowledge encrypted cloud sync. Push a `.env` to a server that is
**mathematically unable** to read it, and pull it on any machine or in any
pipeline.

Everything here is behind `--features cloud`, which is **off by default** and not
enabled in the prebuilt npm / PyPI / Homebrew / Scoop binaries. `cargo install evnx`
produces a byte-identical binary to 0.3.8 in behaviour — no existing command
changed.

### Added

- **`evnx auth`** — `register`, `login`, `logout`, `status`. Authentication is
  SRP-6a: the password never leaves the machine, and the server must prove it
  holds your verifier before any session is stored.
- **`evnx auth totp`** — `enable` (with a scannable QR), `disable`,
  `recovery-codes`. `disable` and `recovery-codes` require a current second
  factor rather than a live session, so a stolen session cannot strip 2FA from an
  account.
- **`evnx auth sessions`** — `list`, `revoke`, `revoke-others`. Revocation kills
  the refresh tokens and blocklists the session, so an outstanding access token
  stops working immediately rather than lingering for its remaining lifetime.
- **`evnx auth token`** — `create`, `list`, `revoke`. API tokens for CI, scopable
  to one vault and to read-only. Supplied to the CLI via `EVNX_TOKEN`.
- **`evnx vault`** — `create`, `list`, `delete`. A vault holds one `.env`,
  versioned. Its key is generated locally and wrapped under your master key
  before it leaves the machine.
- **`evnx cloud`** — `push`, `pull`, `history`, `link`, `unlink`, `status`.
  `push` encrypts the file's **raw bytes**, so comments, ordering, quoting and
  whitespace survive a round trip unchanged.
- **`evnx cloud link`** binds a directory to a vault via `[cloud] vault` in
  `.evnx.toml`, so `push` and `pull` need no `--vault`. Written with a
  format-preserving TOML editor: your comments and unrelated settings survive.
  Safe to commit — a vault name is not a secret, and sharing it means a
  teammate's pull lands in the same place.
- **`evnx cloud status --ping`** exits non-zero when the server is unreachable,
  so it works as a health check. Without `--ping` it reads local files only and
  works offline.
- `rust-version = "1.85"` is now declared. Verified by building on it.

### Security

- **Credentials are stored at mode 0600** in `~/.config/evnx/credentials.json`,
  and the permissions are re-checked on every read — a file found readable by
  anyone else is tightened, with a warning that the tokens should be treated as
  exposed.
- **Plain `http://` is refused** for anything but a loopback address. Access and
  refresh tokens travel in request headers, and a refresh token lives 30 days.
- **Nothing that can decrypt a vault is ever written to disk.** The master key,
  vault keys and the private key are derived in memory, used, and dropped. A
  stolen `credentials.json` buys an authenticated session, and the API answers it
  with ciphertext.
- **No `--password` flag exists**, so a master password cannot reach shell
  history. Commands prompt, or read `--password-stdin`.
- Clears five advisories that were live in the dependency tree, four of them in
  the TLS stack the cloud feature uses: `RUSTSEC-2026-0104`, `-0099`, `-0098`,
  `-0049` (rustls-webpki) and `RUSTSEC-2026-0285` (rustls). Removes `atty`, which
  is unmaintained and carried `RUSTSEC-2021-0145`, in favour of
  `std::io::IsTerminal`.
- A `cargo audit` job was added to CI. Its absence is why those advisories went
  unnoticed.

### Changed

- `evnx --features cloud` roughly doubles the binary, 3.6 MiB → 7.2 MiB, and adds
  ~118 crates. This is why `cloud` is not in `full` and not in the prebuilt
  binaries.

### Known limitations

- **Vault sharing is not included.** Shared vault keys are wrapped with X25519,
  which is not post-quantum safe; a hybrid ML-KEM wrap lands before sharing
  ships. Solo vaults are unaffected — they are wrapped with Argon2id and
  XChaCha20, which are symmetric throughout.
- **A previously installed evnx can shadow the cargo one.** A package-manager
  install often sits earlier in `PATH` than `~/.cargo/bin`, so
  `cargo install evnx --features cloud` appears to succeed while the old binary
  still answers. Diagnose with `which -a evnx`.
- **One `.env` file at a time**, unchanged from 0.3.x: `evnx scan` does not
  discover `.env.prod`, `.env.production`, `.env.local` or `.env.staging` when
  scanning a directory. `validate` and `diff` accept them via `--env`.

---

## [0.3.8] - 2026-03-30

### Added

- `evnx backup --key-file <PATH>` — reads encryption password from a file
  for fully non-interactive CI/CD pipelines. UTF-8 content is trimmed of
  surrounding whitespace; binary content is Base64-encoded before Argon2id.
- `evnx backup --keep <N>` (default: 3) — rotates existing backup files before
  each write, preserving the last N backups. Set `--keep 0` to disable rotation.
- `evnx backup --verify` — re-decrypts the backup immediately after writing and
  compares byte-for-byte against the source. Exits with code 6 on mismatch.
- `BackupError::VerifyFailed` — new typed error variant, exit code 6.
- Exit codes table in `evnx backup --help` covering codes 0–6.
- `evnx restore --inspect` — decrypt a backup and list variable key names
  without writing any files. Values are never displayed.
- `evnx restore --password-file <path>` and `EVNX_PASSWORD` environment
  variable for non-interactive restore in CI/CD environments.
- Argon2id progress spinner during the key-derivation step of `evnx restore`.
- Structured exit codes for `evnx restore` covering codes 0–6.

### Changed

- `backup.rs` refactored into `backup/` module — split into `mod.rs`,
  `core.rs`, and `error.rs` following the same structure as `restore/`.
- Password memory safety — password string now wrapped in a `ZeroizeOnDrop`
  RAII guard inside `backup_inner`, guaranteeing zeroization on all exit paths
  including `?`-propagated errors and panics.
- Argon2id spinner replaces static `println!("Encrypting…")` in backup,
  matching the restore command. Suppressed when `--verbose` is active.
- `evnx restore` internal architecture split into `core`, `source`, and
  `error` modules — pure logic is independently testable without a terminal.
- `evnx restore --verbose` now emits a diagnostic line at every pipeline stage
  instead of a single line at startup.
- Password, decrypted content, and ciphertext blob are all zeroized via RAII
  guard on every exit path including panics.
- Consistent header and success summary formatting across backup and restore
  using shared `ui::` helpers.

### Fixed

- Off-by-one in `rotate_backups` — loop range was `(1..keep)`, stopping one
  position short and silently destroying the oldest backup. Corrected to
  `(1..=keep)`.
- `evnx restore <directory>` now reports a clear "not a regular file" error
  instead of a confusing IO message.
- `encrypt_content` re-export visibility corrected to eliminate unused-import
  warning in production builds.

### Security

- Password zeroization is now unconditional on all exit paths via `ZeroizeOnDrop`.
  The previous implementation had a narrow panic window between password
  acceptance and manual `zeroize()` after encryption.

**Full Changelog**: https://github.com/urwithajit9/evnx/compare/v0.3.7...v0.3.8

---

## [0.3.7] - 2026-03-20

### Fixed

- Scoop and Winget publish jobs moved from standalone workflow files into
  `release.yml` as inline jobs. Standalone `release: published` workflows
  never triggered because GitHub suppresses that event when a release is
  created by `GITHUB_TOKEN` inside a workflow.
- Scoop manifest `architecture.url` was writing a literal `$version` string
  instead of the real version number due to incorrect heredoc escaping.
  Corrected to use `${VERSION}` in the architecture block and `\$version`
  only in the autoupdate block where Scoop expects its own template variable.

**Full Changelog**: https://github.com/urwithajit9/evnx/compare/v0.3.6...v0.3.7

---

## [0.3.6] - 2026-03-19

### Added

- Windows package manager support via two new publish channels:
  - Scoop (user-local, no admin required): `scoop bucket add evnx https://github.com/urwithajit9/scoop-evnx && scoop install evnx`
  - Winget (system-wide): `winget install urwithajit9.evnx`
  - Both channels auto-update on every `v*` tag release via GitHub Actions.
- `evnx pre-commit` subcommand for Git pre-commit hook integration. Supports
  validation, secret scanning, and format checks before commit. An example
  hook script is provided in `scripts/`.
- All CLI `--help` outputs now include a link to the corresponding docs page
  at `https://www.evnx.dev/guides/<command>`. Error messages also link to
  relevant docs pages for faster resolution.
- Windows installation guide: https://www.evnx.dev/guides/install/windows
- Pre-commit integration guide: https://www.evnx.dev/guides/pre-commit

### Fixed

- Scan command panic: resolved `thread 'main' panicked at 'index out of bounds'`
  when scanning empty or malformed `.env` files (#142).
- Secret detection false positives: common placeholder values such as
  `your_key_here`, `CHANGEME`, and `***` are no longer flagged.
- Windows path resolution: `.evnx.toml` config lookup now resolves correctly
  on Windows systems.
- SHA256 validation: fixed checksum verification for cross-platform tarball
  downloads.
- Release workflow: `.zip` and `.zip.sha256` artifacts were produced during the
  build but never copied into the GitHub Release assets. Both Scoop and Winget
  installer URLs would 404 on every run. The asset preparation step now copies
  all `*.zip*` files alongside `*.tar.gz*` files.
- Scoop manifest: `bin` field referenced `evnx.exe`, which does not exist
  inside the archive. Corrected to `evnx-x86_64-pc-windows-msvc.exe` with an
  alias mapping it to the `evnx` command.
- Scoop workflow commit step: bare `$VERSION` shell variable was undefined in
  the commit step context. Replaced with `${{ steps.version.outputs.VERSION }}`.
- Scoop workflow hash source: manifest was fetching the `.tar.gz.sha256` file
  for the Windows ZIP installer. Corrected to fetch `.zip.sha256`.

### Changed

- Release workflow now produces `.zip` and `.zip.sha256` alongside `.tar.gz`
  for the Windows target, required for Scoop and Winget compatibility.
- Binary size reduced by approximately 12% using `cargo build --release --strip`.
- Error messages improved with actionable suggestions and direct links to docs.
- Secret scanning patterns updated to detect current AWS, Azure, and GitHub
  token formats.
- Pre-commit hooks run in an isolated subprocess to prevent environment leakage.

### Packaging

| Channel  | Install command                                      | Auto-update |
|----------|------------------------------------------------------|-------------|
| Scoop    | `scoop install evnx`                                 | Yes         |
| Winget   | `winget install urwithajit9.evnx`                    | Yes         |
| Homebrew | `brew install urwithajit9/evnx/evnx`                 | Yes         |
| Cargo    | `cargo install evnx`                                 | Yes         |
| PyPI     | `pipx install evnx`                                  | Yes         |
| npm      | `npm install -g @evnx/cli`                           | Yes         |

**Full Changelog**: https://github.com/urwithajit9/evnx/compare/v0.3.5...v0.3.6

---

## [0.3.5] - 2026-03-16

### Fixed

- PyPI Linux wheel build matrix reduced to x86_64 only. aarch64 and armv7
  targets were removed because cross-compilation inside the manylinux Docker
  container fails when the migrate feature is enabled — reqwest pulls rustls
  which pulls ring, and ring's ARM assembly fails to compile in the
  cross-compilation environment. x86_64 builds natively and is unaffected.
  ARM Linux users can install via the curl script or cargo instead:
  `curl -sSL https://raw.githubusercontent.com/urwithajit9/evnx/main/scripts/install.sh | bash`
  or `cargo install evnx --features full`.

## [0.3.4] - 2026-03-16

- fix: switch reqwest to native-tls, ring removed from dependency tree


## [0.3.3] - 2026-03-16

### Fixed

- PyPI aarch64 wheel build failure caused by ring crate assembly
  cross-compilation error. Added RING_PREGENERATE_ASM=1 env var to
  maturin linux build job and switched reqwest to rustls-tls-native-roots
  to avoid ring dependency during cross-compilation.
- npm smoke test timing increased to handle registry replication delay.

## [0.3.2] - 2026-03-16

- fix: add features=["full"] to pyproject.toml [tool.maturin]
  so PyPI wheel includes migrate, backup, restore commands

## [0.3.1] - 2026-03-16

Patch release fixing PyPI distribution and Homebrew automation. No changes to
the CLI itself — only release infrastructure and documentation.

### Fixed

- PyPI wheels were published without optional features (`migrate`, `backup`,
  `restore`). All maturin build jobs now include `--features full`, so
  `pipx install evnx` installs the full command set.
- `update-homebrew-tap` job in `release.yml` was incorrectly indented as a
  nested key inside the `release` job. It was never executing. Moved to the
  correct top-level position under `jobs:`.
- Homebrew Formula `install` block updated from `Dir["evnx-*"].first` glob to
  explicit per-platform binary names for reliable installs.

### Added

- Homebrew tap support: `brew install urwithajit9/evnx/evnx`.
  The `update-homebrew-tap` job in `release.yml` now automatically updates
  `urwithajit9/homebrew-evnx` with the correct version and SHA256 checksums
  on every release.
- Homebrew install instructions added to README and release notes template.

### Changed

- README installation section restructured with per-OS pipx setup instructions
  covering macOS, Ubuntu/Debian (including the PEP 668 explanation for 22.04+),
  older Ubuntu (20.04), and Windows.
- Release notes template updated to include the Homebrew install command.

---

## [0.3.0] - 2026-03-14

This release is a comprehensive refactor. The focus is on command consistency,
improved test coverage, and breaking changes to several commands that had
accumulated technical debt from the initial prototype. Users upgrading from
0.2.x should review the breaking changes below before updating.

### Breaking changes

- `evnx init` no longer accepts `--stack` or `--services` flags. The command
  is now fully interactive, using a TUI with three modes: Blank, Blueprint,
  and Architect. Run `evnx init` with no arguments to start.
- `evnx add` interactive flow has been revised. Previous flag-based usage is
  not guaranteed to be compatible.
- Several internal argument names and output formats were normalised for
  consistency across commands. Run `evnx <command> --help` after upgrading.

### Changed

- `evnx init` refactored to interactive TUI with Blank / Blueprint / Architect
  modes replacing the previous `--stack` and `--services` argument approach.
- All commands reviewed and refactored for argument consistency and improved
  error output.
- Test suite significantly expanded across all command paths.

### Fixed

- Shell syntax error in Windows binary extraction during npm publish workflow.
- Dead CHANGELOG link removed from GitHub release notes template.
- npm platform packages now include a stub README to reduce search noise on
  npmjs.com.
- npm install instructions corrected to use `@evnx/cli` package name
  (previously showed unscoped `evnx` which does not exist on npm).
- All documentation and install scripts updated from `dotenv.space` to
  `evnx.dev` domain.

### Added

- `CHANGELOG.md` added to repository.
- Auto-generated "What's Changed" section appended to GitHub releases via
  `generate_release_notes: true`.
- npm badge and PyPI badge added to README.
- `workflow_dispatch` added to `npm-publish.yml` for manual recovery without
  cutting a new release tag.

---

## [0.2.1] - 2026-03-07

### Changed

- Several commands refactored with improved internal structure.
- Test coverage improved across validation and scan paths.

### Fixed

- Various known bugs addressed.

### Added

- npm publish workflow (`npm-publish.yml`) — publishes `@evnx/cli` to npmjs.com
  on each tagged release.
- PyPI publish workflow (`python-publish.yml`) — publishes `evnx` to PyPI via
  maturin on each tagged release. Install with `pipx install evnx`.

---

## [0.2.0] - 2026-03-04

### Breaking changes

- Multiple initial commands revised with updated arguments and behaviour.
  Users upgrading from 0.1.0 should review `evnx --help` for each command.

### Added

- `evnx add` command for adding variables to `.env` interactively from custom
  input, service blueprints, or templates.
- 14+ format targets for `evnx convert`: JSON, YAML, Shell, Docker Compose,
  Kubernetes, Terraform, GitHub Actions, AWS Secrets Manager, GCP Secret
  Manager, Azure Key Vault, Heroku, Vercel, Railway, Doppler.

### Changed

- Secret pattern detection enhanced with improved entropy analysis.
- Error messages for validation failures made more actionable.
- CLI documentation expanded.

### Fixed

- Windows path handling corrected.
- False positives in GitHub token detection reduced.

### Performance

- Validation on large `.env` files approximately 3x faster.

---

## [0.1.0] - 2026-03-01

Initial public release.

### Added

- `evnx init` — stack and service based interactive project setup.
- `evnx validate` — basic validation engine (placeholders, weak secrets,
  misconfigurations).
- `evnx scan` — core secret scanning with pattern matching.
- `evnx diff` — comparison between `.env` and `.env.example`.
- `evnx convert` — basic format conversion.
- `evnx sync` — bidirectional sync between `.env` and `.env.example`.