# Security Policy

## Reporting a vulnerability

**Email [security@evnx.dev](mailto:security@evnx.dev).** Include enough to
reproduce the issue.

We will **acknowledge within three working days** and tell you what we intend to
do about it.

Please do not open a public issue for a security problem. GitHub's private
[Report a vulnerability](../../security/advisories/new) form is also fine if you
prefer it.

## Testing

Test against **your own account**. Do not access anyone else's data, degrade the
service, or run automated scanning without telling us first.

Good-faith research conducted that way will not result in action against you.

## Supported versions

The latest release. evnx is pre-1.0 and fixes land in a new minor or patch
release rather than being backported.

| Version | Supported |
|---------|-----------|
| latest  | ✅ |
| older   | ❌ — upgrade |

## What the threat model actually claims

Worth reading before reporting, because two of these are deliberate and
documented rather than oversights:

- **The server never sees plaintext.** Your password derives a master key with
  Argon2id that never leaves your machine; it unwraps a per-vault key that
  decrypts the vault with AES-256-GCM. The server holds ciphertext and a wrapped
  key it cannot open.
- **Sharing uses a hybrid X25519 + ML-KEM-768 wrap**, so a shared vault stays
  sealed against an adversary with a quantum computer. There is no X25519-only
  path — the database refuses to store half a wrap.
- **Each version's ciphertext is bound to its version number**, so a server
  cannot replay an old version as the current one.
- ⚠️ **A recipient's public keys come from the server.** A malicious server could
  substitute its own and read what you subsequently share. There is no third
  party to check them against and out-of-band fingerprint verification is not
  built, so "the server cannot read your secrets" is "…cannot read them
  *passively*" the moment you share. This is known and documented, not a finding.
- ⚠️ **A lost master password cannot be recovered.** That is the design, not a
  bug.

Full write-up: <https://www.evnx.dev/security>

## Scope

| In scope | Out of scope |
|---|---|
| `evnx` CLI | Anything requiring physical access to an unlocked machine |
| `api.evnx.dev` | Social engineering of maintainers or users |
| `app.evnx.dev` | Volumetric denial of service |
| `evnx-crypto` | Findings from automated scanners with no demonstrated impact |
