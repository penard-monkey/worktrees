//! What goes over the socket, and the per-connection queue it goes through.
//!
//! First the handshake (v1; `policy.rs` has the rules and the reasons):
//!
//!   C→S `{"hello": cn}` · S→C `{"challenge": sn}` · C→S `{"auth": client_proof}`
//!   · S→C `{"ok": true, "version": "<CARGO_PKG_VERSION>", "proof": server_proof}`
//!
//! with the token as the HMAC key and never on the wire; any other frame, a
//! wrong proof or a missed `AUTH_DEADLINE` closes the socket with NOTHING
//! written. The client believes `version` only once `proof` verifies.
//!
//! Then four frame shapes, all JSON text:
//!
//! - listen `{listen: [<event name>, …]}`, client to server: the events this
//!   connection listens for (§9.3), sent after `ok` and again whenever the
//!   set changes. Each is a full replacement and is never answered. Names
//!   serve does not emit are ignored. A one-shot event (`scope::ONE_SHOT`,
//!   the inbox's `app:open-doc`) goes only to a connection that declared it,
//!   and a held `worktrees show` waits until a loopback one has; a connection
//!   that declares nothing, like the app's verification probe, gets none.
//! - request `{id, cmd, args}` → reply `{id, ok: <value>}` or `{id, err: <value>}`,
//!   the same `{cmd, args}` the frontend's `invoke` builds. `err` is what the
//!   Tauri command's `Err` serializes to — a string for 142 commands, and the
//!   `CloneError` object for `clone_project`, whose dialog branches on `kind`.
//!   The point of the exercise is a frontend pointed at this socket unchanged.
//! - channel `{channel, seq, data}` — a `ByteSink` (data: base64) or a typed
//!   progress callback (data: the value). `channel` is the id the client gave
//!   in the request, the way Tauri's `Channel` serializes (`__CHANNEL__:<id>`).
//!   `seq` counts from 1 per channel. The LAST frame on a channel is
//!   `{channel, seq, end: true}` — no `data`, `seq` one past the last data
//!   frame — sent when the engine lets the sink go (a detach, `term_close`,
//!   the shell exiting, the operation behind a progress callback returning),
//!   so the client can drop the handler and whatever it closes over. Tauri
//!   does the same when its Rust `Channel` drops. An `err` reply to the
//!   request that named a channel ALSO finishes it: when the engine had taken
//!   the sink before failing, its end frame precedes the reply; when the
//!   request failed on its arguments, no sink existed and no end frame comes.
//! - event `{event, payload}` — `Host::emit`, to every authenticated socket
//!   (a one-shot one, to those that declared it).
//!
//! **ORDER is the contract.** Every frame for one connection — replies, channel
//! data, events — goes through ONE queue drained by ONE writer task, so a
//! byte that was produced before another is written before it. AGENTS.md
//! records a replay being answered as input because an out-of-band payload
//! overtook the response it belonged to; the queue is what makes that
//! impossible here rather than merely unlikely.
//!
//! **The queue has a byte budget** (Phase 2 §6, `budget::PerConnection`).
//! Unbounded, `yes` in a dock shell with a client that stopped reading took
//! serve to 2.5 GB in 10 s. What happens at the bound depends on what the
//! frame is. Channel data stops at the stream share: the sink reports the
//! stream gone, its owner lets it go, and the end frame follows, so the
//! client attaches again. A reply or an event that arrives while the queue
//! is over the whole budget closes the connection. The client reconnects
//! and asks again, which costs it less than a reply it never gets. A
//! progress frame past the stream share is dropped, and a ping that would
//! need a pong past it closes the connection. End frames always go: the
//! attach caps bound how many there can be.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use worktrees_api::ByteSink;

use super::budget::Held;

/// Handshake frame 2: the server's nonce.
pub fn challenge_frame(sn: &str) -> String {
    json!({ "challenge": sn }).to_string()
}

/// Handshake frame 4, sent only once the client's proof has verified: the
/// version, and the server's proof that it holds the token too.
///
/// On a remote session it also names the device the session belongs to —
/// `{id, principal, name, caps, projects, expiry, stepup_required}` — so the
/// device can show its own grant and expiry (Phase 2 §4.5, as amended by
/// (main) 2026-10-22: the remote listener runs handshake v1 unchanged, and
/// this is its `ok`). Loopback's has no `device`.
pub fn auth_reply(proof: &str, device: Option<&Value>) -> String {
    let mut v = json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "proof": proof });
    if let Some(d) = device {
        v["device"] = d.clone();
    }
    v.to_string()
}

/// `err` for a command the socket will not run.
pub const CLIENT_LOCAL: &str = "client-local";

/// `err` for a request whose body panicked. Nothing of the panic goes with it:
/// its text can carry a path, a client's value or the engine's state, and the
/// log is where that belongs.
pub const INTERNAL_ERROR: &str = "internal error";

/// The listen frame's one key.
pub const LISTEN: &str = "listen";

pub struct Request {
    pub id: Value,
    pub cmd: String,
    pub args: Map<String, Value>,
}

/// What a client sends after the handshake.
pub enum Frame {
    Request(Request),
    /// The events it listens for, as it named them: unknown ones included,
    /// for `scope::Listens` to drop.
    Listen(Vec<String>),
}

/// Parse one client frame: `{listen: [...]}` when it has that key, a request
/// otherwise. A frame that is neither is the client's bug, answered like an
/// unreadable request — with `id: null`, which a client cannot match to a call.
pub fn parse_frame(text: &str) -> Result<Frame, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad request: {e}"))?;
    let Value::Object(mut o) = v else { return Err("bad request: not an object".into()) };
    let Some(names) = o.remove(LISTEN) else { return parse_request(o).map(Frame::Request) };
    if !o.is_empty() {
        return Err(format!("bad listen: `{LISTEN}` is a frame of its own, with no other key"));
    }
    let Value::Array(names) = names else { return Err(format!("bad listen: `{LISTEN}` is not an array")) };
    names
        .into_iter()
        .map(|n| match n {
            Value::String(s) => Ok(s),
            _ => Err(format!("bad listen: `{LISTEN}` holds a name that is not a string")),
        })
        .collect::<Result<_, _>>()
        .map(Frame::Listen)
}

/// One request frame's fields. `id` is echoed back verbatim whatever its type;
/// `args` defaults to `{}` (an `invoke` with no args sends none).
pub fn parse_request(mut o: Map<String, Value>) -> Result<Request, String> {
    let id = o.remove("id").unwrap_or(Value::Null);
    let cmd = match o.remove("cmd") {
        Some(Value::String(s)) if !s.is_empty() => s,
        _ => return Err("bad request: no cmd".into()),
    };
    let args = match o.remove("args") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m,
        Some(_) => return Err("bad request: args is not an object".into()),
    };
    Ok(Request { id, cmd, args })
}

pub fn reply_ok(id: &Value, v: Value) -> String {
    json!({ "id": id, "ok": v }).to_string()
}

pub fn reply_err(id: &Value, e: Value) -> String {
    json!({ "id": id, "err": e }).to_string()
}

pub fn channel_frame(channel: u64, seq: u64, data: Value) -> String {
    json!({ "channel": channel, "seq": seq, "data": data }).to_string()
}

/// The last frame on a channel: no `data`, and `seq` continues the count.
pub fn end_frame(channel: u64, seq: u64) -> String {
    json!({ "channel": channel, "seq": seq, "end": true }).to_string()
}

pub fn event_frame(event: &str, payload: &Value) -> String {
    json!({ "event": event, "payload": payload }).to_string()
}

/// A channel argument as the frontend sends it: Tauri's `Channel` serializes
/// to the string `__CHANNEL__:<id>`; a bare integer is accepted for a client
/// that is not Tauri's bindings.
pub fn channel_id(v: &Value) -> Result<u64, String> {
    if let Some(n) = v.as_u64() {
        return Ok(n);
    }
    v.as_str()
        .and_then(|s| s.strip_prefix("__CHANNEL__:"))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| "a channel must be `__CHANNEL__:<id>` or an integer".to_string())
}

/// The per-connection queue. Cheap to clone; every clone feeds the same writer.
#[derive(Clone)]
pub struct Outbox(Arc<Queue>);

struct Queue {
    tx: UnboundedSender<Message>,
    /// Bytes queued and not yet handed to the socket (`Outbox::sent`).
    queued: AtomicUsize,
    peak: AtomicUsize,
    cap: usize,
    stream_cap: usize,
    /// Set once the budget closed the connection. Every push after it fails.
    over: watch::Sender<bool>,
}

/// Why a stream frame was not queued.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Past the stream share: this stream should end, the connection lives.
    Full,
    /// The connection is gone.
    Gone,
}

/// What a frame costs the budget: its payload bytes.
pub fn cost(m: &Message) -> usize {
    match m {
        Message::Text(t) => t.len(),
        Message::Binary(b) | Message::Ping(b) | Message::Pong(b) => b.len(),
        _ => 0,
    }
}

impl Outbox {
    /// A queue with a budget of `cap` bytes, `stream_cap` of them for channel
    /// data, and the receiver its one writer drains.
    pub fn new(cap: usize, stream_cap: usize) -> (Outbox, UnboundedReceiver<Message>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Outbox::with(tx, cap, stream_cap), rx)
    }

    /// No budget at all: the dispatch tests' queue that nobody drains.
    #[cfg(test)]
    pub fn unbounded(tx: UnboundedSender<Message>) -> Outbox {
        Outbox::with(tx, usize::MAX, usize::MAX)
    }

    fn with(tx: UnboundedSender<Message>, cap: usize, stream_cap: usize) -> Outbox {
        let (over, _) = watch::channel(false);
        Outbox(Arc::new(Queue { tx, queued: AtomicUsize::new(0), peak: AtomicUsize::new(0), cap, stream_cap, over }))
    }

    /// Queue `m` if the queue holds less than `limit` right now. The check is
    /// on what is ALREADY queued, so one frame larger than the whole budget
    /// still goes to a reader that is keeping up: its bytes exist either
    /// way, and what the budget bounds is a backlog.
    fn queue(&self, m: Message, limit: usize) -> Result<(), Refused> {
        let q = &self.0;
        if *q.over.borrow() {
            return Err(Refused::Gone);
        }
        let n = cost(&m);
        let was = q.queued.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |x| (x < limit).then(|| x.saturating_add(n)));
        let Ok(was) = was else { return Err(Refused::Full) };
        q.peak.fetch_max(was.saturating_add(n), Ordering::SeqCst);
        q.tx.send(m).map_err(|_| Refused::Gone)
    }

    /// Queue a reply or an event. `Err` means the connection is gone for
    /// good — and a queue already over its budget is made gone here: the
    /// session sees `closed` and ends the connection.
    pub fn push(&self, text: String) -> Result<(), ()> {
        match self.queue(Message::text(text), self.0.cap) {
            Ok(()) => Ok(()),
            Err(Refused::Full) => {
                self.0.over.send_replace(true);
                Err(())
            }
            Err(Refused::Gone) => Err(()),
        }
    }

    /// Queue channel data, within the stream share.
    pub fn push_stream(&self, text: String) -> Result<(), Refused> {
        self.queue(Message::text(text), self.0.stream_cap)
    }

    /// Queue a channel's end frame: always, while the connection lives.
    fn push_end(&self, text: String) {
        let _ = self.queue(Message::text(text), usize::MAX);
    }

    /// Queue a control frame the connection must get whatever its backlog —
    /// the close a revoke sends (`conn::Clients::end_device`). `Err` means
    /// the connection is gone.
    pub fn send(&self, m: Message) -> Result<(), ()> {
        self.queue(m, usize::MAX).map_err(|_| ())
    }

    /// Answer a ping, within the stream share. `Err` means the peer is a
    /// stream share behind and still pinging: the session closes it. That is
    /// the only bound there is on pongs, because tungstenite answers every
    /// ping as well, into a write buffer of its own that nothing limits —
    /// 200,000 unread pings took serve from 4 MB to 91 MB that way.
    pub fn pong(&self, payload: tokio_tungstenite::tungstenite::Bytes) -> Result<(), Refused> {
        self.queue(Message::Pong(payload), self.0.stream_cap)
    }

    /// The writer handed a frame of `n` bytes (`cost`) to the socket: they
    /// leave the budget.
    pub fn sent(&self, n: usize) {
        let _ = self.0.queued.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |x| Some(x.saturating_sub(n)));
    }

    /// Becomes `true` when the budget has closed the connection.
    pub fn closed(&self) -> watch::Receiver<bool> {
        self.0.over.subscribe()
    }

    /// The most bytes ever queued at once.
    pub fn peak(&self) -> usize {
        self.0.peak.load(Ordering::SeqCst)
    }
}

/// A `ByteSink` over one channel of one connection.
struct FrameSink {
    outbox: Outbox,
    channel: u64,
    seq: AtomicU64,
    /// The device's attach or shell slot (`budget::Ticket::attach`), held for
    /// as long as the stream lives. `None` on the console.
    _slot: Option<Held>,
}

impl ByteSink for FrameSink {
    fn send(&self, bytes: &[u8]) -> Result<(), String> {
        // The sequence number is taken and the frame queued in one critical
        // step from the caller's point of view: `ByteSink::send` is called by
        // one reader thread per stream, in order, so `fetch_add` + `push` on
        // that thread preserves the order the bytes were read in. A frame the
        // budget refuses takes no `seq`, so the end frame that follows is
        // still one past the last data the client was sent.
        let seq = self.seq.load(Ordering::SeqCst) + 1;
        let data = Value::String(worktrees_api::files::b64_encode(bytes));
        match self.outbox.push_stream(channel_frame(self.channel, seq, data)) {
            Ok(()) => {
                self.seq.store(seq, Ordering::SeqCst);
                Ok(())
            }
            // Ending the stream is the owner's act: a tmux attach's reader
            // stops, a dock shell's lets this sink go and keeps its ring. The
            // sink then drops, and the end frame tells the client to attach
            // again.
            Err(Refused::Full) => Err("the client is not reading: over its outbox budget".to_string()),
            Err(Refused::Gone) => Err("client gone".to_string()),
        }
    }
}

impl Drop for FrameSink {
    /// The last `Arc` went: the reader thread that owned this sink has
    /// stopped (EOF, a detach, `term_close`, the client gone), and nothing can
    /// write to this channel again. Say so, after the last byte — the end
    /// frame takes the next `seq`, on the same queue, so it cannot overtake
    /// data. A dead connection is the one case nobody needs telling.
    fn drop(&mut self) {
        let seq = self.seq.load(Ordering::SeqCst) + 1;
        self.outbox.push_end(end_frame(self.channel, seq));
    }
}

/// A typed channel's state, owned by the closure `Frames::typed` returns, so
/// that dropping the closure — the operation has returned — ends the channel.
struct Typed {
    outbox: Outbox,
    channel: u64,
    seq: u64,
    dead: bool,
}

impl Typed {
    fn push(&mut self, data: Value) {
        if self.dead {
            return;
        }
        // Progress is a sample, and a newer one follows: past the stream
        // share this one is dropped, and only a gone connection ends it.
        match self.outbox.push_stream(channel_frame(self.channel, self.seq + 1, data)) {
            Ok(()) => self.seq += 1,
            Err(Refused::Full) => {}
            Err(Refused::Gone) => {
                self.dead = true;
                worktrees_api::log::warn(&format!("serve: progress channel {} closed; the operation runs on", self.channel));
            }
        }
    }
}

impl Drop for Typed {
    fn drop(&mut self) {
        if !self.dead {
            self.outbox.push_end(end_frame(self.channel, self.seq + 1));
        }
    }
}

/// What a request needs to stream back to its own connection, and what the
/// connection needs to clean up when it goes: the tmux attaches it opened.
/// Dock shells are NOT closed with it — they are owned by the process and a
/// re-attach replays them, which is the whole point of them surviving — but
/// the sink this connection holds on one is let go.
///
/// It is also what a connection OWNS, for the resolver (§3.3): `attach(id)`
/// asks `owns_term`, and a clone job's id is namespaced with `tag`, so no
/// connection can type into, resize, close or cancel another's. And it is what
/// a REDUCTION acts on (§3.3, Phase 2 slice 5): each attach with the session it
/// shows, each dock shell this connection holds the sink of, each clone job it
/// started — so a narrowed grant ends exactly the channels it no longer
/// covers, and a revoke cancels the device's jobs.
#[derive(Clone)]
pub struct Frames {
    outbox: Outbox,
    terms: Arc<Mutex<Vec<(u32, String)>>>,
    shells: Arc<Mutex<Vec<ShellHold>>>,
    jobs: Arc<Mutex<Vec<String>>>,
    tag: Arc<str>,
    /// The registry generation this connection's channels were last checked
    /// at (`conn::reconcile`).
    reconciled: Arc<AtomicU64>,
    /// The connection's step-up state (§2.5).
    stepup: Arc<super::stepup::StepUp>,
    /// This request's attach or shell slot, for the sink its body creates.
    slot: Arc<Mutex<Option<Held>>>,
}

/// A dock shell whose output this connection receives: its key, and the
/// attach generation `shell_detach` needs to let go of exactly this sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellHold {
    pub repo: String,
    pub slug: String,
    pub index: u32,
    pub gen: u64,
}

impl Frames {
    /// A connection with no TLS session (the tests' loopback calls).
    #[cfg(test)]
    pub fn new(outbox: Outbox) -> Self {
        Frames::with_exporter(outbox, None)
    }

    /// A connection with a TLS session: `exporter` is its step-up exporter
    /// (`EXPORTER-worktrees-stepup-v1`), what a step-up proof is bound to.
    pub fn with_exporter(outbox: Outbox, exporter: Option<[u8; 32]>) -> Self {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).expect("the OS refused entropy");
        let tag: String = b.iter().map(|x| format!("{x:02x}")).collect();
        Frames {
            outbox,
            terms: Arc::new(Mutex::new(Vec::new())),
            shells: Arc::new(Mutex::new(Vec::new())),
            jobs: Arc::new(Mutex::new(Vec::new())),
            tag: Arc::from(tag.as_str()),
            reconciled: Arc::new(AtomicU64::new(0)),
            stepup: Arc::new(super::stepup::StepUp::new(exporter)),
            slot: Arc::new(Mutex::new(None)),
        }
    }

    /// The connection's frames for one request, carrying `slot` to the sink
    /// that request's body creates. A body that fails first drops it unused.
    /// The same connection in every other way — its attaches, shells, jobs,
    /// tag, step-up and reconcile mark — not a new namespace.
    pub fn for_request(&self, slot: Option<Held>) -> Frames {
        Frames { slot: Arc::new(Mutex::new(slot)), ..self.clone() }
    }

    pub fn stepup(&self) -> &super::stepup::StepUp {
        &self.stepup
    }

    /// This connection's own namespace for ids it hands the engine. Random,
    /// so it cannot be guessed from another connection.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// The attach `id` was opened on this connection and not closed since.
    pub fn owns_term(&self, id: u32) -> bool {
        self.terms.lock().unwrap().iter().any(|(t, _)| *t == id)
    }

    pub fn bytes(&self, channel: u64) -> Arc<dyn ByteSink> {
        let slot = self.slot.lock().unwrap().take();
        Arc::new(FrameSink { outbox: self.outbox.clone(), channel, seq: AtomicU64::new(0), _slot: slot })
    }

    /// A typed progress callback: each value becomes one channel frame. The
    /// clone and sync bodies take `&mut dyn FnMut(T)`; a dead connection is
    /// noted once and the operation runs on, exactly as the Tauri shim did.
    /// The callback is a local of the row's body, so it drops — and the
    /// channel ends — when the operation returns, before the reply is queued.
    pub fn typed<T: serde::Serialize>(&self, channel: u64) -> impl FnMut(T) + Send + 'static {
        let mut ch = Typed { outbox: self.outbox.clone(), channel, seq: 0, dead: false };
        move |t: T| ch.push(serde_json::to_value(&t).unwrap_or(Value::Null))
    }

    /// An attach this connection opened, on the (canonical) session it shows.
    pub fn note_term(&self, id: u32, session: String) {
        self.terms.lock().unwrap().push((id, session));
    }

    pub fn forget_term(&self, id: u32) {
        self.terms.lock().unwrap().retain(|(t, _)| *t != id);
    }

    pub fn take_terms(&self) -> Vec<u32> {
        std::mem::take(&mut *self.terms.lock().unwrap()).into_iter().map(|(t, _)| t).collect()
    }

    /// Every open attach, with its session.
    pub fn terms(&self) -> Vec<(u32, String)> {
        self.terms.lock().unwrap().clone()
    }

    /// A dock shell this connection now holds the sink of. A re-attach of the
    /// same shell replaces the older hold: only the newest generation is ours.
    pub fn note_shell(&self, hold: ShellHold) {
        let mut v = self.shells.lock().unwrap();
        v.retain(|h| (&h.repo, &h.slug, h.index) != (&hold.repo, &hold.slug, hold.index));
        v.push(hold);
    }

    pub fn forget_shell(&self, repo: &str, slug: &str, index: u32) {
        self.shells.lock().unwrap().retain(|h| (h.repo.as_str(), h.slug.as_str(), h.index) != (repo, slug, index));
    }

    /// The hold on this shell, if it is generation `gen` — what a client's
    /// `shell_detach` releases. A detach naming any other generation releases
    /// nothing (`term::shell_detach` ignores it too), so the hold must stay
    /// recorded, or the sink would outlive the next narrowing unseen.
    pub fn take_shell_gen(&self, repo: &str, slug: &str, index: u32, gen: u64) -> bool {
        let mut v = self.shells.lock().unwrap();
        let before = v.len();
        v.retain(|h| (h.repo.as_str(), h.slug.as_str(), h.index, h.gen) != (repo, slug, index, gen));
        v.len() != before
    }

    pub fn shells(&self) -> Vec<ShellHold> {
        self.shells.lock().unwrap().clone()
    }

    pub fn take_shells(&self) -> Vec<ShellHold> {
        std::mem::take(&mut *self.shells.lock().unwrap())
    }

    /// A clone job this connection started, by its namespaced id.
    pub fn note_job(&self, id: String) {
        let mut v = self.jobs.lock().unwrap();
        if !v.contains(&id) {
            v.push(id);
        }
    }

    pub fn take_jobs(&self) -> Vec<String> {
        std::mem::take(&mut *self.jobs.lock().unwrap())
    }

    /// The registry generation this connection's channels were last checked
    /// at; `swap` it for the current one to learn whether a check is due.
    pub fn reconciled(&self) -> &AtomicU64 {
        &self.reconciled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str) -> Result<Request, String> {
        match parse_frame(text)? {
            Frame::Request(r) => Ok(r),
            Frame::Listen(_) => Err("a listen frame".into()),
        }
    }

    /// A request's frames are its connection's: the same attaches and the
    /// same tag. An attach one request opens is the connection's for the next
    /// request (`owns_term`), and a job one request starts is namespaced the
    /// same for the request that cancels it (the resolver, §3.3). A tag per
    /// request would make every clone uncancellable.
    #[test]
    fn a_requests_frames_are_its_connections() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let conn = Frames::new(Outbox::unbounded(tx));
        conn.for_request(None).note_term(7, "s".into());
        let next = conn.for_request(None);
        assert_eq!(next.tag(), conn.tag());
        assert!(next.owns_term(7));
        let (tx2, _rx2) = mpsc::unbounded_channel();
        assert_ne!(Frames::new(Outbox::unbounded(tx2)).tag(), conn.tag(), "another connection's namespace");
    }

    #[test]
    fn a_request_is_id_cmd_and_an_optional_args_object() {
        let r = request(r#"{"id":7,"cmd":"list_workspace"}"#).unwrap();
        assert_eq!(r.id, json!(7));
        assert_eq!(r.cmd, "list_workspace");
        assert!(r.args.is_empty());
        let r = request(r#"{"id":"a-b","cmd":"set_pin","args":{"repo":"/r","slug":"s","on":true}}"#).unwrap();
        assert_eq!(r.id, json!("a-b"));
        assert_eq!(r.args["on"], json!(true));
        for bad in [r#"[]"#, r#"{"cmd":""}"#, r#"{"id":1}"#, r#"{"cmd":"x","args":[1]}"#, "nope"] {
            assert!(request(bad).is_err(), "{bad}");
        }
    }

    /// `{listen: [...]}` is the names as sent, unknown ones too (the set
    /// drops those, `scope::Listens`); anything else under that key, or
    /// beside it, is a client's bug and an error.
    #[test]
    fn a_listen_frame_is_one_array_of_names() {
        let names = |t: &str| match parse_frame(t) {
            Ok(Frame::Listen(n)) => Ok(n),
            Ok(Frame::Request(_)) => Err("a request".to_string()),
            Err(e) => Err(e),
        };
        assert_eq!(names(r#"{"listen":["app:open-doc","no:such"]}"#).unwrap(), vec!["app:open-doc", "no:such"]);
        assert_eq!(names(r#"{"listen":[]}"#).unwrap(), Vec::<String>::new());
        for bad in [r#"{"listen":"app:open-doc"}"#, r#"{"listen":[1]}"#, r#"{"listen":null}"#, r#"{"listen":[],"id":1,"cmd":"x"}"#] {
            assert!(names(bad).unwrap_err().starts_with("bad listen"), "{bad}");
        }
    }

    #[test]
    fn a_channel_is_tauris_spelling_or_an_integer() {
        assert_eq!(channel_id(&json!("__CHANNEL__:42")).unwrap(), 42);
        assert_eq!(channel_id(&json!(3)).unwrap(), 3);
        for bad in [json!("42"), json!("__CHANNEL__:"), json!(-1), json!(null), json!({"id": 1})] {
            assert!(channel_id(&bad).is_err(), "{bad}");
        }
    }

    /// The order contract, on the queue itself: bytes sent through one sink
    /// come out as consecutive `seq`s in send order, and the reply queued
    /// after them lands after them.
    #[test]
    fn frames_keep_the_order_they_were_queued_in() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let out = Outbox::unbounded(tx);
        let frames = Frames::new(out.clone());
        let sink = frames.bytes(5);
        sink.send(b"ab").unwrap();
        sink.send(b"c").unwrap();
        out.push(reply_ok(&json!(1), json!(9))).unwrap();
        let mut seen = Vec::new();
        while let Ok(m) = rx.try_recv() {
            seen.push(m.into_text().unwrap().as_str().to_string());
        }
        assert_eq!(seen[0], r#"{"channel":5,"seq":1,"data":"YWI="}"#);
        assert_eq!(seen[1], r#"{"channel":5,"seq":2,"data":"Yw=="}"#);
        assert_eq!(seen[2], r#"{"id":1,"ok":9}"#);
        drop(rx);
        assert!(sink.send(b"x").is_err(), "a closed connection is an Err, so the pty reader stops");
    }

    /// A sink the engine lets go of ends its channel: one more frame, the
    /// next `seq`, `end: true`, no `data`. The reader thread that owned it is
    /// gone, so nothing can follow it on that channel.
    #[test]
    fn dropping_a_sink_ends_its_channel_after_its_last_byte() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let frames = Frames::new(Outbox::unbounded(tx));
        let sink = frames.bytes(5);
        sink.send(b"ab").unwrap();
        drop(sink);
        let mut seen = Vec::new();
        while let Ok(m) = rx.try_recv() {
            seen.push(m.into_text().unwrap().as_str().to_string());
        }
        assert_eq!(seen, vec![r#"{"channel":5,"seq":1,"data":"YWI="}"#.to_string(), r#"{"channel":5,"seq":2,"end":true}"#.to_string()]);
    }

    /// The same for a typed progress callback: the operation returns, the
    /// callback is dropped, the channel ends — before the reply, which the
    /// dispatcher queues after the body has returned.
    #[test]
    fn dropping_a_typed_callback_ends_its_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let frames = Frames::new(Outbox::unbounded(tx));
        let mut progress = frames.typed::<u32>(9);
        progress(1);
        progress(2);
        drop(progress);
        let mut seen = Vec::new();
        while let Ok(m) = rx.try_recv() {
            seen.push(m.into_text().unwrap().as_str().to_string());
        }
        assert_eq!(seen.last().map(String::as_str), Some(r#"{"channel":9,"seq":3,"end":true}"#), "{seen:?}");
        assert_eq!(seen.len(), 3);
    }

    /// The budget's three rules on the queue itself, with no socket: channel
    /// data stops at the stream share, a reply past the whole budget closes
    /// the connection, and an end frame always goes. What the writer hands to
    /// the socket leaves the budget.
    #[test]
    fn the_outbox_stops_streams_at_their_share_and_closes_on_a_reply_past_the_budget() {
        let (out, mut rx) = Outbox::new(100, 40);
        let closed = out.closed();
        assert_eq!(out.push_stream("x".repeat(30)), Ok(()));
        assert_eq!(out.push_stream("x".repeat(30)), Ok(()), "queued is checked BEFORE adding: 30 < 40");
        assert_eq!(out.push_stream("y".into()), Err(Refused::Full), "60 queued, past the share of 40");
        assert!(out.pong(Default::default()).is_err(), "a pong is held to the share too");
        assert!(out.push("a reply".into()).is_ok(), "60 < 100: replies still go");
        out.push_end("end".into());
        for _ in 0..2 {
            let m = rx.try_recv().unwrap();
            out.sent(cost(&m));
        }
        assert_eq!(out.push_stream("z".into()), Ok(()), "the two streams' 60 bytes written: 10 queued, under the share");
        assert!(out.push("r".repeat(95)).is_ok(), "11 queued < 100: a reply larger than what is left still goes");
        assert!(!*closed.borrow(), "under the budget nothing closes");
        assert!(out.push("one too many".into()).is_err(), "a reply into a queue past its budget");
        assert!(*closed.borrow(), "and the connection is told to close");
        assert_eq!(out.push_stream("after".into()), Err(Refused::Gone), "after which nothing queues");
        assert!(out.peak() >= 100, "{}", out.peak());
    }

    /// A sink refused for the budget ends its channel like any other: no
    /// `seq` spent on the refused frame, then the end frame.
    #[test]
    fn a_sink_over_the_stream_share_fails_and_its_end_frame_follows() {
        // One 35-byte frame takes the queue past a share of 30.
        let (out, mut rx) = Outbox::new(1 << 20, 30);
        let frames = Frames::new(out);
        let sink = frames.bytes(3);
        sink.send(b"ab").unwrap();
        assert!(sink.send(b"cd").is_err(), "the first frame filled the share");
        drop(sink);
        let mut seen = Vec::new();
        while let Ok(m) = rx.try_recv() {
            seen.push(m.into_text().unwrap().as_str().to_string());
        }
        assert_eq!(seen, vec![r#"{"channel":3,"seq":1,"data":"YWI="}"#.to_string(), r#"{"channel":3,"seq":2,"end":true}"#.to_string()]);
    }

    #[test]
    fn the_auth_reply_carries_this_binarys_version_and_the_proof() {
        let v: Value = serde_json::from_str(&auth_reply("p", None)).unwrap();
        assert_eq!(v, json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "proof": "p" }), "loopback's, exactly as it was");
        let d = json!({ "id": "d_1", "principal": "owner" });
        let v: Value = serde_json::from_str(&auth_reply("p", Some(&d))).unwrap();
        assert_eq!(v, json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "proof": "p", "device": d }), "a remote session's names its device");
        let c: Value = serde_json::from_str(&challenge_frame("s")).unwrap();
        assert_eq!(c, json!({ "challenge": "s" }));
    }
}
