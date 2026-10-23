//! Step-up, the device's half (Phase 2 §2.5): a fresh signature from the
//! presence-bound step-up key — Face ID or the passcode on an iPhone — before
//! a drive-class act. serve's `stepup.rs` is the other half.
//!
//! ```text
//! → {"cmd": "stepup_challenge"}            ← {"challenge": <32 bytes, base64>, "valid_secs": 120}
//!   the step-up key signs  challenge ‖ exporter
//!   (exporter: 32 bytes of THIS connection's TLS exporter under EXPORTER-worktrees-stepup-v1)
//! → {"cmd": "stepup_prove", "args": {"spki": <base64 DER>, "signature": <base64 DER>}}
//! ← {"valid_secs": 300}                    the step-up holds on this connection for five minutes
//! ```
//!
//! A [`StepUp`] is one connection's: [`crate::pipe::Pipe::stepup`] makes it
//! from the pipe's own TLS session and the pairing's step-up key, so a proof
//! can only be made for the session it will be sent on, and the exporter
//! never leaves this crate. The page — which runs the protocol — sends the
//! two frames; the shim holds the `StepUp` beside the connection and signs.
//!
//! **The key may block.** Signing with a `.userPresence` key shows Face ID
//! and waits for the person. [`StepUp::prove`] asks it on tokio's blocking
//! pool, off the async reactor; [`StepUp::prove_blocking`] asks it on the
//! caller's thread, for a caller that has its own (never a reactor's). A
//! dismissed prompt is [`SignError::Cancelled`]: nothing was signed and
//! nothing need be sent, the challenge simply lapses (§2.5: one challenge,
//! one try, 120 s), and the connection stays as it was.
//!
//! The signature is checked against the step-up key's own public half before
//! it is handed back (`sign::checked_sign`), so a signer that answers in the
//! wrong encoding never spends the challenge.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::sign::{checked_sign, spki_of, DeviceSigner, SignError};

/// What the step-up key signs: `challenge ‖ exporter` (§2.5, serve's
/// `StepUp::prove`).
pub fn message(challenge: &[u8; 32], exporter: &[u8; 32]) -> [u8; 64] {
    let mut m = [0u8; 64];
    m[..32].copy_from_slice(challenge);
    m[32..].copy_from_slice(exporter);
    m
}

/// `stepup_challenge`'s `challenge`: exactly 32 bytes, in base64 as serve
/// writes it. `None` for anything else.
pub fn parse_challenge(b64: &str) -> Option<[u8; 32]> {
    b64_decode(b64)?.try_into().ok()
}

/// A step-up proof: the step-up key's public half and its signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    pub spki: Vec<u8>,
    pub signature: Vec<u8>,
}

impl Proof {
    /// `stepup_prove`'s arguments.
    pub fn args(&self) -> Value {
        json!({ "spki": b64_encode(&self.spki), "signature": b64_encode(&self.signature) })
    }
}

/// One connection's step-up: the pairing's step-up key and the connection's
/// exporter under `EXPORTER-worktrees-stepup-v1`. Cheap to clone.
#[derive(Clone)]
pub struct StepUp {
    key: Arc<dyn DeviceSigner>,
    exporter: [u8; 32],
}

impl std::fmt::Debug for StepUp {
    /// The key's pin; never the exporter.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepUp").field("key", &crate::tls::pin_of_spki(&self.key.spki_der())).finish()
    }
}

impl StepUp {
    pub(crate) fn new(key: Arc<dyn DeviceSigner>, exporter: [u8; 32]) -> StepUp {
        StepUp { key, exporter }
    }

    /// Sign `challenge` for this connection, on the caller's thread: it may
    /// wait for Face ID, so never call it on an async reactor.
    pub fn prove_blocking(&self, challenge: &[u8; 32]) -> Result<Proof, SignError> {
        let spki = spki_of(&*self.key, "step-up")?;
        let signature = checked_sign(&*self.key, &spki, &message(challenge, &self.exporter))?;
        Ok(Proof { spki, signature })
    }

    /// [`Self::prove_blocking`] on tokio's blocking pool, so the reactor runs
    /// on while Face ID shows. Call it from the runtime that holds the pipe.
    pub async fn prove(&self, challenge: &[u8; 32]) -> Result<Proof, SignError> {
        let (me, c) = (self.clone(), *challenge);
        tokio::task::spawn_blocking(move || me.prove_blocking(&c))
            .await
            .unwrap_or_else(|e| Err(SignError::Failed(format!("the step-up signer stopped: {e}"))))
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64, padded: what serve reads (`stepup::b64_decode`, which
/// accepts only `worktrees_api::files::b64_encode`'s spelling).
pub(crate) fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard base64, padded, read strictly: only [`b64_encode`]'s spelling,
/// so each value has one.
pub(crate) fn b64_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(4) {
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
                B64.iter().position(|x| *x == c)? as u32
            };
            n = (n << 6) | v;
        }
        out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8][..3 - pad]);
    }
    (b64_encode(&out) == s).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign::testkit::{Bent, Soft};

    #[test]
    fn base64_round_trips_and_is_read_strictly() {
        for b in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(b64_decode(&b64_encode(b)).unwrap(), b);
        }
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        for bad in ["Zg", "Zg=", "Zh==", "Z===", "Zm9v\n", "Zm=v", "Zm9v====", "Zm 9"] {
            assert!(b64_decode(bad).is_none(), "{bad}");
        }
        assert!(parse_challenge(&b64_encode(&[7; 31])).is_none(), "31 bytes is not a challenge");
        assert_eq!(parse_challenge(&b64_encode(&[7; 32])), Some([7; 32]));
    }

    /// The proof is the step-up key's signature over `challenge ‖ exporter`,
    /// made off the reactor, and it verifies there with the key it names.
    #[test]
    fn a_proof_signs_the_challenge_and_this_connections_exporter() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let k = Arc::new(Soft::new());
        let s = StepUp::new(k.clone(), [9; 32]);
        let p = rt.block_on(s.prove(&[1; 32])).unwrap();
        assert_eq!(p.spki, k.spki_der());
        // §2.5's message, spelled out rather than taken from `message`.
        let signed = [[1u8; 32], [9u8; 32]].concat();
        assert!(crate::sign::verifies(&p.spki, &signed, &p.signature));
        assert_eq!(message(&[1; 32], &[9; 32]).to_vec(), signed);
        assert!(!crate::sign::verifies(&p.spki, &[[1u8; 32], [8u8; 32]].concat(), &p.signature), "bound to the exporter");
        assert_eq!(p.args()["spki"], json!(b64_encode(&k.spki_der())));
        assert!(!format!("{s:?}").contains(&crate::tls::hex(&[9; 32])), "Debug never shows the exporter");
    }

    /// A Face ID prompt dismissed is a cancel, said as one; a signer that
    /// blocks does so on the blocking pool while the reactor keeps running.
    #[test]
    fn a_cancel_is_a_cancel_and_a_slow_key_does_not_hold_the_reactor() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let cancel = StepUp::new(Arc::new(Bent { key: Soft::new(), how: |_: &Soft, _: &[u8]| Err(SignError::Cancelled) }), [0; 32]);
        assert_eq!(rt.block_on(cancel.prove(&[0; 32])), Err(SignError::Cancelled));
        let slow = StepUp::new(
            Arc::new(Bent {
                key: Soft::new(),
                how: |k: &Soft, m: &[u8]| {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    k.sign(m)
                },
            }),
            [0; 32],
        );
        // One reactor thread: a tick that lands while the key "waits for Face
        // ID" means the wait was not on it.
        let ticked = rt.block_on(async {
            let proving = tokio::spawn(async move { slow.prove(&[0; 32]).await });
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let ticked = !proving.is_finished();
            assert!(proving.await.unwrap().is_ok());
            ticked
        });
        assert!(ticked, "the reactor ran while the key signed");
    }
}
