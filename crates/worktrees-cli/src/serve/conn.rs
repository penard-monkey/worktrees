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
use std::sync::{Arc, Mutex, Weak};
use std::task::{ready, Context, Poll};
use std::time::Instant;

use futures_util::future::{select, Either};
use futures_util::{SinkExt, StreamExt};
use http_body_util::Empty;
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderMap, HeaderName, CONNECTION, HOST, ORIGIN, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::WebSocketStream;
use worktrees_api::log::{error, info, warn};
use worktrees_api::{Ctx, Host};

use super::audit::{Audit, ConnAudit, Fleet, Outcome};
use super::budget::{self, Budgets, Counter, Held, BUSY};
use super::devices::{DeviceEvent, DeviceRegistry};
use super::pairing::Pairing;
use super::dispatch::{self, Args, Call};
use super::flight::Flights;
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
// A flood can no longer take the memory, the descriptors, or a client
// already in. Nor can it hold the door shut: at the caps a NEW connection
// displaces the stalest unauthenticated one (`Shared::admit`) rather than
// being refused, which a reconnecting flood measured as 0 new clients of 49
// (Phase 2 §6). What an authenticated client may cost is `budget.rs`.

/// How long the HTTP half of a connection gets: the upgrade request, from
/// accept to the 101. A browser needs milliseconds; an idle socket holds a
/// task, a descriptor and an unauthenticated slot until this ends it. It was
/// the docs server's 20 s.
pub(crate) const HANDSHAKE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// How many connections may be open at once, and how many of those may be
/// unauthenticated (from accept until the client's proof verifies). At
/// either cap a new connection displaces the stalest unauthenticated one
/// (`Shared::admit`); with every connection authenticated it is closed at
/// once with nothing written.
///
/// The unauthenticated cap is how long a flood must keep up to displace a
/// real client: 256 connections inside one of that client's handshake
/// steps. Measured against a 32-thread reconnecting flood (~12,000 connects
/// a second on macOS), 16 slots gave a displaced client ~1.3 ms per step and
/// let 25 of 49 in; 256 give it ~20 ms. The cost is bounded: per connection
/// at most 64 KiB of request head, then an 8 KiB frame buffer and 512 bytes
/// of frames, so ≤ 18 MiB and 256 descriptors for a whole flood. The 64
/// beyond them are for authenticated clients, which only a token holder can
/// be.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub connections: usize,
    pub unauthenticated: usize,
}
pub const LIMITS: Limits = Limits { connections: 320, unauthenticated: 256 };

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

/// The frame codec's read buffer, per connection (`WebSocketConfig`).
const READ_BUFFER: usize = 8 * 1024;

/// hyper's buffer for the HTTP request, before any upgrade (its default is
/// ~400 KiB). Cookies are per host, not per port, so a browser may send those
/// of every other dev server on 127.0.0.1 — 64 KiB leaves room for them.
const MAX_HTTP_BUF: usize = 64 * 1024;

/// One connection's claim on the caps, from accept to close. The
/// unauthenticated half is handed back the moment the client's proof
/// verifies — or taken, both halves, by a newcomer that displaced this
/// connection as the stalest unauthenticated one (`Shared::admit`).
struct Slot {
    id: u64,
    /// (the connection, the unauthenticated slot).
    held: Mutex<(Option<Held>, Option<Held>)>,
    /// When the server began waiting on this connection's CLIENT — at its
    /// accept, once the session is up, once the challenge is out — in ms
    /// since the server started, or `SERVERS_TURN` while the next step is the
    /// server's own. The connection that has waited on its client longest is
    /// the one displaced, and only a connection whose client owes the next
    /// move can be: a real client answers each step within a round trip,
    /// while a peer without the token waits at the proof for good. The
    /// server's turn always ends by the server's own act, so a peer cannot
    /// hide there.
    progress: AtomicU64,
    /// `true` once displaced: it closes where it stands, nothing written.
    displaced: watch::Sender<bool>,
}

/// `Slot::progress` while the server, not the client, owes the next step.
const SERVERS_TURN: u64 = u64::MAX;

impl Slot {
    /// The client owes the next move, from now.
    fn clients_turn(&self, shared: &Shared) {
        self.progress.store(shared.now_ms(), Ordering::SeqCst);
    }

    /// The server owes the next move: out of reach until it has made it.
    fn servers_turn(&self) {
        self.progress.store(SERVERS_TURN, Ordering::SeqCst);
    }

    /// Resolves once this connection has been displaced.
    async fn displaced(&self) {
        let mut rx = self.displaced.subscribe();
        let _ = rx.wait_for(|d| *d).await;
    }

    /// The proof verified: out of the displaceable set, the unauthenticated
    /// slot back. `false` when it was displaced first — it must close.
    fn authenticated(&self, shared: &Shared) -> bool {
        let mut pending = shared.pending.lock().unwrap();
        if *self.displaced.borrow() {
            return false;
        }
        pending.remove(&self.id);
        self.held.lock().unwrap().1.take();
        true
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
    /// The pairing windows (§2.2), the console's to open.
    pairing: Arc<Pairing>,
    world: Option<Arc<dyn World>>,
    /// What authenticated clients may cost (`budget.rs`) and the reads they
    /// share (`flight.rs`): host-wide, so every listener's connections spend
    /// from one budget and join one set of flights.
    budget: budget::Budget,
    flights: Arc<Flights>,
    /// The sessions that have joined and made their post-join check: what a
    /// test waits for before an act whose close it times, so the time is the
    /// close's and not the session's start.
    #[cfg(test)]
    ready: Mutex<std::collections::HashSet<u64>>,
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
        let devices = Arc::new(DeviceRegistry::in_memory());
        let pairing = Arc::new(Pairing::new(None, devices.clone()));
        Clients {
            map: Mutex::new(HashMap::new()),
            registry,
            devices,
            pairing,
            world,
            budget: budget::Budget::new(budget::BUDGETS),
            flights: Default::default(),
            #[cfg(test)]
            ready: Default::default(),
        }
    }
    /// Grants from the device registry itself, as serve runs.
    pub fn serving(devices: Arc<DeviceRegistry>, pairing: Arc<Pairing>, world: Option<Arc<dyn World>>) -> Self {
        Clients {
            map: Mutex::new(HashMap::new()),
            registry: devices.clone(),
            devices,
            pairing,
            world,
            budget: budget::Budget::new(budget::BUDGETS),
            flights: Default::default(),
            #[cfg(test)]
            ready: Default::default(),
        }
    }
    /// The same set, spending from `budgets` instead of `budget::BUDGETS`.
    pub fn budgeted(mut self, budgets: Budgets) -> Self {
        self.budget = budget::Budget::new(budgets);
        self
    }
    pub fn pairing(&self) -> Arc<Pairing> {
        self.pairing.clone()
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
    /// How many connections, whoever they are, have declared `event`: what
    /// a device's listen frames did, which `has_audience` (the console's
    /// alone) cannot show.
    #[cfg(test)]
    pub fn listening(&self, event: &str) -> usize {
        self.map.lock().unwrap().values().filter(|c| c.listens.has(event)).count()
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
            let why = match e {
                DeviceEvent::Revoked { .. } => "revoked",
                DeviceEvent::Forgotten { .. } => "removed",
                _ => "disabled",
            };
            self.end_device(d, why);
        }
        if e.ends_every_device() {
            let ids: Vec<DeviceId> = {
                let map = self.map.lock().unwrap();
                let mut ids: Vec<DeviceId> =
                    map.values().filter_map(|c| match c.caller.device() { Device::Paired(d) => Some(d.clone()), Device::Console => None }).collect();
                ids.sort();
                ids.dedup();
                ids
            };
            for d in ids {
                self.end_device(&d, "unreadable");
            }
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

    /// Session `id` has joined and made its post-join check.
    #[cfg(test)]
    pub(super) fn is_ready(&self, id: u64) -> bool {
        self.ready.lock().unwrap().contains(&id)
    }

    /// Connection `id` is in the set: joined, and not closed since.
    #[cfg(test)]
    pub(super) fn contains(&self, id: u64) -> bool {
        self.map.lock().unwrap().contains_key(&id)
    }

    /// End `caller`'s device's connections now if the registry no longer
    /// serves it — revoked, disabled, removed, or the file unreadable — with
    /// the word its event would have sent. A session calls it once it has
    /// JOINED: an event that fired between the handshake and the join found
    /// no connection to close, and left a socket open that §2.5 says is dead.
    /// The registry commits a change before it hands its hooks the event, so
    /// every change either shows here or reaches the connection through
    /// [`Clients::hear`].
    pub(super) fn end_if_gone(&self, caller: &Caller) {
        let Device::Paired(d) = caller.device() else { return };
        if self.registry.device_grant(caller.principal(), d).is_none() {
            self.end_device(d, self.devices.ended_word(d));
        }
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
/// order is a property of the code.
///
/// A lane lives only while it has work. Before, an idle one stayed for the
/// connection's life — a parked task and a map entry per key the client ever
/// named, and `term_write` to 200,000 ids was 340 MB. Now the lane task,
/// finding its queue empty after a job, removes its own entry UNDER THE MAP
/// LOCK that every send takes, so no job can reach a lane that is leaving; the
/// next request for that key starts a fresh lane, after the old one's last job
/// has finished. Order holds across the gap. At most `cap` lanes are active at
/// once; a new key past that is `busy` (Phase 2 §6: 64).
struct Lanes {
    map: Mutex<HashMap<String, UnboundedSender<Job>>>,
    cap: usize,
    peak: AtomicUsize,
}

impl Lanes {
    fn new(cap: usize) -> Arc<Self> {
        Arc::new(Lanes { map: Mutex::new(HashMap::new()), cap, peak: AtomicUsize::new(0) })
    }

    /// Queue `job` on `key`'s lane, or hand it back when that would be one
    /// lane too many.
    fn run(self: &Arc<Self>, key: String, job: Job) -> Result<(), Job> {
        let mut map = self.map.lock().unwrap();
        if let Some(tx) = map.get(&key) {
            // An entry in the map has a live lane: the lane removes it, under
            // this lock, before it stops receiving. Failing means the runtime
            // is going down.
            return tx.send(job).map_err(|e| e.0);
        }
        if map.len() >= self.cap {
            return Err(job);
        }
        let (tx, rx) = mpsc::unbounded_channel::<Job>();
        let _ = tx.send(job);
        map.insert(key.clone(), tx);
        self.peak.fetch_max(map.len(), Ordering::SeqCst);
        tokio::spawn(self.clone().drain(key, rx));
        Ok(())
    }

    async fn drain(self: Arc<Self>, key: String, mut rx: UnboundedReceiver<Job>) {
        let mut next = rx.recv().await;
        while let Some(job) = next {
            // A panicking body is that request's failure, not the lane's.
            let _ = tokio::task::spawn_blocking(job).await;
            let mut map = self.map.lock().unwrap();
            next = rx.try_recv().ok();
            if next.is_none() {
                map.remove(&key);
            }
        }
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
    /// The connections that have not proven themselves yet, by slot id: the
    /// ones a newcomer may displace. Weak, so a connection that ended on its
    /// own is simply skipped.
    pending: Mutex<HashMap<u64, Weak<Slot>>>,
    next_slot: AtomicU64,
    started: Instant,
    /// At a cap right now, and how many were displaced or turned away since
    /// it began: the log says so once per episode, not once per connection,
    /// or a flood would become a disk-writing flood.
    capped: AtomicBool,
    displaced: AtomicU64,
    turned_away: AtomicU64,
    /// Test builds: the caller each next authenticated connection is, in
    /// order, so a test can be a paired DEVICE on a loopback socket. Empty
    /// means the console, as every connection is in a build for use.
    #[cfg(test)]
    pub test_callers: Mutex<std::collections::VecDeque<Caller>>,
}

impl Shared {
    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// A slot for a new connection. At either cap, the newcomer takes the
    /// slots of the STALEST unauthenticated connection — the one that has
    /// waited on its client longest (`Slot::progress`) — and that one closes
    /// with nothing written. `None` only when no connection can be displaced.
    ///
    /// Refusing the newcomer instead, as M1 did, let a flood that reconnects
    /// the instant it is closed keep every unauthenticated slot: measured, 0
    /// new clients of 49 on macOS and 0 of 50 on Linux. Displacement turns the
    /// question into a race a real client wins. It moves forward every
    /// millisecond (request, hello, proof), and a peer without the token
    /// stalls at the proof, so it is never the stalest for long.
    fn admit(&self) -> Option<Arc<Slot>> {
        let mut pending = self.pending.lock().unwrap();
        let connection = self.connections.try_hold();
        let unauthenticated = connection.as_ref().and_then(|_| self.unauthenticated.try_hold());
        let held = match (connection, unauthenticated) {
            (Some(c), Some(u)) => {
                // The episode ends once the flood has let go: half the
                // unauthenticated slots free again, not merely one, or a
                // flood would log an end and a start every second.
                if self.unauthenticated.held() <= self.unauthenticated.cap / 2 && self.capped.swap(false, Ordering::SeqCst) {
                    let d = self.displaced.swap(0, Ordering::SeqCst);
                    let n = self.turned_away.swap(0, Ordering::SeqCst);
                    info(&format!(
                        "serve: below the connection caps again; meanwhile {d} unauthenticated connection(s) were displaced \
                         and {n} new one(s) closed unanswered"
                    ));
                    self.audit.limit_cleared();
                }
                (Some(c), Some(u))
            }
            (c, _) => {
                // A newcomer at a cap is a limit tripped (§5.1), whether it
                // displaces or is turned away. The audit keeps the episode: the
                // first is a record, the rest a count in the one that ends it.
                self.audit.limit_reached(if c.is_none() { "connections" } else { "unauthenticated" });
                if !self.capped.swap(true, Ordering::SeqCst) {
                    info(&format!(
                        "serve: at the connection caps ({} total, {} unauthenticated); each new connection displaces the \
                         stalest unauthenticated one",
                        self.connections.cap, self.unauthenticated.cap
                    ));
                }
                pending.retain(|_, w| w.strong_count() > 0);
                let stalest = pending
                    .values()
                    .filter_map(Weak::upgrade)
                    .map(|s| (s.progress.load(Ordering::SeqCst), s))
                    .filter(|(p, _)| *p != SERVERS_TURN)
                    .min_by_key(|(p, _)| *p)
                    .map(|(_, s)| s);
                let Some(victim) = stalest else {
                    // Every connection is authenticated, or mid-step on the
                    // server's side: nobody to displace.
                    self.turned_away.fetch_add(1, Ordering::SeqCst);
                    return None;
                };
                pending.remove(&victim.id);
                let (vc, vu) = std::mem::take(&mut *victim.held.lock().unwrap());
                victim.displaced.send_replace(true);
                self.displaced.fetch_add(1, Ordering::SeqCst);
                // Our own connection slot when we got one (the victim's then
                // drops here), theirs when the total was the full one.
                (c.or(vc), vu)
            }
        };
        let id = self.next_slot.fetch_add(1, Ordering::SeqCst);
        let (displaced, _) = watch::channel(false);
        let slot = Arc::new(Slot { id, held: Mutex::new(held), progress: AtomicU64::new(self.now_ms()), displaced });
        pending.insert(id, Arc::downgrade(&slot));
        Some(slot)
    }

    /// How many unauthenticated connections newcomers have displaced so far
    /// in the current episode.
    #[cfg(test)]
    pub fn displaced(&self) -> u64 {
        self.displaced.load(Ordering::SeqCst)
    }

    /// Who an authenticated connection is: this listener's caller, the owner
    /// on the console — the one listener there is.
    fn caller(&self) -> Caller {
        #[cfg(test)]
        if let Some(c) = self.test_callers.lock().unwrap().pop_front() {
            return c;
        }
        Caller::console()
    }
}

/// A running listener: its port and the accept task.
pub struct Listener {
    pub port: u16,
    task: tokio::task::JoinHandle<()>,
    audit: Arc<Audit>,
    /// What the connections share, for the tests that look inside it.
    #[cfg(test)]
    pub shared: Arc<Shared>,
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
    // Its checkpoints carry the client set's devices, and every registry
    // event — a pairing, a revoke, a refused key — is one of its records.
    let (audit, chain) = Audit::open(&ctx.config_dir, Some(ctx.host.clone()), Fleet::of(&clients.devices()));
    audit.hear(&clients.devices());
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
        pending: Mutex::new(HashMap::new()),
        next_slot: AtomicU64::new(1),
        started: Instant::now(),
        capped: AtomicBool::new(false),
        displaced: AtomicU64::new(0),
        turned_away: AtomicU64::new(0),
        #[cfg(test)]
        test_callers: Mutex::new(Default::default()),
    });
    #[cfg(test)]
    let inside = shared.clone();
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
            // With every slot held by an authenticated client the stream is
            // dropped here: closed, nothing written.
            let Some(slot) = shared.admit() else { continue };
            tokio::spawn(connection(stream, shared.clone(), slot));
        }
    });
    Ok(Listener {
        port,
        task,
        audit,
        #[cfg(test)]
        shared: inside,
    })
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
    let watch = slot.clone();
    let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
        let (shared, slot, peer) = (shared.clone(), slot.clone(), p.clone());
        async move { handshake(req, shared, slot, peer) }
    });
    let mut http = hyper::server::conn::http1::Builder::new();
    http.max_buf_size(MAX_HTTP_BUF);
    let conn = http.serve_connection(io, svc).with_upgrades();
    // The timeout bounds the HTTP phase only: once upgraded, the socket
    // belongs to the session task spawned in `handshake`, and this future
    // has already resolved. Displaced meanwhile, the connection is dropped
    // where it stands, which closes it with nothing written; the audit
    // counted it where `admit` displaced it.
    let http = std::pin::pin!(tokio::time::timeout(HANDSHAKE_DEADLINE, conn));
    let gone = std::pin::pin!(watch.displaced());
    match select(http, gone).await {
        Either::Left((Ok(Err(e)), _)) if e.is_user() => {} // our own Refused, already logged
        Either::Left((Ok(Err(e)), _)) => info(&format!("serve: connection ended: {e}")),
        Either::Left((Ok(Ok(())), _)) | Either::Right(_) => {}
        Either::Left((Err(_), _)) => audit.upgrade_refused("deadline", peer),
    };
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
    // The request is in: the 101 and the upgrade are the server's to do.
    slot.servers_turn();
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
                // An 8 KiB read buffer, not tungstenite's 128 KiB: it is
                // allocated per connection up front, and 256 unproven ones
                // sitting at the proof held 73 MB with the default. A frame
                // larger than the buffer still reserves its whole length.
                let config = WebSocketConfig::default()
                    .max_message_size(Some(MAX_MESSAGE))
                    .max_frame_size(Some(MAX_MESSAGE))
                    .read_buffer_size(READ_BUFFER);
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
async fn handshake_v1(ws: &mut Ws, shared: &Shared, slot: &Slot) -> Option<()> {
    let token = &shared.token;
    slot.clients_turn(shared);
    let cn = policy::hello(&next_text(ws).await?)?;
    slot.servers_turn();
    let sn = servefile::nonce();
    ws.send(Message::text(wire::challenge_frame(&sn))).await.ok()?;
    slot.clients_turn(shared);
    let proof = policy::client_proof(&next_text(ws).await?)?;
    slot.servers_turn();
    if !policy::client_proof_ok(token, &cn, &sn, &proof) {
        return None;
    }
    // Proven, and out of reach of a newcomer from here on — unless one
    // displaced it a moment ago, in which case it closes like any other.
    if !slot.authenticated(shared) {
        return None;
    }
    // Only now does the server prove itself.
    ws.send(Message::text(wire::auth_reply(&policy::server_proof(token, &cn, &sn)))).await.ok()
}

/// The lane a request runs on: the table's rule, plus the test seams'.
fn lane_of(cmd: &str, args: &Args) -> Option<String> {
    #[cfg(test)]
    if cmd == seams::SLEEP {
        return args.0.get("lane").map(|l| format!("test:{l}"));
    }
    dispatch::lane(cmd, args)
}

/// One session on the loopback listener: handshake v1 first, then the
/// console's requests until the client goes.
async fn session(mut ws: Ws, shared: Arc<Shared>, id: u64, origin: String, slot: Arc<Slot>, budget: Arc<AtomicUsize>, peer: Option<String>) {
    // The deadline is on the WHOLE exchange, not per frame: two quick steps
    // adding up to more than it are refused like silence is. A refusal drops
    // the socket here with nothing more written, not even a Close frame. So
    // does being displaced by a newcomer while still unproven.
    let refused = {
        let hs = std::pin::pin!(tokio::time::timeout(policy::AUTH_DEADLINE, handshake_v1(&mut ws, &shared, &slot)));
        match select(hs, std::pin::pin!(slot.displaced())).await {
            Either::Left((Ok(Some(())), _)) => None,
            Either::Left((Ok(None), _)) => Some("handshake"),
            Either::Left((Err(_), _)) => Some("deadline"),
            Either::Right(_) => Some("displaced"),
        }
    };
    if let Some(why) = refused {
        info(&format!("serve: client {id} did not authenticate ({origin})"));
        // A displaced connection failed no handshake: it is the cap's, and
        // the audit counted it where `admit` displaced it.
        if why != "displaced" {
            shared.audit.auth_failed(why, id, peer);
        }
        return;
    }
    // In: the byte budget lifts to the codec's own MAX_MESSAGE. The
    // connection slot is held until the end.
    budget.store(usize::MAX, Ordering::SeqCst);
    info(&format!("serve: client {id} connected ({origin})"));
    // Who this socket is, fixed for its life: THIS listener's caller, the
    // owner on the console. Never derived from the request, a header or the
    // peer address (§3.3); the remote listener (slice 6) sets the device.
    // Every audit record the connection writes names it (§5.1).
    let caller = shared.caller();
    let au = Arc::new(ConnAudit::new(shared.audit.clone(), id, peer, caller.clone()));
    serve_caller(ws, shared.ctx.clone(), shared.clients.clone(), id, caller, None, au).await;
}

/// An authenticated connection's life, whoever accepted it: requests until
/// the client goes or its device is revoked or disabled (§2.5). `caller` is
/// the listener's, fixed here for the life of the socket — the console for
/// loopback; for the remote listener (slice 6), the device whose key completed
/// the handshake, with `stepup_exporter` its session's step-up exporter
/// (`tls::EXPORTER_STEPUP`) — and `au` is the audit's view of the
/// connection, naming it. The in-process tests drive it with fake device
/// callers.
///
/// It is also whose budgets the connection spends (`budget.rs`): the client
/// set's, host-wide, so both listeners' connections spend from one budget
/// and join one set of flights.
pub(super) async fn serve_caller<S>(
    ws: WebSocketStream<S>, ctx: Ctx, clients: Arc<Clients>, id: u64, caller: Caller, stepup_exporter: Option<[u8; 32]>, au: Arc<ConnAudit>,
)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    use futures_util::future::{select, Either};
    let Some(account) = clients.budget.open(id, &caller, Some(au.clone())) else {
        info(&format!("serve: client {id} ({caller}) refused: its device holds every connection it may"));
        return;
    };
    let (sink, mut stream) = ws.split();
    let (outbox, mut rx) = Outbox::new(account.limits.outbox, account.limits.outbox_stream);
    let mut over = outbox.closed();
    // THE writer: one task drains the one queue, so every frame for this
    // connection is written in the order it was queued.
    let out = outbox.clone();
    let mut writer = tokio::spawn(async move {
        let mut sink = sink;
        while let Some(m) = rx.recv().await {
            let (n, close) = (wire::cost(&m), matches!(m, Message::Close(_)));
            if sink.send(m).await.is_err() || close {
                break;
            }
            out.sent(n);
        }
    });
    let frames = Frames::with_exporter(outbox.clone(), stepup_exporter);
    let call = Arc::new(Call {
        ctx: ctx.clone(),
        frames: frames.clone(),
        caller,
        registry: clients.registry(),
        world: clients.world.clone(),
        devices: clients.devices(),
        pairing: clients.pairing(),
        flights: clients.flights.clone(),
    });
    frames.reconciled().store(call.registry.generation(), Ordering::SeqCst);
    let (kill, listens) = clients.join(id, outbox.clone(), call.clone());
    clients.end_if_gone(&call.caller);
    #[cfg(test)]
    clients.ready.lock().unwrap().insert(id);
    let lanes = Lanes::new(account.limits.lanes);

    let mut killed = false;
    loop {
        let next = std::pin::pin!(stream.next());
        let stop = std::pin::pin!(kill.notified());
        let full = std::pin::pin!(over.wait_for(|o| *o));
        let m = match select(next, select(stop, full)).await {
            Either::Left((Some(m), _)) => m,
            Either::Left((None, _)) => break,
            Either::Right((Either::Left(_), _)) => {
                killed = true;
                break;
            }
            Either::Right((Either::Right(_), _)) => {
                info(&format!("serve: client {id}: a reply came while its outbox was over budget ({} KiB); closing", outbox.peak() >> 10));
                au.limit("outbox");
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
                    // answered by nothing. What it holds is bounded by
                    // construction: `Listens` keeps one bit per known event.
                    // What it costs is not: each changed set is a log line,
                    // so a device pays a request token per frame, and one it
                    // has no token for is dropped unapplied — its last set
                    // stands, and the refusal counts into the rate's episode.
                    // The console's frames are free.
                    Ok(wire::Frame::Listen(names)) => {
                        if account.charge().is_err() {
                            continue;
                        }
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
                let wire::Request { id: rid, cmd, args } = req;
                let args = Args(args);
                let lane = lane_of(&cmd, &args);
                // The resolver's half that needs no I/O — the command exists,
                // its exposure and capability allow this caller — runs HERE,
                // before admission, so a request it refuses never takes an
                // in-flight slot, a lane or a thread. It is a request all the
                // same: a device pays a token for it, which also bounds the
                // refusal lines and audit records it can make serve write.
                // Step-up and the object checks need what the resolver finds,
                // so they run in the job, under the request's slot.
                if let Some(r) = precheck_of(&call.caller, &*call.registry, &cmd, &args, &au.request(), || account.charge()) {
                    let _ = outbox.push(match r {
                        Ok(v) => wire::reply_ok(&rid, v),
                        Err(e) => wire::reply_err(&rid, e),
                    });
                    continue;
                }
                // Admitted or refused HERE, before anything runs, and a
                // refusal is answered at once: `busy` or `rate`, never queued.
                let mut ticket = match account.admit(&cmd, &args, lane.as_deref()) {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = outbox.push(wire::reply_err(&rid, Value::from(e)));
                        continue;
                    }
                };
                // The connection's call, with frames that carry this
                // request's attach slot to the sink its body creates.
                let req = call.with(ctx.clone(), frames.for_request(ticket.attach.take()));
                let reply = outbox.clone();
                // The bodies are blocking (git, tmux, the disk), so each runs
                // on the blocking pool and its reply is queued when it is
                // done. Concurrently, as Tauri runs its async commands — a
                // clone that takes minutes must not hold up the keystroke
                // behind it — EXCEPT along one stream, where the keystroke
                // behind a keystroke must stay behind it: those go down the
                // stream's lane, one at a time. The ticket goes with the job,
                // so its slots come back however the job ends — answered,
                // refused by the resolver's object checks, panicked, or
                // dropped unrun. The resolver, the shared reads (`flight.rs`)
                // and the body all run inside `answer`'s `catch_unwind`.
                let busy_id = rid.clone();
                let au = au.clone();
                let job: Job = Box::new(move || {
                    let _ = reply.push(answer(&req, &au, &rid, &cmd, &args));
                    drop(ticket);
                });
                match lane {
                    Some(key) => {
                        if let Err(job) = lanes.run(key, job) {
                            drop(job);
                            let _ = outbox.push(wire::reply_err(&busy_id, Value::from(account.refuse("lanes", BUSY))));
                        }
                    }
                    None => {
                        tokio::task::spawn_blocking(job);
                    }
                }
            }
            // tungstenite queues a Pong on its own when the write half next
            // flushes; queuing one explicitly means it goes out even when
            // nothing else is being written. A peer a stream share behind
            // that still pings is not reading: closed, before tungstenite's
            // own pongs pile up behind ours.
            Ok(Message::Ping(p)) => {
                if outbox.pong(p).is_err() {
                    info(&format!("serve: client {id}: pings while {} KiB behind; closing", outbox.peak() >> 10));
                    au.limit("outbox");
                    break;
                }
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
    // The peaks are what the budgets were sized against: cheap to keep, and
    // the only way to see what a real client asks of them. The limits still
    // tripping end their episodes here, with their counts.
    let trips = account.close();
    info(&format!(
        "serve: client {id} {} ({} attach(es) closed; {}, peak lanes {}, peak outbox {} KiB{}{})",
        if killed { "closed by serve" } else { "disconnected" },
        terms.len(),
        account.summary(),
        lanes.peak.load(Ordering::SeqCst),
        outbox.peak() >> 10,
        if trips.is_empty() { "" } else { "; still tripping: " },
        trips
    ));
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
    let r = catch_unwind(AssertUnwindSafe(|| {
        #[cfg(test)]
        if let Some(r) = seams::body(cmd, args) {
            return r;
        }
        dispatch::call_seen(call, cmd, args, &seen)
    }));
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

/// `dispatch::precheck`, letting the test seams below through.
fn precheck_of(
    caller: &Caller,
    registry: &dyn Registry,
    cmd: &str,
    args: &Args,
    seen: &dyn dispatch::Witness,
    pay: impl FnOnce() -> Result<(), &'static str>,
) -> Option<Result<Value, Value>> {
    #[cfg(test)]
    if cmd == seams::SLEEP {
        return None;
    }
    dispatch::precheck(caller, registry, cmd, args, seen, pay)
}

/// Commands only test builds have, for the limits' tests: a body that takes
/// as long as it is told to, on a lane it names. Never in `TABLE`, so nothing
/// built for use can reach it. (The shared read's seam is dispatch's
/// `TEST_SHARED`, because the flights are there, behind the resolver.)
#[cfg(test)]
pub(crate) mod seams {
    use super::*;

    /// `{ms, lane?}`: sleeps `ms`, on lane `test:<lane>` when given.
    pub const SLEEP: &str = "__test_sleep";

    pub fn body(cmd: &str, args: &Args) -> Option<Result<Value, Value>> {
        let ms = args.0.get("ms").and_then(Value::as_u64).unwrap_or(0);
        match cmd {
            SLEEP => {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Some(Ok(Value::from(ms)))
            }
            _ => None,
        }
    }
}
