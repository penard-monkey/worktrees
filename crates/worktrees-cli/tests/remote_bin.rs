//! The remote listener, against the real binary (Server Edition Phase 2
//! §4.6): IDENTITY, not just reachability. A serve in a sandbox is told to
//! turn remote access on at this machine's own LAN address; a device made
//! here pairs through the real pairing port (the link, the proof bound to
//! the TLS session, the host's Allow) and is served over pinned mutual TLS
//! and handshake v1 keyed by the session; and everything else is refused —
//! no certificate, a key no device holds, the wrong pin, loopback, a closed
//! pairing port, a revoked device, the kill switch.
//!
//! The device side is written here from the spec, not from serve's code, so
//! a server that drifted from the spec fails rather than agreeing with itself.
//! Same-host connects reach a listener tied to its interface on both
//! platforms (measured 2026-10-22: macOS 27, Linux 6.12), so this needs no
//! second machine. Every claim is a CONNECT that follows one that must
//! succeed; on a CI runner, which always has a LAN address, a missing one
//! fails instead of skipping.

mod remote_common;

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use remote_common::*;
use serde_json::json;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

// ── the witnesses ────────────────────────────────────────────────────────────

/// §4.6's identity witness, steps 1–5 and 8, and the bind witness's step 3:
/// the console served FIRST; a device paired through the real pairing port
/// and SERVED over the remote listener, its `ok` naming it; then no
/// certificate, a key no device holds and the wrong pin each refused at TLS;
/// the remote port not on loopback; the pairing port closed with no window.
#[test]
fn remote_access_serves_a_paired_key_and_refuses_every_other() {
    let Some(lan) = lan_or_skip() else { return };
    let (sb, sf) = Sandbox::launch("identity");
    // 1. The console, first: a dead server fails here, not at a refusal.
    let mut con = console(&sf);
    assert!(rpc(&mut con, 1, "list_workspace", json!({})).get("ok").is_some());
    let st = rpc(&mut con, 2, "remote_enable", json!({ "address": lan.to_string() }));
    assert_eq!(st["ok"]["state"], json!("listening"), "{st}");
    let at: SocketAddr = st["ok"]["at"].as_str().unwrap().parse().unwrap();
    let pin = st["ok"]["pin"].as_str().unwrap().to_string();
    assert_eq!(at.ip(), IpAddr::V4(lan));
    // 2. A paired device is served.
    let dev = device();
    let (paired, pairing_at) = pair(&mut con, &dev, &["read"]);
    let (mut ws, ok) = session(at, &pin, &dev).expect("the paired device's session");
    assert_eq!(ok["device"]["id"], paired["device_id"], "{ok}");
    assert_eq!(ok["version"], json!(env!("CARGO_PKG_VERSION")));
    assert!(rpc(&mut ws, 3, "list_workspace", json!({})).get("ok").is_some());
    assert_eq!(rpc(&mut ws, 4, "remote_status", json!({}))["err"], json!("console-only"));
    // 3. No client certificate: refused at TLS.
    assert!(refused_at_tls(tls(at, &pin, None)), "served without a certificate");
    // 4. A valid self-signed key that is not enrolled: refused at TLS.
    assert!(refused_at_tls(tls(at, &pin, Some(&device()))), "served a key no device holds");
    // 5. The wrong pin: the CLIENT refuses (a verifier that accepts anything
    // would let this through).
    let wrong = hex(&[0u8; 32]);
    assert!(tls(at, &wrong, Some(&dev)).is_err(), "the client accepted a server that is not the pin");
    // Bind witness 3: the listener is not on loopback (a wildcard bind would
    // answer both).
    assert!(TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, at.port())), WAIT).is_err(), "the remote port answers on loopback");
    assert_eq!(paired["remote_addr"], json!(at.to_string()), "the pairing answer names the remote listener, zone-less");
    // 8. With no window open, the pairing port is gone; a window that gets
    // one bad proof closes, and its port with it.
    assert!(TcpStream::connect_timeout(&pairing_at, WAIT).is_err(), "the pairing port outlived its window");
    let w = rpc(&mut con, 5, "pair_begin", json!({ "caps": ["read"], "projects": "*", "expiry": null }));
    let link = worktrees_core::pairuri::parse(w["ok"]["uri"].as_str().unwrap(), 0, &worktrees_core::pairuri::dial_rule).unwrap();
    let mut s = tls(link.addr, &link.pin, Some(&device())).unwrap();
    let bad = json!({ "pair": { "name": "racer", "platform": "linux", "stepup_spki": null }, "proof": hex(&[0u8; 32]) });
    s.write_all(format!("{bad}\n").as_bytes()).unwrap();
    let mut got = String::new();
    let _ = BufReader::new(s).read_line(&mut got);
    assert_eq!(got, "", "a bad proof was answered: {got}");
    let t0 = Instant::now();
    let st = loop {
        let st = rpc(&mut con, 6, "pair_status", json!({}));
        if st["ok"]["state"] == json!("closed") || t0.elapsed() > WAIT {
            break st;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(st["ok"]["ended"], json!("bad_proof"), "{st}");
    assert!(TcpStream::connect_timeout(&link.addr, WAIT).is_err(), "the window's port outlived a bad proof");
    drop(sb);
}

/// §4.6 steps 7 and 10: a revoke and the kill switch each close a paired
/// device's live socket within a second, with 1008 and a reason, and what
/// follows is refused; turning remote access on again serves the device on
/// the SAME address (a kill switch must not cost every device its port).
#[test]
fn the_kill_switch_and_a_revoke_end_a_live_device_at_once() {
    let Some(lan) = lan_or_skip() else { return };
    let (sb, sf) = Sandbox::launch("kill");
    let mut con = console(&sf);
    assert!(rpc(&mut con, 1, "list_workspace", json!({})).get("ok").is_some(), "the console first");
    let st = rpc(&mut con, 2, "remote_enable", json!({ "address": lan.to_string() }));
    let at: SocketAddr = st["ok"]["at"].as_str().unwrap_or_else(|| panic!("{st}")).parse().unwrap();
    let pin = st["ok"]["pin"].as_str().unwrap().to_string();
    let dev = device();
    let (paired, _) = pair(&mut con, &dev, &["read"]);
    // 10. The kill switch.
    let (mut ws, _) = session(at, &pin, &dev).expect("served first");
    let t0 = Instant::now();
    let off = rpc(&mut con, 3, "remote_disable", json!({}));
    assert_eq!(off["ok"]["state"], json!("off"), "{off}");
    let (code, why) = closed_with(&mut ws, Duration::from_secs(1)).expect("closed within a second, with a frame");
    assert!(t0.elapsed() < Duration::from_secs(1));
    assert_eq!(code, CloseCode::Policy);
    assert!(why.contains("turned off"), "{why}");
    assert!(TcpStream::connect_timeout(&at, WAIT).is_err(), "the remote port still answers");
    let w = rpc(&mut con, 4, "pair_begin", json!({ "caps": ["read"], "projects": "*", "expiry": null }));
    assert!(w.get("err").is_some(), "a pairing window opened with remote access off: {w}");
    // On again: the same address, and the device is served.
    let st = rpc(&mut con, 5, "remote_enable", json!({ "address": lan.to_string() }));
    assert_eq!(st["ok"]["at"], json!(at.to_string()), "{st}");
    let (mut ws, _) = session(at, &pin, &dev).expect("served again after the kill switch");
    // 7. The revoke.
    let t0 = Instant::now();
    let r = rpc(&mut con, 6, "device_revoke", json!({ "id": paired["device_id"] }));
    assert_eq!(r["ok"], json!(true), "{r}");
    let (code, why) = closed_with(&mut ws, Duration::from_secs(1)).expect("closed within a second, with a frame");
    assert!(t0.elapsed() < Duration::from_secs(1));
    assert_eq!(code, CloseCode::Policy);
    assert_eq!(why, "revoked");
    assert!(refused_at_tls(tls(at, &pin, Some(&dev))), "the revoked key is still served");
    drop(sb);
}
