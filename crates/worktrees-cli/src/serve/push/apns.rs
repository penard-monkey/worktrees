//! The sender's one outbound connection: HTTP/2 to Apple, and nowhere else.
//!
//! - **Exactly two hosts**, named below, chosen by the token's environment.
//!   Nothing else in serve can reach this dial; no argument, file or variable
//!   names a host (a test reads this module for it).
//! - **TLS 1.3, verified**, against the two roots Apple names for APNs,
//!   embedded here. Not the OS store: reading it would let `SSL_CERT_FILE`
//!   change trust. (main) measured both hosts on 2026-11-04: TLS 1.3, ALPN
//!   `h2`, a chain ending at USERTrust RSA. If Apple moves to another CA this
//!   fails until a release does, and says `trust`.
//! - **No redirect, no proxy**, because no HTTP client is above `h2`: a request
//!   is one URI and a 3xx is a status. `TcpStream::connect` reads no variable.
//!   `/etc/hosts` can still send the TCP connection elsewhere, but SNI and the
//!   embedded roots pin the name, so that ends in a `trust` failure, not a push.
//! - **Bounded**: [`CONNECT`] and [`SEND`]. A slow or dead Apple costs one
//!   task per device, whatever it does, and the poll never meets any of it.
//! - The token is in the request's path, so no error here carries a request.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use h2::client::SendRequest;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::key::ApnsKey;
use super::plan::Delivery;
use super::store::PushEnv;

pub const PRODUCTION: &str = "api.push.apple.com";
pub const SANDBOX: &str = "api.sandbox.push.apple.com";

const CONNECT: Duration = Duration::from_secs(10);
const SEND: Duration = Duration::from_secs(15);
/// An error body is a short JSON object. Past this, it is not read.
const MAX_BODY: usize = 4 * 1024;

/// Apple refuses a provider token older than an hour, and more than one new
/// token per 20 minutes. Replaced at 45 minutes: never over 60, never under 20.
const JWT_FRESH: i64 = 45 * 60;
/// A token Apple rejected as expired is replaced early, but never inside this.
const JWT_MIN: i64 = 20 * 60;

const ROOTS: &[&[u8]] = &[include_bytes!("roots/usertrust-rsa.der"), include_bytes!("roots/aaa-certificate-services.der")];

/// One push, ready to go. Deliberately no `Debug`: it holds the token.
pub struct Request {
    pub env: PushEnv,
    pub token: String,
    pub topic: String,
    pub collapse: Option<String>,
    pub expiration: i64,
    pub priority: u8,
    pub body: Vec<u8>,
    /// For the dry run's line: the kind's word and the key id. No secret.
    pub kind: &'static str,
    pub key_id: String,
    pub now: i64,
}

pub type Sending = Pin<Box<dyn Future<Output = Delivery> + Send>>;

/// Where a push goes: Apple, the dry run, or a test.
pub trait Transport: Send + Sync {
    fn send(&self, r: Request) -> Sending;
}

/// The console's dry run: everything up to Apple, then a line in the log. The
/// line holds the kind, the sizes, the key id and that it sealed; `body` is
/// looked at for whether it is sealed and never written.
pub struct DryRun {
    pub line: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Transport for DryRun {
    fn send(&self, r: Request) -> Sending {
        let sealed = serde_json::from_slice::<serde_json::Value>(&r.body).ok().is_some_and(|v| v.get("wt").is_some());
        (self.line)(&format!("serve: push: dry run: kind={} bytes={} key_id={} sealed={sealed} priority={}", r.kind, r.body.len(), r.key_id, r.priority));
        Box::pin(async { Delivery::Sent })
    }
}

/// A provider token, cached.
#[derive(Default)]
struct Jwt {
    made: Option<(i64, String)>,
    stale: bool,
}

/// `header.claims.signature`, base64url with no padding.
fn make_jwt(key: &ApnsKey, now: i64) -> Result<String, String> {
    let header = serde_json::json!({ "alg": "ES256", "kid": key.key_id }).to_string();
    let claims = serde_json::json!({ "iss": key.team_id, "iat": now }).to_string();
    let msg = format!("{}.{}", b64url(header.as_bytes()), b64url(claims.as_bytes()));
    let sig = key.sign(msg.as_bytes())?;
    Ok(format!("{msg}.{}", b64url(&sig)))
}

fn b64url(b: &[u8]) -> String {
    worktrees_api::files::b64_encode(b).replace('+', "-").replace('/', "_").trim_end_matches('=').to_string()
}

impl Jwt {
    fn token(&mut self, key: &ApnsKey, now: i64) -> Result<String, String> {
        if let Some((at, t)) = &self.made {
            let age = now - at;
            if age < JWT_FRESH && !(self.stale && age >= JWT_MIN) {
                return Ok(t.clone());
            }
        }
        let t = make_jwt(key, now)?;
        self.made = Some((now, t.clone()));
        self.stale = false;
        Ok(t)
    }
}

/// How to reach a host: the real one, unless a test says otherwise.
#[derive(Clone)]
pub struct Dial {
    tls: Arc<ClientConfig>,
    /// A fixture's address. Only a test can build a `Dial` with one.
    at: Option<SocketAddr>,
}

fn real_tls() -> Result<Arc<ClientConfig>, String> {
    let mut roots = RootCertStore::empty();
    for r in ROOTS {
        roots.add(CertificateDer::from(*r)).map_err(|e| format!("an embedded root: {e}"))?;
    }
    tls_for(roots)
}

fn tls_for(roots: RootCertStore) -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut c = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    c.alpn_protocols = vec![b"h2".to_vec()];
    Ok(Arc::new(c))
}

impl Dial {
    pub fn apple() -> Result<Dial, String> {
        Ok(Dial { tls: real_tls()?, at: None })
    }

    /// A fixture: a local server, trusted through `roots`, or through Apple's
    /// when `roots` is `None`, to show those refuse it.
    #[cfg(test)]
    pub fn fixture(at: SocketAddr, roots: Option<RootCertStore>) -> Dial {
        let tls = match roots {
            Some(r) => tls_for(r).unwrap(),
            None => real_tls().unwrap(),
        };
        Dial { tls, at: Some(at) }
    }
}

pub fn host(env: PushEnv) -> &'static str {
    match env {
        PushEnv::Development => SANDBOX,
        PushEnv::Production => PRODUCTION,
    }
}

/// HTTP/2 to APNs.
pub struct H2 {
    key: Arc<ApnsKey>,
    dial: Dial,
    connect_for: Duration,
    send_for: Duration,
    jwt: Mutex<Jwt>,
    conns: Arc<tokio::sync::Mutex<HashMap<PushEnv, SendRequest<Bytes>>>>,
}

impl H2 {
    pub fn new(key: Arc<ApnsKey>, dial: Dial) -> Arc<H2> {
        Arc::new(H2 { key, dial, connect_for: CONNECT, send_for: SEND, jwt: Mutex::default(), conns: Default::default() })
    }

    /// The bounds as given, so a test need not wait out the real ones.
    #[cfg(test)]
    pub fn bounded(key: Arc<ApnsKey>, dial: Dial, send_for: Duration) -> Arc<H2> {
        Arc::new(H2 { key, dial, connect_for: CONNECT, send_for, jwt: Mutex::default(), conns: Default::default() })
    }

    async fn connect(dial: &Dial, env: PushEnv, bound: Duration) -> Result<SendRequest<Bytes>, &'static str> {
        let name = host(env);
        let go = async {
            let tcp = match dial.at {
                Some(a) => TcpStream::connect(a).await,
                None => TcpStream::connect((name, 443)).await,
            }
            .map_err(|_| "connect")?;
            let _ = tcp.set_nodelay(true);
            let sni = ServerName::try_from(name).map_err(|_| "connect")?;
            let tls = TlsConnector::from(dial.tls.clone()).connect(sni, tcp).await.map_err(|e| if is_trust(&e) { "trust" } else { "tls" })?;
            if tls.get_ref().1.alpn_protocol() != Some(b"h2") {
                return Err("no-h2");
            }
            let (send, conn) = h2::client::Builder::new().handshake::<_, Bytes>(tls).await.map_err(|_| "handshake")?;
            // The connection's own task: it ends with the connection.
            tokio::spawn(async move {
                let _ = conn.await;
            });
            Ok(send)
        };
        tokio::time::timeout(bound, go).await.unwrap_or(Err("connect-timeout"))
    }

    async fn deliver(self: Arc<Self>, r: Request) -> Delivery {
        let jwt = match self.jwt.lock().unwrap().token(&self.key, r.now) {
            Ok(j) => j,
            Err(_) => return Delivery::Auth("could-not-sign"),
        };
        let env = r.env;
        let dial = self.dial.clone();
        let conns = self.conns.clone();
        // A held connection that has since died gets one fresh try.
        for attempt in 0..2 {
            let cached = conns.lock().await.get(&env).cloned();
            let fresh = cached.is_none();
            let send = match cached {
                Some(s) => s,
                None => match Self::connect(&dial, env, self.connect_for).await {
                    Ok(s) => {
                        conns.lock().await.insert(env, s.clone());
                        s
                    }
                    Err(why) => return Delivery::Retry(why),
                },
            };
            match self.request(send, &r, &jwt).await {
                Ok(d) => return self.after(d),
                Err(why) => {
                    conns.lock().await.remove(&env);
                    if fresh || attempt == 1 {
                        return Delivery::Retry(why);
                    }
                }
            }
        }
        Delivery::Retry("connect")
    }

    /// An expired token asks for a new one on the next send.
    fn after(&self, d: Delivery) -> Delivery {
        if matches!(d, Delivery::Auth("expired-provider-token")) {
            self.jwt.lock().unwrap().stale = true;
        }
        d
    }

    async fn request(&self, send: SendRequest<Bytes>, r: &Request, jwt: &str) -> Result<Delivery, &'static str> {
        let mut b = http::Request::builder()
            .method(http::Method::POST)
            .uri(format!("https://{}/3/device/{}", host(r.env), r.token))
            .header("authorization", format!("bearer {jwt}"))
            .header("apns-push-type", "alert")
            .header("apns-topic", r.topic.as_str())
            .header("apns-priority", r.priority.to_string())
            .header("apns-expiration", r.expiration.to_string());
        if let Some(c) = &r.collapse {
            b = b.header("apns-collapse-id", c.as_str());
        }
        let req = b.body(()).map_err(|_| "request")?;
        let body = Bytes::from(r.body.clone());
        let go = async move {
            let mut send = send.ready().await.map_err(|_| "connection")?;
            let (resp, mut stream) = send.send_request(req, false).map_err(|_| "connection")?;
            stream.send_data(body, true).map_err(|_| "connection")?;
            let resp = resp.await.map_err(|_| "connection")?;
            let status = resp.status().as_u16();
            let mut recv = resp.into_body();
            let mut text = Vec::new();
            while let Some(chunk) = recv.data().await {
                let Ok(chunk) = chunk else { break };
                let _ = recv.flow_control().release_capacity(chunk.len());
                text.extend_from_slice(&chunk);
                if text.len() > MAX_BODY {
                    break;
                }
            }
            Ok(classify(status, &text))
        };
        tokio::time::timeout(self.send_for, go).await.unwrap_or(Err("send-timeout"))
    }
}

impl Transport for Arc<H2> {
    fn send(&self, r: Request) -> Sending {
        Box::pin(self.clone().deliver(r))
    }
}

/// A TLS failure that is about whom we are talking to.
fn is_trust(e: &std::io::Error) -> bool {
    e.get_ref().and_then(|x| x.downcast_ref::<rustls::Error>()).is_some_and(|x| matches!(x, rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented))
}

/// APNs's answer, as a closed list of our own words: Apple's bytes reach
/// neither a log nor a record.
pub fn classify(status: u16, body: &[u8]) -> Delivery {
    let reason = serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_string)).unwrap_or_default();
    match (status, reason.as_str()) {
        (200, _) => Delivery::Sent,
        (410, _) => Delivery::Dead("unregistered"),
        (400, "BadDeviceToken") => Delivery::Dead("bad-device-token"),
        (400, "DeviceTokenNotForTopic") => Delivery::Dead("token-not-for-topic"),
        (400, "BadCollapseId") => Delivery::Rejected("bad-collapse-id"),
        (400, "BadTopic" | "MissingTopic" | "TopicDisallowed") => Delivery::Rejected("bad-topic"),
        (400, "BadExpirationDate" | "BadPriority" | "InvalidPushType") => Delivery::Rejected("bad-header"),
        (400, "PayloadEmpty") | (413, _) => Delivery::Rejected("bad-payload"),
        (403, "ExpiredProviderToken") => Delivery::Auth("expired-provider-token"),
        (403, "InvalidProviderToken" | "MissingProviderToken") => Delivery::Auth("invalid-provider-token"),
        (429, _) => Delivery::Retry("status-429"),
        (500..=599, _) => Delivery::Retry("status-5xx"),
        (403, _) => Delivery::Auth("forbidden"),
        _ => Delivery::Rejected("other"),
    }
}

#[cfg(test)]
pub mod fixture {
    //! A local HTTP/2 server for APNs's half, over real TLS. Tests never reach
    //! Apple: every `Dial` here names this server's address.
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    #[derive(Clone, Debug)]
    pub struct Seen {
        pub authority: String,
        pub path: String,
        pub headers: HashMap<String, String>,
        pub body: Vec<u8>,
    }

    /// What the server answers, in order; the last repeats.
    #[derive(Clone)]
    pub enum Answer {
        Status(u16, &'static str),
        /// Accept the stream and never answer.
        Hang,
    }

    pub struct Server {
        pub at: SocketAddr,
        pub seen: Arc<Mutex<Vec<Seen>>>,
        pub connections: Arc<AtomicUsize>,
        pub roots: RootCertStore,
    }

    pub async fn start(answers: Vec<Answer>) -> Server {
        let names = vec![SANDBOX.to_string(), PRODUCTION.to_string()];
        let cert = rcgen::generate_simple_self_signed(names).unwrap();
        let der = CertificateDer::from(cert.cert.der().to_vec());
        let mut roots = RootCertStore::empty();
        roots.add(der.clone()).unwrap();
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()));
        let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![der], key)
            .unwrap();
        cfg.alpn_protocols = vec![b"h2".to_vec()];
        let acc = TlsAcceptor::from(Arc::new(cfg));
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let at = l.local_addr().unwrap();
        let seen: Arc<Mutex<Vec<Seen>>> = Default::default();
        let connections = Arc::new(AtomicUsize::new(0));
        let next = Arc::new(AtomicUsize::new(0));
        let answers = Arc::new(answers);
        let (s2, c2) = (seen.clone(), connections.clone());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = l.accept().await {
                c2.fetch_add(1, Ordering::SeqCst);
                let (acc, seen, next, answers) = (acc.clone(), s2.clone(), next.clone(), answers.clone());
                tokio::spawn(async move {
                    let Ok(tls) = acc.accept(tcp).await else { return };
                    let Ok(mut conn) = h2::server::handshake(tls).await else { return };
                    while let Some(Ok((req, mut respond))) = conn.accept().await {
                        let (seen, next, answers) = (seen.clone(), next.clone(), answers.clone());
                        tokio::spawn(async move {
                            let (parts, mut body) = req.into_parts();
                            let mut bytes = Vec::new();
                            while let Some(Ok(c)) = body.data().await {
                                let _ = body.flow_control().release_capacity(c.len());
                                bytes.extend_from_slice(&c);
                            }
                            seen.lock().unwrap().push(Seen {
                                authority: parts.uri.authority().map(|a| a.to_string()).unwrap_or_default(),
                                path: parts.uri.path().to_string(),
                                headers: parts.headers.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect(),
                                body: bytes,
                            });
                            let i = next.fetch_add(1, Ordering::SeqCst).min(answers.len() - 1);
                            match answers[i].clone() {
                                Answer::Hang => std::future::pending::<()>().await,
                                Answer::Status(code, reason) => {
                                    let resp = http::Response::builder().status(code).body(()).unwrap();
                                    let Ok(mut s) = respond.send_response(resp, false) else { return };
                                    let b = if reason.is_empty() { Bytes::new() } else { Bytes::from(format!("{{\"reason\":\"{reason}\"}}")) };
                                    let _ = s.send_data(b, true);
                                }
                            }
                        });
                    }
                });
            }
        });
        Server { at, seen, connections, roots }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{self, Answer};
    use super::*;
    use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};
    use std::sync::atomic::Ordering;

    fn on<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap().block_on(f)
    }

    fn key() -> (Arc<ApnsKey>, Vec<u8>) {
        let dir = std::env::temp_dir().join(format!("wt-apns-key-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (pem, public) = super::super::key::test_p8();
        let p8 = dir.join("k.p8");
        std::fs::write(&p8, pem).unwrap();
        let k = super::super::key::import(&dir, &p8, "ABC123DEFG", "M882BTE3A3").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        (Arc::new(k), public)
    }

    fn req(env: PushEnv, now: i64) -> Request {
        Request {
            env,
            token: "ab".repeat(32),
            topic: "com.brethash.worktrees.ios".into(),
            collapse: Some("c0".into()),
            expiration: now + 3600,
            priority: 10,
            body: br#"{"aps":{}}"#.to_vec(),
            kind: "input",
            key_id: "AQIDBAUGBwg=".into(),
            now,
        }
    }

    fn unb64url(s: &str) -> Vec<u8> {
        let mut t = s.replace('-', "+").replace('_', "/");
        while !t.len().is_multiple_of(4) {
            t.push('=');
        }
        crate::serve::stepup::b64_decode(&t).unwrap()
    }

    /// Everything a push puts on the wire: the path, every header, the JWT
    /// verified with the public half, the environment's own host.
    #[test]
    fn the_headers_and_the_token_are_apples() {
        on(async {
            let s = fixture::start(vec![Answer::Status(200, "")]).await;
            let (k, public) = key();
            let h = H2::new(k, Dial::fixture(s.at, Some(s.roots.clone())));
            assert_eq!(h.send(req(PushEnv::Development, 1000)).await, Delivery::Sent);
            assert_eq!(h.send(req(PushEnv::Production, 1000)).await, Delivery::Sent);
            let seen = s.seen.lock().unwrap().clone();
            assert_eq!(seen[0].authority, SANDBOX, "development goes to the sandbox");
            assert_eq!(seen[1].authority, PRODUCTION, "production to production");
            let a = &seen[0];
            assert_eq!(a.path, format!("/3/device/{}", "ab".repeat(32)));
            assert_eq!(a.headers["apns-push-type"], "alert");
            assert_eq!(a.headers["apns-topic"], "com.brethash.worktrees.ios");
            assert_eq!(a.headers["apns-priority"], "10");
            assert_eq!(a.headers["apns-expiration"], "4600");
            assert_eq!(a.headers["apns-collapse-id"], "c0");
            assert_eq!(a.body, br#"{"aps":{}}"#);
            let jwt = a.headers["authorization"].strip_prefix("bearer ").unwrap();
            let parts: Vec<&str> = jwt.split('.').collect();
            let head: serde_json::Value = serde_json::from_slice(&unb64url(parts[0])).unwrap();
            let claims: serde_json::Value = serde_json::from_slice(&unb64url(parts[1])).unwrap();
            assert_eq!(head, serde_json::json!({ "alg": "ES256", "kid": "ABC123DEFG" }));
            assert_eq!(claims, serde_json::json!({ "iss": "M882BTE3A3", "iat": 1000 }));
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public).verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &unb64url(parts[2])).expect("the signature verifies");
        })
    }

    #[test]
    fn no_collapse_id_goes_without_the_header() {
        on(async {
            let s = fixture::start(vec![Answer::Status(200, "")]).await;
            let (k, _) = key();
            let h = H2::new(k, Dial::fixture(s.at, Some(s.roots.clone())));
            let mut r = req(PushEnv::Production, 1);
            r.collapse = None;
            h.send(r).await;
            assert!(!s.seen.lock().unwrap()[0].headers.contains_key("apns-collapse-id"));
        })
    }

    /// Every status APNs documents here, through the wire.
    #[test]
    fn each_status_means_what_it_should() {
        on(async {
            for (status, reason, want) in [
                (200, "", Delivery::Sent),
                (410, "Unregistered", Delivery::Dead("unregistered")),
                (400, "BadDeviceToken", Delivery::Dead("bad-device-token")),
                (400, "DeviceTokenNotForTopic", Delivery::Dead("token-not-for-topic")),
                (400, "BadCollapseId", Delivery::Rejected("bad-collapse-id")),
                (413, "PayloadTooLarge", Delivery::Rejected("bad-payload")),
                (403, "InvalidProviderToken", Delivery::Auth("invalid-provider-token")),
                (403, "ExpiredProviderToken", Delivery::Auth("expired-provider-token")),
                (429, "TooManyRequests", Delivery::Retry("status-429")),
                (500, "InternalServerError", Delivery::Retry("status-5xx")),
                (503, "ServiceUnavailable", Delivery::Retry("status-5xx")),
            ] {
                let s = fixture::start(vec![Answer::Status(status, reason)]).await;
                let (k, _) = key();
                let h = H2::new(k, Dial::fixture(s.at, Some(s.roots.clone())));
                assert_eq!(h.send(req(PushEnv::Production, 1)).await, want, "{status} {reason}");
            }
        })
    }

    /// A 3xx is an answer, not an instruction: nothing follows it.
    #[test]
    fn a_redirect_is_not_followed() {
        on(async {
            let s = fixture::start(vec![Answer::Status(307, "")]).await;
            let (k, _) = key();
            let h = H2::new(k, Dial::fixture(s.at, Some(s.roots.clone())));
            assert_eq!(h.send(req(PushEnv::Production, 1)).await, Delivery::Rejected("other"));
            assert_eq!(s.seen.lock().unwrap().len(), 1, "one request, to one place");
            assert_eq!(s.connections.load(Ordering::SeqCst), 1);
        })
    }

    /// A server that does not chain to Apple's roots is refused before a byte
    /// of the push, and the failure says `trust`, distinct from any other.
    #[test]
    fn a_certificate_apple_did_not_get_is_refused_and_named() {
        on(async {
            let s = fixture::start(vec![Answer::Status(200, "")]).await;
            let (k, _) = key();
            let h = H2::new(k, Dial::fixture(s.at, None));
            assert_eq!(h.send(req(PushEnv::Production, 1)).await, Delivery::Retry("trust"));
            assert_eq!(s.seen.lock().unwrap().len(), 0, "nothing was sent");
        })
    }

    /// Nothing listening: a bounded failure, said as such.
    #[test]
    fn no_server_is_a_retry() {
        on(async {
            let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let at = l.local_addr().unwrap();
            drop(l);
            let (k, _) = key();
            let h = H2::new(k, Dial::fixture(at, Some(RootCertStore::empty())));
            assert_eq!(h.send(req(PushEnv::Production, 1)).await, Delivery::Retry("connect"));
        })
    }

    /// A server that takes the push and never answers costs the send bound,
    /// twice at most (a held connection's one retry). Nothing races: it never
    /// answers, whatever the scheduler does.
    #[test]
    fn a_hung_apple_is_bounded() {
        on(async {
            let s = fixture::start(vec![Answer::Hang]).await;
            let (k, _) = key();
            let h = H2::bounded(k, Dial::fixture(s.at, Some(s.roots.clone())), Duration::from_millis(300));
            let out = h.send(req(PushEnv::Production, 1)).await;
            assert_eq!(out, Delivery::Retry("send-timeout"));
        })
    }

    /// One connection per environment, kept.
    #[test]
    fn the_connection_is_kept() {
        on(async {
            let s = fixture::start(vec![Answer::Status(200, "")]).await;
            let (k, _) = key();
            let h = H2::new(k, Dial::fixture(s.at, Some(s.roots.clone())));
            for _ in 0..3 {
                assert_eq!(h.send(req(PushEnv::Production, 1)).await, Delivery::Sent);
            }
            assert_eq!(s.connections.load(Ordering::SeqCst), 1);
            assert_eq!(s.seen.lock().unwrap().len(), 3);
        })
    }

    /// Made at most every 20 minutes, never over 60 old: the same token inside
    /// its life, a new one at 45, and an expired answer asks for a new one only
    /// once 20 minutes have passed.
    #[test]
    fn the_provider_token_keeps_apples_two_limits() {
        let (k, _) = key();
        let mut j = Jwt::default();
        let t0 = j.token(&k, 0).unwrap();
        assert_eq!(j.token(&k, 19 * 60).unwrap(), t0);
        assert_eq!(j.token(&k, JWT_FRESH - 1).unwrap(), t0);
        let t1 = j.token(&k, JWT_FRESH).unwrap();
        assert_ne!(t1, t0, "replaced before 60 minutes");
        j.stale = true;
        assert_eq!(j.token(&k, JWT_FRESH + JWT_MIN - 1).unwrap(), t1, "not inside 20 minutes, even when told");
        assert_ne!(j.token(&k, JWT_FRESH + JWT_MIN).unwrap(), t1);
        const { assert!(JWT_FRESH < 60 * 60 && JWT_FRESH >= JWT_MIN) };
    }

    /// The only hosts this module can name, and the two roots as measured. A
    /// proxy or host override would have to read a variable: there is none.
    #[test]
    fn exactly_two_hosts_and_nothing_from_the_environment() {
        assert_eq!(host(PushEnv::Production), "api.push.apple.com");
        assert_eq!(host(PushEnv::Development), "api.sandbox.push.apple.com");
        let src = include_str!("apns.rs");
        let live = src.split("#[cfg(test)]\npub mod fixture").next().unwrap();
        let line_of_code = |l: &&str| !l.trim_start().starts_with("//");
        for bad in ["env::var", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "SSL_CERT", "reqwest", "dangerous()"] {
            assert!(!live.lines().filter(line_of_code).any(|l| l.contains(bad)), "{bad}");
        }
        let names: Vec<&str> = live.lines().filter(line_of_code).filter(|l| l.contains(".com")).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        use sha2::{Digest, Sha256};
        let prints: Vec<String> = ROOTS.iter().map(|r| crate::serve::devices::hex(&Sha256::digest(r))).collect();
        assert_eq!(prints, ["e793c9b02fd8aa13e21c31228accb08119643b749c898964b1746d46c3d4cbd2", "d7a7a0fb5d7e2731d771e9484ebcdef71d5f0c3e0a2948782bc83ee0ea699ef4"]);
        assert!(real_tls().is_ok());
    }

    /// The dry run's line holds no token, key or body.
    #[test]
    fn the_dry_run_says_only_what_it_may() {
        on(async {
            let lines: Arc<Mutex<Vec<String>>> = Default::default();
            let l = lines.clone();
            let d = DryRun { line: Arc::new(move |s| l.lock().unwrap().push(s.to_string())) };
            let mut r = req(PushEnv::Production, 1);
            r.body = br#"{"aps":{},"wt":{"c":"CIPHERTEXT"}}"#.to_vec();
            assert_eq!(d.send(r).await, Delivery::Sent);
            let line = lines.lock().unwrap()[0].clone();
            assert_eq!(line, "serve: push: dry run: kind=input bytes=34 key_id=AQIDBAUGBwg= sealed=true priority=10");
        })
    }
}
