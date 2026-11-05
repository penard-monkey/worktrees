//! The payload's seal and the collapse id (the phone's
//! `docs/design/notifications.md`, "The encrypted payload"; the wire is in
//! `docs/push-contract.md`).
//!
//! ChaCha20-Poly1305 through `ring`, which serve already holds for step-up and
//! the JWT: the design named RustCrypto's crate, and the bytes are the same,
//! RFC 8439's, with the tag after the ciphertext. CryptoKit's `ChaChaPoly`
//! opens them (`docs/push-fixtures/seal-vector.json` is checked on both sides).

use hmac::{Hmac, KeyInit, Mac};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use serde_json::{json, Value};
use sha2::Sha256;
use worktrees_api::files::b64_encode;

/// The associated data's label: the ASCII, then the key id's 8 raw bytes.
pub const LABEL: &[u8] = b"worktrees-push-v1";

pub const KEY_LEN: usize = 32;
pub const KEY_ID_LEN: usize = 8;
pub const NONCE_LEN: usize = 12;

/// `plain`, sealed: the ciphertext with its 16-byte tag after it.
pub fn seal(key: &[u8; KEY_LEN], key_id: &[u8; KEY_ID_LEN], nonce: [u8; NONCE_LEN], plain: &[u8]) -> Vec<u8> {
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key).expect("32 bytes is this cipher's key");
    let sealing = LessSafeKey::new(unbound);
    let mut aad = Vec::with_capacity(LABEL.len() + KEY_ID_LEN);
    aad.extend_from_slice(LABEL);
    aad.extend_from_slice(key_id);
    let mut buf = plain.to_vec();
    sealing.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad), &mut buf).expect("a notification is far under the cipher's limit");
    buf
}

/// The push's `wt`, with `nonce` as given: every field standard padded base64.
pub fn wt(key: &[u8; KEY_LEN], key_id: &[u8; KEY_ID_LEN], nonce: [u8; NONCE_LEN], plain: &[u8]) -> Value {
    json!({ "v": 1, "k": b64_encode(key_id), "n": b64_encode(&nonce), "c": b64_encode(&seal(key, key_id, nonce, plain)) })
}

/// A random nonce per push.
pub fn random_nonce() -> [u8; NONCE_LEN] {
    let mut n = [0u8; NONCE_LEN];
    getrandom::fill(&mut n).expect("the OS refused entropy");
    n
}

/// `apns-collapse-id`: HMAC-SHA256(key, repo NUL slug), first 16 bytes in hex.
/// Opaque to Apple and stable per place and key, so a reminder replaces its
/// first notification. Only the host computes it. The NUL keeps `/a` + `bc`
/// from being `/ab` + `c`. 32 characters: APNs allows 64 bytes.
pub fn collapse_id(key: &[u8; KEY_LEN], repo: &str, slug: &str) -> String {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(repo.as_bytes());
    m.update(&[0]);
    m.update(slug.as_bytes());
    let out = m.finalize().into_bytes();
    super::super::devices::hex(&out[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::devices::hex;
    use crate::serve::stepup::b64_decode;
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// RFC 8439 §2.8.2, through the same cipher call `seal` makes, with the AAD
    /// given raw: the check that this is the cipher CryptoKit runs.
    #[test]
    fn the_cipher_is_rfc_8439s() {
        let key = unhex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
        let nonce: [u8; 12] = unhex("070000004041424344454647").try_into().unwrap();
        let aad = unhex("50515253c0c1c2c3c4c5c6c7");
        let plain = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let k = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &key).unwrap());
        let mut buf = plain.to_vec();
        k.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad), &mut buf).unwrap();
        assert_eq!(
            hex(&buf),
            "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd0600691"
        );
    }

    /// The vector the phone runs (`plugins/platform/tests/push-crypto.sh`,
    /// BACKLOG B-14): what `seal` makes of the fixture's inputs is its bytes,
    /// byte for byte, and the file's own lengths and label are what they say.
    #[test]
    fn the_seal_vector_in_the_contract_is_what_seal_makes() {
        let v: Value = serde_json::from_str(include_str!("../../../../../docs/push-fixtures/seal-vector.json")).unwrap();
        let key: [u8; 32] = unhex(v["key_hex"].as_str().unwrap()).try_into().unwrap();
        let key_id: [u8; 8] = unhex(v["key_id_hex"].as_str().unwrap()).try_into().unwrap();
        let nonce: [u8; 12] = unhex(v["nonce_hex"].as_str().unwrap()).try_into().unwrap();
        let plain = v["plaintext"].as_str().unwrap().as_bytes();
        assert_eq!(unhex(v["aad_hex"].as_str().unwrap()), [LABEL, key_id.as_slice()].concat(), "the label, then the key id's raw bytes");
        let sealed = seal(&key, &key_id, nonce, plain);
        assert_eq!(hex(&sealed), v["ciphertext_and_tag_hex"].as_str().unwrap());
        assert_eq!(sealed.len(), plain.len() + 16, "the 16-byte tag follows the ciphertext");
        let w = wt(&key, &key_id, nonce, plain);
        assert_eq!(w, v["wt"], "the push's own `wt`, standard base64");
        assert_eq!(b64_decode(w["c"].as_str().unwrap()).unwrap(), sealed);
        assert_eq!(collapse_id(&key, v["collapse"]["repo"].as_str().unwrap(), v["collapse"]["slug"].as_str().unwrap()), v["collapse"]["id"].as_str().unwrap());
    }

    /// Bytes the key id does not name do not open, so a payload cannot be
    /// moved from one key id to another.
    #[test]
    fn the_key_id_is_bound_into_the_seal() {
        let key = [7u8; 32];
        assert_ne!(seal(&key, &[1; 8], [0; 12], b"x"), seal(&key, &[2; 8], [0; 12], b"x"));
    }

    #[test]
    fn a_collapse_id_is_per_place_and_per_key_and_keeps_parts_apart() {
        let k = [1u8; 32];
        assert_eq!(collapse_id(&k, "/r", "a").len(), 32);
        assert_eq!(collapse_id(&k, "/r", "a"), collapse_id(&k, "/r", "a"));
        assert_ne!(collapse_id(&k, "/r", "a"), collapse_id(&k, "/r", "b"));
        assert_ne!(collapse_id(&k, "/a", "bc"), collapse_id(&k, "/ab", "c"));
        assert_ne!(collapse_id(&k, "/r", "a"), collapse_id(&[2u8; 32], "/r", "a"));
    }

    #[test]
    fn nonces_differ() {
        assert_ne!(random_nonce(), random_nonce());
    }
}
