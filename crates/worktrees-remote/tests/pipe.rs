//! The pipe and handshake v1 over it, against a fixture host: TLS 1.3 with a
//! server identity of its own, a client verifier that admits one pin, a
//! WebSocket upgrade that records what the client sent, and handshake v1 run
//! from the spec with the session's exporter as the key. The real remote
//! listener is serve's (the listener lane); serve's own tests pair against
//! `worktrees_remote::pair` end to end.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hmac::{KeyInit, Mac};
use rcgen::{CertificateParams, KeyPair, PublicKeyData, PKCS_ECDSA_P256_SHA256};
use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, Error, SignatureScheme};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use worktrees_remote::call::{self, CallError};
use worktrees_remote::key::DeviceKey;
use worktrees_remote::pipe::{self, Inbound, PipeError};
use worktrees_remote::tls;

const WAIT: Duration = Duration::from_secs(5);
const VERSION: &str = "9.9.9";

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
}

/// The fixture host's identity.
struct Identity {
    cert: CertificateDer<'static>,
    key: Vec<u8>,
    pin: String,
}

fn identity() -> Identity {
    let pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = CertificateParams::new(Vec::<String>::new()).unwrap().self_signed(&pair).unwrap();
    Identity { cert: cert.der().clone(), key: pair.serialize_der(), pin: tls::pin_of_spki(&pair.subject_public_key_info()) }
}

/// Admits one pin; remembers whether it was ever asked.
#[derive(Debug)]
struct OnePin {
    pin: String,
    asked: Arc<AtomicBool>,
}

impl ClientCertVerifier for OnePin {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(&self, c: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, Error> {
        self.asked.store(true, Ordering::SeqCst);
        match tls::pin_of_cert(c) {
            Some(p) if p == self.pin => Ok(ClientCertVerified::assertion()),
            _ => Err(Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)),
        }
    }
    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        Err(tls::no_tls12())
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        tls::verify13(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![tls::SCHEME]
    }
}

/// What the fixture host saw.
#[derive(Default, Debug)]
struct Seen {
    host: Option<String>,
    origin: Option<String>,
    session_key: Option<String>,
    requests: Vec<Value>,
    asked: bool,
}

/// How the fixture misbehaves, when asked to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Host {
    Honest,
    /// Proves with another key.
    WrongProof,
    /// Closes with 1008 "revoked" right after the handshake.
    Revokes,
}

/// Serve one connection; the address, and what it saw once it is over.
fn host(rt: &tokio::runtime::Runtime, id: &Identity, admit: &str, how: Host) -> (SocketAddr, tokio::task::JoinHandle<Seen>) {
    let asked = Arc::new(AtomicBool::new(false));
    let mut cfg = rustls::ServerConfig::builder_with_provider(tls::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(Arc::new(OnePin { pin: admit.into(), asked: asked.clone() }))
        .with_single_cert(vec![id.cert.clone()], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key.clone())))
        .unwrap();
    cfg.send_tls13_tickets = 0;
    let cfg = Arc::new(cfg);
    let l = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = l.local_addr().unwrap();
    let task = rt.spawn(async move {
        let mut seen = Seen::default();
        let Ok((tcp, _)) = l.accept().await else { return seen };
        let tls = tokio_rustls::TlsAcceptor::from(cfg).accept(tcp).await;
        seen.asked = asked.load(Ordering::SeqCst);
        let Ok(tls) = tls else { return seen };
        let ex: [u8; 32] = tls.get_ref().1.export_keying_material([0u8; 32], b"EXPORTER-worktrees-remote-v1", None).unwrap();
        let key = tls::hex(&ex);
        seen.session_key = Some(key.clone());
        let hdr = Arc::new(Mutex::new((None, None)));
        let h = hdr.clone();
        let ws = tokio_tungstenite::accept_hdr_async(tls, move |req: &Request, resp: Response| {
            let get = |k: &str| req.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
            *h.lock().unwrap() = (get("host"), get("origin"));
            Ok(resp)
        })
        .await;
        (seen.host, seen.origin) = hdr.lock().unwrap().clone();
        let Ok(mut ws) = ws else { return seen };
        // Handshake v1, from the spec, keyed with the session's exporter.
        let mac = |label: &str, cn: &str, sn: &str| {
            let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).unwrap();
            m.update(format!("{label}{cn}\n{sn}").as_bytes());
            tls::hex(&m.finalize().into_bytes())
        };
        let read = |m: Option<Result<Message, _>>| -> Option<Value> {
            match m {
                Some(Ok(Message::Text(t))) => serde_json::from_str(&t).ok(),
                _ => None,
            }
        };
        let Some(hello) = read(ws.next().await) else { return seen };
        let cn = hello["hello"].as_str().unwrap_or_default().to_string();
        let sn = "5".repeat(64);
        ws.send(Message::text(json!({"challenge": sn}).to_string())).await.unwrap();
        let Some(auth) = read(ws.next().await) else { return seen };
        if auth["auth"] != json!(mac("worktrees-serve/v1/client\n", &cn, &sn)) {
            return seen;
        }
        let proof = if how == Host::WrongProof { mac("worktrees-serve/v1/server\n", &cn, &"6".repeat(64)) } else { mac("worktrees-serve/v1/server\n", &cn, &sn) };
        let device = json!({"id": "d_x", "principal": "owner", "name": "Laptop", "caps": ["read"], "projects": "*", "expiry": {"idle_days": 30}, "stepup_required": false});
        ws.send(Message::text(json!({"ok": true, "version": VERSION, "proof": proof, "device": device}).to_string())).await.unwrap();
        if how == Host::Revokes {
            let _ = ws.send(Message::Close(Some(CloseFrame { code: CloseCode::Policy, reason: "revoked".into() }))).await;
            return seen;
        }
        while let Some(req) = read(ws.next().await) {
            seen.requests.push(req.clone());
            let _ = ws.send(Message::text(json!({"event": "noise", "payload": 1}).to_string())).await;
            let _ = ws.send(Message::text(json!({"id": req["id"], "ok": {"echo": req["cmd"]}}).to_string())).await;
        }
        seen
    });
    (addr, task)
}

/// The served case first: a device whose key the host admits completes
/// handshake v1 over the pipe — keyed with ONE session value at both ends —
/// and is answered. The upgrade carries the address literal as Host and no
/// Origin (§4.5).
#[test]
fn a_paired_device_is_served_over_the_pipe_with_one_session_key_at_both_ends() {
    let rt = rt();
    let id = identity();
    let dev = DeviceKey::generate().unwrap();
    let (addr, seen) = host(&rt, &id, dev.pin(), Host::Honest);
    let (token, out) = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
        let token = p.session_token().to_string();
        let mut s = call::handshake(p, VERSION, WAIT).await.unwrap();
        let out = s.request(7, "list_workspace", json!({}), WAIT).await.unwrap();
        assert_eq!(s.welcome["device"]["id"], json!("d_x"), "the device record rides on ok");
        s.close().await;
        (token, out)
    });
    assert_eq!(out, json!({"echo": "list_workspace"}), "the event on the way was skipped");
    let seen = rt.block_on(seen).unwrap();
    assert_eq!(seen.session_key.as_deref(), Some(token.as_str()), "both ends key v1 with the same exporter");
    assert_eq!(seen.host.as_deref(), Some(addr.to_string().as_str()));
    assert_eq!(seen.origin, None, "a native client sends no Origin");
    assert_eq!(seen.requests, vec![json!({"id": 7, "cmd": "list_workspace", "args": {}})]);
}

/// §4.3's client branch: a host whose key is not the pin is refused BEFORE
/// the device's certificate goes out (TLS 1.3 order), so a spoofer learns
/// nothing — and the device says it was the wrong machine.
#[test]
fn a_host_that_is_not_the_pin_is_refused_before_it_sees_the_device() {
    let rt = rt();
    let (id, other) = (identity(), identity());
    let dev = DeviceKey::generate().unwrap();
    let (addr, seen) = host(&rt, &id, dev.pin(), Host::Honest);
    let r = rt.block_on(pipe::open_at(addr, &other.pin, &dev, WAIT));
    assert!(matches!(r, Err(PipeError::NotThePin)), "{:?}", r.err());
    let seen = rt.block_on(seen).unwrap();
    assert!(!seen.asked, "the host never saw the device's certificate");
    assert_eq!(seen.session_key, None);
}

/// A key the host does not hold (a device it never enrolled, or revoked) gets
/// no further than TLS: no upgrade, no frame.
#[test]
fn a_key_the_host_does_not_hold_is_refused_at_tls() {
    let rt = rt();
    let id = identity();
    let (dev, stranger) = (DeviceKey::generate().unwrap(), DeviceKey::generate().unwrap());
    let (addr, seen) = host(&rt, &id, dev.pin(), Host::Honest);
    let r = rt.block_on(pipe::open_at(addr, &id.pin, &stranger, WAIT));
    assert!(matches!(r, Err(PipeError::Tls(_))), "{:?}", r.err().map(|e| e.to_string()));
    let seen = rt.block_on(seen).unwrap();
    assert!(seen.asked && seen.session_key.is_none() && seen.host.is_none(), "refused inside TLS: {seen:?}");
}

/// Proof before anything: a host that cannot prove the session key gets no
/// request.
#[test]
fn nothing_is_sent_to_a_host_whose_proof_fails() {
    let rt = rt();
    let id = identity();
    let dev = DeviceKey::generate().unwrap();
    let (addr, seen) = host(&rt, &id, dev.pin(), Host::WrongProof);
    let r = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
        call::handshake(p, VERSION, WAIT).await.map(|_| ())
    });
    assert!(matches!(r, Err(CallError::Handshake(_))), "{:?}", r.err().map(|e| e.to_string()));
    assert!(rt.block_on(seen).unwrap().requests.is_empty());
}

/// A revoke reaches the pipe with its reason (1008, #544's `end_device`).
#[test]
fn a_revoke_ends_the_pipe_with_its_reason() {
    let rt = rt();
    let id = identity();
    let dev = DeviceKey::generate().unwrap();
    let (addr, _seen) = host(&rt, &id, dev.pin(), Host::Revokes);
    let end = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
        let mut s = call::handshake(p, VERSION, WAIT).await.unwrap();
        s.rx.recv().await
    });
    match end {
        Inbound::Closed(why) => assert!(why.contains("revoked"), "{why}"),
        other => panic!("{other:?}"),
    }
}

/// Another version is refused once proven, before any request.
#[test]
fn a_proven_host_of_another_version_is_not_driven() {
    let rt = rt();
    let id = identity();
    let dev = DeviceKey::generate().unwrap();
    let (addr, seen) = host(&rt, &id, dev.pin(), Host::Honest);
    let r = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
        call::handshake(p, "1.0.0", WAIT).await.map(|_| ())
    });
    assert!(matches!(r, Err(CallError::Version { .. })), "{:?}", r.err().map(|e| e.to_string()));
    assert!(rt.block_on(seen).unwrap().requests.is_empty());
}
