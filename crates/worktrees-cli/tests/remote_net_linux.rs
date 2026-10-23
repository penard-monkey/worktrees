//! §4.6's bind and address witnesses that need scratch interfaces — steps
//! 2, 4 and 5 — on Linux, as root in a network namespace of its own: a
//! container with `NET_ADMIN` and `/dev/net/tun` (measured there, Linux 6.12;
//! the GitHub runners' own kernels are (main)'s CHECKLIST D2). Each test makes
//! its interfaces, says what it proves, and removes them.
//!
//! They run only with `WT_NET_ADMIN=1`, and only on Linux: they change the
//! network configuration of the namespace they run in, which is never the
//! host's on a developer's machine. The macOS half (an alias under `sudo` on
//! a runner) is unmeasured, CHECKLIST D1.
//!
//! ```sh
//! docker run --rm --cap-add NET_ADMIN --device /dev/net/tun \
//!   --sysctl net.ipv4.conf.all.promote_secondaries=1 -v "$PWD":/w:ro \
//!   -e CARGO_TARGET_DIR=/tmp/t rust:1-bookworm sh -c \
//!   'apt-get -qq update && apt-get -qq install -y iproute2 && cd /w && \
//!    WT_NET_ADMIN=1 cargo test -p worktrees-cli --locked --test remote_net_linux'
//! ```
#![cfg(target_os = "linux")]

mod remote_common;

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::os::fd::{AsRawFd, OwnedFd};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use remote_common::*;
use rustls::pki_types::{PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::WebSocket;

fn enabled() -> bool {
    let on = std::env::var_os("WT_NET_ADMIN").is_some_and(|v| v == "1");
    if !on {
        eprintln!("WT_NET_ADMIN is not 1: the scratch-interface witnesses did not run");
    }
    on
}

fn ip(args: &[&str]) {
    let o = Command::new("ip").args(args).output().expect("iproute2's `ip`");
    assert!(o.status.success(), "ip {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr));
}

/// An interface that is removed when the test ends, however it ends.
struct Link(&'static str, Option<OwnedFd>);

impl Drop for Link {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["link", "del", self.0]).output();
    }
}

fn dummy(name: &'static str) -> Link {
    let _ = Command::new("ip").args(["link", "del", name]).output();
    ip(&["link", "add", name, "type", "dummy"]);
    ip(&["link", "set", name, "up"]);
    Link(name, None)
}

/// A TUN — point-to-point, as `tailscale0` is — held open by this process,
/// without which it never reports RUNNING.
fn tun(name: &'static str) -> Link {
    let _ = Command::new("ip").args(["link", "del", name]).output();
    let f = std::fs::OpenOptions::new().read(true).write(true).open("/dev/net/tun").expect("/dev/net/tun (docker --device /dev/net/tun)");
    #[repr(C)]
    struct IfReq {
        name: [u8; 16],
        flags: libc::c_short,
        _pad: [u8; 22],
    }
    let mut req = IfReq { name: [0; 16], flags: (libc::IFF_TUN | libc::IFF_NO_PI) as libc::c_short, _pad: [0; 22] };
    req.name[..name.len()].copy_from_slice(name.as_bytes());
    // SAFETY: TUNSETIFF with an ifreq of the kernel's size, on our own fd.
    let r = unsafe { libc::ioctl(f.as_raw_fd(), 0x400454ca, &mut req) };
    assert_eq!(r, 0, "TUNSETIFF: {}", std::io::Error::last_os_error());
    ip(&["link", "set", name, "up"]);
    Link(name, Some(OwnedFd::from(f)))
}

/// TLS from source address `src` to `addr`: `Ok` when the server took the
/// connection as far as its TLS handshake (any certificate does: a key no
/// device holds still completes the client's half); `Err` when it was
/// dropped before a byte, as the peer rule drops it.
fn reaches_tls(src: IpAddr, addr: SocketAddr, pin: &str) -> Result<(), String> {
    let s = socket2::Socket::new(socket2::Domain::for_address(addr), socket2::Type::STREAM, None).map_err(|e| e.to_string())?;
    s.bind(&SocketAddr::new(src, 0).into()).map_err(|e| format!("bind {src}: {e}"))?;
    s.connect_timeout(&addr.into(), WAIT).map_err(|e| format!("connect: {e}"))?;
    let mut tcp: TcpStream = s.into();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    let dev = device();
    let cfg = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned(pin.to_string())))
        .with_client_auth_cert(vec![dev.cert.clone()], PrivateKeyDer::Pkcs8(dev.key.clone_key()))
        .unwrap();
    let mut conn = ClientConnection::new(Arc::new(cfg), ServerName::IpAddress(addr.ip().into())).unwrap();
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp).map_err(|e| format!("handshake: {e}"))?;
    }
    drop(StreamOwned::new(conn, tcp));
    Ok(())
}

fn connects(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok()
}

/// `remote_status` until `want` holds of it, polling past serve's 3 s address
/// watch; the last status either way.
fn status_until(con: &mut WebSocket<TcpStream>, want: impl Fn(&Value) -> bool) -> Value {
    let t0 = Instant::now();
    loop {
        let st = rpc(con, 900, "remote_status", json!({}))["ok"].clone();
        if want(&st) || t0.elapsed() > Duration::from_secs(12) {
            return st;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Step 2: a public address that IS bindable is refused as `not-private`,
/// never as a bind error. First a plain listener binds it and a connect
/// succeeds, which proves the address is live; then serve is asked for the
/// same address and port, refuses, and nothing answers there.
#[test]
fn a_bindable_public_address_is_refused_as_not_private_linux() {
    if !enabled() {
        return;
    }
    let _l = dummy("wtpub0");
    ip(&["addr", "add", "198.51.100.7/32", "dev", "wtpub0"]);
    let probe = std::net::TcpListener::bind("198.51.100.7:0").expect("the documentation address binds here");
    let at = probe.local_addr().unwrap();
    assert!(connects(at), "the address is not live, so the refusal below would prove nothing");
    drop(probe);
    let (_sb, sf) = Sandbox::launch("netpub");
    let mut con = console(&sf);
    let r = rpc(&mut con, 1, "remote_enable", json!({ "address": "198.51.100.7", "iface": "wtpub0", "port": at.port() }));
    let e = r["err"].as_str().unwrap_or_else(|| panic!("served: {r}"));
    assert!(e.starts_with("not-private"), "{e}");
    assert!(!connects(at), "something listens on {at}");
}

/// Step 4: the address moves (§4.1, §11.2 N4, §12.3 R1). With a paired
/// device connected on a LAN bind: the address leaves → the device's socket
/// closes, the port is gone, remote access is paused; the same address
/// returns → it resumes by itself; a NEW address in the same subnet → the
/// listener moves by itself, and the old address no longer answers; three
/// more moves inside the hour → the fourth pauses and asks; an address in
/// another subnet → paused until the host names it.
#[test]
fn the_listener_follows_its_address_as_4_1_says_linux() {
    if !enabled() {
        return;
    }
    let _l = dummy("wtlan0");
    // A lease that moves is the new address added, the old one removed. By
    // default Linux removes a subnet's secondary addresses WITH its primary,
    // so "add .6, remove .5" would leave nothing; promotion keeps the new one.
    let on = |p: &str| std::fs::read_to_string(p).is_ok_and(|v| v.trim() == "1");
    let _ = std::fs::write("/proc/sys/net/ipv4/conf/wtlan0/promote_secondaries", "1");
    assert!(
        on("/proc/sys/net/ipv4/conf/all/promote_secondaries") || on("/proc/sys/net/ipv4/conf/wtlan0/promote_secondaries"),
        "promote_secondaries is off and /proc/sys is read-only: run the container with --sysctl net.ipv4.conf.all.promote_secondaries=1"
    );
    ip(&["addr", "add", "10.77.0.5/24", "dev", "wtlan0"]);
    let (sb, sf) = Sandbox::launch("netmove");
    let mut con = console(&sf);
    let st = rpc(&mut con, 1, "remote_enable", json!({ "address": "10.77.0.5" }));
    let at: SocketAddr = st["ok"]["at"].as_str().unwrap_or_else(|| panic!("{st}")).parse().unwrap();
    let pin = st["ok"]["pin"].as_str().unwrap().to_string();
    let dev = device();
    pair(&mut con, &dev, &["read"]);
    let (ws, _) = session(at, &pin, &dev).expect("served first");
    // The address leaves.
    ip(&["addr", "del", "10.77.0.5/24", "dev", "wtlan0"]);
    let st = status_until(&mut con, |s| s["state"] == json!("paused"));
    assert_eq!(st["state"], json!("paused"), "{st}");
    assert!(st["paused"].as_str().unwrap_or_default().contains("no longer on wtlan0"), "{st}");
    // The address has left, so nothing can carry a close frame to the device
    // (as §4.1 says of the advert's goodbye): the proof is serve's own, that
    // it ended the device's session at the pause, not waiting for the OS.
    let log = std::fs::read_to_string(sb.root.join("home/.cache/worktrees/serve.log")).unwrap_or_default();
    assert!(log.contains("device connection(s) closed: remote access paused"), "the device's session outlived the pause");
    drop(ws);
    // The same address returns: resumed, on the same port.
    ip(&["addr", "add", "10.77.0.5/24", "dev", "wtlan0"]);
    let st = status_until(&mut con, |s| s["state"] == json!("listening"));
    assert_eq!(st["at"], json!(at.to_string()), "{st}");
    assert!(session(at, &pin, &dev).is_ok(), "not served after the resume");
    // A new address in the same subnet: moved by itself.
    let mut from = "10.77.0.5".to_string();
    for (n, to) in ["10.77.0.6", "10.77.0.7", "10.77.0.8"].into_iter().enumerate() {
        ip(&["addr", "add", &format!("{to}/24"), "dev", "wtlan0"]);
        ip(&["addr", "del", &format!("{from}/24"), "dev", "wtlan0"]);
        let moved: SocketAddr = format!("{to}:{}", at.port()).parse().unwrap();
        let st = status_until(&mut con, |s| s["at"] == json!(moved.to_string()));
        assert_eq!(st["at"], json!(moved.to_string()), "move {}: {st}", n + 1);
        assert!(session(moved, &pin, &dev).is_ok(), "not served at {moved}");
        assert!(!connects(format!("{from}:{}", at.port()).parse().unwrap()), "{from} still answers after the move");
        from = to.to_string();
    }
    // The fourth move inside the hour pauses and asks, with an alert.
    ip(&["addr", "add", "10.77.0.9/24", "dev", "wtlan0"]);
    ip(&["addr", "del", "10.77.0.8/24", "dev", "wtlan0"]);
    let st = status_until(&mut con, |s| s["question"].is_object());
    assert_eq!(st["state"], json!("paused"), "{st}");
    assert_eq!(st["question"]["alert"], json!(true), "{st}");
    assert!(st["question"]["text"].as_str().unwrap_or_default().contains("more than 3 times"), "{st}");
    assert!(!connects(format!("10.77.0.9:{}", at.port()).parse().unwrap()), "moved a fourth time by itself");
    // Another subnet: paused until the host names it.
    ip(&["addr", "del", "10.77.0.9/24", "dev", "wtlan0"]);
    ip(&["addr", "add", "10.88.0.5/24", "dev", "wtlan0"]);
    let st = status_until(&mut con, |s| s["question"]["use_address"] == json!("10.88.0.5"));
    assert_eq!(st["state"], json!("paused"), "{st}");
    assert!(!connects(format!("10.88.0.5:{}", at.port()).parse().unwrap()), "bound another subnet by itself");
    let st = rpc(&mut con, 2, "remote_enable", json!({ "address": "10.88.0.5" }));
    assert_eq!(st["ok"]["state"], json!("listening"), "the host's choice: {st}");
}

/// Step 5: the peer rule (§4.1 parts 2 and 3), with the out-of-row source on
/// the SAME interface as the bind — on another one, the kernel's own binding
/// would refuse it first and this would pass with the rule deleted (§12.4
/// T4). An overlay bind (a TUN, point-to-point) admits its own row and drops
/// an RFC 1918 source; a LAN bind admits on-link and drops a private source
/// off its subnet. Each served case first.
#[test]
fn the_peer_rule_holds_on_an_overlay_and_a_lan_bind_linux() {
    if !enabled() {
        return;
    }
    let (_sb, sf) = Sandbox::launch("netpeer");
    let mut con = console(&sf);
    // Overlay.
    let _t = tun("wtptp0");
    for a in ["100.64.7.1/32", "100.64.7.3/32", "10.66.0.9/32"] {
        ip(&["addr", "add", a, "dev", "wtptp0"]);
    }
    let st = rpc(&mut con, 1, "remote_enable", json!({ "address": "100.64.7.1", "iface": "wtptp0" }));
    assert_eq!(st["ok"]["mode"], json!("overlay"), "{st}");
    let at: SocketAddr = st["ok"]["at"].as_str().unwrap().parse().unwrap();
    let pin = st["ok"]["pin"].as_str().unwrap().to_string();
    reaches_tls("100.64.7.3".parse().unwrap(), at, &pin).expect("a peer in the overlay's own row");
    assert!(reaches_tls("10.66.0.9".parse().unwrap(), at, &pin).is_err(), "an RFC 1918 source reached an overlay bind's TLS");
    rpc(&mut con, 2, "remote_disable", json!({}));
    // LAN.
    let _l = dummy("wtlan1");
    for a in ["10.77.1.5/24", "10.77.1.9/24", "10.99.1.9/32"] {
        ip(&["addr", "add", a, "dev", "wtlan1"]);
    }
    let st = rpc(&mut con, 3, "remote_enable", json!({ "address": "10.77.1.5" }));
    assert_eq!(st["ok"]["mode"], json!("lan"), "{st}");
    let at: SocketAddr = st["ok"]["at"].as_str().unwrap().parse().unwrap();
    reaches_tls("10.77.1.9".parse().unwrap(), at, &pin).expect("an on-link peer");
    assert!(reaches_tls("10.99.1.9".parse().unwrap(), at, &pin).is_err(), "a private source off the subnet reached a LAN bind's TLS");
}
