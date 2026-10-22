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
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::WebSocketStream;
use worktrees_api::log::{error, info, warn};
use worktrees_api::{Ctx, Host};

use super::audit::{Audit, ConnAudit, Outcome};
use super::devices::{DeviceEvent, DeviceRegistry};
use super::dispatch::{self, Args, Call};
use super::policy;
use super::resolve::{self, World};
use super::scope::{self, Listens};
use super::servefile;
use super::wire::{self, Frames, Outbox};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use worktrees_core::authz::{self, Caller, Device, DeviceId, NoDevices, Registry};

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

/// One authenticated connection, as the client set holds it.
struct Conn {
    out: Outbox,
    caller: Caller,
    /// The events it declared it listens for (§9.3): what a one-shot event
    /// and `Host::has_audience` ask. The session's handle is the same set.
    listens: Listens,
    /// What the session answers with — its frames (the attaches, dock shells
    /// and jobs it owns) and its `Ctx`. `None` for a connection added without
    /// a session (the broadcast tests).
    call: Option<Arc<Call>>,
    /// Ends the session: a revoke or a disable closes the device's
    /// connections at once (§2.5).
    kill: Arc<Notify>,
}

/// Every authenticated connection's queue, caller and declared listeners,
/// for broadcasts, and where every caller's grant is read. It is also what a
/// REDUCTION acts on (§3.3): it hears the device registry's events
/// (`Clients::hear`) and closes a revoked or disabled device's connections,
/// or ends each channel a narrowed grant no longer covers — the per-request
/// check alone would let an attach that streams unasked run on.
pub struct Clients {
    map: Mutex<HashMap<u64, Conn>>,
    registry: Arc<dyn Registry>,
    /// The registry the device rows change. In serve it IS `registry`; with a
    /// fixture `registry` (the tests) it is an empty one in memory.
    devices: Arc<DeviceRegistry>,
    world: Option<Arc<dyn World>>,
}

impl Default for Clients {
    /// No paired devices: the console is the only caller with a grant.
    /// Objects resolve against the live registry and tmux.
    fn default() -> Self {
        Clients::with(Arc::new(NoDevices), None)
    }
}

impl Clients {
    /// Grants from `registry` (a fixture, in the tests); `world: None` is the
    /// live one.
    pub fn with(registry: Arc<dyn Registry>, world: Option<Arc<dyn World>>) -> Self {
        Clients { map: Mutex::new(HashMap::new()), registry, devices: Arc::new(DeviceRegistry::in_memory()), world }
    }
    /// Grants from the device registry itself, as serve runs.
    pub fn serving(devices: Arc<DeviceRegistry>, world: Option<Arc<dyn World>>) -> Self {
        Clients { map: Mutex::new(HashMap::new()), registry: devices.clone(), devices, world }
    }
    /// Subscribe to the device registry's events. Weakly: the registry's hook
    /// list must not keep the client set alive.
    pub fn listen(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        self.devices.subscribe(Arc::new(move |e: &DeviceEvent| {
            if let Some(c) = me.upgrade() {
                c.hear(e);
            }
        }));
    }
    /// A connection with no session joins, having declared nothing. The
    /// handle is the set its listen frames would replace.
    #[cfg(test)]
    pub(super) fn add(&self, id: u64, out: Outbox, caller: Caller) -> Listens {
        let listens = Listens::default();
        let conn = Conn { out, caller, listens: listens.clone(), call: None, kill: Arc::new(Notify::new()) };
        self.map.lock().unwrap().insert(id, conn);
        listens
    }
    /// A session joins, having declared nothing: its queue, and the `Call` it
    /// answers with. The handles are what ends it and the set its listen
    /// frames replace.
    pub(super) fn join(&self, id: u64, out: Outbox, call: Arc<Call>) -> (Arc<Notify>, Listens) {
        let kill = Arc::new(Notify::new());
        let listens = Listens::default();
        let caller = call.caller.clone();
        let conn = Conn { out, caller, listens: listens.clone(), call: Some(call), kill: kill.clone() };
        self.map.lock().unwrap().insert(id, conn);
        (kill, listens)
    }
    pub(super) fn remove(&self, id: u64) {
        self.map.lock().unwrap().remove(&id);
    }
    pub fn devices(&self) -> Arc<DeviceRegistry> {
        self.devices.clone()
    }
    /// How many authenticated clients are connected now.
    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }
    pub fn registry(&self) -> Arc<dyn Registry> {
        self.registry.clone()
    }
    /// The registered roots an event's paths are attributed by: the world's,
    /// or the registry file's.
    fn roots(&self) -> Vec<String> {
        match &self.world {
            Some(w) => w.roots(),
            None => worktrees_core::registry::read_lenient().roots(),
        }
    }

    /// One registry event (§2.5, §3.3): a revoke or a disable closes the
    /// device's connections; a reduction ends the channels it no longer
    /// covers; and the console hears it, for the Devices list and the alerts
    /// (§5.3) — `devices:changed`, which reaches no device (`scope::EVENTS`).
    pub fn hear(self: &Arc<Self>, e: &DeviceEvent) {
        if let Some(d) = e.ends() {
            let why = if matches!(e, DeviceEvent::Revoked { .. }) { "revoked" } else { "disabled" };
            self.end_device(d, why);
        }
        if let Some(d) = e.reduces() {
            self.reconcile_device(d);
        }
        Broadcast(self.clone()).emit("devices:changed", serde_json::to_value(e).unwrap_or(Value::Null));
    }

    /// Close every connection of device `d`, now (§2.5): out of the map, so no
    /// event reaches it; its clone jobs cancelled and its attaches closed on
    /// this thread, so nothing streams on while the session notices; a close
    /// frame saying why; then the session ends. A body already running
    /// finishes, and its reply goes nowhere. The ids closed.
    pub fn end_device(&self, d: &DeviceId, why: &'static str) -> Vec<u64> {
        let ended: Vec<(u64, Conn)> = {
            let mut map = self.map.lock().unwrap();
            let ids: Vec<u64> = map.iter().filter(|(_, c)| is_device(&c.caller, d)).map(|(id, _)| *id).collect();
            ids.into_iter().filter_map(|id| map.remove(&id).map(|c| (id, c))).collect()
        };
        for (_, c) in &ended {
            if let Some(call) = &c.call {
                for job in call.frames.take_jobs() {
                    let _ = worktrees_api::workspace::clone_cancel(job);
                }
                for t in call.frames.take_terms() {
                    let _ = worktrees_api::term::term_close(t, &call.ctx.terminals);
                }
                release_shells(call);
            }
            let _ = c.out.send(Message::Close(Some(CloseFrame { code: CloseCode::Policy, reason: why.into() })));
            c.kill.notify_one();
        }
        if !ended.is_empty() {
            info(&format!("serve: device {d} {why}: {} connection(s) closed", ended.len()));
        }
        ended.into_iter().map(|(id, _)| id).collect()
    }

    /// Re-check every open channel of device `d`'s connections against its
    /// grant as it stands (§3.3).
    pub fn reconcile_device(&self, d: &DeviceId) {
        let calls: Vec<Arc<Call>> =
            self.map.lock().unwrap().values().filter(|c| is_device(&c.caller, d)).filter_map(|c| c.call.clone()).collect();
        for call in calls {
            reconcile(&call);
        }
    }
}

fn is_device(c: &Caller, d: &DeviceId) -> bool {
    matches!(c.device(), Device::Paired(x) if x == d)
}

/// Let go of every dock-shell sink this connection holds: the shells stay
/// (they are the process's), and a sink nobody reads is let go now rather
/// than at the next attach.
fn release_shells(call: &Call) {
    for h in call.frames.take_shells() {
        let _ = worktrees_api::term::shell_detach(h.repo, h.slug, h.index, h.gen, &call.ctx.shells);
    }
}

/// End each channel of this connection that its caller's grant, as it stands
/// now, no longer covers (§3.3): an attach whose session is out of scope or
/// whose `stream` is gone is closed, a dock shell's sink let go — each ends
/// with `{channel, seq, end: true}` as its sink drops. The check is the
/// resolver's own, for the row that opened the channel, so "covers" means
/// exactly what it meant at the open. Step-up is not asked again: it gates
/// OPENING a stream, and a proof that lapsed ends nothing already open. The
/// console's grant is everything and is never reconciled.
pub fn reconcile(call: &Call) {
    if call.caller.is_console() {
        return;
    }
    call.frames.reconciled().store(call.registry.generation(), Ordering::SeqCst);
    let grant = authz::effective(&*call.registry, &call.caller);
    let check = |cmd: &str, args: Value, world: &dyn World| {
        let row = dispatch::row(cmd).expect("a stream row");
        let mut a = Args(args.as_object().cloned().unwrap_or_default());
        resolve::authorize(row, call, &grant, &mut a, world).is_ok()
    };
    let mut ended = 0;
    dispatch::with_world(call, |world| {
        for (id, session) in call.frames.terms() {
            if !check("term_open", serde_json::json!({ "session": session }), world) {
                call.frames.forget_term(id);
                let _ = worktrees_api::term::term_close(id, &call.ctx.terminals);
                ended += 1;
            }
        }
        for h in call.frames.shells() {
            if !check("shell_open", serde_json::json!({ "repo": h.repo, "slug": h.slug, "index": h.index }), world) {
                call.frames.forget_shell(&h.repo, &h.slug, h.index);
                let _ = worktrees_api::term::shell_detach(h.repo.clone(), h.slug.clone(), h.index, h.gen, &call.ctx.shells);
                ended += 1;
            }
        }
    });
    if ended > 0 {
        info(&format!("serve: {} narrowed: {ended} channel(s) it no longer covers ended", call.caller));
    }
}

/// The engine's `Host`: an event is one frame to every authenticated socket —
/// PROJECTED to that socket's caller (`scope::event`, §3.3 gap 12). The grant
/// is read at the moment of each emit, never a copy taken at connect, so a
/// narrowed scope stops the events with the next one. A one-shot event goes
/// only to the sockets that declared it (§9.3). Fire-and-forget by
/// construction — a dead queue is dropped from the map on the way past, and
/// nothing here waits on a client.
pub struct Broadcast(pub Arc<Clients>);

impl Host for Broadcast {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let whole = wire::event_frame(event, &payload);
        let clients = &self.0;
        // Read once per emit, and only if a scoped caller needs it.
        let mut roots: Option<Vec<String>> = None;
        let mut map = clients.map.lock().unwrap();
        map.retain(|_, c| {
            if scope::is_one_shot(event) && !c.listens.has(event) {
                return true;
            }
            let grant = authz::effective(&*clients.registry, &c.caller);
            let mut read = || roots.get_or_insert_with(|| clients.roots()).clone();
            match scope::event(event, &payload, &c.caller, &grant, &mut read) {
                Some(p) if p == payload => c.out.push(whole.clone()).is_ok(),
                Some(p) => c.out.push(wire::event_frame(event, &p)).is_ok(),
                None => true,
            }
        });
    }

    /// Somebody at the HOST listens for `event`: a loopback (console)
    /// connection that declared it. What waits on an audience is the inbox's
    /// `app:open-doc`, which only the console receives (§9.1), so a remote
    /// device alone must not drain it into nothing — nor may a console
    /// connection that never said it listens, which is what the app's
    /// verification probe is: it authenticates and closes at once, and a
    /// show released to it was lost (§9.3). A connection joins the map once it
    /// has proven itself, having declared nothing, and leaves it as it goes.
    fn has_audience(&self, event: &str) -> bool {
        self.0.map.lock().unwrap().values().any(|c| c.caller.is_console() && c.listens.has(event))
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
    /// The audit log's one writer (`audit.rs`). Here and not in `Ctx`, so
    /// that no command body can write a line.
    audit: Arc<Audit>,
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
                    self.audit.limit_cleared();
                }
                Some(Arc::new(Slot { _connection: c, unauthenticated: Mutex::new(Some(u)) }))
            }
            (c, _) => {
                let which = if c.is_none() { "connections" } else { "unauthenticated connections" };
                self.turned_away.fetch_add(1, Ordering::SeqCst);
                self.audit.limit_reached(if c.is_none() { "connections" } else { "unauthenticated" });
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
    audit: Arc<Audit>,
}

impl Listener {
    /// Stop accepting and release the port. Live connections are their own
    /// tasks and finish on their own; the engine behind them is `host::stop`'s.
    pub fn stop(&self) {
        self.task.abort();
        self.audit.stopped();
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
    // The audit log opens with the port bound, so its first record names it.
    let (audit, chain) = Audit::open(&ctx.config_dir, Some(ctx.host.clone()));
    audit.started(port, &chain);
    let shared = Arc::new(Shared {
        ctx,
        token: Arc::from(token.as_str()),
        port,
        origins,
        clients,
        audit: audit.clone(),
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
    Ok(Listener { port, task, audit })
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
    // The peer, for the records written before there is an actor to name.
    let peer = stream.peer_addr().ok().map(|a| a.ip().to_string());
    let audit = shared.audit.clone();
    let io = TokioIo::new(stream);
    let p = peer.clone();
    let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
        let (shared, slot, peer) = (shared.clone(), slot.clone(), p.clone());
        async move { handshake(req, shared, slot, peer) }
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
        Ok(Ok(())) => {}
        Err(_) => audit.upgrade_refused("deadline", peer),
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
fn handshake(mut req: Request<Incoming>, shared: Arc<Shared>, slot: Arc<Slot>, peer: Option<String>) -> Result<Response<Empty<Bytes>>, Refused> {
    let refuse = |why: &'static str| {
        info(&format!("serve: refused a connection: {why}"));
        shared.audit.upgrade_refused(why, peer.clone());
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
                session(ws, shared, id, origin, slot, budget, peer).await;
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

/// One session on the loopback listener: handshake v1 first, then the
/// console's requests until the client goes.
async fn session(mut ws: Ws, shared: Arc<Shared>, id: u64, origin: String, slot: Arc<Slot>, budget: Arc<AtomicUsize>, peer: Option<String>) {
    // The deadline is on the WHOLE exchange, not per frame: two quick steps
    // adding up to more than it are refused like silence is. A refusal drops
    // the socket here with nothing more written, not even a Close frame.
    let authed = tokio::time::timeout(policy::AUTH_DEADLINE, handshake_v1(&mut ws, &shared.token)).await;
    if !matches!(authed, Ok(Some(()))) {
        info(&format!("serve: client {id} did not authenticate ({origin})"));
        shared.audit.auth_failed(if authed.is_err() { "deadline" } else { "handshake" }, id, peer);
        return;
    }
    // In: the unauthenticated slot goes back, and the byte budget lifts to
    // the codec's own MAX_MESSAGE. The connection slot is held until the end.
    slot.authenticated();
    budget.store(usize::MAX, Ordering::SeqCst);
    info(&format!("serve: client {id} connected ({origin})"));
    // Who this socket is, fixed for its life: THIS listener's caller, the
    // owner on the console. Never derived from the request, a header or the
    // peer address (§3.3); the remote listener (slice 6) sets the device.
    // Every audit record the connection writes names it (§5.1).
    let caller = Caller::console();
    let au = Arc::new(ConnAudit::new(shared.audit.clone(), id, peer, caller.clone()));
    serve_caller(ws, shared.ctx.clone(), shared.clients.clone(), id, caller, au).await;
}

/// An authenticated connection's life, whoever accepted it: requests until
/// the client goes or its device is revoked or disabled (§2.5). `caller` is
/// the listener's, fixed here for the life of the socket — the console for
/// loopback; for the remote listener (slice 6), the device whose key completed
/// the handshake — and `au` is the audit's view of the connection, naming
/// it. The in-process tests drive it with fake device callers.
pub(super) async fn serve_caller<S>(ws: WebSocketStream<S>, ctx: Ctx, clients: Arc<Clients>, id: u64, caller: Caller, au: Arc<ConnAudit>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    use futures_util::future::{select, Either};
    let (sink, mut stream) = ws.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let outbox = Outbox(tx);
    // THE writer: one task drains the one queue, so every frame for this
    // connection is written in the order it was queued.
    let mut writer = tokio::spawn(async move {
        let mut sink = sink;
        while let Some(m) = rx.recv().await {
            let close = matches!(m, Message::Close(_));
            if sink.send(m).await.is_err() || close {
                break;
            }
        }
    });
    let frames = Frames::new(outbox.clone());
    let call = Arc::new(Call {
        ctx: ctx.clone(),
        frames: frames.clone(),
        caller,
        registry: clients.registry(),
        world: clients.world.clone(),
        devices: clients.devices(),
    });
    frames.reconciled().store(call.registry.generation(), Ordering::SeqCst);
    let (kill, listens) = clients.join(id, outbox.clone(), call.clone());
    let lanes = Lanes::default();

    let mut killed = false;
    loop {
        let next = std::pin::pin!(stream.next());
        let stop = std::pin::pin!(kill.notified());
        let m = match select(next, stop).await {
            Either::Left((Some(m), _)) => m,
            Either::Left((None, _)) => break,
            Either::Right(_) => {
                killed = true;
                break;
            }
        };
        match m {
            Ok(Message::Text(t)) => {
                // Parsed here, on the reader, because the lane is chosen from
                // the request; a frame is small and the parse is cheap.
                let req = match wire::parse_frame(t.as_str()) {
                    Ok(wire::Frame::Request(r)) => r,
                    // Taken here, in order with the requests around it, and
                    // answered by nothing. What it costs is bounded by
                    // construction: `Listens` keeps one bit per known event.
                    Ok(wire::Frame::Listen(names)) => {
                        if let Some(kept) = listens.declare(names.iter().map(String::as_str)) {
                            info(&format!("serve: client {id} listens for [{}]", kept.join(", ")));
                        }
                        continue;
                    }
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
                let au = au.clone();
                let job: Job = Box::new(move || {
                    let _ = outbox.push(answer(&call, &au, &id, &cmd, &args));
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
            // A message past MAX_MESSAGE ends the connection: a limit tripped.
            Err(WsError::Capacity(_)) => {
                au.limit("message_size");
                break;
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Binary(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
        }
    }

    clients.remove(id);
    if killed {
        // `end_device` queued a close frame saying why; give the writer a
        // moment to put it on the wire before the socket goes.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(250), &mut writer).await;
    }
    writer.abort();
    // The tmux attaches this client opened go with it: each is a tmux client
    // on a session, and a client nobody is reading is a client tmux is still
    // painting for. Dock shells stay — they are the process's, and the next
    // attach replays them — but this connection's sinks on them are let go.
    let terms = frames.take_terms();
    for t in &terms {
        let _ = worktrees_api::term::term_close(*t, &ctx.terminals);
    }
    release_shells(&call);
    info(&format!("serve: client {id} {} ({} attach(es) closed)", if killed { "closed by serve" } else { "disconnected" }, terms.len()));
    au.disconnected();
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
///
/// The audit record (§5.1) is taken by the request's `audit::Seen`, which
/// dispatch hands the resolver's verdict: a refusal is recorded where it is
/// made, the stale close answered `ok` included, and an admitted row whose
/// capability is not `read` is recorded by `finish`, with its outcome, once
/// the body is done. The host-admin line serve.log used to carry is one of
/// those records now.
pub(super) fn answer(call: &Call, au: &ConnAudit, id: &Value, cmd: &str, args: &Args) -> String {
    // A registry change this connection has not been checked against yet
    // (§3.3): one atomic load when nothing moved. The registry's own events
    // reconcile at once; this is the backstop for a change that came by
    // another way.
    let device = !call.caller.is_console();
    if device && call.frames.reconciled().load(Ordering::SeqCst) != call.registry.generation() {
        reconcile(call);
    }
    let authorized_at = call.registry.generation();
    let reply = answer_now(call, au, id, cmd, args);
    // A change that landed WHILE this request ran — after it was authorized,
    // perhaps before the channel it opened was noted, so that change's own
    // reconcile could not see it — is checked now: a stream opened under a
    // grant that has since narrowed must not outlive it (§3.3).
    if device && call.registry.generation() != authorized_at {
        reconcile(call);
    }
    reply
}

fn answer_now(call: &Call, au: &ConnAudit, id: &Value, cmd: &str, args: &Args) -> String {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let seen = au.request();
    let r = catch_unwind(AssertUnwindSafe(|| dispatch::call_seen(call, cmd, args, &seen)));
    // The audit's own code is kept out of the reply's way as well: a panic in
    // it loses that record (fail open, as a failed write does), never the
    // reply and never the body.
    let outcome = match &r {
        Ok(Ok(v)) => Outcome::Ok(v),
        Ok(Err(_)) => Outcome::Err,
        Err(_) => Outcome::Panic,
    };
    let _ = catch_unwind(AssertUnwindSafe(|| seen.finish(outcome)));
    match r {
        Ok(Ok(v)) => wire::reply_ok(id, v),
        Ok(Err(e)) => wire::reply_err(id, e),
        Err(p) => {
            error(&format!("serve: {cmd} panicked: {}; answered {:?}", worktrees_api::log::panic_text(&*p), wire::INTERNAL_ERROR));
            wire::reply_err(id, Value::from(wire::INTERNAL_ERROR))
        }
    }
}
