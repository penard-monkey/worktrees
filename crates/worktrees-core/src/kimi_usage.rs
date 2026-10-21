//! kimi's plan usage — read THROUGH kimi, which holds the sign-in, the way
//! `codex_usage` reads through `codex app-server`. worktrees never reads kimi's
//! OAuth token or credentials.
//!
//! Measured on kimi 2.2.0 (`docs/kimi-manual-checks.md`):
//!
//! - `kimi web --no-open --port 0` binds 127.0.0.1 only ("Network: off") and
//!   prints `Local: http://127.0.0.1:<port>/#token=<token>` when ready (~0.9s).
//! - `GET /api/v1/oauth/usage` with that bearer answers
//!   `{code, data: {kind: "ok", quota: {usages: {limit5h?, limit7d?,
//!   monthTotal?, monthCode?: {usedRatio, resetAt?}}}}}` or `{data: {kind:
//!   "error", message, status?}}`.
//! - The bearer is PERSISTENT — the same as `~/.kimi-code/server.token`, shared
//!   by every `kimi web`, a user's own included. It lives in this process's
//!   memory for one request and goes nowhere else: not into `Info`, not into
//!   a log line, not into an error (every failure is a fixed reason word).
//! - There is no single-instance lock (two servers on two ports coexist), so a
//!   user's own `kimi web` is never disturbed. The server forks nothing; it is
//!   still spawned in its own process group, and the group is sent SIGTERM,
//!   then SIGKILL, then reaped, on every path. A SIGKILL leaves a stale
//!   `server/instances/` record, which kimi's next start prunes itself.
//! - At most one probe per `TTL`/`FAILURE_FLOOR` (120s), failures included, so
//!   focus pulls cannot storm it; a probe already running is waited for. The
//!   cache lock is held across the probe (up to `BUDGET` + the 2s kill
//!   grace), so a kimi launch whose gate reads usage while the meter is
//!   pulling waits for that one probe — codex's shape, and a ROADMAP item for
//!   both.
//! - Loopback by construction AND by check: `--host 127.0.0.1` on argv, and a
//!   `Local:` host other than loopback, or a `Network:` line that is not `off`,
//!   ends the server before it is asked anything.
//! - Two windows nothing can close here, said so: if `kimi web` died between
//!   printing its port and our connect, another local process that grabbed
//!   that port would receive the bearer (local only, a sub-second window);
//!   and a worktrees process killed outright (SIGKILL — a panic still runs the
//!   `Drop`) orphans its `kimi web` on loopback until it is stopped. Codex's
//!   reader shares both.

use serde::Serialize;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TTL: u64 = 120;
/// The shortest wait after a failed probe; it doubles per failure, to 900s.
const FAILURE_FLOOR: u64 = 120;
const STALE: i64 = 1800;
const BUDGET: Duration = Duration::from_secs(12);
const OUTPUT_CAP: u64 = 64 * 1024;
const RESPONSE_CAP: u64 = 1024 * 1024;

/// One window, in `codex_usage::Limit`'s shape so the frontend adapts both the
/// same way.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Limit {
    id: String,
    bucket_id: String,
    bucket_label: String,
    window_role: String,
    window_minutes: Option<u64>,
    percent: f64,
    severity: String,
    resets_at: Option<i64>,
}

impl Limit {
    pub fn percent(&self) -> f64 {
        self.percent
    }
    pub fn severity(&self) -> &str {
        &self.severity
    }
    /// What the meter calls it: `5h`, `7d`, `month`, `code`.
    pub fn label(&self) -> &str {
        &self.window_role
    }
    pub fn resets_at(&self) -> Option<i64> {
        self.resets_at
    }
    /// Longer than a day: the weekly and monthly windows.
    pub fn is_weekly(&self) -> bool {
        self.window_minutes.is_none_or(|m| m > 24 * 60)
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Info {
    provider: &'static str,
    state: &'static str,
    source: &'static str,
    fetched_at: Option<i64>,
    retry_at: Option<i64>,
    reason: Option<&'static str>,
    limits: Vec<Limit>,
}

impl Info {
    pub fn limits(&self) -> &[Limit] {
        &self.limits
    }
    pub fn state(&self) -> &str {
        self.state
    }
    /// Why there is no fresh reading, as a fixed word (`timeout`, `expired`…).
    pub fn reason(&self) -> Option<&'static str> {
        self.reason
    }
    fn unavailable(state: &'static str, reason: &'static str) -> Self {
        Self { provider: "kimi", state, source: "unavailable", fetched_at: None, retry_at: None, reason: Some(reason), limits: vec![] }
    }
}

fn epoch() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

/// The windows kimi reports, in a fixed order: (key, label, minutes).
const WINDOWS: &[(&str, &str, Option<u64>)] = &[
    ("limit5h", "5h", Some(300)),
    ("limit7d", "7d", Some(10080)),
    ("monthTotal", "month", None),
    ("monthCode", "code", None),
];

/// The usage route's JSON body, as `Info`.
pub fn normalize(body: &serde_json::Value, now: i64) -> Info {
    let data = body.get("data").unwrap_or(body);
    match data.get("kind").and_then(|k| k.as_str()) {
        Some("ok") => {}
        Some("error") => {
            let status = data.get("status").and_then(|s| s.as_i64());
            return if matches!(status, Some(401 | 403)) {
                Info::unavailable("signed_out", "signed_out")
            } else {
                Info::unavailable("unavailable", "api_error")
            };
        }
        _ => return Info::unavailable("unavailable", "malformed"),
    }
    let Some(usages) = data.pointer("/quota/usages") else {
        return Info::unavailable("unavailable", "malformed");
    };
    let mut limits = Vec::new();
    for (key, label, minutes) in WINDOWS {
        let Some(w) = usages.get(*key) else { continue };
        let Some(ratio) = w.get("usedRatio").and_then(|r| r.as_f64()).filter(|r| r.is_finite() && *r >= 0.0) else {
            continue;
        };
        let percent = ratio * 100.0;
        limits.push(Limit {
            id: format!("kimi:{key}"),
            bucket_id: "kimi".into(),
            bucket_label: "Kimi".into(),
            window_role: (*label).into(),
            window_minutes: *minutes,
            percent,
            severity: if percent >= 100.0 {
                "over"
            } else if percent >= 80.0 {
                "warning"
            } else {
                "normal"
            }
            .into(),
            resets_at: w.get("resetAt").and_then(|t| t.as_str()).and_then(crate::sysclock::parse_iso8601),
        });
    }
    if limits.is_empty() {
        return Info::unavailable("unavailable", "no_limits");
    }
    Info { provider: "kimi", state: "ready", source: "kimi_web", fetched_at: Some(now), retry_at: None, reason: None, limits }
}

// ── the server ───────────────────────────────────────────────────────────────

/// The `kimi web` child. Dropping it ends the WHOLE group on every path —
/// success, timeout, refusal, a panic: SIGTERM so kimi removes its instance
/// record, SIGKILL after a grace, then a reap.
struct Server(Child);

impl Server {
    fn signal(&self, sig: i32) {
        // SAFETY: a plain kill(2) on the child's own process group (it was
        // spawned with `process_group(0)`, so its pgid is its pid).
        unsafe {
            libc::kill(-(self.0.id() as i32), sig);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.signal(libc::SIGTERM);
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            match self.0.try_wait() {
                Ok(Some(_)) => {
                    // The leader is gone; anything left in its group is not.
                    self.signal(libc::SIGKILL);
                    return;
                }
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    self.signal(libc::SIGKILL);
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                    return;
                }
            }
        }
    }
}

/// Where kimi said it is listening, parsed from its startup output.
#[derive(PartialEq)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

/// By hand, so `{ep:?}` — in a log line, an assert's failure message, a panic —
/// can never print the persistent bearer.
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint").field("port", &self.port).field("token", &"<redacted>").finish()
    }
}

/// Read `kimi web`'s startup lines until it says where it listens. `Err` is a
/// fixed reason word — never a line of the output, which carries the token.
/// A `Network:` line that is not `off` means the server is reachable from the
/// network, and is refused outright.
pub fn endpoint_from(lines: impl Iterator<Item = String>) -> Result<Endpoint, &'static str> {
    let mut found: Option<Endpoint> = None;
    for line in lines {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Network:") {
            if !rest.trim_start().starts_with("off") {
                return Err("not_loopback");
            }
            if found.is_some() {
                break;
            }
        }
        if let Some(rest) = t.strip_prefix("Local:") {
            let url = rest.trim();
            let after = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().find_map(|h| url.strip_prefix(h));
            let Some(after) = after else { return Err("unexpected_host") };
            let (port, frag) = after.split_once('/').unwrap_or((after, ""));
            let port: u16 = port.parse().map_err(|_| "no_port")?;
            let token = frag.strip_prefix("#token=").unwrap_or("").to_string();
            if token.is_empty() || !token.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
                return Err("no_token");
            }
            found = Some(Endpoint { port, token });
        }
    }
    found.ok_or("no_port")
}

/// One GET over loopback, with the bearer, read to EOF (`Connection: close`)
/// before `deadline` — ONE deadline for connect, write and every read, so a
/// server dripping a byte at a time cannot stretch it — and within
/// `RESPONSE_CAP`. Returns (status, body).
fn get(port: u16, path: &str, token: &str, deadline: Instant) -> Result<(u16, Vec<u8>), &'static str> {
    let left = || deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or("timeout");
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, left()?).map_err(|_| "connect")?;
    s.set_write_timeout(Some(left()?)).map_err(|_| "connect")?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).map_err(|_| "http")?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        s.set_read_timeout(Some(left()?)).map_err(|_| "timeout")?;
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() as u64 > RESPONSE_CAP {
                    return Err("response_too_large");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err("timeout"),
        }
    }
    parse_http(&raw)
}

/// (status, body) from a raw HTTP/1.1 response, chunked or not.
pub fn parse_http(raw: &[u8]) -> Result<(u16, Vec<u8>), &'static str> {
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("http")?;
    let head = String::from_utf8_lossy(&raw[..at]).to_string();
    let body = &raw[at + 4..];
    let status: u16 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("http")?;
    let chunked = head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    if !chunked {
        return Ok((status, body.to_vec()));
    }
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let nl = rest.windows(2).position(|w| w == b"\r\n").ok_or("http")?;
        let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..nl]).split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "http")?;
        rest = &rest[nl + 2..];
        if size == 0 {
            return Ok((status, out));
        }
        if rest.len() < size + 2 {
            return Err("http");
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

/// One probe: start `kimi web`, find it, ask it, end it. `binary` and `cwd`
/// are the seams the tests drive with a fake kimi.
pub fn probe(binary: &Path, cwd: &Path, budget: Duration) -> Info {
    let deadline = Instant::now() + budget;
    let left = || deadline.checked_duration_since(Instant::now());
    let spawned = crate::childenv::command(binary)
        .args(["web", "--no-open", "--host", "127.0.0.1", "--port", "0"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    let Ok(mut child) = spawned else { return Info::unavailable("unavailable", "spawn") };
    let Some(stdout) = child.stdout.take() else {
        drop(Server(child));
        return Info::unavailable("unavailable", "pipe");
    };
    let server = Server(child);
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout.take(OUTPUT_CAP));
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    // Lines as they come, until the budget runs out. The endpoint is known
    // once both `Local:` and `Network:` have been seen (kimi prints them in
    // that order), or at the first line that refuses it.
    let lines = std::iter::from_fn(|| {
        let wait = left()?;
        rx.recv_timeout(wait).ok()
    });
    let mut seen_network = false;
    let mut collected: Vec<String> = Vec::new();
    for line in lines {
        let is_network = line.trim_start().starts_with("Network:");
        collected.push(line);
        if is_network {
            seen_network = true;
            break;
        }
    }
    if !seen_network {
        drop(server);
        return Info::unavailable("unavailable", if left().is_none() { "timeout" } else { "no_port" });
    }
    let ep = match endpoint_from(collected.into_iter()) {
        Ok(ep) => ep,
        Err(reason) => {
            drop(server);
            return Info::unavailable("unavailable", reason);
        }
    };
    let answer = get(ep.port, "/api/v1/oauth/usage", &ep.token, deadline);
    drop(ep);
    drop(server);
    match answer {
        Ok((200, body)) => match serde_json::from_slice::<serde_json::Value>(&body) {
            Ok(v) => normalize(&v, epoch()),
            Err(_) => Info::unavailable("unavailable", "malformed"),
        },
        Ok((401 | 403, _)) => Info::unavailable("signed_out", "signed_out"),
        Ok(_) => Info::unavailable("unavailable", "http"),
        Err(reason) => Info::unavailable("unavailable", reason),
    }
}

// ── the cache ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Cache {
    key: String,
    good: Option<Info>,
    last: Option<Info>,
    next: Option<Instant>,
    failures: u32,
}

impl Cache {
    fn due(&self, clock: Instant) -> bool {
        self.next.is_none_or(|n| clock >= n)
    }

    /// The last answer, aged: past `STALE` it is unavailable, and with a
    /// window past its reset it is stale (the UI says "awaiting update" rather
    /// than paint a number that is no longer true).
    fn answer(&self, now: i64) -> Info {
        let mut out = self.last.clone().unwrap_or_else(|| Info::unavailable("unavailable", "no_limits"));
        if let Some(fetched) = out.fetched_at {
            if !(0..=STALE).contains(&(now - fetched)) {
                out.state = "unavailable";
                out.source = "unavailable";
                out.reason = Some("expired");
                out.limits.clear();
            } else if out.limits.iter().any(|l| l.resets_at.is_some_and(|t| t <= now)) {
                out.state = "stale";
            }
        }
        out
    }

    fn update(&mut self, mut out: Info, now: i64, clock: Instant) -> Info {
        let delay = if out.state == "ready" {
            self.failures = 0;
            self.good = Some(out.clone());
            TTL
        } else if matches!(out.state, "signed_out" | "missing_cli") || out.reason == Some("no_limits") {
            self.good = None;
            self.failures = 0;
            900
        } else {
            self.failures = self.failures.saturating_add(1);
            let delay = (FAILURE_FLOOR << self.failures.saturating_sub(1).min(3)).min(900);
            // A failed refresh keeps showing the last good reading, marked.
            if let Some(good) = self.good.as_ref().filter(|g| g.fetched_at.is_some_and(|t| (0..=STALE).contains(&(now - t)))) {
                let reason = out.reason;
                out = good.clone();
                out.state = "stale";
                out.source = "cached";
                out.reason = reason;
            }
            delay
        };
        self.next = Some(clock + Duration::from_secs(delay));
        out.retry_at = Some(now.saturating_add(delay as i64));
        self.last = Some(out);
        self.answer(now)
    }
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The neutral working directory the server starts in — never a place: kimi
/// reads the project config of where it starts (`codex_usage::probe_cwd`'s
/// reason).
pub fn probe_cwd() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(Path::new(&home).join(".cache/worktrees/kimi-usage"))
}

fn failure_level(reason: &str) -> &'static str {
    match reason {
        "missing_cli" | "signed_out" => "info",
        _ => "warn",
    }
}

fn read_cached(state: &Mutex<Option<Cache>>, key: String, fetch: impl FnOnce() -> Info) -> Info {
    // Held across the probe: a second caller waits for the first one's answer
    // instead of starting a second server.
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(Cache::default);
    if cache.key != key {
        *cache = Cache { key, ..Cache::default() };
    }
    if !cache.due(Instant::now()) {
        return cache.answer(epoch());
    }
    let info = fetch();
    if let Some(reason) = info.reason {
        // A reason WORD — never output, never the token.
        crate::logsink::log(failure_level(reason), &format!("kimi_usage: {reason}"));
    }
    cache.update(info, epoch(), Instant::now())
}

/// kimi's plan usage now, from the cache when it is fresh.
pub fn read() -> Info {
    let binary = crate::kimi::kimi_bin();
    let stamp = binary.as_ref().and_then(|p| p.metadata().ok()).and_then(|m| m.modified().ok());
    let key = format!("{binary:?}|{stamp:?}");
    read_cached(&CACHE, key, || match (binary, probe_cwd()) {
        (None, _) => Info::unavailable("missing_cli", "missing_cli"),
        (Some(_), None) => Info::unavailable("unavailable", "runtime_directory"),
        (Some(bin), Some(cwd)) => {
            if std::fs::create_dir_all(&cwd).is_err() {
                return Info::unavailable("unavailable", "runtime_directory");
            }
            probe(&bin, &cwd, BUDGET)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const OK: &str = r#"{"code":0,"msg":"success","data":{"kind":"ok","quota":{"usages":{"limit5h":{"usedRatio":0.0041,"resetAt":"2026-10-20T02:36:16Z"},"monthTotal":{"usedRatio":0.0003,"resetAt":"2026-11-19T01:37:17Z"},"monthCode":{"usedRatio":0.0003,"resetAt":"2026-11-19T01:37:17Z"}},"extraUsage":null}},"request_id":"x"}"#;

    #[test]
    fn the_measured_answer_reads_as_three_windows_in_order() {
        let i = normalize(&serde_json::from_str(OK).unwrap(), 100);
        assert_eq!(i.state, "ready");
        let labels: Vec<&str> = i.limits.iter().map(|l| l.label()).collect();
        assert_eq!(labels, ["5h", "month", "code"], "limit7d is absent on this plan, so not shown");
        assert!((i.limits[0].percent - 0.41).abs() < 1e-9);
        assert_eq!(i.limits[0].resets_at, crate::sysclock::parse_iso8601("2026-10-20T02:36:16Z"));
        assert!(!i.limits[0].is_weekly() && i.limits[1].is_weekly());
        assert_eq!(i.limits[0].severity, "normal");
    }

    #[test]
    fn errors_and_odd_shapes_are_honest_states_never_a_zero() {
        let e = |s: &str| normalize(&serde_json::from_str(s).unwrap(), 1);
        assert_eq!(e(r#"{"data":{"kind":"error","message":"no","status":401}}"#).state, "signed_out");
        assert_eq!(e(r#"{"data":{"kind":"error","message":"boom"}}"#).reason, Some("api_error"));
        assert_eq!(e(r#"{"data":{"kind":"ok","quota":{"usages":{}}}}"#).reason, Some("no_limits"));
        assert_eq!(e(r#"{"data":{"kind":"weird"}}"#).reason, Some("malformed"));
        assert!(e(r#"{"data":{"kind":"ok","quota":{"usages":{}}}}"#).limits.is_empty());
        let over = e(r#"{"data":{"kind":"ok","quota":{"usages":{"limit5h":{"usedRatio":1.0}}}}}"#);
        assert_eq!((over.limits[0].severity.as_str(), over.limits[0].resets_at), ("over", None));
    }

    #[test]
    fn the_endpoint_is_found_on_loopback_and_refused_anywhere_else() {
        let l = |v: &[&str]| endpoint_from(v.iter().map(|s| s.to_string()));
        let ok = l(&["  Kimi server ready  2.2.0", "  Local:    http://127.0.0.1:60220/#token=abc-DEF_1", "  Network:  off  use --host to enable"]);
        assert_eq!(ok, Ok(Endpoint { port: 60220, token: "abc-DEF_1".into() }));
        assert_eq!(l(&["  Local:    http://127.0.0.1:1/#token=t", "  Network:  http://192.168.1.4:1/#token=t"]), Err("not_loopback"));
        assert_eq!(l(&["  Local:    http://0.0.0.0:1/#token=t"]), Err("unexpected_host"));
        assert_eq!(l(&["  Local:    http://localhost:7/#token=t", "  Network:  off"]).map(|e| e.port), Ok(7));
        assert_eq!(l(&["  Local:    http://[::1]:8/#token=t", "  Network:  off"]).map(|e| e.port), Ok(8));
        assert_eq!(l(&["  Local:    http://127.0.0.1:1/"]), Err("no_token"));
        assert_eq!(l(&["  Local:    http://127.0.0.1:1/#token=a b"]), Err("no_token"));
        assert_eq!(l(&["  nothing"]), Err("no_port"));
    }

    #[test]
    fn an_endpoint_never_prints_its_token() {
        let ep = Endpoint { port: 1, token: "SECRET-TOKEN-x1".into() };
        let shown = format!("{ep:?} {:?}", Ok::<_, ()>(&ep));
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(shown.contains("redacted"), "{shown}");
    }

    #[test]
    fn http_reads_plain_and_chunked_bodies() {
        let plain = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
        assert_eq!(parse_http(plain), Ok((200, b"{}".to_vec())));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n";
        assert_eq!(parse_http(chunked), Ok((200, b"{\"a\":1}".to_vec())));
        assert_eq!(parse_http(b"HTTP/1.1 401 Unauthorized\r\n\r\n").map(|r| r.0), Ok(401));
        assert_eq!(parse_http(b"garbage"), Err("http"));
    }

    // ── the lifecycle, against a fake `kimi` ─────────────────────────────────

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kimi-usage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A fake `kimi` that records its pid and argv, prints `body`, then waits
    /// until signalled — and marks a SIGTERM, so "terminated politely before
    /// it was killed" is a fact the test can read, as is whether it survives.
    fn fake(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("kimi");
        let d = dir.display();
        std::fs::write(
            &p,
            format!("#!/bin/sh\necho $$ > '{d}/pid'\necho \"$@\" > '{d}/args'\ntrap 'touch \"{d}/term\"; exit 0' TERM\n{body}\nsleep 60 &\nwait\n"),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn alive(dir: &Path) -> bool {
        let Ok(pid) = std::fs::read_to_string(dir.join("pid")) else { return false };
        let pid: i32 = pid.trim().parse().unwrap();
        // SAFETY: signal 0 only asks whether the pid exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn wait_dead(dir: &Path) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if !alive(dir) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// A loopback server that answers one request with `response` and hands
    /// back what it was sent.
    fn serve(response: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            s.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (port, h)
    }

    #[test]
    fn a_probe_reads_usage_and_leaves_no_kimi_running() {
        let d = tmp("ok");
        let body: &'static str = Box::leak(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{OK}", OK.len()).into_boxed_str());
        let (port, server) = serve(body);
        let bin = fake(&d, &format!("echo '  Local:    http://127.0.0.1:{port}/#token=SECRET-TOKEN-x1'\necho '  Network:  off'"));
        let info = probe(&bin, &d, Duration::from_secs(5));
        assert_eq!(info.state, "ready", "{info:?}");
        assert_eq!(info.limits.len(), 3);
        let req = server.join().unwrap();
        assert!(req.starts_with("GET /api/v1/oauth/usage HTTP/1.1\r\n"), "{req}");
        assert!(req.contains("Authorization: Bearer SECRET-TOKEN-x1\r\n"));
        // The token reaches the request and nothing else.
        assert!(!serde_json::to_string(&info).unwrap().contains("SECRET"));
        assert!(wait_dead(&d), "kimi web outlived a successful probe");
        // SIGTERM first — kimi removes its instance record on it — not a
        // straight SIGKILL.
        assert!(d.join("term").exists(), "the server was never sent SIGTERM");
        // Loopback by construction, not only by check.
        let args = std::fs::read_to_string(d.join("args")).unwrap();
        assert!(args.contains("web --no-open --host 127.0.0.1 --port 0"), "{args}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_timed_out_probe_leaves_no_kimi_running() {
        let d = tmp("timeout");
        // Never says where it listens.
        let bin = fake(&d, "echo '  Kimi server ready'");
        let t0 = Instant::now();
        let info = probe(&bin, &d, Duration::from_millis(800));
        assert_eq!((info.state, info.reason), ("unavailable", Some("timeout")));
        assert!(t0.elapsed() < Duration::from_secs(5), "bounded by the budget (plus the kill grace)");
        assert!(wait_dead(&d), "kimi web outlived a timed-out probe");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_server_reachable_from_the_network_is_refused_unasked_and_ended() {
        let d = tmp("lan");
        // Non-blocking and never accepted by a thread: after the probe, an
        // `accept` that would block proves nobody connected — "unasked".
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.set_nonblocking(true).unwrap();
        let port = l.local_addr().unwrap().port();
        let bin = fake(&d, &format!("echo '  Local:    http://127.0.0.1:{port}/#token=t'\necho '  Network:  http://192.168.1.9:{port}/#token=t'"));
        let info = probe(&bin, &d, Duration::from_secs(5));
        assert_eq!(info.reason, Some("not_loopback"));
        assert_eq!(l.accept().map(|_| ()).unwrap_err().kind(), std::io::ErrorKind::WouldBlock, "it was asked");
        assert!(wait_dead(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn signed_out_is_its_own_state() {
        let d = tmp("401");
        let (port, _server) = serve("HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n");
        let bin = fake(&d, &format!("echo '  Local:    http://127.0.0.1:{port}/#token=t'\necho '  Network:  off'"));
        assert_eq!(probe(&bin, &d, Duration::from_secs(5)).state, "signed_out");
        assert!(wait_dead(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    // ── the cache ───────────────────────────────────────────────────────────

    fn ok(at: i64) -> Info {
        normalize(&serde_json::from_str(OK).unwrap(), at)
    }

    #[test]
    fn one_probe_per_120s_success_or_failure() {
        let mut c = Cache::default();
        let clock = Instant::now();
        c.update(ok(100), 100, clock);
        assert!(!c.due(clock + Duration::from_secs(119)));
        assert!(c.due(clock + Duration::from_secs(120)));
        c.update(Info::unavailable("unavailable", "timeout"), 220, clock);
        assert!(!c.due(clock + Duration::from_secs(119)), "a failure waits at least as long");
        assert!(c.due(clock + Duration::from_secs(120)));
        c.update(Info::unavailable("unavailable", "timeout"), 220, clock);
        assert!(!c.due(clock + Duration::from_secs(239)), "and backs off from there");
    }

    #[test]
    fn a_failed_refresh_shows_the_last_reading_as_stale_and_old_data_expires() {
        let mut c = Cache::default();
        let clock = Instant::now();
        c.update(ok(1000), 1000, clock);
        let after = c.update(Info::unavailable("unavailable", "timeout"), 1100, clock);
        assert_eq!((after.state, after.source, after.reason), ("stale", "cached", Some("timeout")));
        assert_eq!(after.limits.len(), 3);
        let old = c.answer(1000 + STALE + 1);
        assert_eq!((old.state, old.reason), ("unavailable", Some("expired")));
        assert!(old.limits.is_empty(), "never a number past its age");
    }

    /// Eight callers at once (the meter's focus pull, the poll, a launch's
    /// gate…) start ONE server: the lock is held across the probe. Copied
    /// from codex's test — a sequential version passes with the lock released
    /// around the fetch, which is exactly the bug.
    #[test]
    fn concurrent_callers_share_one_probe_and_a_new_binary_invalidates_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let cache = Arc::new(Mutex::new(None));
        let calls = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                let calls = calls.clone();
                std::thread::spawn(move || {
                    read_cached(&cache, "bin-a".into(), || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(10));
                        ok(epoch())
                    })
                })
            })
            .collect();
        for t in threads {
            // Every caller got the one probe's windows (their `state` depends
            // on the fixture's reset times against the clock, not on sharing).
            assert_eq!(t.join().unwrap().limits.len(), 3);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        read_cached(&cache, "bin-b".into(), || {
            calls.fetch_add(1, Ordering::SeqCst);
            ok(epoch())
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn expected_states_are_info_and_faults_warn() {
        assert_eq!(failure_level("signed_out"), "info");
        assert_eq!(failure_level("missing_cli"), "info");
        assert_eq!(failure_level("timeout"), "warn");
    }
}
