//! A paired device's side of `worktrees serve`'s remote listener (Server
//! Edition Phase 2, `docs/proposals/server-edition-phase2.md` §2.2–§2.4,
//! §4.2–§4.4): pairing by link, the device key, the hosts it keeps, and the
//! pinned mutual-TLS WebSocket as a PIPE of text frames.
//!
//! The pipe parses nothing. The protocol over it is the loopback one,
//! handshake v1 included — keyed with the TLS session's exporter instead of
//! serve.json's token ([`tls::EXPORTER_SESSION`]) — so the page's
//! `app/src/ipcws.ts` stays its one client; [`call`] is the small Rust copy
//! `worktrees remote call` and the CI witness use.
//!
//! No tauri, no tmux, no engine: the desktop shim, the CLI and the iOS app (a
//! separate repository) all depend on this crate.
//!
//! ## Keys are signers
//!
//! The crate never needs a private key's bytes. Everything that would —
//! the TLS client certificate and its handshake signature, pairing, the pipe,
//! a step-up proof — asks a [`sign::DeviceSigner`] for its public half and a
//! signature (ECDSA P-256 / SHA-256, DER), and checks the signature against
//! that public half before using it. A pairing's keys travel as
//! [`sign::Keys`]: the device key, and on an iPhone or iPad the step-up key.
//! A desktop's key is a `0600` file ([`key::DeviceKey`], `Keys::from(key)`);
//! the iOS app supplies its two Secure Enclave keys by implementing the trait
//! over its Swift plugin. `IOS.md` beside this crate is the contract that
//! plugin meets: key attributes, signature format, which key signs what, and
//! how a cancelled Face ID is reported.
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::time::Duration;
//! use worktrees_remote::sign::{p256_spki, DeviceSigner, Keys, SignError};
//! use worktrees_remote::{pair, pipe, stepup};
//!
//! /// A Secure Enclave key, by its keychain tag.
//! struct Enclave {
//!     tag: &'static str,
//!     spki: Vec<u8>, // p256_spki(SecKeyCopyExternalRepresentation(public key))
//! }
//!
//! impl DeviceSigner for Enclave {
//!     fn spki_der(&self) -> Vec<u8> {
//!         self.spki.clone()
//!     }
//!     fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignError> {
//!         // SecKeyCreateSignature(key, .ecdsaSignatureMessageX962SHA256, message),
//!         // through the Swift plugin, which hands back its CFError's domain,
//!         // code and description: a dismissed Face ID is SignError::Cancelled.
//!         swift_sign(self.tag, message).map_err(|(domain, code, why)| SignError::from_apple(&domain, code, &why))
//!     }
//! }
//! # fn swift_sign(_: &str, _: &[u8]) -> Result<Vec<u8>, (String, i64, String)> { Err(("com.apple.LocalAuthentication".into(), -2, "Canceled by user.".into())) }
//!
//! # async fn shim(link: &str, device_point: &[u8], stepup_point: &[u8], challenge_b64: &str, now: i64) -> Result<(), Box<dyn std::error::Error>> {
//! let keys = Keys {
//!     device: Arc::new(Enclave { tag: "device", spki: p256_spki(device_point).ok_or("not a P-256 point")? }),
//!     stepup: Some(Arc::new(Enclave { tag: "stepup", spki: p256_spki(stepup_point).ok_or("not a P-256 point")? })),
//! };
//! // Pair: the crate keeps nothing; the app keeps the record beside its keys.
//! let rec = pair::pair_keys(link, "Bret's iPhone", pair::Platform::Ios, &keys, now, &mut |_event| {}).await?;
//! // Connect: the page runs the protocol over the pipe; the shim keeps the step-up.
//! let p = pipe::open(&rec, &keys, Duration::from_secs(10)).await?;
//! let stepup = p.stepup().expect("a pairing with a step-up key has one");
//! // A step-up, when the page has a challenge: Face ID shows, off the reactor.
//! let challenge = stepup::parse_challenge(challenge_b64).ok_or("not a challenge")?;
//! match stepup.prove(&challenge).await {
//!     Ok(proof) => { let _args = proof.args(); } // the page sends `stepup_prove` with these
//!     Err(SignError::Cancelled) => {}             // say so; nothing is sent
//!     Err(e) => return Err(e.into()),
//! }
//! # Ok(()) }
//! ```

pub mod call;
pub mod protocol;

/// Text a HOST chose — a close reason, an error, an event's name — made safe
/// to print: control characters and the invisible or direction-changing ones
/// (`pairuri::display_name_ok`'s set) become `\u{…}` escapes, so a host cannot
/// write terminal escape sequences into the device's screen, and it is cut at
/// 300 characters.
pub fn printable(s: &str) -> String {
    let odd = |c: char| {
        c.is_control()
            || matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{FEFF}')
            || ('\u{200B}'..='\u{200F}').contains(&c)
            || ('\u{2028}'..='\u{202E}').contains(&c)
            || ('\u{2060}'..='\u{2069}').contains(&c)
    };
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i == 300 {
            out.push('…');
            break;
        }
        if odd(c) {
            out.push_str(&c.escape_unicode().to_string());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_text_cannot_drive_the_terminal() {
        let s = super::printable("revoked\u{1b}]0;pwned\u{7}\u{1b}[2J\nnext\u{202E}line");
        assert!(!s.chars().any(|c| c.is_control() || c == '\u{202E}'), "{s:?}");
        assert!(s.starts_with("revoked\\u{1b}]0;pwned"), "{s}");
        assert_eq!(super::printable(&"x".repeat(400)).chars().count(), 301);
    }
}
pub mod dial;
pub mod discover;
pub mod hosts;
pub mod key;
pub mod pair;
pub mod pipe;
pub mod reach;
pub mod sign;
pub mod stepup;
pub mod tls;
