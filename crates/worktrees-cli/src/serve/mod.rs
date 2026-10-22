//! `worktrees serve` — the engine, hosted behind a loopback WebSocket.
//!
//! One engine process per host (`worktrees_api`'s crate doc, decided
//! 2026-10-17): everything that owns a registry — the nav-dot poll, the dock
//! shells, the docs server, the writers of `shell-cwds.json` and the rings —
//! runs here, and the desktop app, a second desktop or a phone are clients of
//! it. This slice is the server half: the listener, the loopback rule set, the
//! RPC over the 168 commands, streams and events, and the host wiring every
//! engine process needs. The frontend transport and the shim becoming a
//! client are the next lane's.
//!
//! Modules: `policy` (the pure rule set), `conn` (the socket), `wire` (the
//! frames and the per-connection queue), `dispatch` (the exposure table and
//! the arms), `servefile` (`serve.json`), `devices` (`devices.json`, the
//! device registry every grant is read from), `netwatch` (the remote
//! listener's address rules, not yet wired), `audit` (`audit.jsonl`, which
//! only `conn` writes), `budget` (what an authenticated client may cost) and
//! `flight` (the reads concurrent callers share).

mod audit;
mod budget;
mod conn;
mod devices;
mod dispatch;
mod flight;
mod identity;
mod netwatch;
mod pairing;
mod policy;
mod provenance;
mod resolve;
mod scope;
mod servefile;
mod stepup;
mod tls;
mod wire;

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use worktrees_api::log::LogSink;
use worktrees_api::{host, Ctx};
use worktrees_core::render::error_line;

pub const USAGE: &str = "\
usage: worktrees serve [--config-dir DIR] [--port N] [--allow-origin ORIGIN]... [--owner app]

  --config-dir DIR      the app's config dir (default: the desktop app's own —
                        ~/Library/Application Support/net.casadelvalle.worktrees
                        on macOS, $XDG_CONFIG_HOME/net.casadelvalle.worktrees on Linux)
  --port N              listen on 127.0.0.1:N (default: an ephemeral port)
  --allow-origin ORIGIN also accept this Origin (DEV ONLY; repeatable; a flag,
                        never a config key, so a dev entry cannot ship)
  --owner app           the desktop app started this serve and stops it when it
                        quits; recorded in serve.json so an app relaunched after
                        a crash adopts it instead of leaving it running unseen

Writes <config-dir>/serve.json ({port, token, pid, version[, owner]}, mode 0600) and
serves until SIGINT/SIGTERM. Logs to ~/.cache/worktrees/serve.log, and keeps an
audit trail of what reached the socket in <config-dir>/audit.jsonl (mode 0600,
hash-chained, rotated at 5 MiB, five files kept).";

pub struct Opts {
    pub config_dir: PathBuf,
    pub port: u16,
    pub allow_origin: Vec<String>,
    /// Who manages this serve's life, written into serve.json. `None` — the
    /// CLI, the dev stack — means nobody: a client that finds it uses it and
    /// never signals it.
    pub owner: Option<String>,
}

/// The owners `--owner` takes. One today: the desktop app, which SIGTERMs a
/// serve it owns when it quits cleanly. Closed rather than free text, so a
/// typo is an error at launch and not a serve the app silently adopts or not.
pub const OWNERS: &[&str] = &["app"];

pub fn parse_args(args: &[String]) -> Result<Option<Opts>, String> {
    let mut o = Opts { config_dir: worktrees_api::hostenv::default_config_dir(), port: 0, allow_origin: Vec::new(), owner: None };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" | "help" => return Ok(None),
            "--config-dir" => {
                let v = it.next().ok_or("--config-dir needs a directory")?;
                o.config_dir = PathBuf::from(v);
            }
            "--port" => {
                let v = it.next().ok_or("--port needs a number")?;
                o.port = v.parse().map_err(|_| format!("--port {v:?}: not a port number"))?;
            }
            "--owner" => {
                let v = it.next().ok_or("--owner needs a name")?;
                if !OWNERS.contains(&v.as_str()) {
                    return Err(format!("--owner {v:?}: the only owner is {}", OWNERS.join(", ")));
                }
                o.owner = Some(v.clone());
            }
            "--allow-origin" => {
                let v = it.next().ok_or("--allow-origin needs an origin")?;
                policy::allow_origin_flag_ok(v)?;
                o.allow_origin.push(v.clone());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if o.config_dir.as_os_str().is_empty() {
        return Err("--config-dir: empty".into());
    }
    Ok(Some(o))
}

// ── the log ──────────────────────────────────────────────────────────────────

/// `~/.cache/worktrees/serve.log` — the CLI's cache location (inbox,
/// codex-usage and the plan hook already live there), one file per machine,
/// single-slot rotation at ~1MB like the app's. The first line of every run
/// is the `startup v…` line `host::boot` writes, which is what lets a reader
/// tell WHICH binary wrote a log after an upgrade (AGENTS.md: a long-lived
/// server outlives the CLI that replaced it on disk).
fn log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".cache/worktrees")
}

fn log_file() -> PathBuf {
    log_dir().join("serve.log")
}

struct ServeLog;

impl LogSink for ServeLog {
    fn log(&self, level: &str, msg: &str) {
        append_line(&log_dir(), &log_file(), level, msg);
    }

    fn location(&self) -> Option<(PathBuf, PathBuf)> {
        Some((log_dir(), log_file()))
    }
}

/// One log line, appended to `path` (rotating it into `dir` past ~1MB).
///
/// Owner-only, like serve.json beside it (OWASP L1): the log carries the
/// engine's PATH, the places opened and every client connect, and was created
/// 0644. The mode at creation covers a new file; an existing one is tightened
/// on every write, because every serve on the machine shares this file and an
/// older binary may have made it; and the rotated copy is tightened as well,
/// because a `rename` keeps whatever mode the file had.
///
/// Only ever a REGULAR file. A FIFO planted at the name hung serve at its
/// first line, because a plain open for writing waits for a reader; and a
/// link there was followed by the write, and by the rotation, which renamed
/// it and chmodded the file behind it. Same-user attacks (Phase 2 §1), cheap
/// to close: both opens go through `open_regular`, and the size check that
/// decides a rotation does not follow a link. A line the file refuses goes to
/// stderr, which the app points at /dev/null and a terminal shows.
fn append_line(dir: &Path, path: &Path, level: &str, msg: &str) {
    let ts = worktrees_api::util::fmt_utc(worktrees_core::sysclock::now_epoch());
    // The pid on EVERY line: this is one file for every serve on the
    // machine — the real one, a stack, a server outliving the binary
    // that replaced it — and `grep <pid>` is how one of them is read.
    let line = format!("{ts}Z [{level}] [{}] {msg}\n", std::process::id());
    if let Err(e) = append_to(dir, path, &line) {
        eprint!("serve.log: {e}: {line}");
    }
}

fn append_to(dir: &Path, path: &Path, line: &str) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all(dir);
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() > 1_000_000) {
        let rotated = dir.join("serve.log.1");
        if std::fs::rename(path, &rotated).is_ok() {
            if let Ok(f) = open_regular(&rotated, std::fs::OpenOptions::new().read(true)) {
                worktrees_api::log::owner_only(&f);
            }
        }
    }
    let f = open_regular(path, std::fs::OpenOptions::new().create(true).append(true).mode(0o600))?;
    worktrees_api::log::owner_only(&f);
    // ONE `write(2)` per line. `writeln!` on an unbuffered handle is one
    // write per formatted piece, and two threads' pieces interleave in the
    // file: the transport lane read `…03:07:38serve: host-admin …Z [` in
    // the real log. A single short write to an O_APPEND handle lands whole.
    (&f).write_all(line.as_bytes())
}

/// `opts` plus `O_NOFOLLOW` (a link at the name is an error, not followed)
/// and `O_NONBLOCK` (a FIFO is an error, not a wait for a reader), then
/// refused unless `fstat` says regular file. The audit log's writer opens the
/// same way.
fn open_regular(path: &Path, opts: &mut std::fs::OpenOptions) -> std::io::Result<std::fs::File> {
    let f = opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    if !f.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok(f)
}

/// The soft open-file limit serve asks for: macOS's per-process ceiling for
/// a plain `setrlimit` (OPEN_MAX), and plenty on Linux. Capped rather than
/// "the hard limit", because a shell or tool this process starts inherits it,
/// and some close every descriptor up to the limit on start.
const NOFILE_WANT: libc::rlim_t = 10_240;

/// Raise the SOFT open-file limit toward `NOFILE_WANT`, never past the hard
/// limit and never down. A launchd-started process on macOS begins at 256
/// (hard unlimited), and the OWASP review spent all 256 with 300 idle
/// sockets: the user's own client was RST, an authenticated one's
/// `tmux_check` failed for want of a descriptor. The connection caps bound
/// the sockets; this is the headroom for everything else — ptys, git, tmux.
fn raise_open_file_limit() {
    let mut rl = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: plain syscalls on a stack value.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) } != 0 {
        worktrees_api::log::warn(&format!("serve: open-file limit unreadable: {}", std::io::Error::last_os_error()));
        return;
    }
    let old = rl.rlim_cur;
    let want = NOFILE_WANT.min(rl.rlim_max);
    if old >= want {
        worktrees_api::log::info(&format!("serve: open-file limit {old} (left as it is)"));
        return;
    }
    let new = libc::rlimit { rlim_cur: want, rlim_max: rl.rlim_max };
    // SAFETY: as above.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &new) } != 0 {
        worktrees_api::log::warn(&format!("serve: open-file limit {old}, raising to {want} failed: {}", std::io::Error::last_os_error()));
        return;
    }
    worktrees_api::log::info(&format!("serve: open-file limit {old} → {want}"));
}

// ── the server ───────────────────────────────────────────────────────────────

/// A running server: the listener, the engine's `Ctx`, and the token the
/// `serve.json` carries. `start` builds all three; `cmd_serve` adds the file,
/// the host wiring and the signal wait; the tests stop at `start`.
pub struct Running {
    pub ctx: Ctx,
    pub token: String,
    pub listener: conn::Listener,
    /// The authenticated connections, which the idle watch counts.
    pub clients: Arc<conn::Clients>,
    /// The paired devices (`devices.json`), or `None` when the caller handed
    /// in its own registry (the in-process tests' fixtures).
    pub devices: Option<Arc<devices::DeviceRegistry>>,
}

impl Running {
    pub fn port(&self) -> u16 {
        self.listener.port
    }
}

/// Build the engine and bind the listener. Inside a tokio runtime.
pub fn start(config_dir: PathBuf, port: u16, allow_origin: Vec<String>) -> Result<Running, String> {
    start_with(config_dir, port, allow_origin, conn::LIMITS)
}

/// `start`, with the connection caps given rather than the defaults. Every
/// grant is read from the config dir's `devices.json` (Phase 2 slice 5).
pub fn start_with(config_dir: PathBuf, port: u16, allow_origin: Vec<String>, limits: conn::Limits) -> Result<Running, String> {
    let devices = Arc::new(devices::DeviceRegistry::open(&config_dir));
    // THE audit seam (§5.1–§5.3; the audit log is #534): every registry
    // change, pairing step and refused device handshake is a typed
    // `devices::DeviceEvent`, handed to each hook `devices.subscribe` was
    // given, after the registry's locks are released. `DeviceEvent::alerts`
    // marks §5.3's alerts. The log's rotation checkpoint is
    // `devices.checkpoint()`, and §5.2's boot cross-check is
    // `devices.cross_check(chain's fields per device, now)`. A hook
    // subscribed HERE, before `startup`, hears an unreadable file's alert.
    // Pairing windows bind where slice 6's remote listener says; until it
    // exists, none ever opens (`Pairing::set_bind`).
    let pairing = Arc::new(pairing::Pairing::new(Some(config_dir.clone()), devices.clone()));
    let clients = conn::Clients::serving(devices.clone(), pairing, None);
    let mut running = start_in(config_dir, port, allow_origin, limits, budget::BUDGETS, clients)?;
    running.devices = Some(devices);
    Ok(running)
}

/// `start_with`, with the authenticated clients' budgets given too, over a
/// given client set — its device registry and the world objects resolve
/// against (the in-process tests' fixtures).
pub fn start_in(
    config_dir: PathBuf,
    port: u16,
    allow_origin: Vec<String>,
    limits: conn::Limits,
    budgets: budget::Budgets,
    clients: conn::Clients,
) -> Result<Running, String> {
    let clients = Arc::new(clients.budgeted(budgets));
    // A revoke, a disable or a narrowed grant acts on the live connections.
    clients.listen();
    let ctx = Ctx::new(config_dir, Arc::new(conn::Broadcast(clients.clone())));
    let token = servefile::new_token();
    let listener = conn::start(ctx.clone(), token.clone(), port, allow_origin, clients.clone(), limits)?;
    Ok(Running { ctx, token, listener, clients, devices: None })
}

// ── device expiry ────────────────────────────────────────────────────────────

/// The longest the expiry watch sleeps: also how often it looks for a hand
/// edit of `devices.json` (§5.2) and writes pending `last_seen`s.
const EXPIRY_TICK: Duration = Duration::from_secs(60);

/// The registry's clock (§2.5): wake at the earliest expiry of a live device,
/// or after `EXPIRY_TICK`, or as soon as any record changes (a new `until`
/// may be sooner than the one being slept towards); then re-read the file if
/// it changed, disable whatever ran out — which closes its connections, as a
/// revoke does — and write the `last_seen`s that are due. A device idle past
/// its window is marked here even if it never knocks again, so the Devices
/// list and the alert do not wait for a knock.
pub async fn expiry_watch(devices: Arc<devices::DeviceRegistry>) {
    let changed = Arc::new(tokio::sync::Notify::new());
    {
        let changed = changed.clone();
        devices.subscribe(Arc::new(move |_| changed.notify_one()));
    }
    loop {
        let now = worktrees_core::sysclock::now_epoch();
        let wait = devices
            .next_deadline()
            .map(|at| Duration::from_secs(u64::try_from(at - now).unwrap_or(0)))
            .unwrap_or(EXPIRY_TICK)
            .min(EXPIRY_TICK);
        let sleep = std::pin::pin!(tokio::time::sleep(wait.max(Duration::from_millis(200))));
        let woke = std::pin::pin!(changed.notified());
        futures_util::future::select(sleep, woke).await;
        let d = devices.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let now = worktrees_core::sysclock::now_epoch();
            d.refresh();
            if let Err(e) = d.check_expiry(now) {
                worktrees_api::log::warn(&format!("serve: device expiry: {e}"));
            }
            d.flush_due(now);
        })
        .await;
    }
}

// ── the idle stop ────────────────────────────────────────────────────────────

/// How long a serve the APP owns stays up with no client before it stops itself.
///
/// A crash leaves the app's serve running with nobody to quit it, and before
/// this it ran on unseen for good: a downgraded app then put a second engine
/// on the same config dir — two shells for one dock tab, two polls, and the
/// orphan's saves over the old app's (rehearsal D7). Two minutes is the
/// trade. A crash → relaunch inside it adopts the serve, and the dock's live
/// shells with it; one after it starts a fresh serve, whose shells respawn
/// from the saved ring behind a seam, and tmux is untouched either way.
pub const OWNED_IDLE_GRACE: Duration = Duration::from_secs(120);

/// How often the idle watch counts clients. A client that came and went
/// between two counts is not seen; one that is using the serve stays.
const IDLE_TICK: Duration = Duration::from_secs(5);

/// The idle grace for a serve with this owner, or `None` for one that never
/// stops on its own: a serve somebody started by hand (the CLI, the dev
/// stack) has no owner to come back for it, and is theirs to stop.
pub fn idle_grace(owner: Option<&str>) -> Option<Duration> {
    (owner == Some("app")).then_some(OWNED_IDLE_GRACE)
}

/// One count of the idle watch: (when the server was first counted with no
/// client, if it has none now; whether that has lasted `grace`).
pub fn idle_step(since: Option<Instant>, clients: usize, now: Instant, grace: Duration) -> (Option<Instant>, bool) {
    if clients > 0 {
        return (None, false);
    }
    let since = since.unwrap_or(now);
    (Some(since), now.duration_since(since) >= grace)
}

/// Count `clients` every `tick`; once there has been none for `grace`, say
/// so and notify `stop` — the `Notify` the signals use, so the way down is
/// the one a SIGTERM takes: flush, close, remove serve.json, "stopped".
pub async fn idle_watch(clients: Arc<conn::Clients>, grace: Duration, tick: Duration, stop: Arc<tokio::sync::Notify>) {
    let mut since = None;
    loop {
        tokio::time::sleep(tick).await;
        let (s, out) = idle_step(since, clients.len(), Instant::now(), grace);
        since = s;
        if out {
            worktrees_api::log::info(&format!(
                "serve: no client for {}s, and the app that owns this serve is gone — stopping",
                grace.as_secs()
            ));
            stop.notify_one();
            return;
        }
    }
}

/// `worktrees serve …`. Runs until a signal; the exit code is the usual.
pub fn cmd_serve(args: &[String]) -> i32 {
    let opts = match parse_args(args) {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(e) => {
            eprintln!("{}", error_line(&e));
            eprintln!("{USAGE}");
            return 1;
        }
    };
    // The preamble every engine process shares: log sink, panic hook, PATH,
    // locale, core's log, the startup line.
    host::boot(Arc::new(ServeLog));
    let me = std::process::id();
    worktrees_api::log::info(&format!(
        "serve: pid={me} config_dir={} owner={}",
        opts.config_dir.display(),
        opts.owner.as_deref().unwrap_or("none")
    ));
    raise_open_file_limit();

    // One engine per host: a second serve on the same config dir is refused
    // while the first is live. A stale file (dead pid, or nothing on the
    // port) is overwritten below.
    if let Some(other) = servefile::live_other(&opts.config_dir, me) {
        let msg = format!(
            "another `worktrees serve` (pid {}, v{}) is already serving {} on 127.0.0.1:{}",
            other.pid, other.version, opts.config_dir.display(), other.port
        );
        worktrees_api::log::error(&format!("serve: {msg}"));
        eprintln!("{}", error_line(&msg));
        return 1;
    }

    // Host policy (Codex permissions, pi project trust, the auto-fetch
    // cadence, the two dock-shell switches) is the ENGINE's to read, and this
    // process is the engine: `hostpolicy::boot` migrates and loads
    // `host-policy.json` into core's overrides and the engine's statics. The
    // shim never calls it (hostpolicy-check.mjs). After the one-engine
    // refusal above, because boot may write the file, and before the
    // listener exists, so no client is served under the wrong policy.
    worktrees_api::hostpolicy::boot(&opts.config_dir);

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{}", error_line(&format!("tokio runtime: {e}")));
            return 1;
        }
    };
    let code = rt.block_on(async move {
        // Caught from here on, before serve.json can exist: see `signals`.
        let stop = match signals() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("{}", error_line(&format!("signal handlers: {e}")));
                return 1;
            }
        };
        let running = match start(opts.config_dir.clone(), opts.port, opts.allow_origin.clone()) {
            Ok(r) => r,
            Err(e) => {
                worktrees_api::log::error(&format!("serve: {e}"));
                eprintln!("{}", error_line(&e));
                return 1;
            }
        };
        let sf = servefile::ServeFile {
            port: running.port(),
            token: running.token.clone(),
            pid: me,
            version: env!("CARGO_PKG_VERSION").to_string(),
            owner: opts.owner.clone(),
        };
        if let Err(e) = servefile::write(&opts.config_dir, &sf) {
            worktrees_api::log::error(&format!("serve: serve.json: {e}"));
            eprintln!("{}", error_line(&format!("serve.json: {e}")));
            running.listener.stop();
            return 1;
        }
        // The engine's background work: sweep, merge, the 3 s poll.
        host::start(running.ctx.clone(), host::Hooks::default());
        // The device registry's own: an unreadable file's alert, the expiries
        // that ran out while serve was down, then the expiry clock.
        if let Some(devices) = running.devices.clone() {
            devices.startup(worktrees_core::sysclock::now_epoch());
            tokio::spawn(expiry_watch(devices));
        }
        let file = servefile::path(&opts.config_dir);
        worktrees_api::log::info(&format!(
            "serve: listening on 127.0.0.1:{} serve.json={} origins=+{}",
            running.port(), file.display(), opts.allow_origin.len()
        ));
        // The token is deliberately not printed: it is in the 0600 file and
        // nowhere else.
        println!("worktrees serve: 127.0.0.1:{} · {}", running.port(), file.display());

        // A serve the app started stops itself once no client has come back
        // for a while (`OWNED_IDLE_GRACE`), by the same way down as a signal.
        if let Some(grace) = idle_grace(opts.owner.as_deref()) {
            tokio::spawn(idle_watch(running.clients.clone(), grace, IDLE_TICK, stop.clone()));
        }

        stop.notified().await;

        worktrees_api::log::info("serve: stopping");
        // The dock's rings and directories BEFORE the port closes. A client
        // that finds the port closed may start a new serve at once, and that
        // one must not read a tab's ring ahead of this one's last write of
        // it. `host::stop` flushes again below and finds nothing new.
        worktrees_api::term::save_shell_cwds(&running.ctx, &running.ctx.shells);
        worktrees_api::term::save_shell_scrollback(&running.ctx, &running.ctx.shells);
        // `last_seen` is written at most every ten minutes, and at exit (§2.5).
        if let Some(devices) = &running.devices {
            devices.flush_seen(worktrees_core::sysclock::now_epoch());
        }
        running.listener.stop();
        host::stop(&running.ctx);
        match servefile::remove_if_ours(&opts.config_dir, me) {
            Ok(_) => {}
            Err(e) => worktrees_api::log::warn(&format!("serve: could not remove serve.json: {e}")),
        }
        worktrees_api::log::info("serve: stopped");
        0
    });
    // Dropping a runtime waits for every blocking task; a dispatch still
    // inside git or rsync would hold the exit open for as long as it takes.
    // `host::stop` has already flushed what matters, so bound it.
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    code
}

/// SIGINT or SIGTERM, whichever comes first, as a `Notify` to wait on. No
/// `select!` (tokio's `macros` feature stays off): a task per signal, one
/// `Notify`, whose stored permit keeps a signal that lands before anyone waits.
///
/// Each handler is in place when `signal()` RETURNS, which is why they are
/// made here, synchronously, before `start` and serve.json. serve.json is how
/// a client or a supervisor learns that serve is up, and the handlers used to
/// be registered only after it was written and the engine started, inside
/// spawned tasks: a SIGTERM in that window took the default action, so the
/// process died with the file left behind and no `serve: stopped`.
fn signals() -> std::io::Result<Arc<tokio::sync::Notify>> {
    use tokio::signal::unix::{signal, SignalKind};
    let n = Arc::new(tokio::sync::Notify::new());
    for kind in [SignalKind::interrupt(), SignalKind::terminate()] {
        let mut s = signal(kind)?;
        let n = n.clone();
        tokio::spawn(async move {
            if s.recv().await.is_some() {
                n.notify_one();
            }
        });
    }
    Ok(n)
}

#[cfg(test)]
mod tests;

/// serve.log's mode. Not the socket's business, so not in `tests`: these drive
/// `append_line` on a directory of their own.
#[cfg(test)]
mod log_tests {
    use super::append_line;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-serve-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    /// OWASP L1: serve.log was created 0644, readable by every local user,
    /// with the engine's PATH, the places opened and every client connect in
    /// it — beside a serve.json that is 0600.
    #[test]
    fn a_new_serve_log_is_owner_only() {
        let d = dir("fresh");
        let log = d.join("serve.log");
        append_line(&d, &log, "info", "hello");
        assert_eq!(mode(&log), 0o600, "serve.log is {:o}", mode(&log));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A log a previous serve left world-readable is tightened the next time
    /// one writes to it, not only when a new one is made. Every serve on the
    /// machine shares the file, an older binary among them.
    #[test]
    fn a_wider_serve_log_already_there_is_tightened_and_kept() {
        let d = dir("wide");
        let log = d.join("serve.log");
        std::fs::write(&log, "an earlier line\n").unwrap();
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
        append_line(&d, &log, "info", "hello");
        assert_eq!(mode(&log), 0o600, "serve.log is {:o}", mode(&log));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.starts_with("an earlier line\n") && text.ends_with("hello\n"), "{text}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The rotation is a `rename`, and a rename keeps the mode the file had:
    /// a 0644 log past the cap became a 0644 `serve.log.1`.
    #[test]
    fn the_rotated_log_is_owner_only_too() {
        let d = dir("rotate");
        let log = d.join("serve.log");
        std::fs::write(&log, vec![b'x'; 1_000_001]).unwrap();
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
        append_line(&d, &log, "info", "hello");
        let old = d.join("serve.log.1");
        assert_eq!(std::fs::metadata(&old).unwrap().len(), 1_000_001, "the full log was not the one rotated");
        assert_eq!(mode(&old), 0o600, "serve.log.1 is {:o}", mode(&old));
        assert_eq!(mode(&log), 0o600, "serve.log is {:o}", mode(&log));
        assert!(std::fs::read_to_string(&log).unwrap().ends_with("hello\n"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `append_line` on a thread, bounded: true if it returned within 5 s.
    /// When it did not, a reader is opened on `log` so a writer stuck in
    /// `open(2)` on a FIFO there gets through and the thread ends.
    fn returns_in_time(d: &Path, log: &Path) -> bool {
        use std::os::unix::fs::OpenOptionsExt;
        let (tx, rx) = std::sync::mpsc::channel();
        let (d2, l2) = (d.to_path_buf(), log.to_path_buf());
        std::thread::spawn(move || {
            append_line(&d2, &l2, "info", "hello");
            let _ = tx.send(());
        });
        let done = rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok();
        if !done {
            let _ = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(log);
            let _ = rx.recv_timeout(std::time::Duration::from_secs(5));
        }
        done
    }

    /// A FIFO planted at serve.log hung serve at its first log line: a plain
    /// `open(O_WRONLY)` of a FIFO waits for a reader that never comes. A
    /// same-user attack (Phase 2 §1), and cheap to close: the open does not
    /// wait, the FIFO is refused, and the line goes to stderr instead.
    #[test]
    fn a_fifo_at_the_log_path_is_refused_without_hanging() {
        use std::os::unix::fs::FileTypeExt;
        let d = dir("fifo");
        let log = d.join("serve.log");
        let c = std::ffi::CString::new(log.to_str().unwrap()).unwrap();
        // SAFETY: a plain syscall on a NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0, "mkfifo");
        assert!(returns_in_time(&d, &log), "append_line hung on a FIFO at serve.log");
        let ft = std::fs::symlink_metadata(&log).unwrap().file_type();
        assert!(ft.is_fifo(), "the FIFO was replaced rather than refused");
        assert!(!d.join("serve.log.1").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A link at serve.log is neither followed nor rotated onto. The size
    /// check followed it, so a link to any file over the cap was renamed to
    /// serve.log.1 and the file behind it was opened through the link and
    /// chmodded 0600; the write itself followed a link at the name, too.
    #[test]
    fn a_link_at_the_log_path_is_not_followed_or_rotated() {
        let d = dir("link");
        let target = d.join("elsewhere");
        std::fs::write(&target, vec![b'x'; 1_000_001]).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        let log = d.join("serve.log");
        std::os::unix::fs::symlink(&target, &log).unwrap();
        assert!(returns_in_time(&d, &log), "append_line hung");
        assert!(std::fs::symlink_metadata(&log).unwrap().file_type().is_symlink(), "the link was moved");
        assert!(!d.join("serve.log.1").exists(), "the link was rotated");
        assert_eq!(mode(&target), 0o644, "the file behind the link was chmodded");
        assert_eq!(std::fs::metadata(&target).unwrap().len(), 1_000_001, "the line went through the link");
        let _ = std::fs::remove_dir_all(&d);
    }
}
