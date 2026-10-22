//! The listener and one connection's life: HTTP upgrade, auth, frames.
//!
//! hyper parses the request and the policy runs over its typed headers; a
//! refusal is the service returning `Err`, which hyper answers by dropping the
//! connection with no bytes written. An accepted upgrade hands the raw socket
//! to tungstenite's frame codec (`from_raw_socket`), never to its handshake —
//! that one WRITES an error response on refusal, which is the one thing this
//! server must not do.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdListener};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{ready, Context, Poll};

use futures_util::{SinkExt, StreamExt};
use http_body_util::Empty;
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderMap, HeaderName, CONNECTION, HOST, ORIGIN, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use worktrees_api::log::{error, info, warn};
use worktrees_api::{Ctx, Host};

use super::dispatch::{self, Args, Call};
use super::policy;
use super::resolve::World;
use super::scope;
use super::servefile;
use super::wire::{self, Frames, Outbox};
use worktrees_core::authz::{self, Caller, Capability, NoDevices, Registry};

// ── limits (OWASP M1) ────────────────────────────────────────────────────────
//
// The review measured, against 495df95: six unauthenticated sockets each
// part-way through a 60 MiB message took RSS from 21 MB to 277 MB, and 300
// idle TCP connections spent the default 256 descriptors, after which the
// user's own client was RST and an authenticated one's `tmux_check` failed.
// Nothing here can stop a local process making NEW connections hard to get
// while it floods — it can always hold every unauthenticated slot — but it
// can no longer take the memory, the descriptors, or a client already in.

/// How long the HTTP half of a connection gets: the upgrade request, from
/// accept to the 101. A browser needs milliseconds; an idle socket holds a
/// task, a descriptor and an unauthenticated slot until this ends it. It was
/// the docs server's 20 s.
pub(crate) const HANDSHAKE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// How many connections may be open at once, and how many of those may be
/// unauthenticated (from accept until the client's proof verifies). Beyond
/// either, a NEW connection is accepted and closed at once with nothing
/// written. Generous for any real use — the desktop app holds one socket, a
/// dev page a couple — and small enough that a flood costs bounded memory
/// and descriptors.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub connections: usize,
    pub unauthenticated: usize,
}
pub const LIMITS: Limits = Limits { connections: 64, unauthenticated: 16 };

/// Bytes a peer may send between its upgrade and its proof verifying. The
/// two handshake frames are under 100 bytes each, framing and mask included;
/// anything past this ends the connection where it stands, so a peer with no
/// token cannot make the server buffer a message for it. (tokio-tungstenite
/// cannot change its limits after construction, so the pre-auth bound lives
/// in the socket under it — `Budget` — and the codec carries the post-auth one.)
pub const PREAUTH_BYTES: usize = 512;

/// The largest message, and frame, an AUTHENTICATED client may send.
/// Measured: the editor saves at most a file it could read, and `read_file`
/// stops at 1 MB — 0.98 MiB of JSON for this repository's Rust, 5.72 MiB in
/// the worst escaping case (all control characters); a 1 MB paste goes as a
/// JSON array of bytes, 3.35 MiB of ASCII or 3.81 MiB of emoji. 16 MiB covers
/// the worst save 2.8× over and pastes to about 4 MB, where tungstenite's
/// default let one client make the server hold 64 MiB per message. Past it
/// the connection ends; the client reconnects.
pub const MAX_MESSAGE: usize = 16 << 20;

/// hyper's buffer for the HTTP request, before any upgrade (its default is
/// ~400 KiB). Cookies are per host, not per port, so a browser may send those
/// of every other dev server on 127.0.0.1 — 64 KiB leaves room for them.
const MAX_HTTP_BUF: usize = 64 * 1024;

/// A count with a cap. `try_hold` takes one or says no; the `Held` gives it
/// back on drop, whichever way the connection ends.
struct Counter {
    held: AtomicUsize,
    cap: usize,
}

struct Held(Arc<Counter>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.held.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Counter {
    fn new(cap: usize) -> Arc<Self> {
        Arc::new(Counter { held: AtomicUsize::new(0), cap })
    }
    fn try_hold(self: &Arc<Self>) -> Option<Held> {
        self.held
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < self.cap).then_some(n + 1))
            .ok()
            .map(|_| Held(self.clone()))
    }
}

/// One connection's claim on the caps, from accept to close. The
/// unauthenticated half is handed back the moment the client's proof verifies.
struct Slot {
    _connection: Held,
    unauthenticated: Mutex<Option<Held>>,
}

impl Slot {
    fn authenticated(&self) {
        self.unauthenticated.lock().unwrap().take();
    }
}

/// The upgraded socket, read through a byte budget. `left` counts down what a
/// peer may still send; `usize::MAX` means no budget (set once the client has
/// proven itself), and a peer that reaches 0 gets a read error, which
/// tungstenite turns into the end of the connection. Writes pass through.
struct Budget<T> {
    inner: T,
    left: Arc<AtomicUsize>,
}

impl<T: AsyncRead + Unpin> AsyncRead for Budget<T> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let left = self.left.load(Ordering::SeqCst);
        if left == usize::MAX {
            return Pin::new(&mut self.inner).poll_read(cx, buf);
        }
        if left == 0 {
            return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the pre-auth byte budget is spent")));
        }
        let mut tmp = [0u8; PREAUTH_BYTES];
        let n = left.min(buf.remaining()).min(tmp.len());
        let mut small = ReadBuf::new(&mut tmp[..n]);
        ready!(Pin::new(&mut self.inner).poll_read(cx, &mut small))?;
        let got = small.filled().len();
        buf.put_slice(small.filled());
        // Never undo the lift: only a budget still in force is counted down.
        let _ = self.left.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |l| (l != usize::MAX).then(|| l.saturating_sub(got)));
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Budget<T> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, b: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
    fn poll_write_vectored(mut self: Pin<&mut Self>, cx: &mut Context<'_>, b: &[std::io::IoSlice<'_>]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, b)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

/// The accept loop's pause after a failed `accept()` — hyper's
/// `sleep_on_errors` shape, so a process-wide `EMFILE` is a retry rather than a
/// pegged core (see the docs server, which learned it first).
const ACCEPT_RETRY: std::time::Duration = std::time::Duration::from_millis(50);

/// Every authenticated connection's queue and caller, for broadcasts, and
/// where every caller's grant is read.
pub struct Clients {
    map: Mutex<HashMap<u64, (Outbox, Caller)>>,
    registry: Arc<dyn Registry>,
    world: Option<Arc<dyn World>>,
}

impl Default for Clients {
    /// No paired devices (Phase 2 slice 1): the console is the only caller
    /// with a grant. Objects resolve against the live registry and tmux.
    fn default() -> Self {
        Clients::with(Arc::new(NoDevices), None)
    }
}

impl Clients {
    /// `world: None` is the live one; a test hands in fixtures.
    pub fn with(registry: Arc<dyn Registry>, world: Option<Arc<dyn World>>) -> Self {
        Clients { map: Mutex::new(HashMap::new()), registry, world }
    }
    pub(super) fn add(&self, id: u64, out: Outbox, caller: Caller) {
        self.map.lock().unwrap().insert(id, (out, caller));
    }
    pub(super) fn remove(&self, id: u64) {
        self.map.lock().unwrap().remove(&id);
    }
    /// How many authenticated clients are connected now.
    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }
    pub fn registry(&self) -> Arc<dyn Registry> {
        self.registry.clone()
    }
}

/// The engine's `Host`: an event is one frame to every authenticated socket —
/// PROJECTED to that socket's caller (`scope::event`, §3.3 gap 12). The grant
/// is read at the moment of each emit, never a copy taken at connect, so a
/// narrowed scope stops the events with the next one. Fire-and-forget by
/// construction — a dead queue is dropped from the map on the way past, and
/// nothing here waits on a client.
pub struct Broadcast(pub Arc<Clients>);

impl Host for Broadcast {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let whole = wire::event_frame(event, &payload);
        let clients = &self.0;
        let mut map = clients.map.lock().unwrap();
        map.retain(|_, (out, caller)| {
            let grant = authz::effective(&*clients.registry, caller);
            match scope::event(event, &payload, caller, &grant) {
                Some(p) if p == payload => out.push(whole.clone()).is_ok(),
                Some(p) => out.push(wire::event_frame(event, &p)).is_ok(),
                None => true,
            }
        });
    }

    /// Somebody at the HOST is connected. What waits on an audience is the
    /// inbox's `app:open-doc`, which only the console receives (§9.1), so a
    /// remote device alone must not drain it into nothing. A connection joins
    /// the map once it has proven itself and leaves it as it goes.
    fn has_audience(&self) -> bool {
        self.0.map.lock().unwrap().values().any(|(_, c)| c.is_console())
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// One connection's serial lanes. A request that names a stream
/// (`dispatch::lane`) is queued on that stream's lane and runs after the
/// request before it: each lane awaits its job before taking the next, so the
/// order is a property of the code, and an idle lane holds no thread — a
/// parked task and an empty queue, for the life of the connection.
#[derive(Default)]
struct Lanes(Mutex<HashMap<String, UnboundedSender<Job>>>);

impl Lanes {
    fn run(&self, key: String, job: Job) {
        let mut map = self.0.lock().unwrap();
        let tx = map.entry(key).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
            tokio::spawn(async move {
                while let Some(job) = rx.recv().await {
                    // A panicking body is that request's failure, not the lane's.
                    let _ = tokio::task::spawn_blocking(job).await;
                }
            });
            tx
        });
        // The receiver lives in the lane task, which lives while this sender
        // is in the map; a send can only fail while the runtime is going down.
        let _ = tx.send(job);
    }
}

/// Everything a connection is answered from. One per server.
pub struct Shared {
    pub ctx: Ctx,
    pub token: Arc<str>,
    pub port: u16,
    pub origins: Vec<String>,
    pub clients: Arc<Clients>,
    next_id: AtomicU64,
    connections: Arc<Counter>,
    unauthenticated: Arc<Counter>,
    /// At a cap right now, and how many were turned away since it began: the
    /// log says so once per episode, not once per connection, or a flood
    /// would become a disk-writing flood.
    capped: AtomicBool,
    turned_away: AtomicU64,
}

impl Shared {
    /// A slot for a new connection, or `None` past either cap.
    fn admit(&self) -> Option<Arc<Slot>> {
        let connection = self.connections.try_hold();
        let unauthenticated = connection.as_ref().and_then(|_| self.unauthenticated.try_hold());
        match (connection, unauthenticated) {
            (Some(c), Some(u)) => {
                if self.capped.swap(false, Ordering::SeqCst) {
                    let n = self.turned_away.swap(0, Ordering::SeqCst);
                    info(&format!("serve: below the connection caps again; {n} connection(s) were closed unanswered meanwhile"));
                }
                Some(Arc::new(Slot { _connection: c, unauthenticated: Mutex::new(Some(u)) }))
            }
            (c, _) => {
                let which = if c.is_none() { "connections" } else { "unauthenticated connections" };
                self.turned_away.fetch_add(1, Ordering::SeqCst);
                if !self.capped.swap(true, Ordering::SeqCst) {
                    info(&format!(
                        "serve: at the cap on {which} ({} total, {} unauthenticated); closing new connections unanswered",
                        self.connections.cap, self.unauthenticated.cap
                    ));
                }
                None
            }
        }
    }
}

/// A running listener: its port and the accept task.
pub struct Listener {
    pub port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Listener {
    /// Stop accepting and release the port. Live connections are their own
    /// tasks and finish on their own; the engine behind them is `host::stop`'s.
    pub fn stop(&self) {
        self.task.abort();
    }
}

/// Bind `127.0.0.1:port` (0 = the OS picks) and start accepting. Must be
/// called inside a tokio runtime. Bound with `std` and handed over, as the
/// docs server does: the port exists before this returns, so `serve.json`
/// never names a port nothing is listening on.
pub fn start(ctx: Ctx, token: String, port: u16, origins: Vec<String>, clients: Arc<Clients>, limits: Limits) -> Result<Listener, String> {
    // 127.0.0.1 only, as a rule and not a default (brief: "you bind 127.0.0.1
    // and nothing else"). Not `localhost`, which may resolve to `::1` first
    // and would make the IPv4 address in serve.json a lie.
    let std_listener = StdListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .map_err(|e| format!("could not bind 127.0.0.1:{port}: {e}"))?;
    let port = std_listener.local_addr().map_err(|e| e.to_string())?.port();
    std_listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let listener = tokio::net::TcpListener::from_std(std_listener).map_err(|e| e.to_string())?;
    let shared = Arc::new(Shared {
        ctx,
        token: Arc::from(token.as_str()),
        port,
        origins,
        clients,
        next_id: AtomicU64::new(1),
        connections: Counter::new(limits.connections),
        unauthenticated: Counter::new(limits.unauthenticated.min(limits.connections)),
        capped: AtomicBool::new(false),
        turned_away: AtomicU64::new(0),
    });
    let task = tokio::spawn(async move {
        let mut said = false;
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => {
                    said = false;
                    v
                }
                Err(e) => {
                    if !said {
                        said = true;
                        worktrees_api::log::error(&format!("serve: accept failed: {e}"));
                    }
                    tokio::time::sleep(ACCEPT_RETRY).await;
                    continue;
                }
            };
            // The bind already guarantees this; asserted anyway, because it is
            // one comparison and the cost of being wrong is the machine.
            if !peer.ip().is_loopback() {
                continue;
            }
            // Past a cap the stream is dropped here: closed, nothing written.
            let Some(slot) = shared.admit() else { continue };
            tokio::spawn(connection(stream, shared.clone(), slot));
        }
    });
    Ok(Listener { port, task })
}

/// A refusal. Carries nothing, because nothing is written: hyper closes the
/// connection when the service errors, and that silence is the design.
#[derive(Debug)]
struct Refused(&'static str);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Refused {}

async fn connection(stream: TcpStream, shared: Arc<Shared>, slot: Arc<Slot>) {
    let io = TokioIo::new(stream);
    let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
        let (shared, slot) = (shared.clone(), slot.clone());
        async move { handshake(req, shared, slot) }
    });
    let mut http = hyper::server::conn::http1::Builder::new();
    http.max_buf_size(MAX_HTTP_BUF);
    let conn = http.serve_connection(io, svc).with_upgrades();
    // The timeout bounds the HTTP phase only: once upgraded, the socket
    // belongs to the session task spawned in `handshake`, and this future
    // has already resolved.
    match tokio::time::timeout(HANDSHAKE_DEADLINE, conn).await {
        Ok(Err(e)) if e.is_user() => {} // our own Refused, already logged
        Ok(Err(e)) => info(&format!("serve: connection ended: {e}")),
        Ok(Ok(())) | Err(_) => {}
    }
}

/// Exactly one readable value for `name`, `Ok(None)` for absent. Two values
/// is a request built to be read one way by one reader and another way by the
/// next; an unreadable value is a header we cannot judge. Both are refusals —
/// "I could not tell" and "it is ours" must never share a branch.
fn one_header(h: &HeaderMap, name: HeaderName) -> Result<Option<String>, Refused> {
    let mut it = h.get_all(&name).iter();
    let Some(first) = it.next() else { return Ok(None) };
    if it.next().is_some() {
        return Err(Refused("duplicate header"));
    }
    first.to_str().map(|s| Some(s.to_string())).map_err(|_| Refused("unreadable header"))
}

/// The policy, then the 101. Refusals are logged on the server side only.
fn handshake(mut req: Request<Incoming>, shared: Arc<Shared>, slot: Arc<Slot>) -> Result<Response<Empty<Bytes>>, Refused> {
    let refuse = |why: &'static str| {
        info(&format!("serve: refused a connection: {why}"));
        Refused(why)
    };
    let h = req.headers();
    let host = one_header(h, HOST).map_err(|e| refuse(e.0))?;
    if !policy::host_ok(host.as_deref(), shared.port) {
        return Err(refuse("host"));
    }
    let origin = one_header(h, ORIGIN).map_err(|e| refuse(e.0))?;
    if !policy::origin_ok(origin.as_deref(), &shared.origins) {
        return Err(refuse("origin"));
    }
    let upgrade = one_header(h, UPGRADE).map_err(|e| refuse(e.0))?;
    let connection = one_header(h, CONNECTION).map_err(|e| refuse(e.0))?;
    let version = one_header(h, SEC_WEBSOCKET_VERSION).map_err(|e| refuse(e.0))?;
    let key = one_header(h, SEC_WEBSOCKET_KEY).map_err(|e| refuse(e.0))?;
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
    let origin = origin.unwrap_or_default();

    let on_upgrade = hyper::upgrade::on(&mut req);
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let budget = Arc::new(AtomicUsize::new(PREAUTH_BYTES));
                let io = Budget { inner: TokioIo::new(upgraded), left: budget.clone() };
                let config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
                let ws = WebSocketStream::from_raw_socket(io, Role::Server, Some(config)).await;
                session(ws, shared, id, origin, slot, budget).await;
            }
            Err(e) => warn(&format!("serve: upgrade failed after 101: {e}")),
        }
    });
    Ok(Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .body(Empty::new())
        .expect("a 101 with three fixed headers is a valid response"))
}

type Ws = WebSocketStream<Budget<TokioIo<hyper::upgrade::Upgraded>>>;

/// The next frame, if it is text. Anything else — binary, a ping, a close,
/// an error, the end — is `None`, and `None` before auth is a refusal.
async fn next_text(ws: &mut Ws) -> Option<String> {
    match ws.next().await {
        Some(Ok(Message::Text(t))) => Some(t.as_str().to_string()),
        _ => None,
    }
}

/// Handshake v1 (`policy.rs`), the whole of it inside `AUTH_DEADLINE`. `None`
/// is a refusal, and the caller drops the socket with nothing more written:
/// the challenge is the only thing an unproven peer ever receives.
async fn handshake_v1(ws: &mut Ws, token: &str) -> Option<()> {
    let cn = policy::hello(&next_text(ws).await?)?;
    let sn = servefile::nonce();
    ws.send(Message::text(wire::challenge_frame(&sn))).await.ok()?;
    let proof = policy::client_proof(&next_text(ws).await?)?;
    if !policy::client_proof_ok(token, &cn, &sn, &proof) {
        return None;
    }
    // Proven: only now does the server prove itself.
    ws.send(Message::text(wire::auth_reply(&policy::server_proof(token, &cn, &sn)))).await.ok()
}

/// One authenticated session: the handshake first, then requests until the
/// client goes.
async fn session(mut ws: Ws, shared: Arc<Shared>, id: u64, origin: String, slot: Arc<Slot>, budget: Arc<AtomicUsize>) {
    // The deadline is on the WHOLE exchange, not per frame: two quick steps
    // adding up to more than it are refused like silence is. A refusal drops
    // the socket here with nothing more written, not even a Close frame.
    let authed = tokio::time::timeout(policy::AUTH_DEADLINE, handshake_v1(&mut ws, &shared.token)).await;
    if !matches!(authed, Ok(Some(()))) {
        info(&format!("serve: client {id} did not authenticate ({origin})"));
        return;
    }
    // In: the unauthenticated slot goes back, and the byte budget lifts to
    // the codec's own MAX_MESSAGE. The connection slot is held until the end.
    slot.authenticated();
    budget.store(usize::MAX, Ordering::SeqCst);
    info(&format!("serve: client {id} connected ({origin})"));

    let (sink, mut stream) = ws.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let outbox = Outbox(tx);
    // Who this socket is, fixed for its life: THIS listener's caller, the
    // owner on the console. Never derived from the request, a header or the
    // peer address (§3.3); the remote listener (slice 6) sets the device.
    let caller = Caller::console();
    shared.clients.add(id, outbox.clone(), caller.clone());
    // THE writer: one task drains the one queue, so every frame for this
    // connection is written in the order it was queued.
    let writer = tokio::spawn(async move {
        let mut sink = sink;
        while let Some(m) = rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
    });
    let frames = Frames::new(outbox.clone());
    let call = Arc::new(Call {
        ctx: shared.ctx.clone(),
        frames: frames.clone(),
        caller,
        registry: shared.clients.registry(),
        world: shared.clients.world.clone(),
    });
    let lanes = Lanes::default();

    while let Some(m) = stream.next().await {
        match m {
            Ok(Message::Text(t)) => {
                // Parsed here, on the reader, because the lane is chosen from
                // the request; a frame is small and the parse is cheap.
                let req = match wire::parse_request(t.as_str()) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = outbox.push(wire::reply_err(&Value::Null, e.into()));
                        continue;
                    }
                };
                let wire::Request { id, cmd, args } = req;
                let args = Args(args);
                let lane = dispatch::lane(&cmd, &args);
                // The bodies are blocking (git, tmux, the disk), so each runs
                // on the blocking pool and its reply is queued when it is
                // done. Concurrently, as Tauri runs its async commands — a
                // clone that takes minutes must not hold up the keystroke
                // behind it — EXCEPT along one stream, where the keystroke
                // behind a keystroke must stay behind it: those go down the
                // stream's lane, one at a time.
                let call = call.clone();
                let outbox = outbox.clone();
                let job: Job = Box::new(move || {
                    let _ = outbox.push(answer(&call, &id, &cmd, &args));
                });
                match lane {
                    Some(key) => lanes.run(key, job),
                    None => {
                        tokio::task::spawn_blocking(job);
                    }
                }
            }
            // tungstenite queues a Pong on its own when the write half next
            // flushes; queuing one explicitly means it goes out even when
            // nothing else is being written.
            Ok(Message::Ping(p)) => {
                let _ = outbox.send(Message::Pong(p));
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Binary(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
        }
    }

    shared.clients.remove(id);
    writer.abort();
    // The tmux attaches this client opened go with it: each is a tmux client
    // on a session, and a client nobody is reading is a client tmux is still
    // painting for. Dock shells stay — they are the process's, and the next
    // attach replays them.
    let terms = frames.take_terms();
    for t in &terms {
        let _ = worktrees_api::term::term_close(*t, &shared.ctx.terminals);
    }
    info(&format!("serve: client {id} disconnected ({} attach(es) closed)", terms.len()));
}

/// One parsed request → one reply frame, whatever its body does.
///
/// A body that PANICS is answered as well. Before, it never was: tokio keeps
/// the panic inside the blocking task, so the connection, its lane and the
/// server all carried on and the client waited on that id forever. Now the
/// client gets `internal error` and nothing of the panic, and the log gets
/// one line naming the command, beside the panic hook's own entry. A command
/// that panics is always a row of the table, since an unknown one is an
/// `Err` before any body runs, so `cmd` is safe to log as it is.
/// `AssertUnwindSafe`, because what a body shares is the engine's state
/// behind its mutexes, and a lock the panic poisoned reports itself.
fn answer(call: &Call, id: &Value, cmd: &str, args: &Args) -> String {
    // Console acts and `admin` rows change this machine's user-tier state; a
    // line per call is the audit trail until slice 3's log replaces it.
    if dispatch::exposure(cmd) == Some(dispatch::Exposure::HostConsole) || dispatch::cap_of(cmd) == Some(Capability::Admin) {
        info(&format!("serve: host-admin {cmd}"));
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch::call(call, cmd, args))) {
        Ok(Ok(v)) => wire::reply_ok(id, v),
        Ok(Err(e)) => wire::reply_err(id, e),
        Err(p) => {
            error(&format!("serve: {cmd} panicked: {}; answered {:?}", worktrees_api::log::panic_text(&*p), wire::INTERNAL_ERROR));
            wire::reply_err(id, Value::from(wire::INTERNAL_ERROR))
        }
    }
}
