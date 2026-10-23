//! A device's keys as SIGNERS (Phase 2 §2.1): what this crate asks of a key,
//! and all it asks — its public half, and a signature.
//!
//! On a desktop the key is a `0600` file ([`crate::key::DeviceKey`], one
//! signer among others). On an iPhone it is the Secure Enclave's, where the
//! private key never leaves the chip and the only thing to do with it is ask
//! it to sign (`SecKeyCreateSignature`). Every place this crate needs a
//! private key — the TLS client certificate and its handshake signature
//! (`tls`), pairing (`pair`), the pipe (`pipe`, `call`) and a step-up proof
//! (`stepup`) — goes through [`DeviceSigner`], which has no accessor for key
//! bytes: none of them could read a key if they wanted to.
//!
//! The one scheme is ECDSA P-256 with SHA-256 (`tls::SCHEME`), the only one
//! the Secure Enclave signs with and the only one serve accepts.
//!
//! **Every signature is checked before it is used** ([`checked_sign`]):
//! against the key's own public half, with the check serve itself makes
//! (`ring`'s ECDSA P-256 / SHA-256, ASN.1). A signer that answers with the
//! wrong encoding (a raw `r ‖ s`), over the wrong input (a digest where the
//! message belongs), or with another key, is refused HERE, as
//! [`SignError::Invalid`] — before a byte of it reaches a host, so it cannot
//! spend a pairing window's one attempt or a step-up challenge.

use std::sync::Arc;

/// The DER prefix of every P-256 SubjectPublicKeyInfo with an uncompressed
/// point: SEQUENCE { SEQUENCE { id-ecPublicKey, prime256v1 }, BIT STRING 0x00 },
/// then 65 bytes, `04 ‖ X ‖ Y`. serve's step-up check reads keys with it too.
pub const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
    0x03, 0x42, 0x00,
];

/// The uncompressed point of a P-256 SubjectPublicKeyInfo, or `None` for any
/// other key or any other encoding of one.
pub fn p256_point(spki: &[u8]) -> Option<&[u8]> {
    let point = spki.strip_prefix(&P256_SPKI_PREFIX[..])?;
    (point.len() == 65 && point[0] == 0x04).then_some(point)
}

/// The SubjectPublicKeyInfo of an uncompressed P-256 point (`04 ‖ X ‖ Y`, 65
/// bytes) — what `SecKeyCopyExternalRepresentation` gives for a Secure Enclave
/// key's public half — or `None` for anything else.
pub fn p256_spki(point: &[u8]) -> Option<Vec<u8>> {
    if point.len() != 65 || point[0] != 0x04 {
        return None;
    }
    let mut spki = P256_SPKI_PREFIX.to_vec();
    spki.extend_from_slice(point);
    Some(spki)
}

/// Why a key gave no signature this crate could use.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignError {
    /// The person dismissed the prompt the key raised (Face ID, the passcode):
    /// nothing was signed. A signer returns this, and only this, for a cancel
    /// (`errSecUserCanceled`, `LAError.userCancel`, `.appCancel`,
    /// `.systemCancel`), so the page can say so instead of reporting a failure.
    Cancelled,
    /// The key could not sign, in the signer's own words: the device is
    /// locked, the key is gone, the hardware refused.
    Failed(String),
    /// This crate's verdict, never a signer's: the signature does not verify
    /// against the key's own public half — not DER, not over the message, or
    /// made by another key. Nothing was sent.
    Invalid,
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignError::Cancelled => write!(f, "cancelled on this device; nothing was signed"),
            SignError::Failed(why) => write!(f, "the key could not sign: {}", crate::printable(why)),
            SignError::Invalid => write!(
                f,
                "the key's signature does not verify against its own public key (it must be ECDSA P-256 over the \
                 message with SHA-256, ASN.1 DER: ecdsaSignatureMessageX962SHA256); nothing was sent"
            ),
        }
    }
}

impl std::error::Error for SignError {}

/// A P-256 key this crate may ask to sign, and never read.
///
/// Implemented by [`crate::key::DeviceKey`] (a desktop's `0600` file) and, in
/// the iOS app, by a type that calls its Swift plugin's
/// `SecKeyCreateSignature`. `IOS.md` beside this crate is the contract a
/// Secure Enclave signer meets.
pub trait DeviceSigner: Send + Sync {
    /// The public half: a DER SubjectPublicKeyInfo of a P-256 key with an
    /// uncompressed point — 91 bytes, [`P256_SPKI_PREFIX`] ‖ `04 ‖ X ‖ Y`
    /// ([`p256_spki`] builds it from the point). The same bytes on every call.
    fn spki_der(&self) -> Vec<u8>;

    /// ECDSA P-256 over the SHA-256 of `message` — the signer hashes; it is
    /// never handed a digest — as an ASN.1 DER `Ecdsa-Sig-Value` (X9.62): what
    /// `SecKeyCreateSignature(key, .ecdsaSignatureMessageX962SHA256, message)`
    /// returns.
    ///
    /// A device key is asked on the TLS handshake's own task, so it must
    /// answer without waiting on a person. A step-up key may block while Face
    /// ID shows: this crate asks it off the async reactor ([`crate::stepup`]).
    /// A dismissed prompt is [`SignError::Cancelled`].
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignError>;
}

/// One pairing's keys (§2.1): the device key every connection presents, and,
/// on an iPhone or iPad only, the presence-bound step-up key a drive-class
/// act needs (§2.5). Cheap to clone; the keys are shared.
#[derive(Clone)]
pub struct Keys {
    pub device: Arc<dyn DeviceSigner>,
    pub stepup: Option<Arc<dyn DeviceSigner>>,
}

impl Keys {
    /// A desktop's keys: a device key, no step-up key.
    pub fn device_only(device: Arc<dyn DeviceSigner>) -> Keys {
        Keys { device, stepup: None }
    }
}

impl std::fmt::Debug for Keys {
    /// The pins, never more.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pin = |k: &dyn DeviceSigner| crate::tls::pin_of_spki(&k.spki_der());
        f.debug_struct("Keys").field("device", &pin(&*self.device)).field("stepup", &self.stepup.as_deref().map(pin)).finish()
    }
}

/// `signature` verifies over `message` with the P-256 key `spki`: serve's own
/// check (`stepup.rs`, and rustls's for a handshake).
pub fn verifies(spki: &[u8], message: &[u8], signature: &[u8]) -> bool {
    p256_point(spki).is_some_and(|point| {
        ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point).verify(message, signature).is_ok()
    })
}

/// Ask `key` to sign `message`, and refuse what it answers unless it
/// verifies against `spki`, the key's own public half (read once by the
/// caller, so one dial or one proof uses one view of the key).
pub fn checked_sign(key: &dyn DeviceSigner, spki: &[u8], message: &[u8]) -> Result<Vec<u8>, SignError> {
    let sig = key.sign(message)?;
    if verifies(spki, message, &sig) {
        Ok(sig)
    } else {
        Err(SignError::Invalid)
    }
}

/// A key's public half, refused unless it is a P-256 SPKI.
pub(crate) fn spki_of(key: &dyn DeviceSigner, what: &str) -> Result<Vec<u8>, SignError> {
    let spki = key.spki_der();
    if p256_point(&spki).is_none() {
        return Err(SignError::Failed(format!("the {what} key's public half is not a P-256 SubjectPublicKeyInfo")));
    }
    Ok(spki)
}

#[cfg(test)]
pub(crate) mod testkit {
    //! A software P-256 key behind [`DeviceSigner`] alone, and the ways a
    //! signer can misbehave.
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

    pub struct Soft {
        pair: EcdsaKeyPair,
    }

    impl Soft {
        pub fn new() -> Soft {
            let rng = SystemRandom::new();
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            Soft { pair: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap() }
        }
    }

    impl DeviceSigner for Soft {
        fn spki_der(&self) -> Vec<u8> {
            p256_spki(self.pair.public_key().as_ref()).unwrap()
        }
        fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignError> {
            Ok(self.pair.sign(&SystemRandom::new(), message).unwrap().as_ref().to_vec())
        }
    }

    /// A signer that answers with whatever `how` makes of the honest
    /// signature.
    pub struct Bent<F: Fn(&Soft, &[u8]) -> Result<Vec<u8>, SignError> + Send + Sync> {
        pub key: Soft,
        pub how: F,
    }

    impl<F: Fn(&Soft, &[u8]) -> Result<Vec<u8>, SignError> + Send + Sync> DeviceSigner for Bent<F> {
        fn spki_der(&self) -> Vec<u8> {
            self.key.spki_der()
        }
        fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignError> {
            (self.how)(&self.key, message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn a_point_and_its_spki_are_one_key() {
        let k = Soft::new();
        let spki = k.spki_der();
        assert_eq!(spki.len(), 91);
        let point = p256_point(&spki).unwrap();
        assert_eq!(p256_spki(point).unwrap(), spki);
        assert!(p256_spki(&point[1..]).is_none(), "64 bytes is not a point");
        let mut compressed = point.to_vec();
        compressed[0] = 0x02;
        assert!(p256_spki(&compressed).is_none());
        let mut p384 = spki.clone();
        p384[22] ^= 1;
        assert!(p256_point(&p384).is_none());
    }

    /// The self-check is serve's check: an honest signature passes, and each
    /// thing a Secure Enclave plugin can get wrong is `Invalid` — the raw
    /// `r ‖ s` CryptoKit's `rawRepresentation` gives, a signature over the
    /// digest (`…DigestX962SHA256` handed the message's hash), another key's.
    #[test]
    fn only_a_der_signature_over_the_message_by_this_key_passes() {
        let k = Soft::new();
        let spki = k.spki_der();
        let msg = b"challenge and exporter";
        assert!(checked_sign(&k, &spki, msg).is_ok());
        let raw = Bent {
            key: Soft::new(),
            how: |k: &Soft, m: &[u8]| {
                // DER `30 len 02 lr r 02 ls s` → r ‖ s, each left-padded to 32.
                let der = k.sign(m)?;
                let (rl, r) = (der[3] as usize, &der[4..]);
                let (r, rest) = r.split_at(rl);
                let s = &rest[2..];
                let fix = |x: &[u8]| {
                    let x = x.strip_prefix(&[0u8][..]).unwrap_or(x);
                    let mut v = vec![0u8; 32 - x.len()];
                    v.extend_from_slice(x);
                    v
                };
                Ok([fix(r), fix(s)].concat())
            },
        };
        assert_eq!(checked_sign(&raw, &raw.spki_der(), msg), Err(SignError::Invalid), "raw r ‖ s");
        let digest = Bent { key: Soft::new(), how: |k: &Soft, m: &[u8]| k.sign(&Sha256::digest(m)) };
        assert_eq!(checked_sign(&digest, &digest.spki_der(), msg), Err(SignError::Invalid), "over the digest");
        let other = Soft::new();
        assert_eq!(checked_sign(&other, &spki, msg), Err(SignError::Invalid), "another key");
        let cancel = Bent { key: Soft::new(), how: |_: &Soft, _: &[u8]| Err(SignError::Cancelled) };
        assert_eq!(checked_sign(&cancel, &cancel.spki_der(), msg), Err(SignError::Cancelled), "a cancel is passed on as one");
    }

    #[test]
    fn keys_debug_names_pins_only() {
        let k = Arc::new(Soft::new());
        let pin = crate::tls::pin_of_spki(&k.spki_der());
        let keys = Keys::device_only(k);
        assert_eq!(format!("{keys:?}"), format!("Keys {{ device: {pin:?}, stepup: None }}"));
    }
}
