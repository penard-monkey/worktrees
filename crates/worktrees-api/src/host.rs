//! What every engine process does besides answer commands.
//!
//! The desktop app's `run()` grew this over two years, one hard-won line at a
//! time: install the log before anything can want it, hook panics into it,
//! repair the launchd-bare `PATH` and locale, sweep the viewer's derived trees,
//! merge the pre-registry project list, start the 3 s poll that lights the nav
//! dots, flush the dock rings on a tick and again on the way out. `worktrees
//! serve` needs every one of those, in the same order, for the same reasons —
//! and AGENTS.md records what a host forgetting ONE of them looks like: pi
//! shipped with a correct core and no nav dot at all, because the poll lived
//! in the app and the app's poll was never told about pi. Three reviews passed
//! it. The fix for that class of bug is for the checklist to be code, so the
//! wiring is here, once, and both hosts call it: `boot` → `start` → `stop`.
//!
//! Two things stay with the host, because they are about a window: the log
//! SINK (where the file is, and what identifier it logs under), and raising the
//! window when the inbox delivers a request. `boot` takes the first; `start`
//! takes the second as an optional hook, which a server leaves empty.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use worktrees_core::{provider, sysclock};

use crate::log::{self, LogSink};
use crate::ops::{
    draft_trace_filter, fetch_origin_root, scan_drafts, trace_spawns, Draft, Drafts,
    FETCH_INTERVAL_SECS,
};
use crate::projects::{import_app_projects, read_projects};
use crate::watch::{
    announce_done, backfill_worked, claude_activity, completion_edges, drain_inbox, harness_feed, merge_activity,
    new_dones, Activity, ClaudeActivity, Feed, Inbox, LaneTick,
};
use crate::{term, viewer, workspace, Ctx};

/// The process-level preamble, before a `Ctx` exists.
///
/// In order, and the order is the point: the log sink FIRST (the panic hook and
/// the startup line both want it, and everything the command layer logs before
/// it is installed vanishes silently); the panic hook chained in front of the
/// default one; the GUI-environment repairs (`PATH`, locale) ahead of any
/// shell-out; core's own sink handed the same destination, once for the whole
/// engine; then the startup line, which stamps the version — a long-lived
/// process outlives an upgrade, and that line is what makes "which binary
/// wrote this log" answerable afterwards.
///
/// Returns what `log::install` returned: `false` means a sink was already
/// installed, which is a bug in the host's startup rather than something to
/// paper over.
pub fn boot(sink: Arc<dyn LogSink>) -> bool {
    let took = log::install(sink);
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::log("panic", &panic_line(info));
        prev_hook(info);
    }));
    crate::hostenv::fixup_gui_path();
    crate::hostenv::fixup_gui_locale();
    // Both usage readers live in core and lost their direct log in the move
    // there. Hand it back ONCE for the whole engine: core treats an unset sink
    // as "do not log" rather than as a reason to fail, and a per-module sink
    // would be a list to keep in step with every reader core gains.
    worktrees_core::logsink::install(|level, msg| log::log(level, msg));
    log::info(&format!(
        "startup v{} PATH={}",
        env!("CARGO_PKG_VERSION"),
        std::env::var("PATH").unwrap_or_default()
    ));
    took
}

/// A panic as ONE log line: `panicked at <file:line:col>: "<message>"`, the
/// message escaped and bounded by `log::panic_text`. `info.to_string()` is two
/// lines, the location and then the message raw on its own line, so
/// `grep '[panic]'` found where and never what, and a message carrying a
/// client's text could start a line that read as the log's own.
fn panic_line(info: &std::panic::PanicHookInfo<'_>) -> String {
    let what = log::panic_text(info.payload());
    match info.location() {
        Some(at) => format!("panicked at {at}: {what}"),
        None => format!("panicked: {what}"),
    }
}

/// What a host may want to be told by the poll. Everything here is optional,
/// because a server has no window to do any of it with.
#[derive(Default)]
pub struct Hooks {
    /// The inbox delivered a `worktrees show` request this tick. The command
    /// layer has already emitted `app:open-doc`; the desktop raises its window
    /// so the person sees it. A phone has no window of ours, so a server
    /// leaves this `None` and the event is the whole answer.
    pub on_inbox: Option<Box<dyn Fn() + Send + 'static>>,
}

/// Start the engine's background work for `ctx`, and return the poll thread.
///
/// Does, in order: the viewer sweep (COPIES of the user's documents that a
/// crash left on disk — see `viewer::cleanup`), the one-way merge of the app's
/// pre-registry project list, and then the poll thread, which begins with the
/// cold-start pass (`backfill_worked`, `forget_vanished_places`,
/// `sweep_term_history`) and loops every 3 s for the life of the process.
///
/// The thread holds its own CLONE of `ctx`. That is why `Ctx::clone` must
/// share its registries (headless.rs asserts it): the poll is the hottest
/// reader of the watch registry and the shell map, and a copy that did not
/// share would read an empty one forever with nothing logged.
pub fn start(ctx: Ctx, hooks: Hooks) -> std::thread::JoinHandle<()> {
    // Not a lifecycle hook, a startup sweep: it binds no port, reads no
    // configuration and cannot fail in a way that matters. A crash is the one
    // exit `stop` never sees, and the derived trees it leaves are the user's
    // documents outside the repo's own gitignore, indexed by Spotlight.
    viewer::cleanup(&ctx.config_dir);
    import_app_projects(&ctx);
    std::thread::Builder::new()
        .name("engine-poll".into())
        .spawn(move || poll(ctx, hooks))
        .expect("could not spawn the engine's poll thread")
}

/// The 3 s poll. Live-refresh, change-gated: the tmux session set is a cheap
/// fingerprint that shifts whenever a place is opened or closed (even from a
/// bare terminal); `places:changed` is emitted only when it changes, so the
/// client's full re-pull (a git sweep) does not fire every few seconds. A slow
/// safety re-emit (~30 s) still catches dirty/branch drift that leaves no tmux
/// trace.
///
/// Ported from the desktop app's `run()` without behavioural change; every
/// comment that explains a beat travelled with it.
fn poll(pollctx: Ctx, hooks: Hooks) {
    // Claude's real working state — read from ~/.claude/sessions/<pid>.json
    // probes (see claude_activity), keyed by worktree path. Replaces the old
    // tmux #{session_activity} signal, which tracked CLIENT attach/keypress,
    // not pane output — so the dot decayed while Claude worked.
    let mut last = worktrees_core::tmux::session_fingerprint();
    let mut busy_gate = BusyGate::default();
    let mut last_models: Option<Vec<(i32, Option<String>)>> = None;
    let mut ticks: u32 = 0;
    // Inbox requests the last tick left waiting for a client, for its log line.
    let mut held: usize = 0;
    // Auto-fetch scheduling. The pass runs INLINE on this thread (no extra
    // thread → passes can never stack; the AtomicU64 interval is pushed from
    // the client). Trade-off: while a pass runs, the 3s tmux-fingerprint poll
    // below is paused for up to ~60s per repo — acceptable, since a stale tmux
    // fingerprint only delays a refresh the fetch itself will trigger via
    // places:changed at the end.
    let mut last_fetch = std::time::Instant::now();
    // Consecutive ticks each path has been busy — the dwell guard's memory, and
    // the only state a completion edge needs.
    let mut busy_ticks: HashMap<String, u32> = HashMap::new();
    // The lane completion last stamped per place, per harness (see `new_dones`).
    let mut done_seen: HashMap<&'static str, HashMap<String, i64>> = HashMap::new();
    // Dock-shell cwd sampling, every 5th tick (~15s). Slow on purpose: the exit
    // hook is the accurate capture, this one only has to bound how much a crash
    // or a force-quit can lose.
    let mut cwd_ticks: u32 = 0;
    // The unsent-prompt sample, on the same 15s beat and for the same reason:
    // it costs a subprocess, and a draft is a thing a human is typing — 15s is
    // faster than anyone notices.
    let mut draft_ticks: u32 = 0;
    let mut last_drafts: Vec<Draft> = Vec::new();
    // Logged separately from the emit gate: the SET changes every time someone
    // adds a word to a prompt, and the log rotates at 1MB. The counts are the
    // part worth keeping.
    let mut last_draft_counts = (usize::MAX, usize::MAX);
    // Only the first of a repeating tmux failure is worth a line.
    let mut last_draft_err = String::new();
    // …and only the first of a repeating viewer-refresh line.
    //
    // A SET, not the last message: the tick emits one line per failing place
    // and one per image a derive refused, so two places (or one place and one
    // skipped screenshot) alternate and a one-slot dedup logs both of them
    // every 3 s forever. Cleared when a tick has nothing to say, so a fault
    // that comes back after a recovery is heard again, and capped so a place
    // producing fresh text on every tick cannot grow it without bound.
    let mut said_viewer: HashSet<String> = HashSet::new();
    // One `Ctx` for the thread's whole life, and the registries out of it: the
    // dot poll is the hottest reader of both and must not re-resolve them 20
    // times a minute.
    let watch = pollctx.watch.clone();
    let watch = watch.as_ref();
    // Cold start: what happened while the engine was down. Runs before the
    // first sleep so the nav's afterglow is right on frame one.
    backfill_worked(&pollctx);
    // Same cold-start slot: drop remembered directories for places that were
    // removed while the engine was down.
    term::forget_vanished_places(&pollctx);
    // Same cold-start slot, same reasoning one level out: drop saved scrollback
    // for places that are gone, and anything past the age horizon.
    term::sweep_term_history(&pollctx);
    loop {
        std::thread::sleep(Duration::from_secs(3));
        let interval = FETCH_INTERVAL_SECS.load(Ordering::Relaxed);
        if interval > 0 && last_fetch.elapsed().as_secs() >= interval {
            for root in read_projects(&pollctx) {
                let _ = fetch_origin_root(&root);
            }
            last_fetch = std::time::Instant::now(); // measure gap AFTER the pass
            pollctx.emit("places:changed", serde_json::Value::Null); // re-pull fresh ahead/behind once
        }
        // Every tick, not every fifth: a request has a 30 s life and a person
        // is waiting on it. The command layer emits the `app:open-doc`; raising
        // the window is the half that cannot move, because a phone has no
        // window of ours — so it is the host's hook. A held request is said
        // when the number moves, not on every tick it sits there.
        match drain_inbox(&pollctx) {
            Inbox::Opened(n) => {
                held = 0;
                if n > 0 {
                    if let Some(f) = hooks.on_inbox.as_ref() {
                        f();
                    }
                }
            }
            Inbox::Held(n) => {
                if n > 0 && n != held {
                    log::info(&format!("inbox: {n} request(s) held until a client listens for them"));
                }
                held = n;
            }
        }
        // And the browser's copy of every place the server is serving. Every
        // tick for the same reason: somebody is watching an agent write, and
        // the page polls its `ETag` about once a second — a slower beat here
        // would be a browser tab that is visibly behind the dock beside it,
        // with a conditional GET saying nothing has changed.
        //
        // It costs a stat per document per registered place and NOTHING when
        // none is registered, which is the normal state; `viewer::refresh` is
        // where that is argued. It is not gated on window visibility, unlike
        // the pane's own poll: the whole point of the browser viewer is the
        // second monitor, so "the app is hidden" is exactly when the tab most
        // needs to be current.
        let notes = viewer::refresh(&pollctx.viewer, sysclock::now_epoch());
        if notes.is_empty() {
            said_viewer.clear();
        }
        for n in notes {
            // Deduped, like the draft scan below: a place that cannot be
            // written will fail every 3 s forever, and a log that repeats that
            // is a log nobody reads. The LEVEL is the tick's, not this call
            // site's: a skipped image is the same event `open_docs_viewer`
            // logs at "warn", and saying it louder here made one event two.
            if said_viewer.len() > 64 {
                said_viewer.clear();
            }
            if said_viewer.insert(n.msg.clone()) {
                log::log(n.level, &format!("viewer refresh: {}", n.msg));
            }
        }
        cwd_ticks += 1;
        if cwd_ticks >= 5 {
            cwd_ticks = 0;
            term::save_shell_cwds(&pollctx, &pollctx.shells);
            term::save_shell_scrollback(&pollctx, &pollctx.shells);
        }
        let fp = worktrees_core::tmux::session_fingerprint();
        ticks += 1;
        if fp != last || ticks >= 10 {
            last = fp.clone(); // `fp` is the draft sample's session list too
            ticks = 0;
            pollctx.emit("places:changed", serde_json::Value::Null);
        }
        draft_ticks += 1;
        if draft_ticks >= 5 {
            draft_ticks = 0;
            // `fp` is the session list this tick already paid for, so the whole
            // pass is ONE extra tmux spawn per 15s regardless of how many
            // sessions are open.
            match scan_drafts(&fp) {
                Ok(drafts) => {
                    last_draft_err.clear();
                    if trace_spawns() {
                        log::info("trace: draft sample = 1 tmux spawn (chained)");
                    }
                    // Emit on EVERY change, the transition to empty included: a
                    // draft the user just sent must take its ✎ with it.
                    if drafts != last_drafts {
                        if let Some(f) = draft_trace_filter() {
                            for d in drafts.iter().filter(|d| d.path.contains(&f)) {
                                let head: String = d.text.chars().take(20).collect();
                                log::info(&format!("draft {} queued={} {:?}", d.path, d.queued, head));
                            }
                        }
                        let counts = (drafts.len(), drafts.iter().filter(|d| d.queued).count());
                        if counts != last_draft_counts {
                            last_draft_counts = counts;
                            log::info(&format!("drafts: {} ({} queued)", counts.0, counts.1));
                        }
                        last_drafts = drafts.clone();
                        pollctx.emit("sessions:drafts", payload(&Drafts { drafts }));
                    }
                }
                // Keep the last known set rather than blanking every ✎ on one
                // bad tick (a pane that died mid-chain).
                Err(e) => {
                    if e != last_draft_err {
                        last_draft_err = e.clone();
                        log::warn(&format!("draft sample: {e}"));
                    }
                }
            }
        }
        let (busy, waiting, models) = claude_activity(watch, &fp);
        // Every harness without a probe file, read once each (`harness_feed`:
        // codex's rollouts, pi's session files).
        let lanes: Vec<(&'static str, LaneTick)> = provider::PROVIDERS
            .iter()
            .filter_map(|p| match harness_feed(watch, p.id, &fp) {
                Feed::Lane(t) => Some((p.id, t)),
                Feed::Probes | Feed::Unpolled => None,
            })
            .collect();
        // A live agent now names a different model (a first reply, a `/model`
        // switch, codex's or pi's next turn) → re-list, so the agent label
        // follows within a tick rather than on the 30s safety tick. `None`
        // first: the launch already listed.
        if lanes.iter().any(|(_, t)| t.models_moved) || last_models.as_ref().is_some_and(|m| *m != models) {
            pollctx.emit("places:changed", serde_json::Value::Null);
        }
        last_models = Some(models);
        let waiting_by: BTreeMap<&'static str, Vec<String>> =
            std::iter::once((provider::CLAUDE.id, waiting.clone()))
                .chain(lanes.iter().map(|(id, t)| (*id, t.waiting.clone())))
                .map(|(id, mut v)| {
                    v.sort_unstable();
                    v.dedup();
                    (id, v)
                })
                .collect();
        let Activity { busy, waiting, edges } = merge_activity(busy, waiting, lanes.iter().map(|(_, t)| t));
        // Change-gated: emit only when a set shifts, so an idle machine stays
        // silent (the client just re-applies the last set) — and on the first
        // tick whatever they are (`BusyGate`).
        let sets = (busy, waiting, waiting_by);
        if busy_gate.moved(&sets) {
            let (busy, waiting, waiting_by) = sets;
            pollctx.emit("sessions:busy", payload(&ClaudeActivity { busy, waiting, waiting_by }));
        }
        // Completion edges, computed AFTER the busy emit so the dot's hand-off
        // (green out, ember in) arrives in that order.
        let exits = completion_edges(&mut busy_ticks, &edges);
        if !exits.is_empty() {
            // read_projects only on a real edge — not every 3s tick
            let roots = read_projects(&pollctx);
            let epoch = sysclock::now_epoch();
            for path in exits {
                announce_done(&pollctx, &roots, path, epoch);
            }
        }
        let fresh: Vec<(String, i64)> = lanes
            .iter()
            .flat_map(|(id, t)| new_dones(done_seen.entry(id).or_default(), &t.done))
            .collect();
        if !fresh.is_empty() {
            let roots = read_projects(&pollctx);
            let now = sysclock::now_epoch();
            for (path, at) in fresh {
                // Bounded by now: the stamp is the agent's clock.
                announce_done(&pollctx, &roots, path, at.min(now));
            }
        }
    }
}

/// The busy, waiting and waiting-by sets one tick read.
type BusySets = (Vec<String>, Vec<String>, BTreeMap<&'static str, Vec<String>>);

/// The `sessions:busy` gate: a tick's sets go out only when they differ from
/// the last ones sent, and on the poll's FIRST tick whatever they are. Serve
/// sends a connection that declares the event the last push
/// (`scope::SNAPSHOT`), so the first tick is what gives an idle host a state
/// to send; seeded with empty sets, a host where nothing was busy at start
/// pushed nothing, and a phone that connected showed no state at all until
/// the first agent moved.
#[derive(Default)]
struct BusyGate(Option<BusySets>);

impl BusyGate {
    /// Whether this tick's `sets` are to be emitted; remembered when they are.
    fn moved(&mut self, sets: &BusySets) -> bool {
        if self.0.as_ref() == Some(sets) {
            return false;
        }
        self.0 = Some(sets.clone());
        true
    }
}

/// An event payload as JSON. The app's `emit` took any `Serialize`; `Host::emit`
/// takes a `Value`, and a payload that cannot be serialized (none of these
/// can fail — they are strings and integers) becomes `null` rather than a
/// missed tick.
fn payload<T: serde::Serialize>(t: &T) -> serde_json::Value {
    serde_json::to_value(t).unwrap_or(serde_json::Value::Null)
}

/// How long `stop` waits for a cancelled clone to finish removing its partial
/// folder. Bounded — exit must not hang.
const CLONE_CANCEL_GRACE: Duration = Duration::from_secs(3);

/// The exit sequence, in the order the app's `RunEvent::Exit` arm ran it.
///
/// Where each dock tab ended up, recorded BEFORE the sweep (a killed shell has
/// no cwd left to read); what each tab was SHOWING, likewise before (a killed
/// shell still has its ring, but not for long); then the shells themselves,
/// which have no tmux server holding them up and would otherwise outlive the
/// process as orphaned logins; then the documentation server, for a harder
/// reason — what it holds is a loopback port onto the user's documents, and
/// surviving the process would mean serving them to whatever finds the port
/// with nothing on screen to say it is still running; then in-flight clones,
/// cancelled and given a moment to clean up; and last the viewer's derived
/// trees, which are copies of documents and belong on disk only while
/// something is serving them.
pub fn stop(ctx: &Ctx) {
    term::save_shell_cwds(ctx, &ctx.shells);
    term::save_shell_scrollback(ctx, &ctx.shells);
    let keys: Vec<term::ShellKey> = ctx.shells.0.lock().unwrap().keys().cloned().collect();
    for k in &keys {
        term::kill_shell(&ctx.shells, k);
    }
    viewer::kill(&ctx.viewer);
    let clones = workspace::CLONE_JOBS.cancel_all();
    if !clones.is_empty() {
        log::info(&format!("exit: cancelling {} clone(s): {}", clones.len(), clones.join(", ")));
        let deadline = std::time::Instant::now() + CLONE_CANCEL_GRACE;
        while !workspace::CLONE_JOBS.is_idle() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !workspace::CLONE_JOBS.is_idle() {
            log::warn("exit: a cancelled clone had not finished cleaning up");
        }
    }
    viewer::cleanup(&ctx.config_dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `stop` on a fresh `Ctx` touches only its own config dir and returns:
    /// no shells to kill, no server to stop, no clones to cancel. What it
    /// must NOT do is panic on an empty process — `serve` calls it from a
    /// signal handler, where a panic is an orphaned serve.json.
    #[test]
    fn stop_on_an_idle_engine_is_a_no_op_that_returns() {
        let dir = std::env::temp_dir().join(format!("wt-host-stop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = Ctx::new(dir.clone(), Arc::new(crate::tests::Recorder::default()));
        stop(&ctx);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The poll's first tick sends the busy sets whatever they are, an idle
    /// host's empty ones included, and only a change after that. Serve sends
    /// a connection that declares `sessions:busy` the last push
    /// (`scope::SNAPSHOT`), so a gate seeded with empty sets left a host
    /// where nothing was busy at start with nothing to send, and a phone that
    /// connected showed no state until the first agent moved.
    #[test]
    fn the_first_tick_sends_the_busy_sets_even_when_they_are_empty() {
        let mut g = BusyGate::default();
        let idle = BusySets::default();
        assert!(g.moved(&idle), "the first tick: an idle host's state is a state");
        assert!(!g.moved(&idle), "then only a change");
        let busy: BusySets = (vec!["/a".into()], Vec::new(), BTreeMap::new());
        assert!(g.moved(&busy));
        assert!(!g.moved(&busy));
        let by: BusySets = (vec!["/a".into()], Vec::new(), BTreeMap::from([("claude", Vec::new())]));
        assert!(g.moved(&by), "the per-harness set is part of the state");
        assert!(g.moved(&idle), "and back to idle is a change");
    }

    /// The hook `boot` installs logs a panic as ONE `[panic]` line: where,
    /// then what, escaped. `info.to_string()` was two lines, the location and
    /// then the message raw on a line of its own, so `grep '[panic]'` found
    /// the where and never the what, and a message carrying a client's text
    /// could start a line that read as the log's own. Run in a CHILD (this
    /// test binary again, filtered to this test) because the hook and the
    /// sink are process-wide: installed here, they would take every other
    /// test's panics.
    #[test]
    fn a_panic_is_one_log_line_with_its_location_and_its_message() {
        const CHILD: &str = "WT_PANIC_LINE_CHILD";
        if let Ok(out) = std::env::var(CHILD) {
            struct ToFile(std::path::PathBuf);
            impl LogSink for ToFile {
                fn log(&self, level: &str, msg: &str) {
                    use std::io::Write;
                    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.0).unwrap();
                    f.write_all(format!("[{level}] {msg}\n").as_bytes()).unwrap();
                }
            }
            assert!(boot(Arc::new(ToFile(out.into()))), "a sink was already installed in the child");
            let _ = std::panic::catch_unwind(|| panic!("the first line\n2026-01-01 00:00:00Z [info] forged\u{202e}"));
            return;
        }
        let out = std::env::temp_dir().join(format!("wt-panic-line-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let st = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "host::tests::a_panic_is_one_log_line_with_its_location_and_its_message", "--quiet"])
            .env(CHILD, &out)
            // `boot`'s login-shell PATH probe, kept quick.
            .env("SHELL", "/bin/sh")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(st.success(), "the child: {st}");
        let text = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        let panics: Vec<&str> = text.lines().filter(|l| l.starts_with("[panic] ")).collect();
        assert_eq!(panics.len(), 1, "{text}");
        let line = panics[0];
        assert!(line.starts_with("[panic] panicked at ") && line.contains("host.rs:"), "{line}");
        assert!(line.contains("the first line") && line.contains("forged"), "the message is not on the [panic] line: {text}");
        assert!(!line.chars().any(|c| c.is_control() || c == '\u{202e}'), "{line:?}");
        assert!(!text.lines().any(|l| l.starts_with("2026-01-01")), "a line of the message stood on its own: {text}");
    }
}
