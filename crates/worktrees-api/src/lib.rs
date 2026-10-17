//! The command layer over `worktrees-core`, with no window attached.
//!
//! `app/src-tauri/src/lib.rs` holds 143 `#[tauri::command]` handlers and is the
//! only thing in the repo that can call them. A server edition needs the same
//! 143 from something that is not a WebView, so the bodies move here and the
//! app keeps thin wrappers (`docs/proposals/server-edition.md` §III.0, and the
//! plan in the `feat-pretty-ui-server` place).
//!
//! **What this crate may not depend on.** Not `tauri` — that is the coupling
//! the extraction exists to remove, and `cargo tree -p worktrees-api` is how
//! you check. Not a transport either: the caller owns the socket. This crate
//! takes a `Ctx` and returns data.
//!
//! ## What `AppHandle` actually was
//!
//! Measured on the 11,288-line lib.rs before any of it moved: 60 mentions of
//! `AppHandle`, resolving to six distinct uses — `app.path()` ×11 (the config
//! dir), `app.emit()` ×9 (events to the client), `app.clone()` ×3, and three
//! window calls. So the handle is two things plus a client-local remainder,
//! and `Ctx` is those two things. The window calls stay in the Tauri shim:
//! a phone has no window of ours to address, and §III.4.B of the plan already
//! classes them client-local.
//!
//! Two deviations from the plan's sketch, both found by reading the code
//! rather than the design:
//!
//! - The plan wrote `Ctx { config_dir, events }`, and a first cut here put
//!   logging on the same trait because `import_app_projects` needs `applog`.
//!   Measuring it killed that: 150 `applog` call sites, 59 of the 143 handler
//!   bodies, plus the helpers under them. Threading a `Ctx` into every
//!   function that might warn is churn in service of something none of them
//!   vary by, so the log is a process-global installed sink (`log`) — which
//!   is exactly what `applog` already was — and `Host` carries events alone.
//! - `snapshot()` is NOT pure core, which the measured dependency list hides:
//!   it reaches `watch_lane`, and that writes the process-global codex/pi
//!   watch registries. Handlers touching those cannot move until the registry
//!   has a home — a `Ctx` field, or core. It is the one cluster in the
//!   extraction that is not mechanical, and it is better known now than at
//!   handler 90.

pub mod log;
pub mod meta;
pub mod projects;
pub mod status;
pub mod watch;
pub mod workspace;
pub mod util;
pub mod ops;
pub mod profiles;

use std::path::PathBuf;
use std::sync::Arc;

/// Where an event goes. Tauri implements it with `app.emit`; `worktrees serve`
/// implements it with a frame broadcast.
///
/// Logging is deliberately NOT here — see the `log` module. A log belongs to
/// the process, an event belongs to a client, and the two have different
/// lifetimes: the app logs from its panic hook, long after any `Ctx` is gone.
///
/// Nothing here returns a value to the host: an event is fire-and-forget by
/// construction, so a slow or dead client cannot stall a command.
pub trait Host: Send + Sync {
    /// Push an event to the client(s). Tauri's `app.emit`.
    fn emit(&self, event: &str, payload: serde_json::Value);
}

/// An ordered byte stream to one client — a terminal attach, a dock shell.
///
/// Tauri's `Channel<ArrayBuffer>` implements it; so does the server's frame
/// writer. ORDER is the contract and it is load-bearing: AGENTS.md records a
/// replay being answered as input because an out-of-band payload overtook the
/// response it belonged to, and only the channel's own order was reliable. An
/// implementation that reorders is a bug, not a tuning choice.
pub trait ByteSink: Send + Sync {
    /// Append bytes to the stream. `Err` means the stream is gone for good —
    /// the caller should stop producing, not retry.
    fn send(&self, bytes: &[u8]) -> Result<(), String>;
}

/// What a command is called with, in place of `AppHandle` and `State<_>`.
#[derive(Clone)]
pub struct Ctx {
    /// The app config dir — what `AppHandle::path().app_config_dir()` answers,
    /// and where `ui-state.json` and `projects.json` live. The server is given
    /// the same directory, because the host machine's configuration is the
    /// host machine's whichever client is driving.
    pub config_dir: PathBuf,
    pub host: Arc<dyn Host>,
    /// The nav-dot poll's registries. Per PROCESS, not per client: they track
    /// this machine's tmux sessions, so two clients watching the same place
    /// share one set of watches and one model cache. Previously three
    /// `static`s in the app — see `watch` for why they stopped being that.
    pub watch: Arc<watch::Watch>,
}

impl Ctx {
    pub fn new(config_dir: PathBuf, host: Arc<dyn Host>) -> Self {
        Ctx { config_dir, host, watch: Arc::new(watch::Watch::default()) }
    }

    /// The app's own pre-registry project list. Only ever READ — it is the
    /// source of the one-way merge into core's registry.
    pub fn projects_file(&self) -> PathBuf {
        self.config_dir.join("projects.json")
    }

    /// Convenience for call sites that already hold a `Ctx`; identical to
    /// `log::log`, which is what the helpers below them call.
    pub fn log(&self, level: &str, msg: &str) {
        log::log(level, msg);
    }

    pub fn emit(&self, event: &str, payload: serde_json::Value) {
        self.host.emit(event, payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A host that records, so a moved handler can be tested without a window
    /// — which is most of the point of the extraction.
    #[derive(Default)]
    pub struct Recorder {
        pub events: Mutex<Vec<(String, serde_json::Value)>>,
        pub logs: Mutex<Vec<(String, String)>>,
    }

    impl Host for Recorder {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.events.lock().unwrap().push((event.into(), payload));
        }
    }

    impl log::LogSink for Recorder {
        fn log(&self, level: &str, msg: &str) {
            self.logs.lock().unwrap().push((level.into(), msg.into()));
        }
    }

    #[test]
    fn a_ctx_can_be_built_and_emitted_through_without_a_window() {
        let rec = Arc::new(Recorder::default());
        let ctx = Ctx::new(PathBuf::from("/tmp/wt-cfg"), rec.clone());
        ctx.emit("sessions:busy", serde_json::json!({"slug": "x"}));
        assert_eq!(ctx.projects_file(), PathBuf::from("/tmp/wt-cfg/projects.json"));
        assert_eq!(rec.events.lock().unwrap()[0].0, "sessions:busy");
    }

    /// Logging before any sink is installed must be a no-op rather than a
    /// panic: every unit test below reaches a handler with no host at all.
    #[test]
    fn logging_without_an_installed_sink_is_silent() {
        log::warn("nobody is listening");
    }
}
