//! TLS for a paired device (Phase 2 §4.2, §4.3): TLS 1.3 only, mutual, the
//! server pinned by its SPKI, over rustls with the `ring` provider.
//!
//! This is the ONE client half. serve's `tls.rs` keeps the server half (its
//! config, the device and pairing verifiers, the post-handshake check) and
//! imports what both halves share from here: the scheme, the provider, the
//! pin, rustls's signature check, and the exporter labels.
//!
//! **No resumption, no early data, no SNI.** rustls does not run the client
//! verifier again when a session resumes, so a device revoked after its
//! ticket was issued could resume past it (§4.2): a device resumes nothing.
//! It sends no SNI either, so a spoofed listener learns no host name from its
//! ClientHello (§4.7 D4) — the pin needs no name.
//!
//! [`PinnedServer`] does exactly two things, as §4.3 asks of every custom
//! verifier: the server's SPKI must equal the pin, and the handshake
//! signature is checked by rustls's own helper. No name, no dates, no issuer.
//! In TLS 1.3 the server proves its key before the client sends a
//! certificate, so a server that is not the pin never sees the device's.
//!
//! Every signature is ECDSA P-256 with SHA-256, the one scheme offered: the
//! Secure Enclave signs with P-256 and nothing else (§2.1).

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};

/// The exporter label a pairing proof is bound with (§2.2 step 4).
pub const EXPORTER_PAIR: &[u8] = b"EXPORTER-worktrees-pair-v1";

/// The exporter label a step-up signature is bound with (§2.5).
pub const EXPORTER_STEPUP: &[u8] = b"EXPORTER-worktrees-stepup-v1";

/// The exporter label of a remote session's handshake-v1 key: both ends take
/// 32 bytes under it (no context), and its lowercase hex is the HMAC key
/// handshake v1 runs with, in place of serve.json's token. A departure from
/// §4.5, decided by (main) on 2026-10-22: one client for the protocol
/// (`ipcws.ts`, unchanged), and the value works only inside the TLS session
/// the device's own process holds.
pub const EXPORTER_SESSION: &[u8] = b"EXPORTER-worktrees-remote-v1";

/// The one signature scheme: ECDSA P-256, SHA-256.
pub const SCHEME: SignatureScheme = SignatureScheme::ECDSA_NISTP256_SHA256;

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Lowercase hex.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The pin of a DER SubjectPublicKeyInfo: its SHA-256, lowercase hex.
pub fn pin_of_spki(spki_der: &[u8]) -> String {
    hex(&Sha256::digest(spki_der))
}

/// The pin of a certificate: SHA-256 of its SPKI. `None` for one that does
/// not parse.
pub fn pin_of_cert(cert: &CertificateDer<'_>) -> Option<String> {
    let parsed = rustls::server::ParsedCertificate::try_from(cert).ok()?;
    Some(pin_of_spki(parsed.subject_public_key_info().as_ref()))
}

/// rustls's own TLS 1.3 signature check, for [`SCHEME`] alone.
pub fn verify13(message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
    if dss.scheme != SCHEME {
        return Err(Error::PeerIncompatible(rustls::PeerIncompatible::NoSignatureSchemesInCommon));
    }
    rustls::crypto::verify_tls13_signature(message, cert, dss, &provider().signature_verification_algorithms)
}

/// What every verifier answers a TLS 1.2 signature with.
pub fn no_tls12() -> Error {
    Error::PeerIncompatible(rustls::PeerIncompatible::Tls13RequiredForQuic)
}

/// A device's server verifier: the server's key is the pinned one.
#[derive(Debug)]
pub struct PinnedServer {
    pin: String,
}

impl PinnedServer {
    pub fn new(pin: &str) -> Self {
        PinnedServer { pin: pin.to_string() }
    }
}

impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, Error> {
        match pin_of_cert(end_entity) {
            Some(p) if p == self.pin => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)),
            None => Err(Error::InvalidCertificate(CertificateError::BadEncoding)),
        }
    }

    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        Err(no_tls12())
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify13(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SCHEME]
    }
}

/// A device's config: TLS 1.3, the server pinned, its own key presented, no
/// SNI, nothing resumed, no early data.
pub fn client_config(pin: &str, cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> Result<Arc<ClientConfig>, String> {
    let mut c = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServer::new(pin)))
        .with_client_auth_cert(vec![cert], key)
        .map_err(|e| e.to_string())?;
    c.resumption = rustls::client::Resumption::disabled();
    c.enable_sni = false;
    c.enable_early_data = false;
    Ok(Arc::new(c))
}

/// 32 bytes of a client session's TLS 1.3 exporter (RFC 8446 §7.5) under
/// `label`, no context.
pub fn client_exporter(conn: &rustls::ClientConnection, label: &[u8]) -> Result<[u8; 32], String> {
    conn.export_keying_material([0u8; 32], label, None).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    /// The labels are wire constants both ends export under: a changed byte
    /// is a device and a host that can no longer agree on a key, a proof or a
    /// step-up — and nothing else would say why. Spelled out here, from the
    /// proposal (§2.2, §2.5) and the remote handshake's decision.
    #[test]
    fn the_exporter_labels_are_the_wires() {
        assert_eq!(super::EXPORTER_PAIR, b"EXPORTER-worktrees-pair-v1");
        assert_eq!(super::EXPORTER_STEPUP, b"EXPORTER-worktrees-stepup-v1");
        assert_eq!(super::EXPORTER_SESSION, b"EXPORTER-worktrees-remote-v1");
    }
}

/// Did this handshake fail because the server's key is not the pin? (The
/// device's own refusal, made before it sent anything of its own.)
pub fn is_pin_mismatch(e: &std::io::Error) -> bool {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<Error>())
        .is_some_and(|e| matches!(e, Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)))
}
