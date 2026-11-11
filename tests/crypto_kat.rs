//! Known-answer tests for every primitive evnx's cryptography stands on, and
//! golden vectors for evnx's own envelope (SEC-1).
//!
//! # Why this exists
//!
//! A round-trip test passes against a wrong-but-consistent implementation: swap
//! a nonce width, an HKDF `info` string, an AAD layout or an Argon2 parameter,
//! and `decrypt(encrypt(x)) == x` still holds while every vault ever stored
//! becomes unopenable. Only a fixed answer catches that.
//!
//! # Where `evnx-crypto` lives
//!
//! `evnx-crypto` is its own repository, published on crates.io and consumed here
//! as a version-pinned dependency (the `cloud` feature); it is not a member of
//! this repository. These tests therefore sit in evnx and run against exactly
//! the crate versions in `Cargo.lock`, which is the build users get: a bump of
//! `evnx-crypto` or of a RustCrypto crate that changes an answer fails here.
//!
//! # Sources of the vectors
//!
//! | Test | Primitive | Published in |
//! |---|---|---|
//! | `blake3_*` | BLAKE3 hash, keyed hash, derive_key | the BLAKE3 team's `test_vectors/test_vectors.json`, copied byte for byte to `tests/fixtures/crypto/blake3_test_vectors.json` (CC0-1.0 / Apache-2.0) |
//! | `hkdf_sha256_*` | HKDF-SHA256 | RFC 5869, Appendix A, test cases 1 and 3 |
//! | `x25519_*` | X25519 | RFC 7748, section 5.2 (two vectors) and section 6.1 (Alice and Bob) |
//! | `ed25519_*` | Ed25519 | RFC 8032, section 7.1, TEST 1 |
//! | `chacha20poly1305_*` | ChaCha20-Poly1305 | RFC 8439, section 2.8.2 |
//! | `xchacha20poly1305_*` | XChaCha20-Poly1305 | draft-arciszewski-xchacha-03, appendix A.1 (the draft `draft-irtf-cfrg-xchacha` continues) |
//! | `aes256gcm_*` | AES-256-GCM | McGrew and Viega, "The Galois/Counter Mode of Operation (GCM)", Test Case 16 (also in NIST's GCM validation set) |
//! | `argon2id_*` | Argon2id v1.3 | RFC 9106, section 5.3 |
//! | `envelope_*` | evnx's own format | `tests/fixtures/crypto/envelope_golden.json` |
//!
//! The RFC texts are not reachable from the build machine, so the hexadecimal
//! values for RFC 5869, 7748, 8032, 8439, 9106, the XChaCha draft and the GCM
//! paper were written from the published documents and then checked two ways:
//! each must reproduce under the implementation (a wrong digit cannot), and the
//! RFC 5869, RFC 8439, XChaCha draft and RFC 9106 values were compared with the
//! copies in the upstream crates' own test suites (`hkdf`, `chacha20poly1305`
//! and `argon2`). The BLAKE3 file was fetched from the BLAKE3 repository itself.
//!
//! ML-KEM-768 has no official vector here: its FIPS 203 vectors are
//! kilobytes each, and `evnx-crypto` pins its derivation with its own known
//! answer. What evnx owns is pinned below instead: the public key derived from
//! a stored private key, and a hybrid X25519 plus ML-KEM wrap that must still
//! unwrap.
#![allow(clippy::too_many_arguments)]

use std::path::PathBuf;

fn unhex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    assert!(s.len() & 1 == 0, "odd hex length: {s}");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or_else(|_| panic!("bad hex {s}")))
        .collect()
}

fn tohex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn fixture(name: &str) -> String {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "crypto",
        name,
    ]
    .iter()
    .collect();
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

// ── BLAKE3 ──────────────────────────────────────────────────────────────────

/// The input of every case: 0, 1, ... 250, 0, 1, ... for `len` bytes.
fn blake3_input(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn blake3_vectors() -> serde_json::Value {
    serde_json::from_str(&fixture("blake3_test_vectors.json")).unwrap()
}

fn blake3_xof(h: blake3::Hasher, n: usize) -> String {
    let mut out = vec![0u8; n];
    h.finalize_xof().fill(&mut out);
    tohex(&out)
}

#[test]
fn blake3_hash_matches_the_official_vectors() {
    let doc = blake3_vectors();
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 35);
    for case in cases {
        let len = case["input_len"].as_u64().unwrap() as usize;
        let want = case["hash"].as_str().unwrap();
        let mut h = blake3::Hasher::new();
        h.update(&blake3_input(len));
        assert_eq!(blake3_xof(h, want.len() / 2), want, "hash, {len} bytes");
        // The 32-byte form is what `blake3::hash` (and evnx's blob_hash) gives.
        assert_eq!(
            blake3::hash(&blake3_input(len)).to_hex().as_str(),
            &want[..64],
            "hash (32 bytes), {len} bytes"
        );
    }
}

#[test]
fn blake3_keyed_hash_matches_the_official_vectors() {
    let doc = blake3_vectors();
    let key: [u8; 32] = doc["key"].as_str().unwrap().as_bytes().try_into().unwrap();
    for case in doc["cases"].as_array().unwrap() {
        let len = case["input_len"].as_u64().unwrap() as usize;
        let want = case["keyed_hash"].as_str().unwrap();
        let mut h = blake3::Hasher::new_keyed(&key);
        h.update(&blake3_input(len));
        assert_eq!(
            blake3_xof(h, want.len() / 2),
            want,
            "keyed_hash, {len} bytes"
        );
    }
}

#[test]
fn blake3_derive_key_matches_the_official_vectors() {
    let doc = blake3_vectors();
    let context = doc["context_string"].as_str().unwrap();
    for case in doc["cases"].as_array().unwrap() {
        let len = case["input_len"].as_u64().unwrap() as usize;
        let want = case["derive_key"].as_str().unwrap();
        let mut h = blake3::Hasher::new_derive_key(context);
        h.update(&blake3_input(len));
        assert_eq!(
            blake3_xof(h, want.len() / 2),
            want,
            "derive_key, {len} bytes"
        );
    }
}

// ── HKDF-SHA256 (RFC 5869) ──────────────────────────────────────────────────

fn hkdf_check(ikm: &str, salt: &str, info: &str, len: usize, prk: &str, okm: &str) {
    let salt = unhex(salt);
    let (got_prk, h) = hkdf::Hkdf::<sha2::Sha256>::extract(
        if salt.is_empty() { None } else { Some(&salt) },
        &unhex(ikm),
    );
    assert_eq!(tohex(&got_prk), prk.replace(char::is_whitespace, ""), "PRK");
    let mut out = vec![0u8; len];
    h.expand(&unhex(info), &mut out).unwrap();
    assert_eq!(tohex(&out), okm.replace(char::is_whitespace, ""), "OKM");
}

#[test]
fn hkdf_sha256_rfc5869_test_case_1() {
    hkdf_check(
        "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
        "000102030405060708090a0b0c",
        "f0f1f2f3f4f5f6f7f8f9",
        42,
        "077709362c2e32df0ddc3f0dc47bba63 90b6c73bb50f9c3122ec844ad7c2b3e5",
        "3cb25f25faacd57a90434f64d0362f2a 2d2d0a90cf1a5a4c5db02d56ecc4c5bf 34007208d5b887185865",
    );
}

#[test]
fn hkdf_sha256_rfc5869_test_case_3_empty_salt_and_info() {
    hkdf_check(
        "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
        "",
        "",
        42,
        "19ef24a32c717b167f33a91d6f648bdf 96596776afdb6377ac434c1c293ccb04",
        "8da4e775a563c18f715f802a063c5a31 b8a11f5c5ee1879ec3454e5f3c738d2d 9d201395faa4b61a96c8",
    );
}

// ── X25519 (RFC 7748) ───────────────────────────────────────────────────────

fn x25519_check(scalar: &str, u: &str, want: &str) {
    let k: [u8; 32] = unhex(scalar).try_into().unwrap();
    let u: [u8; 32] = unhex(u).try_into().unwrap();
    assert_eq!(tohex(&x25519_dalek::x25519(k, u)), want);
}

#[test]
fn x25519_rfc7748_section_5_2_first_vector() {
    x25519_check(
        "a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4",
        "e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c",
        "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552",
    );
}

#[test]
fn x25519_rfc7748_section_5_2_second_vector() {
    x25519_check(
        "4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d",
        "e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493",
        "95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957",
    );
}

#[test]
fn x25519_rfc7748_section_6_1_diffie_hellman() {
    let base = {
        let mut b = [0u8; 32];
        b[0] = 9;
        tohex(&b)
    };
    let alice_sk = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
    let alice_pk = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
    let bob_sk = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";
    let bob_pk = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
    let shared = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742";
    x25519_check(alice_sk, &base, alice_pk);
    x25519_check(bob_sk, &base, bob_pk);
    x25519_check(alice_sk, bob_pk, shared);
    x25519_check(bob_sk, alice_pk, shared);
}

// ── Ed25519 (RFC 8032) ──────────────────────────────────────────────────────

#[test]
fn ed25519_rfc8032_section_7_1_test_1() {
    use ed25519_dalek::{Signer, SigningKey};
    let sk: [u8; 32] = unhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
        .try_into()
        .unwrap();
    let key = SigningKey::from_bytes(&sk);
    assert_eq!(
        tohex(key.verifying_key().as_bytes()),
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
    );
    // The message is empty.
    assert_eq!(
        tohex(&key.sign(b"").to_bytes()),
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
         5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
    );
}

// ── ChaCha20-Poly1305 and XChaCha20-Poly1305 ────────────────────────────────

const AEAD_KEY: &str = "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f";
const AEAD_AAD: &str = "50515253c0c1c2c3c4c5c6c7";
const AEAD_PLAINTEXT: &[u8] = b"Ladies and Gentlemen of the class of '99: \
    If I could offer you only one tip for the future, sunscreen would be it.";

fn aead_check<C: chacha20poly1305::aead::Aead + chacha20poly1305::KeyInit>(
    nonce: &str,
    ciphertext: &str,
    tag: &str,
) {
    use chacha20poly1305::aead::{generic_array::GenericArray, Payload};
    let cipher = C::new(GenericArray::from_slice(&unhex(AEAD_KEY)));
    let nonce = unhex(nonce);
    let aad = unhex(AEAD_AAD);
    let want = unhex(&format!("{ciphertext}{tag}"));
    let got = cipher
        .encrypt(
            GenericArray::from_slice(&nonce),
            Payload {
                msg: AEAD_PLAINTEXT,
                aad: &aad,
            },
        )
        .unwrap();
    assert_eq!(tohex(&got), tohex(&want), "ciphertext || tag");
    let back = cipher
        .decrypt(
            GenericArray::from_slice(&nonce),
            Payload {
                msg: &want,
                aad: &aad,
            },
        )
        .unwrap();
    assert_eq!(back, AEAD_PLAINTEXT);
    // One flipped bit anywhere must be refused.
    let mut bad = want.clone();
    bad[0] ^= 1;
    assert!(cipher
        .decrypt(
            GenericArray::from_slice(&nonce),
            Payload {
                msg: &bad,
                aad: &aad
            }
        )
        .is_err());
}

#[test]
fn chacha20poly1305_rfc8439_section_2_8_2() {
    aead_check::<chacha20poly1305::ChaCha20Poly1305>(
        "070000004041424344454647",
        "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d6\
         3dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b36\
         92ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc\
         3ff4def08e4b7a9de576d26586cec64b6116",
        "1ae10b594f09e26a7e902ecbd0600691",
    );
}

#[test]
fn xchacha20poly1305_draft_arciszewski_xchacha_03_appendix_a_1() {
    aead_check::<chacha20poly1305::XChaCha20Poly1305>(
        "404142434445464748494a4b4c4d4e4f5051525354555657",
        "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb\
         731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b452\
         2f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff9\
         21f9664c97637da9768812f615c68b13b52e",
        "c0875924c1c7987947deafd8780acf49",
    );
}

// ── AES-256-GCM ─────────────────────────────────────────────────────────────

#[cfg(feature = "backup")]
mod aes {
    use super::*;
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Key, Nonce};

    #[test]
    fn aes256gcm_mcgrew_viega_test_case_16() {
        let key = unhex("feffe9928665731c6d6a8f9467308308 feffe9928665731c6d6a8f9467308308");
        let iv = unhex("cafebabefacedbaddecaf888");
        let plaintext = unhex(
            "d9313225f88406e5a55909c5aff5269a 86a7a9531534f7da2e4c303d8a318a72 \
             1c3c0c95956809532fcf0e2449a6b525 b16aedf5aa0de657ba637b39",
        );
        let aad = unhex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
        let ciphertext = unhex(
            "522dc1f099567d07f47f37a32a84427d 643a8cdcbfe5c0c97598a2bd2555d1aa \
             8cb08e48590dbb3da7b08b1056828838 c5f61e6393ba7a0abcc9f662",
        );
        let tag = unhex("76fc6ece0f4e1768cddf8853bb2d551b");

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let got = cipher
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .unwrap();
        assert_eq!(
            tohex(&got),
            tohex(&[ciphertext.clone(), tag.clone()].concat())
        );
        let mut bad = got.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(cipher
            .decrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &bad,
                    aad: &aad
                }
            )
            .is_err());
    }
}

// ── Argon2id (RFC 9106) ─────────────────────────────────────────────────────

#[cfg(feature = "backup")]
#[test]
fn argon2id_rfc9106_section_5_3() {
    use argon2::{Algorithm, Argon2, AssociatedData, ParamsBuilder, Version};
    // 32 KiB, 3 passes, 4 lanes, 32-byte tag; a secret and associated data.
    let params = ParamsBuilder::new()
        .m_cost(32)
        .t_cost(3)
        .p_cost(4)
        .data(AssociatedData::new(&[0x04; 12]).unwrap())
        .build()
        .unwrap();
    let ctx =
        Argon2::new_with_secret(&[0x03; 8], Algorithm::Argon2id, Version::V0x13, params).unwrap();
    let mut tag = [0u8; 32];
    ctx.hash_password_into(&[0x01; 32], &[0x02; 16], &mut tag)
        .unwrap();
    assert_eq!(
        tohex(&tag),
        "0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659"
    );
}

// ── evnx's envelope ─────────────────────────────────────────────────────────

/// Golden vectors for the formats evnx writes to a server and reads back
/// months later.
///
/// ⛔ **Changing a value in `envelope_golden.json`, or making one of these tests
/// pass by editing it, is a breaking protocol change.** Every vault key wrapped,
/// private key encrypted and blob stored by an existing client stops opening.
/// The file was generated once from `evnx-crypto` 0.2.1; if a test here fails,
/// the code changed, not the fixture.
#[cfg(all(feature = "cloud", feature = "backup"))]
mod envelope {
    use super::*;
    use evnx_crypto::{
        blob_hash, decrypt_private_key, decrypt_vault, derive_master_key, unwrap_vault_key,
        unwrap_vault_key_with_master_key, vault_aad, EncryptedBlob, EncryptedPrivateKey,
        WrappedVaultKey,
    };
    use sha2::{Digest, Sha256};

    fn golden() -> serde_json::Value {
        serde_json::from_str(&fixture("envelope_golden.json")).unwrap()
    }

    fn field(g: &serde_json::Value, name: &str) -> String {
        g[name]
            .as_str()
            .unwrap_or_else(|| panic!("golden field {name}"))
            .to_string()
    }

    fn master(g: &serde_json::Value) -> evnx_crypto::MasterKey {
        let salt: [u8; 32] = unhex(&field(g, "salt_hex")).try_into().unwrap();
        derive_master_key(field(g, "passphrase").as_bytes(), &salt).unwrap()
    }

    /// Argon2id with evnx's cost parameters and its domain-separation secret.
    #[test]
    fn envelope_master_key_derivation_is_pinned() {
        let g = golden();
        assert_eq!(tohex(master(&g).expose()), field(&g, "master_key_hex"));
    }

    /// XChaCha20-Poly1305 under an HKDF-SHA256 subkey, nonce first.
    #[test]
    fn envelope_vault_key_wrapped_with_the_master_key_still_opens() {
        let g = golden();
        let wrapped = unhex(&field(&g, "wrapped_vault_key_with_master_key_hex"));
        let key = unwrap_vault_key_with_master_key(&wrapped, &master(&g)).unwrap();
        assert_eq!(tohex(key.expose()), field(&g, "vault_key_hex"));

        let mut bad = wrapped;
        bad[30] ^= 1;
        assert!(unwrap_vault_key_with_master_key(&bad, &master(&g)).is_err());
    }

    /// The blob on the wire is `nonce(12) || AES-256-GCM(ciphertext || tag)`,
    /// bound to `(vault id, version)` by `vault_aad`.
    #[test]
    fn envelope_blob_format_is_pinned() {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        use aes_gcm::{Aes256Gcm, Key, Nonce};
        let g = golden();
        let vault_key = unwrap_vault_key_with_master_key(
            &unhex(&field(&g, "wrapped_vault_key_with_master_key_hex")),
            &master(&g),
        )
        .unwrap();

        let aad = vault_aad(
            &field(&g, "vault_id"),
            g["vault_version"].as_u64().unwrap() as u32,
        );
        assert_eq!(tohex(&aad), field(&g, "vault_aad_hex"), "vault_aad layout");

        let nonce: [u8; 12] = unhex(&field(&g, "blob_nonce_hex")).try_into().unwrap();
        let ciphertext = unhex(&field(&g, "blob_ciphertext_hex"));
        let plaintext = field(&g, "blob_plaintext");

        // Fixed key, nonce and plaintext give fixed bytes.
        let sealed = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(vault_key.expose()))
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: &aad,
                },
            )
            .unwrap();
        assert_eq!(tohex(&sealed), tohex(&ciphertext), "sealed bytes");

        // And evnx-crypto opens those bytes.
        let opened = decrypt_vault(
            &EncryptedBlob {
                nonce,
                ciphertext: ciphertext.clone(),
            },
            &vault_key,
            &aad,
        )
        .unwrap();
        assert_eq!(opened, plaintext.as_bytes());

        // The version and the vault id are part of what is authenticated.
        let other_version = vault_aad(&field(&g, "vault_id"), 4);
        assert!(decrypt_vault(
            &EncryptedBlob {
                nonce,
                ciphertext: ciphertext.clone()
            },
            &vault_key,
            &other_version
        )
        .is_err());
        let other_vault = vault_aad("vault-0002", 3);
        assert!(decrypt_vault(
            &EncryptedBlob {
                nonce,
                ciphertext: ciphertext.clone()
            },
            &vault_key,
            &other_vault
        )
        .is_err());

        // The transport hash is BLAKE3 of the ciphertext alone, lowercase hex.
        assert_eq!(blob_hash(&ciphertext), field(&g, "blob_hash"));
    }

    fn keypair(g: &serde_json::Value) -> evnx_crypto::UserKeypair {
        let enc = EncryptedPrivateKey {
            nonce: unhex(&field(g, "encrypted_private_key_nonce_hex"))
                .try_into()
                .unwrap(),
            ciphertext: unhex(&field(g, "encrypted_private_key_ciphertext_hex")),
        };
        decrypt_private_key(&enc, &master(g)).unwrap()
    }

    /// One stored seed derives all three public keys, the same way forever.
    #[test]
    fn envelope_private_key_still_opens_and_derives_the_same_keys() {
        let g = golden();
        let kp = keypair(&g);
        assert_eq!(
            kp.ed25519_public_base64(),
            field(&g, "ed25519_public_base64")
        );
        assert_eq!(kp.x25519_public_base64(), field(&g, "x25519_public_base64"));
        let mlkem = evnx_crypto::b64_decode(&kp.mlkem_public_base64(), "mlkem").unwrap();
        assert_eq!(
            tohex(&Sha256::digest(&mlkem)),
            field(&g, "mlkem768_public_sha256_hex")
        );

        let mut enc = EncryptedPrivateKey {
            nonce: unhex(&field(&g, "encrypted_private_key_nonce_hex"))
                .try_into()
                .unwrap(),
            ciphertext: unhex(&field(&g, "encrypted_private_key_ciphertext_hex")),
        };
        enc.ciphertext[0] ^= 1;
        assert!(decrypt_private_key(&enc, &master(&g)).is_err());
    }

    /// The hybrid wrap: X25519 and ML-KEM-768 shared secrets combined through
    /// HKDF-SHA256, then XChaCha20-Poly1305.
    #[test]
    fn envelope_hybrid_wrapped_vault_key_still_unwraps() {
        let g = golden();
        let kp = keypair(&g);
        let wrapped = WrappedVaultKey {
            eph_pub_key: unhex(&field(&g, "wrapped_eph_pub_key_hex"))
                .try_into()
                .unwrap(),
            mlkem_ciphertext: unhex(&field(&g, "wrapped_mlkem_ciphertext_hex")),
            encrypted_vault_key: unhex(&field(&g, "wrapped_encrypted_vault_key_hex")),
        };
        let key = unwrap_vault_key(&wrapped, &kp).unwrap();
        assert_eq!(tohex(key.expose()), field(&g, "vault_key_hex"));

        // Either half altered, the wrap must be refused.
        let mut bad = wrapped.clone();
        bad.encrypted_vault_key[30] ^= 1;
        assert!(unwrap_vault_key(&bad, &kp).is_err());
        let mut bad = wrapped.clone();
        bad.eph_pub_key[0] ^= 1;
        assert!(unwrap_vault_key(&bad, &kp).is_err());
        let mut bad = wrapped;
        bad.mlkem_ciphertext[0] ^= 1;
        assert!(
            unwrap_vault_key(&bad, &kp).is_err(),
            "ML-KEM implicit rejection must still fail the AEAD"
        );
    }
}
