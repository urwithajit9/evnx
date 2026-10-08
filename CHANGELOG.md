# Changelog

All notable changes to evnx are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

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