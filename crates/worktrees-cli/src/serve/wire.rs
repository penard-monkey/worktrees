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
//! Then three frame shapes, all JSON text:
//!
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
//! - event `{event, payload}` — `Host::emit`, to every authenticated socket.
//!
//! **ORDER is the contract.** Every frame for one connection — replies, channel
//! data, events — goes through ONE unbounded queue drained by ONE writer task,
//! so a byte that was produced before another is written before it. AGENTS.md
//! records a replay being answered as input because an out-of-band payload
//! overtook the response it belonged to; the queue is what makes that
//! impossible here rather than merely unlikely.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::Message;
use worktrees_api::ByteSink;

/// Handshake frame 2: the server's nonce.
pub fn challenge_frame(sn: &str) -> String {
    json!({ "challenge": sn }).to_string()
}

/// Handshake frame 4, sent only once the client's proof has verified: the
/// version, and the server's proof that it holds the token too.
pub fn auth_reply(proof: &str) -> String {
    json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "proof": proof }).to_string()
}

/// `err` for a command the socket will not run.
pub const CLIENT_LOCAL: &str = "client-local";

pub struct Request {
    pub id: Value,
    pub cmd: String,
    pub args: Map<String, Value>,
}

/// Parse one request frame. `id` is echoed back verbatim whatever its type;
/// `args` defaults to `{}` (an `invoke` with no args sends none).
pub fn parse_request(text: &str) -> Result<Request, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad request: {e}"))?;
    let Value::Object(mut o) = v else { return Err("bad request: not an object".into()) };
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
pub struct Outbox(pub UnboundedSender<Message>);

impl Outbox {
    /// Queue one text frame. `Err` means the connection is gone for good.
    pub fn push(&self, text: String) -> Result<(), ()> {
        self.0.send(Message::text(text)).map_err(|_| ())
    }
    pub fn send(&self, m: Message) -> Result<(), ()> {
        self.0.send(m).map_err(|_| ())
    }
}

/// A `ByteSink` over one channel of one connection.
struct FrameSink {
    outbox: Outbox,
    channel: u64,
    seq: AtomicU64,
}

impl ByteSink for FrameSink {
    fn send(&self, bytes: &[u8]) -> Result<(), String> {
        // The sequence number is taken and the frame queued in one critical
        // step from the caller's point of view: `ByteSink::send` is called by
        // one reader thread per stream, in order, so `fetch_add` + `push` on
        // that thread preserves the order the bytes were read in.
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let data = Value::String(worktrees_api::files::b64_encode(bytes));
        self.outbox
            .push(channel_frame(self.channel, seq, data))
            .map_err(|_| "client gone".to_string())
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
        let _ = self.outbox.push(end_frame(self.channel, seq));
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
        self.seq += 1;
        if self.outbox.push(channel_frame(self.channel, self.seq, data)).is_err() {
            self.dead = true;
            worktrees_api::log::warn(&format!("serve: progress channel {} closed; the operation runs on", self.channel));
        }
    }
}

impl Drop for Typed {
    fn drop(&mut self) {
        if !self.dead {
            let _ = self.outbox.push(end_frame(self.channel, self.seq + 1));
        }
    }
}

/// What a request needs to stream back to its own connection, and what the
/// connection needs to clean up when it goes: the tmux attaches it opened.
/// Dock shells are NOT on that list — they are owned by the process and a
/// re-attach replays them, which is the whole point of them surviving.
#[derive(Clone)]
pub struct Frames {
    outbox: Outbox,
    terms: Arc<Mutex<Vec<u32>>>,
}

impl Frames {
    pub fn new(outbox: Outbox) -> Self {
        Frames { outbox, terms: Arc::new(Mutex::new(Vec::new())) }
    }

    pub fn bytes(&self, channel: u64) -> Arc<dyn ByteSink> {
        Arc::new(FrameSink { outbox: self.outbox.clone(), channel, seq: AtomicU64::new(0) })
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

    pub fn note_term(&self, id: u32) {
        self.terms.lock().unwrap().push(id);
    }

    pub fn forget_term(&self, id: u32) {
        self.terms.lock().unwrap().retain(|t| *t != id);
    }

    pub fn take_terms(&self) -> Vec<u32> {
        std::mem::take(&mut *self.terms.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_id_cmd_and_an_optional_args_object() {
        let r = parse_request(r#"{"id":7,"cmd":"list_workspace"}"#).unwrap();
        assert_eq!(r.id, json!(7));
        assert_eq!(r.cmd, "list_workspace");
        assert!(r.args.is_empty());
        let r = parse_request(r#"{"id":"a-b","cmd":"set_pin","args":{"repo":"/r","slug":"s","on":true}}"#).unwrap();
        assert_eq!(r.id, json!("a-b"));
        assert_eq!(r.args["on"], json!(true));
        for bad in [r#"[]"#, r#"{"cmd":""}"#, r#"{"id":1}"#, r#"{"cmd":"x","args":[1]}"#, "nope"] {
            assert!(parse_request(bad).is_err(), "{bad}");
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
        let out = Outbox(tx);
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
        let frames = Frames::new(Outbox(tx));
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
        let frames = Frames::new(Outbox(tx));
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

    #[test]
    fn the_auth_reply_carries_this_binarys_version_and_the_proof() {
        let v: Value = serde_json::from_str(&auth_reply("p")).unwrap();
        assert_eq!(v, json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "proof": "p" }));
        let c: Value = serde_json::from_str(&challenge_frame("s")).unwrap();
        assert_eq!(c, json!({ "challenge": "s" }));
    }
}
