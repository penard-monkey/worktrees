//! TLS for paired devices (Phase 2 §4.2, §4.3): TLS 1.3 only, mutual, pinned
//! both ways, over rustls with the `ring` provider.
//!
//! **No resumption, no tickets, no 0-RTT, no SNI.** rustls does not run the
//! client verifier again when a session resumes, so a device revoked after its
//! ticket was issued could resume past it (§4.2): the server keeps no session
//! cache and issues no ticket, the client resumes nothing, and early data is
//! off on both. The client sends no SNI, so a spoofed listener learns no host
//! name from a device's ClientHello (§4.7 D4).
//!
//! **Three verifiers, each doing exactly two things** (§4.3, where rustls
//! users write their bugs):
//!
//! - [`DeviceVerifier`] (the remote listener's, slice 6): the client
//!   certificate's SPKI pin must be a live device's — `DeviceRegistry::admit`,
//!   which also marks an expiry at the knock — and the handshake signature is
//!   checked by rustls's own `verify_tls13_signature`. Validity dates and the
//!   issuer are ignored: the certificates are self-signed and the pin is the
//!   identity. [`admit_peer`] runs the same check again AFTER the handshake,
//!   from `peer_certificates()`: the verifier is the early exit, not the only
//!   gate.
//! - [`AnyKey`] (the pairing listener's): any certificate, because the
//!   pairing proof is what authenticates there (§2.2) — but the signature
//!   must verify, so a device proves it holds the key it is about to enroll.
//! - [`PinnedServer`] (a device's, in `worktrees_remote::tls` with the rest
//!   of the client half): the server's SPKI must equal the pin; no name is
//!   checked.
//!
//! Every signature is ECDSA P-256 with SHA-256, the one scheme offered: the
//! Secure Enclave signs with P-256 and nothing else (§2.1).

use std::sync::Arc;

use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, Error, ServerConfig, SignatureScheme};
#[cfg(test)]
use rustls::{pki_types::{PrivateKeyDer, ServerName}, ClientConfig};
use worktrees_core::authz::Caller;

#[cfg(test)]
use super::devices::pin_of;
use super::devices::{ConnectRefusal, DeviceRegistry};
use super::identity::Identity;

// The client half, and what both halves share — the exporter labels, the one
// scheme, the provider, the pin, rustls's signature check — are
// `worktrees_remote::tls`'s: one implementation for serve and its devices.
pub use worktrees_remote::tls::{pin_of_cert, provider, EXPORTER_PAIR, EXPORTER_STEPUP, SCHEME};
use worktrees_remote::tls::{no_tls12, verify13};
// The client half, for serve's own tests of a device.
#[cfg_attr(not(test), allow(unused_imports))]
pub use worktrees_remote::tls::{client_config, PinnedServer};

/// The remote listener's client verifier: a live device's key, and a valid
/// signature with it. `peer` is the address the connection came from, which
/// a refused key's event names (§5.1), so the remote listener makes one per
/// connection ([`with_verifier`]).
#[derive(Debug)]
pub struct DeviceVerifier {
    devices: Arc<DeviceRegistry>,
    peer: Option<std::net::IpAddr>,
}

impl DeviceVerifier {
    /// For the base config, which no connection is served by.
    pub fn new(devices: Arc<DeviceRegistry>) -> Self {
        DeviceVerifier { devices, peer: None }
    }

    /// For one connection, from `peer`.
    pub fn for_peer(devices: Arc<DeviceRegistry>, peer: std::net::IpAddr) -> Self {
        DeviceVerifier { devices, peer: Some(peer) }
    }
}

impl std::fmt::Debug for DeviceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceRegistry")
    }
}

impl ClientCertVerifier for DeviceVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, Error> {
        let pin = pin_of_cert(end_entity).ok_or(Error::InvalidCertificate(CertificateError::BadEncoding))?;
        self.devices
            .admit(&pin, worktrees_core::sysclock::now_epoch(), self.peer)
            .map(|_| ClientCertVerified::assertion())
            .map_err(|_| Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure))
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

/// The pairing listener's client verifier: any well-formed certificate, and a
/// valid signature with its key.
#[derive(Debug, Default)]
pub struct AnyKey;

impl ClientCertVerifier for AnyKey {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, Error> {
        pin_of_cert(end_entity).map(|_| ClientCertVerified::assertion()).ok_or(Error::InvalidCertificate(CertificateError::BadEncoding))
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

/// The server's config: TLS 1.3, the identity's certificate, `clients` as
/// the client verifier, and nothing that resumes.
pub fn server_config(id: &Identity, clients: Arc<dyn ClientCertVerifier>) -> Result<Arc<ServerConfig>, String> {
    let c = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_client_cert_verifier(clients)
        .with_single_cert(vec![id.cert()], id.key())
        .map_err(|e| format!("the server identity: {e}"))?;
    Ok(hardened(c))
}

/// `base` with `clients` as its client verifier: the same certificate, and
/// every setting [`server_config`] makes. rustls keeps a config's verifier
/// private, so this is a new config around `base`'s certificate resolver
/// (the key is not parsed again). Measured in release on macOS, 2026-10-23:
/// 0.3 µs to build; a handshake's median moved +2 µs at 8 in flight and +3
/// µs at 1 (of ~420 and ~270 µs), inside the ±15 µs between rounds; and on
/// the remote listener itself, TLS to handshake v1, no difference at all.
pub fn with_verifier(base: &ServerConfig, clients: Arc<dyn ClientCertVerifier>) -> Result<Arc<ServerConfig>, String> {
    let c = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_client_cert_verifier(clients)
        .with_cert_resolver(base.cert_resolver.clone());
    Ok(hardened(c))
}

/// Nothing that resumes, and no early data.
fn hardened(mut c: ServerConfig) -> Arc<ServerConfig> {
    c.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    c.send_tls13_tickets = 0;
    c.max_early_data_size = 0;
    Arc::new(c)
}

/// 32 bytes of the session's TLS 1.3 exporter (RFC 8446 §7.5) under `label`,
/// no context: what binds a pairing proof or a step-up to THIS session.
pub fn exporter(conn: &rustls::ServerConnection, label: &[u8]) -> Result<[u8; 32], String> {
    conn.export_keying_material([0u8; 32], label, None).map_err(|e| e.to_string())
}

/// The device check again, after the handshake, from the peer's certificate
/// against the live set (§4.2): a device revoked between the verifier and
/// here is refused all the same, its event naming `peer`.
pub fn admit_peer(conn: &rustls::ServerConnection, devices: &DeviceRegistry, now: i64, peer: Option<std::net::IpAddr>) -> Result<Caller, ConnectRefusal> {
    let cert = conn.peer_certificates().and_then(|c| c.first()).ok_or(ConnectRefusal::Unknown)?;
    let pin = pin_of_cert(cert).ok_or(ConnectRefusal::Unknown)?;
    devices.admit(&pin, now, peer)
}

#[cfg(test)]
pub(crate) mod testkit {
    //! A device's side, for the tests: a key and its self-signed certificate,
    //! and a TLS client that pins the server.
    use super::*;
    use rcgen::{CertificateParams, KeyPair, PublicKeyData};

    pub struct DeviceKey {
        pub cert: CertificateDer<'static>,
        pub key: PrivateKeyDer<'static>,
        pub spki: Vec<u8>,
        pub pin: String,
    }

    pub fn device_key_with(alg: &'static rcgen::SignatureAlgorithm) -> DeviceKey {
        let pair = KeyPair::generate_for(alg).unwrap();
        let cert = CertificateParams::new(Vec::<String>::new()).unwrap().self_signed(&pair).unwrap();
        let spki = pair.subject_public_key_info();
        DeviceKey {
            cert: cert.der().clone(),
            key: PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(pair.serialize_der())),
            pin: pin_of(&spki),
            spki,
        }
    }

    pub fn device_key() -> DeviceKey {
        device_key_with(&rcgen::PKCS_ECDSA_P256_SHA256)
    }

    /// Dial `addr` as a device with `key`, pinning `pin`.
    pub async fn dial(addr: std::net::SocketAddr, pin: &str, key: &DeviceKey) -> std::io::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        let cfg = client_config(pin, key.cert.clone(), key.key.clone_key()).unwrap();
        let tcp = tokio::net::TcpStream::connect(addr).await?;
        tokio_rustls::TlsConnector::from(cfg).connect(ServerName::IpAddress(addr.ip().into()), tcp).await
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::serve::devices::{Enrollment, Expiry, Platform};
    use std::sync::atomic::{AtomicBool, Ordering};
    use worktrees_core::authz::{Capability, Caps, Grant, PrincipalId, Projects};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
    }

    fn identity(tag: &str) -> (Identity, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("wt-tls-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        (Identity::ensure(&d).unwrap(), d)
    }

    fn enroll(reg: &DeviceRegistry, k: &DeviceKey) -> worktrees_core::authz::DeviceId {
        reg.enroll(
            Enrollment {
                principal: PrincipalId::owner(),
                name: "Laptop".into(),
                platform: Platform::Macos,
                spki_sha256: k.pin.clone(),
                stepup_spki_sha256: None,
                grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All },
                expiry: Expiry::default(),
                addr: None,
            },
            worktrees_core::sysclock::now_epoch(),
        )
        .unwrap()
        .id
    }

    /// What one handshake on 127.0.0.1 came to, on each side: the server's
    /// post-handshake device check, its exporter, whether SNI arrived; the
    /// client's exporter.
    struct Outcome {
        server: Result<(Result<Caller, ConnectRefusal>, [u8; 32], bool), String>,
        client: Result<[u8; 32], String>,
    }

    fn handshake(rt: &tokio::runtime::Runtime, cfg: Arc<ServerConfig>, devices: Arc<DeviceRegistry>, pin: &str, key: &DeviceKey) -> Outcome {
        rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = l.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (tcp, _) = l.accept().await.unwrap();
                let tls = tokio_rustls::TlsAcceptor::from(cfg).accept(tcp).await.map_err(|e| e.to_string())?;
                let conn = tls.get_ref().1;
                let ex = exporter(conn, EXPORTER_PAIR)?;
                Ok((admit_peer(conn, &devices, worktrees_core::sysclock::now_epoch(), None), ex, conn.server_name().is_some()))
            });
            let client = match dial(addr, pin, key).await {
                Ok(tls) => tls.get_ref().1.export_keying_material([0u8; 32], EXPORTER_PAIR, None).map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            Outcome { server: server.await.unwrap(), client }
        })
    }

    /// §4.3's device branch: an enrolled key is served, and the session's
    /// exporter is the same on both sides; an unknown key, a revoked one and
    /// a disabled one are refused AT THE HANDSHAKE — and a re-enabled one is
    /// served again.
    #[test]
    fn only_a_live_devices_key_completes_the_handshake() {
        let rt = rt();
        let (id, dir) = identity("device");
        let devices = Arc::new(DeviceRegistry::in_memory());
        let cfg = server_config(&id, Arc::new(DeviceVerifier::new(devices.clone()))).unwrap();
        let k = device_key();
        let o = handshake(&rt, cfg.clone(), devices.clone(), id.pin(), &k);
        // TLS 1.3: the client's half completes before the server judges its
        // certificate; the refusal is the server's, and the client learns it
        // at its first read.
        assert!(o.server.is_err(), "an unknown key is refused");
        let dev = enroll(&devices, &k);
        let o = handshake(&rt, cfg.clone(), devices.clone(), id.pin(), &k);
        let (who, sx, sni) = o.server.unwrap();
        assert_eq!(who, Ok(Caller::paired(PrincipalId::owner(), dev.clone())));
        assert_eq!(Some(sx), o.client.ok(), "both ends export the same bytes");
        assert!(!sni, "the device sends no SNI");
        devices.disable(&dev, worktrees_core::sysclock::now_epoch()).unwrap();
        assert!(handshake(&rt, cfg.clone(), devices.clone(), id.pin(), &k).server.is_err(), "disabled");
        devices.enable(&dev, worktrees_core::sysclock::now_epoch()).unwrap();
        assert!(handshake(&rt, cfg.clone(), devices.clone(), id.pin(), &k).server.is_ok(), "re-enabled");
        devices.revoke(&Caller::console(), &Grant::everything(), &dev, worktrees_core::sysclock::now_epoch()).unwrap();
        assert!(handshake(&rt, cfg, devices, id.pin(), &k).server.is_err(), "revoked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The step-up exporter (§2.5) is the session's own, the same at both
    /// ends, and not the pairing one: a proof made for one purpose is not one
    /// for the other.
    #[test]
    fn the_step_up_exporter_is_this_sessions_and_only_for_step_up() {
        let rt = rt();
        let (id, dir) = identity("stepup-exporter");
        let cfg = server_config(&id, Arc::new(AnyKey)).unwrap();
        let k = device_key();
        let (server, client) = rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = l.local_addr().unwrap();
            let s = tokio::spawn(async move {
                let (tcp, _) = l.accept().await.unwrap();
                let tls = tokio_rustls::TlsAcceptor::from(cfg).accept(tcp).await.unwrap();
                let c = tls.get_ref().1;
                (exporter(c, EXPORTER_STEPUP).unwrap(), exporter(c, EXPORTER_PAIR).unwrap())
            });
            let tls = dial(addr, id.pin(), &k).await.unwrap();
            let c = tls.get_ref().1.export_keying_material([0u8; 32], EXPORTER_STEPUP, None).unwrap();
            (s.await.unwrap(), c)
        });
        assert_eq!(server.0, client, "both ends");
        assert_ne!(server.0, server.1, "not the pairing exporter");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A certificate's key and the key that signs must be the same: a device
    /// holding someone else's certificate cannot finish the handshake.
    #[test]
    fn the_right_certificate_with_the_wrong_key_is_refused() {
        let rt = rt();
        let (id, dir) = identity("wrongkey");
        let devices = Arc::new(DeviceRegistry::in_memory());
        let real = device_key();
        enroll(&devices, &real);
        let thief = device_key();
        let forged = DeviceKey { cert: real.cert.clone(), key: thief.key.clone_key(), spki: real.spki.clone(), pin: real.pin.clone() };
        for clients in [Arc::new(DeviceVerifier::new(devices.clone())) as Arc<dyn ClientCertVerifier>, Arc::new(AnyKey)] {
            let cfg = server_config(&id, clients).unwrap();
            // rustls may refuse the mismatched pair on the client already;
            // either way no session exists on the server.
            let o = rt.block_on(async {
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = l.local_addr().unwrap();
                let server = tokio::spawn(async move {
                    let (tcp, _) = l.accept().await.unwrap();
                    tokio_rustls::TlsAcceptor::from(cfg).accept(tcp).await.map(|_| ()).map_err(|e| e.to_string())
                });
                let cfg = ClientConfig::builder_with_provider(provider())
                    .with_protocol_versions(&[&rustls::version::TLS13])
                    .unwrap()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(PinnedServer::new(id.pin())))
                    .with_client_cert_resolver(Arc::new(Forged(Arc::new(rustls::sign::CertifiedKey::new(
                        vec![forged.cert.clone()],
                        provider().key_provider.load_private_key(forged.key.clone_key()).unwrap(),
                    )))));
                let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
                let _ = tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(ServerName::IpAddress(addr.ip().into()), tcp).await;
                server.await.unwrap()
            });
            assert!(o.is_err(), "the server refused the signature");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[derive(Debug)]
    struct Forged(Arc<rustls::sign::CertifiedKey>);

    impl rustls::client::ResolvesClientCert for Forged {
        fn resolve(&self, _: &[&[u8]], _: &[SignatureScheme]) -> Option<Arc<rustls::sign::CertifiedKey>> {
            Some(self.0.clone())
        }
        fn has_certs(&self) -> bool {
            true
        }
    }

    /// The pairing listener takes ANY key (the proof authenticates there),
    /// and the device listener refuses that same key until it is enrolled:
    /// "pairing-only" (§4.3).
    #[test]
    fn a_pairing_key_is_accepted_for_pairing_and_nowhere_else() {
        let rt = rt();
        let (id, dir) = identity("pairing");
        let devices = Arc::new(DeviceRegistry::in_memory());
        let k = device_key();
        let pairing = server_config(&id, Arc::new(AnyKey)).unwrap();
        let remote = server_config(&id, Arc::new(DeviceVerifier::new(devices.clone()))).unwrap();
        let o = handshake(&rt, pairing, devices.clone(), id.pin(), &k);
        assert!(o.server.is_ok() && o.client.is_ok(), "pairing: any key");
        assert_eq!(o.server.unwrap().0, Err(ConnectRefusal::Unknown), "and it is nobody's device yet");
        assert!(handshake(&rt, remote, devices, id.pin(), &k).server.is_err(), "remote: refused");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only P-256 (§2.1): a P-384 key is refused even on the pairing listener.
    #[test]
    fn a_key_that_is_not_p256_is_refused() {
        let rt = rt();
        let (id, dir) = identity("p384");
        let devices = Arc::new(DeviceRegistry::in_memory());
        let k = device_key_with(&rcgen::PKCS_ECDSA_P384_SHA384);
        let o = handshake(&rt, server_config(&id, Arc::new(AnyKey)).unwrap(), devices, id.pin(), &k);
        assert!(o.server.is_err(), "P-384 refused");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §4.3's client branch, and §4.7 D4: a server whose key is not the pin
    /// is refused BEFORE the device sends its certificate — the TLS 1.3 order
    /// — so a spoofer learns nothing about the device's key.
    #[test]
    fn a_server_that_is_not_the_pin_never_sees_the_devices_certificate() {
        #[derive(Debug)]
        struct Spy(Arc<AtomicBool>);
        impl ClientCertVerifier for Spy {
            fn root_hint_subjects(&self) -> &[DistinguishedName] {
                &[]
            }
            fn verify_client_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, Error> {
                self.0.store(true, Ordering::SeqCst);
                Ok(ClientCertVerified::assertion())
            }
            fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
                Err(no_tls12())
            }
            fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
                verify13(m, c, d)
            }
            fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
                vec![SCHEME]
            }
        }
        let rt = rt();
        let (real, d1) = identity("pin-real");
        let (spoof, d2) = identity("pin-spoof");
        let saw = Arc::new(AtomicBool::new(false));
        let devices = Arc::new(DeviceRegistry::in_memory());
        let o = handshake(&rt, server_config(&spoof, Arc::new(Spy(saw.clone()))).unwrap(), devices.clone(), real.pin(), &device_key());
        assert!(o.client.is_err(), "the device refuses the wrong key");
        assert!(!saw.load(Ordering::SeqCst), "and the spoofer never saw its certificate");
        let o = handshake(&rt, server_config(&real, Arc::new(Spy(saw.clone()))).unwrap(), devices, real.pin(), &device_key());
        assert!(o.client.is_ok() && saw.load(Ordering::SeqCst), "the pinned server does");
        let _ = std::fs::remove_dir_all(&d1);
        let _ = std::fs::remove_dir_all(&d2);
    }

    /// §4.2: nothing that resumes — no session cache, no tickets, no early
    /// data on the server; no resumption, no early data, no SNI on a device.
    #[test]
    fn nothing_resumes_and_nothing_is_early() {
        let (id, dir) = identity("noresume");
        let s = server_config(&id, Arc::new(AnyKey)).unwrap();
        assert_eq!(s.send_tls13_tickets, 0);
        assert_eq!(s.max_early_data_size, 0);
        assert!(!s.session_storage.can_cache(), "no session cache");
        assert!(!s.ticketer.enabled(), "no tickets");
        let k = device_key();
        let c = client_config(id.pin(), k.cert.clone(), k.key.clone_key()).unwrap();
        assert!(!c.enable_sni && !c.enable_early_data);
        assert_eq!(format!("{:?}", c.resumption), format!("{:?}", rustls::client::Resumption::disabled()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
