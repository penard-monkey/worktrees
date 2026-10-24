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
//!
//! **The device's key is a signer, never bytes** (`crate::sign`). Its
//! certificate is made by rcgen with the key as a REMOTE signer
//! ([`device_cert`]: rcgen 0.14 signs through any `rcgen::SigningKey`, which
//! is the point and a `sign`), and rustls holds it as a `SigningKey` whose
//! `Signer` asks the same trait ([`certified_key`]). So a dial costs the key
//! two signatures, the certificate's and the handshake's, and a Secure
//! Enclave key serves exactly as a key file does.

use std::sync::Arc;

use std::cell::RefCell;

use rcgen::{CertificateParams, PublicKeyData, PKCS_ECDSA_P256_SHA256};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime};
use rustls::sign::CertifiedKey;
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, Error, OtherError, SignatureAlgorithm, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::sign::{checked_sign, p256_point, spki_of, DeviceSigner, SignError};

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

/// rcgen's view of a device key: the point, and signatures through
/// [`checked_sign`]. rcgen reports a failed signature only as
/// `RemoteKeyError`, so the signer's own error is kept here for the caller.
struct CertSigner<'a> {
    key: &'a dyn DeviceSigner,
    spki: &'a [u8],
    failed: RefCell<Option<SignError>>,
}

impl PublicKeyData for CertSigner<'_> {
    fn der_bytes(&self) -> &[u8] {
        p256_point(self.spki).unwrap_or_default()
    }

    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

impl rcgen::SigningKey for CertSigner<'_> {
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        checked_sign(self.key, self.spki, msg).map_err(|e| {
            *self.failed.borrow_mut() = Some(e);
            rcgen::Error::RemoteKeyError
        })
    }
}

/// The certificate a device presents: self-signed by `key` — through the
/// signer, so the key's bytes are never needed — naming nobody. The server
/// pins its SPKI and reads nothing else in it. It is rcgen's certificate from
/// the same parameters a key file's always had, so for one key the
/// to-be-signed part is the same bytes either way.
pub fn device_cert(key: &dyn DeviceSigner) -> Result<CertificateDer<'static>, SignError> {
    let spki = spki_of(key, "device")?;
    let signer = CertSigner { key, spki: &spki, failed: RefCell::new(None) };
    let params = CertificateParams::new(Vec::<String>::new()).map_err(|e| SignError::Failed(e.to_string()))?;
    match params.self_signed(&signer) {
        Ok(cert) => Ok(cert.der().clone()),
        Err(e) => Err(signer.failed.take().unwrap_or_else(|| SignError::Failed(format!("its certificate: {e}")))),
    }
}

/// A device key as rustls holds it: [`SCHEME`] alone, every signature through
/// [`checked_sign`].
struct TlsKey {
    key: Arc<dyn DeviceSigner>,
    spki: Arc<[u8]>,
}

impl std::fmt::Debug for TlsKey {
    /// The pin, never more.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsKey").field("pin", &pin_of_spki(&self.spki)).finish()
    }
}

impl rustls::sign::SigningKey for TlsKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn rustls::sign::Signer>> {
        offered.contains(&SCHEME).then(|| Box::new(TlsSigner { key: self.key.clone(), spki: self.spki.clone() }) as Box<dyn rustls::sign::Signer>)
    }

    fn public_key(&self) -> Option<SubjectPublicKeyInfoDer<'_>> {
        Some(SubjectPublicKeyInfoDer::from(&self.spki[..]))
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ECDSA
    }
}

struct TlsSigner {
    key: Arc<dyn DeviceSigner>,
    spki: Arc<[u8]>,
}

impl std::fmt::Debug for TlsSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsSigner").field("pin", &pin_of_spki(&self.spki)).finish()
    }
}

impl rustls::sign::Signer for TlsSigner {
    /// The handshake's CertificateVerify. A refusal ends the handshake with
    /// the [`SignError`] inside it, which [`sign_error`] finds again.
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, Error> {
        checked_sign(&*self.key, &self.spki, message).map_err(|e| Error::Other(OtherError(Arc::new(e))))
    }

    fn scheme(&self) -> SignatureScheme {
        SCHEME
    }
}

/// A device key as a client certificate: its certificate, made now (one
/// signature), and the key itself for the handshake's (a second, at each
/// dial). A key that cannot sign fails HERE, before anything is dialled.
pub fn certified_key(key: Arc<dyn DeviceSigner>) -> Result<Arc<CertifiedKey>, SignError> {
    let cert = device_cert(&*key)?;
    let spki: Arc<[u8]> = spki_of(&*key, "device")?.into();
    Ok(Arc::new(CertifiedKey::new(vec![cert], Arc::new(TlsKey { key, spki }))))
}

/// The [`SignError`] that ended a handshake, when one did: the device's own
/// key refused or failed to sign, not the network or the host.
pub fn sign_error(e: &std::io::Error) -> Option<&SignError> {
    match e.get_ref()?.downcast_ref::<Error>()? {
        Error::Other(OtherError(inner)) => inner.downcast_ref::<SignError>(),
        _ => None,
    }
}

/// A device's config: TLS 1.3, the server pinned, `key` presented
/// ([`certified_key`]: a device key behind [`DeviceSigner`]), no SNI, nothing
/// resumed, no early data.
pub fn client_config(pin: &str, key: Arc<CertifiedKey>) -> Result<Arc<ClientConfig>, String> {
    let mut c = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServer::new(pin)))
        .with_client_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(key)));
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
