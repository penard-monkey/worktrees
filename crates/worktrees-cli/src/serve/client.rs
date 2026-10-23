//! `worktrees serve call | remote | pair` — console clients of the RUNNING
//! serve (Phase 2 §2.2, §4.1). Each reads `serve.json` from the config dir,
//! connects to `127.0.0.1:<port>` with the app's own `Origin` (§2.6: a native
//! local client sends it, as engine.rs's probe does), completes handshake v1
//! — the token is the HMAC key and never sent — and makes its requests. It
//! holds exactly the authority every loopback client already holds: whoever
//! can read the 0600 serve.json is the owner on the console.
//!
//! The shape is `serve_bin.rs::connect`'s, kept small on purpose; the remote
//! side's v1 client lives in `crates/worktrees-remote` (follow-up: unify).

use std::io::{BufRead, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocket};
use tokio_tungstenite::tungstenite::Message;
use worktrees_core::render::error_line;

use super::{policy, servefile};

pub const USAGE: &str = "\
usage: worktrees serve call <command> [json-args] [--config-dir DIR]
       worktrees serve remote on <address> [--iface IF] [--port N] [--advertise on|off] [--config-dir DIR]
       worktrees serve remote off | status | advertise on|off [--config-dir DIR]
       worktrees serve pair (--projects ROOT[,ROOT…] | --all-projects)
                            [--preset observe|operate|drive] [--caps CAP[,CAP…]] [--admin]
                            [--expiry 30d|never|<YYYY-MM-DDTHH:MM:SSZ>] [--config-dir DIR]

Console clients of the running `worktrees serve` (the one serve.json in the
config dir names). `remote on` turns remote access on, on one private address
of this host (§4.1); `remote off` is the kill switch: every device's
connection, the listener and any pairing window, at once. `remote advertise
off` keeps this host from announcing itself over mDNS on the LAN — set it
before `remote on` and nothing is ever announced; it is on unless you turn it
off (§4.7), and it is remembered.

`pair` opens a pairing window over remote access (turn it on first): it prints
the QR and the link (`worktrees remote pair '<link>'` on the other machine
takes it), then the six-digit code the device shows, and asks you to allow
it. The scope comes first: name the projects, or --all-projects. The preset
is Observe (read) unless you choose another; drive is read, write, answer,
instruct and stream, and a device holding it can run commands here as you.";

/// One console connection.
pub struct Console {
    ws: WebSocket<TcpStream>,
    next: u64,
}

impl Console {
    pub fn connect(config_dir: &Path) -> Result<Console, String> {
        let sf = servefile::read(config_dir).ok_or_else(|| {
            format!("no `worktrees serve` is running for {} (no serve.json there); start one with `worktrees serve`", config_dir.display())
        })?;
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, sf.port));
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| format!("serve on {addr}: {e}"))?;
        s.set_read_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
        let origin = policy::BUILTIN_ORIGINS[0];
        let req = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: {origin}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
            sf.port
        );
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match s.read(&mut b) {
                Ok(1) => head.push(b[0]),
                _ => return Err("serve closed the connection during the upgrade".into()),
            }
        }
        if !head.starts_with(b"HTTP/1.1 101 ") {
            return Err("serve refused the upgrade".into());
        }
        let mut ws = WebSocket::from_raw_socket(s, Role::Client, None);
        let cn = servefile::nonce();
        ws.send(Message::text(json!({ "hello": cn }).to_string())).map_err(|e| e.to_string())?;
        let sn = read_json(&mut ws)?["challenge"].as_str().ok_or("serve sent no challenge")?.to_string();
        let proof = policy::client_proof_for(&sf.token, &cn, &sn);
        ws.send(Message::text(json!({ "auth": proof }).to_string())).map_err(|e| e.to_string())?;
        let ok = read_json(&mut ws).map_err(|_| "serve refused the handshake (is serve.json current?)".to_string())?;
        if ok["ok"] != json!(true) || ok["proof"] != json!(policy::server_proof(&sf.token, &cn, &sn)) {
            return Err("whatever answers on serve's port did not prove it is serve".into());
        }
        if ok["version"] != json!(env!("CARGO_PKG_VERSION")) {
            return Err(format!("serve is v{}, this CLI is v{}", ok["version"].as_str().unwrap_or("?"), env!("CARGO_PKG_VERSION")));
        }
        Ok(Console { ws, next: 1 })
    }

    /// One request; events and channel frames on the way are skipped.
    pub fn call(&mut self, cmd: &str, args: Value) -> Result<Value, Value> {
        let id = self.next;
        self.next += 1;
        let req = json!({ "id": id, "cmd": cmd, "args": args }).to_string();
        self.ws.send(Message::text(req)).map_err(|e| Value::from(e.to_string()))?;
        loop {
            let v = read_json(&mut self.ws).map_err(Value::from)?;
            if v.get("id") != Some(&json!(id)) {
                continue;
            }
            if let Some(ok) = v.get("ok") {
                return Ok(ok.clone());
            }
            return Err(v.get("err").cloned().unwrap_or(Value::Null));
        }
    }
}

fn read_json(ws: &mut WebSocket<TcpStream>) -> Result<Value, String> {
    loop {
        match ws.read().map_err(|e| e.to_string())? {
            Message::Text(t) => return serde_json::from_str(t.as_str()).map_err(|e| e.to_string()),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) | Message::Binary(_) => continue,
            Message::Close(_) => return Err("serve closed the connection".into()),
        }
    }
}

fn err_text(e: &Value) -> String {
    e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string())
}

/// `--config-dir DIR` out of `args`, or the desktop app's own.
fn config_dir(args: &mut Vec<String>) -> Result<PathBuf, String> {
    match args.iter().position(|a| a == "--config-dir") {
        Some(i) => {
            let v = args.get(i + 1).cloned().ok_or("--config-dir needs a directory")?;
            args.drain(i..=i + 1);
            Ok(PathBuf::from(v))
        }
        None => Ok(worktrees_api::hostenv::default_config_dir()),
    }
}

/// The value after `flag`, taken out of `args`.
fn take(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    match args.iter().position(|a| a == flag) {
        Some(i) => {
            let v = args.get(i + 1).cloned().ok_or_else(|| format!("{flag} needs a value"))?;
            args.drain(i..=i + 1);
            Ok(Some(v))
        }
        None => Ok(None),
    }
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    match args.iter().position(|a| a == flag) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

fn fail(msg: &str) -> i32 {
    eprintln!("{}", error_line(msg));
    1
}

/// `worktrees serve call|remote|pair …`; `None` when `args` is none of them.
pub fn dispatch(args: &[String]) -> Option<i32> {
    let (sub, rest) = args.split_first()?;
    let mut rest = rest.to_vec();
    if rest.iter().any(|a| a == "-h" || a == "--help") && matches!(sub.as_str(), "call" | "remote" | "pair") {
        println!("{USAGE}");
        return Some(0);
    }
    let run = match sub.as_str() {
        "call" => call(&mut rest),
        "remote" => remote(&mut rest),
        "pair" => pair(&mut rest),
        _ => return None,
    };
    Some(run.unwrap_or_else(|e| {
        let code = fail(&e);
        if e.starts_with("usage") {
            eprintln!("{USAGE}");
        }
        code
    }))
}

fn call(args: &mut Vec<String>) -> Result<i32, String> {
    let dir = config_dir(args)?;
    let (cmd, json_args) = match args.as_slice() {
        [cmd] => (cmd.clone(), json!({})),
        [cmd, a] => (cmd.clone(), serde_json::from_str::<Value>(a).map_err(|e| format!("json-args: {e}"))?),
        _ => return Err("usage: worktrees serve call <command> [json-args]".into()),
    };
    let mut c = Console::connect(&dir)?;
    match c.call(&cmd, json_args) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            Ok(0)
        }
        Err(e) => Ok(fail(&err_text(&e))),
    }
}

fn remote(args: &mut Vec<String>) -> Result<i32, String> {
    let dir = config_dir(args)?;
    let iface = take(args, "--iface")?;
    let port = take(args, "--port")?.map(|p| p.parse::<u16>().map_err(|_| format!("--port {p:?}: not a port number"))).transpose()?;
    let advertise = match take(args, "--advertise")?.as_deref() {
        None => None,
        Some("on") => Some(true),
        Some("off") => Some(false),
        Some(other) => return Err(format!("--advertise {other:?}: on or off")),
    };
    let (cmd, a) = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["on", addr] => ("remote_enable", json!({ "address": addr, "iface": iface, "port": port, "advertise": advertise })),
        ["off"] => ("remote_disable", json!({})),
        ["status"] | [] => ("remote_status", json!({})),
        ["advertise", "on"] => ("remote_advertise", json!({ "on": true })),
        ["advertise", "off"] => ("remote_advertise", json!({ "on": false })),
        _ => return Err("usage: worktrees serve remote on <address> | off | status | advertise on|off".into()),
    };
    let mut c = Console::connect(&dir)?;
    match c.call(cmd, a) {
        Ok(st) => {
            print_status(&st);
            Ok(0)
        }
        Err(e) => Ok(fail(&err_text(&e))),
    }
}

/// `remote_status`, for a person. An `owner` of "app" means the desktop app
/// started this serve, and quitting it stops serve — and remote access with
/// it (§9.3).
fn print_status(st: &Value) {
    let owned = st["owner"] == json!("app");
    let s = |k: &str| st[k].as_str().unwrap_or_default().to_string();
    match st["state"].as_str() {
        Some("off") => {
            println!("remote access: off");
            if st["advertise"] == json!(false) {
                println!("  advertising: off — this host will not announce itself on the LAN when remote access is on");
            }
        }
        Some("unreadable") => println!("remote access: off — {} (`worktrees serve remote off` removes it)", s("error")),
        Some("listening") => {
            println!("remote access: on, listening on {} ({}, {} mode)", s("at"), s("iface"), s("mode"));
            if let Some(pin) = st["pin"].as_str() {
                println!("  pin:          {pin}");
            }
            if let Some(code) = st["address_code"].as_str() {
                println!("  address code: {code}");
            }
            match (st["advertise"].as_bool(), st["advert"]["state"].as_str()) {
                (Some(false), _) => println!("  advertised:   no — switched off (`worktrees serve remote advertise on`)"),
                (_, Some("registered")) => println!("  advertised:   yes, on the LAN as {}", st["advert"]["host"].as_str().unwrap_or("?")),
                _ => println!("  advertised:   no — {}", st["advert"]["why"].as_str().unwrap_or("not registered")),
            }
            if owned {
                println!("  quitting Worktrees turns remote access off (the app started this serve)");
            }
            println!("  `worktrees serve remote off` turns it off now, closing every device's connection");
        }
        Some(other) => {
            println!("remote access: on, {other} — {}", st["paused"].as_str().or(st["error"].as_str()).unwrap_or("waiting for its address"));
        }
        None => println!("{st}"),
    }
    if let Some(q) = st["question"].as_object() {
        println!("  {}", q.get("text").and_then(Value::as_str).unwrap_or_default());
        if let Some(a) = q.get("use_address").and_then(Value::as_str).filter(|a| !a.is_empty()) {
            println!("  use this address: `worktrees serve remote on {a}`; keep it off: `worktrees serve remote off`");
        }
    }
    if let Some(w) = st["warning"].as_str() {
        println!("  note: {w}");
    }
}

/// The capability sets the pairing dialog offers (§3.1).
fn preset(name: &str) -> Result<&'static [&'static str], String> {
    match name {
        "observe" => Ok(&["read"]),
        "operate" => Ok(&["read", "write"]),
        "drive" => Ok(&["read", "write", "answer", "instruct", "stream"]),
        other => Err(format!("--preset {other:?}: observe, operate or drive")),
    }
}

/// `30d`, `never`, or an RFC 3339 time, as `pair_begin` takes an expiry.
fn expiry(s: &str) -> Result<Value, String> {
    if s == "never" {
        return Ok(json!("never"));
    }
    if let Some(n) = s.strip_suffix('d') {
        let n: u32 = n.parse().map_err(|_| format!("--expiry {s:?}: Nd, never, or YYYY-MM-DDTHH:MM:SSZ"))?;
        return Ok(json!({ "idle_days": n }));
    }
    Ok(json!({ "until": s }))
}

/// Interrupted by ^C: the window is cancelled on the way out.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

fn pair(args: &mut Vec<String>) -> Result<i32, String> {
    let dir = config_dir(args)?;
    let preset_name = take(args, "--preset")?;
    let caps_arg = take(args, "--caps")?;
    let projects_arg = take(args, "--projects")?;
    let all = take_flag(args, "--all-projects");
    let admin = take_flag(args, "--admin");
    let expiry_arg = take(args, "--expiry")?;
    if !args.is_empty() {
        return Err(format!("usage: unexpected {:?}", args[0]));
    }
    let mut caps: Vec<String> = match (&caps_arg, &preset_name) {
        (Some(_), Some(_)) => return Err("--caps or --preset, not both".into()),
        (Some(c), None) => c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        (None, p) => preset(p.as_deref().unwrap_or("observe"))?.iter().map(|s| s.to_string()).collect(),
    };
    if admin && !caps.iter().any(|c| c == "admin") {
        caps.push("admin".into());
    }
    // §2.2 step 1: a scope is always chosen, never inherited.
    let projects = match (projects_arg, all) {
        (Some(_), true) => return Err("--projects or --all-projects, not both".into()),
        (None, false) => return Err("usage: name the projects this device may reach (--projects ROOT,…) or --all-projects".into()),
        (None, true) => json!("*"),
        (Some(p), false) => {
            let roots: Result<Vec<String>, String> = p
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|r| std::fs::canonicalize(r.trim()).map(|c| c.to_string_lossy().into_owned()).map_err(|e| format!("{r}: {e}")))
                .collect();
            json!(roots?)
        }
    };
    let expiry = expiry_arg.as_deref().map(expiry).transpose()?.unwrap_or(Value::Null);
    let mut c = Console::connect(&dir)?;
    let st = c.call("remote_status", json!({})).map_err(|e| err_text(&e))?;
    if st["state"] != json!("listening") {
        return Err(format!(
            "remote access is {}: a device pairs over it, so turn it on first (`worktrees serve remote on <address>`)",
            st["state"].as_str().unwrap_or("off")
        ));
    }
    let w = c.call("pair_begin", json!({ "caps": caps, "projects": projects, "expiry": expiry })).map_err(|e| err_text(&e))?;
    let uri = w["uri"].as_str().ok_or("pair_begin answered no link")?.to_string();
    // SAFETY: a handler that only stores to an atomic.
    unsafe { libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t) };
    print_qr(&uri);
    println!("Pairing link (paste it on a device with no camera: `worktrees remote pair '<link>'`):\n\n  {uri}\n");
    let label = if caps.iter().any(|c| matches!(c.as_str(), "stream" | "instruct" | "answer")) {
        format!(" — can run commands on {} as you", w["host"].as_str().unwrap_or("this host"))
    } else {
        String::new()
    };
    println!("Scope: {} on {}{label}", caps.join(", "), if projects == json!("*") { "every project".to_string() } else { projects.to_string() });
    println!("The window closes in {} s, or after one attempt. ^C cancels it.\n", w["expires_at"].as_i64().map_or(120, |e| e - worktrees_core::sysclock::now_epoch()));
    let mut asked = false;
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            let _ = c.call("pair_cancel", json!({}));
            println!("Cancelled.");
            return Ok(130);
        }
        let s = c.call("pair_status", json!({})).map_err(|e| err_text(&e))?;
        match s["state"].as_str() {
            Some("pending") if !asked => {
                asked = true;
                let r = &s["request"];
                let code = r["code"].as_str().unwrap_or_default();
                let code = if code.len() == 6 { format!("{} {}", &code[..3], &code[3..]) } else { code.to_string() };
                println!(
                    "{} ({}, from {}) wants to pair · code {code}\nAllow only if this code is on your device right now.",
                    r["name"].as_str().unwrap_or("?"),
                    r["platform"].as_str().unwrap_or("?"),
                    r["peer"].as_str().unwrap_or("?"),
                );
                let by = s["decide_by"].as_i64().unwrap_or(0);
                let allow = ask_yes(&format!("Allow? [y/N] "), by);
                match c.call("pair_decide", json!({ "allow": allow })) {
                    Ok(Value::Null) => println!("Denied."),
                    Ok(rec) => println!("Paired: {} ({})", rec["name"].as_str().unwrap_or("?"), rec["id"].as_str().unwrap_or("?")),
                    Err(e) => println!("{}", err_text(&e)),
                }
            }
            Some("closed") => {
                let ended = s["ended"].as_str().unwrap_or("closed");
                if ended == "paired" {
                    return Ok(0);
                }
                println!("The window closed: {ended}.");
                return Ok(1);
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// A yes from the terminal before `by` (unix seconds), or no.
fn ask_yes(prompt: &str, by: i64) -> bool {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        let _ = tx.send(line);
    });
    let left = (by - worktrees_core::sysclock::now_epoch()).max(1) as u64;
    let deadline = Instant::now() + Duration::from_secs(left);
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return false;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(l) => return matches!(l.trim(), "y" | "Y" | "yes" | "Yes"),
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => continue,
            Err(_) => {
                println!("\nNo answer in time: the request runs out as a Deny.");
                return false;
            }
        }
    }
}

/// The link as a QR, two rows of modules per line of half blocks, for a
/// phone's camera. Dark modules are printed as the terminal's foreground, so
/// on a dark terminal the code is light on dark; phone cameras read either.
fn print_qr(uri: &str) {
    match qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M) {
        Ok(q) => {
            let art = q.render::<qrcode::render::unicode::Dense1x2>().quiet_zone(true).build();
            println!("{art}\n");
        }
        Err(e) => println!("(no QR: {e})\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_read_as_pair_begin_takes_them() {
        assert_eq!(expiry("30d").unwrap(), json!({ "idle_days": 30 }));
        assert_eq!(expiry("never").unwrap(), json!("never"));
        assert_eq!(expiry("2027-01-01T00:00:00Z").unwrap(), json!({ "until": "2027-01-01T00:00:00Z" }));
        assert!(expiry("xd").is_err());
        assert_eq!(preset("observe").unwrap(), &["read"]);
        assert_eq!(preset("drive").unwrap(), &["read", "write", "answer", "instruct", "stream"]);
        assert!(preset("god").is_err());
        let mut a: Vec<String> = ["x", "--config-dir", "/d", "--iface", "en0"].iter().map(|s| s.to_string()).collect();
        assert_eq!(config_dir(&mut a).unwrap(), PathBuf::from("/d"));
        assert_eq!(take(&mut a, "--iface").unwrap().as_deref(), Some("en0"));
        assert_eq!(a, vec!["x".to_string()]);
    }

    /// A link the window gives is one a QR can carry at level M.
    #[test]
    fn a_pairing_link_fits_a_qr() {
        let uri = format!("worktrees-pair:v1?addr=[fd00:1234:5678:9abc::1]:65535&pin={}&secret={}&host=Brets-MacBook-Pro&exp=1800000000", "a".repeat(64), "b".repeat(26));
        assert!(qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M).is_ok());
    }
}
