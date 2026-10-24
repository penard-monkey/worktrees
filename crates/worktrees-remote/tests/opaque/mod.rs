//! The Secure Enclave's stand-in for the integration tests: a software P-256
//! key that exposes [`DeviceSigner`] and nothing else — no bytes, no `Debug` —
//! and counts what it is asked, so a test can see a key was ASKED to sign
//! rather than read. Shared by `pipe.rs` and `reach.rs`.
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use worktrees_remote::sign::{self, DeviceSigner, SignError};
use worktrees_remote::tls;

/// What an [`Opaque`] key answers its `n`th request with (from 0), given the
/// honest signature.
pub type How = Box<dyn Fn(usize, Vec<u8>) -> Result<Vec<u8>, SignError> + Send + Sync>;

/// Only `spki_der` and `sign`.
pub struct Opaque {
    pair: ring::signature::EcdsaKeyPair,
    asked: AtomicUsize,
    how: How,
}

impl Opaque {
    pub fn new() -> Arc<Opaque> {
        Opaque::answering(Box::new(|_, sig| Ok(sig)))
    }

    pub fn answering(how: How) -> Arc<Opaque> {
        use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        Arc::new(Opaque { pair, asked: AtomicUsize::new(0), how })
    }

    pub fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    pub fn pin(&self) -> String {
        tls::pin_of_spki(&self.spki_der())
    }
}

impl DeviceSigner for Opaque {
    fn spki_der(&self) -> Vec<u8> {
        use ring::signature::KeyPair;
        sign::p256_spki(self.pair.public_key().as_ref()).unwrap()
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignError> {
        let n = self.asked.fetch_add(1, Ordering::SeqCst);
        let sig = self.pair.sign(&ring::rand::SystemRandom::new(), message).unwrap().as_ref().to_vec();
        (self.how)(n, sig)
    }
}
