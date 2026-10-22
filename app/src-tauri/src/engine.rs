//! Finding or starting this host's `worktrees serve` — the shim's one engine job.
//!
//! One engine process per host (`worktrees_api`'s crate doc, decision 2 of
//! `docs/proposals/server-edition.md`): the nav-dot poll, the dock shells, the
//! docs server and every writer of the engine's files run in `worktrees serve`,
//! and this app is a client of it like any other. What the shim keeps is the
//! window, the client-local commands, and this: read `<app config dir>/
//! serve.json`, decide whether the serve it names is one to use, and if not,
//! start one from the CLI the app carries.
//!
//! The decision is a PURE function over what was observed (`decide`), so every
//! branch is a unit test that runs on both CI platforms; `observe` does the I/O
//! and stops at the first step that fails. A serve counts as usable only when
//! ALL of these hold: the pid is alive, the port accepts a connection, the
//! handshake's server proof verifies (it holds the token serve.json names), and
//! the version it proved is this app's. Anything less is a stale file, an
//! impostor, or a mismatch, and each has its own answer:
//!
//! | observed | decision |
//! |---|---|
//! | no file, an unreadable file, a dead pid, a live pid on a closed port | start one |
//! | the port's holder fails the proof | REFUSE — nothing is killed, the file is kept |
//! | proven, this app's version, `owner: "app"` | adopt it (ours to stop on quit) |
//! | proven, this app's version, no owner | use it (never signalled) |
//! | proven, another version | replace it: SIGTERM, wait, start ours; never SIGKILL |
//!
//! A pid that failed the proof is never signalled: whatever holds that port
//! did not show it holds the token, so it is not ours to stop — and a real
//! serve can fail a probe too (a connection-cap flood; in dev, a serve that
//! does not allow the dev page's origin), in which case starting a second one
//! would be refused by serve itself (`servefile::live_other`).
//!
//! **Lifetime.** A serve this app starts runs `--owner app`; the app SIGTERMs
//! it on a clean quit, which is what ends the dock shells and flushes their
//! rings (serve's `host::stop`). If the app crashes the serve survives (it is
//! started in its own session), and the next launch ADOPTS it because its
//! serve.json says `owner: "app"` — so a crash does not leave an engine running
//! unseen until reboot. A serve someone else started (the CLI, the dev stack)
//! has no owner, and the app uses it and leaves it running.
//!
//! **Reconnecting.** The page retries on its own when the socket drops
//! (`app/src/reconnect.ts` holds the policy: backoff, the crash-loop budget,
//! who may be restarted). The shim holds only the mechanism: an ask with
//! `may_start: false` never spawns and never replaces, and every answer says
//! truthfully whether this call reached `start()` and whether the serve it
//! found is this app's — the two facts that policy needs and only this side
//! knows. A port that closed under a LIVE pid is a serve still stopping, and
//! is waited for before one is started beside it (`settle`).

use std::ffi::OsString;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hmac::{KeyInit, Mac};
use serde::{Deserialize, Serialize};
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

/// The file serve writes into the config dir it serves (`servefile.rs`).
pub const SERVE_FILE: &str = "serve.json";

/// What a serve the app starts is told about its owner (`serve --owner`), and
/// what an adoptable serve.json says.
pub const OWNER: &str = "app";

/// Where every serve logs — named in every error a person sees, because it is
/// the one place that says why a serve did not start.
pub const SERVE_LOG: &str = "~/.cache/worktrees/serve.log";

/// The Origin the shim's own probe presents in a release build: the webview's
/// on macOS and Linux (wry: "the `<scheme>://localhost` protocols used on macOS
/// and Linux"). serve's built-in allowlist has it, and the probe asking with it
/// is the probe asking whether serve will take THIS app's page.
pub const APP_ORIGIN: &str = "tauri://localhost";

// Handshake v1, client side. The spec is (main)'s `handshake-v1.md`; serve's
// side is `crates/worktrees-cli/src/serve/policy.rs`, and
// `engine-route-check.mjs` holds these two labels against its constants.
pub const CLIENT_LABEL: &str = "worktrees-serve/v1/client\n";
pub const SERVER_LABEL: &str = "worktrees-serve/v1/server\n";

/// How long the whole probe may take. serve's own `AUTH_DEADLINE` is 3 s; a
/// server that is there and refusing closes before this.
const PROBE_BUDGET: Duration = Duration::from_secs(4);
/// How long a started serve has to write its file and prove itself. Its boot
/// runs the login-shell PATH probe, which is deadline-guarded at 5 s.
const READY_WAIT: Duration = Duration::from_secs(20);
/// How long a replaced serve has to stop after SIGTERM. Its `host::stop` plus
/// a 5 s runtime shutdown bound; past this the old serve is left running and
/// the app says so, rather than escalate to SIGKILL and lose its rings.
const STOP_WAIT: Duration = Duration::from_secs(10);
/// How long a clean quit waits to see its serve go, for the log line only:
/// the serve finishes stopping on its own either way.
const QUIT_WAIT: Duration = Duration::from_secs(3);

/// `<config dir>/serve.json`, as serve writes it. The token is a capability
/// for everything the engine can do, so `Debug` never prints it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ServeFile {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub version: String,
    #[serde(default)]
    pub owner: Option<String>,
}

impl std::fmt::Debug for ServeFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "serve(pid {}, port {}, v{}, owner {})", self.pid, self.port, self.version, self.owner.as_deref().unwrap_or("none"))
    }
}

/// What `observe` found, in the order it looks: the file, the pid, the port,
/// the proof. Each variant is the first step that failed, or the last one
/// that held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    NoFile,
    Unparsable(String),
    DeadPid(ServeFile),
    PortClosed(ServeFile),
    /// The port answered, and what answered did not prove it holds the token.
    Unproven(ServeFile, String),
    /// A serve that proved itself; the version is the one its PROVEN reply
    /// carried, not the file's claim.
    Proven(ServeFile, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// A proven serve on this version that nobody asked us to manage.
    Use(ServeFile),
    /// A proven serve on this version that an app started: ours to stop.
    Adopt(ServeFile),
    /// A proven serve on another version (that version): stop it, start ours.
    Replace(ServeFile, String),
    /// Nothing usable is there; start one. The reason is for the log.
    Start(String),
    /// Something holds the port that may not be touched.
    Refuse(String),
    /// A start (or a replace) was the answer, and this ask may not start one.
    Decline(String),
}

/// The decision, from the observation alone. Pure: every branch is a test.
pub fn decide(seen: Seen, ours: &str) -> Decision {
    match seen {
        Seen::NoFile => Decision::Start("no serve.json".into()),
        Seen::Unparsable(why) => Decision::Start(format!("serve.json is unreadable ({why})")),
        Seen::DeadPid(sf) => Decision::Start(format!("serve.json names pid {}, which is not running", sf.pid)),
        Seen::PortClosed(sf) => Decision::Start(format!(
            "serve.json names pid {} on port {}, and nothing is listening there",
            sf.pid, sf.port
        )),
        Seen::Unproven(sf, why) => Decision::Refuse(format!(
            "something is listening on 127.0.0.1:{} (serve.json names pid {}) and it did not prove it holds \
             serve.json's token ({why}). It was not stopped, and serve.json was left as it is: a process that \
             cannot prove it is our serve is not ours to stop. If no worktrees serve should be running, stop pid {} \
             yourself or delete serve.json, then reconnect.",
            sf.port, sf.pid, sf.pid
        )),
        Seen::Proven(sf, theirs) if theirs != ours => Decision::Replace(sf, theirs),
        Seen::Proven(sf, _) if sf.owner.as_deref() == Some(OWNER) => Decision::Adopt(sf),
        Seen::Proven(sf, _) => Decision::Use(sf),
    }
}

/// The page's `start: false` applied to a decision. A call that may not start
/// may not replace either: a replace is a SIGTERM and then a start. Pure.
pub fn permit(d: Decision, may_start: bool) -> Decision {
    match d {
        Decision::Start(why) if !may_start => Decision::Decline(format!("nothing to reconnect to: {why}")),
        Decision::Replace(sf, theirs) if !may_start => {
            Decision::Decline(format!("the serve on 127.0.0.1:{} (pid {}) is v{theirs}, not this app's", sf.port, sf.pid))
        }
        d => d,
    }
}

/// Look, in order, and stop at the first step that fails.
pub fn observe(config_dir: &Path, origin: &str) -> Seen {
    let path = config_dir.join(SERVE_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == ErrorKind::NotFound => return Seen::NoFile,
        Err(e) => return Seen::Unparsable(e.to_string()),
    };
    let sf: ServeFile = match serde_json::from_slice(&bytes) {
        Ok(sf) => sf,
        Err(e) => return Seen::Unparsable(e.to_string()),
    };
    if !pid_alive(sf.pid) {
        return Seen::DeadPid(sf);
    }
    if !port_open(sf.port) {
        return Seen::PortClosed(sf);
    }
    match probe(sf.port, &sf.token, origin) {
        Ok(v) => Seen::Proven(sf, v),
        Err(why) => Seen::Unproven(sf, why),
    }
}

/// `kill(pid, 0)`: 0 means it exists, `EPERM` means it exists and is not ours
/// to signal — still "exists". A pid outside `pid_t` cannot be addressed and
/// is dead without asking; cast negative it would name a process GROUP.
pub fn pid_alive(pid: u32) -> bool {
    let Ok(p) = libc::pid_t::try_from(pid) else { return false };
    if p <= 0 {
        return false;
    }
    // SAFETY: signal 0 delivers nothing.
    let r = unsafe { libc::kill(p, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 64 LOWERCASE hex characters — the spec's one spelling; anything else is
/// refused rather than folded.
fn is_hex32(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if !is_hex32(s) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// `HMAC-SHA256(token, label + cn + "\n" + sn)`, not yet finalized.
fn mac(token: &str, label: &str, cn: &str, sn: &str) -> HmacSha256 {
    let mut m = HmacSha256::new_from_slice(token.as_bytes()).expect("HMAC takes a key of any length");
    m.update(label.as_bytes());
    m.update(cn.as_bytes());
    m.update(b"\n");
    m.update(sn.as_bytes());
    m
}

/// This client's proof, as hex.
fn client_proof(token: &str, cn: &str, sn: &str) -> String {
    hex(&mac(token, CLIENT_LABEL, cn, sn).finalize().into_bytes())
}

/// Does `proof` show the server holds `token`, for these nonces? Constant-time.
fn server_proof_ok(token: &str, cn: &str, sn: &str, proof: &str) -> bool {
    let Some(tag) = unhex32(proof) else { return false };
    mac(token, SERVER_LABEL, cn, sn).verify_slice(&tag).is_ok()
}

/// One text frame, or why there was none. A probe has no use for pings.
fn read_json<S: Read + Write>(ws: &mut tungstenite::WebSocket<S>) -> Result<serde_json::Value, String> {
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => return serde_json::from_str(t.as_str()).map_err(|e| format!("an unreadable frame: {e}")),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Ok(Message::Close(_)) => return Err("it closed the connection".into()),
            Ok(_) => return Err("it sent a frame that is not text".into()),
            Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                return Err("it closed the connection".into())
            }
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err("it stopped answering".into())
            }
            Err(e) => return Err(format!("it closed the connection ({e})")),
        }
    }
}

/// Handshake v1 against `127.0.0.1:port`, as a client: hello → challenge →
/// auth → ok + proof. `Ok` is the version the server's PROVEN reply carries;
/// `Err` says what went wrong in words for a person. Nothing but the handshake
/// is ever sent, and the token is the HMAC key — never in a frame.
pub fn probe(port: u16, token: &str, origin: &str) -> Result<String, String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).map_err(|e| format!("connect: {e}"))?;
    let _ = stream.set_read_timeout(Some(PROBE_BUDGET));
    let _ = stream.set_write_timeout(Some(PROBE_BUDGET));
    let mut req = format!("ws://127.0.0.1:{port}/").into_client_request().map_err(|e| e.to_string())?;
    req.headers_mut().insert("Origin", origin.parse().map_err(|_| format!("origin {origin:?} is not a header value"))?);
    let (mut ws, _) = tungstenite::client::client(req, stream).map_err(|e| match e {
        tungstenite::HandshakeError::Failure(e) => format!("it refused the WebSocket upgrade ({e})"),
        tungstenite::HandshakeError::Interrupted(_) => "the WebSocket upgrade was interrupted".to_string(),
    })?;
    let mut cn = [0u8; 32];
    getrandom::fill(&mut cn).map_err(|e| format!("the OS refused entropy: {e}"))?;
    let cn = hex(&cn);
    ws.send(Message::text(serde_json::json!({ "hello": cn }).to_string())).map_err(|e| format!("hello: {e}"))?;
    let challenge = read_json(&mut ws)?;
    let sn = challenge
        .get("challenge")
        .and_then(|v| v.as_str())
        .filter(|s| is_hex32(s) && challenge.as_object().is_some_and(|o| o.len() == 1))
        .ok_or("it answered the hello with something other than a challenge")?
        .to_string();
    ws.send(Message::text(serde_json::json!({ "auth": client_proof(token, &cn, &sn) }).to_string()))
        .map_err(|e| format!("auth: {e}"))?;
    let ok = read_json(&mut ws).map_err(|e| format!("{e} at this client's proof — it does not hold serve.json's token"))?;
    if ok.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err("it answered the proof with something other than ok".into());
    }
    let proof = ok.get("proof").and_then(|v| v.as_str()).ok_or("its ok carries no proof")?;
    if !server_proof_ok(token, &cn, &sn, proof) {
        return Err("its proof does not verify".into());
    }
    let version = ok.get("version").and_then(|v| v.as_str()).ok_or("its ok carries no version")?.to_string();
    let _ = ws.close(None);
    let _ = ws.flush();
    Ok(version)
}

/// serve's argv. `--allow-origin` is added ONLY for `Some(dev_origin)`, and the
/// only producer of a `Some` is `run()`'s `#[cfg(dev)]` path (the page `tauri
/// dev` loads is `http://localhost:1420`, not `tauri://localhost`). A release
/// build passes `None`, so a dev origin cannot ship — `engine-route-check.mjs`
/// holds both halves of that.
pub fn serve_args(config_dir: &Path, dev_origin: Option<&str>) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["serve".into(), "--config-dir".into(), config_dir.as_os_str().to_owned(), "--owner".into(), OWNER.into()];
    if let Some(o) = dev_origin {
        a.push("--allow-origin".into());
        a.push(o.into());
    }
    a
}

/// Where the serve binary is, and how it was found (for app.log).
///
/// A release build looks where the bundle puts the private CLI: beside the
/// app binary on macOS (`Contents/MacOS/worktrees`), and in the resource dir on
/// Linux (`/usr/lib/worktrees/worktrees` for the .deb, `$APPDIR/usr/lib/
/// worktrees/worktrees` for the AppImage — both `resource_dir()/worktrees`).
/// A dev build never looks there: `tauri dev`'s resource dir is
/// `target/debug`, which can hold a debug CLI, so "found there" would not mean
/// "the bundle". Dev takes `WORKTREES_SERVE_BIN`, else the newer of the
/// workspace's `target/release/worktrees` and `target/debug/worktrees`.
pub fn serve_bin(
    dev: bool,
    env: Option<&Path>,
    workspace_target: &Path,
    exe_dir: Option<&Path>,
    resource_dir: Option<&Path>,
) -> Result<(PathBuf, String), String> {
    let file = |p: &Path| std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false);
    if dev {
        if let Some(p) = env {
            return if file(p) {
                Ok((p.to_path_buf(), "WORKTREES_SERVE_BIN".into()))
            } else {
                Err(format!("WORKTREES_SERVE_BIN={} is not a file", p.display()))
            };
        }
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut found: Vec<(PathBuf, &str)> = ["release", "debug"]
            .into_iter()
            .map(|prof| (workspace_target.join(prof).join("worktrees"), prof))
            .filter(|(p, _)| file(p))
            .collect();
        found.sort_by_key(|(p, _)| std::cmp::Reverse(mtime(p)));
        return found
            .into_iter()
            .next()
            .map(|(p, prof)| (p, format!("the workspace's newest build (target/{prof})")))
            .ok_or_else(|| {
                format!(
                    "a dev build needs a worktrees CLI to run serve: set WORKTREES_SERVE_BIN, or build one \
                     (cargo build --release -p worktrees-cli) — looked in {}/{{release,debug}}",
                    workspace_target.display()
                )
            });
    }
    let (dir, what) = if cfg!(target_os = "macos") { (exe_dir, "beside the app binary") } else { (resource_dir, "in the resource dir") };
    let dir = dir.ok_or_else(|| format!("cannot resolve the app's own directory to find its worktrees CLI ({what})"))?;
    let p = dir.join("worktrees");
    if file(&p) {
        Ok((p, format!("the app bundle ({what})")))
    } else {
        Err(format!("this app carries no worktrees CLI at {} — reinstall the app", p.display()))
    }
}

/// What the webview needs to connect: the token travels only in this answer
/// to `engine_connect`, and lives in the page only as an HMAC key.
/// `owned` and `started` are the two facts the page's reconnect policy needs
/// and cannot see: whether this serve is the app's to restart if it dies (one
/// it started or adopted), and whether this call started one (its crash-loop
/// budget counts those).
#[derive(Serialize, Clone)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
    pub version: String,
    pub owned: bool,
    pub started: bool,
}

/// Never the token, as for `ServeFile`.
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "endpoint(port {}, v{}, owned {}, started {})", self.port, self.version, self.owned, self.started)
    }
}

/// What the page asks of one connect. `may_start` is its reconnect policy
/// reaching the one place that can spawn; `manual` is a person's press of
/// Reconnect now, which the log says ("on request"). `reason` is why the page
/// is reconnecting, null at launch.
#[derive(Debug, Clone, Copy)]
pub struct Ask<'a> {
    pub reason: Option<&'a str>,
    pub may_start: bool,
    pub manual: bool,
}

/// Why a connect failed, as the page gets it (Tauri serializes it as the
/// rejection). `started`: this call reached `start()`, whatever came of it —
/// a spawn, a spawn that exited, a missing binary — so a serve that can never
/// come up spends the crash-loop budget instead of being retried forever.
/// `declined`: nothing usable was there and this ask could not start one.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ConnectError {
    pub message: String,
    pub started: bool,
    pub declined: bool,
}

impl ConnectError {
    pub fn failed(message: impl Into<String>) -> Self {
        ConnectError { message: message.into(), started: false, declined: false }
    }
    fn after_start(message: impl Into<String>) -> Self {
        ConnectError { message: message.into(), started: true, declined: false }
    }
}

/// A serve whose life this app manages.
#[derive(Clone)]
enum Owned {
    /// We spawned it: our child. `alive` is cleared by the thread that reaps
    /// it, so the pid is signalled only while it cannot have been reused.
    Started { pid: u32, alive: Arc<AtomicBool> },
    /// An app started it and this one adopted it after a crash. Not our
    /// child, so it is checked by pid AND probe before it is signalled.
    Adopted { pid: u32 },
}

impl Owned {
    fn pid(&self) -> u32 {
        match self {
            Owned::Started { pid, .. } | Owned::Adopted { pid } => *pid,
        }
    }
}

/// Everything the shim needs to find, start and stop its engine. Managed by
/// Tauri; `connect` is serialized, so two Reconnects cannot start two serves.
pub struct Engine {
    pub config_dir: PathBuf,
    /// The Origin the probe presents: `APP_ORIGIN`, or the dev page's.
    pub origin: String,
    /// `--allow-origin` for a serve we start — `Some` only under `cfg(dev)`.
    pub dev_origin: Option<String>,
    pub bin: Result<(PathBuf, String), String>,
    /// The docs viewer's browser bundle, handed to a serve we start as
    /// `WORKTREES_VIEWER_JS` (serve has no resource dir of its own).
    pub viewer_js: Option<PathBuf>,
    pub version: String,
    connecting: Mutex<()>,
    owned: Mutex<Option<Owned>>,
}

impl Engine {
    pub fn new(
        config_dir: PathBuf,
        origin: String,
        dev_origin: Option<String>,
        bin: Result<(PathBuf, String), String>,
        viewer_js: Option<PathBuf>,
        version: String,
    ) -> Self {
        Engine { config_dir, origin, dev_origin, bin, viewer_js, version, connecting: Mutex::new(()), owned: Mutex::new(None) }
    }

    /// Find or start the engine and say how to reach it. `phase` hears what a
    /// long call is doing ("waiting-old", "starting"), for the page's banner.
    /// Blocking: up to `STOP_WAIT` for a stopping serve to go, `READY_WAIT`
    /// when one has to start, `STOP_WAIT` more to replace one.
    pub fn connect(&self, ask: Ask, phase: &dyn Fn(&str)) -> Result<Endpoint, ConnectError> {
        let _one = self.connecting.lock().unwrap_or_else(|p| p.into_inner());
        let on_request = if ask.manual { " on request" } else { "" };
        if let Some(r) = ask.reason {
            log("info", &format!("engine: reconnect requested{on_request} ({r}); may start: {}", if ask.may_start { "yes" } else { "no" }));
        }
        let seen = observe(&self.config_dir, &self.origin);
        log("info", &format!("engine: observed {}", describe(&seen)));
        // Only a call that may start has a reason to wait for the old one to go.
        let seen = if ask.may_start { self.settle(seen, STOP_WAIT, phase) } else { seen };
        match permit(decide(seen, &self.version), ask.may_start) {
            Decision::Use(sf) => {
                self.keep_if(sf.pid);
                log("info", &format!("engine: using {sf:?}, started by someone else — it stays running when the app quits"));
                Ok(self.endpoint(&sf, false))
            }
            Decision::Adopt(sf) => {
                self.adopt(sf.pid);
                log("info", &format!("engine: adopting {sf:?} — an app started it; this app stops it on quit"));
                Ok(self.endpoint(&sf, false))
            }
            Decision::Start(why) => {
                log("info", &format!("engine: {why} → starting one{on_request}"));
                phase("starting");
                self.start(ask.manual)
            }
            Decision::Replace(sf, theirs) => {
                log("info", &format!("engine: {sf:?} proved v{theirs}, this app is v{} → replacing it{on_request}", self.version));
                phase("starting");
                self.terminate(&sf).map_err(ConnectError::failed)?;
                self.start(ask.manual)
            }
            Decision::Decline(why) => {
                log("info", &format!("engine: {why} → not starting one: this attempt may not"));
                Err(ConnectError { message: why, started: false, declined: true })
            }
            Decision::Refuse(why) => {
                log("error", &format!("engine: refused: {why}"));
                Err(ConnectError::failed(format!("{why} (serve's log: {SERVE_LOG})")))
            }
        }
    }

    /// The answer for `sf`. Owned is what this shim MANAGES, not what the file
    /// claims: `owned` names a pid only after a start or an adopt of it.
    fn endpoint(&self, sf: &ServeFile, started: bool) -> Endpoint {
        let owned = self.owned.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|w| w.pid() == sf.pid);
        Endpoint { port: sf.port, token: sf.token.clone(), version: sf.version.clone(), owned, started }
    }

    /// A port that closed under a LIVE pid is, nearly always, a serve on its
    /// way out: serve stops its listener first, then `host::stop` flushes the
    /// dock rings and shell-cwds.json, and only then removes serve.json.
    /// Starting now would put a second engine beside one still writing those
    /// files, and serve's own one-engine refusal cannot see it (`live_other`
    /// asks the PORT). So wait, up to `bound`, for that pid to go, and look
    /// again. Past the bound the pid is taken for a reused one — a stale
    /// serve.json naming some other process — and the observation stands.
    fn settle(&self, seen: Seen, bound: Duration, phase: &dyn Fn(&str)) -> Seen {
        let Seen::PortClosed(sf) = &seen else { return seen };
        phase("waiting-old");
        log("info", &format!("engine: {sf:?} is alive with nothing on its port — waiting up to {}s for it to stop before starting one", bound.as_secs()));
        if wait_gone(&self.config_dir, sf.pid, bound) {
            log("info", &format!("engine: the old serve (pid {}) stopped", sf.pid));
            let again = observe(&self.config_dir, &self.origin);
            log("info", &format!("engine: observed {}", describe(&again)));
            again
        } else {
            log(
                "warn",
                &format!(
                    "engine: pid {} is still alive after {}s with nothing on port {} — taking it for a reused pid, not a serve, and starting one anyway",
                    sf.pid,
                    bound.as_secs(),
                    sf.port
                ),
            );
            seen
        }
    }

    /// A found serve is ours only if it is the one we already manage.
    fn keep_if(&self, pid: u32) {
        let mut o = self.owned.lock().unwrap_or_else(|p| p.into_inner());
        if o.as_ref().is_some_and(|w| w.pid() != pid) {
            *o = None;
        }
    }

    fn adopt(&self, pid: u32) {
        let mut o = self.owned.lock().unwrap_or_else(|p| p.into_inner());
        if o.as_ref().is_some_and(|w| w.pid() == pid) {
            return; // already ours — started by us, and its reaper still watches it
        }
        *o = Some(Owned::Adopted { pid });
    }

    /// SIGTERM a PROVEN serve of another version, and wait for it to go: its
    /// pid gone, or its serve.json no longer naming it. Never SIGKILL.
    fn terminate(&self, sf: &ServeFile) -> Result<(), String> {
        signal(sf.pid, libc::SIGTERM).map_err(|e| format!("could not stop the old serve (pid {}): {e} (serve's log: {SERVE_LOG})", sf.pid))?;
        if wait_gone(&self.config_dir, sf.pid, STOP_WAIT) {
            log("info", &format!("engine: the old serve (pid {}) stopped", sf.pid));
            return Ok(());
        }
        let msg = format!(
            "the running worktrees serve (pid {}, v{}) did not stop within {}s of SIGTERM, so this app (v{}) cannot \
             start its own. It was not killed; stop it yourself and reconnect (serve's log: {SERVE_LOG})",
            sf.pid,
            sf.version,
            STOP_WAIT.as_secs(),
            self.version
        );
        log("error", &format!("engine: {msg}"));
        Err(msg)
    }

    /// Spawn serve and wait until it is proven on this version. Every answer
    /// says `started`: the call got here.
    fn start(&self, manual: bool) -> Result<Endpoint, ConnectError> {
        let (bin, how) = self.bin.clone().map_err(|e| {
            log("error", &format!("engine: no serve binary: {e}"));
            ConnectError::after_start(format!("{e} (serve's log: {SERVE_LOG})"))
        })?;
        let args = serve_args(&self.config_dir, self.dev_origin.as_deref());
        let mut cmd = worktrees_core::childenv::command(&bin);
        cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // The bundle is handed over only when nobody named one already: an
        // override set on purpose wins, exactly as `viewer::bundle_path` reads it.
        if std::env::var_os("WORKTREES_VIEWER_JS").is_none() {
            if let Some(js) = &self.viewer_js {
                cmd.env("WORKTREES_VIEWER_JS", js);
            }
        }
        // Its own session: a serve outlives this app on a crash by design, and
        // must not take a SIGINT or SIGHUP meant for the terminal `tauri dev`
        // runs in.
        // SAFETY: setsid is async-signal-safe and touches nothing of the parent.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut cmd, || {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| {
            let m = format!("could not start {}: {e}", bin.display());
            log("error", &format!("engine: {m}"));
            ConnectError::after_start(format!("{m} (serve's log: {SERVE_LOG})"))
        })?;
        let pid = child.id();
        let on_request = if manual { " on request" } else { "" };
        log(
            "info",
            &format!("engine: started serve pid={pid}{on_request} bin={} ({how}) dev_origin={}", bin.display(), self.dev_origin.as_deref().unwrap_or("none")),
        );
        match self.wait_ready(&mut child) {
            Ok(sf) => {
                let alive = Arc::new(AtomicBool::new(true));
                *self.owned.lock().unwrap_or_else(|p| p.into_inner()) = Some(Owned::Started { pid, alive: alive.clone() });
                reap(child, alive);
                log("info", &format!("engine: serve pid={pid} is ready on 127.0.0.1:{} — this app stops it on quit", sf.port));
                Ok(self.endpoint(&sf, true))
            }
            Err(Ready::Exited(status)) => {
                // Another launch may have won the race to the config dir: serve
                // refuses a second engine (live_other), so look once more.
                log("warn", &format!("engine: serve pid={pid} exited before it was ready ({status})"));
                match decide(observe(&self.config_dir, &self.origin), &self.version) {
                    Decision::Use(sf) => {
                        log("info", &format!("engine: another serve won the race: using {sf:?}"));
                        Ok(self.endpoint(&sf, true))
                    }
                    Decision::Adopt(sf) => {
                        self.adopt(sf.pid);
                        log("info", &format!("engine: another app's serve won the race: adopting {sf:?}"));
                        Ok(self.endpoint(&sf, true))
                    }
                    _ => Err(ConnectError::after_start(format!("worktrees serve exited before it was ready ({status}); its log says why: {SERVE_LOG}"))),
                }
            }
            Err(Ready::Failed(why)) => {
                // Ours and not good: stop it rather than leave it running.
                let _ = signal(pid, libc::SIGTERM);
                reap(child, Arc::new(AtomicBool::new(true)));
                log("error", &format!("engine: serve pid={pid}: {why}"));
                Err(ConnectError::after_start(format!("{why} (serve's log: {SERVE_LOG})")))
            }
        }
    }

    fn wait_ready(&self, child: &mut Child) -> Result<ServeFile, Ready> {
        let pid = child.id();
        let deadline = Instant::now() + READY_WAIT;
        loop {
            if let Ok(Some(st)) = child.try_wait() {
                return Err(Ready::Exited(st.to_string()));
            }
            if let Some(sf) = read_file(&self.config_dir).filter(|f| f.pid == pid) {
                return match probe(sf.port, &sf.token, &self.origin) {
                    Ok(v) if v == self.version => Ok(sf),
                    Ok(v) => Err(Ready::Failed(format!(
                        "the serve binary this app started is v{v}, and this app is v{} — rebuild or reinstall so they match",
                        self.version
                    ))),
                    Err(e) => Err(Ready::Failed(format!("the serve this app started does not pass its own handshake: {e}"))),
                };
            }
            if Instant::now() >= deadline {
                return Err(Ready::Failed(format!("worktrees serve (pid {pid}) did not become ready within {}s", READY_WAIT.as_secs())));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// On a clean quit: stop the serve this app manages, and only that one.
    /// A started serve is signalled while its reaper says it has not exited
    /// (so its pid cannot have been reused); an adopted one only after the
    /// file still names it AND it proves itself again — it is not our child,
    /// and its pid could belong to anything by now.
    pub fn stop_owned(&self) {
        let owned = self.owned.lock().unwrap_or_else(|p| p.into_inner()).take();
        let Some(owned) = owned else {
            log("info", "engine: quitting; the serve in use is not this app's to stop");
            return;
        };
        let pid = owned.pid();
        match &owned {
            Owned::Started { alive, .. } => {
                if !alive.load(Ordering::SeqCst) {
                    log("info", &format!("engine: quitting; serve pid={pid} had already exited"));
                    return;
                }
            }
            Owned::Adopted { .. } => {
                let still = read_file(&self.config_dir).filter(|f| f.pid == pid && f.owner.as_deref() == Some(OWNER));
                let proven = still.as_ref().is_some_and(|f| pid_alive(pid) && probe(f.port, &f.token, &self.origin).is_ok());
                if !proven {
                    log("info", &format!("engine: quitting; the adopted serve pid={pid} is no longer the one serve.json proves, so it is left alone"));
                    return;
                }
            }
        }
        if let Err(e) = signal(pid, libc::SIGTERM) {
            log("warn", &format!("engine: quitting; could not SIGTERM serve pid={pid}: {e}"));
            return;
        }
        let deadline = Instant::now() + QUIT_WAIT;
        let gone = |o: &Owned| match o {
            Owned::Started { alive, .. } => !alive.load(Ordering::SeqCst),
            Owned::Adopted { pid } => !pid_alive(*pid),
        };
        while Instant::now() < deadline && !gone(&owned) {
            std::thread::sleep(Duration::from_millis(50));
        }
        if gone(&owned) {
            log("info", &format!("engine: quitting; serve pid={pid} stopped"));
        } else {
            log("info", &format!("engine: quitting; serve pid={pid} was sent SIGTERM and is still flushing — it finishes on its own"));
        }
    }
}

enum Ready {
    Exited(String),
    Failed(String),
}

/// Wait, up to `bound`, for `pid` to go: gone, or serve.json no longer naming
/// it (serve removes its file as the last step of stopping). True if it went.
fn wait_gone(config_dir: &Path, pid: u32, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if !pid_alive(pid) || !read_file(config_dir).is_some_and(|f| f.pid == pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn read_file(config_dir: &Path) -> Option<ServeFile> {
    serde_json::from_slice(&std::fs::read(config_dir.join(SERVE_FILE)).ok()?).ok()
}

fn signal(pid: u32, sig: libc::c_int) -> Result<(), String> {
    let p = libc::pid_t::try_from(pid).map_err(|_| format!("pid {pid} is out of range"))?;
    if p <= 0 {
        return Err(format!("pid {pid} is not a process"));
    }
    // SAFETY: a plain syscall on a single, positive pid.
    if unsafe { libc::kill(p, sig) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

/// Wait for our child on a thread of its own, so it never lingers as a zombie
/// (`kill(pid, 0)` answers for a zombie too), and so app.log says how it ended.
fn reap(mut child: Child, alive: Arc<AtomicBool>) {
    let pid = child.id();
    let spawned = std::thread::Builder::new().name(format!("serve-reaper-{pid}")).spawn(move || {
        let st = child.wait();
        alive.store(false, Ordering::SeqCst);
        match st {
            Ok(st) => log("info", &format!("engine: serve pid={pid} exited ({st})")),
            Err(e) => log("warn", &format!("engine: serve pid={pid}: wait failed: {e}")),
        }
    });
    if let Err(e) = spawned {
        log("warn", &format!("engine: could not watch serve pid={pid}: {e}"));
    }
}

/// One line for app.log about what was seen — never the token.
fn describe(seen: &Seen) -> String {
    match seen {
        Seen::NoFile => "no serve.json".into(),
        Seen::Unparsable(e) => format!("an unreadable serve.json ({e})"),
        Seen::DeadPid(sf) => format!("{sf:?}: pid not running"),
        Seen::PortClosed(sf) => format!("{sf:?}: port closed"),
        Seen::Unproven(sf, why) => format!("{sf:?}: UNPROVEN ({why})"),
        Seen::Proven(sf, v) => format!("{sf:?}: proven v{v}"),
    }
}

fn log(level: &str, msg: &str) {
    worktrees_api::log::log(level, msg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::process::Command;

    fn sf(pid: u32, owner: Option<&str>) -> ServeFile {
        ServeFile { port: 41_237, token: "t".repeat(64), pid, version: "1.0.0".into(), owner: owner.map(str::to_string) }
    }

    // ── decide: every branch ────────────────────────────────────────────────

    #[test]
    fn no_file_starts_one() {
        assert!(matches!(decide(Seen::NoFile, "1.0.0"), Decision::Start(_)));
    }

    #[test]
    fn an_unparsable_file_starts_one() {
        assert!(matches!(decide(Seen::Unparsable("eof".into()), "1.0.0"), Decision::Start(why) if why.contains("unreadable")));
    }

    #[test]
    fn a_dead_pid_starts_one() {
        assert!(matches!(decide(Seen::DeadPid(sf(7, None)), "1.0.0"), Decision::Start(why) if why.contains("not running")));
    }

    #[test]
    fn a_live_pid_on_a_closed_port_starts_one() {
        assert!(matches!(decide(Seen::PortClosed(sf(7, None)), "1.0.0"), Decision::Start(why) if why.contains("nothing is listening")));
    }

    /// The one branch that may never touch anything: no kill, no delete, and
    /// the words say so — and the token is not among them.
    #[test]
    fn a_port_held_by_something_that_fails_the_proof_is_refused_and_left_alone() {
        let d = decide(Seen::Unproven(sf(7, Some(OWNER)), "its proof does not verify".into()), "1.0.0");
        let Decision::Refuse(why) = d else { panic!("{d:?}") };
        assert!(why.contains("did not prove") && why.contains("was not stopped") && why.contains("serve.json was left"), "{why}");
        assert!(!why.contains(&"t".repeat(64)), "the refusal carries the token");
        // An owner in the file does not make an unproven serve ours.
    }

    #[test]
    fn a_live_serve_on_this_version_with_no_owner_is_used_and_not_ours() {
        assert_eq!(decide(Seen::Proven(sf(7, None), "1.0.0".into()), "1.0.0"), Decision::Use(sf(7, None)));
    }

    #[test]
    fn a_live_serve_on_this_version_that_an_app_started_is_adopted() {
        assert_eq!(decide(Seen::Proven(sf(7, Some("app")), "1.0.0".into()), "1.0.0"), Decision::Adopt(sf(7, Some("app"))));
        // Any other owner string is not this app's to manage.
        assert_eq!(decide(Seen::Proven(sf(7, Some("cli")), "1.0.0".into()), "1.0.0"), Decision::Use(sf(7, Some("cli"))));
    }

    /// The version that counts is the PROVEN one, not the file's claim, and a
    /// mismatch replaces whoever started it.
    #[test]
    fn a_live_serve_on_another_version_is_replaced() {
        assert_eq!(decide(Seen::Proven(sf(7, None), "0.9.0".into()), "1.0.0"), Decision::Replace(sf(7, None), "0.9.0".into()));
        assert_eq!(decide(Seen::Proven(sf(7, Some("app")), "0.9.0".into()), "1.0.0"), Decision::Replace(sf(7, Some("app")), "0.9.0".into()));
        let mut liar = sf(7, None);
        liar.version = "1.0.0".into();
        assert!(matches!(decide(Seen::Proven(liar, "0.9.0".into()), "1.0.0"), Decision::Replace(_, v) if v == "0.9.0"));
    }

    // ── permit: the page's `start: false`, on every decision ────────────────

    #[test]
    fn an_ask_that_may_not_start_neither_starts_nor_replaces_and_changes_nothing_else() {
        let all = || {
            vec![
                Decision::Use(sf(7, None)),
                Decision::Adopt(sf(7, Some(OWNER))),
                Decision::Replace(sf(7, None), "0.9.0".into()),
                Decision::Start("no serve.json".into()),
                Decision::Refuse("unproven".into()),
            ]
        };
        for d in all() {
            assert_eq!(permit(d.clone(), true), d, "may_start leaves every decision as it is");
        }
        let gated: Vec<Decision> = all().into_iter().map(|d| permit(d, false)).collect();
        assert_eq!(gated[0], Decision::Use(sf(7, None)), "a live serve is always reconnected to");
        assert_eq!(gated[1], Decision::Adopt(sf(7, Some(OWNER))));
        assert!(matches!(&gated[2], Decision::Decline(why) if why.contains("v0.9.0") && why.contains("pid 7")), "{:?}", gated[2]);
        assert!(matches!(&gated[3], Decision::Decline(why) if why.contains("no serve.json")), "{:?}", gated[3]);
        assert_eq!(gated[4], Decision::Refuse("unproven".into()), "a refusal stays a refusal");
    }

    /// The page reads these two shapes by name (app/src/ipc.ts, main.tsx).
    #[test]
    fn the_answers_carry_ownership_and_whether_a_start_happened() {
        let ep = Endpoint { port: 1, token: "t".into(), version: "1.0.0".into(), owned: true, started: false };
        let v = serde_json::to_value(&ep).unwrap();
        assert_eq!((v["owned"].as_bool(), v["started"].as_bool(), v["port"].as_u64()), (Some(true), Some(false), Some(1)));
        let e = serde_json::to_value(ConnectError { message: "m".into(), started: true, declined: false }).unwrap();
        assert_eq!(e, serde_json::json!({ "message": "m", "started": true, "declined": false }));
    }

    fn engine_on(d: &Path, bin: Result<(PathBuf, String), String>) -> Engine {
        Engine::new(d.to_path_buf(), APP_ORIGIN.into(), None, bin, None, "1.0.0".into())
    }

    /// A "serve" that records that it ran and exits at once, so a test can
    /// tell a start from no start without a real serve.
    fn marker_bin(d: &Path) -> (PathBuf, PathBuf) {
        let marker = d.join("ran");
        let bin = d.join("fake-serve");
        exec_fixture(&bin, &format!("#!/bin/sh\ntouch '{}'\nexit 3\n", marker.display()));
        (bin, marker)
    }

    /// `worktrees_core::fsx::exec_fixture`, copied because that one is
    /// `#[cfg(test)]` in core: an executable a test is about to run, written
    /// by a short-lived `sh`. An in-process write here leaves an fd that a
    /// concurrent test's fork can inherit, and Linux refuses to exec a file
    /// that is open for writing (ETXTBSY).
    fn exec_fixture(path: &Path, body: &str) {
        use std::io::Write;
        let mut sh = Command::new("/bin/sh")
            .args(["-c", r#"cat > "$1" && chmod 0755 "$1""#, "sh"])
            .arg(path)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        sh.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
        assert!(sh.wait().unwrap().success(), "exec fixture {}", path.display());
    }

    #[test]
    fn connect_under_start_false_spawns_nothing_and_says_it_declined() {
        let d = tmp("decline");
        let (bin, marker) = marker_bin(&d);
        let e = engine_on(&d, Ok((bin, "test".into())));
        let phases = Mutex::new(Vec::<String>::new());
        let rec = |p: &str| phases.lock().unwrap().push(p.to_string());
        let no = e.connect(Ask { reason: Some("lost"), may_start: false, manual: false }, &rec).unwrap_err();
        assert!(no.declined && !no.started && no.message.contains("no serve.json"), "{no:?}");
        assert!(!marker.exists(), "start:false ran the serve binary");
        assert!(phases.lock().unwrap().is_empty(), "nothing to wait for, nothing started");
        // The same ask allowed to start does start — and says so, though this
        // "serve" exits at once: that is what the crash-loop budget counts.
        let yes = e.connect(Ask { reason: Some("lost"), may_start: true, manual: true }, &rec).unwrap_err();
        assert!(yes.started && !yes.declined && yes.message.contains("exited before it was ready"), "{yes:?}");
        assert!(marker.exists(), "start:true did not run the serve binary");
        assert_eq!(*phases.lock().unwrap(), ["starting"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_binary_counts_as_a_start_so_the_budget_ends_it() {
        let d = tmp("nobin");
        let e = engine_on(&d, Err("no CLI".into()));
        let r = e.connect(Ask { reason: None, may_start: true, manual: false }, &|_: &str| {}).unwrap_err();
        assert!(r.started && !r.declined && r.message.contains("no CLI"), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    // ── settle: a serve still stopping is waited for ───────────────────────

    #[test]
    fn a_closed_port_under_a_pid_that_exits_is_waited_for_then_looked_at_again() {
        let d = tmp("settle-gone");
        let mut child = Command::new("sleep").arg("0.4").spawn().unwrap();
        let pid = child.id();
        // Reaped as it exits: kill(pid, 0) answers for a zombie too.
        let reaper = std::thread::spawn(move || child.wait());
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        write(&d, &ServeFile { port: closed, ..sf(pid, Some(OWNER)) });
        let seen = observe(&d, APP_ORIGIN);
        assert!(matches!(seen, Seen::PortClosed(_)), "{seen:?}");
        let phases = Mutex::new(Vec::<String>::new());
        let t0 = Instant::now();
        let after = engine_on(&d, Err("unused".into())).settle(seen, Duration::from_secs(5), &|p: &str| phases.lock().unwrap().push(p.into()));
        assert!(t0.elapsed() < Duration::from_secs(4), "it waited out the bound instead of the pid ({:?})", t0.elapsed());
        assert!(matches!(after, Seen::DeadPid(ref f) if f.pid == pid), "looked again once the pid went: {after:?}");
        assert_eq!(*phases.lock().unwrap(), ["waiting-old"]);
        let _ = reaper.join();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_closed_port_under_a_pid_that_never_goes_falls_through_to_a_start_after_the_bound() {
        let d = tmp("settle-stuck");
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        write(&d, &ServeFile { port: closed, ..sf(live_pid(), None) });
        let seen = observe(&d, APP_ORIGIN);
        let t0 = Instant::now();
        let after = engine_on(&d, Err("unused".into())).settle(seen.clone(), Duration::from_millis(300), &|_: &str| {});
        assert!(t0.elapsed() >= Duration::from_millis(300), "gave up before the bound ({:?})", t0.elapsed());
        assert_eq!(after, seen, "past the bound the observation stands");
        assert!(matches!(decide(after, "1.0.0"), Decision::Start(_)), "…and it is a start");
        // Anything but a closed port is not waited for at all.
        let t1 = Instant::now();
        assert_eq!(engine_on(&d, Err("unused".into())).settle(Seen::NoFile, Duration::from_secs(5), &|_: &str| panic!("no phase")), Seen::NoFile);
        assert!(t1.elapsed() < Duration::from_millis(100));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_token_never_reaches_debug_output() {
        let s = format!("{:?} {:?}", sf(7, None), Seen::Proven(sf(7, None), "1".into()));
        assert!(!s.contains(&"t".repeat(64)), "{s}");
        assert!(s.contains("pid 7"), "{s}");
    }

    // ── the handshake, against the spec's own numbers ──────────────────────

    /// Vectors computed with Python's `hmac` from the spec's formula, not with
    /// this crate's code: `HMAC-SHA256(key, label + cn + "\n" + sn)`.
    const K: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    const CLIENT_VEC: &str = "a8df32262613c998eb04d1aeb0e67d156e8fea0fcb7b576150b4a02e0d302eef";
    const SERVER_VEC: &str = "ab27afbf33c22c0d8effac46229f1fceabcb1e6125ae709ae5f30487e3a74585";

    #[test]
    fn the_proofs_are_the_specs() {
        assert_eq!(client_proof(K, CN, SN), CLIENT_VEC);
        assert!(server_proof_ok(K, CN, SN, SERVER_VEC));
        assert!(!server_proof_ok(K, CN, SN, CLIENT_VEC), "a reflected client proof is not a server proof");
        assert!(!server_proof_ok(K, SN, CN, SERVER_VEC), "cn and sn are ordered");
        assert!(!server_proof_ok(K, CN, SN, &SERVER_VEC.to_uppercase()), "one spelling");
        assert!(!server_proof_ok(&"b".repeat(64), CN, SN, SERVER_VEC), "another token");
    }

    /// What the fake server does once a client is upgraded.
    #[derive(Clone, Copy)]
    enum Fake {
        /// The spec, honestly, with this token and version.
        Honest,
        /// Talks the protocol, but holds another token.
        WrongToken,
    }

    /// A one-connection WebSocket server on an ephemeral port, speaking
    /// handshake v1 from the spec (its proofs are checked against the vectors
    /// above by `the_proofs_are_the_specs`). Returns the port, and the frames
    /// the client sent.
    fn fake_serve(token: &'static str, version: &'static str, how: Fake) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            fn next(ws: &mut tungstenite::WebSocket<TcpStream>, got: &mut Vec<String>) -> Option<serde_json::Value> {
                match ws.read() {
                    Ok(Message::Text(t)) => {
                        got.push(t.as_str().to_string());
                        serde_json::from_str(t.as_str()).ok()
                    }
                    _ => None,
                }
            }
            // Past any connection that never upgrades: `observe`'s bare port
            // check opens and closes one before the probe, as it does on serve.
            // Bounded, so a client that never upgrades fails the test rather
            // than hanging it in `accept` (a mutated probe did exactly that).
            l.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut got = Vec::new();
            let mut ws = loop {
                match l.accept() {
                    Ok((s, _)) => {
                        s.set_nonblocking(false).unwrap();
                        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        if let Ok(ws) = tungstenite::accept(s) {
                            break ws;
                        }
                    }
                    Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => return got,
                }
            };
            let Some(hello) = next(&mut ws, &mut got) else { return got };
            let cn = hello["hello"].as_str().unwrap_or_default().to_string();
            let sn = "fedcba9876543210".repeat(4);
            ws.send(Message::text(serde_json::json!({ "challenge": sn }).to_string())).unwrap();
            let Some(auth) = next(&mut ws, &mut got) else { return got };
            // An honest serve closes, saying nothing, on a client proof that is
            // not the token's; a stand-in without the token cannot check one
            // and answers anyway — with a proof under the key it does hold.
            let key = match how {
                Fake::Honest => {
                    if auth["auth"].as_str() != Some(hex(&mac(token, CLIENT_LABEL, &cn, &sn).finalize().into_bytes()).as_str()) {
                        return got;
                    }
                    token
                }
                Fake::WrongToken => "not the token",
            };
            let proof = hex(&mac(key, SERVER_LABEL, &cn, &sn).finalize().into_bytes());
            let _ = ws.send(Message::text(serde_json::json!({ "ok": true, "version": version, "proof": proof }).to_string()));
            let _ = next(&mut ws, &mut got);
            got
        });
        (port, h)
    }

    #[test]
    fn a_serve_that_proves_itself_is_believed_and_the_token_never_crosses() {
        let (port, h) = fake_serve(K, "1.2.3", Fake::Honest);
        assert_eq!(probe(port, K, APP_ORIGIN), Ok("1.2.3".to_string()));
        let sent = h.join().unwrap();
        assert_eq!(sent.len(), 2, "hello and auth, nothing else: {sent:?}");
        assert!(sent.iter().all(|f| !f.contains(K)), "the token went on the wire: {sent:?}");
        assert!(sent[0].starts_with("{\"hello\":\""), "{sent:?}");
    }

    #[test]
    fn a_holder_of_the_port_without_the_token_is_not_believed() {
        let (port, h) = fake_serve(K, "1.2.3", Fake::WrongToken);
        let e = probe(port, K, APP_ORIGIN).unwrap_err();
        assert!(e.contains("does not verify"), "{e}");
        let _ = h.join();
    }

    #[test]
    fn a_port_that_closes_on_us_is_not_believed() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || drop(l.accept()));
        assert!(probe(port, K, APP_ORIGIN).is_err());
        let _ = h.join();
    }

    // ── observe: the file, the pid, the port, the proof ────────────────────

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-engine-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(d: &Path, sf: &ServeFile) {
        std::fs::write(d.join(SERVE_FILE), serde_json::to_vec(sf).unwrap()).unwrap();
    }

    /// The parent process is alive for the whole test on both platforms, and
    /// is not this one.
    fn live_pid() -> u32 {
        // SAFETY: getppid cannot fail.
        unsafe { libc::getppid() as u32 }
    }

    #[test]
    fn observe_walks_file_pid_port_proof_and_stops_at_the_first_failure() {
        let d = tmp("observe");
        assert_eq!(observe(&d, APP_ORIGIN), Seen::NoFile);

        std::fs::write(d.join(SERVE_FILE), "{not json").unwrap();
        assert!(matches!(observe(&d, APP_ORIGIN), Seen::Unparsable(_)));

        // A pid no process has (xnu's PID_MAX is 99998; Linux caps pid_max at
        // 4194304), and one outside pid_t that a corrupt file can carry.
        write(&d, &sf(2_000_000_000, None));
        assert!(matches!(observe(&d, APP_ORIGIN), Seen::DeadPid(_)));
        write(&d, &sf(4_000_000_000, None));
        assert!(matches!(observe(&d, APP_ORIGIN), Seen::DeadPid(_)));

        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        write(&d, &ServeFile { port: closed, ..sf(live_pid(), None) });
        assert!(matches!(observe(&d, APP_ORIGIN), Seen::PortClosed(_)));

        let (port, h) = fake_serve(K, "1.2.3", Fake::WrongToken);
        write(&d, &ServeFile { port, token: K.into(), ..sf(live_pid(), Some("app")) });
        assert!(matches!(observe(&d, APP_ORIGIN), Seen::Unproven(..)));
        let _ = h.join();

        let (port, h) = fake_serve(K, "1.2.3", Fake::Honest);
        write(&d, &ServeFile { port, token: K.into(), ..sf(live_pid(), Some("app")) });
        assert!(matches!(observe(&d, APP_ORIGIN), Seen::Proven(f, v) if v == "1.2.3" && f.owner.as_deref() == Some("app")));
        let _ = h.join();
        let _ = std::fs::remove_dir_all(&d);
    }

    // ── what is started, and from where ────────────────────────────────────

    #[test]
    fn serve_is_started_as_the_apps_and_a_dev_origin_only_when_given_one() {
        let d = Path::new("/cfg dir");
        let rel: Vec<String> = serve_args(d, None).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(rel, ["serve", "--config-dir", "/cfg dir", "--owner", "app"]);
        assert!(!rel.iter().any(|a| a == "--allow-origin"), "no dev origin, no --allow-origin");
        let dev: Vec<String> = serve_args(d, Some("http://localhost:1420")).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(&dev[5..], ["--allow-origin", "http://localhost:1420"]);
    }

    #[test]
    fn a_dev_build_takes_the_override_then_the_newest_workspace_build_and_never_the_resource_dir() {
        let d = tmp("bin");
        let target = d.join("target");
        let res = d.join("res");
        std::fs::create_dir_all(target.join("release")).unwrap();
        std::fs::create_dir_all(target.join("debug")).unwrap();
        std::fs::create_dir_all(&res).unwrap();
        std::fs::write(res.join("worktrees"), "x").unwrap();
        // Nothing built: an error that says what to do — the resource dir's
        // copy is NOT taken.
        let e = serve_bin(true, None, &target, Some(&res), Some(&res)).unwrap_err();
        assert!(e.contains("WORKTREES_SERVE_BIN") && e.contains("cargo build"), "{e}");
        std::fs::write(target.join("debug/worktrees"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(target.join("release/worktrees"), "x").unwrap();
        let (p, how) = serve_bin(true, None, &target, None, None).unwrap();
        assert_eq!(p, target.join("release/worktrees"), "the newer of the two");
        assert!(how.contains("target/release"), "{how}");
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(target.join("debug/worktrees"), "y").unwrap();
        assert_eq!(serve_bin(true, None, &target, None, None).unwrap().0, target.join("debug/worktrees"));
        let over = d.join("mine");
        std::fs::write(&over, "x").unwrap();
        assert_eq!(serve_bin(true, Some(&over), &target, None, None).unwrap(), (over.clone(), "WORKTREES_SERVE_BIN".to_string()));
        assert!(serve_bin(true, Some(&d.join("nope")), &target, None, None).is_err(), "a broken override does not fall through");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The bundle layouts: macOS beside the app binary, Linux (.deb and
    /// AppImage alike) in the resource dir. The env override is a dev knob.
    #[test]
    fn a_release_build_takes_the_bundles_copy_only() {
        let d = tmp("bundle");
        let exe = d.join("MacOS");
        let res = d.join("Resources");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&res).unwrap();
        let over = d.join("override");
        std::fs::write(&over, "x").unwrap();
        assert!(serve_bin(false, Some(&over), &d, Some(&exe), Some(&res)).is_err(), "release ignores WORKTREES_SERVE_BIN");
        let want = if cfg!(target_os = "macos") { exe.join("worktrees") } else { res.join("worktrees") };
        std::fs::write(&want, "x").unwrap();
        assert_eq!(serve_bin(false, Some(&over), &d, Some(&exe), Some(&res)).unwrap().0, want);
        let _ = std::fs::remove_dir_all(&d);
    }
}
