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
    let running = rt.block_on(async { start_in(dir.clone(), 0, vec![DEV_ORIGIN.to_string()], limits, clients) }).unwrap();
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
    register("/r");
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
    let ours: Vec<&String> = lines.iter().filter(|s| s.contains(&format!("serve: {TEST_PANIC} panicked"))).collect();
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
    }
    let reg = Arc::new(One(Mutex::new(Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::only(["/r/a"]) })));
    let clients = Arc::new(Clients::with(reg.clone(), None));
    let host = Broadcast(clients.clone());
    let (ctx_tx, mut console) = tokio::sync::mpsc::unbounded_channel();
    let (ph_tx, mut phone) = tokio::sync::mpsc::unbounded_channel();
    clients.add(2, Outbox(ph_tx), Caller::paired(PrincipalId::owner(), DeviceId::new("phone")));
    assert!(!host.has_audience(), "a device alone is no audience for the inbox");
    clients.add(1, Outbox(ctx_tx), Caller::console());
    assert!(host.has_audience());

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
/// that is already in.** Past the cap a NEW connection is closed with nothing
/// said; the authenticated client keeps getting answers throughout; and once
/// the flood lets go, a new client gets in.
#[test]
fn connections_past_the_unauthenticated_cap_are_closed_and_a_served_client_keeps_working() {
    let l = live_with("unauthcap", Limits { connections: 32, unauthenticated: 4 });
    let mut served = client(&l);
    let held: Vec<TcpStream> = (0..4).map(|_| idle_tcp(&l)).collect();
    std::thread::sleep(Duration::from_millis(200));
    for i in 0..6 {
        let mut extra = idle_tcp(&l);
        let (bytes, closed) = rest(&mut extra, Duration::from_secs(1));
        assert!(bytes.is_empty() && closed, "connection {i} past the cap: wrote {bytes:?}, closed {closed}");
    }
    assert_eq!(rpc(&mut served, json!(1), "get_settings", json!({})), json!({ "id": 1, "ok": null }), "the served client is still served");
    drop(held);
    std::thread::sleep(Duration::from_millis(300));
    let mut late = client(&l);
    assert_eq!(rpc(&mut late, json!(2), "get_settings", json!({}))["id"], json!(2), "a new client gets in once the flood lets go");
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
