//! Remote access end to end, in process, on this machine's own LAN address:
//! a real `RemoteAccess` enables a real listener (the classifier, the
//! interface binding, the peer rule) and a device dials it over pinned
//! mutual TLS, upgrades, and runs handshake v1 against the session's
//! exporter, as `crates/worktrees-remote` will. Same-host connects reach a
//! listener tied to its interface on both platforms (measured 2026-10-22,
//! macOS 27 and Linux 6.12), so nothing here needs a second machine.
//!
//! The served case comes FIRST in every test, so a dead listener fails
//! rather than passing each refusal after it. On a CI runner, which always
//! has a LAN address, a missing one fails instead of skipping.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hmac::{KeyInit, Mac};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use worktrees_api::Ctx;
use worktrees_core::authz::{Capability, Caps, Caller, Grant, PrincipalId, Projects};

use super::super::conn::{Broadcast, Clients};
use super::super::devices::{DeviceRecord, DeviceRegistry, Enrollment, Expiry, Platform};
use super::super::identity::Identity;
use super::super::pairing::Pairing;
use super::super::resolve::World;
use super::super::tls::testkit::{device_key, DeviceKey};
use super::super::{netwatch, policy};
use super::super::tls::EXPORTER_SESSION;
use super::{NoAuditYet, Reader, RemoteAccess};

const WAIT: Duration = Duration::from_secs(5);

struct Roots(Vec<String>);

impl World for Roots {
    fn roots(&self) -> Vec<String> {
        self.0.clone()
    }
    fn panes(&self) -> Option<worktrees_core::tmux::PaneList> {
        Some(worktrees_core::tmux::PaneList::from_rows(Vec::new()))
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wt-remote-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for p in ["a", "cfg"] {
        std::fs::create_dir_all(d.join(p)).unwrap();
    }
    std::fs::canonicalize(&d).unwrap()
}

/// This machine's own non-loopback IPv4 address, as the kernel would source
/// a packet from it (a UDP socket "connected" to TEST-NET sends nothing), or
/// `None` on a host with only `lo` — which a CI runner never is.
fn own_lan() -> Option<Ipv4Addr> {
    let s = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

fn lan_or_skip() -> Option<Ipv4Addr> {
    let ip = own_lan();
    if ip.is_none() {
        assert!(std::env::var_os("CI").is_none(), "no LAN address on a CI runner: the remote tests did not run");
        eprintln!("no LAN address on this host; the remote tests did not run");
    }
    ip.filter(|ip| worktrees_core::netclass::classify(IpAddr::V4(*ip)).is_ok())
}

/// A serve's remote half over a registry in memory and a fixture world. The
/// reader drops the routers it sees, so no test sends the enable-time
/// datagram to the real network's router.
struct Host {
    rt: tokio::runtime::Runtime,
    dir: PathBuf,
    reg: Arc<DeviceRegistry>,
    remote: Arc<RemoteAccess>,
    audit: Arc<super::super::audit::Audit>,
}

fn host(tag: &str) -> Host {
    host_with(tag, Arc::new(NoAuditYet))
}

fn host_with(tag: &str, gate: Arc<dyn super::Gate>) -> Host {
    host_full(tag, gate, Arc::new(Recorder::default()))
}

fn host_full(tag: &str, gate: Arc<dyn super::Gate>, advert: Arc<dyn super::Advertiser>) -> Host {
    host_made(tag, Some(gate), advert, |_| {})
}

/// The host as serve makes it: the audit log is the gate (§5.1). `prep` runs
/// on the config dir before the log is opened.
fn host_audited(tag: &str, prep: impl FnOnce(&std::path::Path)) -> Host {
    host_made(tag, None, Arc::new(Recorder::default()), prep)
}

fn host_made(tag: &str, gate: Option<Arc<dyn super::Gate>>, advert: Arc<dyn super::Advertiser>, prep: impl FnOnce(&std::path::Path)) -> Host {
    let dir = scratch(tag);
    prep(&dir.join("cfg"));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let reg = Arc::new(DeviceRegistry::in_memory());
    let (remote, audit) = {
        let _in = rt.enter();
        let pairing = Arc::new(Pairing::new(Some(dir.join("cfg")), reg.clone()));
        let world = Roots(vec![dir.join("a").to_string_lossy().into_owned()]);
        let clients = Arc::new(Clients::serving(reg.clone(), pairing.clone(), Some(Arc::new(world))));
        clients.listen();
        let ctx = Ctx::new(dir.join("cfg"), Arc::new(Broadcast(clients.clone())));
        let read: Reader = Box::new(|| {
            netwatch::read::snapshot().map(|mut s| {
                s.routers.clear();
                s
            })
        });
        // As serve's `conn::start` has it (§5.1–§5.2): the log hears the
        // registry, and the boot cross-check runs before it writes a line.
        let (audit, chain) = super::super::audit::Audit::open(&dir.join("cfg"), None, super::super::audit::Fleet::of(&reg));
        audit.hear(&reg);
        reg.cross_check(&super::super::audit::evidence(&dir.join("cfg"), &chain), worktrees_core::sysclock::now_epoch()).unwrap();
        let gate = gate.unwrap_or_else(|| audit.clone());
        let r = RemoteAccess::new(dir.join("cfg"), ctx, clients.clone(), reg.clone(), pairing, audit.clone(), gate, advert, read);
        clients.set_remote(&r);
        (r, audit)
    };
    Host { rt, dir, reg, remote, audit }
}

impl Host {
    fn pin(&self) -> String {
        Identity::load(&self.dir.join("cfg")).unwrap().unwrap().pin().to_string()
    }

    fn enroll(&self, key: &DeviceKey, caps: &[Capability]) -> DeviceRecord {
        self.reg
            .enroll(
                Enrollment {
                    principal: PrincipalId::owner(),
                    name: "Mac B".into(),
                    platform: Platform::Macos,
                    spki_sha256: key.pin.clone(),
                    stepup_spki_sha256: None,
                    grant: Grant { capabilities: Caps::of(caps), projects: Projects::All },
                    expiry: Expiry::default(),
                    addr: None,
                },
                worktrees_core::sysclock::now_epoch(),
            )
            .unwrap()
    }

    fn at(&self) -> SocketAddr {
        self.remote.at().expect("listening")
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

type Ws = WebSocketStream<TlsStream<TcpStream>>;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The spec's HMAC, computed here and not with the server's code.
fn spec_mac(key: &str, label: &str, cn: &str, sn: &str) -> String {
    let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).unwrap();
    m.update(format!("{label}{cn}\n{sn}").as_bytes());
    hex(&m.finalize().into_bytes())
}

/// What a device sees: TLS pinned to the host, then the upgrade request as
/// given. `Ok` with the WebSocket and the session's v1 key on a 101; `Err`
/// with whatever came back instead (empty: nothing was written).
async fn upgrade(at: SocketAddr, pin: &str, key: &DeviceKey, host: &str, origin: Option<&str>) -> Result<(Ws, String), Vec<u8>> {
    let mut tls = super::super::tls::testkit::dial(at, pin, key).await.map_err(|e| e.to_string().into_bytes())?;
    let v1 = hex(&tls.get_ref().1.export_keying_material([0u8; 32], EXPORTER_SESSION, None).unwrap());
    let origin = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\n{origin}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    if tls.write_all(req.as_bytes()).await.is_err() {
        return Err(Vec::new());
    }
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match tokio::time::timeout(WAIT, tls.read(&mut b)).await {
            Ok(Ok(1)) => head.push(b[0]),
            // TLS 1.3: the server judges the client's certificate after the
            // client's half is done, so its refusal is an alert at this read.
            // A close with nothing said (no close_notify either) is an EOF.
            Ok(Err(e)) if head.is_empty() && e.kind() != std::io::ErrorKind::UnexpectedEof => return Err(format!("tls: {e}").into_bytes()),
            _ => return Err(head),
        }
    }
    if !head.starts_with(b"HTTP/1.1 101 ") {
        return Err(head);
    }
    Ok((WebSocketStream::from_raw_socket(tls, Role::Client, None).await, v1))
}

async fn text(ws: &mut Ws) -> Option<Value> {
    match tokio::time::timeout(WAIT, ws.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str(t.as_str()).ok(),
        _ => None,
    }
}

/// Handshake v1 as the spec has it, with `key` as K: the `ok`, or `None`
/// when the server refused.
async fn v1(ws: &mut Ws, key: &str) -> Option<Value> {
    let cn = hex(&[7u8; 32]);
    ws.send(Message::text(json!({ "hello": cn }).to_string())).await.ok()?;
    let sn = text(ws).await?["challenge"].as_str()?.to_string();
    ws.send(Message::text(json!({ "auth": spec_mac(key, policy::CLIENT_LABEL, &cn, &sn) }).to_string())).await.ok()?;
    let ok = text(ws).await?;
    assert_eq!(ok["proof"], json!(spec_mac(key, policy::SERVER_LABEL, &cn, &sn)), "the server's proof: {ok}");
    Some(ok)
}

async fn call(ws: &mut Ws, cmd: &str, args: Value) -> Value {
    ws.send(Message::text(json!({ "id": 1, "cmd": cmd, "args": args }).to_string())).await.unwrap();
    loop {
        let v = text(ws).await.expect("an answer");
        if v.get("id") == Some(&json!(1)) {
            return v;
        }
    }
}

/// Bytes the server sent before the connection ended, within `WAIT`.
async fn rest(ws: Ws) -> Vec<u8> {
    let mut tls = ws.into_inner();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(WAIT, tls.read_to_end(&mut out)).await;
    out
}

/// A request's answer, or how the connection ended instead: `{closed,
/// reason}` for a close frame, `null` for silence or a dropped socket.
async fn answer(ws: &mut Ws, cmd: &str) -> Value {
    if ws.send(Message::text(json!({ "id": 9, "cmd": cmd, "args": {} }).to_string())).await.is_err() {
        return Value::Null;
    }
    loop {
        match tokio::time::timeout(WAIT, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let v: Value = serde_json::from_str(t.as_str()).unwrap_or_default();
                if v["id"] == json!(9) {
                    return v;
                }
            }
            Ok(Some(Ok(Message::Close(f)))) => return closed(f),
            Ok(Some(Ok(_))) => {}
            _ => return Value::Null,
        }
    }
}

fn closed(f: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>) -> Value {
    f.map_or(json!({ "closed": null }), |f| json!({ "closed": u16::from(f.code), "reason": f.reason.to_string() }))
}

/// THE milestone: a paired device's key completes TLS against the pin, the
/// upgrade, and handshake v1 keyed by the session's exporter; the `ok` names
/// the device with its grant; and the device is served.
#[test]
fn an_enrolled_device_completes_tls_and_handshake_v1_and_is_served() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host("served");
    let st = h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    assert_eq!(st["state"], json!("listening"), "{st}");
    let at = h.at();
    assert_eq!(at.ip(), IpAddr::V4(lan));
    let key = device_key();
    let dev = h.enroll(&key, &[Capability::Read, Capability::Write, Capability::Stream]);
    let pin = h.pin();
    h.rt.block_on(async {
        let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.expect("a 101");
        let ok = v1(&mut ws, &k).await.expect("handshake v1 completes");
        assert_eq!(ok["ok"], json!(true));
        assert_eq!(ok["version"], json!(env!("CARGO_PKG_VERSION")));
        assert_eq!(ok["device"]["id"], json!(dev.id), "{ok}");
        assert_eq!(ok["device"]["principal"], json!("owner"));
        assert_eq!(ok["device"]["caps"], json!(["read", "write", "stream"]));
        assert_eq!(ok["device"]["projects"], json!("*"));
        assert_eq!(ok["device"]["stepup_required"], json!(false));
        let r = call(&mut ws, "get_changelog", json!({})).await;
        assert!(r.get("ok").is_some(), "{r}");
        // A console row, to a device: refused, as on any other socket.
        let r = call(&mut ws, "remote_status", json!({})).await;
        assert_eq!(r["err"], json!("console-only"), "{r}");
    });
    assert_eq!(h.reg.get(&PrincipalId::owner(), &dev.id).map(|d| d.id), Some(dev.id));
}

/// §4.3, §4.5: a key no device holds ends INSIDE TLS, with rustls's alert —
/// no 101, not a byte of HTTP — and so does a revoked one; the enrolled key
/// before them is served. (Refused after the handshake instead, by the
/// post-handshake check alone, it would be a clean close, not an alert.)
#[test]
fn a_key_that_is_not_a_live_devices_is_refused_at_tls() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host("unenrolled");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let good = device_key();
    let dev = h.enroll(&good, &[Capability::Read]);
    h.rt.block_on(async {
        assert!(upgrade(at, &pin, &good, &at.to_string(), None).await.is_ok(), "the enrolled key first");
        let stranger = device_key();
        let got = upgrade(at, &pin, &stranger, &at.to_string(), None).await.err().map(|b| String::from_utf8_lossy(&b).into_owned());
        assert!(got.as_deref().is_some_and(|e| e.starts_with("tls: ") && e.contains("alert")), "a stranger got: {got:?}");
    });
    h.reg.revoke(&Caller::console(), &Grant::everything(), &dev.id, worktrees_core::sysclock::now_epoch()).unwrap();
    h.rt.block_on(async {
        let got = upgrade(at, &pin, &good, &at.to_string(), None).await.err().map(|b| String::from_utf8_lossy(&b).into_owned());
        assert!(got.as_deref().is_some_and(|e| e.starts_with("tls: ") && e.contains("alert")), "the revoked key got: {got:?}");
    });
}

/// The remote rule set (§4.5, decided 2026-10-22): `Host` is the bound
/// literal on its port, and there is NO `Origin` — a browser origin, the
/// app's own, or loopback's Host are each refused with NOTHING written.
#[test]
fn the_upgrade_wants_the_bound_host_and_no_origin_and_refuses_in_silence() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host("upgrade");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    h.enroll(&key, &[Capability::Read]);
    h.rt.block_on(async {
        assert!(upgrade(at, &pin, &key, &at.to_string(), None).await.is_ok(), "served first");
        for (host, origin) in [
            (format!("127.0.0.1:{}", at.port()), None),
            (format!("localhost:{}", at.port()), None),
            (format!("{}:{}", at.ip(), at.port() + 1), None),
            (at.to_string(), Some("tauri://localhost")),
            (at.to_string(), Some("http://evil.example")),
            (at.to_string(), Some("null")),
        ] {
            let got = upgrade(at, &pin, &key, &host, origin).await;
            assert_eq!(got.err(), Some(Vec::new()), "Host {host}, Origin {origin:?}: refused with nothing written");
        }
    });
}

/// Handshake v1 over TLS is keyed by THIS session's exporter: a proof made
/// with anything else ends the connection with nothing after the challenge.
#[test]
fn a_proof_keyed_by_anything_but_the_sessions_exporter_is_refused() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host("proof");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    h.enroll(&key, &[Capability::Read]);
    h.rt.block_on(async {
        let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.unwrap();
        assert!(v1(&mut ws, &k).await.is_some(), "served first");
        let (mut ws, _) = upgrade(at, &pin, &key, &at.to_string(), None).await.unwrap();
        let cn = hex(&[7u8; 32]);
        ws.send(Message::text(json!({ "hello": cn }).to_string())).await.unwrap();
        let sn = text(&mut ws).await.unwrap()["challenge"].as_str().unwrap().to_string();
        let wrong = hex(&[0u8; 32]);
        ws.send(Message::text(json!({ "auth": spec_mac(&wrong, policy::CLIENT_LABEL, &cn, &sn) }).to_string())).await.unwrap();
        assert_eq!(rest(ws).await, Vec::<u8>::new(), "nothing after the challenge");
    });
}

/// The kill switch (§4.1, ASI10): a device's live socket is closed with 1008
/// and a reason, the remote port stops answering, a pairing window can no
/// longer open, and the opt-in is gone.
#[test]
fn the_kill_switch_closes_every_device_socket_and_both_ports() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host("kill");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    h.enroll(&key, &[Capability::Read]);
    let mut ws = h.rt.block_on(async {
        let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.unwrap();
        v1(&mut ws, &k).await.unwrap();
        ws
    });
    let st = h.remote.disable().unwrap();
    assert_eq!(st["state"], json!("off"));
    h.rt.block_on(async {
        let closed = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match ws.next().await {
                    Some(Ok(Message::Close(Some(f)))) => return Some(f),
                    Some(Ok(_)) => continue,
                    _ => return None,
                }
            }
        })
        .await
        .expect("closed within a second");
        let f = closed.expect("with a close frame");
        assert_eq!(f.code, CloseCode::Policy);
        assert!(f.reason.contains("turned off"), "{}", f.reason);
        assert!(tokio::time::timeout(WAIT, TcpStream::connect(at)).await.map(|r| r.is_err()).unwrap_or(true), "the remote port still answers");
    });
    assert_eq!(super::file::read(&h.dir.join("cfg")).unwrap().map(|s| s.on), Some(false), "the opt-in says off");
    // On again: the same port, so a paired device's stored address still works.
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    assert_eq!(h.at(), at, "a kill switch cost the devices their address");
    h.remote.disable().unwrap();
    let pair = h.remote.deps.clients.pairing().begin(
        super::super::pairing::Offer { grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All }, expiry: Expiry::default() },
        worktrees_core::sysclock::now_epoch(),
    );
    assert!(pair.is_err(), "a pairing window opened with remote access off");
}

/// A pairing window opens on the remote listener's address, and its link
/// names that address with no zone and the listener's own pin — what
/// `worktrees remote pair <link>` takes (the client lane's `parse_dial`).
#[test]
fn a_pairing_window_opens_where_remote_access_listens() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host("pairbind");
    let off = h.remote.deps.clients.pairing().begin(
        super::super::pairing::Offer { grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All }, expiry: Expiry::default() },
        worktrees_core::sysclock::now_epoch(),
    );
    assert!(off.is_err(), "no window while remote access is off");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let r = h
        .remote
        .deps
        .clients
        .pairing()
        .begin(
            super::super::pairing::Offer { grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All }, expiry: Expiry::default() },
            worktrees_core::sysclock::now_epoch(),
        )
        .unwrap();
    let uri = r["uri"].as_str().unwrap();
    let link = worktrees_core::pairuri::parse(uri, 0, &worktrees_core::pairuri::dial_rule).unwrap();
    assert_eq!(link.addr.ip(), IpAddr::V4(lan));
    assert_ne!(link.addr.port(), h.at().port(), "its own port, never the remote listener's");
    assert_eq!(link.pin, h.pin());
    h.remote.disable().unwrap();
}

/// The kill switch and a revoke reach a device that is still SHAKING HANDS:
/// upgraded, not yet through handshake v1, so not yet in the client set
/// `end_every_device` / `end_device` walk. It finishes v1 and is closed at
/// once with 1008, never served. (Found by the commit's security review: a
/// session that joined after the sweep survived it.)
#[test]
fn a_device_still_shaking_hands_is_closed_by_the_kill_switch_or_its_revoke() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host("midshake");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    let dev = h.enroll(&key, &[Capability::Read]);
    let served = |ws: &mut Ws| {
        h.rt.block_on(async {
            ws.send(Message::text(json!({ "id": 9, "cmd": "get_changelog", "args": {} }).to_string())).await.ok()?;
            loop {
                match tokio::time::timeout(WAIT, ws.next()).await {
                    Ok(Some(Ok(Message::Text(t)))) => {
                        let v: Value = serde_json::from_str(t.as_str()).ok()?;
                        if v["id"] == json!(9) {
                            return Some(v);
                        }
                    }
                    Ok(Some(Ok(Message::Close(f)))) => return f.map(|f| json!({ "closed": u16::from(f.code), "reason": f.reason.to_string() })),
                    _ => return None,
                }
            }
        })
    };
    // Served first: a whole handshake, then a request.
    let (mut ws, k) = h.rt.block_on(upgrade(at, &pin, &key, &at.to_string(), None)).unwrap();
    h.rt.block_on(v1(&mut ws, &k)).unwrap();
    assert!(served(&mut ws).is_some_and(|v| v.get("ok").is_some()), "served first");
    // The kill switch between the 101 and the proof.
    let (mut ws, k) = h.rt.block_on(upgrade(at, &pin, &key, &at.to_string(), None)).unwrap();
    h.remote.disable().unwrap();
    let ok = h.rt.block_on(v1(&mut ws, &k));
    let after = if ok.is_some() { served(&mut ws) } else { None };
    assert!(after.as_ref().is_none_or(|v| v.get("ok").is_none()), "served after the kill switch: {after:?}");
    // A revoke between the 101 and the proof.
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let at = h.at();
    let (mut ws, k) = h.rt.block_on(upgrade(at, &pin, &key, &at.to_string(), None)).unwrap();
    h.reg.revoke(&Caller::console(), &Grant::everything(), &dev.id, worktrees_core::sysclock::now_epoch()).unwrap();
    let ok = h.rt.block_on(v1(&mut ws, &k));
    let after = if ok.is_some() { served(&mut ws) } else { None };
    assert!(after.as_ref().is_none_or(|v| v.get("ok").is_none()), "served after its revoke: {after:?}");
}

/// §6: a device holds at most four LIVE connections, counted across them on
/// the real listener (`budget.rs`). Four are served. A fifth completes TLS
/// and handshake v1, which is where it is known to be this device, and is
/// then closed with the reason, never served; the four are still served after
/// it. One of them ending makes room again, so the cap counts connections
/// alive, not connections ever made.
#[test]
fn a_devices_fifth_live_connection_is_closed_and_the_four_before_it_are_served() {
    // §6's table, not `BUDGETS`: the test holds the number, so a budget
    // that drifts from the spec fails here.
    const PER_DEVICE: usize = 4;
    let Some(lan) = lan_or_skip() else { return };
    let h = host("fifth");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    h.enroll(&key, &[Capability::Read]);
    h.rt.block_on(async {
        let mut live = Vec::new();
        for n in 1..=PER_DEVICE {
            let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.expect("a 101");
            v1(&mut ws, &k).await.expect("handshake v1 completes");
            let r = answer(&mut ws, "get_changelog").await;
            assert!(r.get("ok").is_some(), "connection {n} of {PER_DEVICE} served: {r}");
            live.push(ws);
        }
        // The fifth: closed, and said why, so the device does not take it for
        // a lost host and dial straight back. Nothing is sent on it first: a
        // request at a socket serve has dropped draws a RST, which on Linux
        // discards the close frame still unread.
        let (mut fifth, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.expect("a 101");
        v1(&mut fifth, &k).await.expect("handshake v1 completes");
        let end = match tokio::time::timeout(WAIT, fifth.next()).await {
            Ok(Some(Ok(Message::Close(f)))) => closed(f),
            other => json!({ "not a close": format!("{other:?}") }),
        };
        assert_eq!(end["closed"], json!(u16::from(CloseCode::Policy)), "the fifth: {end}");
        assert_eq!(end["reason"], json!("this device already holds every connection it may"));
        for (n, ws) in live.iter_mut().enumerate() {
            let r = answer(ws, "get_changelog").await;
            assert!(r.get("ok").is_some(), "connection {} still served after the fifth: {r}", n + 1);
        }
        // One ends; a new one is served once serve has seen it go.
        let mut gone = live.pop().unwrap();
        gone.close(None).await.unwrap();
        drop(gone);
        let deadline = tokio::time::Instant::now() + WAIT;
        let last = loop {
            let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.expect("a 101");
            v1(&mut ws, &k).await.expect("handshake v1 completes");
            let r = answer(&mut ws, "get_changelog").await;
            if r.get("ok").is_some() || tokio::time::Instant::now() > deadline {
                break r;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert!(last.get("ok").is_some(), "a connection after one of the four ended: {last}");
    });
}

/// §5.1: a key refused at the remote door is ONE audit record, and it names
/// the address the key came from. The registry's `ConnectRefused` is that
/// record (`Audit::knock`), from a verifier made for this connection; the
/// listener adds nothing for it, so the auth episode ends counting one knock.
#[test]
fn a_key_refused_at_the_remote_door_is_one_record_naming_its_address() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host_audited("knock", |_| {});
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    h.enroll(&key, &[Capability::Read]);
    h.rt.block_on(async {
        let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.expect("a 101");
        assert!(v1(&mut ws, &k).await.is_some(), "the enrolled key is served first");
    });
    let stranger = device_key();
    assert!(h.rt.block_on(upgrade(at, &pin, &stranger, &at.to_string(), None)).is_err(), "refused inside TLS");
    // The listener's own record, if it made one, lands as its connection
    // task ends, which is at once: half a second covers it before the
    // episode is closed and counted.
    std::thread::sleep(Duration::from_millis(500));
    h.audit.stopped();
    let log = std::fs::read_to_string(h.dir.join("cfg/audit.jsonl")).unwrap();
    let auth: Vec<Value> = log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).filter(|r| r["kind"] == "auth").collect();
    let began = auth.iter().find(|r| r["detail"]["episode"] == "began").unwrap_or_else(|| panic!("no knock recorded: {auth:#?}"));
    assert_eq!(began["detail"]["reason"], json!("unknown-key"), "{began}");
    assert_eq!(began["detail"]["key"], json!(stranger.pin), "{began}");
    assert_eq!(began["peer"], json!(lan.to_string()), "the address the key came from: {began}");
    let ended = auth.iter().find(|r| r["detail"]["episode"] == "ended").unwrap_or_else(|| panic!("{auth:#?}"));
    assert_eq!((&ended["detail"]["count"], &ended["detail"]["reasons"]), (&json!(1), &json!({ "unknown-key": 1 })), "one knock, one count: {ended}");
}

/// §5.2 on the remote door, through serve's own start path (`start_with`): a
/// device written into `devices.json` between two runs, which the chain
/// never enrolled, is disabled at boot and refused at the remote listener's
/// TLS, with its address in the knock's record; the device the chain did
/// enroll is served there.
#[test]
fn a_device_the_chain_never_enrolled_is_refused_at_the_remote_door() {
    use super::super::devices::{DevicesFile, Enrollment, Expiry, Platform, FILE};
    use worktrees_core::authz::DeviceId;
    let Some(lan) = lan_or_skip() else { return };
    let dir = scratch("boot").join("cfg");
    std::fs::create_dir_all(&dir).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let boot = || rt.block_on(async { super::super::start_with(dir.clone(), 0, vec![], super::super::conn::LIMITS) }).unwrap();
    let (real, forged) = (device_key(), device_key());
    let first = boot();
    let e = Enrollment {
        principal: PrincipalId::owner(),
        name: "Mac B".into(),
        platform: Platform::Macos,
        spki_sha256: real.pin.clone(),
        stepup_spki_sha256: None,
        grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All },
        expiry: Expiry::default(),
        addr: None,
    };
    first.devices.as_ref().unwrap().enroll(e, worktrees_core::sysclock::now_epoch()).unwrap();
    first.listener.stop();
    drop(first);
    // Between runs, a same-user process adds a device of its own.
    let path = dir.join(FILE);
    let mut f = DevicesFile::parse(&std::fs::read(&path).unwrap()).unwrap();
    let mut d = f.devices[0].clone();
    (d.id, d.spki_sha256) = (DeviceId::new("d_forged"), forged.pin.clone());
    f.devices.push(d);
    std::fs::write(&path, serde_json::to_vec_pretty(&f).unwrap()).unwrap();

    let second = boot();
    let remote = second.remote.clone().unwrap();
    // No advert: a test announces nothing on the LAN.
    let st = rt.block_on(async { remote.enable(&lan.to_string(), None, None, Some(false)) }).unwrap();
    assert_eq!(st["state"], json!("listening"), "{st}");
    let (at, pin) = (remote.at().unwrap(), Identity::load(&dir).unwrap().unwrap().pin().to_string());
    rt.block_on(async {
        let (mut ws, k) = upgrade(at, &pin, &real, &at.to_string(), None).await.expect("a 101");
        assert!(v1(&mut ws, &k).await.is_some(), "the device the chain enrolled is served");
        assert!(upgrade(at, &pin, &forged, &at.to_string(), None).await.is_err(), "the forged one is refused inside TLS");
    });
    std::thread::sleep(Duration::from_millis(500));
    remote.disable().unwrap();
    second.listener.stop();
    let log = std::fs::read_to_string(dir.join(super::super::audit::FILE)).unwrap();
    let recs: Vec<Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let knock = recs.iter().find(|r| r["kind"] == "auth" && r["detail"]["episode"] == "began").unwrap_or_else(|| panic!("{recs:#?}"));
    assert_eq!((knock["detail"]["reason"].as_str(), knock["detail"]["subject"]["device"].as_str()), (Some("disabled"), Some("d_forged")), "{knock}");
    assert_eq!(knock["peer"], json!(lan.to_string()), "{knock}");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// §4.1: an address outside the allowlist is refused as `not-private` —
/// never as a bind error or "not on this host", whether or not this host
/// holds it — and nothing listens or is written. The LAN address first.
#[test]
fn enable_refuses_anything_outside_the_allowlist_as_not_private() {
    let Some(lan) = lan_or_skip() else { return };
    let h = host("notprivate");
    assert!(h.remote.enable(&lan.to_string(), None, None, None).is_ok(), "the LAN address first");
    h.remote.disable().unwrap();
    for a in ["8.8.8.8", "127.0.0.1", "0.0.0.0", "198.51.100.7", "::1", "2001:db8::1", "::ffff:192.168.1.5", "224.0.0.1"] {
        let e = h.remote.enable(a, None, None, None).unwrap_err();
        assert!(e.starts_with("not-private: ") && !e.starts_with("not-private: not-private"), "{a}: {e}");
        assert_eq!(h.remote.status_value()["state"], json!("off"), "{a}");
        assert_eq!(super::file::read(&h.dir.join("cfg")).unwrap().map(|s| s.on), Some(false), "{a} was written");
    }
    let e = h.remote.enable("192.168.001.5", None, None, None).unwrap_err();
    assert!(e.starts_with("bad-address") || e.starts_with("not-held"), "{e}");
}

/// §9.3, decided by the user 2026-10-22: an app-owned serve idles out on
/// LOOPBACK clients alone, so a device can never hold up a crashed,
/// downgraded or deleted app's serve. With only a device connected the
/// watch stops it after its grace; with a console client too, it does not.
#[test]
fn a_device_alone_does_not_keep_an_app_owned_serve_alive() {
    use super::super::wire::Outbox;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let clients = Arc::new(Clients::default());
    let phone = Caller::paired(PrincipalId::owner(), worktrees_core::authz::DeviceId::new("d_phone"));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    clients.add(1, Outbox::unbounded(tx.clone()), phone);
    let watch = |clients: Arc<Clients>| {
        rt.block_on(async {
            let stop = Arc::new(tokio::sync::Notify::new());
            tokio::spawn(super::super::idle_watch(clients, Duration::from_millis(150), Duration::from_millis(20), stop.clone()));
            tokio::time::timeout(Duration::from_secs(2), stop.notified()).await.is_ok()
        })
    };
    // The console keeps it up (the served case first)...
    clients.add(2, Outbox::unbounded(tx.clone()), Caller::console());
    assert!(!watch(clients.clone()), "stopped with the app's own client connected");
    // ...and a device alone does not.
    clients.remove(2);
    assert!(watch(clients.clone()), "a device alone kept an app-owned serve alive");
}

/// A Gate whose health the test flips.
struct Flaky(std::sync::atomic::AtomicBool);

impl super::Gate for Flaky {
    fn healthy(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn record(&self, _: &'static str, _: Value) {}
}

/// §5.1, the device half, decided with (main): while the audit log cannot
/// take a record, no remote listener binds and a device's non-read request
/// is refused — its reads still served. The console is never held to it.
#[test]
fn while_the_audit_log_is_unhealthy_a_device_only_reads_and_nothing_new_binds() {
    let Some(lan) = lan_or_skip() else { return };
    let gate = Arc::new(Flaky(std::sync::atomic::AtomicBool::new(true)));
    let h = host_with("audit", gate.clone());
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    h.enroll(&key, &[Capability::Read, Capability::Write]);
    let root = h.dir.join("a").to_string_lossy().into_owned();
    let note = json!({ "repo": root, "slug": "(main)", "note": "x" });
    let mut ws = h.rt.block_on(async {
        let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.unwrap();
        v1(&mut ws, &k).await.unwrap();
        ws
    });
    h.rt.block_on(async {
        let r = call(&mut ws, "set_note", note.clone()).await;
        assert_ne!(r["err"], json!("audit-unavailable"), "served first: {r}");
    });
    gate.0.store(false, std::sync::atomic::Ordering::SeqCst);
    h.rt.block_on(async {
        let r = call(&mut ws, "set_note", note.clone()).await;
        assert_eq!(r["err"], json!("audit-unavailable"), "{r}");
        let r = call(&mut ws, "get_changelog", json!({})).await;
        assert!(r.get("ok").is_some(), "a read is still served: {r}");
    });
    h.remote.disable().unwrap();
    let e = h.remote.enable(&lan.to_string(), None, None, None).unwrap_err();
    assert!(e.contains("audit log"), "{e}");
    gate.0.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(h.remote.enable(&lan.to_string(), None, None, None).is_ok());
}

/// §5.3: a device seen from a new address CLASS — enrolled from a tailnet
/// address, now on the LAN — raises an alert, once: its next connection,
/// from the same class, does not.
#[test]
fn a_device_seen_from_a_new_address_class_raises_one_alert() {
    use super::super::devices::DeviceEvent;
    let Some(lan) = lan_or_skip() else { return };
    let h = host("addrclass");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    let dev = h
        .reg
        .enroll(
            Enrollment {
                principal: PrincipalId::owner(),
                name: "Mac B".into(),
                platform: Platform::Macos,
                spki_sha256: key.pin.clone(),
                stepup_spki_sha256: None,
                grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All },
                expiry: Expiry::default(),
                addr: Some("100.101.7.3".into()),
            },
            worktrees_core::sysclock::now_epoch(),
        )
        .unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    {
        let seen = seen.clone();
        h.reg.subscribe(Arc::new(move |e: &DeviceEvent| {
            if let DeviceEvent::NewAddressClass { subject, from, to } = e {
                seen.lock().unwrap().push((subject.device.clone(), from.clone(), to.clone()));
            }
        }));
    }
    for _ in 0..2 {
        h.rt.block_on(async {
            let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.unwrap();
            v1(&mut ws, &k).await.unwrap();
        });
    }
    let got = seen.lock().unwrap().clone();
    assert_eq!(got, vec![(dev.id.clone(), "cgnat".to_string(), "rfc1918".to_string())], "one alert, for the one move");
    assert!(DeviceEvent::NewAddressClass { subject: dev.subject(), from: "cgnat".into(), to: "rfc1918".into() }.alerts());
}

/// Records what serve asks the OS's mDNS responder to do (§4.7).
#[derive(Default)]
struct Recorder(std::sync::Mutex<Vec<String>>);

impl super::Advertiser for Recorder {
    fn register(&self, ad: &super::Advert) -> Result<String, String> {
        self.0.lock().unwrap().push(format!("register {} {}:{} if{}", ad.id, ad.ip, ad.port, ad.index));
        Ok(format!("wt-{}.local", ad.id))
    }
    fn withdraw(&self) {
        self.0.lock().unwrap().push("withdraw".into());
    }
}

impl Recorder {
    fn registered(&self) -> usize {
        self.0.lock().unwrap().iter().filter(|e| e.starts_with("register")).count()
    }
}

/// §4.7 D6: the host's switch for the advert works BEFORE remote access is
/// ever on, and with it off nothing is ever registered — not for an instant
/// at enable; `remote_enable {advertise: false}` holds the same. Turned on
/// while listening, it registers once, the pairing port never; a pause or
/// the kill switch withdraws it. With no preference at all the advert
/// starts on, as §4.7 decides ("it starts on, as §11.1 N2 decided").
#[test]
fn with_the_advert_switched_off_nothing_is_ever_registered() {
    let Some(lan) = lan_or_skip() else { return };
    let rec = Arc::new(Recorder::default());
    let h = host_full("advert", Arc::new(NoAuditYet), rec.clone());
    // No preference: on (§4.7), registered once on the LAN bind — served first.
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    assert_eq!(rec.registered(), 1, "{:?}", rec.0.lock().unwrap());
    let id = Identity::load(&h.dir.join("cfg")).unwrap().unwrap().discovery_id().to_string();
    assert_eq!(rec.0.lock().unwrap()[0], format!("register {id} {}:{} if{}", lan, h.at().port(), lan_index(lan)), "the remote listener's port, never the pairing port");
    h.remote.disable().unwrap();
    assert_eq!(rec.0.lock().unwrap().last().map(String::as_str), Some("withdraw"));
    // Switched off while remote access is off: stored, and nothing goes out.
    rec.0.lock().unwrap().clear();
    let st = h.remote.set_advertise(false).unwrap();
    assert_eq!(st["advertise"], json!(false), "{st}");
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    assert_eq!(rec.registered(), 0, "advertised with the switch off: {:?}", rec.0.lock().unwrap());
    // On while listening: now.
    h.remote.set_advertise(true).unwrap();
    assert_eq!(rec.registered(), 1);
    h.remote.disable().unwrap();
    // `remote_enable {advertise: false}`: never, and it is remembered.
    rec.0.lock().unwrap().clear();
    h.remote.enable(&lan.to_string(), None, None, Some(false)).unwrap();
    assert_eq!(rec.registered(), 0);
    assert_eq!(super::file::read(&h.dir.join("cfg")).unwrap().map(|s| s.advertise), Some(false));
    h.remote.disable().unwrap();
    // And a fresh config dir: the switch works before anything was ever on.
    let fresh = host_full("advert-fresh", Arc::new(NoAuditYet), rec.clone());
    rec.0.lock().unwrap().clear();
    fresh.remote.set_advertise(false).unwrap();
    assert_eq!(fresh.remote.status_value()["state"], json!("off"));
    fresh.remote.enable(&lan.to_string(), None, None, None).unwrap();
    assert_eq!(rec.registered(), 0, "the first enable advertised against the stored preference");
    fresh.remote.disable().unwrap();
}

fn lan_index(lan: Ipv4Addr) -> u32 {
    worktrees_core::netif::interfaces().unwrap().into_iter().find(|i| i.holds(IpAddr::V4(lan)).is_some()).unwrap().index
}

/// serve stopping withdraws the advert and closes every device's session
/// with a reason, and keeps the opt-in on for the next serve.
#[test]
fn a_stopping_serve_withdraws_the_advert_and_tells_its_devices() {
    let Some(lan) = lan_or_skip() else { return };
    let rec = Arc::new(Recorder::default());
    let h = host_full("shutdown", Arc::new(NoAuditYet), rec.clone());
    h.remote.enable(&lan.to_string(), None, None, None).unwrap();
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    h.enroll(&key, &[Capability::Read]);
    let mut ws = h.rt.block_on(async {
        let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.unwrap();
        v1(&mut ws, &k).await.unwrap();
        ws
    });
    assert_eq!(rec.registered(), 1, "advertised first");
    h.remote.shutdown();
    assert_eq!(rec.0.lock().unwrap().last().map(String::as_str), Some("withdraw"));
    let f = h.rt.block_on(async {
        loop {
            match tokio::time::timeout(WAIT, ws.next()).await {
                Ok(Some(Ok(Message::Close(f)))) => return f,
                Ok(Some(Ok(_))) => continue,
                _ => return None,
            }
        }
    });
    assert!(f.is_some_and(|f| f.reason.contains("stopping")), "no close frame saying why");
    assert_eq!(super::file::read(&h.dir.join("cfg")).unwrap().map(|s| s.on), Some(true), "the opt-in stays on");
}

/// §4.2, §6: a remote session pings, closes a peer that stops answering
/// (`dead_after` with no pong) and closes at its lifetime whatever it is
/// doing, each with a close frame saying why. A loopback session has none of
/// these. Driven on short times over a socket pair.
#[test]
fn a_remote_session_is_kept_alive_by_pongs_and_ends_at_its_lifetime() {
    use super::super::conn::{serve_caller, Liveness, RemoteSession};
    use tokio_tungstenite::tungstenite::protocol::Role as R;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let dir = scratch("liveness");
    // A live device's session: one the registry does not serve is ended as
    // soon as it joins (`Clients::end_if_gone`), which is not this test's.
    let reg = Arc::new(DeviceRegistry::in_memory());
    let clients = {
        let _in = rt.enter();
        Arc::new(Clients::serving(reg.clone(), Arc::new(Pairing::new(None, reg.clone())), None))
    };
    let ctx = Ctx::new(dir.join("cfg"), Arc::new(Broadcast(clients.clone())));
    let dev = reg
        .enroll(
            Enrollment {
                principal: PrincipalId::owner(),
                name: "Mac B".into(),
                platform: Platform::Macos,
                spki_sha256: device_key().pin,
                stepup_spki_sha256: None,
                grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All },
                expiry: Expiry::default(),
                addr: None,
            },
            worktrees_core::sysclock::now_epoch(),
        )
        .unwrap();
    let phone = Caller::paired(PrincipalId::owner(), dev.id.clone());
    // Answering: the close frame (code, reason, when). Silent: when serve
    // ENDED the session — the server's own truth, because a client that
    // never read answers the queued pings only once it reads, into a socket
    // serve has already closed, and Linux then resets the connection and
    // drops the unread close frame (macOS 27 delivered it; Linux 6.12 did not).
    enum Seen {
        Frame(CloseCode, String, Duration),
        Ended(Duration),
    }
    let run = |live: Liveness, answer: bool| {
        let (clients, ctx, phone) = (clients.clone(), ctx.clone(), phone.clone());
        rt.block_on(async move {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = l.local_addr().unwrap();
            let dial = tokio::spawn(TcpStream::connect(addr));
            let (server, _) = l.accept().await.unwrap();
            let ws = WebSocketStream::from_raw_socket(server, R::Server, None).await;
            let remote = RemoteSession { live, bind_open: Arc::new(std::sync::atomic::AtomicBool::new(true)) };
            let t0 = std::time::Instant::now();
            let (audit, _) = super::super::audit::Audit::open(&ctx.config_dir, None, super::super::audit::Fleet::none());
            let au = Arc::new(super::super::audit::ConnAudit::new(audit, 77, None, phone.clone()));
            let session = tokio::spawn(serve_caller(ws, ctx, clients, 77, phone, None, au, Some(remote)));
            let mut c = WebSocketStream::from_raw_socket(dial.await.unwrap().unwrap(), R::Client, None).await;
            if !answer {
                // Never read: tungstenite answers a ping only while reading.
                // Bounded, so a session that never ends fails here.
                let ended = tokio::time::timeout(Duration::from_secs(5), session).await;
                drop(c);
                return ended.ok().map(|_| Seen::Ended(t0.elapsed()));
            }
            // One deadline for the whole wait: a server that never closes
            // (the rule under test broken) keeps pinging, and must fail here
            // rather than hang.
            let wait = async {
                loop {
                    match c.next().await {
                        Some(Ok(Message::Close(Some(f)))) => return Some(Seen::Frame(f.code, f.reason.to_string(), t0.elapsed())),
                        Some(Ok(_)) => continue,
                        _ => return None,
                    }
                }
            };
            tokio::time::timeout(Duration::from_secs(5), wait).await.ok().flatten()
        })
    };
    let ms = Duration::from_millis;
    // Answering every ping: alive past many `dead_after`s, then the lifetime.
    let Some(Seen::Frame(code, why, took)) = run(Liveness { ping_every: ms(40), dead_after: ms(150), lifetime: ms(700) }, true) else {
        panic!("not closed at its lifetime")
    };
    assert_eq!(code, CloseCode::Away);
    assert!(why.contains("lifetime"), "{why}");
    assert!(took >= ms(700), "closed before its lifetime: {took:?}");
    // Silent: ended for want of a pong — after `dead_after`, and nowhere near
    // the minute-long lifetime, which is the only other way it ends here.
    let Some(Seen::Ended(took)) = run(Liveness { ping_every: ms(40), dead_after: ms(150), lifetime: Duration::from_secs(60) }, false) else {
        panic!("a silent peer was never closed")
    };
    assert!(took >= ms(150) && took < Duration::from_secs(5), "ended after {took:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// §5.1 as serve wires it: the audit log IS the gate. A log whose file
/// another user could read (mode 0644 — the log keeps it as it found it and
/// reports itself unfit) keeps remote access off; a fit one lets it on, and
/// then records the host's acts on remote access and a device's own acts
/// under that device's id and address.
#[test]
fn the_audit_log_gates_remote_access_and_records_its_devices() {
    use std::os::unix::fs::PermissionsExt;
    let Some(lan) = lan_or_skip() else { return };
    // Fit first: the served case.
    let h = host_audited("audited", |_| {});
    h.remote.enable(&lan.to_string(), None, None, None).expect("a fit log lets remote access on");
    let (at, pin) = (h.at(), h.pin());
    let key = device_key();
    let dev = h.enroll(&key, &[Capability::Read, Capability::Write]);
    let root = h.dir.join("a").to_string_lossy().into_owned();
    h.rt.block_on(async {
        let (mut ws, k) = upgrade(at, &pin, &key, &at.to_string(), None).await.unwrap();
        v1(&mut ws, &k).await.unwrap();
        call(&mut ws, "set_note", json!({ "repo": root, "slug": "(main)", "note": "x" })).await;
    });
    let log = std::fs::read_to_string(h.dir.join("cfg/audit.jsonl")).unwrap();
    let lines: Vec<Value> = log.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let has = |f: &dyn Fn(&Value) -> bool| lines.iter().any(f);
    assert!(has(&|r| r["kind"] == json!("lifecycle") && r["detail"]["event"] == json!("remote_enabled")), "{log}");
    assert!(has(&|r| r["kind"] == json!("lifecycle") && r["detail"]["event"] == json!("remote_started")), "{log}");
    assert!(
        has(&|r| r["cmd"] == json!("set_note") && r["device"] == json!(dev.id) && r["peer"] == json!(lan.to_string())),
        "the device's act is not recorded as the device's: {log}"
    );
    // Unfit: off, and saying why.
    let bad = host_audited("audit-unfit", |cfg| {
        std::fs::write(cfg.join("audit.jsonl"), "").unwrap();
        std::fs::set_permissions(cfg.join("audit.jsonl"), std::fs::Permissions::from_mode(0o644)).unwrap();
    });
    let e = bad.remote.enable(&lan.to_string(), None, None, None).unwrap_err();
    assert!(e.contains("audit log"), "{e}");
    assert_eq!(bad.remote.status_value()["state"], json!("off"));
}
