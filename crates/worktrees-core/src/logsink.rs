//! One installable log sink for the whole engine.
//!
//! Core cannot call the app's `applog`: the CLI and every `worktrees mcp`
//! process link this same crate and have no app log to write to. Nor is two
//! call sites worth a logging framework and its initialisation order. So the
//! HOST installs a function pointer at startup and core calls it when one is
//! there.
//!
//! Unset means DO NOT LOG — never a reason to fail. A reader whose only way to
//! report a degraded fetch is a sink nobody installed still has to answer, and
//! the CLI deliberately leaves it unset.
//!
//! ONE sink, not one per module. `codex_usage` and `claude_usage` arrived with
//! a `LOG` static each, which turned installing them into a list the app had to
//! keep in step: a third reader would have been a third `set` call, and
//! forgetting it would have been silent — the module would simply never log,
//! which is indistinguishable from a module that had nothing to say.

/// Where a line goes. Unset in the CLI; installed by the app at startup.
pub static LOG: std::sync::OnceLock<fn(&str, &str)> = std::sync::OnceLock::new();

/// Install the host's logger. First call wins; a second is a no-op rather than
/// a race, so a test binary that stands up more than one app does not panic.
pub fn install(sink: fn(&str, &str)) {
    let _ = LOG.set(sink);
}

/// Emit one line, if anybody is listening.
pub fn log(level: &str, msg: &str) {
    if let Some(sink) = LOG.get() {
        sink(level, msg);
    }
}
