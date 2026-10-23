//! The rule that moves a kept address (Phase 2 §4.1, §4.7): a discovered or
//! pasted address is a CANDIDATE, dialled with the kept pin, and kept only
//! once the whole open succeeds. Against fixture hosts — TLS 1.3 with an
//! identity of their own, a client verifier that admits one key, and the
//! WebSocket upgrade — and a scripted browser. The kept and advertised
//! addresses are private LAN addresses, as a record must hold; `Env::route`
//! sends each to the loopback port its fixture listens on, and records every
//! dial, so "never dialled" is asserted on the dial itself.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rcgen::{CertificateParams, KeyPair, PublicKeyData, PKCS_ECDSA_P256_SHA256};
use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, Error, SignatureScheme};
use tokio::sync::mpsc;
use worktrees_core::netif::{IfAddr, Iface};
use worktrees_core::pairuri;
use worktrees_remote::discover::{Browser, Sighting};
use worktrees_remote::hosts::{new_host_id, HostRecord, HostStore};
use worktrees_remote::key::DeviceKey;
use worktrees_remote::pipe::PipeError;
use worktrees_remote::reach::{self, Env, Lan, ReachError};
use worktrees_remote::sign::Keys;
use worktrees_remote::tls;

const WAIT: Duration = Duration::from_secs(5);
const KEPT: &str = "192.168.1.5:7461";

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
}

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

/// Admits one key; counts every certificate a client showed it.
#[derive(Debug)]
struct OnePin {
    pin: String,
    shown: Arc<AtomicUsize>,
}

impl ClientCertVerifier for OnePin {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(&self, c: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, Error> {
        self.shown.fetch_add(1, Ordering::SeqCst);
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

/// A fixture host on loopback: what it saw, connection by connection.
struct Fixture {
    at: SocketAddr,
    /// Certificates clients showed it.
    shown: Arc<AtomicUsize>,
    /// The SNI of every ClientHello (None: none sent).
    sni: Arc<Mutex<Vec<Option<String>>>>,
    upgraded: Arc<AtomicUsize>,
}

fn host(rt: &tokio::runtime::Runtime, id: &Identity, admit: &str) -> Fixture {
    let shown = Arc::new(AtomicUsize::new(0));
    let mut cfg = rustls::ServerConfig::builder_with_provider(tls::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(Arc::new(OnePin { pin: admit.into(), shown: shown.clone() }))
        .with_single_cert(vec![id.cert.clone()], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key.clone())))
        .unwrap();
    cfg.send_tls13_tickets = 0;
    let cfg = Arc::new(cfg);
    let l = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let at = l.local_addr().unwrap();
    let (sni, upgraded) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicUsize::new(0)));
    let (sni2, up2) = (sni.clone(), upgraded.clone());
    rt.spawn(async move {
        while let Ok((tcp, _)) = l.accept().await {
            let (cfg, sni, up) = (cfg.clone(), sni2.clone(), up2.clone());
            tokio::spawn(async move {
                let Ok(start) = tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), tcp).await else { return };
                sni.lock().unwrap().push(start.client_hello().server_name().map(str::to_string));
                let Ok(tls) = start.into_stream(cfg).await else { return };
                let Ok(mut ws) = tokio_tungstenite::accept_async(tls).await else { return };
                up.fetch_add(1, Ordering::SeqCst);
                use futures_util::StreamExt;
                while let Some(Ok(_)) = ws.next().await {}
            });
        }
    });
    Fixture { at, shown, sni, upgraded }
}

/// A browser that hands over what it was given; counts its rounds. With
/// `hold`, the round stays open that long after the last sighting.
struct Script {
    sightings: Vec<Sighting>,
    rounds: Arc<AtomicUsize>,
    hold: Option<Duration>,
}

impl Browser for Script {
    fn browse(&self, _round: Duration) -> Result<mpsc::Receiver<Sighting>, String> {
        self.rounds.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(64);
        for s in &self.sightings {
            tx.try_send(s.clone()).unwrap();
        }
        if let Some(h) = self.hold {
            std::thread::spawn(move || {
                std::thread::sleep(h);
                drop(tx);
            });
        }
        Ok(rx)
    }
}

/// One device, one kept host, a fake `en0` on 192.168.1.0/24, and the dials.
struct World {
    dir: PathBuf,
    store: HostStore,
    rec: HostRecord,
    key: Arc<DeviceKey>,
    keys: Keys,
    routes: Arc<Mutex<HashMap<SocketAddr, SocketAddr>>>,
    dials: Arc<Mutex<Vec<SocketAddr>>>,
    rounds: Arc<AtomicUsize>,
    closed: SocketAddr,
}

fn en0() -> Iface {
    let a = |ip: &str, prefix| IfAddr { ip: ip.parse().unwrap(), prefix, temporary: false, deprecated: false };
    Iface { name: "en0".into(), index: 4, up: true, running: true, loopback: false, point_to_point: false, addrs: vec![a("192.168.1.20", 24), a("fd00:1::20", 64)] }
}

fn world(rt: &tokio::runtime::Runtime, tag: &str, pin: &str) -> World {
    let dir = std::env::temp_dir().join(format!("wt-remote-reach-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = HostStore::new(&dir);
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).unwrap();
    let rec = HostRecord {
        id: new_host_id(),
        name: "Mac mini".into(),
        addr: KEPT.into(),
        pin: pin.into(),
        discovery_id: Some(pairuri::base32(&b)),
        device_id: "d_abc".into(),
        principal_id: "owner".into(),
        expiry: serde_json::json!({"idle_days": 30}),
        paired_at: 1_800_000_000,
    };
    let key = Arc::new(DeviceKey::generate().unwrap());
    store.add(&rec, &key).unwrap();
    let keys = Keys::device_only(key.clone());
    // A port nothing listens on: a connect there is refused at once.
    let closed = rt.block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap() });
    World { dir, store, rec, key, keys, routes: Arc::default(), dials: Arc::default(), rounds: Arc::default(), closed }
}

impl World {
    fn route(&self, from: &str, to: SocketAddr) {
        self.routes.lock().unwrap().insert(from.parse().unwrap(), to);
    }

    fn env(&self, sightings: Vec<Sighting>, ifaces: Vec<Iface>) -> Env {
        let (routes, dials, closed) = (self.routes.clone(), self.dials.clone(), self.closed);
        Env {
            browser: Arc::new(Script { sightings, rounds: self.rounds.clone(), hold: None }),
            interfaces: Arc::new(move || Ok(ifaces.clone())),
            route: Arc::new(move |a| {
                dials.lock().unwrap().push(a);
                routes.lock().unwrap().get(&a).copied().unwrap_or(closed)
            }),
            round: Duration::from_secs(5),
            candidate_wait: Duration::from_secs(2),
        }
    }

    fn sighting(&self, at: &str) -> Sighting {
        let a: SocketAddr = at.parse().unwrap();
        let id = format!("id={}", self.rec.discovery_id.as_deref().unwrap());
        let mut txt = vec![3];
        txt.extend_from_slice(b"v=1");
        txt.push(id.len() as u8);
        txt.extend_from_slice(id.as_bytes());
        Sighting { txt, ip: a.ip(), port: a.port(), iface: 4 }
    }

    fn file(&self) -> Vec<u8> {
        std::fs::read(self.dir.join("remote-hosts.json")).unwrap()
    }

    fn dialled(&self) -> Vec<String> {
        self.dials.lock().unwrap().iter().map(|a| a.to_string()).collect()
    }

    fn open(&self, rt: &tokio::runtime::Runtime, env: &Env) -> (Result<reach::Reached, ReachError>, Vec<String>) {
        let mut log = Vec::new();
        let r = rt.block_on(reach::open(&self.store, &self.rec, &self.keys, env, WAIT, &mut |m: &str| log.push(m.to_string())));
        (r, log)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The served case: nothing answers at the kept address, the host's advert
/// names its new one, the pinned open there succeeds, and the record follows
/// — kept, 0600, the next connect's address.
#[test]
fn a_moved_host_is_found_on_the_lan_and_its_address_kept() {
    let rt = rt();
    let id = identity();
    let w = world(&rt, "found", &id.pin);
    let real = host(&rt, &id, w.key.pin());
    w.route("192.168.1.9:7461", real.at);
    let (r, log) = w.open(&rt, &w.env(vec![w.sighting("192.168.1.9:7461")], vec![en0()]));
    let r = r.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r.rec.addr, "192.168.1.9:7461");
    assert_eq!(r.moved_from.as_deref(), Some(KEPT));
    assert_eq!(w.store.get(&w.rec.id).unwrap().addr, "192.168.1.9:7461", "kept in the store");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(w.dir.join("remote-hosts.json")).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(w.dialled(), [KEPT, "192.168.1.9:7461"]);
    assert_eq!(real.upgraded.load(Ordering::SeqCst), 1);
    assert!(log.iter().any(|l| l.contains("found on the LAN at 192.168.1.9:7461")), "{log:?}");
}

/// §10's third exit criterion, its second half: an advert carrying the
/// host's own discovery id and someone else's key is tried, refused at the
/// pin before the device's certificate goes out (with no SNI naming the
/// host), and changes nothing — the hosts file is byte for byte as it was.
/// A real advert later in the same round is still found.
#[test]
fn a_spoofed_advert_with_the_same_id_is_refused_at_the_pin_and_changes_nothing() {
    let rt = rt();
    let (id, spoofer) = (identity(), identity());
    let w = world(&rt, "spoof", &id.pin);
    // The spoofer would admit this device's key, were it ever shown.
    let fake = host(&rt, &spoofer, w.key.pin());
    w.route("192.168.1.66:7461", fake.at);
    let before = w.file();
    let (r, log) = w.open(&rt, &w.env(vec![w.sighting("192.168.1.66:7461")], vec![en0()]));
    match r {
        Err(ReachError::NotFound { lan: Lan::Looked(ref tried), .. }) => {
            assert!(matches!(tried.as_slice(), [(a, PipeError::NotThePin)] if a.to_string() == "192.168.1.66:7461"), "{tried:?}");
        }
        Err(e) => panic!("{e}"),
        Ok(r) => panic!("adopted {}", r.rec.addr),
    }
    assert_eq!(w.file(), before, "the record is unchanged");
    assert_eq!(fake.shown.load(Ordering::SeqCst), 0, "the spoofer never saw the device's certificate");
    assert_eq!(fake.upgraded.load(Ordering::SeqCst), 0);
    assert_eq!(*fake.sni.lock().unwrap(), [None], "one ClientHello, and no SNI in it");
    assert!(log.iter().any(|l| l.contains("192.168.1.66:7461") && l.contains("does not match the pin")), "logged: {log:?}");

    let real = host(&rt, &id, w.key.pin());
    w.route("192.168.1.9:7461", real.at);
    let (r, _) = w.open(&rt, &w.env(vec![w.sighting("192.168.1.66:7461"), w.sighting("192.168.1.9:7461")], vec![en0()]));
    assert_eq!(r.unwrap_or_else(|e| panic!("{e}")).rec.addr, "192.168.1.9:7461", "the real advert, after the spoofed one");
}

/// A candidate that holds the pin but refuses THIS device's key (revoked
/// there, or never enrolled) has not opened: nothing is kept.
#[test]
fn a_candidate_that_refuses_this_devices_key_is_not_kept() {
    let rt = rt();
    let id = identity();
    let w = world(&rt, "refuses", &id.pin);
    let other = DeviceKey::generate().unwrap();
    let real = host(&rt, &id, other.pin());
    w.route("192.168.1.9:7461", real.at);
    let before = w.file();
    let (r, _) = w.open(&rt, &w.env(vec![w.sighting("192.168.1.9:7461")], vec![en0()]));
    match r {
        Err(ReachError::NotFound { lan: Lan::Looked(ref tried), .. }) => assert!(matches!(tried.as_slice(), [(_, PipeError::Tls(_))]), "{tried:?}"),
        Err(e) => panic!("{e}"),
        Ok(r) => panic!("adopted {}", r.rec.addr),
    }
    assert_eq!(w.file(), before);
    assert_eq!(real.upgraded.load(Ordering::SeqCst), 0);
}

/// The kept address answered by a machine that is NOT the pin — the old IP
/// handed to another machine, maybe another worktrees host — means the host
/// is not there: the device looks on the LAN, never shows the stranger its
/// certificate, and keeps the real host's address once its pinned open
/// succeeds.
#[test]
fn a_stranger_at_the_kept_address_sends_the_device_looking_and_the_host_is_kept() {
    let rt = rt();
    let (id, stranger) = (identity(), identity());
    let w = world(&rt, "kept-stranger", &id.pin);
    // The stranger would admit this device's key, were it ever shown.
    let not_it = host(&rt, &stranger, w.key.pin());
    let real = host(&rt, &id, w.key.pin());
    w.route(KEPT, not_it.at);
    w.route("192.168.1.9:7461", real.at);
    let (r, log) = w.open(&rt, &w.env(vec![w.sighting("192.168.1.9:7461")], vec![en0()]));
    let r = r.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((r.rec.addr.as_str(), r.moved_from.as_deref()), ("192.168.1.9:7461", Some(KEPT)));
    assert_eq!(w.store.get(&w.rec.id).unwrap().addr, "192.168.1.9:7461", "kept");
    assert_eq!(w.rounds.load(Ordering::SeqCst), 1, "one round on the LAN");
    assert_eq!(w.dialled(), [KEPT, "192.168.1.9:7461"]);
    assert_eq!(not_it.shown.load(Ordering::SeqCst), 0, "the stranger never saw the device's certificate");
    assert!(log.iter().any(|l| l.contains("another key") && l.contains("looking for it on the LAN")), "{log:?}");
}

/// The kept address answered by the PINNED host, which refuses this
/// device's key (revoked, disabled, expired there), is not a move: the device
/// is no longer welcome, and finding the host elsewhere would change nothing.
/// No one goes looking, and nothing is kept.
#[test]
fn a_host_that_refuses_this_devices_key_at_the_kept_address_sends_no_one_looking() {
    let rt = rt();
    let id = identity();
    let w = world(&rt, "kept-refuses", &id.pin);
    let other = DeviceKey::generate().unwrap();
    let refuses = host(&rt, &id, other.pin());
    let real = host(&rt, &id, w.key.pin());
    w.route(KEPT, refuses.at);
    w.route("192.168.1.9:7461", real.at);
    let (r, _) = w.open(&rt, &w.env(vec![w.sighting("192.168.1.9:7461")], vec![en0()]));
    assert!(matches!(r, Err(ReachError::Pipe(PipeError::Tls(_)))), "{:?}", r.as_ref().err().map(|e| e.to_string()));
    assert_eq!(w.rounds.load(Ordering::SeqCst), 0, "no browse");
    assert_eq!(w.dialled(), [KEPT], "the advert was never dialled");
    assert_eq!(real.sni.lock().unwrap().len(), 0);
    assert_eq!(w.store.get(&w.rec.id).unwrap().addr, KEPT);
}

/// The classifier runs on every candidate before any dial: an advert naming
/// a public, loopback, wildcard or mapped address is never dialled — even on
/// an interface that claims to hold that network, so the link check cannot
/// be what refuses it.
#[test]
fn a_public_or_loopback_advert_is_never_dialled() {
    let rt = rt();
    let id = identity();
    let w = world(&rt, "netclass", &id.pin);
    let real = host(&rt, &id, w.key.pin());
    let mut odd = en0();
    let a = |ip: &str, prefix| IfAddr { ip: ip.parse().unwrap(), prefix, temporary: false, deprecated: false };
    odd.addrs.extend([a("8.8.8.1", 24), a("127.0.0.2", 8), a("0.0.0.1", 8), a("2001:db8::1", 64)]);
    let bad = ["8.8.8.8:7461", "127.0.0.1:7461", "0.0.0.0:7461", "[::1]:7461", "[::ffff:192.168.1.9]:7461", "[2001:db8::9]:7461"];
    for b in bad {
        // Each would reach the real host, were it dialled.
        w.route(b, real.at);
    }
    let (r, log) = w.open(&rt, &w.env(bad.iter().map(|b| w.sighting(b)).collect(), vec![odd]));
    assert!(matches!(r, Err(ReachError::NotFound { .. })), "{:?}", r.as_ref().err().map(|e| e.to_string()));
    assert_eq!(w.dialled(), [KEPT], "only the kept address was dialled");
    assert_eq!(real.sni.lock().unwrap().len(), 0, "nothing reached the host");
    assert!(log.iter().filter(|l| l.contains("not-private")).count() == bad.len(), "{log:?}");
}

/// Off-link, from an interface this device does not dial through, or not
/// this host's advert (another id, or this id in another TXT shape): never
/// dialled, though each would have reached the real host.
#[test]
fn an_off_link_foreign_or_misshapen_advert_is_never_dialled() {
    let rt = rt();
    let id = identity();
    let w = world(&rt, "offlink", &id.pin);
    let real = host(&rt, &id, w.key.pin());
    let mut sightings = Vec::new();
    for at in ["10.0.0.9:7461", "192.168.2.9:7461", "[fd00:2::9]:7461"] {
        w.route(at, real.at);
        sightings.push(w.sighting(at));
    }
    for (n, iface) in [(10, 9), (11, u32::MAX)] {
        let at = format!("192.168.1.{n}:7461");
        w.route(&at, real.at);
        sightings.push(Sighting { iface, ..w.sighting(&at) });
    }
    let other_id = pairuri::base32(&[9; 16]);
    for (n, txt) in [(12, format!("v=1|id={other_id}")), (13, format!("v=1|id={}|pin=ab", w.rec.discovery_id.as_deref().unwrap()))] {
        let at = format!("192.168.1.{n}:7461");
        w.route(&at, real.at);
        let mut raw = Vec::new();
        for part in txt.split('|') {
            raw.push(part.len() as u8);
            raw.extend_from_slice(part.as_bytes());
        }
        sightings.push(Sighting { txt: raw, ..w.sighting(&at) });
    }
    let (r, _) = w.open(&rt, &w.env(sightings, vec![en0()]));
    assert!(matches!(r, Err(ReachError::NotFound { .. })));
    assert_eq!(w.dialled(), [KEPT]);
    assert_eq!(real.sni.lock().unwrap().len(), 0);
}

/// At most four distinct candidates a round, each once: a fifth and a sixth
/// are not dialled, a repeat of a tried one is not dialled again, and the
/// kept address (already failed this round) is not a candidate.
#[test]
fn a_round_tries_at_most_four_candidates_once_each() {
    let rt = rt();
    let id = identity();
    let w = world(&rt, "four", &id.pin);
    let at = |n: u8| format!("192.168.1.{n}:7461");
    let mut s: Vec<Sighting> = vec![w.sighting(KEPT)];
    for n in 30..36 {
        s.push(w.sighting(&at(n)));
        s.push(w.sighting(&at(30)));
    }
    let (r, _) = w.open(&rt, &w.env(s, vec![en0()]));
    match r {
        Err(ReachError::NotFound { lan: Lan::Looked(tried), .. }) => assert_eq!(tried.len(), 4),
        other => panic!("{:?}", other.err().map(|e| e.to_string())),
    }
    assert_eq!(w.dialled(), [KEPT.to_string(), at(30), at(31), at(32), at(33)]);
}

/// A round ends on time even while the browser has more to say.
#[test]
fn a_round_is_bounded_in_time() {
    let rt = rt();
    let id = identity();
    let w = world(&rt, "bounded", &id.pin);
    let mut env = w.env(vec![], vec![en0()]);
    env.browser = Arc::new(Script { sightings: vec![], rounds: w.rounds.clone(), hold: Some(Duration::from_secs(30)) });
    env.round = Duration::from_millis(400);
    let t = Instant::now();
    let (r, _) = w.open(&rt, &env);
    assert!(matches!(r, Err(ReachError::NotFound { lan: Lan::Looked(_), .. })));
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
}

/// What the person reads when the host is nowhere: where it was looked for,
/// and the way back — the address code on the host.
#[test]
fn not_found_says_where_it_looked_and_the_way_back() {
    let rt = rt();
    let id = identity();
    let w = world(&rt, "words", &id.pin);
    let (r, _) = w.open(&rt, &w.env(vec![], vec![en0()]));
    let e = r.err().unwrap().to_string();
    assert!(
        e.starts_with("Not found at 192.168.1.5:7461, and not on the LAN. On Mac mini, `worktrees serve remote status` shows its address code."),
        "{e}"
    );
    let mut env = w.env(vec![], vec![en0()]);
    env.browser = Arc::new(worktrees_remote::discover::Unavailable("no browsing here"));
    let e = w.open(&rt, &env).0.err().unwrap().to_string();
    assert!(e.contains("not looked for on the LAN: no browsing here") && e.contains("address code"), "{e}");
    let mut st = w.rec.clone();
    st.discovery_id = None;
    let e = rt.block_on(reach::open(&w.store, &st, &w.keys, &w.env(vec![], vec![en0()]), WAIT, &mut |_: &str| {})).err().unwrap().to_string();
    assert!(e.contains("no discovery id") && e.contains("address code"), "{e}");
}

/// §4.1's address code: a code for another pin is refused before anything
/// is dialled; one whose address answers with another key changes nothing;
/// one whose address opens with the pin is kept.
#[test]
fn an_address_code_is_kept_only_after_its_pinned_open() {
    let rt = rt();
    let (id, spoofer) = (identity(), identity());
    let w = world(&rt, "code", &id.pin);
    let fake = host(&rt, &spoofer, w.key.pin());
    let real = host(&rt, &id, w.key.pin());
    w.route("192.168.1.66:7461", fake.at);
    w.route("192.168.1.9:7461", real.at);
    let env = w.env(vec![], vec![en0()]);
    let code = |addr: &str, pin: &str| format!("worktrees-addr:v1?addr={addr}&pin={pin}");
    let before = w.file();
    let adopt = |c: &str| rt.block_on(reach::adopt_code(&w.store, &w.rec, &reach::parse_code(c).unwrap(), &env, WAIT));
    let e = adopt(&code("192.168.1.9:7461", &spoofer.pin)).unwrap_err();
    assert!(e.contains("another host") && e.contains("Nothing was dialled"), "{e}");
    assert!(w.dialled().is_empty());
    for bad in [code("8.8.8.8:7461", &id.pin), code("[fe80::1%4]:7461", &id.pin), "worktrees-pair:v1?addr=192.168.1.9:7461".into()] {
        assert!(reach::parse_code(&bad).is_err(), "{bad}");
    }
    let e = adopt(&code("192.168.1.66:7461", &id.pin)).unwrap_err();
    assert!(e.contains("does not match the pin") && e.contains("unchanged (192.168.1.5:7461)"), "{e}");
    assert_eq!(w.file(), before);
    assert_eq!(fake.shown.load(Ordering::SeqCst), 0);
    let moved = adopt(&format!("  {}\n", code("192.168.1.9:7461", &id.pin))).unwrap();
    assert_eq!(moved.addr, "192.168.1.9:7461");
    assert_eq!(w.store.get(&w.rec.id).unwrap().addr, "192.168.1.9:7461");
}
