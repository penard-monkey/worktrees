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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocket};
use tokio_tungstenite::tungstenite::Message;

use super::budget::BUDGETS;
use super::conn::Limits;
use super::{start_in, Running};

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
    live_with(tag, super::conn::LIMITS)
}

fn live_with(tag: &str, limits: Limits) -> Live {
    let dir = std::env::temp_dir().join(format!("wt-serve-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let clients = super::conn::Clients::with(Arc::new(worktrees_core::authz::NoDevices), Some(Arc::new(Registered)));
    let running = rt.block_on(async { start_in(dir.clone(), 0, vec![DEV_ORIGIN.to_string()], limits, BUDGETS, clients) }).unwrap();
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

// ── handshake v1, written here from the spec — NOT from policy.rs ────────────
//
// `handshake-v1.md` ((main), 2026-10-18): hello → challenge → auth → ok+proof,
// each proof an HMAC-SHA256 keyed with serve.json's token over nonces the
// OTHER side chose, the token itself never on the wire. Everything below is
// restated from that text, so a test cannot agree with the server by sharing
// its code: the labels are spelled out again, and the HMAC is built from the
// bare SHA-256 primitive by RFC 2104 and checked against RFC 4231's vectors.

const SPEC_CLIENT_LABEL: &str = "worktrees-serve/v1/client\n";
const SPEC_SERVER_LABEL: &str = "worktrees-serve/v1/server\n";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// HMAC-SHA256 by RFC 2104, over SHA-256 alone (no `hmac` crate).
fn spec_hmac(key: &[u8], msg: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key)[..]);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new().chain_update(&ipad).chain_update(msg).finalize();
    hex(&Sha256::new().chain_update(&opad).chain_update(&inner[..]).finalize()[..])
}

/// `HMAC(K, label + cn + "\n" + sn)`, K = the token string's UTF-8 bytes.
fn spec_proof(token: &str, label: &str, cn: &str, sn: &str) -> String {
    spec_hmac(token.as_bytes(), format!("{label}{cn}\n{sn}").as_bytes())
}

fn fresh_hex32() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).unwrap();
    hex(&b)
}

fn is_hex32(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The test's own HMAC, checked before anything leans on it: RFC 4231 test
/// case 2, and test case 6 (a key longer than the block, hashed first).
#[test]
fn the_tests_own_hmac_is_rfc_4231s() {
    assert_eq!(spec_hmac(b"Jefe", b"what do ya want for nothing?"), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    assert_eq!(
        spec_hmac(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First"),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}

/// A socket past its 101, with a client-role frame codec on it.
fn upgraded_ws(l: &Live) -> WebSocket<TcpStream> {
    let (s, head) = upgrade(l.port(), Some(&good_host(l.port())), Some(DEV_ORIGIN), "");
    let head = String::from_utf8_lossy(&head).into_owned();
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    WebSocket::from_raw_socket(s, Role::Client, None)
}

fn text<S: Read + Write>(ws: &mut WebSocket<S>) -> Value {
    let m = ws.read().unwrap_or_else(|e| panic!("expected a frame: {e}"));
    serde_json::from_str(m.to_text().unwrap()).unwrap()
}

/// `{"hello": cn}` → `{"challenge": sn}`; returns sn.
fn hello_with<S: Read + Write>(ws: &mut WebSocket<S>, cn: &str) -> String {
    ws.send(Message::text(json!({ "hello": cn }).to_string())).unwrap();
    let c = text(ws);
    assert_eq!(c.as_object().map(|o| o.len()), Some(1), "a challenge is one key: {c}");
    let sn = c["challenge"].as_str().unwrap_or_else(|| panic!("no challenge: {c}")).to_string();
    assert!(is_hex32(&sn), "sn is 32 bytes in lowercase hex: {sn:?}");
    sn
}

fn hello<S: Read + Write>(ws: &mut WebSocket<S>) -> (String, String) {
    let cn = fresh_hex32();
    let sn = hello_with(ws, &cn);
    (cn, sn)
}

/// The rest of the exchange on a socket that has said hello: the client's
/// proof out, the server's ok in — its proof checked the way a client must.
fn prove<S: Read + Write>(ws: &mut WebSocket<S>, token: &str, cn: &str, sn: &str) -> Value {
    ws.send(Message::text(json!({ "auth": spec_proof(token, SPEC_CLIENT_LABEL, cn, sn) }).to_string())).unwrap();
    let ok = text(ws);
    assert_eq!(ok["ok"], json!(true), "{ok}");
    assert_eq!(ok["proof"], json!(spec_proof(token, SPEC_SERVER_LABEL, cn, sn)), "the server's proof does not verify: {ok}");
    ok
}

/// A served, authenticated socket, ready for requests.
fn client(l: &Live) -> WebSocket<TcpStream> {
    let mut ws = upgraded_ws(l);
    let (cn, sn) = hello(&mut ws);
    prove(&mut ws, &l.token(), &cn, &sn);
    ws
}

fn rpc<S: Read + Write>(ws: &mut WebSocket<S>, id: Value, cmd: &str, args: Value) -> Value {
    ws.send(Message::text(json!({ "id": id, "cmd": cmd, "args": args }).to_string())).unwrap();
    text(ws)
}

/// Whatever is left on the raw socket until it closes or `wait` passes:
/// (the bytes, whether it closed). A reset counts as closed.
///
/// macOS refuses `SO_RCVTIMEO` with EINVAL on a socket the peer has already
/// RESET — a test's send after the server's close draws exactly that — and the
/// handshake-deadline test lost that race under a loaded gate run. Such a
/// socket cannot block a read (the next one answers ECONNRESET, measured), so
/// the timeout is moot there, not missing. Anything else is still a failure.
fn rest(s: &mut TcpStream, wait: Duration) -> (Vec<u8>, bool) {
    if let Err(e) = s.set_read_timeout(Some(wait)) {
        assert_eq!(e.raw_os_error(), Some(libc::EINVAL), "set_read_timeout: {e}");
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match s.read(&mut buf) {
            Ok(0) => return (out, true),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return (out, true),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return (out, false),
            Err(e) => panic!("reading what is left: {e}"),
        }
    }
}

/// D6: a refusal closes the socket and writes NOTHING — no frame, no Close.
fn assert_closed_silently(ws: &mut WebSocket<TcpStream>, what: &str) {
    let (bytes, closed) = rest(ws.get_mut(), Duration::from_secs(5));
    assert!(bytes.is_empty(), "{what}: the server wrote {:?}", String::from_utf8_lossy(&bytes));
    assert!(closed, "{what}: the socket is still open");
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

/// **(a) The raw token as a first frame is refused, with nothing said.** It is
/// the frame every v0 client sent; under v1 it would hand the token to
/// whatever holds the port, so the server must not reward it either. Every
/// first frame that is not exactly `{"hello": <hex32>}` goes the same way.
#[test]
fn the_raw_token_as_a_first_frame_is_closed_with_nothing_said() {
    let l = live("rawtoken");
    let firsts = [
        ("the v0 auth frame, with the REAL token", Message::text(json!({ "auth": l.token() }).to_string())),
        ("a request", Message::text(json!({ "id": 1, "cmd": "list_workspace" }).to_string())),
        ("a hello that is not hex32", Message::text(json!({ "hello": "ab" }).to_string())),
        ("a hello in upper case", Message::text(json!({ "hello": fresh_hex32().to_uppercase() }).to_string())),
        ("a hello with company", Message::text(json!({ "hello": fresh_hex32(), "auth": "x" }).to_string())),
        ("a binary hello", Message::binary(json!({ "hello": fresh_hex32() }).to_string().into_bytes())),
        ("a ping", Message::Ping(Vec::new().into())),
    ];
    for (what, first) in firsts {
        let mut ws = upgraded_ws(&l);
        ws.send(first).unwrap();
        assert_closed_silently(&mut ws, what);
    }
}

/// **(b) A wrong client proof is refused with nothing said** — nothing after
/// the challenge, which is the only byte an unproven peer ever gets.
#[test]
fn a_wrong_client_proof_is_closed_with_nothing_said_after_the_challenge() {
    let l = live("badproof");
    let t = l.token();
    let other = "f".repeat(64);
    let cases: Vec<(&str, Box<dyn Fn(&str, &str) -> Value>)> = vec![
        ("keyed with another token", Box::new(|cn, sn| json!({ "auth": spec_proof(&other, SPEC_CLIENT_LABEL, cn, sn) }))),
        ("the SERVER's label: a reflected proof", Box::new(|cn, sn| json!({ "auth": spec_proof(&t, SPEC_SERVER_LABEL, cn, sn) }))),
        ("cn and sn swapped", Box::new(|cn, sn| json!({ "auth": spec_proof(&t, SPEC_CLIENT_LABEL, sn, cn) }))),
        ("the right proof in upper case", Box::new(|cn, sn| json!({ "auth": spec_proof(&t, SPEC_CLIENT_LABEL, cn, sn).to_uppercase() }))),
        ("the raw token where the proof goes", Box::new(|_, _| json!({ "auth": t }))),
        ("a second hello", Box::new(|cn, _| json!({ "hello": cn }))),
        ("the proof with company", Box::new(|cn, sn| json!({ "auth": spec_proof(&t, SPEC_CLIENT_LABEL, cn, sn), "id": 1 }))),
    ];
    for (what, auth) in &cases {
        let mut ws = upgraded_ws(&l);
        let (cn, sn) = hello(&mut ws);
        ws.send(Message::text(auth(&cn, &sn).to_string())).unwrap();
        assert_closed_silently(&mut ws, what);
    }
    // A REPLAY: the right proof for another connection's challenge, with the
    // same client nonce. The server's fresh `sn` is what makes it stale.
    let cn = fresh_hex32();
    let mut first = upgraded_ws(&l);
    let sn1 = hello_with(&mut first, &cn);
    let mut second = upgraded_ws(&l);
    let sn2 = hello_with(&mut second, &cn);
    assert_ne!(sn1, sn2, "a challenge is fresh per connection");
    second.send(Message::text(json!({ "auth": spec_proof(&t, SPEC_CLIENT_LABEL, &cn, &sn1) }).to_string())).unwrap();
    assert_closed_silently(&mut second, "a proof for another connection's challenge");
}

/// **(c) The server's proof is the spec's formula, computed HERE** — by the
/// test's own HMAC with the spec's labels spelled out, never by calling the
/// server's helper — and the reply is exactly `{ok, version, proof}`.
#[test]
fn the_server_proof_is_the_specs_hmac_computed_here_and_the_reply_names_the_version() {
    let l = live("proof");
    let t = l.token();
    let mut ws = upgraded_ws(&l);
    let (cn, sn) = hello(&mut ws);
    ws.send(Message::text(json!({ "auth": spec_proof(&t, SPEC_CLIENT_LABEL, &cn, &sn) }).to_string())).unwrap();
    let ok = text(&mut ws);
    let keys: std::collections::BTreeSet<&str> = ok.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["ok", "proof", "version"].into_iter().collect(), "{ok}");
    assert_eq!(ok["ok"], json!(true));
    assert_eq!(ok["version"], json!(env!("CARGO_PKG_VERSION")), "the version is this binary's");
    let proof = ok["proof"].as_str().unwrap();
    assert!(is_hex32(proof), "64 lowercase hex: {proof}");
    assert_eq!(proof, spec_proof(&t, SPEC_SERVER_LABEL, &cn, &sn), "server_proof = HMAC(K, server label + cn + \\n + sn)");
    assert_ne!(proof, spec_proof(&t, SPEC_CLIENT_LABEL, &cn, &sn), "the two directions' proofs must differ");
    // And the session is a session.
    let r = rpc(&mut ws, json!(1), "get_changelog", json!({}));
    assert_eq!(r["ok"]["version"], json!(env!("CARGO_PKG_VERSION")));
}

/// Records every byte read off the socket: what the SERVER wrote.
struct Tap {
    s: TcpStream,
    seen: Arc<Mutex<Vec<u8>>>,
}
impl Read for Tap {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.s.read(b)?;
        self.seen.lock().unwrap().extend_from_slice(&b[..n]);
        Ok(n)
    }
}
impl Write for Tap {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.s.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.s.flush()
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// **(d) The token never appears in any byte the server writes** — not in the
/// 101, the challenge, the ok, or the replies after it. Read off the raw
/// socket, as hex and as the 32 bytes it encodes.
#[test]
fn the_token_never_appears_in_any_byte_the_server_writes() {
    let l = live("leak");
    let t = l.token();
    let (s, head) = upgrade(l.port(), Some(&good_host(l.port())), Some(DEV_ORIGIN), "");
    let seen = Arc::new(Mutex::new(head));
    let mut ws = WebSocket::from_raw_socket(Tap { s, seen: seen.clone() }, Role::Client, None);
    let (cn, sn) = hello(&mut ws);
    prove(&mut ws, &t, &cn, &sn);
    for (i, cmd) in ["settings_info", "list_workspace", "get_settings", "get_changelog"].into_iter().enumerate() {
        let r = rpc(&mut ws, json!(i), cmd, json!({}));
        assert!(r.get("ok").is_some() || r.get("err").is_some(), "{cmd}: {r}");
    }
    let seen = seen.lock().unwrap();
    assert!(seen.len() > 200, "the tap saw the session: {} bytes", seen.len());
    let raw: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&t[2 * i..2 * i + 2], 16).unwrap()).collect();
    assert!(!contains(&seen, t.as_bytes()), "the token, in hex, went out on the wire");
    assert!(!contains(&seen, t.to_uppercase().as_bytes()), "the token, in upper-case hex, went out on the wire");
    assert!(!contains(&seen, &raw), "the token's 32 bytes went out on the wire");
}

/// **(e) The deadline covers the WHOLE exchange.** Two steps, each well inside
/// `AUTH_DEADLINE`, adding up to more than it: refused, with nothing said
/// after the challenge. A per-frame timeout would let this through. The
/// control first: the same exchange done briskly is served.
#[test]
fn a_handshake_that_outlasts_the_deadline_is_refused_even_when_each_step_is_quick() {
    let l = live("slowhs");
    let t = l.token();
    let d = super::policy::AUTH_DEADLINE;

    let mut ws = upgraded_ws(&l);
    std::thread::sleep(Duration::from_millis(200));
    let (cn, sn) = hello(&mut ws);
    std::thread::sleep(Duration::from_millis(600));
    prove(&mut ws, &t, &cn, &sn);

    let gap = d * 57 / 100;
    let mut ws = upgraded_ws(&l);
    std::thread::sleep(gap);
    let (cn, sn) = hello(&mut ws);
    std::thread::sleep(gap);
    // The server may already have closed; a failed send is part of the answer.
    let _ = ws.send(Message::text(json!({ "auth": spec_proof(&t, SPEC_CLIENT_LABEL, &cn, &sn) }).to_string()));
    assert_closed_silently(&mut ws, "an exchange of two quick steps longer than the deadline");
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
    let repo = scratch_repo("rpc");
    let r = rpc(&mut ws, json!(5), "set_pin", json!({ "repo": repo, "slug": "(main)" }));
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

/// A log sink that keeps its lines. Installed by the one test that reads the
/// log; serve's own sink would write the REAL `~/.cache/worktrees/serve.log`.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl worktrees_api::log::LogSink for Recorder {
    fn log(&self, level: &str, msg: &str) {
        self.0.lock().unwrap().push(format!("[{level}] {msg}"));
    }
}

/// A body that panics is answered — `internal error`, and nothing of the
/// panic — and the socket, the lane it ran on and the server all go on
/// serving. Unanswered, the client waits on that id forever: tokio keeps the
/// panic inside its blocking task, so nothing else ever notices. The log
/// gets the panic on ONE line with the command it came from, its text
/// escaped, since a body can panic over a client's value.
#[test]
fn a_body_that_panics_is_answered_and_everything_after_it_is_served() {
    use super::dispatch::TEST_PANIC;
    let rec = Arc::new(Recorder::default());
    assert!(worktrees_api::log::install(rec.clone()), "a log sink was already installed in this test binary");
    let l = live("panic");
    let mut ws = client(&l);
    let why = "a body fell over\n2026-01-01 00:00:00Z [info] forged";

    // On the concurrent pool, then the next request on the same socket.
    assert_eq!(rpc(&mut ws, json!(1), TEST_PANIC, json!({ "why": why })), json!({ "id": 1, "err": "internal error" }));
    assert_eq!(rpc(&mut ws, json!(2), "get_settings", json!({})), json!({ "id": 2, "ok": null }));

    // On an attach's lane, with a request queued behind it on that lane.
    ws.send(Message::text(json!({ "id": 3, "cmd": TEST_PANIC, "args": { "id": 7, "why": why } }).to_string())).unwrap();
    ws.send(Message::text(json!({ "id": 4, "cmd": "term_write", "args": { "id": 7, "data": [97] } }).to_string())).unwrap();
    assert_eq!(text(&mut ws), json!({ "id": 3, "err": "internal error" }));
    assert_eq!(text(&mut ws), json!({ "id": 4, "err": "no such terminal" }));

    // The server: a new client is served too.
    let mut other = client(&l);
    assert_eq!(rpc(&mut other, json!(5), "get_settings", json!({})), json!({ "id": 5, "ok": null }));

    let lines = rec.0.lock().unwrap().clone();
    // Other tests panic their bodies too, into the same recorder: ours are
    // the lines with this test's own text.
    let ours: Vec<&String> =
        lines.iter().filter(|s| s.contains(&format!("serve: {TEST_PANIC} panicked")) && s.contains("a body fell over")).collect();
    assert_eq!(ours.len(), 2, "one line per panic: {lines:#?}");
    for s in ours {
        assert!(s.starts_with("[error] ") && s.contains("a body fell over") && s.contains("internal error"), "{s}");
        assert!(!s.chars().any(char::is_control), "the panic's text is escaped onto one line: {s:?}");
    }
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

/// gap 12, on the real `Broadcast`: an event reaches each socket PROJECTED to
/// that socket's caller, read from the registry at the moment of the emit.
/// The console hears everything exactly as before; a device scoped to one
/// project hears its members of the busy set and nothing of the other's, and
/// no `app:open-doc` at all — and the inbox waits for a console, since a
/// device alone would drain it into nothing.
#[test]
fn an_event_reaches_each_socket_projected_to_its_caller() {
    use super::conn::{Broadcast, Clients};
    use super::wire::Outbox;
    use worktrees_api::Host;
    use worktrees_core::authz::{Capability, Caps, Caller, DeviceId, Grant, PrincipalId, Projects, Registry};

    struct One(Mutex<Grant>);
    impl Registry for One {
        fn principal_grant(&self, _: &PrincipalId) -> Option<Grant> {
            None
        }
        fn device_grant(&self, p: &PrincipalId, d: &DeviceId) -> Option<Grant> {
            (p.is_owner() && d.as_str() == "phone").then(|| self.0.lock().unwrap().clone())
        }
        fn device_principal(&self, _: &DeviceId) -> Option<PrincipalId> {
            Some(PrincipalId::owner())
        }
        fn stepup_required(&self, _: &PrincipalId, _: &DeviceId) -> bool {
            false
        }
        fn generation(&self) -> u64 {
            0
        }
    }
    struct Two;
    impl super::resolve::World for Two {
        fn roots(&self) -> Vec<String> {
            vec!["/r/a".into(), "/r/b".into()]
        }
        fn panes(&self) -> Option<worktrees_core::tmux::PaneList> {
            None
        }
    }
    let reg = Arc::new(One(Mutex::new(Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::only(["/r/a"]) })));
    let clients = Arc::new(Clients::with(reg.clone(), Some(Arc::new(Two))));
    let host = Broadcast(clients.clone());
    let (ctx_tx, mut console) = tokio::sync::mpsc::unbounded_channel();
    let (ph_tx, mut phone) = tokio::sync::mpsc::unbounded_channel();
    // Both say they listen for `worktrees show`; only the console is shown it.
    clients.add(2, Outbox::unbounded(ph_tx), Caller::paired(PrincipalId::owner(), DeviceId::new("phone"))).declare(["app:open-doc"]);
    assert!(!host.has_audience("app:open-doc"), "a device alone is no audience for the inbox, declared or not");
    clients.add(1, Outbox::unbounded(ctx_tx), Caller::console()).declare(["app:open-doc"]);
    assert!(host.has_audience("app:open-doc"));

    let next = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<Message>| -> Option<Value> {
        rx.try_recv().ok().map(|m| serde_json::from_str(m.to_text().unwrap()).unwrap())
    };
    host.emit("sessions:busy", json!({ "busy": ["/r/a/.worktrees/x", "/r/b"], "waiting": [], "waiting_by": {} }));
    assert_eq!(next(&mut console).unwrap()["payload"]["busy"], json!(["/r/a/.worktrees/x", "/r/b"]));
    assert_eq!(next(&mut phone).unwrap()["payload"]["busy"], json!(["/r/a/.worktrees/x"]));
    host.emit("sessions:done", json!({ "path": "/r/b", "epoch": 1 }));
    assert!(next(&mut console).is_some());
    assert!(next(&mut phone).is_none(), "another project's finished turn");
    host.emit("app:open-doc", json!({ "repo": "/r/a", "slug": "(main)", "path": "/r/a/x.md" }));
    assert!(next(&mut console).is_some());
    assert!(next(&mut phone).is_none(), "worktrees show is for someone at the host");
    // The grant is read per emit: narrowed to nothing, the phone hears nothing
    // more of the busy set, with no reconnect.
    *reg.0.lock().unwrap() = Grant::nothing();
    host.emit("sessions:busy", json!({ "busy": ["/r/a"], "waiting": [], "waiting_by": {} }));
    assert_eq!(next(&mut phone).unwrap()["payload"]["busy"], json!([]));
    assert!(next(&mut console).is_some());
    host.emit("places:changed", Value::Null);
    assert!(next(&mut phone).is_some(), "\"re-read\" goes to everyone: the re-read is scoped");
}

/// §9.3, slice 1b: **a one-shot event goes to a connection that LISTENS for
/// it.** The app's verification probe is a console connection that proves
/// itself and closes, declaring nothing; while it counted as an audience, a
/// held `worktrees show` released in those few ms went into a closing socket.
/// Now it is no audience and is sent no `app:open-doc`, while the app, which
/// declared it, is — and every other event still reaches every socket,
/// declared or not. Each declaration replaces the last.
#[test]
fn a_one_shot_event_goes_only_to_a_console_connection_that_declared_it() {
    use super::conn::{Broadcast, Clients};
    use super::wire::Outbox;
    use worktrees_api::Host;
    use worktrees_core::authz::Caller;

    let clients = Arc::new(Clients::default());
    let host = Broadcast(clients.clone());
    let (probe_tx, mut probe) = tokio::sync::mpsc::unbounded_channel();
    let (app_tx, mut app) = tokio::sync::mpsc::unbounded_channel();
    let next = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<Message>| -> Option<Value> {
        rx.try_recv().ok().map(|m| serde_json::from_str(m.to_text().unwrap()).unwrap())
    };
    let doc = || json!({ "repo": "/r/a", "slug": "(main)", "path": "/r/a/x.md" });

    clients.add(1, Outbox::unbounded(probe_tx), Caller::console());
    assert!(!host.has_audience("app:open-doc"), "a console connection that declared nothing is no audience");
    let listens = clients.add(2, Outbox::unbounded(app_tx), Caller::console());
    listens.declare(["places:changed"]);
    assert!(!host.has_audience("app:open-doc"), "declaring another event is not declaring this one");
    listens.declare(["places:changed", "app:open-doc"]);
    assert!(host.has_audience("app:open-doc"));

    host.emit("app:open-doc", doc());
    assert_eq!(next(&mut app).unwrap()["event"], json!("app:open-doc"));
    assert!(next(&mut probe).is_none(), "a connection that declared nothing was sent a one-shot event");
    host.emit("sessions:busy", json!({ "busy": [], "waiting": [], "waiting_by": {} }));
    assert!(next(&mut probe).is_some() && next(&mut app).is_some(), "a state event goes to every socket, declared or not");

    listens.declare([]);
    assert!(!host.has_audience("app:open-doc"), "a declaration replaces the last");
    host.emit("app:open-doc", doc());
    assert!(next(&mut app).is_none() && next(&mut probe).is_none());
}

/// The listen frame over a real socket: read in order with the requests
/// around it, never answered, unknown names dropped without a word, a
/// malformed one refused like an unreadable request (and the set left as it
/// was), and a connection that goes takes its declaration with it.
#[test]
fn a_listen_frame_is_read_in_order_and_never_answered() {
    let l = live("listen");
    let host = l.running.as_ref().unwrap().ctx.host.clone();
    let mut ws = client(&l);
    assert!(!host.has_audience("app:open-doc"), "authenticated, and declared nothing");

    ws.send(Message::text(json!({ "listen": ["no:such-event", "app:open-doc"] }).to_string())).unwrap();
    let r = rpc(&mut ws, json!(1), "get_changelog", json!({}));
    assert!(r["id"] == json!(1) && r.get("ok").is_some(), "the next frame answers the request behind it: {r}");
    assert!(host.has_audience("app:open-doc"), "read before the request that followed it was answered");
    host.emit("app:open-doc", json!({ "repo": "/r", "slug": "(main)", "path": "/r/x.md" }));
    assert_eq!(text(&mut ws)["event"], json!("app:open-doc"));

    ws.send(Message::text(json!({ "listen": "app:open-doc" }).to_string())).unwrap();
    let e = text(&mut ws);
    assert!(e["id"].is_null() && e["err"].as_str().is_some_and(|s| s.starts_with("bad listen")), "{e}");
    assert!(host.has_audience("app:open-doc"), "a frame serve could not read changes nothing");

    drop(ws);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while host.has_audience("app:open-doc") {
        assert!(std::time::Instant::now() < deadline, "a closed connection still counts as an audience");
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = &l.rt;
}

// ── limits (OWASP M1) ────────────────────────────────────────────────────────

fn idle_tcp(l: &Live) -> TcpStream {
    TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, l.port())), Duration::from_secs(5)).unwrap()
}

/// **Before auth a peer may send a few hundred bytes, not 16 MiB.** The
/// review drove RSS from 21 MB to 277 MB with six unauthenticated sockets,
/// each part-way through a 60 MiB message. A frame header claiming 1 MiB and
/// 4 KiB of its payload must end the connection AT ONCE, with nothing said —
/// not at the auth deadline, by which time the payload has been buffered.
#[test]
fn a_preauth_frame_past_the_byte_budget_is_closed_at_once() {
    let l = live("budget");
    let ws = upgraded_ws(&l);
    let mut s = ws.into_inner();
    let mut frame = vec![0x81u8, 0x80 | 127];
    frame.extend_from_slice(&(1u64 << 20).to_be_bytes());
    frame.extend_from_slice(&[1, 2, 3, 4]);
    frame.extend(std::iter::repeat_n(b'a', 4096));
    let started = std::time::Instant::now();
    let _ = s.write_all(&frame);
    let (bytes, closed) = rest(&mut s, Duration::from_secs(5));
    let took = started.elapsed();
    assert!(bytes.is_empty(), "the server wrote {:?}", String::from_utf8_lossy(&bytes));
    assert!(closed, "the socket is still open");
    assert!(took < Duration::from_secs(1), "closed after {took:?}: the budget did not stop it, the auth deadline did");
}

/// **Unauthenticated connections are capped, and the cap spares the client
/// that is already in.** Past the cap a new connection DISPLACES the stalest
/// unauthenticated one, which is closed with nothing said, oldest first; the
/// authenticated client keeps getting answers throughout; and a new client
/// gets in while the cap is still full. (Under M1 the newcomer was the one
/// closed, which is how a reconnecting flood kept the door shut.)
#[test]
fn connections_past_the_unauthenticated_cap_displace_the_stalest_and_a_served_client_keeps_working() {
    let l = live_with("unauthcap", Limits { connections: 32, unauthenticated: 4 });
    let mut served = client(&l);
    let mut held: Vec<TcpStream> = Vec::new();
    for _ in 0..4 {
        held.push(idle_tcp(&l));
        // Distinct accept times, so "stalest" names exactly one.
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    for i in 0..4 {
        let extra = idle_tcp(&l);
        let (bytes, closed) = rest(&mut held[i], Duration::from_secs(1));
        assert!(bytes.is_empty() && closed, "the stalest, #{i}, displaced by a newcomer: wrote {bytes:?}, closed {closed}");
        for later in &mut held[i + 1..] {
            later.set_nonblocking(true).unwrap();
            let mut b = [0u8; 1];
            assert!(matches!(later.read(&mut b), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "only the stalest goes");
            later.set_nonblocking(false).unwrap();
        }
        held.push(extra);
    }
    assert_eq!(rpc(&mut served, json!(1), "get_settings", json!({})), json!({ "id": 1, "ok": null }), "the served client is still served");
    let mut late = client(&l);
    assert_eq!(rpc(&mut late, json!(2), "get_settings", json!({}))["id"], json!(2), "a new client gets in with the cap still full");
}

/// **All connections are capped too**, authenticated ones included: past the
/// cap even a well-behaved newcomer is closed with nothing said, until one of
/// the connections ends.
#[test]
fn connections_past_the_total_cap_are_closed() {
    let l = live_with("conncap", Limits { connections: 3, unauthenticated: 3 });
    let mut ins: Vec<WebSocket<TcpStream>> = (0..3).map(|_| client(&l)).collect();
    let mut fourth = idle_tcp(&l);
    let (bytes, closed) = rest(&mut fourth, Duration::from_secs(1));
    assert!(bytes.is_empty() && closed, "a fourth connection past a cap of three: wrote {bytes:?}, closed {closed}");
    assert_eq!(rpc(&mut ins[0], json!(1), "get_settings", json!({}))["id"], json!(1));
    drop(ins.pop());
    std::thread::sleep(Duration::from_millis(300));
    let mut next = client(&l);
    assert_eq!(rpc(&mut next, json!(2), "get_settings", json!({}))["id"], json!(2), "a slot frees when a connection ends");
}

/// One text message as masked frames of `chunk` bytes: a first frame and
/// continuations. A browser sends one frame per message; a hostile client
/// need not, which is why the MESSAGE cap is separate from the frame cap.
fn send_fragmented(s: &mut TcpStream, payload: &[u8], chunk: usize) {
    let parts: Vec<&[u8]> = payload.chunks(chunk).collect();
    let mask = [7u8, 1, 3, 9];
    for (i, part) in parts.iter().enumerate() {
        let fin = if i + 1 == parts.len() { 0x80 } else { 0 };
        let op = if i == 0 { 0x1 } else { 0x0 };
        let mut f = vec![fin | op];
        let n = part.len();
        if n < 126 {
            f.push(0x80 | n as u8);
        } else if n < 65_536 {
            f.push(0x80 | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            f.push(0x80 | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
        f.extend_from_slice(&mask);
        f.extend(part.iter().enumerate().map(|(j, b)| b ^ mask[j % 4]));
        if s.write_all(&f).is_err() {
            return; // the server has hung up: that is the answer
        }
    }
}

/// **An authenticated message is capped at `MAX_MESSAGE`, with headroom over
/// the largest legitimate one.** A 1 MB paste (bytes as a JSON array, ~3.5
/// MiB) is served; a message one byte past the cap, sent as 1 MiB frames so
/// that no single frame is large, ends the connection unanswered.
/// (tungstenite's defaults already refused a FRAME past 16 MiB; its MESSAGE
/// default was 64 MiB, and that is the one this pins.)
#[test]
fn a_message_past_the_post_auth_cap_ends_the_connection_and_a_large_paste_is_served() {
    let l = live("msgcap");
    let mut ws = client(&l);
    ws.get_ref().set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let paste: Vec<u8> = (0..1_000_000u32).map(|i| b' ' + (i % 90) as u8).collect();
    let r = rpc(&mut ws, json!(1), "shell_write", json!({ "repo": "/nowhere", "slug": "s", "index": 0, "data": paste }));
    assert_eq!(r["id"], json!(1), "a 1 MB paste is answered: {r}");

    let cap = super::conn::MAX_MESSAGE;
    let head = r#"{"id":2,"cmd":"get_settings","args":{"pad":""#;
    let tail = r#""}}"#;
    let mut big = String::with_capacity(cap + 1);
    big.push_str(head);
    big.extend(std::iter::repeat_n('x', cap + 1 - head.len() - tail.len()));
    big.push_str(tail);
    assert_eq!(big.len(), cap + 1);
    send_fragmented(ws.get_mut(), big.as_bytes(), 1 << 20);
    match ws.read() {
        Ok(Message::Text(t)) => panic!("a message past the cap was answered: {}", &t.as_str()[..t.as_str().len().min(120)]),
        Ok(other) => assert!(matches!(other, Message::Close(_)), "{other:?}"),
        Err(_) => {}
    }
}

/// **The HTTP phase is short.** A TCP connection that never sends a byte holds
/// a task and a descriptor until the handshake deadline: five seconds, no
/// longer (it was twenty).
#[test]
fn a_connection_that_never_speaks_is_closed_within_five_seconds() {
    let l = live("httpidle");
    let mut s = idle_tcp(&l);
    let started = std::time::Instant::now();
    let (bytes, closed) = rest(&mut s, Duration::from_secs(7));
    assert!(bytes.is_empty(), "wrote {bytes:?}");
    assert!(closed, "still open after {:?}", started.elapsed());
    assert!(started.elapsed() <= Duration::from_millis(5600), "closed after {:?}", started.elapsed());
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
    register(&d.to_string_lossy());
    d
}

/// The projects these servers know, as if registered — the user's real
/// registry is never read or written by a test. Only ever ADDED to, and asked
/// only "is this one in it", so no test depends on another's entries (a
/// process-global list that a test asserted EXACTLY would be order-dependent).
static REGISTERED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn register(root: &str) {
    REGISTERED.lock().unwrap().push(root.to_string());
}

/// The world every `live()` server resolves against: `REGISTERED`, and no
/// tmux server at all.
struct Registered;

impl super::resolve::World for Registered {
    fn roots(&self) -> Vec<String> {
        REGISTERED.lock().unwrap().clone()
    }
    fn panes(&self) -> Option<worktrees_core::tmux::PaneList> {
        None
    }
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

/// `--owner` takes the one owner there is and nothing else: a typo must be an
/// error at launch, not a serve.json the app then adopts (or fails to).
#[test]
fn the_owner_flag_takes_app_and_nothing_else() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let o = super::parse_args(&s(&["--config-dir", "/tmp/x", "--owner", "app"])).unwrap().unwrap();
    assert_eq!(o.owner.as_deref(), Some("app"));
    let o = super::parse_args(&s(&["--config-dir", "/tmp/x"])).unwrap().unwrap();
    assert_eq!(o.owner, None, "no flag, no owner: the CLI's and the dev stack's serve");
    for bad in [&["--owner", "App"][..], &["--owner", "cli"], &["--owner", ""], &["--owner"]] {
        assert!(super::parse_args(&s(bad)).is_err(), "{bad:?} must be refused");
    }
}

// ── the idle stop ────────────────────────────────────────────────────────────
// A serve the app started outlived an app crash with no client, indefinitely;
// a downgraded app then ran a second engine beside it on one config dir, and
// the orphan's flush overwrote the dock ring the old app had just saved
// (rehearsal D7).

/// Only the app's serve stops on its own. One somebody started by hand — the
/// CLI, the dev stack — has no owner to come back for it, and never does.
#[test]
fn only_a_serve_the_app_owns_has_an_idle_grace() {
    assert_eq!(super::idle_grace(Some("app")), Some(super::OWNED_IDLE_GRACE));
    assert_eq!(super::idle_grace(None), None, "a serve with no owner must never stop on its own");
    // Long enough that a crash → relaunch normally finds it and adopts it.
    assert!(super::OWNED_IDLE_GRACE >= Duration::from_secs(60), "{:?}", super::OWNED_IDLE_GRACE);
}

/// The rule, on a scripted clock: the grace runs only while there is NO
/// client, and any client starts it over.
#[test]
fn the_idle_grace_runs_only_while_no_client_is_connected() {
    use super::idle_step;
    let g = Duration::from_secs(120);
    let t0 = std::time::Instant::now();
    let at = |s: u64| t0 + Duration::from_secs(s);
    assert_eq!(idle_step(None, 1, at(0), g), (None, false), "a client is connected: no clock");
    let (since, out) = idle_step(None, 0, at(5), g);
    assert_eq!((since, out), (Some(at(5)), false), "the first count with none starts the clock");
    assert_eq!(idle_step(since, 0, at(124), g), (Some(at(5)), false), "short of the grace");
    assert_eq!(idle_step(since, 0, at(125), g), (Some(at(5)), true), "the grace has run out");
    let (since, out) = idle_step(since, 1, at(60), g);
    assert_eq!((since, out), (None, false), "a client in between resets it");
    let (since, _) = idle_step(since, 0, at(65), g);
    assert!(!idle_step(since, 0, at(184), g).1);
    assert!(idle_step(since, 0, at(185), g).1);
}

/// Wired to the real count: an authenticated client holds the serve up for as
/// long as it stays, and once it leaves the watch notifies the stop it was
/// given — which `cmd_serve` gives it as the signals' own.
#[test]
fn the_idle_watch_stops_a_serve_its_last_client_left() {
    let l = live("idle");
    let clients = l.running.as_ref().unwrap().clients.clone();
    let until = |n: usize| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while clients.len() != n {
            assert!(std::time::Instant::now() < deadline, "clients stayed at {}, not {n}", clients.len());
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let mut ws = client(&l);
    until(1);
    let stop = Arc::new(tokio::sync::Notify::new());
    let grace = Duration::from_millis(200);
    l.rt.spawn(super::idle_watch(clients.clone(), grace, Duration::from_millis(20), stop.clone()));
    let fired = |wait: Duration| l.rt.block_on(async { tokio::time::timeout(wait, stop.notified()).await.is_ok() });
    assert!(!fired(grace * 5), "stopped with a client connected");
    let _ = ws.close(None);
    drop(ws);
    until(0);
    assert!(fired(Duration::from_secs(5)), "no stop once the last client had gone for the grace");
}

// ── the audit log (Phase 2 §5) ───────────────────────────────────────────────

/// `audit.jsonl`'s lines, parsed.
fn audit_lines(l: &Live) -> Vec<Value> {
    std::fs::read_to_string(l.dir.join(super::audit::FILE))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

/// The first audit record `pred` accepts, waited for (at most 5 s): a record
/// a connection writes as it goes lands after the client has seen it close.
fn audit_record(l: &Live, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let recs = audit_lines(l);
        if let Some(r) = recs.iter().find(|r| pred(r)) {
            return r.clone();
        }
        assert!(std::time::Instant::now() < deadline, "no such record: {recs:#?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The audit unit tests pass with every call in `conn.rs` deleted; this is
/// whether the socket WRITES them. A refused upgrade and a wrong proof (no
/// actor, the peer named), then a served client: a read (no record), a write
/// (a record, its content nowhere), a client-local row, an unknown command
/// and a stale close (refusals), and the listener's start and stop around it
/// all — one chain.
#[test]
fn the_audit_log_records_what_reaches_the_socket() {
    let l = live("audit");
    let (_s, head) = upgrade(l.port(), Some(&good_host(l.port())), Some("http://evil.example"), "");
    assert!(head.is_empty(), "{}", String::from_utf8_lossy(&head));
    let mut bad = upgraded_ws(&l);
    let (cn, sn) = hello(&mut bad);
    bad.send(Message::text(json!({ "auth": spec_proof("not-the-token", SPEC_CLIENT_LABEL, &cn, &sn) }).to_string())).unwrap();
    assert_closed_silently(&mut bad, "a wrong proof");

    let mut ws = client(&l);
    assert_eq!(rpc(&mut ws, json!(1), "get_settings", json!({}))["ok"], Value::Null);
    assert_eq!(rpc(&mut ws, json!(2), "set_settings", json!({ "settings": { "theme": "a-secret-theme" } }))["ok"], Value::Null);
    assert_eq!(rpc(&mut ws, json!(3), "copy_text", json!({ "text": "NEVER" }))["err"], json!("client-local"));
    assert!(rpc(&mut ws, json!(4), "nope", json!({}))["err"].is_string());
    // A close of an attach this connection never opened: `ok` on the wire,
    // a refused act in the log (§5.1).
    assert_eq!(rpc(&mut ws, json!(5), "term_close", json!({ "id": 987_654 }))["ok"], Value::Null);
    l.running.as_ref().unwrap().listener.stop();

    let recs = audit_lines(&l);
    let find = |pred: &dyn Fn(&Value) -> bool| recs.iter().find(|r| pred(r)).cloned().unwrap_or_else(|| panic!("{recs:#?}"));
    assert_eq!(recs[0]["detail"]["reason"], json!("genesis"));
    assert_eq!(recs[1]["detail"]["event"], json!("started"));
    assert_eq!(recs[1]["detail"]["address"], json!(format!("127.0.0.1:{}", l.port())));
    assert_eq!(recs.last().unwrap()["detail"]["event"], json!("stopped"));

    let origin = find(&|r| r["detail"]["class"] == "upgrade" && r["detail"]["episode"] == "began");
    assert_eq!((origin["detail"]["reason"].as_str(), origin["peer"].as_str()), (Some("origin"), Some("127.0.0.1")), "{origin}");
    assert_eq!((origin["principal"].clone(), origin["device"].clone()), (Value::Null, Value::Null), "{origin}");
    let auth = find(&|r| r["kind"] == "auth" && r["detail"]["episode"] == "began");
    assert_eq!(auth["detail"]["reason"], json!("handshake"), "{auth}");

    let write = find(&|r| r["cmd"] == "set_settings");
    assert_eq!((write["kind"].as_str(), write["outcome"].as_str()), (Some("call"), Some("ok")), "{write}");
    assert_eq!((write["principal"].as_str(), write["device"].as_str()), (Some("owner"), Some("loopback")), "{write}");
    assert!(write["conn"].as_u64().is_some(), "{write}");
    assert_eq!(find(&|r| r["cmd"] == "copy_text")["detail"]["reason"], json!("client-local"));
    assert_eq!(find(&|r| r["cmd"] == "unknown")["detail"]["len"], json!(4));
    let stale = find(&|r| r["cmd"] == "term_close");
    assert_eq!((stale["kind"].as_str(), stale["detail"]["reason"].as_str()), (Some("refused"), Some("not-owned")), "{stale}");
    assert_eq!((stale["detail"]["answered"].as_str(), stale["object"].as_str()), (Some("ok"), Some("987654")), "{stale}");
    assert!(recs.iter().all(|r| r["cmd"] != "get_settings"), "a read was recorded: {recs:#?}");

    let text = std::fs::read_to_string(l.dir.join(super::audit::FILE)).unwrap();
    assert!(!text.contains("a-secret-theme") && !text.contains("NEVER") && !text.contains("not-the-token"), "{text}");
    assert_eq!(super::audit::verify(&l.dir).broken, None);
}

/// The limits, in the log: a run of newcomers at the cap — each displacing
/// the stalest unauthenticated connection, which closes with nothing written
/// — is ONE record when it begins and one when the flood lets go, with the
/// count, not a line per connection, which would turn a flood into a
/// disk-writing flood. A message past `MAX_MESSAGE` is a refusal naming its
/// actor.
#[test]
fn the_audit_log_records_limits_once_per_episode() {
    let l = live_with("auditcap", Limits { connections: 32, unauthenticated: 2 });
    // Every connection apart by more than the progress clock's millisecond,
    // so the oldest one left is the stalest, never a tie.
    let gap = || std::thread::sleep(Duration::from_millis(20));
    let mut idle = std::collections::VecDeque::new();
    for _ in 0..2 {
        idle.push_back(idle_tcp(&l));
        gap();
    }
    for _ in 0..5 {
        let newcomer = idle_tcp(&l);
        let mut stalest = idle.pop_front().unwrap();
        let (bytes, closed) = rest(&mut stalest, Duration::from_secs(2));
        assert!(bytes.is_empty() && closed, "the stalest is displaced, silently");
        idle.push_back(newcomer);
        gap();
    }
    drop(idle);
    std::thread::sleep(Duration::from_millis(300));
    let mut ws = client(&l);
    ws.get_ref().set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let cap = super::conn::MAX_MESSAGE;
    send_fragmented(ws.get_mut(), &vec![b' '; cap + 1], 1 << 20);
    let _ = ws.read();

    let recs = audit_lines(&l);
    let limit: Vec<&Value> = recs.iter().filter(|r| r["detail"]["class"] == "limit").collect();
    assert_eq!(limit.len(), 2, "one record as the cap is reached, one as it clears: {limit:#?}");
    assert_eq!((limit[0]["detail"]["episode"].as_str(), limit[0]["detail"]["reason"].as_str()), (Some("began"), Some("unauthenticated")));
    assert_eq!((limit[1]["detail"]["episode"].as_str(), limit[1]["detail"]["count"].as_u64()), (Some("ended"), Some(5)));
    let msg = recs.iter().find(|r| r["detail"]["limit"] == "message_size").unwrap_or_else(|| panic!("{recs:#?}"));
    assert_eq!((msg["kind"].as_str(), msg["principal"].as_str(), msg["device"].as_str()), (Some("refused"), Some("owner"), Some("loopback")));
}

/// The device registry reaches the log through serve's own wiring
/// (`conn::start`), not a test's: a device in the registry before serve
/// started, with no enrollment in the chain, is disabled by the boot
/// cross-check (§5.2) and listed so in the first checkpoint; its revoke and
/// a knock by its revoked key are records, in one chain with the listener's.
#[test]
fn the_audit_log_records_the_device_registry() {
    use super::devices::{Enrollment, Expiry, Platform};
    use worktrees_core::authz::{Caller, Capability, Caps, Grant, PrincipalId, Projects};
    let dir = std::env::temp_dir().join(format!("wt-serve-auditdev-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let clients = super::conn::Clients::with(Arc::new(worktrees_core::authz::NoDevices), Some(Arc::new(Registered)));
    let reg = clients.devices();
    let now = worktrees_core::sysclock::now_epoch();
    let pin = "c".repeat(64);
    let e = Enrollment {
        principal: PrincipalId::owner(),
        name: "Mac".into(),
        platform: Platform::Macos,
        spki_sha256: pin.clone(),
        stepup_spki_sha256: None,
        grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All },
        expiry: Expiry::default(),
        addr: None,
    };
    let mac = reg.enroll(e, now).unwrap();
    let running = rt.block_on(async { start_in(dir.clone(), 0, vec![DEV_ORIGIN.to_string()], super::conn::LIMITS, BUDGETS, clients) }).unwrap();
    let l = Live { running: Some(running), dir, rt };
    reg.revoke(&Caller::console(), &Grant::everything(), &mac.id, now).unwrap();
    assert!(reg.admit(&pin, now).is_err());
    l.running.as_ref().unwrap().listener.stop();

    let recs = audit_lines(&l);
    let listed = &recs[0]["detail"]["devices"];
    assert_eq!((listed[0]["device"].clone(), listed[0]["spki_sha256"].clone()), (json!(mac.id), json!(pin)), "{}", recs[0]);
    assert_eq!(listed[0]["disabled"]["reason"], json!("unrecorded"), "checked before the first checkpoint: {}", recs[0]);
    assert!(recs.iter().any(|r| r["detail"]["event"] == "disabled" && r["detail"]["reason"] == "unrecorded"), "{recs:#?}");
    let revoked = recs.iter().find(|r| r["kind"] == "device" && r["detail"]["event"] == "revoked").unwrap_or_else(|| panic!("{recs:#?}"));
    assert_eq!(revoked["detail"]["subject"], json!({ "principal": "owner", "device": mac.id }));
    assert_eq!((revoked["principal"].as_str(), revoked["device"].as_str()), (Some("owner"), Some("loopback")));
    let knock = recs.iter().find(|r| r["kind"] == "auth").unwrap_or_else(|| panic!("{recs:#?}"));
    assert_eq!((knock["detail"]["reason"].as_str(), knock["detail"]["subject"]["device"].clone()), (Some("revoked"), json!(mac.id)));
    assert!(recs.iter().any(|r| r["kind"] == "alert" && r["detail"]["event"] == "connect_refused"), "{recs:#?}");
    assert_eq!(super::audit::verify(&l.dir).broken, None);
}

/// §5.2 at boot, on serve's own start path (`start_with`, `devices.json` in
/// the config dir): a device the chain never enrolled — written into the
/// file by hand between two runs — is disabled `unrecorded` before anything
/// can serve it, says so in the log and on disk, and is refused at the
/// handshake and on every request; a device the chain did enroll is served.
#[test]
fn a_device_the_chain_never_enrolled_is_disabled_at_boot_and_never_served() {
    use super::devices::{ConnectRefusal, DevicesFile, Enrollment, Expiry, Platform, FILE};
    use worktrees_core::authz::{Capability, Caps, DeviceId, Grant, PrincipalId, Projects, Registry};
    let dir = std::env::temp_dir().join(format!("wt-serve-boot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let now = worktrees_core::sysclock::now_epoch();
    let boot = || rt.block_on(async { super::start_with(dir.clone(), 0, vec![DEV_ORIGIN.to_string()], super::conn::LIMITS) }).unwrap();

    // The first run pairs a device: its enrollment is in the chain.
    let first = boot();
    let e = Enrollment {
        principal: PrincipalId::owner(),
        name: "Mac".into(),
        platform: Platform::Macos,
        spki_sha256: "c".repeat(64),
        stepup_spki_sha256: None,
        grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All },
        expiry: Expiry::default(),
        addr: None,
    };
    let real = first.devices.as_ref().unwrap().enroll(e, now).unwrap();
    first.listener.stop();
    drop(first);

    // Between runs, a same-user process adds a device of its own.
    let path = dir.join(FILE);
    let mut f = DevicesFile::parse(&std::fs::read(&path).unwrap()).unwrap();
    let mut forged = f.devices[0].clone();
    (forged.id, forged.spki_sha256, forged.caps, forged.projects) = (DeviceId::new("d_forged"), "e".repeat(64), Caps::ALL, Projects::All);
    f.devices.push(forged);
    std::fs::write(&path, serde_json::to_vec_pretty(&f).unwrap()).unwrap();

    let second = boot();
    let reg = second.devices.clone().unwrap();
    assert_eq!(reg.admit(&"e".repeat(64), now).err(), Some(ConnectRefusal::Disabled), "refused at the handshake");
    assert_eq!(reg.device_grant(&PrincipalId::owner(), &DeviceId::new("d_forged")), None, "and on every request");
    assert!(reg.admit(&"c".repeat(64), now).is_ok(), "the enrolled device is served");
    assert!(reg.device_grant(&PrincipalId::owner(), &real.id).is_some());
    let on_disk = DevicesFile::parse(&std::fs::read(&path).unwrap()).unwrap();
    let d = on_disk.device(&DeviceId::new("d_forged")).unwrap();
    assert_eq!(d.disabled.map(|x| x.reason), Some(super::devices::DisabledReason::Unrecorded), "written back");
    second.listener.stop();
    let text = std::fs::read_to_string(dir.join(super::audit::FILE)).unwrap();
    let recs: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let flagged = recs.iter().find(|r| r["detail"]["event"] == "disabled").unwrap_or_else(|| panic!("{recs:#?}"));
    assert_eq!((flagged["detail"]["subject"]["device"].as_str(), flagged["detail"]["reason"].as_str()), (Some("d_forged"), Some("unrecorded")));
    let started = recs.iter().rposition(|r| r["detail"]["event"] == "started").unwrap();
    let at = recs.iter().position(|r| r["detail"]["event"] == "disabled").unwrap();
    assert!(at < started, "flagged before the listener's start record");
    assert!(recs.iter().any(|r| r["kind"] == "alert" && r["detail"]["event"] == "disabled"), "an alert (§5.3)");
    assert_eq!(super::audit::verify(&dir).broken, None);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── paired devices on the real client set (Phase 2 slice 5) ──────────────────
//
// No remote listener exists before slice 6, so a "device" here is
// `conn::serve_caller` — exactly what that listener will run once mTLS has
// named the device — driven over a loopback socket pair with a paired caller,
// on the REAL `Clients`, listening to a REAL `DeviceRegistry`.

mod devices {
    use super::super::conn::{self, Broadcast, Clients};
    use super::super::pairing::Pairing;
    use super::super::devices::{DeviceRecord, DeviceRegistry, Enrollment, Expiry, Platform};
    use super::super::dispatch::{self, Args, Call};
    use super::super::resolve::World;
    use super::super::wire::{Frames, Outbox, ShellHold};
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::net::TcpStream;
    use tokio_tungstenite::tungstenite::protocol::Role;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::WebSocketStream;
    use worktrees_api::Ctx;
    use worktrees_core::authz::{self, Capability, Caps, Caller, DeviceId, Grant, PrincipalId, Projects, Registry};
    use Capability::*;

    const NOW: i64 = 1_800_000_000;

    /// Real directories for the registered roots: the resolver canonicalizes.
    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-serve-dev-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for p in ["a", "b", "cfg"] {
            std::fs::create_dir_all(d.join(p)).unwrap();
        }
        std::fs::canonicalize(&d).unwrap()
    }

    fn s(p: &std::path::Path) -> String {
        p.to_string_lossy().into_owned()
    }

    struct Roots {
        roots: Vec<String>,
        panes: Vec<(String, String, String)>,
    }

    impl World for Roots {
        fn roots(&self) -> Vec<String> {
            self.roots.clone()
        }
        fn panes(&self) -> Option<worktrees_core::tmux::PaneList> {
            Some(worktrees_core::tmux::PaneList::from_rows(self.panes.clone()))
        }
    }

    fn enroll(reg: &DeviceRegistry, name: &str, key: u8, caps: &[Capability], projects: Projects) -> DeviceRecord {
        reg.enroll(
            Enrollment {
                principal: PrincipalId::owner(),
                name: name.into(),
                platform: Platform::Macos,
                spki_sha256: super::super::devices::hex(&[key; 32]),
                stepup_spki_sha256: None,
                grant: Grant { capabilities: Caps::of(caps), projects },
                expiry: Expiry::default(),
                addr: None,
            },
            NOW,
        )
        .unwrap()
    }

    fn caller(d: &DeviceRecord) -> Caller {
        Caller::paired(d.principal.clone(), d.id.clone())
    }

    /// The engine as serve builds it, over a registry in memory and a world of
    /// fixtures: the client set listens to the registry, the `Ctx` emits
    /// through it.
    struct Engine {
        rt: tokio::runtime::Runtime,
        reg: Arc<DeviceRegistry>,
        clients: Arc<Clients>,
        ctx: Ctx,
        dir: PathBuf,
        next: AtomicU64,
        /// Every frame `rpc` skipped on the way to a reply: the events.
        skipped: Mutex<Vec<Value>>,
        /// Each session's task, by connection id, to see it end.
        tasks: Mutex<Vec<(u64, tokio::task::JoinHandle<()>)>>,
        /// The audit log, in the engine's own config dir.
        audit: Arc<super::super::audit::Audit>,
        /// Each client's own address, to its connection id ([`Engine::ended`]).
        ids: Mutex<std::collections::HashMap<std::net::SocketAddr, u64>>,
    }

    /// How long a test waits for a frame it is owed before calling it a hang.
    /// NOT a bound on serve: the rule a close is held to is that the act made
    /// it ([`Engine::ended`]), and how soon the queued frame then crosses a
    /// loopback socket is the scheduler's — a starved, saturated host took
    /// 1.47 s once. The one wall-clock bound is the revoke test's (§10).
    const HANG: Duration = Duration::from_secs(10);

    impl Drop for Engine {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// The pairing deadlines of the one test that waits for them: the real
    /// 120 s and 60 s would make it take minutes. Every other test runs with
    /// [`PAIR_LONG`], so a slow runner cannot time a handshake out.
    const PAIR_OPEN: Duration = Duration::from_millis(1500);
    const PAIR_DECIDE: Duration = Duration::from_millis(1500);
    const PAIR_LONG: Duration = Duration::from_secs(30);

    fn engine(tag: &str, panes: Vec<(String, String, String)>) -> Engine {
        engine_times(tag, panes, PAIR_LONG, PAIR_LONG, false)
    }

    /// With `on_disk`, the registry is `devices.json` in the engine's config
    /// dir, as serve's is.
    fn engine_times(tag: &str, panes: Vec<(String, String, String)>, open_for: Duration, decide_for: Duration, on_disk: bool) -> Engine {
        let dir = scratch(tag);
        let reg = Arc::new(if on_disk { DeviceRegistry::open(&dir.join("cfg")) } else { DeviceRegistry::in_memory() });
        let world = Roots { roots: vec![s(&dir.join("a")), s(&dir.join("b"))], panes };
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();

        // Inside the runtime: a pairing window runs on the one it was made in.
        let clients = {
            let _in = rt.enter();
            let pairing = Arc::new(Pairing::with_times(Some(dir.join("cfg")), reg.clone(), open_for, decide_for));
            Arc::new(Clients::serving(reg.clone(), pairing, Some(Arc::new(world))))
        };
        clients.listen();
        let ctx = Ctx::new(dir.join("cfg"), Arc::new(Broadcast(clients.clone())));
        // As `conn::start` opens serve's: the registry's devices in every
        // checkpoint, and its events in the log.
        let (audit, _) = super::super::audit::Audit::open(&dir.join("cfg"), None, super::super::audit::Fleet::of(&reg));
        audit.hear(&reg);
        Engine {
            rt,
            reg,
            clients,
            ctx,
            dir,
            next: AtomicU64::new(100),
            skipped: Mutex::new(Vec::new()),
            tasks: Mutex::new(Vec::new()),
            audit,
            ids: Mutex::new(Default::default()),
        }
    }

    type Client = WebSocketStream<TcpStream>;

    impl Engine {
        /// A session for `who` over a fresh loopback socket pair.
        fn connect(&self, who: Caller) -> Client {
            self.connect_with(who, None)
        }

        /// The same, as a TLS session would be: with its step-up exporter.
        fn connect_with(&self, who: Caller, exporter: Option<[u8; 32]>) -> Client {
            let (clients, ctx, id) = (self.clients.clone(), self.ctx.clone(), self.next.fetch_add(1, Ordering::SeqCst));
            let au = Arc::new(self.audit_of(id, &who));
            let (client, task) = self.rt.block_on(async move {
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = l.local_addr().unwrap();
                let dial = tokio::spawn(TcpStream::connect(addr));
                let (server, peer) = l.accept().await.unwrap();
                let ws = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
                let task = tokio::spawn(conn::serve_caller(ws, ctx, clients, id, who, exporter, au, None));
                (WebSocketStream::from_raw_socket(dial.await.unwrap().unwrap(), Role::Client, None).await, (task, peer))
            });
            let (task, peer) = task;
            self.tasks.lock().unwrap().push((id, task));
            self.ids.lock().unwrap().insert(peer, id);
            // Returned once the session has joined the client set and made its
            // post-join check. A test that times a close starts its clock after
            // this, so a starved runner's slow session START is not read as a
            // slow close (macOS CI, 2026-10-23). The bound is a hang guard.
            let deadline = Instant::now() + Duration::from_secs(60);
            while !self.clients.is_ready(id) {
                assert!(Instant::now() < deadline, "session {id} never joined");
                std::thread::sleep(Duration::from_millis(1));
            }
            client
        }

        /// The audit's view of connection `id`, for `who`.
        fn audit_of(&self, id: u64, who: &Caller) -> super::super::audit::ConnAudit {
            super::super::audit::ConnAudit::new(self.audit.clone(), id, None, who.clone())
        }

        /// The server's side of the session most recently connected before
        /// `n` others ends within `within` — whatever the client does. A
        /// client that never answers the close is ended all the same.
        fn session_ends(&self, nth_last: usize, within: Duration) {
            let mut tasks = self.tasks.lock().unwrap();
            let i = tasks.len() - 1 - nth_last;
            let (_, task) = tasks.remove(i);
            let r = self.rt.block_on(async move { tokio::time::timeout(within, task).await });
            assert!(r.is_ok(), "the session was still running after {within:?}");
        }

        /// The next frame within `wait`: `Some(Ok(json))` for text,
        /// `Some(Err(reason))` for a close, `None` for silence or the end.
        fn next(&self, c: &mut Client, wait: Duration) -> Option<Result<Value, String>> {
            self.rt.block_on(async {
                loop {
                    match tokio::time::timeout(wait, c.next()).await.ok()?? {
                        Ok(Message::Text(t)) => return Some(Ok(serde_json::from_str(t.as_str()).unwrap())),
                        Ok(Message::Close(f)) => return Some(Err(f.map(|f| f.reason.to_string()).unwrap_or_default())),
                        Ok(_) => continue,
                        Err(_) => return None,
                    }
                }
            })
        }

        /// One request, and its reply (events and channel frames skipped).
        fn rpc(&self, c: &mut Client, cmd: &str, args: Value) -> Value {
            let id = self.next.fetch_add(1, Ordering::SeqCst);
            self.rt.block_on(c.send(Message::text(json!({"id": id, "cmd": cmd, "args": args}).to_string()))).unwrap();
            loop {
                match self.next(c, Duration::from_secs(5)) {
                    Some(Ok(v)) if v["id"] == json!(id) => return v,
                    Some(Ok(v)) => self.skipped.lock().unwrap().push(v),
                    other => panic!("{cmd}: no reply, got {other:?}"),
                }
            }
        }

        /// The connection behind `c` is out of the client set NOW: the act that
        /// just returned closed it — its close frame queued and its session
        /// told, on the act's own thread — with no poll left to wait for (§2.5).
        fn ended(&self, c: &Client) {
            let at = c.get_ref().local_addr().unwrap();
            let id = self.ids.lock().unwrap()[&at];
            assert!(!self.clients.contains(id), "connection {id} is still in the client set: its close waits on something");
        }

        /// The socket is closed by serve, saying `why`, and nothing follows.
        fn closed(&self, c: &mut Client, why: &str) {
            self.closed_within(c, why, HANG)
        }

        /// [`Engine::closed`], within `within` of this call.
        fn closed_within(&self, c: &mut Client, why: &str, within: Duration) {
            let t0 = Instant::now();
            loop {
                match self.next(c, within) {
                    Some(Ok(_)) => continue,
                    Some(Err(reason)) => {
                        assert_eq!(reason, why);
                        break;
                    }
                    None => panic!("no close frame within {within:?}"),
                }
            }
            assert!(t0.elapsed() < within, "closed after {:?}", t0.elapsed());
            assert!(self.next(c, Duration::from_millis(500)).is_none(), "nothing after the close");
        }
    }

    /// §2.5 and exit criterion "a revoked device's live socket dies within
    /// 1 s": the console's revoke closes EVERY connection of the device, with
    /// a close frame saying why; the device's next request on any socket is
    /// refused and its next handshake fails; nothing else is touched, and the
    /// console hears of it while no device does.
    #[test]
    fn a_revoke_closes_every_connection_of_the_device_at_once() {
        let e = engine("revoke", vec![]);
        let phone = enroll(&e.reg, "Phone", 1, &[Read], Projects::All);
        let laptop = enroll(&e.reg, "Laptop", 2, &[Read], Projects::All);
        let (mut p1, p2) = (e.connect(caller(&phone)), e.connect(caller(&phone)));
        let mut lap = e.connect(caller(&laptop));
        let mut con = e.connect(Caller::console());
        assert!(e.rpc(&mut p1, "get_changelog", json!({})).get("ok").is_some(), "the device is served");
        assert_eq!(e.clients.len(), 4);
        let r = e.rpc(&mut con, "device_revoke", json!({"id": phone.id.as_str()}));
        assert_eq!(r["ok"], json!(true), "{r}");
        // The act closed both, on its own thread; and the socket dies within
        // 1 s (§10's exit criterion — this test's bound, and the only one).
        e.ended(&p1);
        e.ended(&p2);
        e.closed_within(&mut p1, "revoked", Duration::from_secs(1));
        // p2 is never read again, so it never answers the close: serve ends
        // its session anyway.
        e.session_ends(2, HANG);
        assert_eq!(e.clients.len(), 2, "out of the map at once");
        // The console heard it (just ahead of its reply); the other device did not.
        let heard = e.skipped.lock().unwrap().iter().any(|v| v["event"] == json!("devices:changed") && v["payload"]["event"] == json!("revoked"));
        assert!(heard, "the console hears devices:changed: {:?}", e.skipped.lock().unwrap());
        let leaked = std::iter::from_fn(|| e.next(&mut lap, Duration::from_millis(300))).flatten().any(|v| v["event"] == json!("devices:changed"));
        assert!(!leaked, "no device hears another's revoke");
        assert!(e.rpc(&mut lap, "get_changelog", json!({})).get("ok").is_some(), "another device is untouched");
        // A socket that outlived the close for any reason: its next request.
        let stale = Call {
            ctx: e.ctx.clone(),
            frames: Frames::new(Outbox::unbounded(tokio::sync::mpsc::unbounded_channel().0)),
            caller: caller(&phone),
            registry: e.reg.clone(),
            world: None,
            devices: e.reg.clone(),
            pairing: e.clients.pairing(),
            flights: Default::default(),
            remote: None,
        };
        assert_eq!(dispatch::call(&stale, "get_changelog", &Args(Default::default())), Err(json!("denied")));
        // And its next handshake.
        assert_eq!(e.reg.admit(&phone.spki_sha256, NOW), Err(super::super::devices::ConnectRefusal::Revoked));
    }

    /// §2.5: a device always revokes ITSELF; another device of its principal
    /// only with `admin`; and a device without it is refused, not served.
    #[test]
    fn a_device_revokes_itself_and_with_admin_a_peer() {
        let e = engine("selfrevoke", vec![]);
        let plain = enroll(&e.reg, "Plain", 1, &[Read], Projects::All);
        let admin = enroll(&e.reg, "Admin", 2, &[Read, Admin], Projects::All);
        let target = enroll(&e.reg, "Target", 3, &[Read], Projects::All);
        let mut p = e.connect(caller(&plain));
        let mut a = e.connect(caller(&admin));
        let mut t = e.connect(caller(&target));
        let r = e.rpc(&mut p, "device_revoke", json!({"id": target.id.as_str()}));
        assert_eq!(r["err"], json!("denied"), "{r}");
        assert_eq!(e.rpc(&mut p, "devices_list", json!({}))["err"], json!("denied"), "a listing is admin's");
        assert_eq!(e.rpc(&mut a, "devices_list", json!({}))["ok"].as_array().unwrap().len(), 3);
        assert_eq!(e.rpc(&mut a, "device_revoke", json!({"id": target.id.as_str()}))["ok"], json!(true));
        e.ended(&t);
        e.closed(&mut t, "revoked");
        // The device that revokes itself is answered by the close.
        e.rt.block_on(p.send(Message::text(json!({"id": 1, "cmd": "device_revoke", "args": {"id": plain.id.as_str()}}).to_string()))).unwrap();
        e.closed(&mut p, "revoked");
        assert!(e.reg.get(&PrincipalId::owner(), &plain.id).unwrap().revoked_at.is_some());
        // Console acts stay the console's, admin or not.
        for (cmd, args) in [
            ("device_enable", json!({"id": target.id.as_str()})),
            ("device_set_scope", json!({"id": admin.id.as_str(), "caps": ["read", "stream"], "projects": "*"})),
            ("device_set_expiry", json!({"id": admin.id.as_str(), "expiry": "never"})),
            ("devices_revoke_all_others", json!({"keep": admin.id.as_str()})),
        ] {
            assert_eq!(e.rpc(&mut a, cmd, args)["err"], json!("console-only"), "{cmd}");
        }
        // An admin device narrows a peer; it can neither widen one nor touch itself.
        let peer = enroll(&e.reg, "Peer", 4, &[Read, Write], Projects::All);
        let r = e.rpc(&mut a, "device_narrow", json!({"id": peer.id.as_str(), "caps": ["read"], "projects": "*"}));
        assert_eq!(r["ok"], json!(true), "{r}");
        let r = e.rpc(&mut a, "device_narrow", json!({"id": peer.id.as_str(), "caps": ["read", "stream"], "projects": "*"}));
        assert!(r["err"].as_str().unwrap().contains("widening is the host's"), "{r}");
        let r = e.rpc(&mut a, "device_narrow", json!({"id": admin.id.as_str(), "caps": ["read"], "projects": "*"}));
        assert!(r["err"].as_str().unwrap().contains("own scope"), "{r}");
    }

    /// §2.5: a disable closes the device's connections exactly as a revoke
    /// does, and refuses it — its handshake, a connection that got past one,
    /// a request on a socket that outlived the close — until the HOST
    /// re-enables it, which not even an admin device may; after that it is
    /// served again.
    #[test]
    fn a_disable_closes_like_a_revoke_and_only_the_host_reenables() {
        let e = engine("disable", vec![]);
        let phone = enroll(&e.reg, "Phone", 1, &[Read], Projects::All);
        let admin = enroll(&e.reg, "Admin", 2, &[Read, Admin], Projects::All);
        let mut p = e.connect(caller(&phone));
        let mut adm = e.connect(caller(&admin));
        let mut con = e.connect(Caller::console());
        assert_eq!(e.rpc(&mut con, "device_disable", json!({"id": phone.id.as_str()}))["ok"], json!(true));
        e.ended(&p);
        e.closed(&mut p, "disabled");
        assert_eq!(e.reg.admit(&phone.spki_sha256, NOW), Err(super::super::devices::ConnectRefusal::Disabled));
        let mut again = e.connect(caller(&phone));
        e.ended(&again);
        e.closed(&mut again, "disabled");
        let stale = Call {
            ctx: e.ctx.clone(),
            frames: Frames::new(Outbox::unbounded(tokio::sync::mpsc::unbounded_channel().0)),
            caller: caller(&phone),
            registry: e.reg.clone(),
            world: None,
            devices: e.reg.clone(),
            pairing: e.clients.pairing(),
            flights: Default::default(),
            remote: None,
        };
        let enable = || Args(json!({"id": phone.id.as_str()}).as_object().cloned().unwrap());
        assert_eq!(dispatch::call(&stale, "get_changelog", &Args(Default::default())), Err(json!("denied")));
        assert_eq!(dispatch::call(&stale, "device_enable", &enable()), Err(json!("console-only")));
        assert_eq!(e.rpc(&mut adm, "device_enable", json!({"id": phone.id.as_str()}))["err"], json!("console-only"));
        assert_eq!(e.rpc(&mut con, "device_enable", json!({"id": phone.id.as_str()}))["ok"], json!(true));
        let mut back = e.connect(caller(&phone));
        assert!(e.rpc(&mut back, "get_changelog", json!({})).get("ok").is_some(), "served again");
        // An expiry that runs out closes it the same way (the timer's act).
        e.rpc(&mut con, "device_set_expiry", json!({"id": phone.id.as_str(), "expiry": {"idle_days": 1}}));
        let mut late = e.connect(caller(&phone));
        // Enrolled at NOW, which is later than the re-enable's wall clock.
        assert_eq!(e.reg.check_expiry(NOW + 2 * 86_400).unwrap(), vec![phone.id.clone()]);
        e.ended(&late);
        e.ended(&back);
        e.closed(&mut late, "disabled");
        e.closed(&mut back, "disabled");
    }

    /// The gap between a device's handshake and its session joining the
    /// client set: an event that fires there finds no connection to close
    /// (macOS CI, 2026-10-23: `late` above, connected and disabled before its
    /// session had joined, was never closed). The session asks the registry
    /// once it has joined, so a device disabled, revoked or removed in that
    /// gap is closed as it joins, saying why — not left on an open socket
    /// whose every request is refused.
    #[test]
    fn a_device_ended_before_its_session_joins_is_closed_as_it_joins() {
        let e = engine("prejoin", vec![]);
        let phone = enroll(&e.reg, "Phone", 1, &[Read], Projects::All);
        let tablet = enroll(&e.reg, "Tablet", 2, &[Read], Projects::All);
        let laptop = enroll(&e.reg, "Laptop", 3, &[Read], Projects::All);
        // Each act lands as if after the handshake admitted the device and
        // before its session joined: no connection of it exists yet.
        e.reg.disable(&phone.id, NOW).unwrap();
        e.reg.revoke(&Caller::console(), &Grant::everything(), &tablet.id, NOW).unwrap();
        let mut p = e.connect(caller(&phone));
        e.ended(&p);
        e.closed(&mut p, "disabled");
        let mut t = e.connect(caller(&tablet));
        e.ended(&t);
        e.closed(&mut t, "revoked");
        let mut ghost = e.connect(Caller::paired(PrincipalId::owner(), DeviceId::new("d_ghost")));
        e.ended(&ghost);
        e.closed(&mut ghost, "removed");
        let mut l = e.connect(caller(&laptop));
        assert!(e.rpc(&mut l, "get_changelog", json!({})).get("ok").is_some(), "a live device is served");
        assert_eq!(e.clients.len(), 1, "the ended ones are out of the map");
    }

    /// The console's device rows answer it, and a device of ANOTHER principal
    /// reads as missing to everyone — the words a device gets are the fixed
    /// ones, so they say nothing about what exists (§3.3).
    #[test]
    fn a_device_row_naming_a_missing_device_says_the_fixed_words() {
        let e = engine("missing", vec![]);
        let admin = enroll(&e.reg, "Admin", 2, &[Read, Admin], Projects::All);
        let mut a = e.connect(caller(&admin));
        let mut con = e.connect(Caller::console());
        let r = e.rpc(&mut a, "device_revoke", json!({"id": "d_nosuch"}));
        assert_eq!(r["err"], json!("not found or not in this device's scope"), "{r}");
        let r = e.rpc(&mut a, "device_narrow", json!({"id": "d_nosuch", "caps": [], "projects": []}));
        assert_eq!(r["err"], json!("not found or not in this device's scope"), "{r}");
        let r = e.rpc(&mut con, "device_disable", json!({"id": "d_nosuch"}));
        assert!(r["err"].as_str().unwrap().contains("no device d_nosuch"), "the console is told the detail: {r}");
        // The console widens with registered roots only, and the caps by name.
        let root_a = s(&e.dir.join("a"));
        let r = e.rpc(&mut con, "device_set_scope", json!({"id": admin.id.as_str(), "caps": ["read", "admin"], "projects": ["/not/registered"]}));
        assert!(r["err"].as_str().unwrap().contains("not a registered project"), "{r}");
        let r = e.rpc(&mut con, "device_set_scope", json!({"id": admin.id.as_str(), "caps": ["read", "root"], "projects": [root_a]}));
        assert!(r["err"].as_str().unwrap().contains("unknown capability"), "{r}");
        let r = e.rpc(&mut con, "device_set_scope", json!({"id": admin.id.as_str(), "caps": ["read", "admin", "stream"], "projects": [root_a]}));
        assert_eq!(r["ok"], json!(true), "{r}");
        let r = e.rpc(&mut con, "device_set_expiry", json!({"id": admin.id.as_str(), "expiry": {"idle_days": 0}}));
        assert!(r["err"].is_string(), "{r}");
    }

    /// §3.3: "a narrowed scope or a removed capability ends each open channel
    /// whose object it no longer covers". Two attaches (sessions in a and b)
    /// and two dock shells (a and b) on one device's connection; the console
    /// narrows the device to a, and only b's channels end; it removes
    /// `stream`, and the rest do. The check is the resolver's own.
    #[test]
    fn a_narrowed_grant_ends_the_channels_it_no_longer_covers() {
        let e = engine("narrow", vec![]);
        let (a, b) = (s(&e.dir.join("a")), s(&e.dir.join("b")));
        // The fixture world's panes: session sa is cwd'd in a, sb in b.
        let world = Roots { roots: vec![a.clone(), b.clone()], panes: vec![("sa".into(), a.clone(), "zsh".into()), ("sb".into(), b.clone(), "zsh".into())] };
        let dev = enroll(&e.reg, "Dev", 1, &[Read, Stream], Projects::only([a.clone(), b.clone()]));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let out = Outbox::unbounded(tx);
        let call = Arc::new(Call {
            ctx: e.ctx.clone(),
            frames: Frames::new(out.clone()),
            caller: caller(&dev),
            registry: e.reg.clone(),
            world: Some(Arc::new(world)),
            devices: e.reg.clone(),
            pairing: e.clients.pairing(),
            flights: Default::default(),
            remote: None,
        });
        call.frames.note_term(901, "sa".into());
        call.frames.note_term(902, "sb".into());
        call.frames.note_shell(ShellHold { repo: a.clone(), slug: "(main)".into(), index: 0, gen: 7 });
        call.frames.note_shell(ShellHold { repo: b.clone(), slug: "(main)".into(), index: 0, gen: 9 });
        let _kill = e.clients.join(1, out, call.clone());
        // A widening ends nothing.
        e.reg.set_scope(&dev.id, Grant { capabilities: Caps::of(&[Read, Write, Stream]), projects: Projects::only([a.clone(), b.clone()]) }).unwrap();
        assert_eq!(call.frames.terms().len(), 2);
        assert_eq!(call.frames.shells().len(), 2);
        e.reg.set_scope(&dev.id, Grant { capabilities: Caps::of(&[Read, Stream]), projects: Projects::only([a.clone()]) }).unwrap();
        assert_eq!(call.frames.terms(), vec![(901, "sa".to_string())], "b's attach ended");
        assert_eq!(call.frames.shells().iter().map(|h| h.repo.clone()).collect::<Vec<_>>(), vec![a.clone()], "b's shell let go");
        e.reg.set_scope(&dev.id, Grant { capabilities: Caps::of(&[Read]), projects: Projects::only([a.clone()]) }).unwrap();
        assert!(call.frames.terms().is_empty() && call.frames.shells().is_empty(), "no stream, no channel");
        assert_eq!(e.clients.len(), 1, "a narrowing ends channels, not the connection");
    }

    /// A registry whose changes reach NO hook: the per-request generation
    /// check is what catches them (§3.3), one atomic load per request when
    /// nothing moved.
    struct Quiet {
        grant: Mutex<Grant>,
        gen: AtomicU64,
    }

    impl Registry for Quiet {
        fn principal_grant(&self, _: &PrincipalId) -> Option<Grant> {
            None
        }
        fn device_grant(&self, _: &PrincipalId, _: &DeviceId) -> Option<Grant> {
            Some(self.grant.lock().unwrap().clone())
        }
        fn device_principal(&self, _: &DeviceId) -> Option<PrincipalId> {
            Some(PrincipalId::owner())
        }
        fn stepup_required(&self, _: &PrincipalId, _: &DeviceId) -> bool {
            false
        }
        fn generation(&self) -> u64 {
            self.gen.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn the_next_request_checks_open_channels_when_the_registry_moved() {
        let e = engine("backstop", vec![]);
        let a = s(&e.dir.join("a"));
        let reg = Arc::new(Quiet { grant: Mutex::new(Grant { capabilities: Caps::of(&[Read, Stream]), projects: Projects::All }), gen: AtomicU64::new(1) });
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let call = Call {
            ctx: e.ctx.clone(),
            frames: Frames::new(Outbox::unbounded(tx)),
            caller: Caller::paired(PrincipalId::owner(), DeviceId::new("d_q")),
            registry: reg.clone(),
            world: Some(Arc::new(Roots { roots: vec![a.clone()], panes: vec![("sa".into(), a.clone(), "zsh".into())] })),
            devices: e.reg.clone(),
            pairing: e.clients.pairing(),
            flights: Default::default(),
            remote: None,
        };
        call.frames.reconciled().store(1, Ordering::SeqCst);
        call.frames.note_term(903, "sa".into());
        let au = e.audit_of(1, &call.caller);
        let _ = conn::answer(&call, &au, &json!(1), "get_changelog", &Args(Default::default()));
        assert_eq!(call.frames.terms().len(), 1, "nothing moved: nothing checked, nothing ended");
        *reg.grant.lock().unwrap() = Grant { capabilities: Caps::of(&[Read]), projects: Projects::All };
        let _ = conn::answer(&call, &au, &json!(2), "get_changelog", &Args(Default::default()));
        assert_eq!(call.frames.terms().len(), 1, "the grant changed but the generation did not: unseen");
        reg.gen.store(2, Ordering::SeqCst);
        let _ = conn::answer(&call, &au, &json!(3), "get_changelog", &Args(Default::default()));
        assert!(call.frames.terms().is_empty(), "the generation moved: the attach it no longer covers ended");
        assert_eq!(call.frames.reconciled().load(Ordering::SeqCst), 2);
    }

    /// A `shell_detach` naming a generation this connection does not hold
    /// releases nothing — `term::shell_detach` ignores it — so the hold stays
    /// recorded and a later narrowing still lets go of the sink. Forgetting it
    /// on the client's word would leave the sink attached and unseen.
    #[test]
    fn a_detach_naming_another_generation_keeps_the_hold() {
        let e = engine("detachgen", vec![]);
        let a = s(&e.dir.join("a"));
        let dev = enroll(&e.reg, "Dev", 1, &[Read, Stream], Projects::only([a.clone()]));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let out = Outbox::unbounded(tx);
        let call = Arc::new(Call {
            ctx: e.ctx.clone(),
            frames: Frames::new(out.clone()),
            caller: caller(&dev),
            registry: e.reg.clone(),
            world: Some(Arc::new(Roots { roots: vec![a.clone()], panes: vec![] })),
            devices: e.reg.clone(),
            pairing: e.clients.pairing(),
            flights: Default::default(),
            remote: None,
        });
        call.frames.note_shell(ShellHold { repo: a.clone(), slug: "(main)".into(), index: 0, gen: 7 });
        let _kill = e.clients.join(1, out, call.clone());
        let detach = |gen: u64| dispatch::call(&call, "shell_detach", &Args(json!({"repo": a, "slug": "(main)", "index": 0, "gen": gen}).as_object().cloned().unwrap()));
        assert_eq!(detach(8), Ok(Value::Null));
        assert_eq!(call.frames.shells().len(), 1, "another generation's detach keeps this one's hold");
        e.reg.set_scope(&dev.id, Grant { capabilities: Caps::of(&[Read]), projects: Projects::only([a.clone()]) }).unwrap();
        assert!(call.frames.shells().is_empty(), "and the narrowing found it");
        call.frames.note_shell(ShellHold { repo: a.clone(), slug: "(main)".into(), index: 0, gen: 9 });
        e.reg.set_scope(&dev.id, Grant { capabilities: Caps::of(&[Read, Stream]), projects: Projects::only([a.clone()]) }).unwrap();
        assert_eq!(detach(9), Ok(Value::Null));
        assert!(call.frames.shells().is_empty(), "its own generation's detach releases it");
    }

    /// The narrowing that lands WHILE a stream is being opened: the request
    /// was authorized under the old grant, and the narrowing's own reconcile
    /// ran before the new channel was noted, so it could not see it. The
    /// world fixture fires that interleaving from inside the request (its
    /// pane lookup is the resolver's, mid-authorization); the request's
    /// end-of-answer check must end the channel all the same.
    struct Racing {
        root: String,
        reg: Arc<Quiet>,
        frames: Frames,
        fired: std::sync::atomic::AtomicBool,
    }

    impl World for Racing {
        fn roots(&self) -> Vec<String> {
            vec![self.root.clone()]
        }
        fn panes(&self) -> Option<worktrees_core::tmux::PaneList> {
            if !self.fired.swap(true, Ordering::SeqCst) {
                *self.reg.grant.lock().unwrap() = Grant { capabilities: Caps::of(&[Read]), projects: Projects::All };
                self.reg.gen.store(2, Ordering::SeqCst);
                self.frames.reconciled().store(2, Ordering::SeqCst);
                self.frames.note_term(904, "sa".into());
            }
            Some(worktrees_core::tmux::PaneList::from_rows(vec![("sa".into(), self.root.clone(), "zsh".into())]))
        }
    }

    #[test]
    fn a_narrowing_that_lands_mid_open_still_ends_the_new_channel() {
        let e = engine("midopen", vec![]);
        let a = s(&e.dir.join("a"));
        let reg = Arc::new(Quiet { grant: Mutex::new(Grant { capabilities: Caps::of(&[Read, Stream]), projects: Projects::All }), gen: AtomicU64::new(1) });
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let frames = Frames::new(Outbox::unbounded(tx));
        let world = Racing { root: a.clone(), reg: reg.clone(), frames: frames.clone(), fired: Default::default() };
        let call = Call {
            ctx: e.ctx.clone(),
            frames: frames.clone(),
            caller: Caller::paired(PrincipalId::owner(), DeviceId::new("d_q")),
            registry: reg.clone(),
            world: Some(Arc::new(world)),
            devices: e.reg.clone(),
            pairing: e.clients.pairing(),
            flights: Default::default(),
            remote: None,
        };
        frames.reconciled().store(1, Ordering::SeqCst);
        // No `cols`: the body fails on its arguments before it reaches tmux.
        let au = e.audit_of(1, &call.caller);
        let _ = conn::answer(&call, &au, &json!(1), "term_open", &Args(json!({"session": "sa"}).as_object().cloned().unwrap()));
        assert!(frames.terms().is_empty(), "the channel opened under the old grant was ended: {:?}", frames.terms());
    }

    /// §2.5: rotating the server identity is the console's, makes a new pin,
    /// and revokes every device — each holds the old pin and must pair again
    /// — closing their connections like any revoke.
    #[test]
    fn rotating_the_identity_revokes_every_device() {
        let e = engine("rotate", vec![]);
        let before = super::super::identity::Identity::ensure(&e.dir.join("cfg")).unwrap();
        let phone = enroll(&e.reg, "Phone", 1, &[Read, Admin], Projects::All);
        let laptop = enroll(&e.reg, "Laptop", 2, &[Read], Projects::All);
        let mut p = e.connect(caller(&phone));
        let mut con = e.connect(Caller::console());
        assert_eq!(e.rpc(&mut p, "identity_rotate", json!({}))["err"], json!("console-only"));
        let r = e.rpc(&mut con, "identity_rotate", json!({}));
        let pin = r["ok"]["pin"].as_str().unwrap().to_string();
        assert_ne!(pin, before.pin());
        assert_eq!(super::super::identity::Identity::load(&e.dir.join("cfg")).unwrap().unwrap().pin(), pin);
        assert_eq!(r["ok"]["revoked"].as_array().unwrap().len(), 2, "{r}");
        e.ended(&p);
        e.closed(&mut p, "revoked");
        assert!(e.reg.get(&PrincipalId::owner(), &laptop.id).unwrap().revoked_at.is_some());
        let rotated = e.skipped.lock().unwrap().iter().any(|v| v["payload"]["event"] == json!("identity_rotated") && v["payload"]["pin"] == json!(pin));
        assert!(rotated, "the console hears the rotation, with the new pin");
    }

    // ── pairing, end to end, restated from §2.2's text ──────────────────────
    //
    // The device side is written from the proposal, not from pairing.rs: the
    // labels are spelled again, the HMAC is the parent module's RFC 2104 one
    // over bare SHA-256, and the code is computed here. A server that agreed
    // with itself would not pass.

    const SPEC_PAIR_LABEL: &[u8] = b"worktrees-pair/v1\n";
    const SPEC_SAS_LABEL: &[u8] = b"worktrees-pair/v1/sas\n";
    const SPEC_EXPORTER: &[u8] = b"EXPORTER-worktrees-pair-v1";

    /// A literal `ip:port` and nothing more: a device's rule
    /// (`pairuri::dial_rule`, §4.1) refuses 127.0.0.1 in a real link, so the
    /// tests read theirs, over the loopback listener, with this one.
    fn literal(v: &str) -> Result<std::net::SocketAddr, String> {
        v.parse().map_err(|e| format!("{e}"))
    }

    fn sha(b: &[u8]) -> Vec<u8> {
        use sha2::Digest;
        sha2::Sha256::digest(b).to_vec()
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// What a device chooses to send; each field can be bent to test a refusal.
    struct Attempt {
        name: String,
        platform: &'static str,
        stepup: Option<super::super::stepup::testkit::StepUpKey>,
        /// The secret the proof is keyed with (default: the link's).
        secret: Option<Vec<u8>>,
        /// An exporter to prove over instead of the session's.
        exporter: Option<[u8; 32]>,
        /// A key to prove for instead of the one presented.
        proof_spki: Option<Vec<u8>>,
        /// Replace the whole line.
        line: Option<String>,
        /// Text put just inside the (otherwise correct) frame's opening
        /// brace: padding, or a key the frame should not have.
        insert: String,
    }

    impl Attempt {
        fn desktop(name: &str) -> Attempt {
            Attempt { name: name.into(), platform: "macos", stepup: None, secret: None, exporter: None, proof_spki: None, line: None, insert: String::new() }
        }
        fn iphone(name: &str) -> Attempt {
            Attempt { platform: "ios", stepup: Some(super::super::stepup::testkit::StepUpKey::new()), ..Attempt::desktop(name) }
        }
    }

    type DeviceTls = tokio_rustls::client::TlsStream<TcpStream>;

    struct Paired {
        tls: DeviceTls,
        code: String,
        key: super::super::tls::testkit::DeviceKey,
    }

    impl Engine {
        /// Open a window from the console as the host would.
        fn begin(&self, con: &mut Client, caps: &[&str], projects: Value, expiry: Value) -> Value {
            self.rpc(con, "pair_begin", json!({"caps": caps, "projects": projects, "expiry": expiry}))
        }

        /// Scan `uri`, dial the pairing listener pinned, send the one frame.
        fn attempt(&self, uri: &str, a: &Attempt) -> Result<Paired, String> {
            let link = worktrees_core::pairuri::parse(uri, worktrees_core::sysclock::now_epoch(), &literal)?;
            let key = super::super::tls::testkit::device_key();
            self.rt.block_on(async {
                let mut tls = super::super::tls::testkit::dial(link.addr, &link.pin, &key).await.map_err(|e| e.to_string())?;
                let ex: [u8; 32] = tls.get_ref().1.export_keying_material([0u8; 32], SPEC_EXPORTER, None).map_err(|e| e.to_string())?;
                let secret = a.secret.clone().unwrap_or(link.secret.to_vec());
                let mut msg = SPEC_PAIR_LABEL.to_vec();
                msg.extend_from_slice(&a.exporter.unwrap_or(ex));
                msg.extend_from_slice(&sha(a.proof_spki.as_deref().unwrap_or(&key.spki)));
                msg.extend_from_slice(&a.stepup.as_ref().map(|k| sha(&k.spki)).unwrap_or(vec![0; 32]));
                msg.extend_from_slice(a.name.as_bytes());
                let proof = super::spec_hmac(&secret, &msg);
                let stepup = a.stepup.as_ref().map(|k| worktrees_api::files::b64_encode(&k.spki));
                let line = a.line.clone().unwrap_or_else(|| {
                    let l = json!({"pair": {"name": a.name, "platform": a.platform, "stepup_spki": stepup}, "proof": proof}).to_string();
                    format!("{{{}{}", a.insert, &l[1..])
                });
                use tokio::io::AsyncWriteExt;
                tls.write_all(format!("{line}\n").as_bytes()).await.map_err(|e| e.to_string())?;
                tls.flush().await.map_err(|e| e.to_string())?;
                let mut sas = SPEC_SAS_LABEL.to_vec();
                sas.extend_from_slice(&ex);
                let t = unhex(&super::spec_hmac(&link.secret, &sas));
                let code = format!("{:06}", (u32::from_be_bytes([t[0], t[1], t[2], t[3]]) & 0x7fff_ffff) % 1_000_000);
                Ok(Paired { tls, code, key })
            })
        }

        /// The device's next line from the server, or `None` for silence, the
        /// end or a reset, within `wait`.
        fn line(&self, tls: &mut DeviceTls, wait: Duration) -> Option<Value> {
            use tokio::io::AsyncReadExt;
            self.rt.block_on(async {
                let mut buf = Vec::new();
                let mut b = [0u8; 1];
                loop {
                    match tokio::time::timeout(wait, tls.read(&mut b)).await {
                        Ok(Ok(1)) if b[0] == b'\n' => return serde_json::from_slice(&buf).ok(),
                        Ok(Ok(1)) => buf.push(b[0]),
                        _ => return None,
                    }
                }
            })
        }

        fn status(&self, con: &mut Client) -> Value {
            self.rpc(con, "pair_status", json!({}))["ok"].clone()
        }

        /// The pairing window's port takes no connection any more.
        fn port_closed(&self, uri: &str) -> bool {
            let link = worktrees_core::pairuri::parse(uri, 0, &literal).unwrap();
            std::net::TcpStream::connect_timeout(&link.addr, Duration::from_millis(500)).is_err()
        }

        /// Wait (bounded) until the window is closed, and say how.
        fn closed_as(&self, con: &mut Client) -> Value {
            let t0 = Instant::now();
            loop {
                let st = self.status(con);
                if st["state"] == json!("closed") || t0.elapsed() > Duration::from_secs(5) {
                    return st;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        fn bind_loopback(&self) {
            self.clients.pairing().set_bind(Some(super::super::pairing::Bind {
                ip: std::net::Ipv4Addr::LOCALHOST.into(),
                remote: "127.0.0.1:7461".parse().unwrap(),
                index: 0,
                rule: None,
            }));
        }
    }

    /// §2.2 from end to end: the host chooses the scope, the window opens, the
    /// device proves the secret over pinned TLS bound to the session, both
    /// screens show one code, and the host's Allow writes the record — with
    /// the offer's scope, the device's keys, step-up on — and answers the
    /// device. Then the listener is gone.
    #[test]
    fn a_device_pairs_by_link_proof_code_and_the_hosts_allow() {
        let e = engine("pair", vec![]);
        e.bind_loopback();
        let mut con = e.connect(Caller::console());
        let a = s(&e.dir.join("a"));
        let r = e.begin(&mut con, &["read", "write"], json!([a]), json!({"idle_days": 7}));
        let uri = r["ok"]["uri"].as_str().unwrap_or_else(|| panic!("{r}")).to_string();
        let identity = super::super::identity::Identity::load(&e.dir.join("cfg")).unwrap().unwrap();
        assert_eq!(r["ok"]["pin"], json!(identity.pin()));
        assert_eq!(e.status(&mut con)["state"], json!("open"));
        let mut p = e.attempt(&uri, &Attempt::iphone("Bret's iPhone")).unwrap();
        assert_eq!(e.line(&mut p.tls, Duration::from_secs(5)), Some(json!({"pending": true})));
        let st = e.status(&mut con);
        assert_eq!(st["state"], json!("pending"), "{st}");
        assert_eq!(st["request"]["code"], json!(p.code), "both screens show the same code");
        assert_eq!(st["request"]["name"], json!("Bret's iPhone"));
        assert_eq!(st["request"]["platform"], json!("ios"));
        assert!(st["request"].get("spki_sha256").is_none(), "keys are not the dialog's");
        let rec = e.rpc(&mut con, "pair_decide", json!({"allow": true}));
        let rec = rec["ok"].clone();
        assert_eq!(rec["spki_sha256"], json!(p.key.pin), "{rec}");
        assert_eq!(rec["caps"], json!(["read", "write"]));
        assert_eq!(rec["projects"], json!([a]));
        assert_eq!(rec["expiry"], json!({"idle_days": 7}));
        assert_eq!(rec["stepup_required"], json!(true), "on by default (§11 Q8)");
        assert_eq!(rec["last_addr"], json!("127.0.0.1"));
        let answer = e.line(&mut p.tls, Duration::from_secs(5)).expect("the device's answer");
        let paired = &answer["paired"];
        assert_eq!(paired["device_id"], rec["id"]);
        assert_eq!(paired["principal_id"], json!("owner"));
        assert_eq!(paired["remote_addr"], json!("127.0.0.1:7461"));
        assert_eq!(paired["expiry"], json!({"idle_days": 7}));
        assert_eq!(paired["discovery_id"], json!(identity.discovery_id()));
        assert!(paired["host_name"].is_string());
        let st = e.closed_as(&mut con);
        assert_eq!((st["ended"].clone(), st["device"].clone()), (json!("paired"), rec["id"].clone()));
        assert!(e.port_closed(&uri), "the pairing listener is gone");
        assert_eq!(e.reg.admit(&p.key.pin, worktrees_core::sysclock::now_epoch()).map(|c| c.device().clone()), Ok(worktrees_core::authz::Device::Paired(DeviceId::new(rec["id"].as_str().unwrap()))));
        let events: Vec<String> = e.skipped.lock().unwrap().iter().filter(|v| v["event"] == json!("devices:changed")).map(|v| v["payload"]["event"].as_str().unwrap_or("").to_string()).collect();
        let want = ["pairing_opened", "pairing_attempt", "pairing_pending", "pairing_decided", "enrolled", "pairing_closed"];
        let mut it = events.iter();
        assert!(want.iter().all(|w| it.any(|e| e == w)), "in order: {events:?}");
    }

    /// §2.2 step 5, §2.4: ANY failure closes the window, with no retry and
    /// nothing said — a proof keyed with another secret, bound to another
    /// session, or made for another key; a frame that is malformed, too long,
    /// names its device with a direction-changing character, or brings no
    /// step-up key from an iPhone.
    #[test]
    fn any_failure_closes_the_window_with_no_retry() {
        let e = engine("pairfail", vec![]);
        e.bind_loopback();
        let mut con = e.connect(Caller::console());
        let other = super::super::tls::testkit::device_key();
        let cases: Vec<(&str, Attempt, &str)> = vec![
            ("another secret", Attempt { secret: Some(vec![1; 16]), ..Attempt::desktop("Mac") }, "bad_proof"),
            ("another session", Attempt { exporter: Some([0; 32]), ..Attempt::desktop("Mac") }, "bad_proof"),
            ("another key", Attempt { proof_spki: Some(other.spki.clone()), ..Attempt::desktop("Mac") }, "bad_proof"),
            ("a name with an override", Attempt::desktop("Bret\u{202E}enohP"), "malformed"),
            ("an iPhone with no step-up key", Attempt { platform: "ios", ..Attempt::desktop("iPhone") }, "malformed"),
            ("a desktop with one", Attempt { platform: "macos", ..Attempt::iphone("Mac") }, "malformed"),
            ("an unknown key", Attempt { insert: r#""extra": 1, "#.into(), ..Attempt::desktop("Mac") }, "malformed"),
            ("an unknown platform", Attempt { platform: "windows", ..Attempt::desktop("PC") }, "malformed"),
            ("a good frame past 4 KiB", Attempt { insert: " ".repeat(5000), ..Attempt::desktop("Mac") }, "malformed"),
            ("a line that never ends", Attempt { line: Some("x".repeat(5000)), ..Attempt::desktop("x") }, "malformed"),
        ];
        for (what, a, ended) in cases {
            let r = e.begin(&mut con, &["read"], json!("*"), Value::Null);
            let uri = r["ok"]["uri"].as_str().unwrap().to_string();
            let mut p = e.attempt(&uri, &a).unwrap();
            assert_eq!(e.line(&mut p.tls, Duration::from_millis(500)), None, "{what}: nothing said");
            let st = e.closed_as(&mut con);
            assert_eq!(st["ended"], json!(ended), "{what}: {st}");
            assert!(e.port_closed(&uri), "{what}: no retry");
            assert_eq!(e.rpc(&mut con, "pair_decide", json!({"allow": true}))["err"], json!("no pairing request is waiting"), "{what}");
        }
        assert!(e.reg.list(&PrincipalId::owner()).unwrap().is_empty(), "nothing enrolled");
    }

    /// §2.4: a second connection ends the window — the race the comparison
    /// code exists for — and the device already waiting is told `closed`.
    #[test]
    fn a_second_connection_ends_the_window_and_the_waiting_device_is_told() {
        let e = engine("pairsecond", vec![]);
        e.bind_loopback();
        let mut con = e.connect(Caller::console());
        let uri = e.begin(&mut con, &["read"], json!("*"), Value::Null)["ok"]["uri"].as_str().unwrap().to_string();
        let mut p = e.attempt(&uri, &Attempt::desktop("Mac")).unwrap();
        assert_eq!(e.line(&mut p.tls, Duration::from_secs(5)), Some(json!({"pending": true})));
        let link = worktrees_core::pairuri::parse(&uri, 0, &literal).unwrap();
        let _racer = std::net::TcpStream::connect(link.addr).unwrap();
        assert_eq!(e.line(&mut p.tls, Duration::from_secs(5)), Some(json!({"denied": "closed"})));
        assert_eq!(e.closed_as(&mut con)["ended"], json!("second_connection"));
        assert_eq!(e.rpc(&mut con, "pair_decide", json!({"allow": true}))["err"], json!("no pairing request is waiting"));
        assert!(e.reg.list(&PrincipalId::owner()).unwrap().is_empty());
    }

    /// §2.4's two deadlines, each running out as a Deny: no attempt in the
    /// window, and no decision after the frame.
    #[test]
    fn the_deadlines_run_out_as_a_deny() {
        let e = engine_times("pairdeadline", vec![], PAIR_OPEN, PAIR_DECIDE, false);
        e.bind_loopback();
        let mut con = e.connect(Caller::console());
        let before = worktrees_core::sysclock::now_epoch();
        let r = e.begin(&mut con, &["read"], json!("*"), Value::Null);
        // 1.5 s rounds UP: the link must not read as expired while it is open.
        assert!(r["ok"]["expires_at"].as_i64().unwrap() >= before + 2, "{r}");
        let uri = r["ok"]["uri"].as_str().unwrap().to_string();
        std::thread::sleep(PAIR_OPEN + Duration::from_millis(300));
        assert_eq!(e.closed_as(&mut con)["ended"], json!("expired"));
        assert!(e.port_closed(&uri));
        let uri = e.begin(&mut con, &["read"], json!("*"), Value::Null)["ok"]["uri"].as_str().unwrap().to_string();
        let mut p = e.attempt(&uri, &Attempt::desktop("Mac")).unwrap();
        assert_eq!(e.line(&mut p.tls, Duration::from_secs(5)), Some(json!({"pending": true})));
        assert_eq!(e.line(&mut p.tls, PAIR_DECIDE + Duration::from_secs(2)), Some(json!({"denied": "timeout"})));
        assert_eq!(e.closed_as(&mut con)["ended"], json!("undecided"));
        assert!(e.reg.list(&PrincipalId::owner()).unwrap().is_empty());
    }

    /// Deny, cancel, and one window at a time (§2.4).
    #[test]
    fn deny_cancel_and_one_window_at_a_time() {
        let e = engine("pairdeny", vec![]);
        e.bind_loopback();
        let mut con = e.connect(Caller::console());
        let uri = e.begin(&mut con, &["read"], json!("*"), Value::Null)["ok"]["uri"].as_str().unwrap().to_string();
        let mut p = e.attempt(&uri, &Attempt::desktop("Mac")).unwrap();
        assert_eq!(e.line(&mut p.tls, Duration::from_secs(5)), Some(json!({"pending": true})));
        assert_eq!(e.rpc(&mut con, "pair_decide", json!({"allow": false}))["ok"], Value::Null);
        assert_eq!(e.line(&mut p.tls, Duration::from_secs(5)), Some(json!({"denied": "denied"})));
        assert_eq!(e.closed_as(&mut con)["ended"], json!("denied"));
        let first = e.begin(&mut con, &["read"], json!("*"), Value::Null)["ok"]["uri"].as_str().unwrap().to_string();
        let second = e.begin(&mut con, &["read"], json!("*"), Value::Null)["ok"]["uri"].as_str().unwrap().to_string();
        assert_ne!(first, second, "a new window has a new secret and port");
        // Well inside the first window's own deadline, so only the second
        // window's opening can have closed it. Watched by its event, not by
        // dialling it: a dial IS an attempt, and a second one would close it.
        let t0 = Instant::now();
        let superseded = || e.skipped.lock().unwrap().iter().any(|v| v["payload"]["event"] == json!("pairing_closed") && v["payload"]["ended"] == json!("superseded"));
        while !superseded() && t0.elapsed() < Duration::from_millis(500) {
            e.status(&mut con);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(superseded(), "opening a second window closed the first");
        assert!(e.port_closed(&first));
        assert_eq!(e.status(&mut con)["state"], json!("open"));
        assert_eq!(e.rpc(&mut con, "pair_cancel", json!({}))["ok"], json!(true));
        assert_eq!(e.closed_as(&mut con)["ended"], json!("cancelled"));
        assert!(e.port_closed(&second));
        let used = e.attempt(&second, &Attempt::desktop("Late"));
        assert!(used.is_err(), "a closed window's link reaches nothing");
    }

    /// Only the console opens a window (§11 Q5), only while remote access is
    /// on, and never past 16 enrolled devices (§2.4).
    #[test]
    fn a_window_is_the_hosts_and_opens_only_where_and_while_it_may() {
        let e = engine("pairgate", vec![]);
        let mut con = e.connect(Caller::console());
        let r = e.begin(&mut con, &["read"], json!("*"), Value::Null);
        assert!(r["err"].as_str().unwrap().contains("remote access is off"), "{r}");
        e.bind_loopback();
        let admin = enroll(&e.reg, "Admin", 1, &[Read, Admin, Write, Stream], Projects::All);
        let mut dev = e.connect(caller(&admin));
        for cmd in ["pair_begin", "pair_decide", "pair_cancel", "pair_status"] {
            assert_eq!(e.rpc(&mut dev, cmd, json!({"caps": ["read"], "projects": "*", "allow": true}))["err"], json!("console-only"), "{cmd}");
        }
        for i in 2..=16u8 {
            enroll(&e.reg, &format!("D{i}"), i, &[Read], Projects::All);
        }
        let r = e.begin(&mut con, &["read"], json!("*"), Value::Null);
        assert!(r["err"].as_str().unwrap().contains("16 devices"), "{r}");
        assert_eq!(e.status(&mut con)["state"], json!("none"), "refused before anything opened");
        let r = e.begin(&mut con, &["read"], json!(["/not/registered"]), Value::Null);
        assert!(r["err"].is_string());
    }

    /// A pairing, as the audit log has it (§5.1): the window opened, the
    /// attempt and its verified proof, the host's Allow, the enrolled device
    /// and the window closed, in that order — and nowhere the comparison
    /// code both screens showed, the link, or its secret.
    #[test]
    fn a_pairing_is_audited_without_its_code_link_or_secret() {
        let e = engine("pairaudit", vec![]);
        e.bind_loopback();
        let mut con = e.connect(Caller::console());
        let r = e.begin(&mut con, &["read"], json!("*"), Value::Null);
        let uri = r["ok"]["uri"].as_str().unwrap_or_else(|| panic!("{r}")).to_string();
        let mut p = e.attempt(&uri, &Attempt::desktop("Mac")).unwrap();
        assert_eq!(e.line(&mut p.tls, Duration::from_secs(5)), Some(json!({"pending": true})));
        let rec = e.rpc(&mut con, "pair_decide", json!({"allow": true}))["ok"].clone();
        assert_eq!(e.closed_as(&mut con)["ended"], json!("paired"));

        let path = e.dir.join("cfg").join(super::super::audit::FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let steps: Vec<(String, String)> = lines
            .iter()
            .filter(|l| l["kind"] == "pairing" || l["kind"] == "device")
            .map(|l| (l["kind"].as_str().unwrap().to_string(), l["detail"]["event"].as_str().unwrap().to_string()))
            .collect();
        let want = [("pairing", "opened"), ("pairing", "attempt"), ("pairing", "verified"), ("pairing", "decided"), ("device", "enrolled"), ("pairing", "closed")];
        assert_eq!(steps, want.map(|(k, e)| (k.to_string(), e.to_string())), "{lines:#?}");
        let enrolled = lines.iter().find(|l| l["detail"]["event"] == "enrolled").unwrap();
        assert_eq!((enrolled["detail"]["subject"]["device"].clone(), enrolled["detail"]["fields"]["spki_sha256"].clone()), (rec["id"].clone(), json!(p.key.pin)));
        let closed = lines.iter().find(|l| l["detail"]["event"] == "closed").unwrap();
        assert_eq!((closed["detail"]["ended"].as_str(), closed["detail"]["subject"]["device"].clone()), (Some("paired"), rec["id"].clone()));
        let link = worktrees_core::pairuri::parse(&uri, 0, &literal).unwrap();
        let secret = worktrees_core::pairuri::base32(&link.secret);
        assert!(!text.contains(&p.code), "the comparison code {} is in the log", p.code);
        assert!(!text.contains(&secret) && !text.contains(&uri), "the link or its secret is in the log");
        assert!(lines.iter().all(|l| l["kind"] != "alert" || l["detail"]["event"] == "enrolled"), "{lines:#?}");
        assert_eq!(super::super::audit::verify(&e.dir.join("cfg")).broken, None);
    }

    /// While a request waits, the device's connection is watched: a second
    /// frame, or leaving, ends the window before the host can Allow it.
    #[test]
    fn a_frame_after_the_first_or_leaving_ends_the_request() {
        let e = engine("pairafter", vec![]);
        e.bind_loopback();
        let mut con = e.connect(Caller::console());
        for (what, ended) in [("another frame", "frame_after_first"), ("leaving", "device_left")] {
            let uri = e.begin(&mut con, &["read"], json!("*"), Value::Null)["ok"]["uri"].as_str().unwrap().to_string();
            let mut p = e.attempt(&uri, &Attempt::desktop("Mac")).unwrap();
            assert_eq!(e.line(&mut p.tls, Duration::from_secs(5)), Some(json!({"pending": true})));
            e.rt.block_on(async {
                use tokio::io::AsyncWriteExt;
                if what == "leaving" {
                    let _ = p.tls.shutdown().await;
                } else {
                    let _ = p.tls.write_all(b"{}\n").await;
                    let _ = p.tls.flush().await;
                }
            });
            if what == "leaving" {
                drop(p);
            }
            assert_eq!(e.closed_as(&mut con)["ended"], json!(ended), "{what}");
            assert_eq!(e.rpc(&mut con, "pair_decide", json!({"allow": true}))["err"], json!("no pairing request is waiting"), "{what}");
        }
    }

    // ── step-up (§2.5) ───────────────────────────────────────────────────────

    fn iphone(reg: &DeviceRegistry, caps: &[Capability]) -> (DeviceRecord, super::super::stepup::testkit::StepUpKey) {
        let k = super::super::stepup::testkit::StepUpKey::new();
        let rec = reg
            .enroll(
                Enrollment {
                    principal: PrincipalId::owner(),
                    name: "iPhone".into(),
                    platform: Platform::Ios,
                    spki_sha256: super::super::devices::hex(&[42; 32]),
                    stepup_spki_sha256: Some(k.pin.clone()),
                    grant: Grant { capabilities: Caps::of(caps), projects: Projects::All },
                    expiry: Expiry::default(),
                    addr: None,
                },
                NOW,
            )
            .unwrap();
        (rec, k)
    }

    /// An iPhone holding `admin`: its admin rows wait for a step-up signed over
    /// THIS connection's exporter; reads never do; the proof holds on its own
    /// connection only; and turning step-up off on the host removes the gate.
    #[test]
    fn a_drive_class_act_needs_a_step_up_proved_on_its_own_connection() {
        let e = engine("stepup", vec![]);
        let (rec, k) = iphone(&e.reg, &[Read, Admin]);
        let ex1 = [1u8; 32];
        let mut c1 = e.connect_with(caller(&rec), Some(ex1));
        let mut c2 = e.connect_with(caller(&rec), Some([2u8; 32]));
        assert_eq!(e.rpc(&mut c1, "devices_list", json!({}))["err"], json!("step-up required"));
        assert!(e.rpc(&mut c1, "get_changelog", json!({})).get("ok").is_some(), "a read is never asked");
        let c = e.rpc(&mut c1, "stepup_challenge", json!({}))["ok"]["challenge"].as_str().unwrap().to_string();
        let mut msg = super::super::stepup::b64_decode(&c).unwrap();
        // Signed over the OTHER connection's exporter: refused.
        let mut wrong = msg.clone();
        wrong.extend_from_slice(&[2u8; 32]);
        let r = e.rpc(&mut c1, "stepup_prove", json!({"spki": worktrees_api::files::b64_encode(&k.spki), "signature": worktrees_api::files::b64_encode(&k.sign(&wrong))}));
        assert!(r["err"].as_str().unwrap().contains("does not verify"), "{r}");
        let c = e.rpc(&mut c1, "stepup_challenge", json!({}))["ok"]["challenge"].as_str().unwrap().to_string();
        msg = super::super::stepup::b64_decode(&c).unwrap();
        msg.extend_from_slice(&ex1);
        let r = e.rpc(&mut c1, "stepup_prove", json!({"spki": worktrees_api::files::b64_encode(&k.spki), "signature": worktrees_api::files::b64_encode(&k.sign(&msg))}));
        assert_eq!(r["ok"]["valid_secs"], json!(300), "{r}");
        assert!(e.rpc(&mut c1, "devices_list", json!({})).get("ok").is_some(), "proved: served");
        assert_eq!(e.rpc(&mut c2, "devices_list", json!({}))["err"], json!("step-up required"), "per connection");
        // Off on the host: a widening, audited — and the gate is gone.
        let mut con = e.connect(Caller::console());
        assert_eq!(e.rpc(&mut con, "device_set_stepup", json!({"id": rec.id.as_str(), "required": false}))["ok"], json!(true));
        assert!(e.rpc(&mut c2, "devices_list", json!({})).get("ok").is_some());
        // The console and a desktop are never asked.
        assert!(e.rpc(&mut con, "devices_list", json!({})).get("ok").is_some());
        assert_eq!(e.rpc(&mut con, "stepup_prove", json!({"spki": "", "signature": ""}))["err"], json!("the console needs no step-up"));
        let mac = enroll(&e.reg, "Mac", 9, &[Read, Admin], Projects::All);
        let mut m = e.connect_with(caller(&mac), Some([3u8; 32]));
        assert!(e.rpc(&mut m, "devices_list", json!({})).get("ok").is_some(), "a desktop has no presence key to ask for");
    }

    /// A step-up refusal is the host's, so the audit log has it as a refusal
    /// with its own reason, `step-up` (§5.1) — never as an admitted call — and
    /// the same act once the step-up holds is an admitted one.
    #[test]
    fn a_step_up_refusal_is_audited_as_a_refusal() {
        let e = engine("stepup-audit", vec![]);
        let (rec, k) = iphone(&e.reg, &[Read, Admin]);
        let ex = [1u8; 32];
        let mut c = e.connect_with(caller(&rec), Some(ex));
        assert_eq!(e.rpc(&mut c, "devices_list", json!({}))["err"], json!("step-up required"));
        let lines = || -> Vec<Value> {
            let text = std::fs::read_to_string(e.dir.join("cfg").join(super::super::audit::FILE)).unwrap();
            text.lines().map(|l| serde_json::from_str(l).unwrap()).filter(|r: &Value| r["cmd"] == json!("devices_list")).collect()
        };
        let recs = lines();
        assert_eq!(recs.len(), 1, "{recs:#?}");
        assert_eq!((recs[0]["kind"].as_str(), recs[0]["outcome"].as_str()), (Some("refused"), Some("refused")), "{:#}", recs[0]);
        assert_eq!(recs[0]["detail"]["reason"], json!("step-up"), "{:#}", recs[0]);
        assert_eq!(recs[0]["device"].as_str(), Some(rec.id.as_str()), "the record names the device");
        let ch = e.rpc(&mut c, "stepup_challenge", json!({}))["ok"]["challenge"].as_str().unwrap().to_string();
        let mut msg = super::super::stepup::b64_decode(&ch).unwrap();
        msg.extend_from_slice(&ex);
        let r = e.rpc(&mut c, "stepup_prove", json!({"spki": worktrees_api::files::b64_encode(&k.spki), "signature": worktrees_api::files::b64_encode(&k.sign(&msg))}));
        assert!(r.get("ok").is_some(), "{r}");
        assert!(e.rpc(&mut c, "devices_list", json!({})).get("ok").is_some());
        let recs = lines();
        assert_eq!((recs[1]["kind"].as_str(), recs[1]["outcome"].as_str()), (Some("call"), Some("ok")), "{recs:#?}");
    }

    /// The bypass a review found: dock shells are the process's, so a device
    /// with `stream` could type into the shell the DESKTOP opened without ever
    /// opening one itself — and only opening was gated. Typing into a shell
    /// this connection does not hold is asked for a step-up; typing into one
    /// it opened (which was) is not; nor is typing into its own attach.
    #[test]
    fn typing_into_a_shell_this_connection_never_opened_needs_a_step_up() {
        let e = engine("stepup-shell", vec![]);
        let a = s(&e.dir.join("a"));
        let (rec, _k) = iphone(&e.reg, &[Read, Stream]);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let call = Call {
            ctx: e.ctx.clone(),
            frames: Frames::with_exporter(Outbox::unbounded(tx), Some([1u8; 32])),
            caller: caller(&rec),
            registry: e.reg.clone(),
            world: Some(Arc::new(Roots { roots: vec![a.clone()], panes: vec![] })),
            devices: e.reg.clone(),
            pairing: e.clients.pairing(),
            flights: Default::default(),
            remote: None,
        };
        let write = |cmd: &str, extra: Value| {
            let mut args = json!({"repo": a, "slug": "(main)", "index": 0, "data": [104, 105], "id": 905});
            args.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
            dispatch::call(&call, cmd, &Args(args.as_object().cloned().unwrap()))
        };
        for cmd in ["shell_write", "shell_resize", "close_shell_session"] {
            assert_eq!(write(cmd, json!({"cols": 80, "rows": 24})), Err(json!("step-up required")), "{cmd} on a shell it never opened");
        }
        // A hold on a shell that is not live (no shell runs in a test) holds
        // nothing either.
        call.frames.note_shell(ShellHold { repo: a.clone(), slug: "(main)".into(), index: 0, gen: 3 });
        assert_eq!(write("shell_write", json!({})), Err(json!("step-up required")), "a hold on no live shell");
        assert_eq!(write("shell_open", json!({"cols": 80, "rows": 24})), Err(json!("step-up required")), "opening is always asked");
        call.frames.note_term(905, "s".into());
        assert_ne!(write("term_write", json!({})), Err(json!("step-up required")), "its own attach");
        // Held means: the shell's LIVE attach is this connection's.
        let row = dispatch::row("shell_write").unwrap();
        let args = Args(json!({"repo": a, "slug": "(main)", "index": 0}).as_object().cloned().unwrap());
        assert!(dispatch::holds_channel_with(&call, row, &args, &|_, _, _| Some(3)), "its own live attach");
        assert!(!dispatch::holds_channel_with(&call, row, &args, &|_, _, _| Some(4)), "re-attached since, or a new shell under the key");
        assert!(!dispatch::holds_channel_with(&call, row, &args, &|_, _, _| None), "no live shell");
        let other = Args(json!({"repo": a, "slug": "(main)", "index": 1}).as_object().cloned().unwrap());
        assert!(!dispatch::holds_channel_with(&call, row, &other, &|_, _, _| Some(3)), "another tab");
    }

    /// Which acts are drive-class (§2.5): instruct, answer and admin, a raised
    /// capability as much as a row's own; OPENING a stream; and any stream act
    /// on a channel this connection does not hold — not typing into one it
    /// opened. Revoking another device is admin; revoking itself is not.
    #[test]
    fn drive_class_is_instruct_answer_admin_and_any_stream_act_not_on_a_held_channel() {
        use super::super::dispatch::{needs_stepup, row, TABLE};
        for r in TABLE {
            let caps: Vec<_> = r.cap.into_iter().collect();
            let want = matches!(r.cap, Some(Instruct | Answer | Admin | Stream));
            assert_eq!(needs_stepup(r, &caps, false), want, "{} on a channel it does not hold", r.name);
            let held = match r.cap {
                Some(Instruct | Answer | Admin) => true,
                Some(Stream) => matches!(r.name, "term_open" | "shell_open"),
                _ => false,
            };
            assert_eq!(needs_stepup(r, &caps, true), held, "{} on a channel it holds", r.name);
        }
        assert!(needs_stepup(row("write_file").unwrap(), &[Write, Instruct], false), "a write raised to instruct by its path");
        let e = engine("stepup-revoke", vec![]);
        let (rec, _k) = iphone(&e.reg, &[Read, Admin]);
        let peer = enroll(&e.reg, "Peer", 8, &[Read], Projects::All);
        let mut c = e.connect_with(caller(&rec), Some([1u8; 32]));
        assert_eq!(e.rpc(&mut c, "device_revoke", json!({"id": peer.id.as_str()}))["err"], json!("step-up required"), "another device: admin");
        e.rt.block_on(c.send(Message::text(json!({"id": 1, "cmd": "device_revoke", "args": {"id": rec.id.as_str()}}).to_string()))).unwrap();
        e.closed(&mut c, "revoked");
    }

    /// A path's class (§7.1, #549) RAISES the act to drive-class. Writing an
    /// instruction file needs `instruct` and reading a secret needs `stream`,
    /// so both wait for the step-up, while a plain file in the same project,
    /// and an instruction file READ, do not. Until #549 every path was plain,
    /// so nothing exercised the raise.
    #[test]
    fn a_paths_class_raises_the_act_to_drive_class_for_step_up() {
        let e = engine("stepup-class", vec![]);
        let a = e.dir.join("a");
        for f in ["AGENTS.md", ".env", "lib.rs"] {
            std::fs::write(a.join(f), "x").unwrap();
        }
        let (rec, k) = iphone(&e.reg, &[Read, Write, Instruct, Stream]);
        let ex = [1u8; 32];
        let mut c = e.connect_with(caller(&rec), Some(ex));
        let due = json!("step-up required");
        let ask = |c: &mut Client, cmd: &str, f: &str| {
            let path = s(&a.join(f));
            let args = if cmd == "read_file" { json!({"path": path, "max_bytes": null}) } else { json!({"path": path, "content": "y", "expected_mtime": null}) };
            e.rpc(c, cmd, args)["err"].clone()
        };
        assert_ne!(ask(&mut c, "read_file", "lib.rs"), due, "a plain read");
        assert_ne!(ask(&mut c, "write_file", "lib.rs"), due, "a plain write");
        assert_ne!(ask(&mut c, "read_file", "AGENTS.md"), due, "an instruction file is read as plain");
        assert_eq!(ask(&mut c, "write_file", "AGENTS.md"), due, "written, it is instruct");
        assert_eq!(ask(&mut c, "read_file", ".env"), due, "a secret is stream, even to read");
        let ch = e.rpc(&mut c, "stepup_challenge", json!({}))["ok"]["challenge"].as_str().unwrap().to_string();
        let mut msg = super::super::stepup::b64_decode(&ch).unwrap();
        msg.extend_from_slice(&ex);
        let r = e.rpc(&mut c, "stepup_prove", json!({"spki": worktrees_api::files::b64_encode(&k.spki), "signature": worktrees_api::files::b64_encode(&k.sign(&msg))}));
        assert!(r.get("ok").is_some(), "{r}");
        assert_ne!(ask(&mut c, "write_file", "AGENTS.md"), due, "proved on this connection");
        assert_ne!(ask(&mut c, "read_file", ".env"), due);
    }

    /// The file is the registry: a device whose record is cut out of
    /// `devices.json`, or whose file is deleted, loses its open connections at
    /// the next look (the expiry watch's minute, or any write), not only its
    /// next request; and a file that turns unreadable closes every device's.
    #[test]
    fn a_device_whose_record_leaves_the_file_is_closed_and_an_unreadable_file_closes_them_all() {
        let e = engine_times("forgotten", vec![], PAIR_LONG, PAIR_LONG, true);
        let file = e.dir.join("cfg").join(super::super::devices::FILE);
        let a = enroll(&e.reg, "A", 1, &[Read], Projects::All);
        let b = enroll(&e.reg, "B", 2, &[Read], Projects::All);
        let (mut ca, mut cb) = (e.connect(caller(&a)), e.connect(caller(&b)));
        let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        v["devices"].as_array_mut().unwrap().retain(|x| x["id"] != json!(a.id.as_str()));
        std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
        e.reg.refresh();
        e.ended(&ca);
        e.closed(&mut ca, "removed");
        assert!(e.rpc(&mut cb, "get_changelog", json!({})).get("ok").is_some(), "the other device is untouched");
        std::fs::write(&file, b"not json").unwrap();
        e.reg.refresh();
        e.ended(&cb);
        e.closed(&mut cb, "unreadable");
        let mut con = e.connect(Caller::console());
        assert!(e.rpc(&mut con, "get_changelog", json!({})).get("ok").is_some(), "the console keeps working");
    }

    /// A revoke cancels the clone jobs the device started (§2.5): every new
    /// job id the resolver namespaces for a connection is remembered for it.
    #[test]
    fn a_revoke_cancels_the_devices_clone_jobs() {
        let e = engine("jobs", vec![]);
        let dev = enroll(&e.reg, "Dev", 1, &[Read, Admin], Projects::All);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let out = Outbox::unbounded(tx);
        let call = Arc::new(Call {
            ctx: e.ctx.clone(),
            frames: Frames::new(out.clone()),
            caller: caller(&dev),
            registry: e.reg.clone(),
            world: Some(Arc::new(Roots { roots: vec![], panes: vec![] })),
            devices: e.reg.clone(),
            pairing: e.clients.pairing(),
            flights: Default::default(),
            remote: None,
        });
        let grant = authz::effective(&*e.reg, &call.caller);
        let mut args = Args(json!({"id": "job-1", "parent": "/x", "url": "https://example.com/r.git"}).as_object().cloned().unwrap());
        let _ = super::super::resolve::authorize(dispatch::row("clone_project").unwrap(), &call, &grant, &mut args, &Roots { roots: vec![], panes: vec![] });
        let job = format!("{}/job-1", call.frames.tag());
        assert_eq!(call.frames.take_jobs(), vec![job.clone()], "the new job is the connection's");
        call.frames.note_job(job.clone());
        let _kill = e.clients.join(7, out, call.clone());
        e.reg.revoke(&Caller::console(), &Grant::everything(), &dev.id, NOW).unwrap();
        assert!(call.frames.take_jobs().is_empty(), "the job {job} was taken to be cancelled");
        assert_eq!(e.clients.len(), 0);
    }
}

// ── resource limits (Phase 2 §6, R6) ─────────────────────────────────────────

/// One whole new client — upgrade, handshake v1, one answered request — or
/// `false` at the first thing that goes wrong. The flood tests count these,
/// so nothing here may panic: a refused connection is a data point.
fn try_served(port: u16, token: &str) -> bool {
    let attempt = || -> Option<()> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        let req = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {DEV_ORIGIN}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        s.write_all(req.as_bytes()).ok()?;
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if s.read(&mut b).ok()? == 0 {
                return None;
            }
            head.push(b[0]);
        }
        head.starts_with(b"HTTP/1.1 101 ").then_some(())?;
        let mut ws = WebSocket::from_raw_socket(s, Role::Client, None);
        let next = |ws: &mut WebSocket<TcpStream>| -> Option<Value> {
            serde_json::from_str(ws.read().ok()?.to_text().ok()?).ok()
        };
        let cn = fresh_hex32();
        ws.send(Message::text(json!({ "hello": cn }).to_string())).ok()?;
        let sn = next(&mut ws)?["challenge"].as_str()?.to_string();
        ws.send(Message::text(json!({ "auth": spec_proof(token, SPEC_CLIENT_LABEL, &cn, &sn) }).to_string())).ok()?;
        (next(&mut ws)?["ok"] == json!(true)).then_some(())?;
        ws.send(Message::text(json!({ "id": 1, "cmd": "get_settings" }).to_string())).ok()?;
        (next(&mut ws)?["id"] == json!(1)).then_some(())
    };
    attempt().is_some()
}

/// `n` tokenless threads, each opening a connection and holding it until the
/// server closes it, then, 50 ms later, opening the next. `hello` makes each
/// one do everything a peer without the token CAN do first — upgrade and say
/// hello — so it waits at the proof instead of at the request line.
///
/// The pause keeps this a test of the RULE and not of a debug build's
/// throughput: under M1's refuse-the-newcomer rule the door stays shut just
/// the same — slots free only at the 5 s request deadline, and an attacker
/// retrying every 50 ms is as likely as anyone to get each one — while
/// displacement lets a newcomer in at once. The release harness measures the
/// flood that reconnects with no pause at all.
fn churn(port: u16, n: usize, hello: bool, stop: Arc<std::sync::atomic::AtomicBool>) -> Vec<std::thread::JoinHandle<u64>> {
    use std::sync::atomic::Ordering::Relaxed;
    (0..n)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut opened = 0u64;
                let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
                while !stop.load(Relaxed) {
                    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else { continue };
                    opened += 1;
                    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                    if hello {
                        let req = format!(
                            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {DEV_ORIGIN}\r\nUpgrade: websocket\r\n\
                             Connection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\n\r\n"
                        );
                        let _ = s.write_all(req.as_bytes());
                        // A masked text frame: {"hello":"<64 hex>"}.
                        let body = json!({ "hello": fresh_hex32() }).to_string().into_bytes();
                        let mut f = vec![0x81u8, 0x80 | body.len() as u8, 0, 0, 0, 0];
                        f.extend_from_slice(&body);
                        let _ = s.write_all(&f);
                    }
                    let mut buf = [0u8; 4096];
                    // Until the server closes it: everything it sends is read
                    // and dropped, as a peer that never proves itself would.
                    while matches!(s.read(&mut buf), Ok(n) if n > 0) {}
                    std::thread::sleep(Duration::from_millis(50));
                }
                opened
            })
        })
        .collect()
}

/// The flood's result: how many of `tries` new clients were served, 50 ms
/// apart, while `n` attackers churn against 32 unauthenticated slots — twice
/// as many attackers as slots, the serve lane's ratio (32 against 16) — and
/// how many connections the flood displaced, which must not be 0: a flood
/// that never reached the cap tests nothing.
fn served_under_churn(tag: &str, n: usize, hello: bool, tries: usize) -> (usize, u64) {
    super::raise_open_file_limit();
    let l = live_with(tag, Limits { connections: 96, unauthenticated: 32 });
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let attackers = churn(l.port(), n, hello, stop.clone());
    std::thread::sleep(Duration::from_millis(500));
    let token = l.token();
    let served = (0..tries)
        .filter(|_| {
            let ok = try_served(l.port(), &token);
            std::thread::sleep(Duration::from_millis(50));
            ok
        })
        .count();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let displaced = l.running.as_ref().unwrap().listener.shared.displaced();
    // The server goes first, so every attacker's read ends and its thread
    // sees the stop.
    drop(l);
    for t in attackers {
        t.join().unwrap();
    }
    (served, displaced)
}

/// **A reconnecting flood of idle sockets does not lock a new client out.**
/// The serve lane measured 0 of 49 on macOS: 32 tokenless threads, each
/// holding a bare socket and reopening it the instant the server closed it,
/// kept all 16 unauthenticated slots full, so every NEW connection — the
/// app's included — was the one closed at accept. A newcomer now displaces
/// the STALEST unauthenticated connection instead, and a real client, which
/// moves forward every millisecond, is never the stalest for long.
#[test]
fn a_reconnecting_flood_of_idle_sockets_does_not_lock_a_new_client_out() {
    let (served, displaced) = served_under_churn("churn-idle", 64, false, 20);
    assert!(displaced > 0, "the flood never filled the unauthenticated slots: this measured nothing");
    assert!(served >= 18, "{served} of 20 new clients served while 64 idle attackers churned ({displaced} displaced)");
}

/// The same with a flood that does everything a peer WITHOUT the token can
/// do — upgrade, hello — and then waits at the proof it cannot give.
#[test]
fn a_reconnecting_flood_that_says_hello_does_not_lock_a_new_client_out() {
    let (served, displaced) = served_under_churn("churn-hello", 64, true, 20);
    assert!(displaced > 0, "the flood never filled the unauthenticated slots: this measured nothing");
    assert!(served >= 18, "{served} of 20 new clients served while 64 hello attackers churned ({displaced} displaced)");
}

// ── authenticated budgets (budget.rs) ────────────────────────────────────────

use super::budget::{Budgets, Rate};
use super::conn::seams;
use super::dispatch::TEST_SHARED;
use worktrees_core::authz::{Caller, DeviceId, Grant, PrincipalId, Registry};

/// Every paired device is the owner's and granted everything: these tests are
/// about what a device may COST, and the resolver's own tests about what it
/// may do.
struct Unscoped;

impl Registry for Unscoped {
    fn stepup_required(&self, _: &PrincipalId, _: &DeviceId) -> bool {
        false
    }
    fn generation(&self) -> u64 {
        0
    }
    fn principal_grant(&self, _: &PrincipalId) -> Option<Grant> {
        None
    }
    fn device_grant(&self, p: &PrincipalId, _: &DeviceId) -> Option<Grant> {
        p.is_owner().then(Grant::everything)
    }
    fn device_principal(&self, _: &DeviceId) -> Option<PrincipalId> {
        Some(PrincipalId::owner())
    }
}

/// The owner's devices, each known to the registry and granted NOTHING: a
/// device it did not know would be closed as it joins (`end_if_gone`), and
/// these tests are about what a device's refused requests cost.
struct Grantless;

impl Registry for Grantless {
    fn stepup_required(&self, _: &PrincipalId, _: &DeviceId) -> bool {
        false
    }
    fn generation(&self) -> u64 {
        0
    }
    fn principal_grant(&self, _: &PrincipalId) -> Option<Grant> {
        None
    }
    fn device_grant(&self, p: &PrincipalId, _: &DeviceId) -> Option<Grant> {
        use worktrees_core::authz::{Caps, Projects};
        p.is_owner().then(|| Grant { capabilities: Caps::of(&[]), projects: Projects::All })
    }
    fn device_principal(&self, _: &DeviceId) -> Option<PrincipalId> {
        Some(PrincipalId::owner())
    }
}

fn live_budgeted(tag: &str, budgets: Budgets) -> Live {
    live_budgeted_in(tag, budgets, Arc::new(Unscoped))
}

/// `live_budgeted`, with the devices' grants from `registry`.
fn live_budgeted_in(tag: &str, budgets: Budgets, registry: Arc<dyn Registry>) -> Live {
    let dir = std::env::temp_dir().join(format!("wt-serve-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let clients = super::conn::Clients::with(registry, Some(Arc::new(Registered)));
    let running = rt
        .block_on(async { start_in(dir.clone(), 0, vec![DEV_ORIGIN.to_string()], super::conn::LIMITS, budgets, clients) })
        .unwrap();
    Live { running: Some(running), dir, rt }
}

/// The next client this test connects is device `id`, the owner's, as a
/// paired device on the remote listener will be.
fn as_device(l: &Live, id: &str) {
    let caller = Caller::paired(PrincipalId::owner(), DeviceId::new(id));
    l.running.as_ref().unwrap().listener.shared.test_callers.lock().unwrap().push_back(caller);
}

fn send(ws: &mut WebSocket<TcpStream>, id: Value, cmd: &str, args: Value) {
    ws.send(Message::text(json!({ "id": id, "cmd": cmd, "args": args }).to_string())).unwrap();
}

/// Replies (frames with an `id`) until `n` have come, by id.
fn replies(ws: &mut WebSocket<TcpStream>, n: usize) -> std::collections::HashMap<String, Value> {
    let mut out = std::collections::HashMap::new();
    while out.len() < n {
        let f = frame(ws, Duration::from_secs(10)).unwrap_or_else(|| panic!("{} of {n} replies came", out.len()));
        if let Some(id) = f.get("id") {
            out.insert(id.to_string(), f);
        }
    }
    out
}

/// **In-flight requests are capped per connection, and a refusal is at
/// once.** Four slow bodies fill a cap of four; the two requests behind them
/// are `busy` before any of the four is done, never queued. When the four are
/// answered, their slots are back.
#[test]
fn requests_past_the_in_flight_cap_are_busy_at_once_and_their_slots_come_back() {
    let mut b = BUDGETS;
    b.loopback.in_flight = 4;
    let l = live_budgeted("inflight", b);
    let mut ws = client(&l);
    let t0 = std::time::Instant::now();
    for i in 0..4 {
        send(&mut ws, json!(i), seams::SLEEP, json!({ "ms": 600 }));
    }
    send(&mut ws, json!(4), seams::SLEEP, json!({ "ms": 600 }));
    send(&mut ws, json!(5), "get_settings", json!({}));
    let first_two = replies(&mut ws, 2);
    assert!(t0.elapsed() < Duration::from_millis(500), "the refusals waited for the bodies: {:?}", t0.elapsed());
    assert_eq!(first_two["4"]["err"], json!("busy"), "{first_two:?}");
    assert_eq!(first_two["5"]["err"], json!("busy"), "{first_two:?}");
    let rest = replies(&mut ws, 4);
    assert!(rest.values().all(|r| r["ok"] == json!(600)), "{rest:?}");
    for i in 10..14 {
        send(&mut ws, json!(i), "get_settings", json!({}));
    }
    assert!(replies(&mut ws, 4).values().all(|r| r.get("ok").is_some()), "the slots came back");
}

/// **A panicking body is answered and gives its slot back.** At a cap of
/// two, two panics leave room for the next two requests; a slot held by a
/// body that never returned would leave none.
#[test]
fn a_body_that_panics_gives_its_in_flight_slot_back() {
    use super::dispatch::TEST_PANIC;
    let mut b = BUDGETS;
    b.loopback.in_flight = 2;
    let l = live_budgeted("panicslot", b);
    let mut ws = client(&l);
    for round in 0..3 {
        send(&mut ws, json!(format!("p{round}a")), TEST_PANIC, json!({ "why": "fell over" }));
        send(&mut ws, json!(format!("p{round}b")), TEST_PANIC, json!({ "why": "fell over" }));
        let r = replies(&mut ws, 2);
        assert!(r.values().all(|f| f["err"] == json!("internal error")), "round {round}: {r:?}");
    }
    send(&mut ws, json!(1), "get_settings", json!({}));
    send(&mut ws, json!(2), "get_settings", json!({}));
    assert!(replies(&mut ws, 2).values().all(|r| r.get("ok").is_some()), "six panics later, both slots are free");
}

/// **A device is rate-limited; the console is not.** A device's burst, then
/// `rate`; the desk sends the same twenty and gets twenty answers.
#[test]
fn a_device_is_rate_limited_and_the_console_is_not() {
    let mut b = BUDGETS;
    b.device.requests = Rate { per_sec: 0.5, burst: 5.0 };
    let l = live_budgeted("rate", b);
    as_device(&l, "phone");
    let mut phone = client(&l);
    let mut desk = client(&l);
    let mut got = std::collections::HashMap::<String, usize>::new();
    for i in 0..20 {
        let r = rpc(&mut phone, json!(i), "get_settings", json!({}));
        let k = if r.get("ok").is_some() { "ok".to_string() } else { r["err"].as_str().unwrap().to_string() };
        *got.entry(k).or_default() += 1;
    }
    assert_eq!(got.get("ok"), Some(&5), "{got:?}");
    assert_eq!(got.get("rate"), Some(&15), "{got:?}");
    for i in 0..20 {
        assert!(rpc(&mut desk, json!(i), "get_settings", json!({})).get("ok").is_some(), "the desk's request {i}");
    }
}

/// **Devices cannot take the bodies the desk needs.** Remote bodies have a
/// host-wide budget: two devices fill it with slow requests, the next device
/// request is `busy`, and the desk's own slow requests run beside them.
#[test]
fn devices_cannot_take_the_bodies_the_desk_needs() {
    let mut b = BUDGETS;
    b.remote_bodies = 4;
    let l = live_budgeted("bodies", b);
    as_device(&l, "phone");
    let mut phone = client(&l);
    as_device(&l, "tablet");
    let mut tablet = client(&l);
    let mut desk = client(&l);
    for i in 0..2 {
        send(&mut phone, json!(i), seams::SLEEP, json!({ "ms": 800 }));
        send(&mut tablet, json!(i), seams::SLEEP, json!({ "ms": 800 }));
    }
    std::thread::sleep(Duration::from_millis(100));
    send(&mut tablet, json!(9), seams::SLEEP, json!({ "ms": 1 }));
    for i in 0..8 {
        send(&mut desk, json!(i), seams::SLEEP, json!({ "ms": 300 }));
    }
    let t = replies(&mut tablet, 1);
    assert_eq!(t["9"]["err"], json!("busy"), "a fifth remote body past a budget of four: {t:?}");
    let d = replies(&mut desk, 8);
    assert!(d.values().all(|r| r["ok"] == json!(300)), "the desk is not starved by devices: {d:?}");
    assert_eq!(replies(&mut phone, 2).len(), 2);
}

/// **A device's connections are capped.** The third connection of a device
/// allowed two is closed once it has authenticated; another device and the
/// desk still get in.
#[test]
fn a_device_past_its_connection_cap_is_closed() {
    let mut b = BUDGETS;
    b.device.connections = 2;
    let l = live_budgeted("devconn", b);
    let mut ins = Vec::new();
    for _ in 0..2 {
        as_device(&l, "phone");
        ins.push(client(&l));
    }
    as_device(&l, "phone");
    let mut third = client(&l);
    send(&mut third, json!(1), "get_settings", json!({}));
    let (bytes, closed) = rest(third.get_mut(), Duration::from_secs(5));
    assert!(closed && !String::from_utf8_lossy(&bytes).contains("\"id\":1"), "the third connection was served");
    let r = audit_record(&l, |r| r["detail"]["limit"] == "device_connections");
    assert_eq!((r["detail"]["episode"].as_str(), r["device"].as_str()), (Some("began"), Some("phone")), "a limit tripped (§5.1): {r}");
    as_device(&l, "tablet");
    let mut other = client(&l);
    assert!(rpc(&mut other, json!(2), "get_settings", json!({})).get("ok").is_some());
    assert!(rpc(&mut ins[0], json!(3), "get_settings", json!({})).get("ok").is_some(), "the two already in are served");
}

/// **A request the resolver refuses takes no slot.** Its checks that need no
/// I/O — the command exists, its exposure and capability — run on the reader
/// before admission (`dispatch::precheck`). So with its one in-flight slot
/// held, a device the registry knows and grants nothing is told `denied` for a read and
/// `unknown command` for nonsense, never `busy`. Each refusal still costs it a
/// request token, so its refusals, and the log line serve writes for each,
/// stop at its rate.
#[test]
fn a_refused_request_takes_no_slot_and_still_costs_a_device_its_rate() {
    use super::resolve::ERR_DENIED;
    let mut b = BUDGETS;
    b.remote.in_flight = 1;
    b.device.requests = Rate { per_sec: 0.001, burst: 5.0 };
    let l = live_budgeted_in("refused", b, Arc::new(Grantless));
    as_device(&l, "phone");
    let mut phone = client(&l);
    // The reader takes frames in order: this one holds the slot (and one
    // token) before the next is read.
    send(&mut phone, json!("hold"), seams::SLEEP, json!({ "ms": 1500 }));
    for i in 0..3 {
        send(&mut phone, json!(i), "get_settings", json!({}));
    }
    send(&mut phone, json!(3), "not_a_command", json!({}));
    send(&mut phone, json!(4), "get_settings", json!({}));
    let r = replies(&mut phone, 5);
    for i in 0..3 {
        assert_eq!(r[&i.to_string()]["err"], json!(ERR_DENIED), "refusal {i}, with the one slot held: {r:?}");
    }
    assert_eq!(r["3"]["err"], json!("unknown command: not_a_command"), "{r:?}");
    assert_eq!(r["4"]["err"], json!("rate"), "five tokens: the hold and four refusals: {r:?}");
    assert_eq!(replies(&mut phone, 1)["\"hold\""]["ok"], json!(1500), "the slot's own request ran");
}

/// §5.1 on the socket. A refusal the reader makes before admission is
/// recorded exactly once, and a limit tripped is ONE record as its episode
/// begins and one as it ends — at the disconnect here, with the count —
/// however many requests it refused: the record is rate-limited as the log
/// line is, or a flood would be a disk-writing flood.
#[test]
fn refusals_and_limits_reach_the_audit_log_once() {
    use super::resolve::ERR_DENIED;
    let mut b = BUDGETS;
    b.device.requests = Rate { per_sec: 0.001, burst: 2.0 };
    let l = live_budgeted_in("auditrate", b, Arc::new(Grantless));
    as_device(&l, "phone");
    let mut phone = client(&l);
    for i in 0..12 {
        send(&mut phone, json!(i), "get_settings", json!({}));
    }
    let r = replies(&mut phone, 12);
    for i in 0..12 {
        let want = if i < 2 { json!(ERR_DENIED) } else { json!("rate") };
        assert_eq!(r[&i.to_string()]["err"], want, "request {i}: {r:?}");
    }
    let _ = phone.close(None);
    drop(phone);
    let ended = |recs: &[Value]| recs.iter().any(|r| r["detail"]["limit"] == "request_rate" && r["detail"]["episode"] == "ended");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ended(&audit_lines(&l)) {
        assert!(std::time::Instant::now() < deadline, "the episode never ended: {:#?}", audit_lines(&l));
        std::thread::sleep(Duration::from_millis(20));
    }
    let recs = audit_lines(&l);
    let refused: Vec<&Value> = recs.iter().filter(|r| r["cmd"] == "get_settings").collect();
    assert_eq!(refused.len(), 2, "one record per refusal the resolver made: {refused:#?}");
    assert!(refused.iter().all(|r| r["detail"]["reason"] == "denied" && r["device"] == "phone"), "{refused:#?}");
    let rate: Vec<&Value> = recs.iter().filter(|r| r["detail"]["limit"] == "request_rate").collect();
    assert_eq!(rate.len(), 2, "ten refusals for rate: one record as they began, one as they ended: {rate:#?}");
    assert_eq!((rate[0]["detail"]["episode"].as_str(), rate[0]["detail"]["answered"].as_str()), (Some("began"), Some("rate")));
    assert_eq!((rate[1]["detail"]["episode"].as_str(), rate[1]["detail"]["count"].as_u64()), (Some("ended"), Some(10)));
    assert!(rate.iter().all(|r| r["kind"] == "refused" && r["principal"] == "owner" && r["device"] == "phone"), "{rate:#?}");
    assert_eq!(super::audit::verify(&l.dir).broken, None);
}

/// #550's listen frames cost a device a request token each, so a device that
/// keeps changing what it listens for is rate-limited like one that keeps
/// asking — and so is the log line each change writes. A frame it has no
/// token for is dropped unapplied, its last set standing. The console's
/// frames are free.
#[test]
fn a_devices_listen_frames_cost_it_requests() {
    let mut b = BUDGETS;
    b.device.requests = Rate { per_sec: 0.001, burst: 1.0 };
    let l = live_budgeted("listenrate", b);
    let clients = l.running.as_ref().unwrap().clients.clone();
    let listen = |ws: &mut WebSocket<TcpStream>, names: &[&str]| ws.send(Message::text(json!({ "listen": names }).to_string())).unwrap();
    as_device(&l, "phone");
    let mut phone = client(&l);
    listen(&mut phone, &["app:open-doc"]);
    listen(&mut phone, &[]);
    assert_eq!(rpc(&mut phone, json!(1), "get_settings", json!({}))["err"], json!("rate"), "two frames, a burst of one");
    assert_eq!(clients.listening("app:open-doc"), 1, "the first applied, the second dropped unapplied");

    let mut desk = client(&l);
    for i in 0..50 {
        listen(&mut desk, if i % 2 == 0 { &["app:open-doc"] } else { &[] });
    }
    assert!(rpc(&mut desk, json!(1), "get_settings", json!({})).get("ok").is_some(), "the console's frames are free");
    assert_eq!(clients.listening("app:open-doc"), 1, "the desk's last frame declared nothing");
}

/// **Stream input is metered per stream, by the byte.** A write that would
/// take a device's attach past its burst is dropped whole and answered
/// `rate`; the admission comes first, so a write the resolver then refuses
/// (an attach this connection never opened) still pays.
#[test]
fn a_devices_stream_input_past_its_rate_is_dropped_and_answered_rate() {
    use super::resolve::ERR_NOT_FOUND;
    let mut b = BUDGETS;
    b.device.input = Rate { per_sec: 1.0, burst: 10.0 };
    let l = live_budgeted("input", b);
    as_device(&l, "phone");
    let mut phone = client(&l);
    let w = |n: usize| json!({ "id": 77, "data": vec![97u8; n] });
    assert_eq!(rpc(&mut phone, json!(1), "term_write", w(8))["err"], json!(ERR_NOT_FOUND), "admitted, then the resolver ran");
    assert_eq!(rpc(&mut phone, json!(2), "term_write", w(8))["err"], json!("rate"));
    assert_eq!(rpc(&mut phone, json!(3), "term_write", json!({ "id": 78, "data": vec![97u8; 8] }))["err"], json!(ERR_NOT_FOUND), "its own bucket");
    let mut desk = client(&l);
    for i in 0..5 {
        assert_eq!(rpc(&mut desk, json!(i), "term_write", w(8))["err"], json!("no such terminal"), "the console is not metered");
    }
}

/// **Idle lanes are reaped; active ones are capped.** A stream key the
/// client names gets a lane only while work is queued on it: 300 writes to
/// 300 distinct ids, one after the other, never meet a cap of four. Four
/// streams with work in flight do, and a fifth is `busy` — while more work
/// for a stream that HAS a lane queues behind it, in order.
#[test]
fn idle_lanes_are_reaped_and_active_ones_are_capped() {
    let mut b = BUDGETS;
    b.loopback.lanes = 4;
    let l = live_budgeted("lanes", b);
    let mut ws = client(&l);
    for i in 0..300 {
        let r = rpc(&mut ws, json!(i), "term_write", json!({ "id": 5000 + i, "data": [97] }));
        assert_eq!(r["err"], json!("no such terminal"), "write {i}: {r}");
    }
    for (i, lane) in ["a", "b", "c", "d"].iter().enumerate() {
        send(&mut ws, json!(format!("s{i}")), seams::SLEEP, json!({ "ms": 600, "lane": lane }));
    }
    send(&mut ws, json!("e"), seams::SLEEP, json!({ "ms": 1, "lane": "e" }));
    send(&mut ws, json!("a2"), seams::SLEEP, json!({ "ms": 1, "lane": "a" }));
    let r = replies(&mut ws, 6);
    assert_eq!(r["\"e\""]["err"], json!("busy"), "a fifth active lane: {r:?}");
    assert_eq!(r["\"a2\""]["ok"], json!(1), "work for an active lane queues on it: {r:?}");
    send(&mut ws, json!("e2"), seams::SLEEP, json!({ "ms": 1, "lane": "e" }));
    assert_eq!(replies(&mut ws, 1)["\"e2\""]["ok"], json!(1), "the lanes drained and went");
}

/// **Concurrent identical reads share one body**, host-wide: five clients
/// asking at once get ONE computation's answer, each under its own id, and
/// the next call — after that answer is out — computes again.
#[test]
fn concurrent_identical_reads_share_one_body() {
    let l = live("flight");
    let mut cs: Vec<WebSocket<TcpStream>> = (0..5).map(|_| client(&l)).collect();
    for (i, c) in cs.iter_mut().enumerate() {
        send(c, json!(i), TEST_SHARED, json!({ "ms": 500, "tag": "flight" }));
    }
    for (i, c) in cs.iter_mut().enumerate() {
        let r = replies(c, 1);
        assert_eq!(r[&i.to_string()]["ok"], json!(1), "client {i} got the one computation's answer: {r:?}");
    }
    assert_eq!(rpc(&mut cs[0], json!(9), TEST_SHARED, json!({ "ms": 1, "tag": "flight" }))["ok"], json!(2), "not a cache");
    // Different arguments are a different question.
    send(&mut cs[0], json!(10), TEST_SHARED, json!({ "ms": 300, "tag": "flight" }));
    send(&mut cs[1], json!(11), TEST_SHARED, json!({ "ms": 300, "tag": "other" }));
    assert_eq!(replies(&mut cs[0], 1)["10"]["ok"], json!(3));
    assert_eq!(replies(&mut cs[1], 1)["11"]["ok"], json!(1));
}

/// A dock shell printing `yes` forever, on channel 7 of `ws`.
fn shell_flooding(ws: &mut WebSocket<TcpStream>, repo: &str) {
    std::env::set_var("SHELL", "/bin/sh");
    let r = rpc(ws, json!(1), "shell_open", json!({ "repo": repo, "slug": "(main)", "index": 0, "cols": 80, "rows": 24, "onBytes": "__CHANNEL__:7" }));
    assert!(r["ok"]["gen"].is_u64(), "shell_open: {r}");
    drain(ws, Duration::from_millis(1000));
    send(ws, json!(2), "shell_write", json!({ "repo": repo, "slug": "(main)", "index": 0, "data": b"yes\r".to_vec() }));
}

/// **A stream that outruns its reader ends; the connection lives.** A client
/// stops reading while its dock shell prints without pause. Unbounded, serve
/// grew by ~500 MB a second (2.5 GB in 10 s, measured). Now the stream stops
/// at the outbox's stream share: the client finds an end frame on the
/// channel when it reads again, the connection still answers, and the shell
/// itself is fine — a fresh attach replays it.
#[test]
fn a_stream_that_outruns_its_reader_ends_and_the_connection_lives() {
    let mut b = BUDGETS;
    b.loopback.outbox = 1 << 20;
    b.loopback.outbox_stream = 64 << 10;
    let l = live_budgeted("streamcap", b);
    let repo = scratch_repo("streamcap");
    let repo_s = repo.to_string_lossy().into_owned();
    let mut ws = client(&l);
    shell_flooding(&mut ws, &repo_s);
    std::thread::sleep(Duration::from_secs(2));
    let mut ended = false;
    let mut data = 0usize;
    // Bounded both ways: a stream that never ends must fail this test, not
    // hang it (it did, with the share switched off).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while let Some(f) = frame(&mut ws, Duration::from_secs(2)) {
        if f["channel"] == json!(7) {
            if f["end"] == json!(true) {
                ended = true;
                break;
            }
            data += f["data"].as_str().map_or(0, str::len);
        }
        if data > 64 << 20 || std::time::Instant::now() > deadline {
            break;
        }
    }
    assert!(ended, "no end frame after {data} bytes of channel data");
    assert!(data < 8 << 20, "{data} bytes reached a client that was not reading");
    let r = rpc(&mut ws, json!("after"), "get_settings", json!({}));
    assert!(r.get("ok").is_some(), "{r}");
    let _ = rpc(&mut ws, json!("x"), "close_shell_session", json!({ "repo": repo_s, "slug": "(main)", "index": 0, "keepCwd": false }));
    let _ = std::fs::remove_dir_all(&repo);
}

/// **A reply into a full outbox closes the connection.** Fifty changelogs
/// (~220 KB each) asked for and never read: the queue passes its budget, and
/// the reply that arrives then ends the connection rather than wait in
/// memory. Under the old unbounded queue all fifty were delivered.
#[test]
fn a_reply_into_a_full_outbox_closes_the_connection() {
    let mut b = BUDGETS;
    b.loopback.outbox = 512 << 10;
    b.loopback.outbox_stream = 64 << 10;
    let l = live_budgeted("replycap", b);
    let mut ws = client(&l);
    for i in 0..50 {
        send(&mut ws, json!(i), "get_changelog", json!({}));
    }
    std::thread::sleep(Duration::from_secs(2));
    let mut got = 0;
    let closed = loop {
        ws.get_ref().set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        match ws.read() {
            Ok(Message::Text(_)) => got += 1,
            Ok(_) => {}
            Err(_) => break true,
        }
        if got == 50 {
            break false;
        }
    };
    assert!(closed && got < 50, "{got} of 50 replies delivered to a client that read none of them for 2 s");
    let r = audit_record(&l, |r| r["detail"]["limit"] == "outbox");
    assert_eq!((r["kind"].as_str(), r["device"].as_str()), (Some("refused"), Some("loopback")), "a limit tripped (§5.1): {r}");
    let mut next = client(&l);
    assert!(rpc(&mut next, json!(1), "get_settings", json!({})).get("ok").is_some(), "the server lives on");
}

/// **Pings without reading cost the stream share, not memory.** 300,000
/// pings — 38 MB of pongs owed, past what the kernel's loopback buffers take
/// (~12 MB, measured: below that nothing backs up in serve at all) — from a
/// client that reads nothing. Unbounded, serve grew by ~90 MB for 200,000
/// (measured), most of it in tungstenite's own pongs. Now the peer is closed
/// once its pongs are a share behind, having been sent far fewer than it
/// asked for, and the server goes on serving.
#[test]
fn a_client_that_pings_without_reading_is_closed_a_stream_share_behind() {
    let mut b = BUDGETS;
    b.loopback.outbox_stream = 16 << 10;
    let l = live_budgeted("pongs", b);
    let mut ws = client(&l);
    let ping = {
        let mut f = vec![0x89u8, 0x80 | 125, 0, 0, 0, 0];
        f.extend(std::iter::repeat_n(b'p', 125));
        f
    };
    let burst: Vec<u8> = ping.iter().copied().cycle().take(ping.len() * 1000).collect();
    for _ in 0..300 {
        // Closed part-way, the writes start failing: that is the outcome.
        if ws.get_mut().write_all(&burst).is_err() {
            break;
        }
    }
    let mut pongs = 0;
    // EINVAL here is macOS refusing a timeout on a socket the peer RESET
    // (see `rest`): closed, which is the outcome.
    let reset = ws.get_ref().set_read_timeout(Some(Duration::from_secs(5))).is_err();
    let closed = reset || loop {
        match ws.read() {
            Ok(Message::Pong(_)) => pongs += 1,
            Ok(_) => {}
            Err(tokio_tungstenite::tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => break false,
            Err(_) => break true,
        }
    };
    assert!(closed, "a client 300,000 pings in without reading is still connected ({pongs} pongs)");
    assert!(pongs < 300_000, "{pongs} of 300,000 pongs delivered: none ever backed up");
    audit_record(&l, |r| r["detail"]["limit"] == "outbox");
    let mut ws2 = client(&l);
    assert!(rpc(&mut ws2, json!(1), "get_settings", json!({})).get("ok").is_some());
}

/// **A device's shell slot lives as long as its stream.** One shell allowed:
/// the second `shell_open` is `busy` while the first is attached — the
/// slot is held by the sink, not by the request that opened it — and gets
/// in once the first is detached.
#[test]
fn a_devices_shell_slot_is_held_by_the_stream_until_it_detaches() {
    std::env::set_var("SHELL", "/bin/sh");
    let mut b = BUDGETS;
    b.device.shells = 1;
    let l = live_budgeted("shellcap", b);
    let repo = scratch_repo("shellcap");
    let repo_s = repo.to_string_lossy().into_owned();
    as_device(&l, "phone");
    let mut phone = client(&l);
    // By id: a channel's frames (and a detach's end frame) can come first.
    let call = |ws: &mut WebSocket<TcpStream>, id: u32, cmd: &str, args: Value| {
        send(ws, json!(id), cmd, args);
        replies(ws, 1).remove(&id.to_string()).unwrap()
    };
    let open = |ws: &mut WebSocket<TcpStream>, id: u32, index: u32| {
        call(ws, id, "shell_open", json!({ "repo": repo_s, "slug": "(main)", "index": index, "cols": 80, "rows": 24, "onBytes": format!("__CHANNEL__:{index}") }))
    };
    let first = open(&mut phone, 1, 0);
    let gen = first["ok"]["gen"].as_u64().unwrap_or_else(|| panic!("{first}"));
    drain(&mut phone, Duration::from_millis(500));
    assert_eq!(open(&mut phone, 2, 1)["err"], json!("busy"), "a second shell while the first is attached");
    let d = call(&mut phone, 3, "shell_detach", json!({ "repo": repo_s, "slug": "(main)", "index": 0, "gen": gen }));
    assert!(d.get("ok").is_some(), "{d}");
    drain(&mut phone, Duration::from_millis(300));
    let again = open(&mut phone, 4, 1);
    assert!(again["ok"]["gen"].is_u64(), "the detach gave the slot back: {again}");
    for i in 0..2u32 {
        let _ = call(&mut phone, 10 + i, "close_shell_session", json!({ "repo": repo_s, "slug": "(main)", "index": i, "keepCwd": false }));
    }
    let _ = std::fs::remove_dir_all(&repo);
}

/// **A serve a test starts cannot write the user's real project registry.**
/// The registry is `$XDG_CONFIG_HOME/worktrees/projects.json` — the USER's,
/// not under the config dir a test gives serve — so a socket test's
/// `add_project` once registered six scratch repos in the real file, which
/// were then deleted from disk (2026-10-22). Every serve started in a test
/// process forbids that (`registry::forbid_real_writes`), and the write now
/// fails, saying why, while the real file never sees the repo.
#[test]
fn a_test_serve_cannot_write_the_users_project_registry() {
    let l = live("realreg");
    let repo = scratch_repo("realreg");
    let canon = std::fs::canonicalize(&repo).unwrap().to_string_lossy().into_owned();
    let mut ws = client(&l);
    let r = rpc(&mut ws, json!(1), "add_project", json!({ "dir": repo.to_string_lossy() }));
    let listed = worktrees_core::registry::read_lenient().roots().contains(&canon);
    if listed {
        // Never leave it behind, whatever the assertions below say.
        let _ = worktrees_core::registry::remove(&canon);
    }
    let _ = std::fs::remove_dir_all(&repo);
    assert!(!listed, "the scratch repo landed in the real registry: {r}");
    let e = r["err"].as_str().unwrap_or_default();
    assert!(e.contains("real project registry"), "the refusal says why: {r}");
}
