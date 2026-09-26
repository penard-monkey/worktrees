//! Account limits only. Codex owns authentication and its runtime; no rollouts,
//! conversations, configuration writes, or model requests belong in this reader.
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TTL: u64 = 120;
const STALE: i64 = 1800;
const RESPONSE_CAP: u64 = 1024 * 1024;
const ATTEMPT: Duration = Duration::from_secs(13);

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
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
    fn unavailable(state: &'static str, reason: &'static str) -> Self {
        Self {
            provider: "codex",
            state,
            source: "unavailable",
            fetched_at: None,
            retry_at: None,
            reason: Some(reason),
            limits: vec![],
        }
    }
}

fn epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn normalize(value: &Value, now: i64) -> Info {
    let mut buckets: Vec<(String, &Value)> =
        match value.get("rateLimitsByLimitId").and_then(Value::as_object) {
            Some(map) if !map.is_empty() => map.iter().map(|(id, v)| (id.clone(), v)).collect(),
            _ => value
                .get("rateLimits")
                .map(|v| {
                    vec![(
                        v.get("limitId")
                            .and_then(Value::as_str)
                            .unwrap_or("codex")
                            .to_string(),
                        v,
                    )]
                })
                .unwrap_or_default(),
        };
    buckets.sort_by(|a, b| (a.0 != "codex", &a.0).cmp(&(b.0 != "codex", &b.0)));
    let mut limits = vec![];
    for (id, bucket) in buckets {
        let label = bucket
            .get("limitName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(if id == "codex" { "Codex" } else { &id });
        for role in ["primary", "secondary"] {
            let Some(window) = bucket.get(role) else {
                continue;
            };
            let Some(percent) = window
                .get("usedPercent")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0)
            else {
                continue;
            };
            limits.push(Limit {
                id: format!("{id}:{role}"),
                bucket_id: id.clone(),
                bucket_label: label.to_string(),
                window_role: role.into(),
                window_minutes: window
                    .get("windowDurationMins")
                    .and_then(Value::as_u64)
                    .filter(|n| *n > 0),
                percent,
                severity: if percent >= 100.0 {
                    "over"
                } else if percent >= 80.0 {
                    "warning"
                } else {
                    "normal"
                }
                .into(),
                resets_at: window
                    .get("resetsAt")
                    .and_then(Value::as_i64)
                    .filter(|n| *n > 0),
            });
        }
    }
    if limits.is_empty() {
        return Info::unavailable("unavailable", "no_limits");
    }
    Info {
        provider: "codex",
        state: "ready",
        source: "app_server",
        fetched_at: Some(now),
        retry_at: None,
        reason: None,
        limits,
    }
}

// The guard also reaps on parse/write errors and unwinding. Never leave a second
// Codex service running after a usage read. Reader threads finish when pipes close.
struct Server(Child);
impl Server {
    fn kill_group(&self) {
        // Every server is spawned into its own process group, including children.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.0.stdin.take();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            match self.0.try_wait() {
                Ok(Some(_)) => {
                    self.kill_group();
                    return;
                }
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    self.kill_group();
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                    return;
                }
            }
        }
    }
}

#[derive(Debug)]
struct Fault {
    reason: &'static str,
    retry_secs: Option<u64>,
}
impl From<&'static str> for Fault {
    fn from(reason: &'static str) -> Self {
        Self {
            reason,
            retry_secs: None,
        }
    }
}

struct Rpc {
    server: Server,
    lines: mpsc::Receiver<Result<Vec<u8>, &'static str>>,
    deadline: Instant,
}
impl Rpc {
    fn open(binary: &Path, cwd: &Path, home: &Path, budget: Duration) -> Result<Self, Fault> {
        let deadline = Instant::now() + budget;
        let mut server = Server(
            Command::new(binary)
                .args(["app-server", "--listen", "stdio://"])
                .current_dir(cwd)
                .env("CODEX_HOME", home)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .map_err(|_| "spawn")?,
        );
        let stdout = server.0.stdout.take().ok_or("pipe")?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            // Bound the ENTIRE exchange, including notifications and malformed data.
            let mut reader = BufReader::new(stdout.take(RESPONSE_CAP + 1));
            let mut total = 0;
            loop {
                let mut line = vec![];
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n as u64;
                        if total > RESPONSE_CAP {
                            let _ = tx.send(Err("response_too_large"));
                            break;
                        }
                        if tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(Err("read"));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            server,
            lines,
            deadline,
        })
    }
    fn send(&mut self, value: Value) -> Result<(), Fault> {
        let stdin = self.server.0.stdin.as_mut().ok_or("pipe")?;
        writeln!(stdin, "{value}").map_err(|_| Fault::from("write"))
    }
    fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value, Fault> {
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            let remaining = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or("timeout")?;
            let line = self
                .lines
                .recv_timeout(remaining)
                .map_err(|e| match e {
                    mpsc::RecvTimeoutError::Timeout => "timeout",
                    _ => "eof",
                })?
                .map_err(Fault::from)?;
            let v: Value = serde_json::from_slice(&line).map_err(|_| "malformed")?;
            if v.get("method").is_some() {
                if let Some(request_id) = v.get("id") {
                    // One small refusal, then close. An untrusted stream of
                    // server requests must not fill stdin and block a write.
                    if request_id.is_u64() || request_id.as_str().is_some_and(|id| id.len() <= 128)
                    {
                        self.send(json!({"id":request_id,"error":{"code":-32601,"message":"Unsupported request"}}))?;
                    }
                    return Err("unexpected_request".into());
                }
                continue;
            }
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = v.get("error") {
                return Err(Fault {
                    reason: if error.get("code").and_then(Value::as_i64) == Some(-32601) {
                        "unsupported_protocol"
                    } else {
                        "rpc"
                    },
                    retry_secs: error
                        .pointer("/data/retryAfterSeconds")
                        .and_then(Value::as_u64),
                });
            }
            return v
                .get("result")
                .cloned()
                .ok_or_else(|| Fault::from("malformed"));
        }
    }
}

struct Observation {
    info: Info,
    identity: Option<String>,
    auth: Option<String>,
    retry_secs: Option<u64>,
}
fn probe(binary: &Path, cwd: &Path, home: &Path, budget: Duration) -> Observation {
    let mut auth = None;
    let result = (|| -> Result<(Info, Option<String>), Fault> {
        let mut rpc = Rpc::open(binary, cwd, home, budget)?;
        rpc.request(
            1,
            "initialize",
            json!({"clientInfo":{"name":"worktrees_usage","title":"Worktrees plan usage",
            "version":env!("CARGO_PKG_VERSION")},"capabilities":null}),
        )?;
        rpc.send(json!({"method":"initialized"}))?;
        let account = rpc.request(2, "account/read", json!({"refreshToken":false}))?;
        let a = account.get("account").ok_or("malformed")?;
        auth = Some(a.to_string()); // memory only; never return/log email or identity
        if a.is_null() {
            return Ok((Info::unavailable("signed_out", "signed_out"), None));
        }
        match a.get("type").and_then(Value::as_str) {
            Some("chatgpt") => (),
            Some(_) => {
                return Ok((
                    Info::unavailable("unsupported_auth", "unsupported_auth"),
                    None,
                ))
            }
            None => return Err("malformed".into()),
        }
        let value = rpc.request(
            3,
            "account/rateLimits/read",
            json!({"excludeResetCreditDetails":true}),
        )?;
        let identity = value
            .get("accountId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from);
        Ok((normalize(&value, epoch()), identity))
    })();
    match result {
        Ok((info, identity)) => Observation {
            info,
            identity,
            auth,
            retry_secs: None,
        },
        Err(fault) => Observation {
            info: Info::unavailable("unavailable", fault.reason),
            identity: None,
            auth,
            retry_secs: fault.retry_secs,
        },
    }
}

#[derive(Default)]
struct Cache {
    key: String,
    good: Option<Info>,
    identity: Option<String>,
    auth: Option<String>,
    last: Option<Info>,
    next: Option<Instant>,
    failures: u32,
}
impl Cache {
    fn due(&self, clock: Instant) -> bool {
        self.next.is_none_or(|next| clock >= next)
    }

    fn answer(&self, now: i64) -> Info {
        let mut out = self
            .last
            .clone()
            .unwrap_or_else(|| Info::unavailable("unavailable", "no_limits"));
        if let Some(fetched) = out.fetched_at {
            if !(0..=STALE).contains(&(now - fetched)) {
                out.state = "unavailable";
                out.source = "unavailable";
                out.reason = Some("expired");
                out.limits.clear();
            } else if out
                .limits
                .iter()
                .any(|l| l.resets_at.is_some_and(|t| t <= now))
            {
                // Keep the row identity so the UI can say "Awaiting update";
                // neither the compact selection nor detail paints expired values.
                out.state = "stale";
            }
        }
        out
    }
    fn update(&mut self, o: Observation, now: i64, clock: Instant) -> Info {
        if o.auth
            .as_ref()
            .is_some_and(|a| self.auth.as_ref().is_some_and(|old| a != old))
            || o.identity
                .as_ref()
                .is_some_and(|id| self.identity.as_ref().is_some_and(|old| id != old))
        {
            self.good = None;
            self.identity = None;
        }
        if o.auth.is_some() {
            self.auth = o.auth;
        }
        let mut out = o.info;
        let delay = if out.state == "ready" {
            self.failures = 0;
            self.identity = o.identity;
            self.good = Some(out.clone());
            TTL
        } else if matches!(out.state, "signed_out" | "unsupported_auth" | "missing_cli")
            || out.reason == Some("no_limits")
        {
            self.good = None;
            self.identity = None;
            self.failures = 0;
            if out.state == "signed_out" {
                180
            } else {
                900
            }
        } else {
            self.failures = self.failures.saturating_add(1);
            let delay = if out.reason == Some("unsupported_protocol") {
                900
            } else {
                (60u64 << self.failures.saturating_sub(1).min(4)).min(900)
            };
            if self.identity.is_some() {
                if let Some(good) = self.good.as_ref().filter(|g| {
                    g.fetched_at
                        .is_some_and(|t| (0..=STALE).contains(&(now - t)))
                }) {
                    let reason = out.reason;
                    out = good.clone();
                    out.state = "stale";
                    out.source = "cached";
                    out.reason = reason;
                } else {
                    out.fetched_at = self.good.as_ref().and_then(|g| g.fetched_at);
                }
            }
            delay.max(o.retry_secs.unwrap_or(0))
        };
        // Avoid overflow from an untrusted retry delay; a day is a conservative
        // unavailable horizon even for a nonsensical upstream value.
        let delay = delay.min(86400);
        self.next = Some(clock + Duration::from_secs(delay));
        out.retry_at = Some(now.saturating_add(delay as i64));
        self.last = Some(out);
        self.answer(now)
    }
}
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn executable() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|p| p.is_absolute())
        .map(|p| p.join("codex"))
        .find(|p| {
            use std::os::unix::fs::PermissionsExt;
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

pub fn read(cwd: PathBuf) -> Info {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
        });
    let binary = executable();
    let stamp = binary
        .as_ref()
        .and_then(|p| p.metadata().ok())
        .and_then(|m| m.modified().ok());
    let key = format!("{home:?}|{binary:?}|{stamp:?}");
    read_cached(&CACHE, key, || match binary {
        None => Observation {
            info: Info::unavailable("missing_cli", "missing_cli"),
            identity: None,
            auth: None,
            retry_secs: None,
        },
        Some(binary) if std::fs::create_dir_all(&cwd).is_ok() => {
            probe(&binary, &cwd, &home, ATTEMPT)
        }
        Some(_) => Observation {
            info: Info::unavailable("unavailable", "runtime_directory"),
            identity: None,
            auth: None,
            retry_secs: None,
        },
    })
}

fn failure_level(reason: &str) -> &'static str {
    match reason {
        "missing_cli" | "signed_out" | "unsupported_auth" => "info",
        _ => "warn",
    }
}

fn read_cached(
    state: &Mutex<Option<Cache>>,
    key: String,
    fetch: impl FnOnce() -> Observation,
) -> Info {
    // The lock is held only on a blocking worker. Waiting callers share the
    // completed observation; no check-then-spawn race on focus/poll overlap.
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(Cache::default);
    if cache.key != key {
        *cache = Cache {
            key,
            ..Cache::default()
        };
    }
    let now = epoch();
    if !cache.due(Instant::now()) {
        return cache.answer(now);
    }
    let observation = fetch();
    if let Some(reason) = observation.info.reason {
        super::applog(failure_level(reason), &format!("codex_usage: {reason}"));
    }
    cache.update(observation, epoch(), Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn expected_account_states_are_info_but_protocol_failures_warn() {
        for reason in ["missing_cli", "signed_out", "unsupported_auth"] {
            assert_eq!(failure_level(reason), "info");
        }
        for reason in ["rpc", "timeout", "malformed", "unexpected_request"] {
            assert_eq!(failure_level(reason), "warn");
        }
    }

    #[test]
    fn success_and_failure_ttl_boundaries_use_monotonic_time() {
        let mut cache = Cache::default();
        let clock = Instant::now();
        cache.update(good(100, Some("a")), 100, clock);
        assert!(!cache.due(clock + Duration::from_secs(119)));
        assert!(cache.due(clock + Duration::from_secs(120)));
        cache.update(failure(), 220, clock);
        assert!(!cache.due(clock + Duration::from_secs(59)));
        assert!(cache.due(clock + Duration::from_secs(60)));
    }

    fn window(percent: f64, minutes: Option<u64>, reset: Option<i64>) -> Value {
        json!({"usedPercent":percent,"windowDurationMins":minutes,"resetsAt":reset})
    }
    fn good(at: i64, identity: Option<&str>) -> Observation {
        Observation {
            info: normalize(
                &json!({"rateLimits":{"limitId":"codex",
            "primary":window(48.5,Some(300),Some(at+2000)),"secondary":window(21.0,Some(10080),None)}}),
                at,
            ),
            identity: identity.map(String::from),
            auth: Some("account-a".into()),
            retry_secs: None,
        }
    }
    fn failure() -> Observation {
        Observation {
            info: Info::unavailable("unavailable", "rpc"),
            identity: None,
            auth: None,
            retry_secs: None,
        }
    }
    #[test]
    fn weekly_primary_and_nullable_secondary_are_not_fixed_windows() {
        let info = normalize(
            &json!({"rateLimits":{"primary":window(24.0,Some(10080),None),"secondary":null}}),
            10,
        );
        assert_eq!(info.limits.len(), 1);
        assert_eq!(info.limits[0].window_minutes, Some(10080));
        assert_eq!(info.limits[0].resets_at, None);
    }
    #[test]
    fn map_wins_without_duplicates_and_sorts_main_first() {
        let bucket = json!({"primary":window(8.0,Some(90),Some(100))});
        let info = normalize(
            &json!({"rateLimits":bucket,"rateLimitsByLimitId":{"z":bucket,"codex":bucket,"a":bucket}}),
            10,
        );
        assert_eq!(
            info.limits
                .iter()
                .map(|l| l.bucket_id.as_str())
                .collect::<Vec<_>>(),
            ["codex", "a", "z"]
        );
        for map in [Value::Null, json!({})] {
            assert_eq!(
                normalize(&json!({"rateLimits":bucket,"rateLimitsByLimitId":map}), 10)
                    .limits
                    .len(),
                1
            );
        }
    }
    #[test]
    fn invalid_window_does_not_erase_valid_sibling() {
        for invalid in [
            json!({}),
            json!({"usedPercent":-1}),
            json!({"usedPercent":"NaN"}),
            Value::Null,
        ] {
            let info = normalize(
                &json!({"rateLimits":{"primary":invalid,"secondary":window(104.5,None,Some(-1))}}),
                10,
            );
            assert_eq!(info.limits.len(), 1);
            assert_eq!(info.limits[0].percent, 104.5);
            assert_eq!(info.limits[0].severity, "over");
            assert_eq!(info.limits[0].window_minutes, None);
            assert_eq!(info.limits[0].resets_at, None);
        }
        assert_eq!(normalize(&json!({}), 10).reason, Some("no_limits"));
    }
    #[test]
    fn severity_boundaries_preserve_fractional_usage() {
        for (percent, severity) in [
            (79.9, "normal"),
            (80.0, "warning"),
            (99.9, "warning"),
            (100.0, "over"),
        ] {
            let info = normalize(
                &json!({"rateLimits":{"primary":window(percent,None,None)}}),
                10,
            );
            assert_eq!(info.limits[0].severity, severity);
            assert_eq!(info.limits[0].percent, percent);
        }
    }
    #[test]
    fn failed_refresh_keeps_original_timestamp_until_inclusive_ceiling() {
        let mut cache = Cache::default();
        let clock = Instant::now();
        cache.update(good(100, Some("a")), 100, clock);
        let stale = cache.update(failure(), 220, clock + Duration::from_secs(120));
        assert_eq!(stale.state, "stale");
        assert_eq!(stale.fetched_at, Some(100));
        assert_eq!(cache.answer(1900).limits.len(), 2);
        assert!(cache.answer(1901).limits.is_empty());
        assert_eq!(cache.answer(1901).fetched_at, Some(100));
        assert_eq!(cache.answer(99).state, "unavailable");
    }
    #[test]
    fn reset_is_checked_during_positive_ttl() {
        let mut cache = Cache::default();
        let clock = Instant::now();
        let mut o = good(100, Some("a"));
        o.info.limits[0].resets_at = Some(105);
        cache.update(o, 100, clock);
        assert_eq!(cache.answer(104).state, "ready");
        assert_eq!(cache.answer(105).state, "stale");
        assert_eq!(cache.answer(105).limits.len(), 2); // row retained for Awaiting update
    }
    #[test]
    fn backoff_grows_success_resets_and_server_delay_is_respected() {
        let mut cache = Cache::default();
        let clock = Instant::now();
        for delay in [60, 120, 240, 480, 900, 900] {
            assert_eq!(
                cache.update(failure(), 100, clock).retry_at,
                Some(100 + delay)
            );
        }
        assert_eq!(
            cache.update(good(100, Some("a")), 100, clock).retry_at,
            Some(220)
        );
        assert_eq!(cache.update(failure(), 220, clock).retry_at, Some(280));
        let mut f = failure();
        f.retry_secs = Some(2000);
        assert_eq!(cache.update(f, 300, clock).retry_at, Some(2300));
    }
    #[test]
    fn auth_change_and_missing_identity_do_not_reuse_another_accounts_reading() {
        let clock = Instant::now();
        let mut cache = Cache::default();
        cache.update(good(100, None), 100, clock);
        assert!(cache.update(failure(), 220, clock).limits.is_empty());
        cache.update(good(100, Some("a")), 100, clock);
        let mut f = failure();
        f.auth = Some("account-b".into());
        assert!(cache.update(f, 220, clock).limits.is_empty());
        cache.update(good(100, Some("a")), 100, clock);
        let mut logout = failure();
        logout.info = Info::unavailable("signed_out", "signed_out");
        assert!(cache.update(logout, 220, clock).limits.is_empty());
        assert!(cache.good.is_none());
    }
    #[test]
    fn concurrent_callers_share_one_attempt_and_home_changes_invalidate_it() {
        let cache = Arc::new(Mutex::new(None));
        let calls = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                let calls = calls.clone();
                std::thread::spawn(move || {
                    read_cached(&cache, "home-a".into(), || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(10));
                        good(epoch(), Some("a"))
                    })
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap().state, "ready");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        read_cached(&cache, "home-b".into(), || {
            calls.fetch_add(1, Ordering::SeqCst);
            good(epoch(), Some("b"))
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static SEQ: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "wt-codex-usage-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn script(&self, body: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let p = self.0.join("codex");
            std::fs::write(&p, format!("#!/usr/bin/env python3\n{body}")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
            p
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn fake_cli_checks_exact_read_only_protocol_cwd_home_and_clean_shutdown() {
        let tmp = Scratch::new();
        let binary=tmp.script(r#"import json,os,sys
assert sys.argv[1:]==['app-server','--listen','stdio://']
assert os.path.realpath(os.environ['CODEX_HOME'])==os.getcwd()
expected=['initialize','initialized','account/read','account/rateLimits/read']
for method in expected:
    r=json.loads(sys.stdin.readline())
    assert r['method']==method
    if method=='initialized': continue
    if method=='account/read':
        assert r['params']=={'refreshToken':False}
        result={'account':{'type':'chatgpt','email':'private@example.invalid'}}
    elif method=='account/rateLimits/read':
        assert r['params']=={'excludeResetCreditDetails':True}
        result={'accountId':'private-account','rateLimits':{'primary':{'usedPercent':24,'windowDurationMins':10080,'resetsAt':None}}}
    else: result={}
    print(json.dumps({'method':'notification','params':{}}),flush=True)
    print(json.dumps({'id':r['id'],'result':result}),flush=True)
assert sys.stdin.read()==''
open('exited','w').write('yes')
"#);
        let out = probe(&binary, &tmp.0, &tmp.0, Duration::from_secs(3));
        assert_eq!(out.info.state, "ready");
        assert!(tmp.0.join("exited").exists());
        // Exercise the cache boundary that holds identity, not just normalize().
        let mut cache = Cache::default();
        let now = epoch();
        let ready = cache.update(out, now, Instant::now());
        assert_eq!(cache.identity.as_deref(), Some("private-account"));
        assert!(cache
            .auth
            .as_ref()
            .unwrap()
            .contains("private@example.invalid"));
        let stale = cache.update(failure(), now + 120, Instant::now());
        assert_eq!(stale.state, "stale");
        for info in [ready, stale] {
            let text = serde_json::to_string(&info).unwrap();
            assert!(!text.contains("private@example.invalid"));
            assert!(!text.contains("private-account"));
        }
    }
    #[test]
    fn drop_and_timeout_close_descendant_sockets_even_after_leader_exit() {
        use std::net::TcpStream;
        // A listening descendant is positive evidence of a live process. Testing
        // the socket closing avoids treating a not-yet-reaped zombie as alive.
        for leader_exits in [false, true] {
            let tmp = Scratch::new();
            let binary = tmp.script(&format!(
                r#"import os,socket,sys,time
pid=os.fork()
if pid==0:
 s=socket.socket();s.bind(('127.0.0.1',0));s.listen()
 with open('ready','w') as f: f.write(str(os.getpid())+' '+str(s.getsockname()[1]))
 s.settimeout(.1)
 until=time.monotonic()+60
 while time.monotonic()<until:
  try:
   connection,_=s.accept();connection.close()
  except socket.timeout: pass
else:
 while not os.path.exists('ready'): time.sleep(.01)
 if {leader_exits}: sys.exit(0)
 time.sleep(60)
"#,
                leader_exits = if leader_exits { "True" } else { "False" }
            ));
            let mut rpc = Rpc::open(&binary, &tmp.0, &tmp.0, Duration::from_secs(2)).unwrap();
            let until = Instant::now() + Duration::from_secs(2);
            let ready = loop {
                if let Ok(text) = std::fs::read_to_string(tmp.0.join("ready")) {
                    if text.split_whitespace().count() == 2 {
                        break text;
                    }
                }
                assert!(Instant::now() < until, "fake descendant did not start");
                std::thread::sleep(Duration::from_millis(10));
            };
            let parts: Vec<_> = ready.split_whitespace().collect();
            let pid: i32 = parts[0].parse().unwrap();
            let address = format!("127.0.0.1:{}", parts[1]);
            assert!(TcpStream::connect(&address).is_ok());
            if !leader_exits {
                assert_eq!(
                    rpc.request(1, "initialize", json!({})).unwrap_err().reason,
                    "timeout"
                );
            }
            drop(rpc);
            let until = Instant::now() + Duration::from_secs(2);
            while TcpStream::connect(&address).is_ok() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
            let stopped = TcpStream::connect(&address).is_err();
            // Clean up this fixture even when verifying the pre-fix failure.
            if !stopped {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
            assert!(stopped, "descendant survived (leader_exits={leader_exits})");
        }
    }

    #[test]
    fn fake_cli_malformed_eof_buffer_cap_and_deadline_are_bounded() {
        for (script, reason) in [
            ("print('not json',flush=True)", "malformed"),
            ("pass", "eof"),
            ("print('x'*1048577,flush=True)", "response_too_large"),
            ("import time; time.sleep(30)", "timeout"),
        ] {
            let tmp = Scratch::new();
            let binary = tmp.script(&format!("input()\n{script}"));
            let start = Instant::now();
            let out = probe(
                &binary,
                &tmp.0,
                &tmp.0,
                if reason == "timeout" {
                    Duration::from_millis(300)
                } else {
                    Duration::from_secs(3)
                },
            );
            assert_eq!(out.info.reason, Some(reason));
            assert!(start.elapsed() < Duration::from_secs(4));
        }
    }
    #[test]
    fn fake_cli_auth_classification_and_rpc_failure_do_not_invent_logout() {
        for (account, state) in [
            ("null", "signed_out"),
            (r#"{"type":"apiKey"}"#, "unsupported_auth"),
        ] {
            let tmp = Scratch::new();
            let binary = tmp.script(&format!(
                r#"import sys,json
for line in sys.stdin:
 r=json.loads(line)
 if r['method']=='initialized': continue
 print(json.dumps({{'id':r['id'],'result':{{'account':json.loads('{account}')}}}}),flush=True)
"#
            ));
            assert_eq!(
                probe(&binary, &tmp.0, &tmp.0, Duration::from_secs(3))
                    .info
                    .state,
                state
            );
        }
        let tmp = Scratch::new();
        let binary = tmp.script("print('{\"id\":1,\"error\":{\"code\":-32601}}',flush=True)");
        let out = probe(&binary, &tmp.0, &tmp.0, Duration::from_secs(3));
        assert_eq!(out.info.state, "unavailable");
        assert_eq!(out.info.reason, Some("unsupported_protocol"));
    }
}

#[cfg(test)]
mod live_check {
    /// Explicit opt-in only: sends account reads using the human's existing CLI
    /// login. No conversation, sign-in flow, config write or raw response output.
    #[test]
    #[ignore = "requires a real Codex ChatGPT login and network"]
    fn real_login_returns_plan_windows() {
        let dir = std::env::temp_dir().join(format!("wt-codex-live-{}", std::process::id()));
        let result = super::read(dir.clone());
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(
            result.state, "ready",
            "usage read category: {:?}",
            result.reason
        );
        assert!(!result.limits.is_empty());
        println!(
            "real Codex login: {} quota windows; response values withheld",
            result.limits.len()
        );
    }
}
