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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use http_body_util::Empty;
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderMap, HeaderName, CONNECTION, HOST, ORIGIN, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use worktrees_api::log::{info, warn};
use worktrees_api::{Ctx, Host};

use super::dispatch::{self, Args, Call};
use super::policy;
use super::wire::{self, Frames, Outbox};

/// How long the HTTP half of a connection gets. The docs server's figure: a
/// loopback client that opened a socket and then says nothing holds a task
/// and a descriptor, and nothing else ends it.
const HANDSHAKE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

/// The accept loop's pause after a failed `accept()` — hyper's
/// `sleep_on_errors` shape, so a process-wide `EMFILE` is a retry rather than a
/// pegged core (see the docs server, which learned it first).
const ACCEPT_RETRY: std::time::Duration = std::time::Duration::from_millis(50);

/// Every authenticated connection's queue, for broadcasts.
#[derive(Default)]
pub struct Clients(Mutex<HashMap<u64, Outbox>>);

impl Clients {
    fn add(&self, id: u64, out: Outbox) {
        self.0.lock().unwrap().insert(id, out);
    }
    fn remove(&self, id: u64) {
        self.0.lock().unwrap().remove(&id);
    }
}

/// The engine's `Host`: an event is one frame to every authenticated socket.
/// Fire-and-forget by construction — a dead queue is dropped from the map on
/// the way past, and nothing here waits on a client.
pub struct Broadcast(pub Arc<Clients>);

impl Host for Broadcast {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let text = wire::event_frame(event, &payload);
        let mut map = self.0 .0.lock().unwrap();
        map.retain(|_, out| out.push(text.clone()).is_ok());
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
pub fn start(ctx: Ctx, token: String, port: u16, origins: Vec<String>, clients: Arc<Clients>) -> Result<Listener, String> {
    // 127.0.0.1 only, as a rule and not a default (brief: "you bind 127.0.0.1
    // and nothing else"). Not `localhost`, which may resolve to `::1` first
    // and would make the IPv4 address in serve.json a lie.
    let std_listener = StdListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .map_err(|e| format!("could not bind 127.0.0.1:{port}: {e}"))?;
    let port = std_listener.local_addr().map_err(|e| e.to_string())?.port();
    std_listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let listener = tokio::net::TcpListener::from_std(std_listener).map_err(|e| e.to_string())?;
    let shared = Arc::new(Shared { ctx, token: Arc::from(token.as_str()), port, origins, clients, next_id: AtomicU64::new(1) });
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
            tokio::spawn(connection(stream, shared.clone()));
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

async fn connection(stream: TcpStream, shared: Arc<Shared>) {
    let io = TokioIo::new(stream);
    let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
        let shared = shared.clone();
        async move { handshake(req, shared) }
    });
    let conn = hyper::server::conn::http1::Builder::new().serve_connection(io, svc).with_upgrades();
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
fn handshake(mut req: Request<Incoming>, shared: Arc<Shared>) -> Result<Response<Empty<Bytes>>, Refused> {
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
                let ws = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
                session(ws, shared, id, origin).await;
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

type Ws = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;

/// One authenticated session: the auth frame first, then requests until the
/// client goes.
async fn session(mut ws: Ws, shared: Arc<Shared>, id: u64, origin: String) {
    // FIRST frame, within the deadline, exactly the auth object. Anything
    // else — wrong token, a request, a binary frame, silence — drops the
    // socket here with nothing written, not even a Close frame.
    let first = tokio::time::timeout(policy::AUTH_DEADLINE, ws.next()).await;
    let authed = matches!(&first, Ok(Some(Ok(Message::Text(t)))) if policy::auth_ok(t.as_str(), &shared.token));
    if !authed {
        info(&format!("serve: client {id} did not authenticate ({origin})"));
        return;
    }
    if ws.send(Message::text(wire::auth_reply())).await.is_err() {
        return;
    }
    info(&format!("serve: client {id} connected ({origin})"));

    let (sink, mut stream) = ws.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let outbox = Outbox(tx);
    shared.clients.add(id, outbox.clone());
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
    let call = Arc::new(Call { ctx: shared.ctx.clone(), frames: frames.clone() });

    while let Some(m) = stream.next().await {
        match m {
            Ok(Message::Text(t)) => {
                // Concurrently, as Tauri runs its async commands: a clone that
                // takes minutes must not hold up the keystroke behind it. The
                // bodies are blocking (git, tmux, the disk), so each runs on
                // the blocking pool and its reply is queued when it is done.
                let call = call.clone();
                let outbox = outbox.clone();
                tokio::task::spawn_blocking(move || {
                    let _ = outbox.push(answer(&call, t.as_str()));
                });
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

/// One request frame → one reply frame.
fn answer(call: &Call, text: &str) -> String {
    let req = match wire::parse_request(text) {
        Ok(r) => r,
        Err(e) => return wire::reply_err(&serde_json::Value::Null, e.into()),
    };
    // Host-admin commands change this machine's user-tier state; a line per
    // call is the audit trail Phase 2's gate will want, and it is cheap now.
    if dispatch::exposure(&req.cmd) == Some(dispatch::Exposure::HostAdmin) {
        info(&format!("serve: host-admin {}", req.cmd));
    }
    match dispatch::call(call, &req.cmd, &Args(req.args)) {
        Ok(v) => wire::reply_ok(&req.id, v),
        Err(e) => wire::reply_err(&req.id, e),
    }
}
