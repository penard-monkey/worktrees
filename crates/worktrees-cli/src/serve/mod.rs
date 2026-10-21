//! `worktrees serve` — the engine, hosted behind a loopback WebSocket.
//!
//! One engine process per host (`worktrees_api`'s crate doc, decided
//! 2026-10-17): everything that owns a registry — the nav-dot poll, the dock
//! shells, the docs server, the writers of `shell-cwds.json` and the rings —
//! runs here, and the desktop app, a second desktop or a phone are clients of
//! it. This slice is the server half: the listener, the loopback rule set, the
//! RPC over the 150 commands, streams and events, and the host wiring every
//! engine process needs. The frontend transport and the shim becoming a
//! client are the next lane's.
//!
//! Modules: `policy` (the pure rule set), `conn` (the socket), `wire` (the
//! frames and the per-connection queue), `dispatch` (the exposure table and
//! the arms), `servefile` (`serve.json`).

mod conn;
mod dispatch;
mod policy;
mod servefile;
mod wire;

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
serves until SIGINT/SIGTERM. Logs to ~/.cache/worktrees/serve.log.";

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
fn append_line(dir: &Path, path: &Path, level: &str, msg: &str) {
    let _ = std::fs::create_dir_all(dir);
    if std::fs::metadata(path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let rotated = dir.join("serve.log.1");
        if std::fs::rename(path, &rotated).is_ok() {
            if let Ok(f) = std::fs::File::open(&rotated) {
                worktrees_api::log::owner_only(&f);
            }
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path) {
        worktrees_api::log::owner_only(&f);
        let ts = worktrees_api::util::fmt_utc(worktrees_core::sysclock::now_epoch());
        // The pid on EVERY line: this is one file for every serve on the
        // machine — the real one, a stack, a server outliving the binary
        // that replaced it — and `grep <pid>` is how one of them is read.
        //
        // ONE `write(2)` per line. `writeln!` on an unbuffered handle is one
        // write per formatted piece, and two threads' pieces interleave in the
        // file: the transport lane read `…03:07:38serve: host-admin …Z [` in
        // the real log. A single short write to an O_APPEND handle lands whole.
        let line = format!("{ts}Z [{level}] [{}] {msg}\n", std::process::id());
        let _ = f.write_all(line.as_bytes());
    }
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

/// `start`, with the connection caps given rather than the defaults.
pub fn start_with(config_dir: PathBuf, port: u16, allow_origin: Vec<String>, limits: conn::Limits) -> Result<Running, String> {
    let clients = Arc::new(conn::Clients::default());
    let ctx = Ctx::new(config_dir, Arc::new(conn::Broadcast(clients.clone())));
    let token = servefile::new_token();
    let listener = conn::start(ctx.clone(), token.clone(), port, allow_origin, clients, limits)?;
    Ok(Running { ctx, token, listener })
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

    // Host policy (Codex permissions, pi project trust) is the ENGINE's to
    // read, and this process is the engine: `hostpolicy::boot` migrates and
    // loads `host-policy.json` into core's overrides, as the app's `.setup`
    // does. Here and not in `host::start`, which the app also calls: the
    // app's one call sits in its `.setup` (hostpolicy-check.mjs). After the
    // one-engine refusal above, because boot may write the file, and before
    // the listener exists, so no client is served under the wrong policy.
    worktrees_api::hostpolicy::boot(&opts.config_dir);

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{}", error_line(&format!("tokio runtime: {e}")));
            return 1;
        }
    };
    let code = rt.block_on(async move {
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
        let file = servefile::path(&opts.config_dir);
        worktrees_api::log::info(&format!(
            "serve: listening on 127.0.0.1:{} serve.json={} origins=+{}",
            running.port(), file.display(), opts.allow_origin.len()
        ));
        // The token is deliberately not printed: it is in the 0600 file and
        // nowhere else.
        println!("worktrees serve: 127.0.0.1:{} · {}", running.port(), file.display());

        wait_for_signal().await;

        worktrees_api::log::info("serve: stopping");
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

/// SIGINT or SIGTERM, whichever comes first. No `select!` (tokio's `macros`
/// feature stays off): two tasks, one `Notify`.
async fn wait_for_signal() {
    let n = Arc::new(tokio::sync::Notify::new());
    let n1 = n.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        n1.notify_one();
    });
    let n2 = n.clone();
    tokio::spawn(async move {
        if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            term.recv().await;
            n2.notify_one();
        }
    });
    n.notified().await;
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
}
