//! `evnx-crypto` on arbitrary bytes: every way it is handed ciphertext, a
//! wrapped key or an encoded key that a server, a file or an attacker chose.
//!
//! Each must return an error and never panic. A forged ciphertext that
//! decrypts is a worse failure still, so decryption results are asserted to be
//! errors: without the key, arbitrary bytes cannot authenticate.
#![no_main]

use evnx_crypto::{
    b64_decode, decrypt_private_key, decrypt_vault, derive_master_key, generate_keypair,
    mlkem_encapsulate, salt_from_base64, unwrap_vault_key, unwrap_vault_key_with_master_key,
    EncryptedBlob, EncryptedPrivateKey, MasterKey, MlKem768PublicKey, UserKeypair,
    UserPublicKeys, VaultKey, WrappedVaultKey, MLKEM768_CIPHERTEXT_LEN, MLKEM768_PUBLIC_LEN,
};
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

struct Keys {
    master: MasterKey,
    vault: VaultKey,
    pair: UserKeypair,
}

/// Built once: Argon2id at the production cost would otherwise dominate every
/// iteration.
fn keys() -> &'static Keys {
    static KEYS: OnceLock<Keys> = OnceLock::new();
    KEYS.get_or_init(|| Keys {
        master: derive_master_key(b"fuzz", &[7u8; 32]).expect("derive"),
        vault: VaultKey::generate(),
        pair: generate_keypair(),
    })
}

/// The next `n` bytes of `data`, zero-padded when it runs short.
fn take<const N: usize>(data: &mut &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    let n = N.min(data.len());
    out[..n].copy_from_slice(&data[..n]);
    *data = &data[n..];
    out
}

fuzz_target!(|input: &[u8]| {
    let k = keys();
    let mut data = input;

    // 1. A stored blob: nonce || ciphertext, with any associated data.
    let nonce = take::<12>(&mut data);
    let aad_len = usize::from(take::<1>(&mut data)[0]).min(data.len());
    let (aad, rest) = data.split_at(aad_len);
    let blob = EncryptedBlob {
        nonce,
        ciphertext: rest.to_vec(),
    };
    assert!(decrypt_vault(&blob, &k.vault, aad).is_err());

    // 2. A vault key wrapped under the master key: nonce || ciphertext.
    assert!(unwrap_vault_key_with_master_key(input, &k.master).is_err());

    // 3. An encrypted private key.
    let mut d = input;
    let enc = EncryptedPrivateKey {
        nonce: take::<24>(&mut d),
        ciphertext: d.to_vec(),
    };
    assert!(decrypt_private_key(&enc, &k.master).is_err());

    // 4. A hybrid-wrapped vault key. The ML-KEM ciphertext has a fixed length
    //    on the wire, so it is padded or cut to that, and the other two parts
    //    are whatever is left.
    let mut d = input;
    let eph = take::<32>(&mut d);
    let mlkem = take::<MLKEM768_CIPHERTEXT_LEN>(&mut d);
    let wrapped = WrappedVaultKey {
        eph_pub_key: eph,
        mlkem_ciphertext: mlkem.to_vec(),
        encrypted_vault_key: d.to_vec(),
    };
    assert!(unwrap_vault_key(&wrapped, &k.pair).is_err());
    // And with a ciphertext of the wrong length, which must be refused, not
    // sliced.
    let short = WrappedVaultKey {
        mlkem_ciphertext: input.to_vec(),
        ..wrapped
    };
    let _ = unwrap_vault_key(&short, &k.pair);

    // 5. Encoded forms a server sends. Text is whatever the bytes make of it.
    let text = String::from_utf8_lossy(input);
    let _ = b64_decode(&text, "fuzz");
    let _ = salt_from_base64(&text);
    let _ = EncryptedPrivateKey::from_base64(&text);
    let _ = WrappedVaultKey::from_base64(&text, &text, &text);
    let mid = (0..=text.len() / 2)
        .rev()
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    let (a, b) = text.split_at(mid);
    let _ = UserPublicKeys::from_base64(a, b);

    // 6. A recipient's ML-KEM public key of any content, encapsulated to.
    let _ = MlKem768PublicKey::from_bytes(input);
    let mut d = input;
    let pk = take::<MLKEM768_PUBLIC_LEN>(&mut d);
    if let Ok(pk) = MlKem768PublicKey::from_bytes(&pk) {
        let _ = mlkem_encapsulate(&pk);
    }
});
