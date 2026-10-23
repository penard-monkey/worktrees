//! The remote listener (Phase 2 §4.1, §4.5): one private address on one
//! interface, mutual TLS pinned both ways, then the same WebSocket and the
//! same handshake v1 as loopback, keyed by the TLS session.
//!
//! A connection, in order — and nothing is written on any refusal before the
//! TLS handshake, which is the one unavoidable write (a TLS alert, §4.5):
//!
//! 1. **The peer rule** (`peer.rs`) on the source address, then the caps
//!    (§6): connections in all, handshakes in flight, handshakes from one
//!    source. Past any of them the socket is dropped where it stands.
//! 2. **TLS 1.3** with `tls::DeviceVerifier`, inside [`TLS_DEADLINE`]: a key
//!    that is not a live device's ends inside rustls, before HTTP — an
//!    unknown device never gets as far as a line in serve.log of its own
//!    (they are counted per episode, `Episode`). The verifier is made per
//!    connection (`tls::with_verifier`) so the registry's `ConnectRefused`
//!    names the address, and that event is the audit log's ONE record of the
//!    knock: the listener records only what ends TLS otherwise. Then
//!    `tls::admit_peer` again from `peer_certificates()`: the verifier is the
//!    early exit, not the only gate (§4.2).
//! 3. **The upgrade**, on the remote rule set (`policy::remote_host_ok`,
//!    `remote_origin_ok`, `upgrade_ok`). A refusal closes with nothing
//!    written, as on loopback.
//! 4. **Handshake v1, unchanged** (`conn::handshake_v1`), keyed by
//!    `hex(exporter(EXPORTER-worktrees-remote-v1))` instead of serve.json's
//!    token: the same 512-byte budget, the same `AUTH_DEADLINE`, and the `ok`
//!    names the device. Decided by (main) 2026-10-22 so that `ipcws.ts` stays
//!    the protocol's one client and the shim a pipe.
//! 5. **The session**: `conn::serve_caller` with the device's `Caller`, its
//!    step-up exporter, and a keepalive and lifetime ([`LIVENESS`]).
//!
//! Who the caller is comes from the key that completed the handshake and
//! nothing else (§3.3): never a header, an `Origin` or the peer address.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::Empty;
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONNECTION, HOST, ORIGIN, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::WebSocketStream;
use worktrees_api::log::{info, warn};
use worktrees_api::Ctx;
use worktrees_core::authz::{Caller, Device};

use super::super::audit::{Audit, ConnAudit};
use super::super::budget::{Counter, Held};
use super::super::conn::{self, Budget, Clients, Liveness, Refused, RemoteSession, MAX_HTTP_BUF, MAX_MESSAGE, PREAUTH_BYTES};
use super::super::devices::{hex, DeviceEvent, DeviceRegistry, Subject};
use super::super::{policy, tls};
use super::peer::Rule;
use super::sock;

/// The label handshake v1's key is exported under on a remote session: 32
/// bytes of RFC 8446 §7.5, no context, computed by both ends.
// moves to worktrees-remote
pub const EXPORTER_REMOTE: &[u8] = b"EXPORTER-worktrees-remote-v1";

/// The TLS handshake's deadline (§6).
pub const TLS_DEADLINE: Duration = Duration::from_secs(5);

/// The remote listener's own caps (§6), apart from loopback's: a flood on
/// one listener never fills the other's slots.
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    /// Remote connections in all, from accept to close.
    pub connections: usize,
    /// Connections that have not finished handshake v1 yet: TLS, the
    /// upgrade and the proof.
    pub handshakes: usize,
    /// Of those, from any one source address.
    pub per_source: usize,
}

pub const CAPS: Caps = Caps { connections: 32, handshakes: 8, per_source: 2 };

/// §4.2: a ping every 20 s, closed after 60 s with no pong (§6), and a fresh
/// handshake — and step-up — at least once a day.
pub const LIVENESS: Liveness =
    Liveness { ping_every: Duration::from_secs(20), dead_after: Duration::from_secs(60), lifetime: Duration::from_secs(24 * 3600) };

/// Refusals of one kind, said once per episode with a count rather than once
/// each: a flood of strangers must not become a disk-writing flood.
pub struct Episode {
    what: &'static str,
    open: AtomicBool,
    count: AtomicU64,
    last: AtomicI64,
}

/// An episode ends after this long with no refusal of its kind.
const EPISODE_QUIET: i64 = 60;

impl Episode {
    pub const fn new(what: &'static str) -> Episode {
        Episode { what, open: AtomicBool::new(false), count: AtomicU64::new(0), last: AtomicI64::new(0) }
    }

    pub fn note(&self) {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.last.store(worktrees_core::sysclock::now_epoch(), Ordering::SeqCst);
        if !self.open.swap(true, Ordering::SeqCst) {
            info(&format!("serve: remote: {}; counting until it stops", self.what));
        }
    }

    /// The poll's tick: close an episode that has gone quiet, with its count.
    pub fn tick(&self, now: i64) {
        if self.open.load(Ordering::SeqCst) && now - self.last.load(Ordering::SeqCst) >= EPISODE_QUIET && self.open.swap(false, Ordering::SeqCst) {
            let n = self.count.swap(0, Ordering::SeqCst);
            info(&format!("serve: remote: {} — {n} in that episode", self.what));
        }
    }
}

/// What every remote connection is answered from. One per serve: it outlives
/// each bind, so a pause and a resume change nothing a device is measured by.
pub struct Deps {
    pub ctx: Ctx,
    pub clients: Arc<Clients>,
    pub devices: Arc<DeviceRegistry>,
    /// The audit log's one writer (§5.1): each remote connection's records,
    /// and its refusals before it is a connection.
    pub audit: Arc<Audit>,
    /// The TLS configuration over the server identity; replaced when the
    /// identity rotates, read once per connection.
    pub tls: Mutex<Option<Arc<rustls::ServerConfig>>>,
    pub caps: Caps,
    pub live: Liveness,
    /// Peers outside the rule (§4.1), connections past a cap, handshakes
    /// refused at TLS (an unknown, revoked, disabled or expired key, or one
    /// too slow).
    pub off_rule: Episode,
    pub capped: Episode,
    pub refused: Episode,
}

impl Deps {
    pub fn new(ctx: Ctx, clients: Arc<Clients>, devices: Arc<DeviceRegistry>, audit: Arc<Audit>) -> Deps {
        Deps {
            ctx,
            clients,
            devices,
            audit,
            tls: Mutex::new(None),
            caps: CAPS,
            live: LIVENESS,
            off_rule: Episode::new("dropping connections from outside the peer rule (§4.1)"),
            capped: Episode::new("at a connection cap; dropping new connections"),
            refused: Episode::new("refusing TLS handshakes from keys that are not a live device's"),
        }
    }

    pub fn tick(&self, now: i64) {
        self.off_rule.tick(now);
        self.capped.tick(now);
        self.refused.tick(now);
    }
}

/// One bind's state: its address and rule, and its counts.
struct Shared {
    deps: Arc<Deps>,
    /// The address as a device dials it: no zone.
    at: SocketAddr,
    rule: Rule,
    /// Cleared by `RemoteListener::stop`, BEFORE the caller sweeps the device
    /// connections: a connection still shaking hands checks it once its
    /// session has joined (`Clients::end_if_gone`).
    open: Arc<AtomicBool>,
    connections: Arc<Counter>,
    handshakes: Arc<Counter>,
    sources: Mutex<HashMap<IpAddr, usize>>,
}

/// A connection's claim on the caps. The handshake half (and the source's
/// count) is given back once handshake v1 completes; the connection half
/// when the connection ends.
struct Admission {
    _connection: Held,
    handshake: Mutex<Option<(Held, Source)>>,
}

impl Admission {
    fn authenticated(&self) {
        self.handshake.lock().unwrap().take();
    }
}

/// One handshake's count against its source address.
struct Source {
    shared: Arc<Shared>,
    ip: IpAddr,
}

impl Drop for Source {
    fn drop(&mut self) {
        let mut m = self.shared.sources.lock().unwrap();
        if let Some(n) = m.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                m.remove(&self.ip);
            }
        }
    }
}

impl Shared {
    fn admit(self: &Arc<Self>, ip: IpAddr) -> Option<Arc<Admission>> {
        let connection = self.connections.try_hold()?;
        let handshake = self.handshakes.try_hold()?;
        {
            let mut m = self.sources.lock().unwrap();
            let n = m.entry(ip).or_insert(0);
            if *n >= self.deps.caps.per_source {
                return None;
            }
            *n += 1;
        }
        let source = Source { shared: self.clone(), ip };
        Some(Arc::new(Admission { _connection: connection, handshake: Mutex::new(Some((handshake, source))) }))
    }
}

/// A running remote listener: where devices reach it, and its accept task.
pub struct RemoteListener {
    pub at: SocketAddr,
    /// The kernel would not tie the socket to its interface (an older Linux).
    pub warning: Option<String>,
    open: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl RemoteListener {
    /// Stop accepting and release the port, and refuse every connection of
    /// this bind that has not joined yet. The ones that have are the
    /// caller's to end next (`Clients::end_every_device`), with a reason.
    pub fn stop(&self) {
        self.open.store(false, Ordering::SeqCst);
        self.task.abort();
    }
}

impl Drop for RemoteListener {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Bind `ip:port` (0: the OS picks) on interface `index`, and accept. Inside
/// a tokio runtime. `rule` is the bind's peer rule (§4.1).
pub fn start(deps: Arc<Deps>, ip: IpAddr, port: u16, index: u32, rule: Rule) -> std::io::Result<RemoteListener> {
    let (std_listener, warning) = sock::listen(ip, port, index)?;
    let at = sock::public(ip, std_listener.local_addr()?.port());
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    let open = Arc::new(AtomicBool::new(true));
    let shared = Arc::new(Shared {
        open: open.clone(),
        connections: Counter::new(deps.caps.connections),
        handshakes: Counter::new(deps.caps.handshakes.min(deps.caps.connections)),
        deps,
        at,
        rule,
        sources: Mutex::new(HashMap::new()),
    });
    let task = tokio::spawn(async move {
        let mut said = false;
        loop {
            let (tcp, peer) = match listener.accept().await {
                Ok(v) => {
                    said = false;
                    v
                }
                Err(e) => {
                    if !said {
                        said = true;
                        warn(&format!("serve: remote: accept failed: {e}"));
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            // Before TLS, so before a byte either way: a stream dropped here
            // is closed with nothing written.
            if !shared.rule.admits(peer.ip()) {
                shared.deps.off_rule.note();
                continue;
            }
            let Some(adm) = shared.admit(peer.ip()) else {
                shared.deps.capped.note();
                continue;
            };
            tokio::spawn(connection(tcp, peer, shared.clone(), adm));
        }
    });
    Ok(RemoteListener { at, warning, open, task })
}

/// The `device` an `ok` names: the record as it stands now.
pub fn device_frame(devices: &DeviceRegistry, caller: &Caller) -> Option<Value> {
    let Device::Paired(d) = caller.device() else { return None };
    let r = devices.get(caller.principal(), d)?;
    Some(json!({
        "id": r.id, "principal": r.principal, "name": r.name, "caps": r.caps,
        "projects": r.projects, "expiry": r.expiry, "stepup_required": r.stepup_required,
    }))
}

/// §5.3: an alert when a device connects from another address CLASS than it
/// was last seen from (`netclass::Class`) — a tailnet device now on the LAN,
/// a LAN device now through a carrier's CGNAT. Its address within a class
/// moves all the time and says nothing.
/// Did `DeviceVerifier` refuse the key? It answers exactly this error for a
/// registry refusal and for nothing else, and tokio-rustls hands the rustls
/// error back inside the `io::Error`.
fn knocked(e: &std::io::Error) -> bool {
    e.get_ref()
        .and_then(|i| i.downcast_ref::<rustls::Error>())
        .is_some_and(|e| matches!(e, rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)))
}

fn new_address_class(devices: &DeviceRegistry, caller: &Caller, d: &worktrees_core::authz::DeviceId, now: IpAddr) {
    let class = |ip: IpAddr| worktrees_core::netclass::classify(ip).ok();
    let was = devices.last_addr(d).and_then(|a| a.parse::<IpAddr>().ok()).and_then(class);
    let (Some(was), Some(is)) = (was, class(now)) else { return };
    if was == is {
        return;
    }
    let name = |c: worktrees_core::netclass::Class| {
        use worktrees_core::netclass::Class::*;
        match c {
            Rfc1918 => "rfc1918",
            Cgnat => "cgnat",
            LinkLocal => "link-local",
            UniqueLocal => "ula",
        }
    };
    let subject = Subject { principal: caller.principal().clone(), device: d.clone() };
    devices.announce(DeviceEvent::NewAddressClass { subject, from: name(was).into(), to: name(is).into() });
}

/// What the TLS session established, for the upgrade and the session.
struct Session {
    caller: Caller,
    /// handshake v1's key: `hex(exporter(EXPORTER_REMOTE))`.
    key: String,
    stepup: [u8; 32],
    device: Value,
}

async fn connection(tcp: TcpStream, peer: SocketAddr, shared: Arc<Shared>, adm: Arc<Admission>) {
    let Some(base) = shared.deps.tls.lock().unwrap().clone() else { return };
    let verifier = Arc::new(tls::DeviceVerifier::for_peer(shared.deps.devices.clone(), peer.ip()));
    let Ok(cfg) = tls::with_verifier(&base, verifier) else { return };
    let _ = tcp.set_nodelay(true);
    let id = shared.deps.clients.next_id();
    let tls = match tokio::time::timeout(TLS_DEADLINE, tokio_rustls::TlsAcceptor::from(cfg).accept(tcp)).await {
        Ok(Ok(s)) => s,
        r => {
            shared.deps.refused.note();
            // A key the verifier refused is the registry's event, which the
            // audit log records with this address (`Audit::knock`); recording
            // it here too would count one knock twice.
            if !matches!(&r, Ok(Err(e)) if knocked(e)) {
                shared.deps.audit.auth_failed(if r.is_err() { "tls-deadline" } else { "tls" }, id, Some(peer.ip().to_string()));
            }
            return;
        }
    };
    let session = {
        let c = tls.get_ref().1;
        let now = worktrees_core::sysclock::now_epoch();
        // A refusal here is the registry's event as well, with the address:
        // the handshake verified a certificate, so it is the registry's
        // check that said no.
        let Ok(caller) = tls::admit_peer(c, &shared.deps.devices, now, Some(peer.ip())) else {
            shared.deps.refused.note();
            return;
        };
        let (Ok(key), Ok(stepup)) = (tls::exporter(c, EXPORTER_REMOTE), tls::exporter(c, tls::EXPORTER_STEPUP)) else { return };
        let Some(device) = device_frame(&shared.deps.devices, &caller) else { return };
        Arc::new(Session { caller, key: hex(&key), stepup, device })
    };
    let svc = {
        let (shared, adm) = (shared.clone(), adm.clone());
        hyper::service::service_fn(move |req: Request<Incoming>| {
            let (shared, adm, session) = (shared.clone(), adm.clone(), session.clone());
            async move { upgrade(req, shared, adm, session, peer, id) }
        })
    };
    let mut http = hyper::server::conn::http1::Builder::new();
    http.max_buf_size(MAX_HTTP_BUF);
    let conn = http.serve_connection(TokioIo::new(tls), svc).with_upgrades();
    match tokio::time::timeout(conn::HANDSHAKE_DEADLINE, conn).await {
        Ok(Err(e)) if e.is_user() => {}
        Ok(Err(e)) => info(&format!("serve: remote: connection from {peer} ended: {e}")),
        Ok(Ok(())) | Err(_) => {}
    }
}

/// The remote rule set, then the 101. A device that holds an enrolled key
/// and is refused here is worth a line; nobody else gets this far.
fn upgrade(mut req: Request<Incoming>, shared: Arc<Shared>, adm: Arc<Admission>, session: Arc<Session>, peer: SocketAddr, id: u64) -> Result<Response<Empty<Bytes>>, Refused> {
    let refuse = |why: &'static str| {
        info(&format!("serve: remote: refused {}'s upgrade from {peer}: {why}", session.caller));
        shared.deps.audit.upgrade_refused(why, Some(peer.ip().to_string()));
        Refused(why)
    };
    let h = req.headers();
    let host = conn::one_header(h, HOST).map_err(|e| refuse(e.0))?;
    if !policy::remote_host_ok(host.as_deref(), shared.at) {
        return Err(refuse("host"));
    }
    let origin = conn::one_header(h, ORIGIN).map_err(|e| refuse(e.0))?;
    if !policy::remote_origin_ok(origin.as_deref()) {
        return Err(refuse("origin"));
    }
    let upgrade = conn::one_header(h, UPGRADE).map_err(|e| refuse(e.0))?;
    let connection = conn::one_header(h, CONNECTION).map_err(|e| refuse(e.0))?;
    let version = conn::one_header(h, SEC_WEBSOCKET_VERSION).map_err(|e| refuse(e.0))?;
    let key = conn::one_header(h, SEC_WEBSOCKET_KEY).map_err(|e| refuse(e.0))?;
    let hs = policy::Handshake {
        method: req.method().as_str(),
        path: req.uri().path(),
        query: req.uri().query(),
        upgrade: upgrade.as_deref(),
        connection: connection.as_deref(),
        version: version.as_deref(),
        key: key.as_deref(),
    };
    if !policy::upgrade_ok(&hs) {
        return Err(refuse("not a websocket upgrade"));
    }
    let accept = derive_accept_key(key.as_deref().unwrap_or_default().trim().as_bytes());
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        let upgraded = match on_upgrade.await {
            Ok(u) => u,
            Err(e) => return warn(&format!("serve: remote: upgrade failed after 101: {e}")),
        };
        let budget = Arc::new(AtomicUsize::new(PREAUTH_BYTES));
        let io = Budget { inner: TokioIo::new(upgraded), left: budget.clone() };
        let config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
        let mut ws = WebSocketStream::from_raw_socket(io, Role::Server, Some(config)).await;
        let authed = tokio::time::timeout(policy::AUTH_DEADLINE, conn::handshake_v1(&mut ws, &session.key, Some(&session.device), &())).await;
        if !matches!(authed, Ok(Some(()))) {
            info(&format!("serve: remote: {} did not complete handshake v1 from {peer}", session.caller));
            shared.deps.audit.auth_failed(if authed.is_err() { "deadline" } else { "handshake" }, id, Some(peer.ip().to_string()));
            return;
        }
        adm.authenticated();
        budget.store(usize::MAX, Ordering::SeqCst);
        if let Device::Paired(d) = session.caller.device() {
            new_address_class(&shared.deps.devices, &session.caller, d, peer.ip());
            shared.deps.devices.seen(d, Some(peer.ip().to_string()), worktrees_core::sysclock::now_epoch());
        }
        info(&format!("serve: client {id} connected: {} from {peer}", session.caller));
        let d = &shared.deps;
        let remote = RemoteSession { live: d.live, bind_open: shared.open.clone() };
        // Every audit record the connection writes names it (§5.1).
        let au = Arc::new(ConnAudit::new(d.audit.clone(), id, Some(peer.ip().to_string()), session.caller.clone()));
        conn::serve_caller(ws, d.ctx.clone(), d.clients.clone(), id, session.caller.clone(), Some(session.stepup), au, Some(remote)).await;
        drop(adm);
    });
    Ok(Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .body(Empty::new())
        .expect("a 101 with three fixed headers is a valid response"))
}
