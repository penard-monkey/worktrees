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
            // Tight, so a test that stops serve the moment it is up does so
            // within a few ms of the file appearing, not up to 50 ms later.
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// A git repository under the sandbox, registered in the sandbox's OWN
    /// registry (`XDG_CONFIG_HOME` is the sandbox's): the canonical root.
    fn project(&self, name: &str) -> String {
        let dir = self.root.join("projects").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = Command::new("git").args(["init", "-q"]).current_dir(&dir).status().unwrap().success();
        assert!(ok, "git init {}", dir.display());
        let root = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
        self.register(&root, name);
        root
    }

    /// `root` added to the sandbox's registry as `name`, by writing the file:
    /// no `add_project`, so nothing is adopted.
    fn register(&self, root: &str, name: &str) {
        let reg_dir = self.root.join("xdg/worktrees");
        std::fs::create_dir_all(&reg_dir).unwrap();
        let reg = reg_dir.join("projects.json");
        let mut v: Value = std::fs::read(&reg).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({ "version": 1, "projects": [] }));
        v["projects"].as_array_mut().unwrap().push(json!({ "root": root, "name": name }));
        std::fs::write(&reg, serde_json::to_vec(&v).unwrap()).unwrap();
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

/// serve.json is how a client, or whatever supervises serve, learns that it
/// is up, so from the moment the file exists a SIGTERM must be a clean stop:
/// exit 0, the file removed. The handler used to be installed only after the
/// file was written and the engine started, and a SIGTERM inside that window
/// took the default action — killed by the signal, serve.json left behind.
/// Two tests that stop serve straight after launching it lost that race on a
/// machine with no tmux.
#[test]
fn a_sigterm_as_soon_as_serve_json_exists_is_a_clean_stop() {
    let mut sb = Sandbox::new("earlyterm");
    for round in 0..10 {
        sb.launch(&[]);
        let st = sb.terminate();
        assert_eq!(st.code(), Some(0), "round {round}: {st}");
        assert!(!sb.serve_json().exists(), "round {round}: serve.json was left behind");
    }
}

/// Host policy is the ENGINE's (`worktrees_api::hostpolicy`: "only the engine
/// reads it; `boot` loads it at startup"), and `serve` is the engine. A host
/// that chose `ask` for Codex in Settings and then restarted serve must launch
/// lanes under `ask`, not under the default the file is there to override
/// (`auto-review`) — and the same for the auto-fetch cadence and the two
/// dock-shell switches.
///
/// The same boot carries the one-time migration from the app's ui-state.json:
/// a key the host lacks and the old file CHOSE (scrollback off) is copied; a
/// key the host holds is not (the cadence stays 60, not the stale 15); a
/// default there (per-tab history on) is not copied at all. And a set over
/// the socket persists before it answers, then takes effect.
#[test]
fn serve_loads_host_policy_at_boot() {
    let mut sb = Sandbox::new("hostpolicy");
    let policy = sb.cfg().join("host-policy.json");
    std::fs::write(&policy, r#"{"codex_permissions": "ask", "fetch_interval_min": 60}"#).unwrap();
    std::fs::write(
        sb.cfg().join("ui-state.json"),
        r#"{"fetch_interval_min": 15, "term_persist_scrollback": false, "term_per_tab_history": true}"#,
    )
    .unwrap();
    let sf = sb.launch(&["--allow-origin", "http://localhost:1420"]);
    let mut ws = connect(&sf, "http://localhost:1420");
    let r = rpc(&mut ws, 1, "get_codex_permissions", json!({}));
    assert_eq!(r["ok"], json!("ask"), "host-policy.json was not loaded at boot: {r}");
    let r = rpc(&mut ws, 2, "get_fetch_interval", json!({}));
    assert_eq!(r["ok"], json!(60), "the host's cadence, not ui-state.json's: {r}");
    let r = rpc(&mut ws, 3, "get_term_history_opts", json!({}));
    assert_eq!(r["ok"], json!({"persist_scrollback": false, "per_tab_history": true}), "{r}");
    let on_disk = || serde_json::from_slice::<Value>(&std::fs::read(&policy).unwrap()).unwrap();
    let v = on_disk();
    assert_eq!((v["fetch_interval_min"].clone(), v["term_persist_scrollback"].clone()), (json!(60), json!(false)), "{v}");
    assert!(v.get("term_per_tab_history").is_none(), "a default was copied: {v}");

    let r = rpc(&mut ws, 4, "set_term_history_opts", json!({"perTabHistory": false}));
    assert_eq!(r["ok"], Value::Null, "{r}");
    assert_eq!(on_disk()["term_per_tab_history"], json!(false));
    let r = rpc(&mut ws, 5, "get_term_history_opts", json!({}));
    assert_eq!(r["ok"], json!({"persist_scrollback": false, "per_tab_history": false}), "{r}");
    let r = rpc(&mut ws, 6, "set_fetch_interval", json!({"mins": 7}));
    assert!(r["err"].as_str().is_some_and(|e| e.contains("unknown auto-fetch interval 7")), "{r}");
    assert_eq!(on_disk()["fetch_interval_min"], json!(60), "a refused cadence was written");
    assert_eq!(sb.terminate().code(), Some(0));
}

/// The crash-restart case (#525's reconnect). Before host policy the page
/// pushed the auto-fetch cadence and the terminal switches from effects keyed
/// on hydration and settings, and the app stays mounted across a reconnect, so
/// a serve started again after a crash ran on its boot defaults: auto-fetch
/// off, and scrollback persistence back ON, until the user next changed a
/// setting. Now a set persists before it answers, and boot loads it, so the
/// new serve holds what was set with no client pushing anything — and a stale
/// client copy in ui-state.json does not get a say either.
#[test]
fn a_serve_started_again_after_a_crash_holds_the_engine_policies_it_was_set() {
    let mut sb = Sandbox::new("hostpolicy-restart");
    let sf = sb.launch(&["--allow-origin", "http://localhost:1420"]);
    let mut ws = connect(&sf, "http://localhost:1420");
    let r = rpc(&mut ws, 1, "set_fetch_interval", json!({"mins": 15}));
    assert_eq!(r["ok"], Value::Null, "{r}");
    let r = rpc(&mut ws, 2, "set_term_history_opts", json!({"persistScrollback": false}));
    assert_eq!(r["ok"], Value::Null, "{r}");
    drop(ws);

    // A crash: SIGKILL, so nothing of serve's own exit path runs.
    let mut dead = sb.child.take().unwrap();
    dead.kill().unwrap();
    dead.wait().unwrap();
    std::fs::write(
        sb.cfg().join("ui-state.json"),
        r#"{"fetch_interval_min": 60, "term_persist_scrollback": true, "term_per_tab_history": true}"#,
    )
    .unwrap();

    let sf = sb.launch(&["--allow-origin", "http://localhost:1420"]);
    let mut ws = connect(&sf, "http://localhost:1420");
    let r = rpc(&mut ws, 1, "get_fetch_interval", json!({}));
    assert_eq!(r["ok"], json!(15), "the restarted serve lost the cadence it was set: {r}");
    let r = rpc(&mut ws, 2, "get_term_history_opts", json!({}));
    assert_eq!(r["ok"], json!({"persist_scrollback": false, "per_tab_history": true}), "{r}");
    // Every set is a record in the audit log, which both serves append to
    // (§5.1): the two before the crash, and nothing since.
    let audit = std::fs::read_to_string(sb.cfg().join("audit.jsonl")).unwrap();
    let sets = audit
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|r| r["kind"] == "call" && r["cmd"].as_str().is_some_and(|c| c.starts_with("set_")))
        .count();
    assert_eq!(sets, 2, "only the two sets before the crash may set host policy:\n{audit}");
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
        self.try_run(args).expect("tmux")
    }
    /// `run` for cleanup, which must not panic when there is no tmux at all:
    /// the macOS `rust` CI job has none, and a test that needs no tmux (the
    /// refusal below) failed in its Drop.
    fn try_run(&self, args: &[&str]) -> std::io::Result<std::process::Output> {
        Command::new("tmux").arg("-S").arg(&self.sock).args(args).env_remove("TMUX").output()
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
        let _ = self.try_run(&["kill-server"]);
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
/// The positive control is in the same server: the real name attaches. And
/// only a PLACE's session (Server Edition Phase 2, gap 3): one with a pane
/// in a registered project. A live session that is no place's — the user's
/// own tmux — is refused, the console included.
#[test]
fn term_open_attaches_only_the_session_named_exactly() {
    if !PrivateTmux::available() {
        eprintln!("skipped: no tmux on PATH (the refusal test above still runs)");
        return;
    }
    let tmux = PrivateTmux::new("exact");
    let mut sb = Sandbox::new("termexact");
    let project = sb.project("lane");
    let made = tmux.run(&["-f", "/dev/null", "new-session", "-d", "-s", "wtsb-lane", "-c", &project, "-x", "80", "-y", "24", "sleep 600"]);
    assert!(made.status.success(), "private tmux: {}", String::from_utf8_lossy(&made.stderr));
    let made = tmux.run(&["new-session", "-d", "-s", "wtsb-stranger", "-c", "/", "-x", "80", "-y", "24", "sleep 600"]);
    assert!(made.status.success(), "private tmux: {}", String::from_utf8_lossy(&made.stderr));
    let sf = sb.launch_env(&["--allow-origin", "http://localhost:1420"], &[("TMUX", &tmux.env())]);
    let mut ws = connect(&sf, "http://localhost:1420");

    let r = rpc(&mut ws, 1, "term_open", json!({ "session": "wtsb", "cols": 80, "rows": 24, "onBytes": "__CHANNEL__:1" }));
    std::thread::sleep(Duration::from_millis(500));
    assert!(r["err"].is_string(), "a prefix of a session's name was attached: {r}, clients on {:?}", tmux.attached());
    assert!(tmux.attached().is_empty(), "{:?}", tmux.attached());

    let r = rpc(&mut ws, 4, "term_open", json!({ "session": "wtsb-stranger", "cols": 80, "rows": 24, "onBytes": "__CHANNEL__:4" }));
    std::thread::sleep(Duration::from_millis(500));
    assert!(r["err"].is_string(), "a session that is no place's was attached: {r}");
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

/// A registered repo with two documents in it, and its canonical path: what a
/// `worktrees show` names, through the app's list that serve merges at start,
/// so a request is one serve accepts and names a place.
fn shown_repo(sb: &Sandbox) -> PathBuf {
    let repo = sb.root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git").args(["init", "-q"]).current_dir(&repo).env("HOME", sb.root.join("home")).output().unwrap();
    assert!(init.status.success(), "git init: {}", String::from_utf8_lossy(&init.stderr));
    std::fs::write(repo.join("README.md"), "# shown\n").unwrap();
    std::fs::write(repo.join("NOTES.md"), "# also shown\n").unwrap();
    let repo = std::fs::canonicalize(&repo).unwrap();
    std::fs::write(sb.cfg().join("projects.json"), json!([repo.to_string_lossy()]).to_string()).unwrap();
    repo
}

/// `worktrees show <file>` in `repo`, as a session runs it: into the inbox.
fn show(sb: &Sandbox, repo: &Path, file: &str) {
    let out = Command::new(bin())
        .args(["show", file])
        .current_dir(repo)
        .env("HOME", sb.root.join("home"))
        .env("XDG_CONFIG_HOME", sb.root.join("xdg"))
        .output()
        .unwrap();
    assert!(out.status.success(), "worktrees show: {}", String::from_utf8_lossy(&out.stderr));
}

/// How many requests wait in the inbox.
fn pending(sb: &Sandbox) -> usize {
    std::fs::read_dir(sb.root.join("home/.cache/worktrees/inbox"))
        .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).count())
        .unwrap_or(0)
}

/// How far serve.log has got: `until_held` reads only what comes after.
fn log_mark(sb: &Sandbox) -> usize {
    std::fs::read(sb.log()).map(|b| b.len()).unwrap_or(0)
}

/// Until a tick SAYS, after `mark`, that it holds `n` requests, with every one
/// still in the inbox at each look. serve logs the held count when it moves,
/// so no sleep decides it: code that takes a request fails on the other
/// branch of the same wait.
fn until_held(sb: &Sandbox, n: usize, mark: usize, why: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert_eq!(pending(sb), n, "{why}");
        let log = std::fs::read(sb.log()).unwrap_or_default();
        if String::from_utf8_lossy(log.get(mark..).unwrap_or_default()).contains(&format!("inbox: {n} request(s) held")) {
            return;
        }
        assert!(Instant::now() < deadline, "no tick said it held {n} request(s)");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Say which events this connection listens for: `{"listen": [...]}` (§9.3),
/// a full replacement, never answered.
fn listen(ws: &mut WebSocket<TcpStream>, names: &[&str]) {
    ws.send(Message::text(json!({ "listen": names }).to_string())).unwrap();
}

/// Read until `n` `app:open-doc` events have come; their paths, sorted. Any
/// `err` on the way fails — a listen frame serve could not read is one.
fn docs(ws: &mut WebSocket<TcpStream>, n: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut got = Vec::new();
    while got.len() < n {
        let v: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert!(v.get("err").is_none(), "serve refused a frame: {v}");
        if v["event"] == json!("app:open-doc") {
            assert_eq!(v["payload"]["slug"], json!("(main)"), "{v}");
            got.push(v["payload"]["path"].as_str().unwrap().to_string());
        }
        assert!(Instant::now() < deadline, "only {} of {n} app:open-doc arrived", got.len());
    }
    got.sort();
    got
}

/// One round trip, and every `app:open-doc` that arrived before its reply.
/// The read loop takes a connection's frames in order, so a frame sent before
/// this request has been read by the time it is answered.
fn docs_before_reply(ws: &mut WebSocket<TcpStream>, id: u64) -> usize {
    ws.send(Message::text(json!({ "id": id, "cmd": "get_changelog", "args": {} }).to_string())).unwrap();
    let mut docs = 0;
    loop {
        let v: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        if v["id"] == json!(id) {
            assert!(v.get("ok").is_some(), "{v}");
            return docs;
        }
        assert!(v.get("err").is_none(), "serve refused a frame: {v}");
        docs += usize::from(v["event"] == json!("app:open-doc"));
    }
}

/// A `worktrees show` that lands while NO client is connected waits for the
/// next one, rather than being taken off the inbox and emitted to nobody. After
/// an app crash the app's serve lives on client-less, and it used to consume
/// every request within a tick, lost — where v0.42.0, with the app down, left
/// it for the next launch, inside the inbox's 30 s life (rehearsal D2).
///
/// No sleep decides it: serve SAYS when a tick holds a request, and the old
/// code fails on the other branch of the same wait, the request gone.
#[test]
fn a_show_with_no_client_connected_waits_for_the_next_client() {
    let mut sb = Sandbox::new("inbox");
    let repo = shown_repo(&sb);
    let sf = sb.launch(&["--allow-origin", "http://localhost:1420"]);

    let mark = log_mark(&sb);
    show(&sb, &repo, "README.md");
    assert_eq!(pending(&sb), 1, "the ask must reach the inbox");
    until_held(&sb, 1, mark, "the request was taken off the inbox with no client connected to show it to");

    // The next client that listens for it is shown it, and that consumes it.
    let mut ws = connect(&sf, "http://localhost:1420");
    listen(&mut ws, &["app:open-doc"]);
    assert_eq!(docs(&mut ws, 1), vec![repo.join("README.md").to_string_lossy().to_string()]);
    assert_eq!(pending(&sb), 0, "a request shown to a client is consumed");
    drop(ws);
    assert!(sb.terminate().success());
}

/// **A held show goes to a connection that LISTENS for it, not to any socket**
/// (§9.3, slice 1b). The app's verification probe completes handshake v1, so
/// serve counted it as an audience and could release a held show into a
/// socket about to close (ROADMAP's probe-drain item). Now:
///
/// - a connection that declares nothing — the probe's shape — drains nothing
///   and is sent no `app:open-doc`, however long it stays;
/// - one that declares `app:open-doc` is shown every held request;
/// - a declaration REPLACES the last, so declaring nothing makes it no
///   audience again;
/// - a new connection (a reconnect) that declares again is shown the next.
#[test]
fn a_held_show_goes_only_to_a_connection_that_declared_it_listens() {
    let mut sb = Sandbox::new("listen");
    let repo = shown_repo(&sb);
    let sf = sb.launch(&["--allow-origin", "http://localhost:1420"]);
    let (readme, notes) = (repo.join("README.md").to_string_lossy().to_string(), repo.join("NOTES.md").to_string_lossy().to_string());

    let mark = log_mark(&sb);
    show(&sb, &repo, "README.md");
    until_held(&sb, 1, mark, "taken with no client connected");
    // The probe's shape: proven, and declaring nothing. A second ask while it
    // is connected makes a tick SAY it held both, which proves a tick ran
    // with this connection in.
    let mut probe = connect(&sf, "http://localhost:1420");
    let mark = log_mark(&sb);
    show(&sb, &repo, "NOTES.md");
    until_held(&sb, 2, mark, "a connection that declared nothing took a held show");
    assert_eq!(docs_before_reply(&mut probe, 1), 0, "a connection that declared nothing was sent an app:open-doc");
    drop(probe);

    // A connection that declares it is shown both, and that consumes them.
    let mut app = connect(&sf, "http://localhost:1420");
    listen(&mut app, &["places:changed", "app:open-doc", "no:such-event"]);
    assert_eq!(docs(&mut app, 2), vec![notes.clone(), readme.clone()]);
    assert_eq!(pending(&sb), 0, "a request shown to a client is consumed");

    // Each frame replaces the last: declaring nothing, it is no audience.
    listen(&mut app, &[]);
    assert_eq!(docs_before_reply(&mut app, 2), 0);
    let mark = log_mark(&sb);
    show(&sb, &repo, "README.md");
    until_held(&sb, 1, mark, "a connection whose last declaration was empty took a held show");
    drop(app);

    // A reconnect: the new connection declares again, and is shown it.
    let mut again = connect(&sf, "http://localhost:1420");
    listen(&mut again, &["app:open-doc"]);
    assert_eq!(docs(&mut again, 1), vec![readme]);
    assert_eq!(pending(&sb), 0);
    drop(again);
    assert!(sb.terminate().success());
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

/// This machine's own non-loopback IPv4 address, or `None` if it has only
/// `lo`: the docs server's `own_lan_addr`, which a test crate cannot import. A
/// UDP socket "connected" to a TEST-NET address sends nothing, but the kernel
/// picks the route's source address and `local_addr` reads it back.
fn own_lan_addr() -> Option<Ipv4Addr> {
    let s = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// Reachable on loopback and from nowhere else. The witness is a CONNECT to
/// this machine's own LAN address on serve's port, never a bind: no bind
/// answers the same on macOS and Linux (the table on the docs server's
/// `the_server_is_bound_to_loopback_and_its_token_is_fresh_each_time`), while
/// a connect does — measured on macOS 27.0 and Linux 6.12, refused against a
/// `127.0.0.1` listener and accepted against `0.0.0.0` on both. `conn.rs`'s
/// after-accept peer check does not hide a wider bind from this: the kernel
/// completes the handshake before `accept` returns, so the connect succeeds
/// and only then is the stream dropped.
///
/// The loopback connect comes FIRST because without it a dead server passes.
/// And on a CI runner, which always has a LAN address, a missing one fails
/// rather than skipping — a witness that quietly did not run is not one.
#[test]
fn serve_can_be_reached_on_loopback_and_not_off_it() {
    let mut sb = Sandbox::new("lan");
    let sf = sb.launch(&[]);
    let port = sf["port"].as_u64().unwrap() as u16;
    let wait = Duration::from_secs(5);

    let via_lo = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    assert!(
        TcpStream::connect_timeout(&via_lo, wait).is_ok(),
        "serve did not answer on loopback, so the refusal below would prove nothing"
    );

    match own_lan_addr() {
        // A firewall that DROPs rather than refuses is a timeout, also an
        // `Err` — the fail-safe direction.
        Some(ip) => {
            let via_lan = SocketAddr::from((ip, port));
            assert!(
                TcpStream::connect_timeout(&via_lan, wait).is_err(),
                "{via_lan} answered, so serve is reachable off loopback"
            );
        }
        None => {
            assert!(std::env::var_os("CI").is_none(), "no non-loopback IPv4 on a CI runner: the LAN half did not run");
            eprintln!("no non-loopback IPv4 on this host; the LAN half did not run");
        }
    }
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

fn git_in(dir: &Path, args: &[&str]) {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

/// A stand-in Claude: a copy of `sleep` named `claude`, so tmux reports
/// `claude` as the pane's command on both OSes. Copied by `cp`, so this
/// process never holds the file open for writing (the ETXTBSY trap, AGENTS.md),
/// and ad-hoc signed on macOS, which kills an unsigned copy of a system binary.
fn fake_claude(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join("claude");
    let sleep = if Path::new("/bin/sleep").exists() { "/bin/sleep" } else { "/usr/bin/sleep" };
    assert!(Command::new("cp").arg(sleep).arg(&bin).status().unwrap().success(), "cp {sleep}");
    if cfg!(target_os = "macos") {
        let _ = Command::new("codesign").args(["-s", "-", "-f"]).arg(&bin).output();
    }
    bin
}

/// The reported bug, end to end through the binary. A project's folder is
/// renamed with Claude running in its (main) and in a lane; the user removes
/// the old root and adds the new one. `agent_turns` must read the live
/// Claude's turns both BEFORE adoption — its session still `old-(main)`, found
/// by pane cwd, by the snapshot's rule — and AFTER `add_project` renamed it to
/// `new-(main)`, while its probe still names `old-(main)` and only the pane id
/// ties the two. Before the fix the first read answered "no agent is running
/// in this place".
#[test]
fn a_moved_projects_agent_is_read_before_and_after_it_is_adopted() {
    if !PrivateTmux::available() {
        eprintln!("skipped: no tmux on PATH");
        return;
    }
    let tmux = PrivateTmux::new("moved");
    let mut sb = Sandbox::new("moved");
    let projects = sb.root.join("projects");
    let old = projects.join("old");
    std::fs::create_dir_all(&old).unwrap();
    git_in(&old, &["init", "-q"]);
    git_in(&old, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"]);
    git_in(&old, &["worktree", "add", "-q", ".worktrees/lane", "-b", "lane"]);
    let old_root = std::fs::canonicalize(&old).unwrap().to_string_lossy().into_owned();
    let claude = fake_claude(&sb.root.join("fake"));
    for (session, dir) in [("old-(main)", old_root.clone()), ("old-lane", format!("{old_root}/.worktrees/lane"))] {
        let made = tmux.run(&["-f", "/dev/null", "new-session", "-d", "-s", session, "-c", &dir, "-x", "80", "-y", "24", &format!("{} 600", claude.display())]);
        assert!(made.status.success(), "private tmux: {}", String::from_utf8_lossy(&made.stderr));
    }
    // Claude's own files for the (main) one, as Claude writes them: its probe
    // names the session and pane it started in, its transcript is filed under
    // the folder it started in.
    let shown = tmux.run(&["display-message", "-p", "-t", "=old-(main):", "#{pane_pid}\t#{window_id}\t#{pane_id}"]);
    let shown = String::from_utf8_lossy(&shown.stdout).trim().to_string();
    let mut it = shown.split('\t');
    let (pid, window, pane) = (it.next().unwrap().to_string(), it.next().unwrap().to_string(), it.next().unwrap().to_string());
    let claude_home = sb.root.join("home/.claude");
    std::fs::create_dir_all(claude_home.join("sessions")).unwrap();
    std::fs::write(
        claude_home.join(format!("sessions/{pid}.json")),
        json!({ "pid": pid.parse::<i64>().unwrap(), "cwd": old_root, "status": "busy", "kind": "interactive",
                "tmux": format!("old-(main):{window}.{pane}"), "sessionId": "sid-main" }).to_string(),
    )
    .unwrap();
    let mangled: String = old_root.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let tdir = claude_home.join("projects").join(mangled);
    std::fs::create_dir_all(&tdir).unwrap();
    std::fs::write(tdir.join("sid-main.jsonl"), include_str!("../../worktrees-core/tests/fixtures/transcripts/claude-2.1.309.jsonl")).unwrap();

    // The rename, outside worktrees. Both registrations, as the user had them.
    let new = projects.join("new");
    std::fs::rename(&old, &new).unwrap();
    let new_root = std::fs::canonicalize(&new).unwrap().to_string_lossy().into_owned();
    sb.register(&old_root, "old");
    sb.register(&new_root, "new");
    let sf = sb.launch_env(&["--allow-origin", "http://localhost:1420"], &[("TMUX", &tmux.env())]);
    let mut ws = connect(&sf, "http://localhost:1420");
    listen(&mut ws, &["sessions:busy"]);

    // Removing the old root: its folder is gone, so nothing of it runs there.
    let r = rpc(&mut ws, 1, "remove_project", json!({ "root": old_root }));
    assert!(r["ok"]["projects"].is_array() && r["ok"].get("left_running").is_none(), "{r}");

    // A + B, no rename: the session found by pane cwd, the probe by name and pane.
    let r = rpc(&mut ws, 2, "agent_turns", json!({ "repo": new_root, "slug": "(main)" }));
    let before = &r["ok"];
    assert_eq!(before["session"], json!("old-(main)"), "{r}");
    assert_eq!(before["harness"], json!("claude"), "{r}");
    let turns = before["turns"].as_array().unwrap_or_else(|| panic!("{r}")).clone();
    assert!(!turns.is_empty(), "{r}");

    // D: adding the new root adopts it — the lane's git links repaired, both
    // sessions renamed, nothing closed.
    let r = rpc(&mut ws, 3, "add_project", json!({ "dir": new_root }));
    let adopted = &r["ok"]["adopted"];
    assert_eq!(adopted["repaired"], json!([format!("{new_root}/.worktrees/lane")]), "{r}");
    let renamed: Vec<(String, String)> = adopted["renamed"]
        .as_array()
        .unwrap_or_else(|| panic!("{r}"))
        .iter()
        .map(|x| (x["from"].as_str().unwrap().to_string(), x["to"].as_str().unwrap().to_string()))
        .collect();
    assert!(renamed.contains(&("old-(main)".into(), "new-(main)".into())) && renamed.contains(&("old-lane".into(), "new-lane".into())), "{r}");
    let live = String::from_utf8_lossy(&tmux.run(&["list-sessions", "-F", "#{session_name}"]).stdout).into_owned();
    let mut live: Vec<&str> = live.lines().collect();
    live.sort_unstable();
    assert_eq!(live, ["new-(main)", "new-lane"]);
    git_in(&new.join(".worktrees/lane"), &["status", "-s"]);

    // B, after the rename: the probe still says `old-(main)`; its pane is the lane's.
    let r = rpc(&mut ws, 4, "agent_turns", json!({ "repo": new_root, "slug": "(main)" }));
    assert_eq!(r["ok"]["session"], json!("new-(main)"), "{r}");
    assert_eq!(r["ok"]["turns"], json!(turns), "the same conversation: {r}");

    // Adding it again finds nothing more to do.
    let r = rpc(&mut ws, 5, "add_project", json!({ "dir": new_root }));
    assert!(r["ok"]["projects"].is_array() && r["ok"].get("adopted").is_none(), "{r}");

    // The nav follows the pane too: the model label (found by the session the
    // Claude runs in now) and the busy dot (keyed by the place it is in now,
    // never the folder it started in, which is gone).
    let r = rpc(&mut ws, 6, "list_places", json!({ "repo": new_root }));
    let main = r["ok"]["places"].as_array().and_then(|ps| ps.iter().find(|p| p["slug"] == json!("(main)"))).unwrap_or_else(|| panic!("{r}"));
    assert_eq!(main["agent_sessions"]["claude"]["name"], json!("new-(main)"), "{main}");
    assert!(main["agent_sessions"]["claude"]["model"].is_string(), "the model label after the rename: {main}");
    let busy = busy_until(&mut ws, &json!([new_root]), Duration::from_secs(15));
    assert_eq!(busy, json!([new_root]), "the busy dot names the new folder");
    assert_eq!(sb.terminate().code(), Some(0));
}

/// Read `sessions:busy` events until one's `busy` is `want`, or `within` has
/// passed; the last `busy` seen either way.
fn busy_until(ws: &mut WebSocket<TcpStream>, want: &Value, within: Duration) -> Value {
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let deadline = Instant::now() + within;
    let mut last = Value::Null;
    while Instant::now() < deadline && last != *want {
        match ws.read() {
            Ok(m) => {
                let v: Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
                if v["event"] == json!("sessions:busy") {
                    last = v["payload"]["busy"].clone();
                }
            }
            Err(tokio_tungstenite::tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => panic!("reading events: {e}"),
        }
    }
    ws.get_mut().set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    last
}

/// C, through the binary: `project_sessions` lists a project's live sessions
/// with the agent in each, and `remove_project` names what it leaves running
/// — and adds nothing to its answer when nothing is.
#[test]
fn remove_project_names_the_sessions_it_leaves_running() {
    if !PrivateTmux::available() {
        eprintln!("skipped: no tmux on PATH");
        return;
    }
    let tmux = PrivateTmux::new("rmproj");
    let mut sb = Sandbox::new("rmproj");
    let busy = sb.project("busy");
    let quiet = sb.project("quiet");
    let claude = fake_claude(&sb.root.join("fake"));
    let made = tmux.run(&["-f", "/dev/null", "new-session", "-d", "-s", "busy-(main)", "-c", &busy, "-x", "80", "-y", "24", &format!("{} 600", claude.display())]);
    assert!(made.status.success(), "private tmux: {}", String::from_utf8_lossy(&made.stderr));
    let sf = sb.launch_env(&["--allow-origin", "http://localhost:1420"], &[("TMUX", &tmux.env())]);
    let mut ws = connect(&sf, "http://localhost:1420");

    let want = json!([{ "slug": "(main)", "session": "busy-(main)", "agent": "claude" }]);
    let r = rpc(&mut ws, 1, "project_sessions", json!({ "root": busy }));
    assert_eq!(r["ok"]["sessions"], want, "{r}");
    let r = rpc(&mut ws, 2, "project_sessions", json!({ "root": quiet }));
    assert_eq!(r["ok"]["sessions"], json!([]), "{r}");

    let r = rpc(&mut ws, 3, "remove_project", json!({ "root": quiet }));
    assert!(r["ok"]["projects"].is_array() && r["ok"].get("left_running").is_none(), "nothing new without sessions: {r}");
    let r = rpc(&mut ws, 4, "remove_project", json!({ "root": busy }));
    assert_eq!(r["ok"]["left_running"], want, "{r}");
    assert_eq!(r["ok"]["projects"], json!([]), "removed all the same: {r}");
    // Nothing was closed: removing a project only unregisters it.
    assert!(tmux.run(&["has-session", "-t", "=busy-(main)"]).status.success());
    assert_eq!(sb.terminate().code(), Some(0));
}
