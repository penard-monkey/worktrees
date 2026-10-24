//! Shared by the remote-access witnesses (`remote_bin.rs`, and the Linux
//! NET_ADMIN ones in `remote_net_linux.rs`): a serve in a sandbox, the
//! console over loopback handshake v1, and a device written from the spec —
//! its key, its pinned TLS, the upgrade, handshake v1 keyed by the session,
//! and pairing through the real port — never from serve's code.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use hmac::{KeyInit, Mac};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocket};
use tokio_tungstenite::tungstenite::Message;
pub const WAIT: Duration = Duration::from_secs(5);

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_worktrees")
}

// ── the host ─────────────────────────────────────────────────────────────────

pub struct Sandbox {
    pub root: PathBuf,
    child: Option<Child>,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Sandbox {
    /// A serve with its own HOME, registry and config dir, and the user's
    /// tmux out of reach.
    pub fn launch(tag: &str) -> (Sandbox, Value) {
        let root = std::env::temp_dir().join(format!("wt-remote-bin-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["home", "xdg", "cfg", "tmux"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let child = Command::new(bin())
            .args(["serve", "--config-dir"])
            .arg(root.join("cfg"))
            .env("HOME", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("xdg"))
            .env("SHELL", "/bin/sh")
            .env_remove("TMUX")
            .env("TMUX_TMPDIR", root.join("tmux"))
            .env_remove("WORKTREES_CODEX_PERMISSIONS")
            .env_remove("WORKTREES_PI_PROJECT_TRUST")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn worktrees serve");
        let pid = child.id();
        let sb = Sandbox { root, child: Some(child) };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(v) = std::fs::read(sb.root.join("cfg/serve.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                if v["pid"].as_u64() == Some(pid as u64) {
                    return (sb, v);
                }
            }
            assert!(Instant::now() < deadline, "serve.json never appeared");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// This machine's own non-loopback IPv4 address, as the kernel would source
/// a packet from it (a UDP socket "connected" to TEST-NET sends nothing).
pub fn own_lan_addr() -> Option<Ipv4Addr> {
    let s = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

pub fn lan_or_skip() -> Option<Ipv4Addr> {
    let ip = own_lan_addr();
    if ip.is_none() {
        assert!(std::env::var_os("CI").is_none(), "no LAN address on a CI runner: the remote witness did not run");
        eprintln!("no LAN address on this host; the remote witness did not run");
    }
    ip
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

pub fn hmac(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut m = hmac::Hmac::<Sha256>::new_from_slice(key).unwrap();
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().to_vec()
}

/// Handshake v1 as the spec has it, on any stream: `key` is the HMAC key.
pub fn v1<S: Read + Write>(ws: &mut WebSocket<S>, key: &str, remote: bool) -> Option<Value> {
    let cn = hex(&[0x5a; 32]);
    let mut hello = json!({ "hello": cn });
    if remote { hello["proto"] = json!(worktrees_remote::protocol::PROTO); }
    ws.send(Message::text(hello.to_string())).ok()?;
    let ch: Value = serde_json::from_str(ws.read().ok()?.to_text().ok()?).ok()?;
    let sn = ch["challenge"].as_str()?.to_string();
    let proof = |label: &str| hex(&hmac(key.as_bytes(), &[label.as_bytes(), cn.as_bytes(), b"\n", sn.as_bytes()]));
    ws.send(Message::text(json!({ "auth": proof("worktrees-serve/v1/client\n") }).to_string())).ok()?;
    let ok: Value = serde_json::from_str(ws.read().ok()?.to_text().ok()?).ok()?;
    assert_eq!(ok["proof"], json!(proof("worktrees-serve/v1/server\n")), "the server's proof: {ok}");
    Some(ok)
}

/// One request and its answer, skipping events and channel frames.
pub fn rpc<S: Read + Write>(ws: &mut WebSocket<S>, id: u64, cmd: &str, args: Value) -> Value {
    ws.send(Message::text(json!({ "id": id, "cmd": cmd, "args": args }).to_string())).unwrap();
    loop {
        let v: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        if v["id"] == json!(id) {
            return v;
        }
    }
}

/// The console: loopback, the app's Origin, serve.json's token as the key.
pub fn console(sf: &Value) -> WebSocket<TcpStream> {
    let port = sf["port"].as_u64().unwrap() as u16;
    let mut s = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), WAIT).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let head = upgrade(&mut s, &format!("127.0.0.1:{port}"), Some("tauri://localhost")).expect("the console's 101");
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    let mut ws = WebSocket::from_raw_socket(s, Role::Client, None);
    v1(&mut ws, sf["token"].as_str().unwrap(), false).expect("the console's handshake");
    ws
}

/// Write an upgrade request; the response head, or `None` when the server
/// closed (or alerted) before a byte of one.
pub fn upgrade<S: Read + Write>(s: &mut S, host: &str, origin: Option<&str>) -> Option<String> {
    let origin = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\n{origin}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    s.write_all(req.as_bytes()).ok()?;
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut b) {
            Ok(1) => head.push(b[0]),
            _ => return (!head.is_empty()).then(|| String::from_utf8_lossy(&head).into_owned()),
        }
    }
    Some(String::from_utf8_lossy(&head).into_owned())
}

// ── the device ───────────────────────────────────────────────────────────────

/// A device's P-256 key and its self-signed certificate (§2.1).
pub struct Device {
    pub cert: CertificateDer<'static>,
    pub key: PrivatePkcs8KeyDer<'static>,
    /// SHA-256 of its SPKI, lowercase hex.
    pub pin: String,
}

pub fn device() -> Device {
    use rcgen::PublicKeyData;
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap().self_signed(&pair).unwrap();
    Device { cert: cert.der().clone(), key: PrivatePkcs8KeyDer::from(pair.serialize_der()), pin: hex(&Sha256::digest(pair.subject_public_key_info())) }
}

/// The client's verifier (§4.3): the server's SPKI must equal the pin.
#[derive(Debug)]
pub struct Pinned(pub String);

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, ee: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let spki = rustls::server::ParsedCertificate::try_from(ee)?.subject_public_key_info();
        if hex(&Sha256::digest(spki.as_ref())) == self.0 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
        }
    }
    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.3 only".into()))
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

pub type Tls = StreamOwned<ClientConnection, TcpStream>;

/// TLS 1.3 to `addr`, pinning `pin`, presenting `dev`'s key (or none); no
/// SNI, nothing resumed. The handshake completes on the client's side; a
/// server that refuses the certificate says so at the first read (§4.3).
pub fn tls(addr: SocketAddr, pin: &str, dev: Option<&Device>) -> Result<Tls, String> {
    let b = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned(pin.to_string())));
    let mut cfg = match dev {
        Some(d) => b.with_client_auth_cert(vec![d.cert.clone()], PrivateKeyDer::Pkcs8(d.key.clone_key())).unwrap(),
        None => b.with_no_client_auth(),
    };
    cfg.enable_sni = false;
    cfg.resumption = rustls::client::Resumption::disabled();
    let mut conn = ClientConnection::new(Arc::new(cfg), ServerName::IpAddress(addr.ip().into())).unwrap();
    let mut tcp = TcpStream::connect_timeout(&addr, WAIT).map_err(|e| format!("connect: {e}"))?;
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp).map_err(|e| format!("handshake: {e}"))?;
    }
    Ok(StreamOwned::new(conn, tcp))
}

/// A device's remote session: TLS, the upgrade (the bound literal as Host,
/// no Origin), handshake v1 keyed by the session's exporter. `Err` with what
/// happened instead.
pub fn session(at: SocketAddr, pin: &str, dev: &Device) -> Result<(WebSocket<Tls>, Value), String> {
    let mut s = tls(at, pin, Some(dev))?;
    let key = hex(&s.conn.export_keying_material([0u8; 32], b"EXPORTER-worktrees-remote-v1", None).unwrap());
    match upgrade(&mut s, &at.to_string(), None) {
        Some(h) if h.starts_with("HTTP/1.1 101 ") => {}
        other => return Err(format!("no 101: {other:?}")),
    }
    let mut ws = WebSocket::from_raw_socket(s, Role::Client, None);
    let ok = v1(&mut ws, &key, true).ok_or("handshake v1 refused")?;
    Ok((ws, ok))
}

/// A server that refuses the client's certificate: the first read after the
/// handshake is a TLS alert, and no application byte ever arrives.
pub fn refused_at_tls(r: Result<Tls, String>) -> bool {
    match r {
        Err(_) => true,
        Ok(mut s) => {
            let mut b = [0u8; 1];
            matches!(s.read(&mut b), Err(e) if e.to_string().contains("alert"))
        }
    }
}

/// Pair `dev` with the host from end to end (§2.2): the console opens a
/// window, the device dials the pairing port from the link and proves the
/// secret over TLS bound to the session, the host allows it.
pub fn pair(con: &mut WebSocket<TcpStream>, dev: &Device, caps: &[&str]) -> (Value, SocketAddr) {
    let w = rpc(con, 100, "pair_begin", json!({ "caps": caps, "projects": "*", "expiry": null }));
    let uri = w["ok"]["uri"].as_str().unwrap_or_else(|| panic!("pair_begin: {w}")).to_string();
    let link = worktrees_core::pairuri::parse(&uri, 0, &worktrees_core::pairuri::dial_rule).unwrap();
    let mut s = tls(link.addr, &link.pin, Some(dev)).unwrap();
    let exporter = s.conn.export_keying_material([0u8; 32], b"EXPORTER-worktrees-pair-v1", None).unwrap();
    let name = "CI device";
    let proof = hmac(&link.secret, &[b"worktrees-pair/v1\n", &exporter, &unhex(&dev.pin), &[0u8; 32], name.as_bytes()]);
    let platform = if cfg!(target_os = "macos") { "macos" } else { "linux" };
    let frame = json!({ "pair": { "name": name, "platform": platform, "stepup_spki": null }, "proof": hex(&proof) });
    s.write_all(format!("{frame}\n").as_bytes()).unwrap();
    let mut lines = BufReader::new(s);
    let mut line = String::new();
    lines.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), json!({ "pending": true }), "{line}");
    let d = rpc(con, 101, "pair_decide", json!({ "allow": true }));
    assert!(d.get("ok").is_some(), "pair_decide: {d}");
    line.clear();
    lines.read_line(&mut line).unwrap();
    let paired: Value = serde_json::from_str(&line).unwrap();
    assert!(paired["paired"]["device_id"].is_string(), "{line}");
    (paired["paired"].clone(), link.addr)
}

/// The answer from the far side, until the socket closes: the close frame
/// (code, reason), or `None` if it ended without one within `within`.
pub fn closed_with<S: Read + Write>(ws: &mut WebSocket<S>, within: Duration) -> Option<(CloseCode, String)> {
    let t0 = Instant::now();
    while t0.elapsed() < within {
        match ws.read() {
            Ok(Message::Close(Some(f))) => return Some((f.code, f.reason.to_string())),
            Ok(_) => continue,
            Err(_) => return None,
        }
    }
    None
}

