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

/// Notifications through the real binary and a really paired device, with no
/// Apple anywhere: the `ok` flag; an Observe device registers; `not-configured`
/// with no key; the console's dry run, then the whole path to a sealed payload
/// and a line in the log, which names no token, key or name; a device cannot
/// turn the dry run on; the kill switch stops `push_prefs`'s connection.
#[test]
fn a_paired_device_registers_and_the_dry_run_seals_a_push() {
    let Some(lan) = lan_or_skip() else { return };
    let (sb, sf) = Sandbox::launch("push");
    let mut con = console(&sf);
    let st = rpc(&mut con, 2, "remote_enable", json!({ "address": lan.to_string() }));
    let at: SocketAddr = st["ok"]["at"].as_str().unwrap().parse().unwrap();
    let pin = st["ok"]["pin"].as_str().unwrap().to_string();
    let dev = device();
    pair(&mut con, &dev, &["read"]);
    let (mut ws, ok) = session(at, &pin, &dev).expect("a session");
    assert_eq!(ok["push"], json!(true), "the flag: {ok}");
    let prefs = rpc(&mut ws, 3, "push_prefs", json!({}));
    assert_eq!(prefs["ok"], json!({ "registered": false, "input": true, "reminder": true, "security": null, "muted": [] }), "{prefs}");
    assert_eq!(rpc(&mut ws, 4, "push_test", json!({}))["err"], json!("not-registered"));
    let token = "a1".repeat(32);
    let key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
    let key_id = "AQIDBAUGBwg=";
    let reg = rpc(&mut ws, 5, "push_register", json!({ "token": token, "env": "development", "topic": "com.brethash.worktrees.ios", "key": key, "key_id": key_id }));
    assert_eq!(reg["ok"], json!({ "registered": true }), "an Observe device registers: {reg}");
    assert!(rpc(&mut ws, 6, "push_test", json!({}))["err"].as_str().unwrap().starts_with("not-configured"));
    assert_eq!(rpc(&mut ws, 7, "push_dry_run", json!({ "on": true }))["err"], json!("console-only"));
    assert_eq!(rpc(&mut con, 8, "push_dry_run", json!({ "on": true }))["ok"]["dry_run"], json!(true));
    assert_eq!(rpc(&mut ws, 9, "push_test", json!({}))["ok"], json!({ "sent": true }));

    let home = sb.root.join("home");
    let log = std::fs::read_to_string(home.join(".cache/worktrees/serve.log")).unwrap();
    assert!(log.contains("dry run: kind=test") && log.contains("sealed=true"), "{log}");
    let audit = std::fs::read_to_string(sb.root.join("cfg/audit.jsonl")).unwrap();
    let devices = std::fs::read_to_string(sb.root.join("cfg/devices.json")).unwrap();
    for (what, text) in [("serve.log", &log), ("audit.jsonl", &audit), ("devices.json", &devices)] {
        assert!(!text.contains(&token) && !text.contains(key), "{what} holds a secret");
    }
    let held = std::fs::read_to_string(sb.root.join("cfg/push-devices.json")).unwrap();
    assert!(held.contains(&token), "the one file that holds it");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(sb.root.join("cfg/push-devices.json")).unwrap().permissions().mode() & 0o777, 0o600);

    // What the console may read, and the kill switch.
    let status = rpc(&mut con, 10, "push_status", json!({}));
    assert_eq!(status["ok"]["devices"][0]["registered"], json!(true), "{status}");
    assert!(!status.to_string().contains(&token));
    let off = rpc(&mut con, 11, "remote_disable", json!({}));
    assert_eq!(off["ok"]["state"], json!("off"), "{off}");
    assert!(closed_with(&mut ws, WAIT).is_some(), "the device's socket ended");
}

/// The whole path, in the real binary, with no Apple: a pane in a registered
/// project showing Claude's folder-trust prompt, which is the waiting edge the
/// amber dot draws; serve's own poll hears it; 30 real seconds later the sender
/// seals a needs-input push for a paired device and says so in the log. Real
/// time on purpose: every other test here runs the rule on a scripted clock,
/// and this is the one that sees the poll and the sender joined up.
#[test]
fn a_real_waiting_edge_becomes_a_sealed_push_in_the_dry_run() {
    let Some(lan) = lan_or_skip() else { return };
    let (sb, sf) = Sandbox::launch("push-edge");
    let git = |dir: &std::path::Path, a: &[&str]| {
        let ok = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(a).output().unwrap();
        assert!(ok.status.success(), "git {a:?}: {ok:?}");
    };
    let repo = sb.root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "x"]);
    let repo = std::fs::canonicalize(&repo).unwrap();

    let mut con = console(&sf);
    let added = rpc(&mut con, 1, "add_project", json!({ "dir": repo }));
    assert!(added.get("ok").is_some(), "{added}");
    let st = rpc(&mut con, 2, "remote_enable", json!({ "address": lan.to_string() }));
    let at: SocketAddr = st["ok"]["at"].as_str().unwrap().parse().unwrap();
    let pin = st["ok"]["pin"].as_str().unwrap().to_string();
    let dev = device();
    pair(&mut con, &dev, &["read"]);
    let (mut ws, _) = session(at, &pin, &dev).expect("a session");
    let reg = rpc(&mut ws, 3, "push_register", json!({ "token": "b2".repeat(32), "env": "production", "topic": "com.brethash.worktrees.ios", "key": "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=", "key_id": "AQIDBAUGBwg=" }));
    assert_eq!(reg["ok"], json!({ "registered": true }), "{reg}");
    assert_eq!(rpc(&mut con, 4, "push_dry_run", json!({ "on": true }))["ok"]["dry_run"], json!(true));

    // A pane running something called `claude` that shows the prompt. A copy of
    // bash: AGENTS.md's coreutils trap rules out copying `sleep`.
    let fake = sb.root.join("claude");
    assert!(std::process::Command::new("cp").arg("/bin/bash").arg(&fake).status().unwrap().success());
    let screen = sb.root.join("screen.txt");
    std::fs::write(&screen, include_str!("../../worktrees-core/tests/fixtures/claude-screen/2.1.309/trust.txt")).unwrap();
    let uid = unsafe { libc::getuid() };
    let sock = sb.root.join(format!("tmux/tmux-{uid}/default"));
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(sock.parent().unwrap(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let tmux = |a: &[&str]| std::process::Command::new("tmux").env_remove("TMUX").arg("-S").arg(&sock).args(a).output().unwrap();
    struct Down<'a>(&'a dyn Fn(&[&str]) -> std::process::Output);
    impl Drop for Down<'_> {
        fn drop(&mut self) {
            let _ = (self.0)(&["kill-server"]);
        }
    }
    let _down = Down(&tmux);
    let cmd = format!("{} -c 'cat {}; while :; do read -t 100 x; done'", fake.display(), screen.display());
    let made = tmux(&["new-session", "-d", "-x", "100", "-y", "40", "-s", "repo-main", "-c", repo.to_str().unwrap(), &cmd]);
    assert!(made.status.success(), "{made:?}");

    let log = sb.root.join("home/.cache/worktrees/serve.log");
    let deadline = Instant::now() + Duration::from_secs(75);
    let found = loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("dry run: kind=input") {
            break text;
        }
        assert!(Instant::now() < deadline, "no push within 75 s. pane: {:?}\nlog:\n{text}", String::from_utf8_lossy(&tmux(&["list-panes", "-a", "-F", "#{session_name} #{pane_current_command} #{pane_current_path}"]).stdout));
        std::thread::sleep(Duration::from_millis(500));
    };
    let line = found.lines().find(|l| l.contains("dry run: kind=input")).unwrap();
    assert!(line.contains("sealed=true"), "{line}");
    assert!(!found.contains(&"b2".repeat(32)), "the token is not in the log");
    assert!(!line.contains("repo"), "no name in the line: {line}");
}
