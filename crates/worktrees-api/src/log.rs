//! The host's log, as a process-global sink.
//!
//! `app/src-tauri`'s `applog` was a free function that anyone — handler,
//! helper, panic hook — could call, writing one `app.log` for the process.
//! That shape is kept deliberately, because the fact it models has not
//! changed: a log belongs to the MACHINE, not to a request or a client, and
//! `worktrees serve` has exactly one of them just as the app does.
//!
//! The alternative was threading `&Ctx` into every function that might warn.
//! Measured on the pre-extraction lib.rs: 150 `applog` call sites, 59 of the
//! 143 handler bodies calling it directly, plus helpers below them. That is
//! churn in the signature of nearly every function in the crate to carry
//! something none of them vary by.
//!
//! What the seam buys that a `const` did not: `log_dir()` derived its path
//! from `APP_IDENT`, a compile-time constant, so a sandbox launched under
//! `net.casadelvalle.worktrees.sbx` wrote into the REAL app's log — a gap
//! AGENTS.md records under `sandbox.sh`. An installed sink knows the
//! identifier it is actually running as.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// Somewhere a line of log can go. The app writes `app.log`; the server
/// writes its own; a test records.
pub trait LogSink: Send + Sync {
    /// `level` is `info` / `warn` / `error`.
    fn log(&self, level: &str, msg: &str);

    /// `(directory, file)` the sink writes to, for the two handlers that
    /// report and tail it. `None` for a sink with no file behind it (a test
    /// recorder, or a server logging to stderr), and both of those handlers
    /// answer emptily rather than inventing a path.
    fn location(&self) -> Option<(PathBuf, PathBuf)> {
        None
    }
}

static SINK: OnceLock<Arc<dyn LogSink>> = OnceLock::new();

/// Install the process's log sink. The FIRST call wins and later ones are
/// ignored, which is why this returns whether it took: a second installer is
/// a bug in the host's startup, not something to paper over.
pub fn install(sink: Arc<dyn LogSink>) -> bool {
    SINK.set(sink).is_ok()
}

/// Append one line. A no-op before `install` — a library that cannot log is
/// strictly better than one that panics or invents a path to write to, and
/// unit tests reach handlers without a host at all.
pub fn log(level: &str, msg: &str) {
    if let Some(s) = SINK.get() {
        s.log(level, msg);
    }
}

pub fn info(msg: &str) {
    log("info", msg);
}
pub fn warn(msg: &str) {
    log("warn", msg);
}
pub fn error(msg: &str) {
    log("error", msg);
}

/// The most of a panic's message that goes into a log line.
pub const PANIC_TEXT_MAX: usize = 500;

/// A panic's message as log text: on one line, bounded, quoted. `{:?}` escapes
/// every control and line separator (C0, C1, U+2028/2029, the bidi controls —
/// measured), and the cap matters because a panic can carry a client's value
/// whole. Shared by the panic hook and `serve`'s reply to a body that panicked.
pub fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    let s = match (payload.downcast_ref::<&str>(), payload.downcast_ref::<String>()) {
        (Some(s), _) => *s,
        (_, Some(s)) => s.as_str(),
        _ => "(not a string)",
    };
    let cut: String = s.chars().take(PANIC_TEXT_MAX).collect();
    format!("{cut:?}")
}

/// Take group and other access off an open log file — what both hosts' sinks
/// do to their file on every write and to its rotated copy (OWASP L1: serve.log
/// and app.log were 0644, with the engine's PATH and the places opened in
/// them). Through the handle, so the file checked is the file changed. A
/// failure is left alone: a file this user does not own cannot be appended
/// to either.
pub fn owner_only(f: &std::fs::File) {
    use std::os::unix::fs::PermissionsExt;
    if f.metadata().is_ok_and(|m| m.permissions().mode() & 0o077 != 0) {
        let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
}

/// Where the installed sink writes, if anywhere.
pub fn location() -> Option<(PathBuf, PathBuf)> {
    SINK.get().and_then(|s| s.location())
}

/// What `log_info` reports: the log's directory and file, empty strings when
/// the sink has no file.
pub fn paths() -> (String, String) {
    location()
        .map(|(d, f)| (d.to_string_lossy().into_owned(), f.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// The last `n` lines of the log. Empty when there is no file, or it is
/// unreadable — a log that cannot be read is not an error worth failing a
/// command over, and the Settings pane renders the emptiness honestly.
pub fn tail(n: usize) -> String {
    let Some((_, file)) = location() else { return String::new() };
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(n)..].join("\n")
}
