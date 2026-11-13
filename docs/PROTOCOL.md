# Cloud vault protocol

How an evnx cloud vault is built: the keys, the bytes, and the requests. This
describes evnx v0.14.0 against evnx-server at the commit pinned in section 1.
It is read from code. Where the code does not settle a point, the text says so.

For what this protects against, and what it does not, read
[THREAT_MODEL.md](./THREAT_MODEL.md). Section numbers there refer to this file.
What changed in 0.14.0 is listed in [section 12](#12-changes-in-0140).

## Contents

1. [Sources and notation](#1-sources-and-notation)
2. [Primitives and parameters](#2-primitives-and-parameters)
3. [Account keys](#3-account-keys)
4. [Vault keys](#4-vault-keys)
5. [A version](#5-a-version)
6. [Push and pull](#6-push-and-pull)
7. [Revoke and rekey](#7-revoke-and-rekey)
8. [Client-side checks](#8-client-side-checks)
9. [What the server stores](#9-what-the-server-stores)
10. [Versioning and compatibility](#10-versioning-and-compatibility)
11. [Not settled by the code](#11-not-settled-by-the-code)
12. [Changes in 0.14.0](#12-changes-in-0140)

## 1. Sources and notation

Every section ends with a **Source** line. A reviewer can check each claim
against it.

| Prefix | Means | Pinned at |
|---|---|---|
| `cli:` | this repository, `src/cloud/` and related | `5311ccc` (origin/main; no source change since the `v0.14.0` tag) |
| `srv:` | the evnx-server repository | `34b664f` (origin/main, includes PRs #13, #14 and #15) |
| `crypto:` | the `evnx-crypto` crate from crates.io, `src/` | 0.2.1 (`cli:Cargo.toml:131`, `cli:Cargo.lock:942-943`) |

`crypto:` is not in this repository. It is a registry dependency, so its line
numbers refer to the published 0.2.1 sources.

Notation: `a || b` is concatenation. All integers in the AAD are big-endian.
"b64" is standard base64 **with** padding (`crypto:encoding.rs:3-17`).

## 2. Primitives and parameters

| Purpose | Algorithm | Parameters |
|---|---|---|
| Password to keys | Argon2id, version 0x13 | m = 65536 KiB (64 MiB), t = 3, p = 4, 32-byte output |
| Argon2 domain separation | Argon2 secret input `K` | `evnx-master-key-v1` or `evnx-srp-password-v1` (not secret) |
| Login proof | SRP-6a | RFC 5054 2048-bit group, SHA-256 |
| Subkey derivation | HKDF-SHA256, no salt | 32-byte output, one `info` string per purpose |
| Vault content | AES-256-GCM | 32-byte key, 12-byte random nonce, 16-byte tag, AAD |
| Key wrap (own copy, private key) | XChaCha20-Poly1305 | 32-byte key, 24-byte random nonce, 16-byte tag, no AAD |
| Key wrap (shared) | X25519 + ML-KEM-768 hybrid, then XChaCha20-Poly1305 | see 4.3 |
| Identity keypair | Ed25519 seed, X25519 and ML-KEM-768 derived from it | see 3.2 |
| Transport integrity hash | BLAKE3, lowercase hex, of the ciphertext alone | 64 hex characters |
| API token storage, backup-code storage | BLAKE3 of the raw value | server side only |
| Server session tokens | JWT, HS256 | access 15 minutes, refresh 30 days (defaults; `srv:src/config.rs:613-616`, `srv:src/services/jwt.rs:67`) |
| TOTP secret at rest (server side) | XChaCha20-Poly1305 under an HKDF-SHA256 purpose key derived from `SERVER_DATA_KEY` | 24-byte random nonce, user id as AAD; stored as `v1:` + base64(`nonce24 \|\| ciphertext`) |
| Server-side decoy salts | HMAC-SHA256 under an HKDF purpose key derived from `SERVER_DATA_KEY` | 32 bytes, from the address and a label |

Source: `crypto:kdf.rs:15-27,41-42,105-160`, `crypto:srp.rs:41-42,51-53`,
`crypto:vault.rs:164-188,270-290`, `crypto:keypair.rs:554-603`,
`crypto:lib.rs:92-102` (the crate's own table of choices),
`srv:src/services/secrets.rs:12-14,43-45,98-124,164-178`.

HKDF `info` strings. Each is a separate key purpose:

| `info` | Used for | Source |
|---|---|---|
| `evnx-private-key-enc-v1` | key that seals the Ed25519 seed | `crypto:keypair.rs:80` |
| `evnx-vault-key-wrap-mk-v1` | key that wraps the owner's own vault key | `crypto:vault.rs:31` |
| `evnx-vault-key-wrap-v2` | key that wraps a vault key for another member | `crypto:keypair.rs:77` |
| `evnx-x25519-from-ed25519-v1` | X25519 private key from the Ed25519 seed | `crypto:keypair.rs:425` |
| `evnx/mlkem768/v1` | 64-byte ML-KEM-768 seed from the Ed25519 seed | `crypto:keypair.rs:92` |

The server uses two more `info` strings, on its own key `SERVER_DATA_KEY`
(32 bytes, base64, from the server environment): `evnx/v1/totp-secret` and
`evnx/v1/srp-decoy-salt` (`srv:src/services/secrets.rs:43-45`). They are
unrelated to any user key.

The ML-KEM and X25519 `info` strings can never change. Changing them derives a
different key for every existing user (`crypto:keypair.rs:82-92`, `:445-452`).

### 2.1 Nonce handling

| Where | Size | Source of the nonce | Notes |
|---|---|---|---|
| Vault content (AES-GCM) | 96 bit | OS CSPRNG, fresh per call | `crypto:vault.rs:170-173`. Collision bound is about 2^-32 after 2^32 encryptions under one key (`crypto:vault.rs`, doc on `encrypt_vault`). One push is one encryption. |
| Re-encryption on rekey | 96 bit | fresh, via `encrypt_vault` | `crypto:vault.rs:249-263` |
| Wraps and sealed private key | 192 bit | OS CSPRNG, fresh per call | `crypto:vault.rs:277-278`, `crypto:keypair.rs:778-779` |

Nonces are stored in front of the ciphertext. They are not secret.

### 2.2 Sizes

| Value | Bytes | Base64 characters |
|---|---|---|
| Argon2 salt, SRP salt | 32 | 44 |
| Ed25519 public key, X25519 public key, ephemeral X25519 key | 32 | 44 |
| ML-KEM-768 public key | 1184 | 1580 |
| ML-KEM-768 ciphertext | 1088 | 1452 |
| Sealed private key (`nonce24 \|\| seed32 \|\| tag16`) | 72 | 96 |
| Wrapped vault key (`nonce24 \|\| key32 \|\| tag16`) | 72 | 96 |
| SRP verifier | 256 to 1024 hex characters | (hex, not base64) |

Source: `crypto:keypair.rs:105-111`, `crypto:encoding.rs:3-17`,
`srv:src/routes/auth.rs:29-68` (the server checks the lengths 44, 44, 44, 1580 and
60..=300 for the sealed key), `srv:migrations/004_mlkem_hybrid.sql:14,24`.
The 72-byte wrapped-key size is computed from the layout, not stated as a
number in the code.

**Request size.** The server rejects a request body above `MAX_REQUEST_SIZE_KB`,
default 64 (`srv:src/lib.rs:36-40`, `srv:src/config.rs:637`). A push carries the
ciphertext as base64 inside JSON, so the largest `.env` that fits is somewhat
under 48 KiB at the default. That figure is derived, not stated in code.

## 3. Account keys

### 3.1 Registration

The client does all of this before it sends anything
(`cli:src/cloud/auth.rs:146-182`):

1. Normalise the email: trim, lowercase (`cli:src/cloud/auth.rs:193-195`). The
   email is the SRP identity, so both ends must agree.
2. Generate two independent 32-byte salts, `srp_salt` and `argon2_salt`.
3. `srp_password = Argon2id(password, srp_salt, K = "evnx-srp-password-v1")`.
4. `verifier = SRP-6a verifier(email, srp_password, srp_salt)`.
5. `master_key = Argon2id(password, argon2_salt, K = "evnx-master-key-v1")`.
6. Generate the keypair (3.2).
7. `encrypted_private_key = b64(nonce24 || XChaCha20-Poly1305(HKDF(master_key,
   "evnx-private-key-enc-v1"), nonce24, ed25519_seed))`.

The minimum password length for a new account is 12 characters
(`cli:src/cloud/auth.rs:41`). The server does not see the password and cannot
check it.

`POST /api/v1/auth/register` carries: `email`, `srp_verifier`, `srp_salt`,
`argon2_salt`, `ed25519_public_key`, `x25519_public_key`, `mlkem_public_key`,
`encrypted_private_key` (`cli:src/cloud/auth.rs:43-58`,
`srv:src/routes/auth.rs:29-68`).

Source: `crypto:kdf.rs:25-27,41-42,105-160`, `crypto:keypair.rs:340-355`.

### 3.2 The identity keypair

One 32-byte Ed25519 seed is the only secret that is stored. The other two keys
are derived from it:

```text
ed25519_seed   = 32 random bytes (OS CSPRNG)
x25519_private = HKDF-SHA256(ikm = ed25519_seed, info = "evnx-x25519-from-ed25519-v1", 32 bytes)
mlkem_seed     = HKDF-SHA256(ikm = ed25519_seed, info = "evnx/mlkem768/v1", 64 bytes)
mlkem_decap    = ML-KEM-768 key from seed (FIPS 203 d || z)
```

Consequence: a password change re-seals the same seed, so the public keys do
not change (`srv:src/routes/master_key.rs:46-50`).

The Ed25519 key is registered but **never used to sign** in this version
(`crypto:lib.rs:100`). One doc comment in the crate still says the key "is used
to verify vault access grants" (`crypto:keypair.rs:7-9`). The code produces no
signature anywhere, so that sentence is wrong or describes a plan.

Source: `crypto:keypair.rs:291-327,420-467`.

### 3.3 Login

SRP-6a with mutual proof, then JWTs (`cli:src/cloud/auth.rs:338-440`):

1. Client sends `email` and its ephemeral `A` (hex) to `POST /auth/srp/init`.
2. Server answers with `session_id`, `srp_salt`, `argon2_salt` and `B`. The
   session lives in the cache for 5 minutes (`srv:src/routes/auth.rs:315-393`).
3. Client derives `srp_password` with the returned salt (the full Argon2id cost)
   and sends proof `M1` to `POST /auth/srp/verify`.
4. Server verifies `M1` and returns `M2`. The client refuses to continue if `M2`
   does not verify (`cli:src/cloud/auth.rs:389-398`).
5. If TOTP is on, the server returns a 5-minute `totp_pending` token. The client
   then posts the code to `/auth/totp/verify`
   (`srv:src/routes/auth.rs:590-621`, `srv:src/middleware/auth.rs:38-44`).
6. The server issues an access JWT (HS256) and a refresh token. A refresh token
   is single-use: a refresh revokes the old one
   (`cli:src/cloud/client.rs:16-17`).

**Unknown addresses.** The start step does not reveal whether an address has an
account. For an unknown address the server answers in the same shape as for a
real one: both salts are derived from the address (HMAC-SHA256 under a purpose
key of `SERVER_DATA_KEY`, one label per salt), so the same address gets the same
salts on every call, and the server public value `B` is computed from a random
throwaway verifier. A proof for an unknown address does the same computation, is
counted toward the same lockout, and fails
(`srv:src/routes/auth.rs:328-352,473-512`, `srv:src/services/secrets.rs:164-178`).
The undo step of a master-password change (`/auth/master-key/undo/init`) uses the
same technique with its own labels (`srv:src/routes/master_key.rs:591-620`).
This covers the sign-in start step only. Registration answers 409 "Email already
registered" for an existing address (`srv:src/routes/auth.rs:100-103`), and
authenticated callers can look up a public key by email (4.4).

Throttles: 5 failed proofs lock the email for 900 seconds; 30 `srp/init` calls
per address per 900 seconds (`srv:src/routes/auth.rs:188-191,207-210`). TOTP: 3
failures, 900 seconds (`srv:src/routes/auth.rs:215-218`).

Credentials on the client are in `~/.config/evnx/credentials.json`, mode 0600:
access token, expiry, refresh token, email. Nothing that can decrypt a vault is
stored (`cli:src/cloud/creds.rs:1-17`).

### 3.4 Unlocking

Every command that touches a vault key asks for the password again
(`cli:src/cloud/vault.rs:229-233`). The master password source is, in order:
`--password-stdin`, `EVNX_MASTER_PASSWORD_FILE`, `EVNX_MASTER_PASSWORD`, a
prompt (`cli:src/utils/secret_source.rs:21-24`).

1. `GET /auth/me` returns `argon2_salt` and `encrypted_private_key`.
2. `master_key = Argon2id(password, argon2_salt, ...)`.
3. Decrypt the seed. A wrong password fails the Poly1305 tag. That is the only
   password check (`cli:src/cloud/auth.rs:934-1008`).
4. Re-derive the X25519 and ML-KEM keys from the seed.

An API token (`evnx_tok_...`) authenticates requests but decrypts nothing. A
pipeline needs the token, the master password **and a key bundle**
(`cli:src/cloud/token.rs:3-17`).

#### 3.4.1 The key bundle

A token is not given the salt and the sealed seed. CI carries them itself, in a
key bundle.

| Item | Value |
|---|---|
| Form | `evnxkb1_` followed by base64url (no padding) of a JSON object |
| Fields | `v` (1), `user_id`, `argon2_salt`, `encrypted_private_key` |
| Secret content | None in the clear. `argon2_salt` is public. `encrypted_private_key` is the sealed seed of 3.1, openable only with the master password. |
| Source in CI | `EVNX_KEY_BUNDLE`, or the first line of the file named by `EVNX_KEY_BUNDLE_FILE`. If both are set, the variable wins. |
| How to get one | `evnx auth key-bundle` (needs a signed-in session, not a token). `evnx auth token create` prints one next to the token; `--no-key-bundle` skips it, and `-q` prints the token only. |

Behaviour of the client:

- With a bundle, the unlock of 3.4 takes the salt and the sealed seed from the
  bundle and makes no `GET /auth/me` call. A bundle with another `v`, an
  empty field, a wrong prefix or bad base64url is refused with a message that
  does not echo the input.
- Without a bundle, a token makes one `GET /auth/me` call. If the server answers
  403 the client stops and says to make a bundle.
- The bundle's `user_id` is not compared with the token's. A bundle from another
  account fails later, when that account's copy of the vault key does not open.
- The bundle holds the sealed seed as it was when the bundle was made. After a
  master-password change (3.5) the old bundle no longer opens, and the client says
  to make a new one.

Source: `cli:src/cloud/keybundle.rs:1-12,29-32,36-92,94-118`,
`cli:src/cloud/auth.rs:934-985,1034-1070`, `cli:src/cloud/token.rs:166-192`,
`cli:src/cli.rs:1051-1062,1244-1270`.

#### 3.4.2 `GET /auth/me` and API tokens

| Caller | Setting | Answer |
|---|---|---|
| Signed-in session (also before the email is verified) | any | 200 with `user_id`, `email`, `email_verified`, `encrypted_private_key`, `argon2_salt`, `totp_enabled`, `has_mlkem_key` |
| API token, any scope | `AUTH_ME_SESSION_ONLY=true` | 403 |
| API token, any scope | `AUTH_ME_SESSION_ONLY=false` (the code default) | 200 with `encrypted_private_key` and `argon2_salt` only |
| Unknown token | either | 401 |

The setting exists so that clients older than 0.14.0, which read the key
material with a token, keep working while CI adopts the bundle. The cloud service
run by the evnx maintainers has set it to `true` since 2026-11-13. That value is
deployment, not code (see section 11). A server you run yourself needs it set.
With `false`, the server logs the first read by each token (by token id, once per
process start).

Source: `srv:src/middleware/auth.rs:77-86,285-319`, `srv:src/routes/mod.rs:283-296`,
`srv:src/routes/auth.rs:1502-1535`, `srv:src/config.rs:646-651`,
`srv:docs/API-CHANGES.md` (entry 2026-11-12).

### 3.5 Changing the master password

The client re-derives the master key, re-seals the seed, and re-wraps every
**own-copy** vault key (rows where `eph_pub_key IS NULL`) in one request
(`srv:migrations/009_master_key_rotation.sql:9-22`). Vault keys, blobs and
shared wraps do not change. The client checks that its own output unwraps before
sending, and writes the old wraps to a 0600 file as an offline undo
(`cli:src/cloud/rotate.rs:14-31`).

The server requires a fresh SRP proof of the current password, plus TOTP if
enabled (`srv:src/routes/master_key.rs:20-40`). With reason `routine` it keeps a
snapshot of the previous material for a window (`MASTER_KEY_UNDO_WINDOW_HOURS`,
default 72 hours, 0 to 8760 allowed: `srv:src/config.rs:653-662`,
`srv:src/routes/master_key.rs:464-476`). With reason `compromised` it keeps none
(`srv:migrations/009_master_key_rotation.sql:35-40`).

**The snapshot is erased when it closes.** A snapshot closes when its window runs
out, when it is restored, or when the same account rotates again after it
expired. Closing sets the five `prev_*` columns (old verifier, salts, sealed seed
and wraps) to NULL. The row stays, with its times and its outcome (`expired`,
`restored`). A table constraint refuses a closed row that still holds any of that
material (migration 015, which also cleared rows closed earlier). The server
closes expired snapshots for every account on a one-hour timer. A snapshot past
its `expires_at` cannot be restored even before the timer runs.

Source: `srv:src/db/rotation.rs:190-250,305-330,351-362`,
`srv:migrations/015_rotation_snapshots_are_audit_only.sql:24-61`,
`srv:src/main.rs:99,163-178`.

## 4. Vault keys

### 4.1 Creation

`vault_key` is 32 bytes from the OS CSPRNG, generated on the client
(`crypto:vault.rs:44-50`). The creator's copy is wrapped under the master key:

```text
wrap_key = HKDF-SHA256(ikm = master_key, info = "evnx-vault-key-wrap-mk-v1")
wrapped  = nonce24 || XChaCha20-Poly1305(wrap_key, nonce24, vault_key)      // no AAD
```

`POST /api/v1/vaults` sends `name`, `environment`, `encrypted_vault_key = b64(wrapped)`.
`eph_pub_key` and `mlkem_ciphertext` are omitted. The server inserts the vault
and the owner's `vault_members` row in one transaction
(`cli:src/cloud/vault.rs:213-271`, `srv:src/routes/vaults.rs:77-159`).

This path uses no key agreement. It is symmetric, so it does not depend on
X25519 (`srv:migrations/004_mlkem_hybrid.sql:17-22`).

Name rules: 1 to 64 characters, `^[a-z0-9\-]+$`. Environment: one of
`production`, `staging`, `development`, `test`
(`srv:src/routes/vaults.rs:23-31,64-68,90-97`).

### 4.2 The two shapes of a member's copy

Every `vault_members` row holds exactly one of two shapes. The pair of optional
columns tells the client which.

| Shape | `eph_pub_key` | `mlkem_ciphertext` | Wrapped under |
|---|---|---|---|
| Own copy (the creator; also the caller of a rekey, whose row is re-sealed under their master key) | NULL | NULL | HKDF subkey of the member's master key |
| Shared copy | present | present | hybrid X25519 + ML-KEM-768, to the member's public keys |

Half a wrap is refused three times: by the database
(`vault_members_wrap_is_whole`, `srv:migrations/004_mlkem_hybrid.sql:69-74`), by
the server's types (`srv:src/db/members.rs:39-54`), and by the client
(`cli:src/cloud/sync.rs:980-1014`).

The constraint was added `NOT VALID`, which exempted older rows. Migration 016
deletes every row that holds exactly one of the two fields and then validates the
constraint for all rows. Such a row was a pre-hybrid share, which the client does
not open. The migration stops with an error if the row belongs to a vault
owner, and deletes nothing in that case. A member who held such a row has to be
shared with again
(`srv:migrations/016_wrap_is_whole_is_validated.sql:51-75`).

### 4.3 Wrapping for another member

`wrap_vault_key_for_user(vault_key, recipient_public_keys)`
(`crypto:keypair.rs:623-675`):

```text
(eph_secret, eph_pub) = fresh X25519 keypair
ss_x25519             = X25519(eph_secret, recipient_x25519)          // reject if all-zero (low-order key)
(mlkem_ct, ss_mlkem)  = ML-KEM-768.Encaps(recipient_mlkem_pub)        // fresh randomness
ikm                   = ss_mlkem(32) || ss_x25519(32) || mlkem_ct(1088) || eph_pub(32) || recipient_x25519_pub(32)
wrap_key              = HKDF-SHA256(ikm, info = "evnx-vault-key-wrap-v2")
encrypted_vault_key   = nonce24 || XChaCha20-Poly1305(wrap_key, nonce24, vault_key)   // no AAD
```

The ikm order is fixed (`crypto:keypair.rs:581-603`). The `v2` string replaced
`v1` so that a v1 wrap cannot be presented as a hybrid one
(`crypto:keypair.rs:69-77`).

Unwrapping (`crypto:keypair.rs:690-749`): compute `ss_x25519`; **refuse** if
it is non-contributory; decapsulate; rebuild `wrap_key`; open the AEAD. ML-KEM
decapsulation never fails on a forged ciphertext (implicit rejection). The AEAD
tag is what fails (`crypto:keypair.rs:709-712`).

### 4.4 What a share sends

`evnx vault share` (`cli:src/cloud/vault.rs:427-610`):

1. Needs a user session. An API token is refused client-side.
2. `GET /users/{email}/public-key` returns the recipient's `x25519_public_key`,
   `ed25519_public_key`, `mlkem_public_key` (`srv:src/routes/users.rs:15-35`).
   The client uses them as given. No fingerprint is shown or pinned.
3. The client unlocks, opens its own copy of the vault key, wraps it for the
   recipient (4.3).
4. `POST /vaults/{id}/members` with `user_email`, `role`, `encrypted_vault_key`,
   `eph_pub_key`, `mlkem_ciphertext`.

Server rules (`srv:src/routes/members.rs:32-165`): caller is admin or owner; role
is `viewer`, `developer` or `admin` and must be below the caller's rank; the
recipient must have an ML-KEM key on file (409 if not); the insert never
overwrites an existing member (409 if already a member). The audit event stores
the member id and role, not the wrap.

The server cannot check that the wrap is correct, or who made it.

### 4.5 Public-key lifecycle

The ML-KEM public key can be uploaded once for an account that predates it:
`PUT /auth/public-keys`. A different value gets 409. It needs a user session, not
an API token (`srv:src/routes/users.rs:62-110`). No endpoint changes the X25519
or Ed25519 key.

## 5. A version

### 5.1 What is encrypted

The raw bytes of the `.env` file, as read from disk. Not parsed, not
re-serialised: comments, order and quoting survive. An empty file is refused
(`cli:src/cloud/sync.rs:12-18,376-379`). There is no padding and no
compression, so the ciphertext length is the plaintext length plus 16.

The client also parses the file for key **names** and sends them in the clear,
with a count (`cli:src/cloud/sync.rs:393,1052-1068`). If the file is not valid
UTF-8 or does not parse, the push still goes and carries no names.

### 5.2 The AAD

```text
offset  size  field
0       4     ASCII "evnx"
4       2     length of vault_id in bytes, u16 big-endian
6       n     vault_id, UTF-8 bytes
6+n     4     version number, u32 big-endian
```

For a hyphenated UUID, `n` = 36 and the AAD is 46 bytes. The length prefix makes the
encoding unambiguous. Source: `crypto:vault.rs:130-138`.

`vault_id` is the string, byte for byte. The client **lowercases** it when the
user passes an id (C10, `cli:src/cloud/vault.rs:73-81`). When the vault is
resolved by name it uses the id the server returned, which is lowercase
(`cli:src/cloud/vault.rs:74-76`, comment). Since 2026-11-11 (commit `fb4b810`)
the two paths agree. Versions pushed earlier with an upper-case id were sealed
with that spelling and open only with it (`cli:CHANGELOG.md:320-328`).

`version` is the number the server assigns, `latest + 1`. The client predicts it
before encrypting (`cli:src/cloud/sync.rs:20-30,390-396`). After a push it checks
the server's answer equals the prediction
(`cli:src/cloud/sync.rs:433-442`).

### 5.3 Blob layout

Encryption: `AES-256-GCM(vault_key, nonce12, plaintext, aad)`; output is
`ciphertext || tag16` (`crypto:vault.rs:164-188`).

| Place | Layout |
|---|---|
| Object storage (stored blob) | `nonce12 \|\| ciphertext \|\| tag16` |
| Storage key | `vaults/{vault_id}/{version:08}/{random uuid}.enc` |
| `blob_size_bytes` (database) | length of the stored blob, 12 + plaintext + 16 |
| Download response body | the stored blob, `application/octet-stream` |
| Download response header | `X-Blob-Hash: <blob_hash>` |

Source: `srv:src/routes/versions.rs:96-107,369-374`, `srv:src/services/storage.rs:118-125`.

The storage key ends in a fresh UUID, so a retry or a rekey never overwrites an
existing object.

### 5.4 The transport hash

`blob_hash = lowercase_hex(BLAKE3(ciphertext))`. The input is the ciphertext
**without** the nonce (`crypto:vault.rs:118-120` and its doc).

Both ends use it:

| Who | When | Source |
|---|---|---|
| Client | computes it and sends it with the push | `cli:src/cloud/sync.rs:398` |
| Server | recomputes it on push, rejects a mismatch with 422 | `srv:src/routes/versions.rs:88-94` |
| Server | recomputes it over `blob[12..]` on download, fails with an internal error on a mismatch | `srv:src/routes/versions.rs:330-338` |
| Server | recomputes it over each staged rekey blob, twice (at staging and before the swap) | `srv:src/routes/rekey.rs:89,276-288` |
| Client | compares `X-Blob-Hash` with its own hash of the bytes it got; skipped if the header is absent | `cli:src/cloud/sync.rs:674-687` |

It is not a keyed MAC. Anyone can recompute it over altered bytes. It detects
corruption and mismatched uploads. Authenticity comes from the GCM tag and the
AAD (`crypto:vault.rs:105-117`).

The server computes it in one function, `transport_hash`, and uses it on push,
on staging, on download and when it checks a rekey
(`srv:src/services/transport.rs:14-30`, `srv:src/routes/versions.rs:89,331`,
`srv:src/routes/rekey.rs:89,277`). It does not call the client crate's
`evnx_crypto::blob_hash`: the server's `src/` does not import that crate. Both
compute BLAKE3 over the same bytes. A unit test pins the server's function to
BLAKE3 test vectors for empty and one-byte input (`transport.rs:42-55`). No
server test calls the crate's function.

### 5.5 Push request body

`POST /api/v1/vaults/{id}/versions` (`srv:src/routes/versions.rs:24-37`):

| Field | Type | Meaning |
|---|---|---|
| `nonce` | b64 | the 12-byte nonce |
| `ciphertext` | b64 | ciphertext with tag |
| `blob_hash` | hex | BLAKE3 of the decoded ciphertext |
| `key_names` | string array | `.env` key names, plaintext |
| `key_count` | integer | number of names |
| `base_version` | integer, optional | version the client based this on |

The server requires `nonce` to decode to exactly 12 bytes and `blob_hash` to be
64 lowercase hex characters. Anything else is refused with 422 before the blob is
stored. Staging a rekey blob (7.2) has the same rules, through the same function
(`srv:src/routes/versions.rs:82-94,371-405`, `srv:src/routes/rekey.rs:81-93`).
The server joins the decoded nonce and the ciphertext and hashes only the
ciphertext. The client splits a download at byte 12.

Response: `201 {version_num, pushed_at}`.

## 6. Push and pull

### 6.1 Push

Client (`cli:src/cloud/sync.rs:333-444`):

1. Resolve the vault (by id, name, or `.evnx.toml` binding). Choose the file.
2. Read the file; refuse if empty.
3. Ask for the master password. Unlock. Open the vault key (4.2).
4. `GET /vaults/{id}/versions/latest` gives `current` (404 means 0). The
   high-water mark is checked here (section 8).
5. `target = current + 1`. Build the AAD. Encrypt. Hash.
6. `POST .../versions` with `base_version = current`.
7. Record the new version in the high-water file.

Server (`srv:src/routes/versions.rs:45-152,179-260`):

1. Role check: developer or above (`VaultAccess<AtLeastDeveloper>`). A viewer is
   refused before the body is read. An API token must be `read_write`.
2. Early version-limit check (6.4). It is a quick refusal, not the guarantee.
3. Read `MAX(version_num)`. If `base_version` is given and differs: **409**.
4. Decode, check the nonce and hash sizes, verify `blob_hash`, upload the blob.
   No lock is held during upload.
5. Open a transaction. Lock the vault row `FOR KEY SHARE`.
6. Lock the caller's membership row `FOR SHARE`. If it is gone, or the role is
   below developer: **403**.
7. Re-read the latest version. If it moved: **409**.
8. Count the vault's versions against the plan limit, inside the transaction
   (6.4). Over the limit: **403**.
9. `INSERT` the version. A unique violation on `(vault_id, version_num)`: **409**.
10. Write the audit event (event `push`, metadata `{version}`) **in the same
    transaction**, then commit. If the event cannot be written, the push does not
    happen.
11. If anything fails after the upload, delete the blob, best effort.

The 409 text is "Remote is at version N. Pull latest before pushing"
(`srv:src/routes/versions.rs:157-161`). A 409 cannot be retried with the same
bytes. The version is in the AAD, so the file must be encrypted again against the
new number (`cli:src/cloud/sync.rs:29-30`).

If the client sends no `base_version`, the server still returns 409 when the
vault moved between its own two reads (step 7). It does not renumber.

### 6.2 Pull

Client (`cli:src/cloud/sync.rs:448-549,641-724`):

1. Resolve the vault. Choose the version: `--version N`, else `GET .../versions/latest`.
   A vault with no versions is an error.
2. `GET /vaults/{id}/versions/{n}/blob`. Check `X-Blob-Hash`. Split the first 12
   bytes as the nonce. Raise the high-water mark to `n`.
3. Only now ask for the password. Unlock. `GET /vaults/{id}/my-key`. Open the key.
4. Decrypt with the AAD for `(vault_id, n)`. On failure, say that the server
   served a different version or altered it.
5. Write the file (or stdout), with an overwrite check.

Server (`srv:src/routes/versions.rs:295-375`): any member may pull. It reads the
blob, checks its hash, records a `pull` audit event, and streams the bytes. The
pull event is written after the check without waiting for it; a failed write is
logged and the pull goes ahead.
`?purpose=rekey` marks the read in the audit trail, honoured only for admin and
owner.

### 6.3 Locking (C4, evnx-server #13)

Order: **vault row first, then membership row**. Never the other way. No storage
I/O while a lock is held (`srv:src/db/locks.rs:3-36`).

| Operation | Vault row | Membership row |
|---|---|---|
| Push | `FOR KEY SHARE` | caller's row, `FOR SHARE` |
| Add member | `FOR KEY SHARE` | the new row, by the insert |
| Remove member, change role | `FOR KEY SHARE` | target's row, by the `DELETE`/`UPDATE` |
| Rekey | `FOR UPDATE` | when removing someone: caller's and leaver's rows `FOR SHARE`; then every member's row by the writes |
| Delete a version | `FOR UPDATE` | none |

Source: `srv:src/db/locks.rs:11-15`, `srv:src/db/members.rs:142-164,220-245`,
`srv:src/routes/members.rs:96-120`, `srv:src/routes/rekey.rs:347-374`,
`srv:src/routes/versions.rs:179-216,493-495`.

What this gives:

- A pusher whose membership is removed or lowered while the push is in flight
  gets **403**. The revocation waits for an in-flight push to commit. A push that
  starts after the revocation commits sees the row gone or lowered.
  A revoked user cannot land a version after their revocation.
- Two pushes to one vault do not serialise, because `FOR KEY SHARE` does not
  conflict with itself. The loser hits `UNIQUE (vault_id, version_num)` and gets 409.
- A rekey waits for pushes in flight and holds new ones off until it commits.
  The version set it reads under the lock is the set present at commit.

### 6.4 The plan's version limit

The limit comes from the vault owner's plan. A push checks it twice:

- before the upload, on the pool, so an obvious refusal leaves no blob in
  storage (`srv:src/routes/versions.rs:57-63`);
- inside the push transaction, after the vault and membership locks and before the
  insert. This is the check that counts
  (`srv:src/routes/versions.rs:225-227`, `srv:src/services/quota.rs:377-470`).

Two pushes that race both compute the same next version number. Only one insert
can succeed on `UNIQUE (vault_id, version_num)`, so the count taken in the
transaction is the count that holds when the row goes in. The server's tests
include one for this (`srv:tests/integration_test.rs`, test
`c4_a_push_is_counted_against_the_version_limit_after_its_locks`). The reasoning
in this paragraph is read from the code, not stated in it.

A refusal is 403 with reason `PLAN_LIMIT`. Deleting a version makes room
(7.4).

## 7. Revoke and rekey

### 7.1 Remove without rekey

`DELETE /vaults/{id}/members/{user}` deletes the membership row. The vault key
does not change. The removed member's API access ends (the extractor answers 404
for a non-member). Anything they hold still decrypts: the old vault key opens
every version, including versions pushed after removal, for anyone who can
obtain the ciphertext. The audit event records `"rekeyed": false`
(`srv:src/routes/members.rs:309-379`, `srv:src/routes/rekey.rs:5-17`).

Rules: the owner cannot be removed. Removing another member requires outranking
them, so two admins cannot evict each other. Leaving is always allowed.

### 7.2 Rekey

`evnx vault revoke` rekeys by default; `--no-rekey` skips it. `evnx vault rekey`
rotates with nobody leaving (`cli:src/cloud/vault.rs:1251-1431,1479-1592`).

Client, `rotate_vault_key` (`cli:src/cloud/vault.rs:1636-1796`):

1. Open the current vault key.
2. Generate a new vault key.
3. For every version `n`: download the blob (`?purpose=rekey`), run
   `reencrypt_vault(old_key, new_key, aad(vault_id, n))`, and stage the result with
   `POST /vaults/{id}/rekey/blobs`. The AAD is the **same** as before, so version
   `n` stays version `n`. A fresh nonce is used.
4. For every remaining member: wrap the new key. The caller's own copy goes under
   the master key. Everyone else gets a hybrid wrap to the public keys fetched
   from `/users/{email}/public-key`.
5. `POST /vaults/{id}/rekey` with `versions`, `members`, and optionally
   `remove_user_id`.

Two requests are used because one version can approach the 64 KiB request limit
(`srv:src/routes/rekey.rs:19-31`). A rekey needs admin or owner and a user
session.

Server, `rekey` (`srv:src/routes/rekey.rs:310-533`).

**Before the transaction**, the server checks every entry in `versions` against
what is in storage (`verify_staged_blobs`, `srv:src/routes/rekey.rs:229-297`).
This reads storage, so it runs before any lock is taken. An entry is refused
unless:

- its `blob_key` has exactly the form staging gives this vault and this version,
  `vaults/{vault_id}/{version:08}/{uuid}.enc`, with a canonical lowercase UUID and
  nothing added (422). A key that names another vault's or another version's
  object, or a path that only looks like one, fails here
  (`srv:src/services/storage.rs:130-145`);
- its `blob_hash` is 64 lowercase hex characters, and no `blob_key` appears twice
  (422);
- the object exists (409, "stage it again");
- the object is at least 12 bytes, its ciphertext (after the nonce) hashes to
  the `blob_hash` sent, and its length equals `blob_size_bytes` (422).

The server cannot tell whether the bytes are valid ciphertext under the new key.
It checks name, existence, size and hash.

**Then, in one transaction:**

1. Lock the vault row `FOR UPDATE`.
2. If `remove_user_id` is set, lock and re-check the caller's role (admin or
   above) and the leaver's role under the lock. The owner cannot be removed; the
   caller must outrank the leaver.
3. **Stale-set check.** The `versions` in the request must equal the vault's
   current set of version numbers exactly, with no duplicates. If not: **409**
   ("a re-key must cover every version at once").
4. The `members` in the request must equal the current members minus the leaver,
   exactly. The leaver must not be in it. If not: **409**.
5. Each member entry must be a whole hybrid wrap, or a master-key wrap for the
   caller's own row only. Anything else: **422**.
6. A quoted `blob_key` must differ from the blob the version already uses (422).
   Quoting the current object would leave that version under the old key.
7. Repoint every version to its staged blob (`blob_key`, `blob_hash`,
   `blob_size_bytes`). Set every member's wrap. Delete the leaver.
8. Commit. Write the `vault_rekey` audit event.

**After the commit**, the server deletes the blob each version used before the
swap, one delete per version. This is best effort: a failed delete is logged and
does not fail the rekey (`srv:src/routes/rekey.rs:507-522`). Nothing is deleted
before the commit, because until then the old blobs are what the vault opens with.

If a push or a membership change lands between the client's reads and the swap,
the sets no longer match and the rekey fails with 409. The staged blobs are then
unreferenced objects (7.3). The client re-runs the rekey.

### 7.3 What a removed member can still read

| Case | Can they read it? |
|---|---|
| Versions they downloaded or decrypted before removal | Yes. Copies cannot be recalled. |
| Old ciphertext, if they keep a copy, and the old vault key | Yes. Rekey does not change old key material they hold. |
| Versions pushed after a rekey-and-remove | No, unless they obtain the new key. |
| Versions pushed after a bare removal (`--no-rekey`) | Yes, if they can obtain the ciphertext. They lost API access, so this needs another route to the bytes. |
| Anything, via the API | No. Non-members get 404. |

**Replaced blobs are deleted.** After a rekey commits, the server deletes the
replaced objects (7.2). `delete_blob` is also called on a failed push, on
`delete_version`, and on account deletion (`srv:src/routes/versions.rs:135,548`,
`srv:src/routes/auth.rs:1820`, `srv:src/routes/rekey.rs:515`). A deleted object
is gone from storage. Copies a removed member already downloaded are not.

**Unreferenced objects and `sweep-blobs`.** A delete can fail, a push can fail
after its upload, and a rekey can be abandoned after staging. Each leaves an
object that no `vault_versions` row points to. The operator command
`evnx-server sweep-blobs` lists them, and `evnx-server sweep-blobs --apply`
deletes them:

| Option | Meaning |
|---|---|
| (none) | Dry run. Prints counts and the object keys it would delete. |
| `--apply` | Delete the listed objects. Exit 1 if any delete fails. |
| `--min-age-hours N` | Only objects older than N hours (default 24, 0 allowed). A push uploads before it inserts its row, and a rekey stages before it swaps. Both look like orphans in between. |

It looks only at keys shaped like `vaults/{uuid}/{version:08}/{uuid}.enc`; other
keys are counted as unrecognised and never touched. It reads the storage listing
before the table, so an object written during the run is either too new or already
referenced. It does not run on a schedule; an operator runs it
(`srv:src/services/blob_sweep.rs:1-21,51-64,67-98,117-190`,
`srv:src/main.rs:37-57`, `srv:docs/DEPLOYMENT.md`, section on the sweep).

The CLI says so in plain words after every rekey: rotation stops future reads,
and the secrets themselves must be rotated at their source
(`cli:src/cloud/vault.rs:1415-1428`, `srv:src/routes/rekey.rs:524-532`).

### 7.4 Deleting a version

`DELETE /vaults/{id}/versions/{n}` needs admin or above on the vault
(`srv:src/routes/versions.rs:482-565`):

1. It takes the vault row `FOR UPDATE`, so a push cannot land in the middle and
   two admins cannot empty the vault between them.
2. It refuses (409) only when the version is the vault's **only** version.
   Any other version can be deleted, including the latest.
3. If the latest is deleted, the previous version becomes the latest. The next
   push is numbered `latest + 1`, so it reuses the deleted number.
4. It commits the row delete, then records a `version_deleted` audit event
   (`version_num`, `was_latest`, `latest_after`), then deletes the blob. The event
   is awaited, but a failed write is only logged. A failed blob delete is logged
   and leaves an unreferenced object (7.3).
5. The response carries `deleted`, `latest` (after the deletion) and `was_latest`.

Deleting a version makes room under the plan's version limit (6.4).

The 0.14.0 CLI does not use the new case. `evnx cloud delete-version` needs a
signed-in session and still refuses the latest version before it calls the
server (`cli:src/cloud/sync.rs:1355-1369,1397-1404`). An admin can delete the
latest through the API.

A machine that had seen the deleted number will see `latest` go down. The
high-water check (8.1) warns, and its message names an admin deleting the latest
version as one cause (`cli:src/cloud/highwater.rs:72-86`).

## 8. Client-side checks

| Check | What it does | Source |
|---|---|---|
| AEAD tag and AAD on a version | A blob for another vault or version fails to decrypt | `cli:src/cloud/sync.rs:706-724` |
| Low-order X25519 key | Refused on wrap and unwrap | `crypto:keypair.rs:643,705` |
| Half a wrap | Refused (ephemeral without ML-KEM, or the reverse) | `cli:src/cloud/sync.rs:999-1013` |
| `X-Blob-Hash` | Compared to the downloaded bytes if the header is present | `cli:src/cloud/sync.rs:674-687` |
| Server proof `M2` at login | Login aborts if it does not verify | `cli:src/cloud/auth.rs:389-398` |
| Plain `http://` | Refused except loopback; userinfo in the URL refused | `cli:src/cloud/config.rs:149-190` |
| Rotation self-check | Unwraps its own output before sending | `cli:src/cloud/rotate.rs:14-21` |
| Key bundle | Refused if the prefix, the version, the base64url body or a field is wrong | `cli:src/cloud/keybundle.rs:66-92` |

### 8.1 The RM-89 high-water mark

The CLI keeps `high-water.json` in the config directory: vault id (lowercased) to
the highest version number this machine has seen. No name, key, value or hash.
Mode 0600 on Unix (`cli:src/cloud/highwater.rs:1-13,47-70`).

- `observe(latest)` runs when a command reads `versions/latest`. If `latest` is
  **lower** than the mark, it prints a warning on stderr naming both numbers. The
  mark is kept, so the warning repeats. Otherwise the mark is raised
  (`cli:src/cloud/highwater.rs:72-101`, `cli:src/cloud/sync.rs:938-957`).
- A 404 on `latest` counts as `latest = 0`.
- `latest` also goes down when an admin deletes the latest version (7.4). The
  warning fires then too, and its text names that as one possible cause.
- `note(version)` raises the mark after a push, and after a blob download
  (`cli:src/cloud/sync.rs:414,691`).
- `vault delete` clears the entry (`cli:src/cloud/vault.rs:372`).
- It is a hint. It never stops a command, and a failure to read or write the file
  is ignored.

Limits, from the code:

- Per machine. A new machine, a fresh CI runner, or a deleted file starts at 0.
- It catches "latest went backwards". It cannot catch a rollback to a version at
  or above the mark, or a freeze (the server keeps reporting the same latest).
- The mark is raised by a blob download before decryption succeeds
  (`cli:src/cloud/sync.rs:691`). A server can serve a blob when the client asks
  for `--version N` and so raise the mark. The effect is a repeating false
  warning, not a bypass.
- `pull --version N` does not read `latest`, so it does not warn.
- The file is plain JSON owned by the user. A local attacker can edit it.

It is the client-only part of rollback detection. The server keeps no mark. See
THREAT_MODEL, section 5.

## 9. What the server stores

"Plaintext" means the server can read the value. "Ciphertext" means it holds
bytes it cannot open without a key it never receives.

| Where | Column or blob | Plaintext or ciphertext | Notes |
|---|---|---|---|
| `users` | `email` | plaintext | lowercased |
| `users` | `srp_verifier`, `srp_salt` | verifier: password-derived, not a password | offline-guessable material |
| `users` | `argon2_salt` | plaintext, public | |
| `users` | `ed25519_public_key`, `x25519_public_key`, `mlkem_public_key` | plaintext, public | `mlkem_public_key` nullable |
| `users` | `encrypted_private_key` | ciphertext | the Ed25519 seed under the master key; returned by `/auth/me` to a session, and carried in a key bundle |
| `users` | `totp_secret_enc` | ciphertext | `v1:` + base64(`nonce24 \|\| XChaCha20-Poly1305`), key derived from `SERVER_DATA_KEY`, user id as AAD. A value written before sealing existed is bare base32; it is re-saved sealed the next time it is used (below) |
| Cache (Valkey) | `totp_setup:{user_id}` | **plaintext** base32 | the secret between `totp/setup` and `totp/confirm`, 600 seconds, then deleted on confirm |
| `users` | `plan`, `is_active`, timestamps | plaintext | |
| `vaults` | `name`, `environment` | plaintext | name is `[a-z0-9-]`; environment is one of four |
| `vault_members` | `role` | plaintext | |
| `vault_members` | `encrypted_vault_key` | ciphertext | a wrap |
| `vault_members` | `eph_pub_key` | public key | NULL for an own copy |
| `vault_members` | `mlkem_ciphertext` | KEM ciphertext, public | NULL for an own copy |
| `vault_members` | `granted_by`, `granted_at` | plaintext | who shared, when |
| `vault_versions` | `version_num`, `pushed_at`, `blob_size_bytes` | plaintext | size is plaintext length + 28 |
| `vault_versions` | `key_names`, `key_count` | **plaintext** | the `.env` key names |
| `vault_versions` | `blob_hash` | plaintext hash of ciphertext | |
| `vault_versions` | `pushed_by` | plaintext user id | NULL after that account is deleted. An API-token push is recorded under the token owner's id |
| `vault_versions` | `blob_key` | plaintext pointer | |
| Object storage | the blob `nonce12 \|\| ciphertext` | ciphertext | |
| `api_tokens` | `token_hash` | BLAKE3 of the token | token itself is shown once |
| `api_tokens` | `name`, `scope`, `vault_id`, `expires_at`, `last_used_at` | plaintext | |
| `refresh_tokens` | `token_hash` | hash | |
| `totp_backup_codes` | `code_hash` | BLAKE3 hash | |
| `audit_events` | `event_type`, `vault_id`, `user_id`, `metadata`, `created_at` | plaintext | append-only by trigger |
| `audit_events` | `ip_hash`, `user_agent_hash` | keyed BLAKE3 digest | key is `AUDIT_HASH_KEY`, falling back to `JWT_SECRET` |
| `master_key_rotations` | `prev_*`, `prev_wraps` | old verifier, salts, sealed key, wraps | written for a `routine` rotation, only while the undo window is open. When the snapshot closes these columns are set to NULL; the row keeps `rotated_at`, `consumed_at` and `outcome` (3.5) |
| `organizations`, `organization_*` | names, slugs, seats, invites | plaintext | billing and directory; cannot grant vault access |

**The server never receives:** the password, the master key, the Argon2 output,
any vault key, any private key, any plaintext `.env` value.

**The server can never see (without a client's help):** the values. Everything
else in the table it sees.

Source: `srv:migrations/001_initial.sql:9-103`, `002`, `003`, `004`, `006`,
`007`, `009`, `010:200-201`, `015:15-61`, `srv:src/db/rotation.rs:190-250,351-362`;
`srv:src/routes/versions.rs:96-107`; `srv:src/services/secrets.rs:12-25,126-157`;
`srv:src/routes/auth.rs:268-306,1138-1143,1183-1190`;
`srv:src/middleware/client_ip.rs:15-30`; `srv:src/routes/tokens.rs:99-104`.

**The TOTP key.** `SERVER_DATA_KEY` is held in the server's environment, not in
the database. A copy of the database alone does not open a sealed TOTP secret.
Someone who can read both the database and the server environment can. The
secret is sealed with the user id as AAD, so a sealed value copied onto another
account's row does not open. The server upgrades an old bare value only when that
secret is used, and only if it is valid base32. It is not upgraded in bulk
(`srv:src/services/secrets.rs:21-25`, `srv:src/routes/auth.rs:268-306`). Outside
development the server refuses to start without the key
(`srv:src/config.rs:385-400`).

A member can read `key_names`, `pushed_by`, `pushed_at` and the member list
(roles, emails, `granted_by`). The audit view hides `ip_hash` and
`user_agent_hash` (`srv:src/routes/vaults.rs:262-267`).

## 10. Versioning and compatibility

**There is no protocol version number.** Nothing in the blob, the AAD, the wrap
formats or the HTTP bodies carries one. The only visible versions are:

- the URL prefix `/api/v1` (`cli:src/cloud/client.rs:1009`);
- version strings inside HKDF `info` and Argon2 domain tags (`-v1`, `-v2`), which
  act as format versions for their own key;
- the key bundle's prefix `evnxkb1_` and its `v` field
  (`cli:src/cloud/keybundle.rs:29-32`), and the `v1:` prefix of a sealed TOTP value
  (`srv:src/services/secrets.rs:38-39`). Both belong to a single value, not to the
  vault protocol;
- the crate version.

Compatibility facts the code shows:

| Subject | Fact | Source |
|---|---|---|
| Blob and AAD format | Fixed. Changing it breaks every stored blob. | `crypto:vault.rs:123-129` |
| Argon2 costs and domain tags | Fixed per account. Changing them invalidates every existing account. | `crypto:kdf.rs:34-40` |
| X25519 and ML-KEM derivation | Fixed forever | `crypto:keypair.rs:82-92` |
| Shared wrap v1 (X25519 only) | Not openable by this client. The sharer must re-share. | `cli:src/cloud/sync.rs:999-1008` |
| Server rows with ephemeral but no ML-KEM | Deleted by migration 016, which then validates the CHECK for every row. The client still refuses such a wrap. | `srv:migrations/016_wrap_is_whole_is_validated.sql:51-75`, `srv:migrations/004_mlkem_hybrid.sql:38-68` |
| Account without an ML-KEM key | Cannot be shared with or rekeyed around | `srv:src/routes/members.rs:75-94` |
| Upper-case vault id pushes before C10 | Open only with the same spelling, by 0.13.0 or earlier | `cli:CHANGELOG.md:320-328` |
| Crate versions | CLI links `evnx-crypto` 0.2.1; the server's lockfile has 0.1.0 and `src/` does not import it (tests do) | `cli:Cargo.lock:942-943`, `srv:Cargo.lock:885-886`, `srv:src/` |
| Old servers | The client works without `X-Blob-Hash` and without some `/me` fields | `cli:src/cloud/sync.rs:674-679` |

Because nothing is versioned in-band, a change to any format has to be
coordinated by hand between client, server and every stored object.

## 11. Not settled by the code

- **Whether the server's vault id is always lowercase.** The CLI comment says it
  is (`cli:src/cloud/vault.rs:74-75`). The server stores a Postgres `UUID` and
  returns it through serde. That produces lowercase hyphenated text, but no test
  or comment in the server asserts it.
- **Whether storage encrypts at rest.** `object_store` backends are configured
  by environment (`srv:src/services/storage.rs`). The code sets no server-side
  encryption option. Deployment may.
- **TLS termination.** The server sets HSTS only in production
  (`srv:src/middleware/security_headers.rs:51-59`). Where TLS ends is
  deployment, not code. See `srv:docs/DEPLOYMENT.md`.
- **The role of the Ed25519 key.** See 3.2.
- **The value of `AUTH_ME_SESSION_ONLY` on a given server.** The code default is
  `false`. The maintainers state that the evnx cloud service has run with `true`
  since 2026-11-13. That is a deployment setting, not something the code shows.
- **Whether a deployed database still holds TOTP secrets in the old bare form.**
  The server re-saves one only when it is used (9).
- **Whether the blob sweep has been run on a deployed bucket.** The command exists
  (7.3). The code does not run it.

## 12. Changes in 0.14.0

What changed in the protocol and the service for v0.14.0 (CLI) and evnx-server
`34b664f`. The vault formats did not change: the blob layout, the AAD, the wrap
formats and the Argon2 and SRP parameters are as before, and the CLI links the
same `evnx-crypto` 0.2.1 as 0.13.0 (`cli:Cargo.lock:942-943`). No vault had to be
re-encrypted and no password had to change.

| Area | Change | Section |
|---|---|---|
| CI key material | New key bundle `EVNX_KEY_BUNDLE` (`evnxkb1_`), made by `evnx auth key-bundle` or `evnx auth token create`. A token plus the bundle plus the master password unlocks a vault. | 3.4.1 |
| `GET /auth/me` | Sessions get the full response. API tokens get 403 when `AUTH_ME_SESSION_ONLY=true`, and only the sealed seed and salt when it is `false`. | 3.4.2 |
| Sign-in start | An address with no account gets salts derived from the address, the same on every call. | 3.3 |
| TOTP secret | Sealed at rest under a key derived from `SERVER_DATA_KEY`. | 2, 9 |
| Master-password undo | The previous material is erased when the snapshot closes. | 3.5 |
| Shared wraps | Rows holding half a wrap were removed; the database check is validated for all rows. | 4.2 |
| Push | Nonce must be 12 bytes and hash 64 lowercase hex. The version limit is counted inside the transaction. The `push` audit event is written in the same transaction. | 5.5, 6.1, 6.4 |
| Transport hash | Computed in one server function. | 5.4 |
| Rekey | Staged blobs are checked for name, existence, size and hash before the swap. Replaced blobs are deleted after it. `evnx-server sweep-blobs` removes unreferenced objects. | 7.2, 7.3 |
| Delete a version | The server accepts the latest version when an older one exists. | 7.4 |

For existing installs: a CLI older than 0.14.0 that unlocks with a token calls
`/auth/me`. It gets 403 on a server that has `AUTH_ME_SESSION_ONLY=true`, and
needs 0.14.0 and a key bundle. A signed-in session works with any version.
