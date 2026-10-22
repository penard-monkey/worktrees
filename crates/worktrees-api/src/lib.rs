//! The command layer over `worktrees-core`, with no window attached.
//!
//! `app/src-tauri/src/lib.rs` holds 143 `#[tauri::command]` handlers and is the
//! only thing in the repo that can call them. A server edition needs the same
//! 143 from something that is not a WebView, so the bodies move here and the
//! app keeps thin wrappers (`docs/proposals/server-edition.md` §III.2 and
//! §III.9 Phase 0).
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
//!
//! ## One engine process per host (decided 2026-10-17)
//!
//! The plan drew today's app and tomorrow's `worktrees serve` as alternatives
//! and never both on one machine — which is the normal case, since the person
//! holding the phone is sitting at the desktop. Two processes each building a
//! `Ctx` means two nav-dot polls, two sets of dock-shell PTYs the other cannot
//! see, two docs servers, and two writers to `shell-cwds.json`, the term-hist
//! rings and `ui-events.jsonl`. So: **the desktop app becomes a client of a
//! local `worktrees serve`**, the same way the plan already makes a second
//! desktop one. The shim keeps the window, the client-local commands
//! (§III.4.B) and the job of finding or starting the server; everything that
//! owns a registry runs in exactly one process per host. That makes a version
//! handshake mandatory rather than nice — after an upgrade, "the app is new and
//! the server is old" is the default state, not a corner.

pub mod argv;
pub mod config;
pub mod docserver;
pub mod editors;
pub mod files;
pub mod host;
pub mod hostenv;
pub mod hostpolicy;
pub mod log;
pub mod meta;
pub mod project;
pub mod projects;
pub mod status;
pub mod term;
pub mod watch;
pub mod workspace;
pub mod util;
pub mod viewer;
pub mod ops;
pub mod prefs;
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

    /// Whether `event` emitted now would reach anyone who listens for it. The
    /// app always has its window; a server answers for the clients that have
    /// DECLARED they listen for it (serve's `{"listen": [...]}`), so a client
    /// that never does — the app's own verification probe — is no audience.
    /// What is CONSUMED by being emitted (the inbox's `worktrees show`) has to
    /// wait for somebody rather than go out to nobody.
    fn has_audience(&self, _event: &str) -> bool {
        true
    }
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
    /// The tmux attaches this process holds open.
    pub terminals: Arc<term::Terminals>,
    /// The dock's own shells — PTYs this process OWNS, unlike the attaches
    /// above, which belong to tmux.
    pub shells: Arc<term::Shells>,
    /// The documentation server and what it is currently serving.
    pub viewer: Arc<viewer::Viewer>,
    /// basename → paths per repo, for the Files tab's "find by name".
    pub docnames: Arc<files::DocNameIndex>,
    /// Which projects THIS call may see: every registered one for the engine's
    /// own work (the poll, the shim) and for the console, and the caller's
    /// grant for a request from a scoped device. `projects::read_projects`,
    /// `project_roots` and `guard_under_projects` answer with it, so a body
    /// that enumerates the registry is the second check behind serve's
    /// resolver, and a body that forgets a filter fails closed (Server
    /// Edition Phase 2, §3.3). Set only through [`Ctx::scoped`].
    scope: Arc<worktrees_core::authz::Projects>,
    /// Who is asking the host to fetch, which decides how this call's git
    /// may dial (`worktrees_core::fetchpolicy`): the console for the engine's
    /// own work and the console's requests, and a device, with the addresses
    /// serve's resolver checked, for a paired device's. Set only through
    /// [`Ctx::fetching`], which narrows.
    fetch: Arc<worktrees_core::fetchpolicy::Fetcher>,
    /// What this call's caller holds, for the bodies that ENUMERATE paths: a
    /// directory listing marks, and the docs index and the path links drop,
    /// what it may not read by the path's class (§7.1). serve's resolver
    /// checks only the path a request NAMES. Every capability, and the
    /// console, for the engine's own work and the console's requests. Set only
    /// through [`Ctx::granted`], which narrows.
    caps: worktrees_core::authz::Caps,
    console: bool,
}

impl Ctx {
    pub fn new(config_dir: PathBuf, host: Arc<dyn Host>) -> Self {
        Ctx {
            config_dir,
            host,
            watch: Arc::new(watch::Watch::default()),
            terminals: Arc::new(term::Terminals::default()),
            shells: Arc::new(term::Shells::default()),
            viewer: Arc::new(viewer::Viewer::default()),
            docnames: Arc::new(files::DocNameIndex::default()),
            scope: Arc::new(worktrees_core::authz::Projects::All),
            fetch: Arc::new(worktrees_core::fetchpolicy::Fetcher::Console),
            caps: worktrees_core::authz::Caps::ALL,
            console: true,
        }
    }

    /// The same engine, seen by a caller whose grant reaches `scope`. Every
    /// registry and handle is shared; only the view of the registry narrows.
    /// Narrowing only: a scope is intersected with the one already here, so a
    /// scoped `Ctx` can never be widened by scoping it again.
    pub fn scoped(&self, scope: worktrees_core::authz::Projects) -> Ctx {
        Ctx { scope: Arc::new(self.scope.intersect(&scope)), ..self.clone() }
    }

    /// The projects this `Ctx` may see.
    pub fn scope(&self) -> &worktrees_core::authz::Projects {
        &self.scope
    }

    /// The same engine, for a caller whose fetches are `fetcher`'s. Narrowing
    /// only: a device's `Ctx` stays a device's.
    pub fn fetching(&self, fetcher: worktrees_core::fetchpolicy::Fetcher) -> Ctx {
        Ctx { fetch: Arc::new(self.fetch.narrow(fetcher)), ..self.clone() }
    }

    pub fn fetcher(&self) -> &worktrees_core::fetchpolicy::Fetcher {
        &self.fetch
    }

    /// The same engine, for a caller holding `caps`, and the console or not.
    /// Narrowing only: capabilities are intersected, and a `Ctx` that is not
    /// the console's never becomes it again.
    pub fn granted(&self, caps: worktrees_core::authz::Caps, console: bool) -> Ctx {
        Ctx { caps: self.caps.intersect(caps), console: self.console && console, ..self.clone() }
    }

    /// This caller may `access` a path of `class`: the base capability
    /// (`read` or `write`) and whatever the class adds
    /// (`worktrees_core::pathclass::requirement`).
    pub fn may(&self, class: worktrees_core::pathclass::Class, access: worktrees_core::authz::Access) -> bool {
        use worktrees_core::authz::{Access, Capability};
        let base = match access {
            Access::Read => Capability::Read,
            Access::Write => Capability::Write,
        };
        self.caps.has(base) && worktrees_core::pathclass::requirement(class, access).met_by(self.caps, self.console)
    }

    /// This caller may read a path of every class, which only the console
    /// can. The bodies that would class every path they list skip the work.
    pub fn reads_every_class(&self) -> bool {
        worktrees_core::pathclass::Class::ALL.iter().all(|c| self.may(*c, worktrees_core::authz::Access::Read))
    }

    /// How this call's git fetches `url` (exactly as git is handed it):
    /// today's git for the console; for a device, pinned to the addresses its
    /// request was checked against, or `Err` when it may not fetch `url`.
    pub fn fetch_policy(&self, url: &str) -> Result<worktrees_core::fetchpolicy::Policy, String> {
        self.fetch.policy(url)
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
