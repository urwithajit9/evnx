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
- ⛔ **A server that can write its own database can choose your vault key.**
  The client decides how to unwrap a vault key from two fields the *server*
  sends — `eph_pub_key` and `mlkem_ciphertext`. Both absent means "your own
  copy, wrapped under your master key"; both present means "shared with you,
  wrapped by the hybrid path". The hybrid wrap is not authenticated to a sender
  and needs only the recipient's public keys, which the server already holds.

  So an attacker with database write access can set those two fields on a vault
  **you created and never shared**, to a wrap of a key they chose. Your next
  `cloud push` encrypts under that key, and they can read it.

  ⚠️ This limit was previously described here as applying "the moment you
  share". That was too narrow: nothing local records which vaults you own, so
  the client cannot tell a genuine share from a fabricated one. It applies to
  every vault.

  **What still holds:** the server cannot read what is already stored — the
  ciphertext at rest and the wrapped keys stay closed. What it can do is
  influence what *future* pushes are encrypted under. A passive observer,
  a database dump, a court order against stored data: all still fail.

  **Fixing it needs** signed wraps plus locally pinned identities, and a server
  change. Until then, treat cloud sync as **beta** and do not put secrets there
  that you could not rotate.

- ⚠️ **A recipient's public keys come from the server**, with no out-of-band
  fingerprint check — the same root cause as above, seen from the sharing side.
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
