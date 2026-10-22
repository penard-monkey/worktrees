//! Step-up for drive-class acts (Phase 2 §2.5, §11 Q8): the SERVER's half.
//!
//! An iPhone's connection key has no presence requirement, so status and
//! reads stay seamless; its riskiest acts also need a fresh signature from a
//! second, presence-bound key (Face ID or the passcode), enrolled at pairing
//! as `stepup_spki_sha256`. On by default for every device that holds such a
//! key; off is a console act (`device_set_stepup`).
//!
//! The exchange, on one connection: `stepup_challenge` → 32 random bytes, good
//! for [`CHALLENGE_FOR`] and for one proof; `stepup_prove {spki, signature}`
//! → the device's step-up public key (whose SHA-256 must be the record's) and
//! an ECDSA P-256 / SHA-256 signature, ASN.1 DER — what the Secure Enclave's
//! `SecKeyCreateSignature` makes — over `challenge ‖ exporter`, where
//! `exporter` is 32 bytes of the connection's own TLS 1.3 exporter under
//! `EXPORTER-worktrees-stepup-v1`. Bound to the session, so a proof cannot be
//! carried to another connection. It then holds on THIS connection for
//! [`VALID_FOR`].
//!
//! Which acts (`dispatch::needs_stepup`): any row needing `instruct`,
//! `answer` or `admin`, whether the row's own capability or one its objects
//! raised (a path class, revoking another device), and OPENING a stream —
//! `term_open`, `shell_open`. The keystrokes, resizes and closes of a stream
//! already open are not re-asked, and a proof that lapses ends nothing that
//! is open: step-up gates the act, not the session.
//!
//! The device's key and its signing call are the hardware spike's (§10 A4);
//! nothing here assumes how the device makes the signature.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::devices::pin_of;

/// How long a proof holds on its connection (§2.5).
pub const VALID_FOR: Duration = Duration::from_secs(300);

/// How long a challenge may wait for its proof: room for a Face ID prompt,
/// and no more.
pub const CHALLENGE_FOR: Duration = Duration::from_secs(120);

/// `err` for a drive-class act from a device whose step-up is due.
pub const ERR_STEPUP: &str = "step-up required";

/// The DER prefix of every P-256 SubjectPublicKeyInfo with an uncompressed
/// point: SEQUENCE { SEQUENCE { id-ecPublicKey, prime256v1 }, BIT STRING 0x00 },
/// then 65 bytes, `04 ‖ X ‖ Y`.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
    0x03, 0x42, 0x00,
];

/// The uncompressed point of a P-256 SubjectPublicKeyInfo, or `None` for any
/// other key or any other encoding of one.
pub fn p256_point(spki: &[u8]) -> Option<&[u8]> {
    let point = spki.strip_prefix(&P256_SPKI_PREFIX[..])?;
    (point.len() == 65 && point[0] == 0x04).then_some(point)
}

#[derive(Default)]
struct Inner {
    challenge: Option<([u8; 32], Instant)>,
    valid_until: Option<Instant>,
}

/// One connection's step-up state.
pub struct StepUp {
    /// The connection's exporter under `EXPORTER-worktrees-stepup-v1`, or
    /// `None` on a connection that is not TLS (loopback): there is nothing to
    /// bind a proof to, and nothing there needs one.
    exporter: Option<[u8; 32]>,
    inner: Mutex<Inner>,
}

impl StepUp {
    pub fn new(exporter: Option<[u8; 32]>) -> StepUp {
        StepUp { exporter, inner: Mutex::new(Inner::default()) }
    }

    /// A fresh challenge, replacing any outstanding one.
    pub fn challenge(&self, now: Instant) -> Result<[u8; 32], String> {
        if self.exporter.is_none() {
            return Err("this connection has no TLS session to bind a step-up to".into());
        }
        let mut c = [0u8; 32];
        getrandom::fill(&mut c).expect("the OS refused entropy");
        self.inner.lock().unwrap().challenge = Some((c, now));
        Ok(c)
    }

    /// Check a proof against the outstanding challenge, which it uses up
    /// whatever the outcome: one challenge, one try. `stepup_pin` is the
    /// device record's. On success the step-up holds until the returned time.
    pub fn prove(&self, stepup_pin: Option<&str>, spki: &[u8], signature: &[u8], now: Instant) -> Result<Instant, String> {
        let exporter = self.exporter.ok_or("this connection has no TLS session to bind a step-up to")?;
        let mut inner = self.inner.lock().unwrap();
        let (challenge, at) = inner.challenge.take().ok_or("no step-up challenge is outstanding")?;
        if now.duration_since(at) > CHALLENGE_FOR {
            return Err("the step-up challenge expired; ask for another".into());
        }
        let pin = stepup_pin.ok_or("this device enrolled no step-up key")?;
        if pin_of(spki) != pin {
            return Err("that is not this device's step-up key".into());
        }
        let point = p256_point(spki).ok_or("the step-up key is not a P-256 key")?;
        let mut msg = Vec::with_capacity(64);
        msg.extend_from_slice(&challenge);
        msg.extend_from_slice(&exporter);
        ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point)
            .verify(&msg, signature)
            .map_err(|_| "the step-up signature does not verify".to_string())?;
        let until = now + VALID_FOR;
        inner.valid_until = Some(until);
        Ok(until)
    }

    /// A proof holds on this connection at `now`.
    pub fn fresh(&self, now: Instant) -> bool {
        self.inner.lock().unwrap().valid_until.is_some_and(|u| now < u)
    }
}

/// Standard base64 with padding, read strictly: only what
/// `worktrees_api::files::b64_encode` writes, so each value has one spelling.
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    if s.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in s.as_bytes().chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for (i, &c) in chunk.iter().enumerate() {
            let v = if c == b'=' && i >= 2 {
                pad += 1;
                0
            } else if pad > 0 {
                return None;
            } else {
                T.iter().position(|x| *x == c)? as u32
            };
            n = (n << 6) | v;
        }
        let b = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&b[..3 - pad]);
    }
    (worktrees_api::files::b64_encode(&out) == s).then_some(out)
}

#[cfg(test)]
pub(crate) mod testkit {
    //! A device's step-up key, for the tests: P-256 in software, standing in
    //! for the Secure Enclave key the spike measures.
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

    pub struct StepUpKey {
        pair: EcdsaKeyPair,
        pub spki: Vec<u8>,
        pub pin: String,
    }

    impl StepUpKey {
        pub fn new() -> StepUpKey {
            let rng = SystemRandom::new();
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
            let mut spki = super::P256_SPKI_PREFIX.to_vec();
            spki.extend_from_slice(pair.public_key().as_ref());
            let pin = super::pin_of(&spki);
            StepUpKey { pair, spki, pin }
        }

        pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
            self.pair.sign(&SystemRandom::new(), msg).unwrap().as_ref().to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::StepUpKey;
    use super::*;

    const EXPORTER: [u8; 32] = [9; 32];

    fn signed(k: &StepUpKey, challenge: &[u8; 32], exporter: &[u8; 32]) -> Vec<u8> {
        let mut m = challenge.to_vec();
        m.extend_from_slice(exporter);
        k.sign(&m)
    }

    #[test]
    fn a_signature_over_the_challenge_and_this_sessions_exporter_holds_for_five_minutes() {
        let k = StepUpKey::new();
        let s = StepUp::new(Some(EXPORTER));
        let t0 = Instant::now();
        assert!(!s.fresh(t0));
        let c = s.challenge(t0).unwrap();
        let until = s.prove(Some(&k.pin), &k.spki, &signed(&k, &c, &EXPORTER), t0).unwrap();
        assert_eq!(until, t0 + VALID_FOR);
        assert!(s.fresh(t0 + VALID_FOR - Duration::from_secs(1)));
        assert!(!s.fresh(t0 + VALID_FOR), "five minutes, then it is due again");
    }

    /// Each way a proof can be wrong is refused, and every try uses up the
    /// challenge.
    #[test]
    fn a_proof_is_refused_for_the_wrong_key_session_challenge_or_age() {
        let k = StepUpKey::new();
        let other = StepUpKey::new();
        let s = StepUp::new(Some(EXPORTER));
        let t0 = Instant::now();
        assert_eq!(s.prove(Some(&k.pin), &k.spki, b"x", t0).unwrap_err(), "no step-up challenge is outstanding");
        let c = s.challenge(t0).unwrap();
        assert!(s.prove(Some(&k.pin), &k.spki, &signed(&k, &c, &[0; 32]), t0).unwrap_err().contains("does not verify"), "another session's exporter");
        assert!(s.prove(Some(&k.pin), &k.spki, &signed(&k, &c, &EXPORTER), t0).unwrap_err().contains("no step-up challenge"), "one challenge, one try");
        let c = s.challenge(t0).unwrap();
        assert!(s.prove(Some(&k.pin), &other.spki, &signed(&other, &c, &EXPORTER), t0).unwrap_err().contains("not this device's"), "another key");
        let c = s.challenge(t0).unwrap();
        assert!(s.prove(Some(&k.pin), &k.spki, &signed(&other, &c, &EXPORTER), t0).unwrap_err().contains("does not verify"), "signed by another key");
        let c = s.challenge(t0).unwrap();
        let stale = signed(&k, &[0; 32], &EXPORTER);
        assert!(s.prove(Some(&k.pin), &k.spki, &stale, t0).unwrap_err().contains("does not verify"), "an old challenge");
        let c2 = s.challenge(t0).unwrap();
        assert_ne!(c, c2);
        assert!(s.prove(Some(&k.pin), &k.spki, &signed(&k, &c2, &EXPORTER), t0 + CHALLENGE_FOR + Duration::from_secs(1)).unwrap_err().contains("expired"));
        assert!(s.prove(None, &k.spki, b"x", t0).is_err(), "a device with no step-up key");
        assert!(!s.fresh(t0));
        let plain = StepUp::new(None);
        assert!(plain.challenge(t0).is_err(), "no TLS session, nothing to bind to");
    }

    #[test]
    fn only_a_p256_key_has_a_point() {
        let k = StepUpKey::new();
        assert_eq!(p256_point(&k.spki).unwrap().len(), 65);
        let mut wrong_curve = k.spki.clone();
        wrong_curve[22] ^= 1;
        assert!(p256_point(&wrong_curve).is_none());
        assert!(p256_point(&k.spki[..90]).is_none());
        let mut compressed = k.spki.clone();
        compressed[26] = 0x02;
        assert!(p256_point(&compressed).is_none());
    }

    #[test]
    fn base64_is_read_strictly() {
        for b in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(b64_decode(&worktrees_api::files::b64_encode(b)).unwrap(), b);
        }
        for bad in ["Zg", "Zg=", "Zh==", "Z===", "Zm9v\n", "Zm=v", "Zm9v====", "Zm 9"] {
            assert!(b64_decode(bad).is_none(), "{bad}");
        }
    }
}
