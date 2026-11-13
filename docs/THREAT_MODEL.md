# Threat model for cloud vaults

What evnx cloud vaults protect, from whom, and where they stop. It describes
evnx v0.14.0 against evnx-server at the commit pinned in PROTOCOL.md. A reference
like "P5.2" means section 5.2 of [PROTOCOL.md](./PROTOCOL.md), which also defines
the `cli:`, `srv:` and `crypto:` source prefixes and the pinned commits. What
changed in 0.14.0 is in [section 7](#7-changes-in-0140).

Short version. The server holds ciphertext, wrapped keys and public keys. It never
receives a password, a master key, a vault key or a plaintext value. That holds
against a database dump, a storage leak and a passive network observer. It does
**not** hold against a server that actively lies about keys: such a server can
supply the key your next push is encrypted under, or a different public key for a
recipient ([section 5](#5-known-limits)). Treat cloud sync as beta
(`cli:SECURITY.md:89-91`).

## Contents

1. [Assets](#1-assets)
2. [Actors and trust boundaries](#2-actors-and-trust-boundaries)
3. [Threats](#3-threats)
4. [Non-goals and assumptions](#4-non-goals-and-assumptions)
5. [Known limits](#5-known-limits)
6. [Planned work](#6-planned-work)
7. [Changes in 0.14.0](#7-changes-in-0140)

## 1. Assets

| Asset | Where it lives | Protected by | Worst case if it leaks |
|---|---|---|---|
| Secret values (the `.env` contents) | Only on user devices, in memory and in files the user writes. At rest elsewhere: ciphertext. | AES-256-GCM under the vault key (P5) | The secrets are exposed. Rotate them at the source. |
| Vault key (32 bytes, one per vault) | Never stored in clear. One wrapped copy per member on the server. In memory on a client while a command runs. | Wrapped under the owner's master key, or hybrid X25519 + ML-KEM-768 to each member (P4) | Whoever holds it opens every version of that vault, old and new. |
| Master password | The user's head, a password manager, or a CI secret | Never sent. Used for Argon2id, then dropped (P3.4) | Full account access, including every vault key the account holds. With TOTP off, it is also a full login. |
| Master key | Client memory only | Argon2id 64 MiB, t=3, p=4 (P2) | Same as the password, for decryption. |
| Identity seed (Ed25519 seed; X25519 and ML-KEM keys derive from it) | Server: sealed under the master key. Client: memory. | XChaCha20-Poly1305 under an HKDF subkey of the master key (P3.1, P3.2) | Opens every shared wrap addressed to that account. |
| SRP verifier and salts | Server database | Argon2id + SRP group (P3.1) | Lets an attacker guess passwords offline (T8). |
| Session tokens (access JWT, refresh token) | `~/.config/evnx/credentials.json`, mode 0600 | File permissions only (`cli:src/cloud/creds.rs:1-17`) | API access as the user for 15 minutes (access) or 30 days (refresh). No decryption. |
| API token (`evnx_tok_...`) | CI secret store; hash on server | Scope (read, read_write), optional single-vault pin, optional expiry (P3.4). The server does not give a token key material (P3.4.2). | API access within its scope. No decryption. |
| TOTP secret | Server: sealed with a key from `SERVER_DATA_KEY`, which is in the server environment (P9); the user's authenticator | XChaCha20-Poly1305, user id as AAD | With the database and the server environment both read, the second factor is lost. Vault confidentiality is not affected. |
| Key bundle (`EVNX_KEY_BUNDLE`) | CI secret store | The sealed seed and Argon2 salt it holds are openable only with the master password (P3.4.1) | The same offline-guessing target a signed-in session can read (T8). No decryption without the password. |
| Metadata | Server: vault names, environments, key names, sizes, times, members, roles, pushers, audit events | Access control and TLS only. Not encrypted. | Reveals what exists and who touches it (T9). |
| Audit trail | Server | Append-only trigger (`srv:migrations/006_audit_is_append_only.sql`) | Loss of evidence. |
| Release artefacts | GitHub Releases, npm, PyPI, crates.io, Homebrew, Scoop, winget, ghcr.io | Signed provenance, publishing credentials (T10) | A malicious client reads secrets before encryption. |

## 2. Actors and trust boundaries

| Actor | Trusted with | Not trusted with | Source |
|---|---|---|---|
| **User device** | Plaintext, master password, all keys while a command runs | Nothing is protected against code running here (T6) | `crypto:lib.rs:33-35` |
| **evnx server** | Availability, access control, ordering of versions, ciphertext custody | Keys, plaintext. And, by design, nothing it says about keys, wraps or "latest" | `crypto:lib.rs:14-20` |
| **Operator** (whoever runs the server, has the database and storage credentials) | Same as the server, plus metadata | Same as the server. Can write the database (section 5), serve old data (T2), read all metadata. | `cli:SECURITY.md:68-87` |
| **Object storage** | Holding bytes | Reading them (they are ciphertext). Not trusted to keep or to be correct. | `srv:src/services/storage.rs` |
| **Network** | Carrying TLS | Content. The client refuses plain http except loopback. | `cli:src/cloud/config.rs:149-190` |
| **Other members** | Whatever their role can read: all versions of that vault, including ones pushed before they joined | Admin acts above their role. Cannot be stopped from copying what they read (T5). | `srv:src/middleware/vault_role.rs` |
| **Removed members** | Nothing new after a rekey. Copies they already hold stay with them. | Everything pushed after a rekey-and-remove | P7 |
| **CI runner** | The master password (if the pipeline decrypts), a token, the key bundle, plaintext during the run | Account management. A token cannot mint tokens, enrol TOTP or share (T7). | `cli:src/cloud/token.rs:27-34` |
| **Email provider, billing provider (Paddle)** | Delivering mail; payment data | Vault data. They hold none. | `srv:migrations/010_organizations.sql:200-201` |
| **Web app (app.evnx.dev)** | Not examined. Neither repository contains it. The crypto crate has a `wasm` feature. | No claim is made. | `crypto:Cargo.toml` features |

Trust boundaries, in order of importance:

1. **Client / server.** Everything crossing it is attacker-controlled data as far
   as the client's crypto is concerned (`crypto:lib.rs:14-20`).
2. **Client / local disk.** The credentials file and the high-water file are
   plain files. Pulled `.env` files are plaintext on disk.
3. **Server / storage and database.** A breach of either is assumed possible.
4. **Account / vault.** The vault key is the boundary. A token or a session is
   not.

## 3. Threats

Each row names what protects against it, with a protocol section, and what remains.
Limits that remain are in [section 5](#5-known-limits).

### T1. Server database or storage breach

| Threat | Protection | Residual risk |
|---|---|---|
| Attacker reads the database | Values: none stored. Vault keys: only wrapped copies (P4). Private seed: sealed (P3.1). Tokens and backup codes: BLAKE3 hashes (P9). TOTP secrets: sealed under a key that is not in the database (P9). Shared wraps without an ML-KEM half: removed, and the database refuses them (P4.2). | Verifiers and sealed seeds are offline-guessable (T8). Emails, vault names, key names, sizes, times, members and roles are readable (T9). A TOTP secret opens for someone who can read both the database and the server environment. A TOTP secret not used since the server was updated may still be in the old unsealed form (P9). |
| Attacker reads object storage | Blobs are AES-256-GCM ciphertext under per-vault keys (P5). Objects replaced by a rekey are deleted after the swap commits (P7.2). | Sizes, object keys and write times leak. A failed delete leaves an unreferenced object until the operator runs `evnx-server sweep-blobs --apply` (P7.3). Objects left by earlier rekeys are found by the same command. |
| Attacker reads both | Nothing decrypts without a password. | Same as above. A weak master password is the whole risk. |
| Attacker can **write** the database | Ciphertext cannot be forged: the GCM tag fails. | They can swap wraps and supply the key your next push uses (5.2). |
| Master-password rotation snapshots | A `routine` rotation stores the previous verifier, salts, sealed seed and wraps for the undo window only. When the snapshot closes (window over, restored, or superseded) they are set to NULL, and a table constraint refuses a closed row that holds them (P3.5; `srv:migrations/015_rotation_snapshots_are_audit_only.sql:15-61`, `srv:src/db/rotation.rs:190-250`). | Inside the window (default 72 hours) the old material is in the database. After it, an hourly sweep erases it, and a snapshot past its window cannot be restored in the meantime. |

### T2. Malicious or compromised server operator

| Threat | Protection | Residual risk |
|---|---|---|
| Reads secrets | Not possible without a master password (P3, P5). | None known beyond the client being replaced (T10). |
| Serves a blob for the wrong vault or version | AAD binds `(vault_id, version)`. Decrypt fails (P5.2, P8). | None. |
| Serves an altered blob | GCM tag fails. | Denial of service only. |
| Serves an **older real version** as latest | AAD does not help: version k is genuine for k. RM-89 warns if the machine has seen a higher one (P8.1). | Not caught on a machine that has not seen a higher version, on a fresh CI runner, after the file is lost, or for a freeze at the mark (5.3). |
| Silently drops a push it acknowledged | The pusher's mark records the new number; the next `latest` is lower, so it warns (P8.1). | Only on the pusher's machine. |
| Hides or deletes versions | `delete_version` needs admin and is audited with `was_latest`. It refuses only the last remaining version (`srv:src/routes/versions.rs:482-565`, P7.4). | The operator has direct database access. RM-89 warns only if latest drops, which an admin deleting the latest also causes. |
| Replaces a wrap, so your next push is encrypted under a key the operator chose | The client refuses half-wraps and low-order keys (P8). | **Not prevented.** Wraps are unauthenticated, and the client takes "own copy or share" from two server fields (5.2). Already in `cli:SECURITY.md:68-87`. |
| Gives a sender the wrong recipient public key | None. | Not prevented. The client shows no fingerprint and pins nothing (5.1, 5.4). |
| Rewrites the audit log | Trigger blocks update and delete (`srv:migrations/006`). | The table owner can drop the trigger (`006:65-70`) (5.6). |
| Learns who talks to whom and when | TLS hides contents from others, not from the server. | Accepted (T9). |

### T3. Network attacker

| Threat | Protection | Residual risk |
|---|---|---|
| Reads or alters traffic | TLS via rustls; plain `http://` refused except loopback; credentials in the URL refused (`cli:src/cloud/config.rs:149-190`, `cli:Cargo.toml:114`). The server sets HSTS in production (`srv:src/middleware/security_headers.rs:51-59`). | Trusted roots can be extended from `SSL_CERT_FILE` and `SSL_CERT_DIR` (`cli:src/core/http.rs:8,26-28`). No certificate pinning in the code. A rogue root lets a MITM act as the server (T2). |
| Impersonates the server at login | SRP mutual proof: the client aborts if `M2` does not verify (`cli:src/cloud/auth.rs:389-398`). | Only protects login. Afterwards, bearer tokens ride on TLS alone. |
| Steals a bearer token in transit | TLS. Access tokens last 15 minutes by default. | A stolen refresh token lasts 30 days unless revoked. |
| Captures the password | Not sent. SRP proves knowledge. | None known. |
| Records traffic for later decryption | Own copies use symmetric keys. Shared wraps are hybrid with ML-KEM-768 (P4). | None known in the wrap formats. |

### T4. Removed member, before and after rekey

| Case | Protection | Residual risk |
|---|---|---|
| After bare removal (`--no-rekey`) | Their API access ends: non-members get 404. | They keep the vault key. It opens every version, including future ones, for anyone who can get the ciphertext (P7.1). The CLI warns. |
| After revoke with rekey | New key, every version re-encrypted, every remaining member re-wrapped, leaver deleted, all in one transaction (P7.2). Future versions are unreadable to them. Replaced objects are deleted afterwards. | They can read old ciphertext they kept, with the old key. |
| Revoked while pushing | C4 locks: the push gets 403, or the revocation waits (P6.3). | None known. |
| Push lands during a rekey | Rekey takes the vault row `FOR UPDATE`. Stale sets get 409 (P7.2). | Retry needed. Staged blobs become unreferenced objects until swept (P7.3). |
| Rekey swap names the wrong object | Before the swap the server checks that each `blob_key` has the shape staging gives this vault and version, that the object exists, and that its size and ciphertext hash match. The swap commits whole or not at all (P7.2; `srv:src/routes/rekey.rs:229-297`). | The server cannot check that the bytes are valid ciphertext under the new key. After a swap the replaced blobs are deleted. An admin can already delete versions or the vault (T6). |
| Revoked member's access to old data via the API | None needed: 404. | See T5 for what they already hold. |

### T5. A member copying values before removal

| Threat | Protection | Residual risk |
|---|---|---|
| Member reads, then copies, secrets | None possible. Anyone with the key has the plaintext. | Unmitigated by design. Rotate the secrets at their source. The CLI says so after every revoke and rekey (`cli:src/cloud/vault.rs:1415-1428`). |
| A new member reads history | None. One key covers all versions. | A new member can read every version, not only later ones. |
| Detection | Pulls are audited with the user id; members can read the trail (`srv:src/routes/vaults.rs:249-327`). | Retention shown by the API depends on plan (`vaults.rs:280-301`). A pull by a stolen token is attributed to the token's owner. |
| A developer pushes bad values | Roles: developer or above can push (P6.1). | No signature says who pushed. `pushed_by` is the server's claim (5.2). |

### T6. Stolen laptop or compromised client

| Threat | Protection | Residual risk |
|---|---|---|
| Disk stolen, locked | `credentials.json` is 0600, not encrypted (`cli:src/cloud/creds.rs:13-24`). The master key and vault keys are never stored. | Relies on disk encryption. The thief gets API access until the session is revoked. |
| What a stolen session can do | Reads ciphertext and wraps. Cannot decrypt. Cannot share or rekey (needs the vault key). Rotating the master password needs a fresh SRP proof and TOTP (`srv:src/routes/master_key.rs:20-40`). | It can read the sealed seed and Argon2 salt from `/auth/me` (T8), mint API tokens (user-session routes), delete versions as admin, delete a vault as owner, and push data that does not decrypt (5.5). Revoke sessions to end it. |
| Master password also stolen | None. | Full compromise of that account. TOTP, if on, still gates login. |
| Malware or a hostile local process | Out of scope (`crypto:lib.rs:33-35`). Keys are zeroized on drop, best effort (`cli:src/cloud/creds.rs:30-35`). | Reads plaintext and the typed password. |
| Plaintext on disk after `pull` | `.gitignore` handling for outputs (`cli:src/cloud/sync.rs:612-618`). | The file is plaintext until the user deletes it. `cloud run` avoids the file and puts values in the child's environment (`cli:src/cloud/run.rs:1-29`). |
| Local edit of `high-water.json` | Mode 0600. | A local attacker can reset it. It is a hint. |

### T7. Leaked CLI token

| Threat | Protection | Residual risk |
|---|---|---|
| Token used to read secrets | Cannot decrypt: the vault key is under the master key (`cli:src/cloud/token.rs:3-17`). | The attacker gets ciphertext and wraps for every vault in scope. |
| Token used beyond its scope | Read-only tokens cannot mutate; a vault-pinned token reaches only that vault, and `GET /vaults` and `POST /vaults` are refused (`srv:src/middleware/auth.rs:321-364`). Account routes refuse tokens. A token cannot mint tokens, enrol TOTP or share. `GET /auth/me` answers a token with 403 when `AUTH_ME_SESSION_ONLY=true` (P3.4.2). | `GET /users/{email}/public-key` accepts any token and ignores its scope; it returns public keys only (`srv:src/routes/mod.rs:337-342`, `srv:src/middleware/auth.rs:251-262`). With `AUTH_ME_SESSION_ONLY=false`, the code default, a token reads the sealed seed and Argon2 salt from `/auth/me`. The maintainers' cloud service has used `true` since 2026-11-13; a server you run needs the setting. |
| Key bundle leaked | The bundle holds the sealed seed and the salt, not a key. It needs the master password to open (P3.4.1). | It is the offline-guessing target of T8, and it is the same material a signed-in session reads. It identifies the account (`user_id`). After a master-password change the old bundle no longer opens. |
| Token used to write | A `read_write` token can push. | A token alone has no vault key, so it can only push data that does not decrypt. The server cannot tell, and the data becomes the latest version until an admin deletes it or a valid push follows (5.5). Pushes are recorded under the owner's id (`srv:src/middleware/auth.rs:173-179`). |
| Duration | Revocation takes effect on the next request (database lookup per call). Optional expiry, up to 3650 days or none (`srv:src/routes/tokens.rs:23-24,52-64`). | Tokens with no expiry live until revoked. |
| Pipeline also holds the master password | None. | A leaked token, bundle and password together are full decryption of the pinned vault. The password alone, with TOTP off, is a full login (`cli:src/cloud/run.rs:17-23`). |

### T8. Brute force of the master password

| Threat | Protection | Residual risk |
|---|---|---|
| Online guessing | 5 failed SRP proofs lock the email for 900 s; `srp/init` limited to 30 per address per 900 s; TOTP 3 failures (`srv:src/routes/auth.rs:188-218`). | Throttles are per email and per address, not a global cap. |
| Offline guessing after a breach | Each guess costs one Argon2id run with 64 MiB, t=3, p=4. Two independent salts, two domain tags, so cracking the verifier does not reuse work for the master key (P2, P3.1). Minimum 12 characters at account creation (`cli:src/cloud/auth.rs:30-41`). | Targets for the attacker: the SRP verifier (with its salt) or the sealed seed (with its Argon2 salt). The sealed seed and salt are also in a key bundle and in the `/auth/me` answer to a session. No second secret is mixed in (no pepper, no device key). The 12-character floor is checked by the client only (5.8). Costs are constants in the crate, not stored per account, so raising them is a format change (P10). |
| Rotation snapshot | `compromised` rotations keep none (`srv:migrations/009:35-40`). A `routine` rotation keeps the old material only for the undo window and then erases it (P3.5). | Inside the window the old verifier and sealed seed are in the database. A password changed because it was weak is attackable from there until the window closes. Choose `--compromised` when the old password may be known. |
| Password lost | None. By design there is no recovery (`cli:SECURITY.md:95-96`). | Data loss. |

### T9. Metadata leakage

| What the server sees | Why | Source |
|---|---|---|
| Vault name and environment | Stored plaintext (name `[a-z0-9-]`, four environments) | `srv:src/routes/vaults.rs:23-31,90-97` |
| `.env` key names and count, per version | Sent on purpose for listings. A sensitive name should not be a name. | `cli:src/cloud/sync.rs:5-10` |
| Exact plaintext length, per version | No padding: blob size = plaintext + 28 | `srv:src/routes/versions.rs:96-101` |
| Which key names changed between versions | Names per version, hash of ciphertext | `cli:src/cloud/sync.rs:277-283` |
| Who pushed, who pulled, when | `pushed_by`, audit events with user id | `srv:src/routes/versions.rs:242-256,340-360` |
| Which token acted | Recorded under the token owner's id, so a token and its owner look alike | `cli:src/cloud/sync.rs:764-768` |
| Members, roles, who granted whom | Stored plaintext, readable by any member | `srv:src/routes/members.rs:167-219` |
| Who has an account | Any authenticated caller, including a token, gets 200 or 404 from the public-key lookup. Registration answers 409 for an address that has an account. The sign-in start step does not reveal it (P3.3) | `srv:src/routes/users.rs:15-35`, `srv:src/routes/auth.rs:100-103` |
| Network metadata: IPs, user agents, timing | IP and user-agent are stored as keyed BLAKE3 digests. The key falls back to `JWT_SECRET`. | `srv:src/middleware/client_ip.rs:7-30` |
| Request patterns and volume | Inherent | none |

Protection. TLS hides all of it from the network. Nothing hides it from the server.
Members do not see IP or user-agent digests (`srv:src/routes/vaults.rs:262-267`).
Residual risk. All of the above, accepted. See the non-goals.

### T10. Supply chain

| Channel | What the code does | Residual risk |
|---|---|---|
| GitHub Release archives | Each build job signs a SLSA build-provenance attestation with Sigstore through `actions/attest-build-provenance`, one retry on failure. A failed build holds back every other channel (`cli:.github/workflows/release.yml:158-193`, `cli:docs/RELEASING.md:205-235`). SHA-256 files are published (`release.yml:146,155`). | Verifying is opt-in: `gh attestation verify`. The code does not state a SLSA level. |
| SBOM | A `sbom` job installs `cargo-cyclonedx` 0.5.10 with `--locked`, writes `evnx-<version>.cdx.json` (CycloneDX 1.5, all features) and its `.sha256`, attests the document like the archives, and the release job attaches both (`cli:.github/workflows/release.yml:202-253,274`). | Verifying is opt-in (`gh attestation verify`). It lists dependencies; it does not vouch for them. |
| Post-publish check | `verify-provenance` runs the user's command against published assets (`release.yml:402-442`). | It gates nothing (owner decision, `cli:docs/RELEASING.md:237-238`). |
| PyPI | Trusted publishing over OIDC, no stored password; PEP 740 attestations (`cli:.github/workflows/python-publish.yml:258-290`). | Tag push triggers it independently of `release.yml`, so PyPI can publish while the Release does not (`RELEASING.md:223-226`). |
| npm | `npm publish --access public --provenance` (`cli:scripts/npm-publish-packages.sh:68`). Publishes the binaries extracted from the release archives. | Uses a stored `NPM_TOKEN` (`cli:.github/workflows/npm-publish.yml:476-539`). |
| Homebrew, Scoop, winget | Formulas are generated with the SHA-256 read from the release assets (`release.yml:517-529`). | Each uses a stored personal-access token (`HOMEBREW_TAP_TOKEN`, `SCOOP_BUCKET_TOKEN`, `WINGETCREATE_TOKEN`; `release.yml:540,681,797`). Their integrity check is the hash from the same release. |
| Container image | `provenance: true`, `sbom: true` (`release.yml:504-505`). | Same trust in the workflow. |
| Dependencies | `cargo audit` in CI (`cli:.github/workflows/ci.yml:233`), Dependabot, actions pinned by commit SHA. Cargo.lock is committed. | `evnx-crypto` is one maintainer's crate, not in this repository. ML-KEM is a young scheme (`crypto:keypair.rs:520-526`). |
| Server | Not covered by these workflows. The server is a separate repository. | Not examined here. |

A malicious client is the one attack that defeats everything above. It sees the
password and the plaintext. Provenance is the only mitigation, and the user has to
check it.

### T11. Further threats found in the code

| Threat | Protection | Residual risk |
|---|---|---|
| Account takeover through email | Vault routes need a verified email (`srv:src/middleware/auth.rs:342-344`). Login needs the password. | Email cannot reset a password. There is no reset (`cli:src/cloud/auth.rs:35-40`). |
| Replay of a stolen rotation request | Needs a fresh SRP proof of the current password (`srv:src/routes/master_key.rs:20-40`). | A `routine` rotation stays reversible for the window by whoever holds the old password (`009:35-40`). |
| Public-key substitution by session holder | ML-KEM key is write-once and needs a user session (`srv:src/routes/users.rs:62-110`). No endpoint changes the X25519 or Ed25519 key. | A database writer can still change any key (5.1, 5.2). |
| Account enumeration at login | Unknown addresses get a random verifier and salts derived from the address, the same on every call. Proofs for them count toward the same lockout (P3.3; `srv:src/routes/auth.rs:328-352,473-512`). | Registration and the public-key lookup still tell an existing address from an unknown one (T9). |
| Downgrade of a wrap to X25519 only | `v2` HKDF string, database CHECK, server type, client refusal (P4.2, P4.3). Migration 016 removed older rows that held half a wrap and validated the CHECK for every row. | None known. |
| Cross-vault wrap confusion | Not prevented: wraps carry no vault id in their AAD (5.2). | A database writer can move a wrap between rows. Blobs then fail to open, or a push goes under the wrong key. |
| Lost update between two pushers | `base_version`, locks and `UNIQUE` give 409 (P6.1, P6.3). | None known. |
| Quota bypass by concurrent pushes | The version limit is counted again inside the push transaction, after the locks. Two pushes for the same next version cannot both commit (P6.4). | None known. |
| Large or malformed push | Body limit (default 64 KiB) before parsing (`srv:src/lib.rs:36-40`). The nonce must be 12 bytes and the hash 64 lowercase hex characters, on push and on rekey staging (P5.5). | None known. |
| Push without its audit entry | The `push` event is written in the push transaction. If it cannot be written, the push does not happen (P6.1). | Other events are written after the work and a failed write is only logged (5.6). |
| Account deletion with shared vaults | Refused while the account owns a vault others are in (`srv:src/routes/auth.rs:1631-1651`). | None known. |

## 4. Non-goals and assumptions

**Non-goals.** The design does not try to:

- stop a member from copying what they can read (T5);
- recall secrets already read by a removed member (P7.3);
- hide metadata from the server (T9);
- hide the existence of an account or a vault from other authenticated users (T9);
- protect a machine that is already running attacker code (`crypto:lib.rs:33-35`);
- recover a lost master password (`cli:SECURITY.md:95-96`);
- keep the service available if the operator withholds or deletes data;
- give forward secrecy for past versions once a vault key leaks;
- attribute a push cryptographically to one member (5.2).

**Assumptions.** The guarantees hold only if:

- the user's master password is strong and not reused. The code enforces 12
  characters at creation, on the client;
- the device's storage is encrypted, because `credentials.json` and any pulled
  `.env` are not;
- the CLI binary the user runs is the one that was built from the source. They
  verify provenance themselves;
- the OS CSPRNG works, and the RustCrypto and ML-KEM implementations are correct.
  The code references no external audit report of `evnx-crypto`;
- TLS roots on the device are honest;
- the server enforces the access rules and locks described in P6. The client does
  not check them;
- operators keep the database trigger that makes the audit log append-only.

**Scope not examined.** The web app (app.evnx.dev), deployment and hosting of
`api.evnx.dev`, backups, and the billing flow.

## 5. Known limits

These are limits of the current design. They are stated as they are. A future
release adds signed key sharing, key fingerprints (trust on first use) and
rollback protection (section 6).

What a server cannot do under any of them, as far as the code shows: read a
value from a stored version without a vault key. The limits below are about which
keys the client is given and what it is told about the order of versions.

### 5.1 The server supplies recipients' public keys

`evnx vault share` and `evnx vault rekey` ask the server for each recipient's
public keys (`GET /users/{email}/public-key`) and use what it returns. If the
server returned a different key for a recipient, the vault key would be wrapped to
a key the server holds, and its holder could open the wrap and every version of
that vault. The member list that a rekey wraps for also comes from the server.

Source: `cli:src/cloud/vault.rs:524-557,1272-1313,1765-1772`,
`srv:src/routes/users.rs:15-35`, `cli:SECURITY.md:93-94`.

### 5.2 Wraps, membership and rekeys are not signed

- A wrap has no sender signature and carries no vault id. The Ed25519 key is
  registered and signs nothing in this version.
- The client decides whether a vault key is "your own copy" or "shared with you"
  from two fields the server sends (`eph_pub_key`, `mlkem_ciphertext`). It keeps no
  local record of which vaults you created.
- A hybrid wrap needs only the recipient's public keys, which the server holds.
  So a server that can write its database could supply a wrap of a key it chose,
  for a vault you created and never shared. Your next push would be encrypted
  under that key. Versions stored earlier stay closed to it.
- There is no signed record of who is a member, of role changes, or of rekeys.
  `pushed_by`, `granted_by` and the audit trail are the server's statements.

Source: `cli:src/cloud/sync.rs:980-997`, `crypto:keypair.rs:623-675` (the AEAD
has no AAD, `:660-663`), `crypto:vault.rs:270-290` (no AAD), `crypto:lib.rs:100`,
`cli:SECURITY.md:68-87`.

### 5.3 A server can withhold or replay versions

A server could keep returning an older version as the latest, or stop returning
newer ones. Each blob is bound to its version number, so a blob cannot pass as a
different version, but version k is genuine for k.

What the CLI does today (RM-89, PROTOCOL 8.1): it keeps the highest version
number this machine has seen for each vault, and prints a warning on stderr when
the server later reports a lower `latest`. It does not stop the command.

What it does not catch:

- a machine that has not seen the higher version: a new machine, a fresh CI
  runner, or a deleted `high-water.json`;
- a rollback to a version at or above the mark, and a server that keeps reporting
  the same latest after newer pushes;
- `pull --version N`, which does not read `latest`;
- a local attacker, who can edit the file.

The server keeps no mark of its own. A lower `latest` also appears when an admin
deletes the latest version (PROTOCOL 7.4), so the warning is a prompt to look at
`evnx cloud history`, not proof of tampering.

Source: `cli:src/cloud/highwater.rs:1-13,72-110`,
`cli:src/cloud/sync.rs:691,938-957`.

### 5.4 Keys have no fingerprints

The client does not show a fingerprint for a public key, does not ask you to
compare one, and does not remember the key it saw before. Combined with 5.1, a
changed key for a recipient is not noticed.

Source: `cli:SECURITY.md:93-94`, `cli:src/cloud/vault.rs:524-557`.

### 5.5 Writers can push data that does not decrypt

The server cannot check ciphertext. Anyone who may push (developer and above, or a
`read_write` token) can push bytes that do not decrypt, and they become the latest
version. Nothing notices until a client pulls. Recovery: push a valid version, or
have an admin delete the bad one. The server accepts deleting the latest version
when an older one exists (PROTOCOL 7.4). In 0.14.0 `evnx cloud delete-version`
still refuses the latest, so that case needs a direct API call.

Source: `srv:src/routes/versions.rs:45-152,482-565`,
`cli:src/cloud/sync.rs:1397-1404`.

### 5.6 The audit trail

The audit table is append-only through a trigger. The table owner can drop the
trigger. The `push` event is written in the push transaction. Events for pulls,
rekeys, membership changes, version deletions and account deletion are written
after the work, and a failed write is logged and does not fail the operation.

Source: `srv:migrations/006_audit_is_append_only.sql:65-70`,
`srv:src/routes/versions.rs:242-256,340-360,525-545`,
`srv:src/routes/rekey.rs:494-505`, `srv:src/services/audit.rs:15-48`.

### 5.7 No protocol version inside the data

Blobs, wraps and requests carry no version field, so a format change has to be
coordinated by hand between clients, the server and stored data (PROTOCOL 10).

### 5.8 Master-password strength is checked by the client

The 12-character minimum is enforced by the CLI when it creates an account or
changes a password. The server never sees the password and cannot check it
(`cli:src/cloud/auth.rs:30-41`).

### 5.9 Not settled by the code

The code does not settle whether the server's vault ids are always lowercase,
whether object storage encrypts at rest, where TLS ends in a deployment, what a
given deployment sets `AUTH_ME_SESSION_ONLY` to, whether old TOTP secrets are
still unsealed in a given database, or whether the blob sweep has been run
(PROTOCOL 11).

## 6. Planned work

A future release adds three things that address the limits above: signed key
sharing (5.1, 5.2), key fingerprints with trust on first use (5.1, 5.4), and
rollback protection (5.3). No version or date is promised here.

## 7. Changes in 0.14.0

What changed in the model with v0.14.0 and the matching server. PROTOCOL
section 12 has the protocol side.

| Area | Now | Where |
|---|---|---|
| API tokens | A token is not given the sealed seed or the salt. CI carries a key bundle. `GET /auth/me` is session-only when the server setting is on. | T7, T8, P3.4 |
| TOTP secret | Sealed at rest under a key held outside the database. | T1, P9 |
| Replaced data | Blobs replaced by a rekey are deleted. Closed rotation snapshots are erased. `evnx-server sweep-blobs` removes unreferenced objects. | T1, T4 |
| Rekey | The server checks each staged blob before the swap. | T4 |
| Pushes | Nonce and hash sizes are checked. The plan's version limit is counted in the transaction. The push event is written in the transaction. | T11 |
| Sign-in start | Unknown addresses get stable, address-derived salts. | T11 |
| Shared wraps | Rows holding half a wrap are removed, and the database refuses them. | T3, T11 |
| Deleting versions | The server accepts the latest version when an older one exists. | T2, 5.5 |
| Known limits | Unchanged by 0.14.0: 5.1 to 5.4. | 5 |
