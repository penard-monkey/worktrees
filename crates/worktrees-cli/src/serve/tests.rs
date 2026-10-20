//! The server over real sockets.
//!
//! The pure tests in `policy` are the rule set; these are whether the rule set
//! is WIRED — delete the `host_ok` call from `handshake` and every pure test
//! still passes. Raw `std` sockets and a hand-written upgrade request, for the
//! docs server's reason: a client library would send a well-formed request
//! and the refusals are about requests that are not.
//!
//! The one client library in use is tungstenite's SYNC frame codec over the
//! raw socket once the 101 has been read by hand — the same crate the server
//! uses for frames, in its blocking form, so the test needs no runtime of its
//! own on the client side.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocket};
use tokio_tungstenite::tungstenite::Message;

use super::{start, Running};

/// RFC 6455 §1.3's worked example: this key yields this accept value. A
/// server that derives it right is talking the handshake the spec wrote down.
const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
const DEV_ORIGIN: &str = "http://localhost:1420";

/// A server on an ephemeral port over a temp config dir, with the dev origin
/// allowed. Stopped however the test ends: a `#[test]` that panics before its
/// own teardown would otherwise leave a real server on a real port on the
/// machine of whoever ran it.
struct Live {
    running: Option<Running>,
    dir: PathBuf,
    rt: tokio::runtime::Runtime,
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(r) = self.running.take() {
            r.listener.stop();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn live(tag: &str) -> Live {
    let dir = std::env::temp_dir().join(format!("wt-serve-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let running = rt.block_on(async { start(dir.clone(), 0, vec![DEV_ORIGIN.to_string()]) }).unwrap();
    Live { running: Some(running), dir, rt }
}

impl Live {
    fn port(&self) -> u16 {
        self.running.as_ref().unwrap().port()
    }
    fn token(&self) -> String {
        self.running.as_ref().unwrap().token.clone()
    }
}

/// One raw upgrade request, byte for byte as given, and WHATEVER the server
/// wrote back before closing — which for a refusal must be nothing at all.
/// `host`/`origin` as `None` omit the header entirely.
fn upgrade(port: u16, host: Option<&str>, origin: Option<&str>, extra: &str) -> (TcpStream, Vec<u8>) {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("the server is listening");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut req = String::from("GET / HTTP/1.1\r\n");
    if let Some(h) = host {
        req.push_str(&format!("Host: {h}\r\n"));
    }
    if let Some(o) = origin {
        req.push_str(&format!("Origin: {o}\r\n"));
    }
    req.push_str(&format!(
        "Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\n{extra}\r\n"
    ));
    s.write_all(req.as_bytes()).unwrap();
    s.flush().unwrap();
    // Read the response head only — up to the blank line — so the stream is
    // positioned at the first frame for a served connection. A refused one
    // reaches EOF with nothing read.
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    loop {
        match s.read(&mut b) {
            Ok(0) => break,
            Ok(_) => {
                head.push(b[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(e) => panic!("reading the response: {e} (got {:?} so far)", String::from_utf8_lossy(&head)),
        }
    }
    (s, head)
}

fn good_host(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

/// A served, authenticated socket, ready for requests.
fn client(l: &Live) -> WebSocket<TcpStream> {
    let (s, head) = upgrade(l.port(), Some(&good_host(l.port())), Some(DEV_ORIGIN), "");
    let head = String::from_utf8_lossy(&head).into_owned();
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    let mut ws = WebSocket::from_raw_socket(s, Role::Client, None);
    ws.send(Message::text(json!({ "auth": l.token() }).to_string())).unwrap();
    let reply: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(reply["ok"], json!(true), "{reply}");
    ws
}

fn rpc(ws: &mut WebSocket<TcpStream>, id: Value, cmd: &str, args: Value) -> Value {
    ws.send(Message::text(json!({ "id": id, "cmd": cmd, "args": args }).to_string())).unwrap();
    serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap()
}

/// **The connect-witness, four directions, served case FIRST** so a dead
/// server cannot pass: a server that refused everything would pass the three
/// refusals and fail the first line. Each refusal is a CLOSED socket with
/// zero bytes written — no status, no body.
#[test]
fn a_loopback_client_is_served_and_the_three_forgeries_are_closed_with_nothing_said() {
    let l = live("witness");
    let p = l.port();

    let (_s, head) = upgrade(p, Some(&good_host(p)), Some(DEV_ORIGIN), "");
    let head = String::from_utf8_lossy(&head).into_owned();
    assert!(head.starts_with("HTTP/1.1 101 "), "served: {head}");
    assert!(head.to_ascii_lowercase().contains(&format!("sec-websocket-accept: {}", ACCEPT.to_ascii_lowercase())), "{head}");
    assert!(!head.to_ascii_lowercase().contains("access-control-allow-origin"), "no CORS header, ever: {head}");

    let (_s, foreign) = upgrade(p, Some(&good_host(p)), Some("https://evil.example"), "");
    assert!(foreign.is_empty(), "a foreign Origin was answered: {:?}", String::from_utf8_lossy(&foreign));

    let (_s, missing) = upgrade(p, Some(&good_host(p)), None, "");
    assert!(missing.is_empty(), "a missing Origin was answered: {:?}", String::from_utf8_lossy(&missing));

    let (_s, forged) = upgrade(p, Some("attacker.example"), Some(DEV_ORIGIN), "");
    assert!(forged.is_empty(), "a forged Host was answered: {:?}", String::from_utf8_lossy(&forged));
}

/// The built-in origins are served without any flag; the dev one only
/// because this server was started with it; and a duplicate `Host` is a
/// refusal, not an ambiguity to resolve.
#[test]
fn the_webview_origins_are_built_in_and_a_duplicate_host_is_refused() {
    let l = live("origins");
    let p = l.port();
    for o in super::policy::BUILTIN_ORIGINS {
        let (_s, head) = upgrade(p, Some(&good_host(p)), Some(o), "");
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101 "), "{o}");
    }
    let (_s, dup) = upgrade(p, Some(&good_host(p)), Some(DEV_ORIGIN), &format!("Host: {}\r\n", good_host(p)));
    assert!(dup.is_empty(), "two Host headers: {:?}", String::from_utf8_lossy(&dup));
    // A plain GET of the root — right Host, right Origin, no upgrade — is
    // closed too: there is no page here.
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, p));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(format!("GET / HTTP/1.1\r\nHost: {}\r\nOrigin: {DEV_ORIGIN}\r\n\r\n", good_host(p)).as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    assert!(out.is_empty(), "a plain GET was answered: {:?}", String::from_utf8_lossy(&out));
}

/// The first frame decides: the right token is answered with the version;
/// a wrong token, a request sent first, or a binary frame each close the
/// socket with no frame written back — not even a Close.
#[test]
fn the_first_frame_must_be_the_auth_object_and_the_reply_names_the_version() {
    let l = live("auth");
    let p = l.port();

    let mut ws = client(&l);
    // (client() already asserted ok:true) — and the version is this binary's.
    ws.send(Message::text(json!({ "id": 1, "cmd": "get_changelog" }).to_string())).unwrap();
    let r: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(r["ok"]["version"], json!(env!("CARGO_PKG_VERSION")));

    let closed = |first: Message| {
        let (s, head) = upgrade(p, Some(&good_host(p)), Some(DEV_ORIGIN), "");
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101 "));
        let mut ws = WebSocket::from_raw_socket(s, Role::Client, None);
        ws.send(first).unwrap();
        match ws.read() {
            Ok(m) => panic!("the server answered an unauthenticated frame: {m:?}"),
            Err(e) => assert!(
                !matches!(e, tokio_tungstenite::tungstenite::Error::ConnectionClosed),
                "a Close frame was sent — the socket must be dropped with nothing said: {e}"
            ),
        }
    };
    closed(Message::text(json!({ "auth": "0".repeat(64) }).to_string()));
    closed(Message::text(json!({ "id": 1, "cmd": "list_workspace" }).to_string()));
    closed(Message::binary(l.token().into_bytes()));
}

/// The version in the auth reply is `CARGO_PKG_VERSION` — what the app will
/// compare against its own, and what the serve.json carries.
#[test]
fn the_auth_reply_version_is_the_cargo_version() {
    let l = live("version");
    let (s, _) = upgrade(l.port(), Some(&good_host(l.port())), Some(DEV_ORIGIN), "");
    let mut ws = WebSocket::from_raw_socket(s, Role::Client, None);
    ws.send(Message::text(json!({ "auth": l.token() }).to_string())).unwrap();
    let reply: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(reply, json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }));
}

/// A client that says nothing after the upgrade is cut off at the deadline,
/// with nothing written. Costs the deadline, once.
#[test]
fn a_silent_client_is_dropped_at_the_auth_deadline() {
    let l = live("silent");
    let (mut s, _) = upgrade(l.port(), Some(&good_host(l.port())), Some(DEV_ORIGIN), "");
    s.set_read_timeout(Some(super::policy::AUTH_DEADLINE + Duration::from_secs(5))).unwrap();
    let started = std::time::Instant::now();
    let mut out = Vec::new();
    let r = s.read_to_end(&mut out);
    assert!(r.is_ok(), "expected EOF, got {r:?}");
    assert!(out.is_empty(), "something was written to a silent client: {out:?}");
    assert!(started.elapsed() >= super::policy::AUTH_DEADLINE - Duration::from_millis(200), "cut off too early");
}

/// The RPC round trip, headless: requests go in as the frontend's `invoke`
/// would build them — camelCase keys, a `Channel` spelled Tauri's way — and
/// replies come back by id, out of order when the work finishes out of order.
#[test]
fn requests_are_answered_by_id_with_tauris_shapes() {
    let l = live("rpc");
    let mut ws = client(&l);

    // Nothing stored → None → null, through a temp config dir only.
    assert_eq!(rpc(&mut ws, json!(1), "get_settings", json!({})), json!({ "id": 1, "ok": null }));
    assert_eq!(rpc(&mut ws, json!("s"), "set_settings", json!({ "settings": { "theme": "nord" } })), json!({ "id": "s", "ok": null }));
    let r = rpc(&mut ws, json!(2), "get_settings", json!({}));
    assert_eq!(r["ok"]["theme"], json!("nord"));
    assert!(l.dir.join("ui-state.json").exists(), "the write landed in the Ctx's config dir");

    // camelCase as the frontend spells it.
    let r = rpc(&mut ws, json!(3), "ui_usage", json!({ "days": 7, "tzOffsetMin": 0 }));
    assert_eq!(r["ok"]["days"].as_array().map(Vec::len), Some(7), "{r}");

    // Errors: unknown command, invalid args naming the key, client-local.
    assert_eq!(rpc(&mut ws, json!(4), "nope", json!({}))["err"], json!("unknown command: nope"));
    let r = rpc(&mut ws, json!(5), "set_pin", json!({ "repo": "/r", "slug": "s" }));
    assert!(r["err"].as_str().unwrap().starts_with("invalid args `on`"), "{r}");
    assert_eq!(rpc(&mut ws, json!(6), "copy_text", json!({ "text": "NEVER" }))["err"], json!("client-local"));
    // A frame that is not a request at all is answered, with no id to echo.
    ws.send(Message::text("[1,2,3]")).unwrap();
    let r: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(r["id"], Value::Null);
    assert!(r["err"].as_str().unwrap().starts_with("bad request"), "{r}");

    // Two requests in flight: both answered, each under its own id.
    ws.send(Message::text(json!({ "id": "a", "cmd": "get_changelog" }).to_string())).unwrap();
    ws.send(Message::text(json!({ "id": "b", "cmd": "log_info" }).to_string())).unwrap();
    let mut ids: Vec<Value> = (0..2)
        .map(|_| serde_json::from_str::<Value>(ws.read().unwrap().to_text().unwrap()).unwrap()["id"].clone())
        .collect();
    ids.sort_by_key(|v| v.to_string());
    assert_eq!(ids, vec![json!("a"), json!("b")]);
}

/// `Host::emit` reaches every authenticated socket as `{event, payload}` —
/// and only the authenticated ones.
#[test]
fn an_event_is_broadcast_to_every_authenticated_socket() {
    let l = live("events");
    let mut a = client(&l);
    let mut b = client(&l);
    // A third socket that upgraded but never authenticated must hear nothing.
    let (mut quiet, _) = upgrade(l.port(), Some(&good_host(l.port())), Some(DEV_ORIGIN), "");
    quiet.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

    l.running.as_ref().unwrap().ctx.emit("sessions:busy", json!({ "busy": ["/x"] }));
    for ws in [&mut a, &mut b] {
        let f: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(f, json!({ "event": "sessions:busy", "payload": { "busy": ["/x"] } }));
    }
    let mut buf = [0u8; 64];
    match quiet.read(&mut buf) {
        Ok(0) | Err(_) => {}
        Ok(n) => panic!("an unauthenticated socket received {n} bytes"),
    }
    // A client that has gone is dropped from the broadcast list on the next
    // emit rather than wedging it.
    drop(a);
    std::thread::sleep(Duration::from_millis(100));
    l.running.as_ref().unwrap().ctx.emit("places:changed", Value::Null);
    let f: Value = serde_json::from_str(b.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(f["event"], json!("places:changed"));
    // Keep the runtime alive until here.
    let _ = &l.rt;
}

// ── streams ──────────────────────────────────────────────────────────────────

/// A dock shell needs a git repository to be a place of: the `(main)` of a
/// fresh `git init` is one, with no registration, no tmux and no commit.
fn scratch_repo(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wt-serve-repo-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let st = std::process::Command::new("git").args(["init", "-q"]).arg(&d).status().expect("git on PATH");
    assert!(st.success(), "git init");
    d
}

/// One frame, or `None` when nothing arrived within `wait`. Control frames
/// are tungstenite's business and skipped.
fn frame(ws: &mut WebSocket<TcpStream>, wait: Duration) -> Option<Value> {
    ws.get_ref().set_read_timeout(Some(wait)).unwrap();
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => return Some(serde_json::from_str(t.as_str()).unwrap()),
            Ok(_) => continue,
            Err(tokio_tungstenite::tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
            {
                return None
            }
            Err(e) => panic!("reading a frame: {e}"),
        }
    }
}

/// Read until the socket has been quiet for `quiet`.
fn drain(ws: &mut WebSocket<TcpStream>, quiet: Duration) -> Vec<Value> {
    let mut seen = Vec::new();
    while let Some(v) = frame(ws, quiet) {
        seen.push(v);
    }
    seen
}

fn b64(v: &Value) -> Vec<u8> {
    // RFC 4648, the alphabet `files::b64_encode` writes.
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let s = v.as_str().unwrap().as_bytes();
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for &c in s {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | A.iter().position(|&a| a == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// Open dock shell `index` of the scratch repo on `channel` and leave it
/// quiet: a prompt that paints late would be indistinguishable from an echo.
fn open_quiet_shell(ws: &mut WebSocket<TcpStream>, repo: &str, index: u32, channel: u64) -> u64 {
    let r = rpc(
        ws,
        json!(1000 + index),
        "shell_open",
        json!({ "repo": repo, "slug": "(main)", "index": index, "cols": 80, "rows": 24, "onBytes": format!("__CHANNEL__:{channel}") }),
    );
    let gen = r["ok"]["gen"].as_u64().unwrap_or_else(|| panic!("shell_open: {r}"));
    drain(ws, Duration::from_millis(1500));
    // An empty prompt, so nothing the shell paints can be mistaken for input.
    let line: Vec<u8> = b"PS1=''\r".to_vec();
    rpc(ws, json!(1100 + index), "shell_write", json!({ "repo": repo, "slug": "(main)", "index": index, "data": line }));
    drain(ws, Duration::from_millis(1500));
    gen
}

/// **The keystroke order.** 60 single-byte writes to one dock shell, sent
/// back to back with ascending ids, and the pty's echo — the record of the
/// order the WRITES reached it — read back off the channel. The transport
/// lane measured `0124356789…` for 26 in-order frames against a dispatcher
/// that gave every frame its own blocking task; a lane per stream is what
/// makes the echo the alphabet.
#[test]
fn writes_to_one_stream_reach_the_pty_in_the_order_they_were_sent() {
    std::env::set_var("SHELL", "/bin/sh");
    let l = live("order");
    let repo = scratch_repo("order");
    let repo_s = repo.to_string_lossy().into_owned();
    let mut ws = client(&l);
    open_quiet_shell(&mut ws, &repo_s, 0, 7);

    const ALPHABET: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWX";
    for (i, c) in ALPHABET.bytes().enumerate() {
        let req = json!({ "id": 2000 + i, "cmd": "shell_write", "args": { "repo": repo_s, "slug": "(main)", "index": 0, "data": [c] } });
        ws.send(Message::text(req.to_string())).unwrap();
    }
    // Everything the pty echoed, in channel order, until the socket is quiet.
    let mut echoed = Vec::new();
    let mut replies = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        match frame(&mut ws, Duration::from_millis(800)) {
            Some(v) if v["channel"] == json!(7) && v.get("data").is_some() => echoed.extend(b64(&v["data"])),
            Some(v) if v.get("id").is_some() => replies += 1,
            Some(_) => {}
            None if replies == ALPHABET.len() => break,
            None => {}
        }
    }
    let got: String = echoed.iter().filter(|b| ALPHABET.as_bytes().contains(b)).map(|&b| b as char).collect();
    assert_eq!(replies, ALPHABET.len(), "every write is answered");
    assert_eq!(got, ALPHABET, "the pty saw the writes in this order");

    rpc(&mut ws, json!(3), "close_shell_session", json!({ "repo": repo_s, "slug": "(main)", "index": 0, "keepCwd": false }));
    let _ = std::fs::remove_dir_all(&repo);
}

/// **End of channel.** A `ByteSink` the engine lets go of — a detach, the
/// shell exiting — ends its channel with `{channel, seq, end: true}` on the
/// same queue, so the client can drop the handler and the xterm it closes
/// over. Tauri does this when the Rust `Channel` drops; before this frame a
/// serve client had to keep every channel for the life of the page.
#[test]
fn a_channel_ends_when_its_sink_is_dropped_by_a_detach_or_an_exit() {
    std::env::set_var("SHELL", "/bin/sh");
    let l = live("end");
    let repo = scratch_repo("end");
    let repo_s = repo.to_string_lossy().into_owned();
    let mut ws = client(&l);

    // Detach: the end frame precedes the detach's own reply.
    let gen = open_quiet_shell(&mut ws, &repo_s, 0, 21);
    ws.send(Message::text(json!({ "id": 50, "cmd": "shell_detach", "args": { "repo": repo_s, "slug": "(main)", "index": 0, "gen": gen } }).to_string())).unwrap();
    let mut before_reply = Vec::new();
    loop {
        let v = frame(&mut ws, Duration::from_secs(5)).expect("the detach is answered");
        if v["id"] == json!(50) {
            break;
        }
        before_reply.push(v);
    }
    let end = before_reply.iter().find(|v| v["channel"] == json!(21) && v["end"] == json!(true));
    let end = end.unwrap_or_else(|| panic!("no end frame for channel 21 before the detach reply; saw {before_reply:?}"));
    assert!(end.get("data").is_none(), "an end frame carries no data: {end}");
    assert!(end["seq"].as_u64().is_some(), "an end frame has the channel's next seq: {end}");

    // Exit: the shell goes away on its own; the channel ends BEFORE the
    // `shell:exit` event that announces it, both on the one queue.
    open_quiet_shell(&mut ws, &repo_s, 1, 22);
    let line: Vec<u8> = b"exit\r".to_vec();
    ws.send(Message::text(json!({ "id": 60, "cmd": "shell_write", "args": { "repo": repo_s, "slug": "(main)", "index": 1, "data": line } }).to_string())).unwrap();
    let mut seen = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if let Some(v) = frame(&mut ws, Duration::from_millis(500)) {
            let is_exit = v["event"] == json!("shell:exit");
            seen.push(v);
            if is_exit {
                break;
            }
        }
    }
    let end_at = seen.iter().position(|v| v["channel"] == json!(22) && v["end"] == json!(true));
    let exit_at = seen.iter().position(|v| v["event"] == json!("shell:exit"));
    assert!(exit_at.is_some(), "the shell's exit is announced; saw {seen:?}");
    assert!(end_at.is_some(), "channel 22 ends when its shell exits; saw {seen:?}");
    assert!(end_at < exit_at, "the end frame comes before the event: {seen:?}");

    rpc(&mut ws, json!(70), "close_shell_session", json!({ "repo": repo_s, "slug": "(main)", "index": 0, "keepCwd": false }));
    rpc(&mut ws, json!(71), "close_shell_session", json!({ "repo": repo_s, "slug": "(main)", "index": 1, "keepCwd": false }));
    let _ = std::fs::remove_dir_all(&repo);
}

/// **A dock shell outlives the client that attached it.** A page reload
/// drops the socket while the shell still holds that connection's sink; the
/// next thing the shell prints fails to send. That must cost the dead client
/// its sink, NOT cost the shell its reader — or the shell runs on with
/// nobody reading its pty, and the next client re-attaches to a corpse that
/// never answers again.
#[test]
fn a_dock_shell_keeps_its_reader_when_the_client_that_attached_it_goes() {
    std::env::set_var("SHELL", "/bin/sh");
    let l = live("outlive");
    let repo = scratch_repo("outlive");
    let repo_s = repo.to_string_lossy().into_owned();

    // Client A attaches, starts something that prints after A is gone, goes.
    let mut a = client(&l);
    open_quiet_shell(&mut a, &repo_s, 0, 31);
    let later: Vec<u8> = b"sleep 1; echo printed-while-nobody-was-attached\r".to_vec();
    rpc(&mut a, json!(40), "shell_write", json!({ "repo": repo_s, "slug": "(main)", "index": 0, "data": later }));
    drop(a);
    std::thread::sleep(Duration::from_millis(2500));

    // Client B re-attaches the same tab — the replay is the channel's first
    // frame and precedes the reply — and types.
    let mut b = client(&l);
    let open = json!({ "repo": repo_s, "slug": "(main)", "index": 0, "cols": 80, "rows": 24, "onBytes": "__CHANNEL__:32" });
    b.send(Message::text(json!({ "id": 41, "cmd": "shell_open", "args": open }).to_string())).unwrap();
    let mut out = Vec::new();
    loop {
        let v = frame(&mut b, Duration::from_secs(5)).expect("the re-attach is answered");
        if v["id"] == json!(41) {
            assert!(v["ok"]["gen"].as_u64().is_some(), "re-attach: {v}");
            break;
        }
        if v["channel"] == json!(32) && v.get("data").is_some() {
            out.extend(b64(&v["data"]));
        }
    }
    let typed: Vec<u8> = b"echo still-$((40+2))\r".to_vec();
    b.send(Message::text(json!({ "id": 42, "cmd": "shell_write", "args": { "repo": repo_s, "slug": "(main)", "index": 0, "data": typed } }).to_string())).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        if let Some(v) = frame(&mut b, Duration::from_millis(400)) {
            if v["channel"] == json!(32) && v.get("data").is_some() {
                out.extend(b64(&v["data"]));
                if String::from_utf8_lossy(&out).contains("still-42") {
                    break;
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&out).into_owned();
    // The replay carries what the shell printed while nobody was attached,
    // and the shell answers what B typed: `still-42` exists only as output.
    assert!(text.contains("printed-while-nobody-was-attached\r\n"), "replay: {text:?}");
    assert!(text.contains("still-42"), "the shell answered nothing after its first client left: {text:?}");

    rpc(&mut b, json!(43), "close_shell_session", json!({ "repo": repo_s, "slug": "(main)", "index": 0, "keepCwd": false }));
    let _ = std::fs::remove_dir_all(&repo);
}

// ── the log ──────────────────────────────────────────────────────────────────

/// **No torn lines.** Eight writers, 300 lines each, through the sink's own
/// append path to one file: every line must be whole. The transport lane read
/// `…03:07:38serve: host-admin codex_mcp_statusZ [` in the real log — a
/// `writeln!` on an unbuffered handle is several `write(2)`s, and two threads'
/// pieces interleave. One `write_all` of one formatted `String` is one write.
#[test]
fn log_lines_from_concurrent_writers_are_whole() {
    let dir = std::env::temp_dir().join(format!("wt-serve-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("serve.log");
    const WRITERS: usize = 8;
    const LINES: usize = 300;
    let tail = "x".repeat(200);
    let hs: Vec<_> = (0..WRITERS)
        .map(|w| {
            let (dir, path, tail) = (dir.clone(), path.clone(), tail.clone());
            std::thread::spawn(move || {
                for i in 0..LINES {
                    super::append_line(&dir, &path, "info", &format!("w{w}-{i}-{tail}"));
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), WRITERS * LINES, "one line per call");
    let head_tail = format!("Z [info] [{}] w", std::process::id());
    let mut seen = std::collections::HashSet::new();
    for line in &lines {
        // `<19-char UTC stamp>Z [info] [<pid>] w<writer>-<i>-<200 x>` — anything
        // else is two writers' pieces in one line.
        let (ts, rest) = line.split_at(line.len().min(19));
        assert!(ts.len() == 19 && ts.as_bytes()[4] == b'-' && ts.as_bytes()[10] == b' ', "stamp: {line:?}");
        let body = rest.strip_prefix(&head_tail).unwrap_or_else(|| panic!("torn: {line:?}"));
        let (id, x) = body.rsplit_once('-').unwrap_or_else(|| panic!("torn: {line:?}"));
        assert_eq!(x, tail, "torn: {line:?}");
        assert!(seen.insert(id.to_string()), "twice: {line:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
