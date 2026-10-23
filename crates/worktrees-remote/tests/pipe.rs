//! The pipe and handshake v1 over it, against a fixture host: TLS 1.3 with a
//! server identity of its own, a client verifier that admits one pin, a
//! WebSocket upgrade that records what the client sent, and handshake v1 run
//! from the spec with the session's exporter as the key. The real remote
//! listener is serve's (the listener lane); serve's own tests pair against
//! `worktrees_remote::pair` end to end, and dial its remote listener with the
//! opaque signer below.
//!
//! [`Opaque`] stands in for the Secure Enclave: a software P-256 key that
//! exposes [`DeviceSigner`] and nothing else — no bytes, no `Debug` — and
//! counts what it is asked, so a test can see the key was ASKED to sign rather
//! than read.

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
use worktrees_remote::protocol::PROTO;
use worktrees_remote::sign::{self, DeviceSigner, Keys, SignError};

mod opaque;
use opaque::{How, Opaque};
use worktrees_remote::tls;

const WAIT: Duration = Duration::from_secs(5);
/// The fixture host's release: never this crate's, which a remote client
/// does not compare.
const VERSION: &str = "9.9.9";
const HOST: &str = "Studio";

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
}

/// A desktop's keys: a key file, as `HostStore::key_for` hands it over.
fn file_keys() -> (Keys, String) {
    let k = DeviceKey::generate().unwrap();
    let pin = k.pin().to_string();
    (Keys::device_only(Arc::new(k)), pin)
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
    /// A connection reached the listener at all.
    accepted: bool,
    host: Option<String>,
    origin: Option<String>,
    session_key: Option<String>,
    /// The session's exporter under `EXPORTER-worktrees-stepup-v1`.
    stepup_exporter: Option<[u8; 32]>,
    requests: Vec<Value>,
    asked: bool,
    hello: Value,
}

/// How the fixture misbehaves, when asked to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Host {
    Honest,
    /// Speaks protocol 2 and still serves 1.
    Newer,
    /// Speaks protocol 1 and serves clients from before the protocol too.
    ServesAll,
    /// A host from before the protocol: a hello with anything beside its
    /// nonce is refused, the socket dropped with nothing said.
    PreProtocol,
    /// Proves itself, and its ok names no protocol.
    NoProto,
    /// Speaks protocol 0, below every client's floor.
    BelowFloor,
    /// Serves protocol 2 and up only.
    NewMinimum,
    /// Refuses this client's protocol once its proof verifies, as serve does
    /// below `PROTO_MIN`: 1008 "Update the app".
    RefusesClient,
    /// Proves with another key, and claims a protocol no client accepts.
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
        // A device refused before its dial never comes: the fixture stops
        // waiting rather than hang the test.
        let Ok(Ok((tcp, _))) = tokio::time::timeout(Duration::from_secs(2), l.accept()).await else { return seen };
        seen.accepted = true;
        let tls = tokio_rustls::TlsAcceptor::from(cfg).accept(tcp).await;
        seen.asked = asked.load(Ordering::SeqCst);
        let Ok(tls) = tls else { return seen };
        let ex: [u8; 32] = tls.get_ref().1.export_keying_material([0u8; 32], b"EXPORTER-worktrees-remote-v1", None).unwrap();
        seen.stepup_exporter = Some(tls.get_ref().1.export_keying_material([0u8; 32], b"EXPORTER-worktrees-stepup-v1", None).unwrap());
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
        seen.hello = hello.clone();
        if how == Host::PreProtocol && hello.as_object().is_none_or(|o| o.len() != 1) {
            return seen;
        }
        let cn = hello["hello"].as_str().unwrap_or_default().to_string();
        let sn = "5".repeat(64);
        ws.send(Message::text(json!({"challenge": sn}).to_string())).await.unwrap();
        let Some(auth) = read(ws.next().await) else { return seen };
        if auth["auth"] != json!(mac("worktrees-serve/v1/client\n", &cn, &sn)) {
            return seen;
        }
        let proof = if how == Host::WrongProof { mac("worktrees-serve/v1/server\n", &cn, &"6".repeat(64)) } else { mac("worktrees-serve/v1/server\n", &cn, &sn) };
        let device = json!({"id": "d_x", "principal": "owner", "name": "Laptop", "caps": ["read"], "projects": "*", "expiry": {"idle_days": 30}, "stepup_required": false});
        if how == Host::RefusesClient {
            ws.send(Message::Close(Some(CloseFrame { code: CloseCode::Policy, reason: "Update the app".into() }))).await.unwrap();
            return seen;
        }
        let mut ok = json!({"ok": true, "version": VERSION, "proof": proof, "device": device});
        let range = match how {
            Host::NoProto => None,
            Host::BelowFloor | Host::WrongProof => Some((0, 0)),
            Host::Newer => Some((2, 1)),
            Host::ServesAll => Some((1, 0)),
            Host::NewMinimum => Some((2, 2)),
            _ => Some((1, 1)),
        };
        if let Some((proto, min)) = range {
            ok["proto"] = json!(proto);
            ok["proto_min"] = json!(min);
        }
        ws.send(Message::text(ok.to_string())).await.unwrap();
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
    let (dev, pin) = file_keys();
    let (addr, seen) = host(&rt, &id, &pin, Host::Honest);
    let (token, out) = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
        assert!(p.stepup().is_none(), "a desktop has no step-up key");
        let token = p.session_token().to_string();
        let mut s = call::handshake(p, HOST, WAIT).await.unwrap();
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
    let (dev, pin) = file_keys();
    let (addr, seen) = host(&rt, &id, &pin, Host::Honest);
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
    let ((_, pin), (stranger, _)) = (file_keys(), file_keys());
    let (addr, seen) = host(&rt, &id, &pin, Host::Honest);
    let r = rt.block_on(pipe::open_at(addr, &id.pin, &stranger, WAIT));
    assert!(matches!(r, Err(PipeError::Tls(_))), "{:?}", r.err().map(|e| e.to_string()));
    let seen = rt.block_on(seen).unwrap();
    assert!(seen.asked && seen.session_key.is_none() && seen.host.is_none(), "refused inside TLS: {seen:?}");
}

/// Proof before anything: a host that cannot prove the session key gets no
/// request, and its protocol claim is not believed either.
#[test]
fn nothing_is_sent_to_a_host_whose_proof_fails() {
    let rt = rt();
    let id = identity();
    let (dev, pin) = file_keys();
    let (addr, seen) = host(&rt, &id, &pin, Host::WrongProof);
    let r = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
        call::handshake(p, HOST, WAIT).await.map(|_| ())
    });
    assert!(matches!(r, Err(CallError::Handshake(_))), "{:?}", r.err().map(|e| e.to_string()));
    assert!(rt.block_on(seen).unwrap().requests.is_empty());
}

/// A revoke reaches the pipe with its reason (1008, #544's `end_device`).
#[test]
fn a_revoke_ends_the_pipe_with_its_reason() {
    let rt = rt();
    let id = identity();
    let (dev, pin) = file_keys();
    let (addr, _seen) = host(&rt, &id, &pin, Host::Revokes);
    let end = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
        let mut s = call::handshake(p, HOST, WAIT).await.unwrap();
        s.rx.recv().await
    });
    match end {
        Inbound::Closed(why) => assert!(why.contains("revoked"), "{why}"),
        other => panic!("{other:?}"),
    }
}

/// iOS Q10: a host on another release is driven — this crate's version is
/// not compared — and so is a newer one that still serves this protocol, and
/// one that serves clients from before the protocol as well.
/// The hello says which protocol this client speaks, and a row the host
/// lacks is the host's own refusal, passed on.
#[test]
fn a_proven_host_of_another_release_on_a_protocol_this_client_speaks_is_driven() {
    assert_ne!(VERSION, env!("CARGO_PKG_VERSION"));
    for how in [Host::Honest, Host::Newer, Host::ServesAll] {
        let rt = rt();
        let id = identity();
        let (dev, pin) = file_keys();
        let (addr, seen) = host(&rt, &id, &pin, how);
        let r = rt.block_on(async {
            let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
            let mut s = call::handshake(p, HOST, WAIT).await?;
            let out = s.request(1, "list_workspace", json!({}), WAIT).await;
            s.close().await;
            out
        });
        assert_eq!(r.map_err(|e| e.to_string()).unwrap(), json!({"echo": "list_workspace"}));
        let seen = rt.block_on(seen).unwrap();
        assert_eq!(seen.hello.as_object().map(|o| o.len()), Some(2), "{}", seen.hello);
        assert_eq!(seen.hello["proto"], json!(PROTO));
        assert_eq!(seen.requests.len(), 1);
    }
}

/// iOS Q10, the other half: a host from before the protocol, one that names
/// none, one below this client's floor, and one that no longer serves this
/// client are each sent no request, and the words say which side to update.
#[test]
fn a_host_on_an_incompatible_protocol_is_sent_nothing_and_says_which_side_to_update() {
    for (how, want) in [
        (Host::PreProtocol, "Update Worktrees on Studio"),
        (Host::NoProto, "Update Worktrees on Studio"),
        (Host::BelowFloor, "Update Worktrees on Studio"),
        (Host::NewMinimum, "Update the app"),
        (Host::RefusesClient, "Update the app"),
    ] {
        let rt = rt();
        let id = identity();
        let (dev, pin) = file_keys();
        let (addr, seen) = host(&rt, &id, &pin, how);
        let r = rt.block_on(async {
            let p = pipe::open_at(addr, &id.pin, &dev, WAIT).await.unwrap();
            call::handshake(p, HOST, WAIT).await.map(|_| ())
        });
        let err = r.err().map(|e| e.to_string());
        assert_eq!(err.as_deref(), Some(want));
        assert!(rt.block_on(seen).unwrap().requests.is_empty(), "{want}: nothing sent");
    }
}

// ── the device key as a signer: the Secure Enclave's stand-in ───────────────

/// The served case with a key this crate can only ask: the pipe opens, v1
/// completes, the request is answered — and the key was asked exactly twice
/// (its certificate, then the handshake), the only way the crate can use it.
#[test]
fn an_opaque_key_is_served_having_only_been_asked_to_sign() {
    let rt = rt();
    let id = identity();
    let key = Opaque::new();
    let (addr, seen) = host(&rt, &id, &key.pin(), Host::Honest);
    let keys = Keys::device_only(key.clone());
    let out = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &keys, WAIT).await.unwrap_or_else(|e| panic!("{e}"));
        let mut s = call::handshake(p, HOST, WAIT).await.unwrap();
        let out = s.request(1, "list_workspace", json!({}), WAIT).await.unwrap();
        s.close().await;
        out
    });
    assert_eq!(out, json!({"echo": "list_workspace"}));
    assert_eq!(key.asked(), 2, "the certificate's signature and the handshake's");
    assert!(rt.block_on(seen).unwrap().asked, "the host judged the key by its certificate");
}

/// A key that cannot sign is refused BEFORE anything is dialled: the
/// certificate is its first signature. The host never sees a connection.
#[test]
fn a_key_that_cannot_sign_never_reaches_the_host() {
    let rt = rt();
    let id = identity();
    for (what, how, want) in [
        ("cancelled", Box::new(|_: usize, _: Vec<u8>| Err(SignError::Cancelled)) as How, SignError::Cancelled),
        ("locked", Box::new(|_: usize, _: Vec<u8>| Err(SignError::Failed("the device is locked".into()))), SignError::Failed("the device is locked".into())),
        ("64 bytes, not DER", Box::new(|_: usize, sig: Vec<u8>| Ok(sig[sig.len() - 64..].to_vec())), SignError::Invalid),
    ] {
        let key = Opaque::answering(how);
        let (addr, seen) = host(&rt, &id, &key.pin(), Host::Honest);
        let r = rt.block_on(pipe::open_at(addr, &id.pin, &Keys::device_only(key.clone()), WAIT));
        match r {
            Err(PipeError::Sign(e)) => assert_eq!(e, want, "{what}"),
            other => panic!("{what}: {:?}", other.err().map(|e| e.to_string())),
        }
        assert_eq!(key.asked(), 1, "{what}: asked once, for the certificate");
        assert!(!rt.block_on(seen).unwrap().accepted, "{what}: nothing dialled");
    }
}

/// A key that signs its certificate and then refuses the handshake — Face ID
/// dismissed, or garbage — ends the handshake with the key's own reason, not
/// a TLS failure with none; nothing panics, the host serves nothing.
#[test]
fn a_key_that_fails_inside_the_handshake_is_named_as_the_reason() {
    let rt = rt();
    let id = identity();
    for (what, how, want) in [
        ("cancelled", Box::new(|n: usize, sig: Vec<u8>| if n == 0 { Ok(sig) } else { Err(SignError::Cancelled) }) as How, SignError::Cancelled),
        ("malformed", Box::new(|n: usize, sig: Vec<u8>| if n == 0 { Ok(sig) } else { Ok(vec![0x30, 0x03, 0x02, 0x01, 0x00]) }), SignError::Invalid),
        ("another message", Box::new(|n: usize, sig: Vec<u8>| if n == 0 { Ok(sig.clone()) } else { Ok(sig.iter().map(|b| b ^ 1).collect()) }), SignError::Invalid),
    ] {
        let key = Opaque::answering(how);
        let (addr, seen) = host(&rt, &id, &key.pin(), Host::Honest);
        let r = rt.block_on(pipe::open_at(addr, &id.pin, &Keys::device_only(key.clone()), WAIT));
        match &r {
            Err(PipeError::Sign(e)) => assert_eq!(*e, want, "{what}"),
            other => panic!("{what}: {:?}", other.as_ref().err().map(|e| e.to_string())),
        }
        assert!(r.err().unwrap().to_string().contains(&want.to_string()), "{what}: the reason is in the words");
        assert_eq!(key.asked(), 2, "{what}");
        let seen = rt.block_on(seen).unwrap();
        assert!(seen.accepted && seen.session_key.is_none() && seen.host.is_none(), "{what}: refused inside TLS: {seen:?}");
    }
}

/// A device with a step-up key gets one per connection: proofs signed by the
/// STEP-UP key (never the device key) over THIS session's exporter, which the
/// host computes on its side. A desktop's pipe has none.
#[test]
fn a_step_up_proof_is_the_step_up_keys_over_this_session() {
    let rt = rt();
    let id = identity();
    let (device, stepup) = (Opaque::new(), Opaque::new());
    let (addr, seen) = host(&rt, &id, &device.pin(), Host::Honest);
    let keys = Keys { device: device.clone(), stepup: Some(stepup.clone()) };
    let challenge = [5u8; 32];
    let proof = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &keys, WAIT).await.unwrap();
        let su = p.stepup().expect("a device with a step-up key has a step-up");
        let proof = su.prove(&challenge).await.unwrap();
        call::handshake(p, HOST, WAIT).await.unwrap().close().await;
        proof
    });
    let host_side = rt.block_on(seen).unwrap().stepup_exporter.unwrap();
    // §2.5's message, spelled out here rather than taken from `stepup::message`.
    let signed = [&challenge[..], &host_side[..]].concat();
    assert_eq!(proof.spki, stepup.spki_der(), "the step-up key's public half");
    assert!(sign::verifies(&stepup.spki_der(), &signed, &proof.signature), "over challenge ‖ the host's own exporter");
    assert!(!sign::verifies(&device.spki_der(), &signed, &proof.signature), "not the device key's");
    assert_eq!((stepup.asked(), device.asked()), (1, 2), "the step-up key signed once; the device key only for TLS");
}

/// A step-up cancelled on the device is said as a cancel, and the pipe it
/// belongs to is untouched.
#[test]
fn a_cancelled_step_up_is_a_cancel() {
    let rt = rt();
    let id = identity();
    let device = Opaque::new();
    let (addr, _seen) = host(&rt, &id, &device.pin(), Host::Honest);
    let keys = Keys { device: device.clone(), stepup: Some(Opaque::answering(Box::new(|_: usize, _: Vec<u8>| Err(SignError::Cancelled)))) };
    let r = rt.block_on(async {
        let p = pipe::open_at(addr, &id.pin, &keys, WAIT).await.unwrap();
        let r = p.stepup().unwrap().prove(&[1; 32]).await;
        let mut s = call::handshake(p, HOST, WAIT).await.unwrap();
        assert_eq!(s.request(1, "list_workspace", json!({}), WAIT).await.unwrap(), json!({"echo": "list_workspace"}), "still served");
        r
    });
    assert_eq!(r, Err(SignError::Cancelled));
}
