//! `worktrees serve`, the binary, driven as a client would drive it.
//!
//! The in-process tests (`src/serve/tests.rs`) cover the socket; this covers
//! the PROCESS: the subcommand parses, writes `serve.json` where and how it
//! says, runs the host wiring, answers `list_workspace`, stamps its version on
//! the log's first line, and takes the file with it on SIGTERM. Hermetic by
//! environment: `HOME` and `XDG_CONFIG_HOME` point into the sandbox, so the
//! registry it lists is empty and nothing of the real machine is read or
//! written. The same position `tests/headless.rs` takes for the api crate —
//! only the public seam, which for a binary is its command line.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocket};
use tokio_tungstenite::tungstenite::Message;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_worktrees")
}

struct Sandbox {
    root: PathBuf,
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
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("wt-serve-bin-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["home", "xdg", "cfg"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        Sandbox { root, child: None }
    }
    fn cfg(&self) -> PathBuf {
        self.root.join("cfg")
    }
    fn serve_json(&self) -> PathBuf {
        self.cfg().join("serve.json")
    }
    fn log(&self) -> PathBuf {
        self.root.join("home/.cache/worktrees/serve.log")
    }

    /// Launch `serve` and wait for its serve.json. `SHELL=/bin/sh` keeps the
    /// login-shell PATH probe in `host::boot` quick; everything else of the
    /// environment is inherited, PATH included, so `git` still resolves.
    fn launch(&mut self, extra: &[&str]) -> Value {
        self.launch_with(extra, None)
    }

    /// `launch`, with variables of the test's own added to serve's environment.
    fn launch_env(&mut self, extra: &[&str], env: &[(&str, &str)]) -> Value {
        self.launch_full(extra, None, env)
    }

    /// `launch`, optionally with the child's SOFT open-file limit lowered
    /// first (the hard limit left as it is, the way launchd leaves it).
    fn launch_with(&mut self, extra: &[&str], soft_nofile: Option<libc::rlim_t>) -> Value {
        self.launch_full(extra, soft_nofile, &[])
    }

    fn launch_full(&mut self, extra: &[&str], soft_nofile: Option<libc::rlim_t>, env: &[(&str, &str)]) -> Value {
        let mut cmd = Command::new(bin());
        cmd.envs(env.iter().copied());
        if let Some(n) = soft_nofile {
            use std::os::unix::process::CommandExt;
            // SAFETY: getrlimit/setrlimit are async-signal-safe, and the
            // closure touches nothing else between fork and exec.
            unsafe {
                cmd.pre_exec(move || {
                    let mut rl = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
                    if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    rl.rlim_cur = n;
                    if libc::setrlimit(libc::RLIMIT_NOFILE, &rl) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let child = cmd
            .arg("serve")
            .arg("--config-dir")
            .arg(self.cfg())
            .args(extra)
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .env("SHELL", "/bin/sh")
            // Host policy's env tier would outrank a test's file; hermetic.
            .env_remove("WORKTREES_CODEX_PERMISSIONS")
            .env_remove("WORKTREES_PI_PROJECT_TRUST")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn worktrees serve");
        self.child = Some(child);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(b) = std::fs::read(self.serve_json()) {
                if let Ok(v) = serde_json::from_slice::<Value>(&b) {
                    if v["pid"].as_u64() == Some(self.pid() as u64) {
                        return v;
                    }
                }
            }
            if let Some(st) = self.child.as_mut().unwrap().try_wait().unwrap() {
                let mut err = String::new();
                let _ = self.child.as_mut().unwrap().stderr.take().unwrap().read_to_string(&mut err);
                panic!("serve exited early ({st}): {err}");
            }
            assert!(Instant::now() < deadline, "serve.json never appeared at {}", self.serve_json().display());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// SIGTERM, then the exit status, within a bound.
    fn terminate(&mut self) -> std::process::ExitStatus {
        let pid = self.pid() as libc::pid_t;
        // SAFETY: our own child, by the pid `spawn` returned.
        assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
        let child = self.child.as_mut().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(st) = child.try_wait().unwrap() {
                return st;
            }
            assert!(Instant::now() < deadline, "serve did not exit on SIGTERM");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Connect, upgrade, authenticate. The handshake is written by hand so the
/// test states what it sends; the frames are tungstenite's sync codec.
fn connect(sf: &Value, origin: &str) -> WebSocket<TcpStream> {
    let port = sf["port"].as_u64().unwrap() as u16;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(
        format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .as_bytes(),
    )
    .unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = s.read(&mut b).unwrap();
        assert!(n > 0, "closed during the handshake: {:?}", String::from_utf8_lossy(&head));
        head.push(b[0]);
    }
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101 "), "{}", String::from_utf8_lossy(&head));
    let mut ws = WebSocket::from_raw_socket(s, Role::Client, None);
    // Handshake v1, from the spec: the token is the HMAC key and never sent.
    let token = sf["token"].as_str().unwrap();
    let cn = fresh_hex32();
    ws.send(Message::text(json!({ "hello": cn }).to_string())).unwrap();
    let challenge: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
    let sn = challenge["challenge"].as_str().unwrap_or_else(|| panic!("no challenge: {challenge}")).to_string();
    let proof = |label: &str| spec_hmac(token.as_bytes(), format!("{label}{cn}\n{sn}").as_bytes());
    ws.send(Message::text(json!({ "auth": proof("worktrees-serve/v1/client\n") }).to_string())).unwrap();
    let reply: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert_eq!(reply["proof"], json!(proof("worktrees-serve/v1/server\n")), "the server's proof: {reply}");
    assert_eq!(reply["version"], json!(env!("CARGO_PKG_VERSION")));
    ws
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn fresh_hex32() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).unwrap();
    hex(&b)
}

/// HMAC-SHA256 by RFC 2104 over bare SHA-256 — the same independence from
/// the server's code as the in-process tests (keys here are 64 bytes, so
/// never longer than the block).
fn spec_hmac(key: &[u8], msg: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    assert!(key.len() <= 64);
    let mut k = [0u8; 64];
    k[..key.len()].copy_from_slice(key);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new().chain_update(&ipad).chain_update(msg).finalize();
    hex(&Sha256::new().chain_update(&opad).chain_update(&inner[..]).finalize()[..])
}

fn rpc(ws: &mut WebSocket<TcpStream>, id: u64, cmd: &str, args: Value) -> Value {
    ws.send(Message::text(json!({ "id": id, "cmd": cmd, "args": args }).to_string())).unwrap();
    // Events may arrive between replies (the poll emits `places:changed`
    // within seconds of starting), and so may the frames of a terminal this
    // test attached; skip both for the reply with our id.
    loop {
        let v: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        if v["id"] == json!(id) {
            return v;
        }
        assert!(v.get("event").is_some() || v.get("channel").is_some(), "an unexpected frame: {v}");
    }
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

/// OWASP M1: a launchd-started process begins at a SOFT limit of 256 open
/// files on macOS (hard unlimited), and 300 idle sockets were enough to deny
/// the user's own client. serve raises its soft limit at boot and says so.
#[test]
fn serve_raises_a_low_soft_open_file_limit_at_boot() {
    let mut sb = Sandbox::new("nofile");
    sb.launch_with(&[], Some(256));
    let log = std::fs::read_to_string(sb.log()).expect("serve.log");
    let line = log.lines().find(|l| l.contains("serve: open-file limit")).unwrap_or_else(|| panic!("no open-file limit line in:\n{log}"));
    let tail = line.split("serve: open-file limit ").nth(1).unwrap();
    let (old, new) = tail.split_once(" → ").unwrap_or_else(|| panic!("{line}"));
    assert_eq!(old, "256", "{line}");
    let new: u64 = new.trim().parse().unwrap_or_else(|_| panic!("{line}"));
    assert!(new > 256, "{line}");
    assert_eq!(sb.terminate().code(), Some(0));
}

/// Host policy is the ENGINE's (`worktrees_api::hostpolicy`: "only the engine
/// reads it; `boot` loads it at startup"), and `serve` is the engine. A host
/// that chose `ask` for Codex in Settings and then restarted serve must launch
/// lanes under `ask`, not under the default the file is there to override
/// (`auto-review`).
#[test]
fn serve_loads_host_policy_at_boot() {
    let mut sb = Sandbox::new("hostpolicy");
    std::fs::write(sb.cfg().join("host-policy.json"), r#"{"codex_permissions": "ask"}"#).unwrap();
    let sf = sb.launch(&["--allow-origin", "http://localhost:1420"]);
    let mut ws = connect(&sf, "http://localhost:1420");
    let r = rpc(&mut ws, 1, "get_codex_permissions", json!({}));
    assert_eq!(r["ok"], json!("ask"), "host-policy.json was not loaded at boot: {r}");
    assert_eq!(sb.terminate().code(), Some(0));
}

/// OWASP L1, through the binary: `log_event` is an Engine command, so any
/// authenticated client reaches it, and the review used it to write a whole
/// forged `serve: host-admin` line, timestamp and all, into serve.log. It
/// must land as ONE `ui:` line, with no byte in the file a terminal or a
/// line reader would act on.
#[test]
fn a_client_cannot_write_a_line_of_its_own_into_serve_log() {
    let mut sb = Sandbox::new("forge");
    let sf = sb.launch(&["--allow-origin", "http://localhost:1420"]);
    let mut ws = connect(&sf, "http://localhost:1420");
    let forged = "boom\n2026-01-01T00:00:00Z [info] [1] serve: host-admin forged_entry\r\x1b[2K\0tail";
    let r = rpc(&mut ws, 1, "log_event", json!({ "level": "info", "msg": forged }));
    assert_eq!(r["ok"], Value::Null, "{r}");
    let log = std::fs::read(sb.log()).expect("serve.log");
    let bad: Vec<u8> = log.iter().copied().filter(|b| (*b < 0x20 && *b != b'\n') || *b == 0x7f).collect();
    assert!(bad.is_empty(), "control bytes {bad:?} in:\n{}", String::from_utf8_lossy(&log));
    let log = String::from_utf8(log).unwrap();
    let hits: Vec<&str> = log.lines().filter(|l| l.contains("forged_entry")).collect();
    assert_eq!(hits.len(), 1, "{log}");
    assert!(hits[0].contains("] ui: boom") && hits[0].ends_with("tail"), "{}", hits[0]);
    assert!(!log.lines().any(|l| l.starts_with("2026-01-01")), "{log}");
    assert_eq!(sb.terminate().code(), Some(0));
}

/// OWASP L1, through the binary: serve.log is owner-only like serve.json,
/// whether this launch made it or found one a wider-moded writer left.
#[test]
fn serve_log_is_owner_only_new_or_inherited() {
    let mut sb = Sandbox::new("logmode");
    sb.launch(&[]);
    assert_eq!(mode(&sb.log()), 0o600, "a new serve.log is {:o}", mode(&sb.log()));
    assert_eq!(sb.terminate().code(), Some(0));

    std::fs::set_permissions(sb.log(), std::fs::Permissions::from_mode(0o644)).unwrap();
    sb.launch(&[]);
    assert_eq!(mode(&sb.log()), 0o600, "an inherited 0644 serve.log is {:o}", mode(&sb.log()));
    assert_eq!(sb.terminate().code(), Some(0));
}

/// A private tmux server for one test, on a socket of its own that serve is
/// pointed at through `TMUX` — which is where a tmux client looks when it is
/// given no `-L`/`-S`, so every tmux call serve makes lands here and none on
/// the server of whoever runs the tests. This side passes `-S` on every call,
/// `kill-server` included. In the temp dir itself: a unix socket path has a
/// ~104-byte limit on macOS.
struct PrivateTmux {
    sock: PathBuf,
}

impl PrivateTmux {
    fn new(tag: &str) -> Self {
        PrivateTmux { sock: std::env::temp_dir().join(format!("wtsb-{tag}-{}.sock", std::process::id())) }
    }
    /// The value serve's `TMUX` gets: socket, server pid, session index —
    /// tmux reads only the first field.
    fn env(&self) -> String {
        format!("{},0,0", self.sock.display())
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new("tmux").arg("-S").arg(&self.sock).args(args).env_remove("TMUX").output().expect("tmux")
    }
    fn available() -> bool {
        Command::new("tmux").arg("-V").output().is_ok_and(|o| o.status.success())
    }
    /// The sessions with a client attached, as tmux lists them.
    fn attached(&self) -> Vec<String> {
        let o = self.run(&["list-clients", "-F", "#{session_name}"]);
        String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect()
    }
}

impl Drop for PrivateTmux {
    fn drop(&mut self) {
        let _ = self.run(&["kill-server"]);
        let _ = std::fs::remove_file(&self.sock);
    }
}

/// `term_open` names a tmux session, and a client's name for one is either a
/// session that exists or an error that says which. It answered ok for a
/// session that did not exist — the tmux client inside the pty then printed
/// tmux's complaint and quit, so the client held an id for nothing.
#[test]
fn term_open_refuses_a_session_that_does_not_exist() {
    let tmux = PrivateTmux::new("missing");
    let mut sb = Sandbox::new("termmissing");
    let sf = sb.launch_env(&["--allow-origin", "http://localhost:1420"], &[("TMUX", &tmux.env())]);
    let mut ws = connect(&sf, "http://localhost:1420");
    let r = rpc(&mut ws, 1, "term_open", json!({ "session": "wtsb-missing", "cols": 80, "rows": 24, "onBytes": "__CHANNEL__:1" }));
    let err = r["err"].as_str().unwrap_or_else(|| panic!("term_open answered for a missing session: {r}"));
    assert!(err.contains("wtsb-missing"), "{err}");
    assert_eq!(sb.terminate().code(), Some(0));
}

/// ...and the session named EXACTLY. A tmux target is a prefix match, so a
/// `term_open` for a session that had gone attached whichever session's name
/// began with it — another place's agent, typed into from this one's pane.
/// The positive control is in the same server: the real name attaches.
#[test]
fn term_open_attaches_only_the_session_named_exactly() {
    if !PrivateTmux::available() {
        eprintln!("skipped: no tmux on PATH (the refusal test above still runs)");
        return;
    }
    let tmux = PrivateTmux::new("exact");
    let made = tmux.run(&["-f", "/dev/null", "new-session", "-d", "-s", "wtsb-lane", "-x", "80", "-y", "24", "sleep 600"]);
    assert!(made.status.success(), "private tmux: {}", String::from_utf8_lossy(&made.stderr));
    let mut sb = Sandbox::new("termexact");
    let sf = sb.launch_env(&["--allow-origin", "http://localhost:1420"], &[("TMUX", &tmux.env())]);
    let mut ws = connect(&sf, "http://localhost:1420");

    let r = rpc(&mut ws, 1, "term_open", json!({ "session": "wtsb", "cols": 80, "rows": 24, "onBytes": "__CHANNEL__:1" }));
    std::thread::sleep(Duration::from_millis(500));
    assert!(r["err"].is_string(), "a prefix of a session's name was attached: {r}, clients on {:?}", tmux.attached());
    assert!(tmux.attached().is_empty(), "{:?}", tmux.attached());

    let r = rpc(&mut ws, 2, "term_open", json!({ "session": "wtsb-lane", "cols": 80, "rows": 24, "onBytes": "__CHANNEL__:2" }));
    let id = r["ok"].as_u64().unwrap_or_else(|| panic!("the real session was refused: {r}"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while tmux.attached().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(tmux.attached(), vec!["wtsb-lane".to_string()]);
    let r = rpc(&mut ws, 3, "term_close", json!({ "id": id }));
    assert_eq!(r["ok"], Value::Null, "{r}");
    assert_eq!(sb.terminate().code(), Some(0));
}

/// The whole life of a `serve`, end to end through the binary.
#[test]
fn serve_writes_its_file_answers_the_workspace_and_cleans_up_on_sigterm() {
    let mut sb = Sandbox::new("life");
    let sf = sb.launch(&["--allow-origin", "http://localhost:1420"]);

    // serve.json: the four fields, 0600, this child's pid, this version.
    // Measured on macOS 27.0 (Darwin 27) and on Linux 6.12 (Alpine 3.24 and
    // Debian 12 containers), under umask 022, 077 and 000: `mode & 0o777`
    // reads 0o600 on all of them for a file created with mode 0600.
    assert_eq!(mode(&sb.serve_json()), 0o600, "serve.json is {:o}", mode(&sb.serve_json()));
    assert_eq!(sf["pid"].as_u64(), Some(sb.pid() as u64));
    assert_eq!(sf["version"], json!(env!("CARGO_PKG_VERSION")));
    assert_eq!(sf["token"].as_str().map(str::len), Some(64), "256 bits as hex");
    assert!(sf["port"].as_u64().is_some_and(|p| p > 0));

    // The log's FIRST line stamps the version: a long-lived server outlives
    // the binary that replaced it, and this is how a reader tells which one
    // wrote what.
    let log = std::fs::read_to_string(sb.log()).expect("serve.log under the sandbox HOME");
    let first = log.lines().next().unwrap_or_default();
    assert!(first.contains(&format!("startup v{}", env!("CARGO_PKG_VERSION"))), "first log line: {first:?}");

    // The dev origin the flag allowed is served; a request round-trips; the
    // workspace is the sandbox's empty registry, not this machine's.
    let mut ws = connect(&sf, "http://localhost:1420");
    let r = rpc(&mut ws, 1, "list_workspace", json!({}));
    assert_eq!(r["ok"]["projects"], json!([]), "{r}");
    let r = rpc(&mut ws, 2, "settings_info", json!({}));
    assert!(r["ok"]["file"].as_str().unwrap().starts_with(sb.cfg().to_str().unwrap()), "{r}");
    drop(ws);

    // A second serve on the same config dir is refused while this one lives.
    //
    // Bounded, not `.output()`: a second serve that FAILED to refuse would
    // serve forever, and a test that waits on it hangs rather than fails —
    // which is exactly what the first fail-first run of this test did under
    // that mutation. So: spawn, give it a few seconds to exit, and if it is
    // still alive it is a running second engine, which is the failure.
    let mut second = Command::new(bin())
        .args(["serve", "--config-dir"])
        .arg(sb.cfg())
        .env("HOME", sb.root.join("home"))
        .env("XDG_CONFIG_HOME", sb.root.join("xdg"))
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(st) = second.try_wait().unwrap() {
            break Some(st);
        }
        if Instant::now() > deadline {
            let _ = second.kill();
            let _ = second.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut err = String::new();
    let _ = second.stderr.take().unwrap().read_to_string(&mut err);
    let status = status.expect("a second serve started beside a live one and kept running");
    assert!(!status.success(), "a second serve exited 0 beside a live one: {err}");
    assert!(err.contains("already serving"), "{err}");
    assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(sb.serve_json()).unwrap()).unwrap()["pid"], sf["pid"], "the live file was overwritten by the refused second");

    // SIGTERM: a clean exit, and the file goes with it.
    let st = sb.terminate();
    assert!(st.success(), "exit on SIGTERM: {st}");
    assert!(!sb.serve_json().exists(), "serve.json survived the server");
    let log = std::fs::read_to_string(sb.log()).unwrap();
    assert!(log.contains("serve: stopped"), "{log}");
}

/// Every launch rotates the token, and the file is rewritten rather than
/// appended or merged.
#[test]
fn each_launch_rewrites_serve_json_with_a_new_token() {
    let mut sb = Sandbox::new("rotate");
    let a = sb.launch(&[]);
    sb.terminate();
    let b = sb.launch(&[]);
    assert_ne!(a["token"], b["token"], "the token must change every launch");
    assert_ne!(a["port"], json!(0));
    assert_eq!(mode(&sb.serve_json()), 0o600);
    // A WebView origin needs no flag.
    let mut ws = connect(&b, "tauri://localhost");
    assert_eq!(rpc(&mut ws, 1, "get_changelog", json!({}))["ok"]["version"], json!(env!("CARGO_PKG_VERSION")));
    sb.terminate();
}

/// A bad flag is an error at launch, not a server with a silently empty
/// allow-list; `--help` is help.
#[test]
fn the_flags_are_checked_at_launch() {
    let out = Command::new(bin()).args(["serve", "--allow-origin", "localhost:1420"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--allow-origin"), "{}", String::from_utf8_lossy(&out.stderr));
    let out = Command::new(bin()).args(["serve", "--port", "nope"]).output().unwrap();
    assert!(!out.status.success());
    let out = Command::new(bin()).args(["serve", "--help"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("--config-dir"));
}
