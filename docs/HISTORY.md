# How behaviour changed: notes moved out of the README

The README describes what evnx does today. These notes explain how some of it
came to be, for anyone who relied on an earlier release. The CHANGELOG has the
full list by version.

## Multiline values and escapes

⚠️ **Corrected 2026-09-27.** The README's "Known Limitations" said multiline and
array values "will not parse correctly" and gave three examples. Two of the three
parse fine, and multiline values have been supported all along: the parser accepts
a quoted value that spans lines and `convert` round-trips the newlines intact. The
wrong claim was repeated into several planning documents and nearly produced work
to build a feature that already existed.

`evnx convert --to json` gives `"-----BEGIN PRIVATE KEY-----\nMIIEvQ…\n-----END PRIVATE KEY-----"`
for a multiline key, and `evnx scan` reports it as a private key. Single quotes span
lines too, without unescaping.

⚠️ **This changed; see the CHANGELOG for the version.** An escaped quote inside a
double-quoted value that spans lines used to be a parse error, and a double-quoted
value that spanned lines kept its backslashes instead of resolving them, so `"x\ty"`
gave a tab on one line but a literal backslash and `t` across two. If you relied on
backslashes surviving verbatim inside a multiline double-quoted value, switch it to
single quotes, which are literal by definition.

## `scan` and the `.env.<name>` files

**Multiple environments are supported as of v0.5.0.** `evnx scan` reads every `.env`
variant it walks past, and `--env-name production` resolves to `.env.production`
across `validate`, `diff`, `sync`, `convert`, `backup` and `template`.

⚠️ Before v0.5.0, `evnx scan` silently skipped `.env.production`, `.env.local`,
`.env.staging` and every other dotted variant, and reported "no secrets detected"
without opening them. If you relied on `evnx scan` as a gate on an earlier version,
it never saw the file most likely to hold production credentials.

## Key names (v0.13.0)

Keys may contain `.`, `-`, `[`, `]` or a leading digit. evnx reads the names Docker
Compose reads, such as `spring.datasource.url` and `my-key`. `validate` warns that a
shell cannot `export` them, and each `convert` format or `migrate` destination that
cannot hold one refuses it by name.

## `${VAR}` expansion (v0.13.0)

`${VAR}` expands top to bottom, as Docker Compose and python-dotenv do: with `B=1`,
`A=${B}`, `B=2`, evnx gives `A=1`, and a reference to a variable assigned only below
it is an error that names the line to move. Until v0.13.0 evnx expanded against each
key's final value (`A=2`).

## Install addresses (v0.15.0)

The install script was reachable under three spellings (`dotenv.space/install.sh`, the
`raw.githubusercontent.com` link to `scripts/install.sh`, and `evnx.dev/install`). The
one documented address is `https://evnx.dev/install.sh`; `dotenv.space/install.sh`
redirects to it, so an old pinned command keeps working while that redirect stays.
