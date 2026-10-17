//! The nav dots: which places have an agent busy, waiting, or just finished.
//!
//! The app builds its dots in its own poll — `claude_activity` + each
//! harness's tick → `merge_activity` → `sessions:busy` / `sessions:done`.
//! AGENTS.md records why that poll is a surface of its own: pi shipped with a
//! correct core (`place_status` said `busy`) and no dot at all, because core
//! reporting a thing is not the user seeing it. `harness_feed` and
//! `watch_lane` both have drift tests that fail for a provider nobody wired
//! in, and those tests move here with them.
//!
//! ## The registries are `Watch`, not statics
//!
//! `MODEL_CACHE`, `CODEX_WATCH` and `PI_WATCH` were process-global `static`s.
//! They are per-process state by nature — one machine, one set of tmux
//! sessions — but a `static` also makes every test that touches one
//! order-dependent, which AGENTS.md documents biting `viewer::ISSUED`: the
//! suite passed on an accident of alphabetical order. They are fields on
//! `Watch` now, which `Ctx` owns, so a test owns the map it asserts against
//! and the server gets one per process exactly as the app does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use worktrees_core::{git, store, sysclock, Project};

use crate::log::log as applog;
use crate::Ctx;

/// The three registries the dot poll keeps between ticks.
#[derive(Default)]
pub struct Watch {
    /// Last answer per transcript/rollout: its length when read, and the model
    /// it named. Read on the 3s snapshot poll, so a file is re-read only once
    /// it has GROWN — an idle session costs one `stat`.
    pub model_cache: Mutex<HashMap<PathBuf, (u64, Option<String>)>>,
    /// Worktrees with a live codex session: its tmux session name and the
    /// model the last snapshot showed. Codex writes no status file, so nothing
    /// else tells the poll a codex turn landed; the tick re-reads these and
    /// re-lists when a model CHANGES — exactly when the label would, and never
    /// while a turn merely streams. The session name is what the approval
    /// sample captures.
    pub codex: Mutex<HashMap<String, CodexWatched>>,
    /// Live pi lanes, by place path: set by the snapshot (like `codex`) while
    /// pi itself runs in the place's `~agent~pi` session.
    pub pi: Mutex<HashMap<String, PiWatched>>,
}

// The probe reader lives in core (`worktrees_core::agent`) so the MCP server's
// `place_status` and these dots read ONE truth. This is the app's projection of
// it: which worktree PATHS have a busy / waiting claude right now.

/// Scan the live probes and bucket their `cwd`s by state. Returns
/// (busy_cwds, waiting_cwds). Paths are pushed as-is (the probe cwd and a
/// place's `path` both derive from the same worktree dir, so the frontend
/// matches raw). Any I/O or parse failure degrades to empty.
/// Busy and waiting cwds, plus the model each tmux-hosted session currently
/// names (by pid). The snapshot carries that model, and nothing else the poll
/// watches moves when it changes: a `/model` switch touches neither tmux nor
/// the probe's status, and a 2s first reply never spans the two ticks the busy
/// set needs to see it. Costs a `stat` per session per tick, plus a tail read
/// only for a transcript that grew.
pub fn claude_activity(w: &Watch) -> (Vec<String>, Vec<String>, Vec<(i32, Option<String>)>) {
    let mut busy = Vec::new();
    let mut waiting = Vec::new();
    let mut models = Vec::new();
    for probe in worktrees_core::agent::live_probes() {
        if probe.tmux.is_some() {
            models.push((probe.pid, probe_model(w, &probe)));
        }
        // The mapping is core's (`activity::claude_state`), shared with the
        // MCP server's `place_status` / `wait`, so the dot and an orchestrator
        // cannot disagree about what "busy" means.
        match activity::claude_state(&worktrees_core::agent::effective_state(&probe)) {
            activity::State::Busy => busy.push(probe.cwd),
            activity::State::Waiting => waiting.push(probe.cwd),
            _ => {} // idle / shell / parked-away busy (`delegated`) → no dot
        }
    }
    models.sort_unstable();
    (busy, waiting, models)
}

/// Payload for `sessions:busy` — PATHS (worktree dirs), keyed to a place's `path`.
#[derive(Serialize, Clone)]
pub struct ClaudeActivity {
    pub busy: Vec<String>,
    pub waiting: Vec<String>,
    /// `waiting`, per harness id (`claude`, `codex`, `pi`). Additive: the dot
    /// reads the merged set; the nav drag needs to know whether THE agent it
    /// would paste into is the one stopped on a person (cross-project §6.3 —
    /// `drop_reference` refuses per provider, and the chip should agree).
    pub waiting_by: std::collections::BTreeMap<&'static str, Vec<String>>,
}

// ── task-completed stamps (the nav's decaying "afterglow" dot) ───────────────
// A place stops being busy → Claude finished a task there. That instant is worth
// keeping: the green dot vanishing is currently the end of all visibility, and
// "which places did work recently" has no answer at all once it goes.
//
// Why the busy-EXIT edge and not "a session exists": opening or resuming a
// session leaves its probe at `status: "idle"`, so this signal already excludes
// mere presence — the constraint the whole feature hangs on. The dwell guard
// below is the only other filter needed.
//
// The stamp is a monotonic FACT, so it gets its own event (`sessions:done`)
// rather than riding on `sessions:busy`, which is a live SET that reconciles to
// current truth every tick. Merging the two would make a completion something
// the next tick could retract.

/// Consecutive 3s ticks a place must be observed busy before its exit counts as
/// a finished task. Two ticks (~3–6s) discards startup/resume blips; no real
/// prompt turns around that fast.
pub const DONE_DWELL_TICKS: u32 = 2;

/// Payload for `sessions:done` — one place finished a task at `epoch`.
#[derive(Serialize, Clone)]
pub struct TaskDone {
    pub path: String,
    pub epoch: i64,
}

/// Advance the dwell counters by one tick and return the paths that just
/// FINISHED: busy for at least `DONE_DWELL_TICKS` consecutive observations, and
/// no longer busy now. Pure, so the ordering (measure exits, then drop, then
/// count) is testable without a Claude session — it is the one piece here with
/// genuinely tricky sequencing, and there is no fake `claude` to drive it.
///
/// `busy_now` must be de-duplicated; a repeated path would double-count.
///
/// busy→waiting is an exit too: the work finished, and amber merely out-ranks
/// the ember in the dot until the question is answered. (busy→dead-pid is also
/// an exit — a killed session is indistinguishable from a finished one here,
/// which the busy-edge design accepts.)
///
/// Ticks are only as regular as the loop: the inline auto-fetch pass can stall
/// it for up to ~60s per repo, so a task that starts and ends inside a stall is
/// missed entirely. A miss, never a false stamp — and the next cold start's
/// backfill picks it up from history.
pub fn completion_edges(busy_ticks: &mut HashMap<String, u32>, busy_now: &[String]) -> Vec<String> {
    let exits: Vec<String> = busy_ticks
        .iter()
        .filter(|(p, t)| **t >= DONE_DWELL_TICKS && !busy_now.contains(p))
        .map(|(p, _)| p.clone())
        .collect();
    busy_ticks.retain(|p, _| busy_now.contains(p)); // sub-dwell blips drop unstamped
    for p in busy_now {
        *busy_ticks.entry(p.clone()).or_insert(0) += 1;
    }
    exits
}

/// Resolve a session cwd → (repo root, store slug), or `None` when the path is
/// not inside a TRACKED project. The guard matters: `claude_activity` reports
/// every live session on the machine, and without it a Claude run in some
/// unrelated clone would drop a `.worktrees.places.json` into that repo.
///
/// The slug comes from the session's WORKTREE TOP, never from the cwd itself.
/// A session's cwd is wherever the user happened to be — `<place>/app` is
/// entirely normal — and slugging that basename would both invent a phantom
/// store entry ("app") and miss the place that actually did the work. Store
/// keys are `basename(worktree_dir)`, or `(main)` for the main root
/// (`project::place_json`), so the cwd has to be resolved back to that dir
/// first. `--show-toplevel` answers exactly that, including for linked
/// worktrees, and it normalizes a symlinked spelling on the way.
pub fn place_key_for(path: &str, roots: &[String]) -> Option<(String, String)> {
    let project = Project::discover(Path::new(path)).ok()?;
    let root = project.main_root;
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    let root_c = canon(&root);
    if !roots.iter().any(|r| canon(r) == root_c) {
        return None; // untracked repo — not ours to write in
    }
    let top = git::git_out(path, &["rev-parse", "--show-toplevel"])?;
    let top_c = canon(top.trim());
    if top_c == root_c {
        return Some((root, "(main)".to_string()));
    }
    let slug = top_c.file_name()?.to_string_lossy().into_owned();
    Some((root, slug))
}

/// What the frontend is told when a Claude session asks for a document.
///
/// `repo`/`slug` and not just the path: the pane that renders a document is
/// mounted under a SELECTED place, so the request has to name the place before
/// it can name the file.
#[derive(Clone, Serialize)]
pub struct OpenDoc {
    pub repo: String,
    pub slug: String,
    pub path: String,
    /// Where in the file (1-based), when the asker said. Absent from an older
    /// CLI's request, and then the viewer opens at the top.
    pub line: Option<u32>,
    pub col: Option<u32>,
}

/// Serve whatever a session has dropped in `~/.cache/worktrees/inbox`.
///
/// Runs on the 3 s watcher tick. The directory is empty essentially always, so
/// the steady-state cost is one `read_dir` of an empty directory — cheaper than
/// the tmux fingerprint the same tick already pays for.
///
/// Every request is re-validated here even though the writer canonicalised it.
/// It arrived from another process; `guard_under_projects` is the same check
/// every file-reading command makes, and this is the one entry point where the
/// asker is not the user's own click.
/// Drain `worktrees open` requests and emit one `app:open-doc` per accepted
/// one. Returns how many were emitted, because the caller has one more thing
/// to do that this layer cannot: bring the window to the front. A phone has no
/// window of ours to raise, so that half stays in the Tauri shim (§III.4.B
/// classes window calls client-local).
///
/// Focusing ONCE for a drained batch rather than once per request is the only
/// behavioural difference, and it is not one: `show`/`set_focus` are
/// idempotent, and the batch is drained in a single tick.
pub fn drain_inbox(ctx: &Ctx) -> usize {
    let reqs = worktrees_core::inbox::drain(sysclock::now_epoch());
    if reqs.is_empty() {
        return 0;
    }
    let roots = crate::projects::read_projects(ctx);
    let mut opened = 0;
    for r in reqs {
        // Not under a registered project → refused, and SAID so. A silent drop
        // here is a request that vanished with no way to tell whether the app
        // saw it (CLAUDE.md: never swallow errors).
        if crate::projects::guard_under_projects(ctx, &r.path).is_err() {
            applog("warn", &format!("inbox: refused {} (pid {}) — outside every registered project", r.path, r.pid));
            continue;
        }
        // `place_key_for` runs git in the path it is given, so it needs the
        // DIRECTORY — `git -C <a file>` is not a thing.
        let Some(parent) = Path::new(&r.path).parent().map(|p| p.to_string_lossy().to_string()) else {
            continue;
        };
        let Some((repo, slug)) = place_key_for(&parent, &roots) else {
            applog("warn", &format!("inbox: {} is in no tracked place", r.path));
            continue;
        };
        applog("info", &format!("inbox: open {} in {repo}:{slug} (pid {})", r.path, r.pid));
        ctx.emit(
            "app:open-doc",
            serde_json::to_value(OpenDoc { repo, slug, path: r.path, line: r.line, col: r.col })
                .unwrap_or(serde_json::Value::Null),
        );
        opened += 1;
    }
    opened
}

/// Stamp `last_worked_epoch` forward-only. Returns true when the store actually
/// moved, so callers only emit an event for a real change. Never fatal: an
/// untracked path is silent (expected), a write failure is logged.
pub fn stamp_worked(roots: &[String], path: &str, epoch: i64) -> bool {
    let Some((repo, slug)) = place_key_for(path, roots) else {
        return false;
    };
    if store::read_lenient(&repo)
        .places
        .get(&slug)
        .and_then(|d| d.last_worked_epoch)
        .unwrap_or(0)
        >= epoch
    {
        return false; // already know about newer work here
    }
    match store::edit(&repo, &slug, |d| {
        if d.last_worked_epoch.unwrap_or(0) < epoch {
            d.last_worked_epoch = Some(epoch);
        }
    }) {
        Ok(()) => true,
        Err(e) => {
            applog("warn", &format!("worked stamp {path}: {e}"));
            false
        }
    }
}

/// How far back the startup backfill looks. Matches the UI's afterglow horizon —
/// anything older renders as an empty dot slot, so reading it would be waste.
pub const BACKFILL_WINDOW_SECS: i64 = 12 * 3600;
/// Tail of `history.jsonl` to read. The file is append-only and grows without
/// bound (MBs), but 12h of prompts is a few KB; this is slack for pasted blobs.
pub const HISTORY_TAIL_BYTES: u64 = 512 * 1024;
/// Tail of a session TRANSCRIPT to read when dating its last turn. Transcripts
/// reach tens of MB (one line per message, tool results included), and only the
/// newest timestamp is wanted — but a single line can be a pasted file or a
/// whole tool result, so this is slack for a few of those rather than a line
/// count. A tail that lands inside one enormous line yields no whole line at
/// all, which `transcript_epoch` reports as "no answer" rather than a wrong one.
pub const TRANSCRIPT_TAIL_BYTES: u64 = 256 * 1024;
/// Slash commands that are session HOUSEKEEPING, not a task. They land in
/// history.jsonl exactly like a prompt, and without this a `/clear` ten minutes
/// ago would light a place where nothing was done. Unknown slash commands are
/// deliberately NOT filtered — a user's own `/close-out` or `/commit` is work.
pub const NON_WORK_SLASH: &[&str] = &[
    "/clear", "/exit", "/quit", "/help", "/status", "/config", "/model", "/login", "/logout",
    "/cost", "/usage", "/resume", "/compact", "/doctor", "/context", "/permissions", "/mcp",
    "/memory", "/hooks", "/agents", "/add-dir", "/export", "/todos", "/ide", "/statusline",
    "/bug", "/vim", "/terminal-setup", "/release-notes",
];

#[derive(serde::Deserialize)]
pub struct HistLine {
    pub display: Option<String>,
    /// Claude Code has written this as both a JSON number and a quoted string
    /// across versions; parsed leniently below rather than trusted as one shape.
    #[serde(default)]
    pub timestamp: serde_json::Value,
    pub project: Option<String>,
    /// Names THIS prompt's transcript file, which is how the completion time is
    /// refined without letting unrelated activity in the same place count.
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}

pub fn hist_epoch(v: &serde_json::Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        .map(|ms| ms / 1000)
}

/// When a transcript entry was written. Transcript lines carry ISO-8601
/// (`2026-09-20T01:49:52.243Z`); `history.jsonl`'s epoch-millis shape is
/// accepted too, tried second so a date string is never mistaken for a number.
pub fn entry_epoch(v: &serde_json::Value) -> Option<i64> {
    v.as_str().and_then(sysclock::parse_iso8601).or_else(|| hist_epoch(v))
}

/// When a session transcript's newest entry was written, per the timestamps the
/// FILE carries. `None` when the file is missing, unreadable, or its tail holds
/// no whole line with a timestamp.
///
/// ⚠ Deliberately not the file's mtime, which this used to read and which is
/// not a fact about the work. Claude Code keeps rewriting a live session's
/// `.jsonl` long after its last turn: measured across every transcript touched
/// in a day, 24 of 28 had an mtime running from 12 minutes to 34 HOURS ahead of
/// the last entry inside the file. Since the stamp is forward-only and the
/// backfill runs on every launch, reading the mtime re-dated every place with a
/// still-open session to "just finished" at each restart — which the unread
/// rule then renders as a place shouting for attention it has already had, and
/// which the nav reads as the row's age and sort key.
///
/// The max over the tail rather than the last line with a timestamp: a
/// transcript is not strictly ordered (a rewritten summary lands at the end
/// carrying an older stamp), and the caller bounds the answer by `now` anyway.
pub fn transcript_epoch(path: &Path) -> Option<i64> {
    tail_lines(path, TRANSCRIPT_TAIL_BYTES)
        .iter()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("timestamp").and_then(entry_epoch))
        .max()
}


#[derive(Clone)]
pub struct CodexWatched {
    pub session: String,
    pub model: Option<String>,
}

/// The model named in `path`'s tail by `parse`, cached by file length. A tail
/// that now names nothing (one enormous tool result filling it) KEEPS the last
/// answer: the model did not change because a big line landed.
pub fn cached_model(w: &Watch, path: &Path, parse: fn(&[String]) -> Option<String>) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    let mut cache = w.model_cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((l, m)) = cache.get(path) {
        if *l == len {
            return m.clone();
        }
    }
    let prev = cache.get(path).and_then(|(_, m)| m.clone());
    let m = parse(&tail_lines(path, TRANSCRIPT_TAIL_BYTES)).or(prev);
    cache.insert(path.to_path_buf(), (len, m.clone()));
    m
}

/// The model the pane-0 claude in tmux session `session` currently names,
/// display-named (`Opus 5.5`). Found by the probe whose `tmux` names that
/// session — the same session the label sits over, not merely a claude with
/// the same cwd (a dock shell may be running another).
pub fn claude_model(w: &Watch, probes: &[worktrees_core::agent::ClaudeProbe], session: &str) -> Option<String> {
    probe_model(w, probes.iter().find(|p| {
        p.tmux.as_deref().and_then(worktrees_core::agent::session_name) == Some(session)
    })?)
}

pub fn probe_model(w: &Watch, p: &worktrees_core::agent::ClaudeProbe) -> Option<String> {
    let sid = p.session_id.as_deref()?;
    let jsonl = Path::new(&worktrees_core::project::claude_dir_in(&p.root, &p.cwd)).join(format!("{sid}.jsonl"));
    cached_model(w, &jsonl, worktrees_core::agent::transcript_model)
        .map(|id| worktrees_core::agent::model_display(&id))
}

// The rollout tail cache (`codex_tail`) and the whole codex state derivation
// live in core (`worktrees_core::activity`), shared with the MCP server's
// `place_status` / `wait`. What stays here is the scheduling: which sessions
// the tick watches (`CODEX_WATCH`) and what it does with the answer.
use worktrees_core::activity::{self, codex_tail};

/// The model the newest codex session in worktree `cwd` last ran a turn on.
pub fn codex_model(cwd: &str) -> Option<String> {
    codex_tail(&worktrees_core::codex::latest_rollout(cwd)?).0
}

/// Record what a snapshot showed for `cwd`'s codex session: its tmux session
/// while live, `None` when down (which stops the tick watching it).
pub fn codex_watch_set(watch: &Watch, cwd: &str, session: Option<&String>, model: &Option<String>) {
    let mut w = watch.codex.lock().unwrap_or_else(|e| e.into_inner());
    match session {
        Some(session) => {
            w.insert(cwd.to_string(), CodexWatched { session: session.clone(), model: model.clone() });
        }
        None => {
            w.remove(cwd);
        }
    }
}

/// One tick's read of every watched (live) session of ONE harness that has no
/// probe file — codex, pi. Claude's dots come from its probes
/// (`claude_activity`); every other harness answers in this shape
/// (`harness_feed`), so the poll merges them without knowing which is which.
#[derive(Default)]
pub struct LaneTick {
    /// Some session's model differs from what the last snapshot showed.
    pub models_moved: bool,
    /// Places whose agent is mid-turn — merged into `sessions:busy`, so the
    /// nav's green dot needs no harness-specific path.
    pub busy: Vec<String>,
    /// Places stopped on a person (codex: an approval or a question on its
    /// pane; pi: the trust modal) — `sessions:busy`'s waiting set (amber).
    pub waiting: Vec<String>,
    /// (place, epoch) of each session's newest FINISHED turn, dated by the
    /// harness's own record of it. The caller stamps only what moved.
    pub done: Vec<(String, i64)>,
}

/// Read every watched codex session's rollout once: the model the label shows
/// and the turn state the dots show. Only sessions the snapshot saw UP are
/// watched, so a codex killed mid-turn (its rollout ending on `task_started`
/// forever) stops counting as busy as soon as the next snapshot lists it down.
///
/// A mid-turn session may be parked on an approval or a plan-mode question,
/// which nothing codex writes to disk tells apart from a running command
/// (`findings.md`, 2026-09-26) — so those panes, and only those, are captured
/// and read with `codex::waiting_on_screen`. `sessions` is the tick's tmux
/// fingerprint, reused exactly as `scan_drafts` reuses it.
pub fn codex_tick(watch: &Watch, sessions: &str) -> LaneTick {
    let watched: Vec<(String, CodexWatched)> = {
        let guard = watch.codex.lock().unwrap_or_else(|e| e.into_inner());
        guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };
    let mut out = LaneTick::default();
    let mut turns: Vec<(String, Option<worktrees_core::codex::Turn>)> = Vec::new();
    let mut mid_turn: Vec<(String, String)> = Vec::new();
    for (cwd, w) in watched {
        let (model, turn) = match worktrees_core::codex::latest_rollout(&cwd) {
            Some(r) => codex_tail(&r),
            None => (None, None),
        };
        if model != w.model {
            codex_watch_set(watch, &cwd, Some(&w.session), &model);
            out.models_moved = true;
        }
        if matches!(turn, Some(worktrees_core::codex::Turn::Busy)) {
            mid_turn.push((cwd.clone(), w.session));
        }
        turns.push((cwd, turn));
    }
    // Only mid-turn panes are captured: one chained tmux call for all of them,
    // none at all while no codex turn runs.
    let panes = activity::codex_panes(sessions, &mid_turn);
    for (cwd, turn) in turns {
        let pane = panes.iter().find(|(c, _)| *c == cwd).map(|(_, p)| *p);
        // ONE derivation (core's), the same `place_status` and `wait` use.
        let (state, done) = activity::codex_state(turn.as_ref(), pane);
        match state {
            activity::State::Busy => out.busy.push(cwd.clone()),
            activity::State::Waiting => out.waiting.push(cwd.clone()),
            _ => {}
        }
        if let Some(at) = done {
            out.done.push((cwd, at));
        }
    }
    out
}


#[derive(Clone, Debug, PartialEq)]
pub struct PiWatched {
    pub session: String,
    /// The launch the snapshot saw (`PaneList::session_launch`): a close +
    /// open reuses the session NAME, and must start the startup read over.
    pub launch: Option<String>,
    pub model: Option<String>,
    /// The session file (path, length) the first tick found — `Some(None)`
    /// when it found none — so a later tick can tell the file has MOVED.
    pub first_file: Option<Option<(PathBuf, u64)>>,
    /// Past startup: the file moved since the watch began, or a capture showed
    /// pi's composer. Until then the file cannot rule out the trust modal (a
    /// resumed session's file ends idle UNDER the modal), so the pane is read.
    pub settled: bool,
}

/// Record what a snapshot showed for `cwd`'s pi lane: its tmux session while
/// pi runs there, `None` when not (which stops the tick watching it). The
/// startup memory survives a re-list of the same session and resets for a
/// new one — a relaunch is a new startup, modal and all.
pub fn pi_watch_set(watch: &Watch, cwd: &str, session: Option<&String>, launch: Option<&str>, model: &Option<String>) {
    let mut w = watch.pi.lock().unwrap_or_else(|e| e.into_inner());
    match session {
        Some(session) => {
            let launch = launch.map(str::to_string);
            let keep = w.get(cwd).filter(|x| x.session == *session && x.launch == launch).cloned();
            let (first_file, settled) = keep.map_or((None, false), |x| (x.first_file, x.settled));
            w.insert(cwd.to_string(), PiWatched { session: session.clone(), launch, model: model.clone(), first_file, settled });
        }
        None => {
            w.remove(cwd);
        }
    }
}

/// One tick's read of every watched pi lane. The state is core's
/// `pi::pi_state` — the same derivation `place_status` makes — fed the lane's
/// session file and, only where the file cannot answer, one capture of its
/// pane: while the lane is still starting (blank pane, or the trust modal:
/// `waiting`) and while it has no turn on file at all. Every such pane goes
/// in ONE chained `tmux` call; a settled lane with a turn costs a `read_dir`
/// and a `stat`. What the dots then miss that `place_status` would see is a pi
/// killed mid-turn, whose file ends busy forever — the snapshot stops
/// watching it on the next re-list, exactly as for codex.
pub fn pi_tick(watch: &Watch, sessions: &str) -> LaneTick {
    let mut watched: HashMap<String, PiWatched> = {
        let guard = watch.pi.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    };
    let before = watched.clone();
    let out = pi_tick_in(&mut watched, worktrees_core::pi::lane_file, |targets| {
        worktrees_core::pi::pi_panes(sessions, targets)
    });
    // Write back only what this tick learned, and only for lanes the snapshot
    // has not replaced or dropped in the meantime.
    let mut w = watch.pi.lock().unwrap_or_else(|e| e.into_inner());
    for (cwd, now) in watched {
        if before.get(&cwd) == w.get(&cwd) {
            w.insert(cwd, now);
        }
    }
    out
}

/// `pi_tick`'s body over an explicit watch map, file reader and pane
/// sampler, so a test drives it with a fixture file and a canned screen.
pub fn pi_tick_in(
    watched: &mut HashMap<String, PiWatched>,
    file_of: impl Fn(&str) -> Option<worktrees_core::pi::LaneFile>,
    panes_of: impl FnOnce(&[(String, String)]) -> Vec<(String, worktrees_core::pi::PiScreen, bool)>,
) -> LaneTick {
    let mut out = LaneTick::default();
    let mut files: Vec<(String, Option<worktrees_core::pi::LaneFile>)> = Vec::new();
    let mut targets: Vec<(String, String)> = Vec::new();
    for (cwd, w) in watched.iter_mut() {
        let file = file_of(cwd);
        let at = file.as_ref().map(|f| (f.path.clone(), f.len));
        match &w.first_file {
            None => w.first_file = Some(at),
            Some(first) if *first != at => w.settled = true,
            _ => {}
        }
        let model = file.as_ref().and_then(|f| f.model.clone());
        if model.is_some() && model != w.model {
            w.model = model;
            out.models_moved = true;
        }
        if !w.settled || file.as_ref().and_then(|f| f.turn.as_ref()).is_none() {
            targets.push((cwd.clone(), w.session.clone()));
        }
        files.push((cwd.clone(), file));
    }
    let panes = if targets.is_empty() { Vec::new() } else { panes_of(&targets) };
    for (cwd, file) in files {
        let pane = panes.iter().find(|(c, ..)| *c == cwd);
        if pane.is_some_and(|(_, _, prompt)| *prompt) {
            if let Some(w) = watched.get_mut(&cwd) {
                w.settled = true;
            }
        }
        // ONE derivation (core's), the same `place_status` and `wait` use.
        let turn = file.and_then(|f| f.turn);
        let (state, done) = worktrees_core::pi::pi_state(turn.as_ref(), pane.map(|(_, s, _)| *s));
        match state {
            activity::State::Busy => out.busy.push(cwd.clone()),
            activity::State::Waiting => out.waiting.push(cwd.clone()),
            _ => {}
        }
        if let Some(at) = done {
            out.done.push((cwd, at));
        }
    }
    out
}

/// How the poll reads a harness's dots. There is deliberately no `_ => Lane`:
/// a harness added to `provider::PROVIDERS` lands in `Unpolled` until someone
/// wires it here, and `every_harness_feeds_the_dot_poll` fails until they do
/// — pi shipped with no dot for exactly this want of an arm.
pub enum Feed {
    /// Claude: `claude_activity`, from its probe files.
    Probes,
    Lane(LaneTick),
    Unpolled,
}

pub fn harness_feed(watch: &Watch, id: &str, sessions: &str) -> Feed {
    match id {
        "claude" => Feed::Probes,
        "codex" => Feed::Lane(codex_tick(watch, sessions)),
        "pi" => Feed::Lane(pi_tick(watch, sessions)),
        _ => Feed::Unpolled,
    }
}

/// The snapshot's half of the same wiring: record a live harness session for
/// the tick to watch (`session` is `None` when that harness is not running in
/// the place). `false` for a harness nothing watches — the drift test's other
/// arm.
pub fn watch_lane(watch: &Watch, id: &str, cwd: &str, session: Option<&String>, launch: Option<&str>, model: &Option<String>) -> bool {
    match id {
        "claude" => true, // probes are global; nothing to watch per place
        "codex" => {
            codex_watch_set(watch, cwd, session, model);
            true
        }
        "pi" => {
            pi_watch_set(watch, cwd, session, launch, model);
            true
        }
        _ => false,
    }
}

/// One tick's dot sets: what `sessions:busy` carries, and what the dwell
/// counter sees. Lane harnesses (codex, pi) are merged into the first two
/// only — each dates its own finished turns (`LaneTick::done`, stamped via
/// `new_dones`), and an interrupted one (codex `turn_aborted`, pi `aborted`)
/// is not work to announce; feeding them to `completion_edges` too would stamp
/// every turn twice, and the second stamp would say an Esc was finished work.
pub struct Activity {
    pub busy: Vec<String>,
    pub waiting: Vec<String>,
    pub edges: Vec<String>,
}

/// Merge claude's and every lane harness's sets for one tick, sorted and
/// de-duplicated (two sessions in one dir push its cwd twice; the dwell
/// counter would count it twice and let a blip through).
pub fn merge_activity<'a>(
    claude_busy: Vec<String>,
    claude_waiting: Vec<String>,
    lanes: impl IntoIterator<Item = &'a LaneTick> + Clone,
) -> Activity {
    let norm = |mut v: Vec<String>| {
        v.sort_unstable();
        v.dedup();
        v
    };
    let edges = norm(claude_busy.clone());
    let busy = norm(claude_busy.into_iter().chain(lanes.clone().into_iter().flat_map(|l| l.busy.iter().cloned())).collect());
    let waiting = norm(claude_waiting.into_iter().chain(lanes.into_iter().flat_map(|l| l.waiting.iter().cloned())).collect());
    Activity { busy, waiting, edges }
}

/// The lane completions not yet stamped: those whose epoch differs from the
/// one last seen for that place. The store stamp is forward-only anyway, but
/// reaching it costs a git spawn (`place_key_for`), so a turn that finished an
/// hour ago must not be re-offered every 3s. Pure, for the test; the memory is
/// the caller's (one per harness).
pub fn new_dones(seen: &mut HashMap<String, i64>, done: &[(String, i64)]) -> Vec<(String, i64)> {
    let fresh: Vec<(String, i64)> = done.iter().filter(|(p, at)| seen.get(p) != Some(at)).cloned().collect();
    for (p, at) in &fresh {
        seen.insert(p.clone(), *at);
    }
    fresh
}

pub fn is_work_prompt(display: &str) -> bool {
    let t = display.trim();
    if t.is_empty() {
        return false;
    }
    if !t.starts_with('/') {
        return true;
    }
    let head = t.split_whitespace().next().unwrap_or(t);
    !NON_WORK_SLASH.contains(&head)
}

/// Read the tail of a file as whole lines (the first, possibly-truncated line is
/// dropped). Returns empty on any I/O failure — a backfill is a nicety.
///
/// Decoded LOSSILY on purpose. Seeking to a byte offset lands mid-character
/// whenever the boundary falls inside a multi-byte char — routine in a file full
/// of pasted prompts — and a strict decode would throw away the whole tail, not
/// just the fragment that is discarded anyway. Worse, the boundary only moves as
/// the file grows, so a strict failure would be silent AND sticky.
///
/// The reader itself is core's (`activity::tail_lines_checked`), shared with
/// the codex rollout tail; this wrapper only puts its failures in the app log.
pub fn tail_lines(path: &Path, max_bytes: u64) -> Vec<String> {
    match activity::tail_lines_checked(path, max_bytes) {
        Ok(lines) => lines,
        Err(e) => {
            applog("warn", &e);
            Vec::new()
        }
    }
}

/// Backfill afterglow from `history.jsonl` at startup, so a machine where the
/// app was closed all night still shows what was worked on. Live observation
/// (the poll thread) can only see completions while the app runs; this is the
/// half that survives a cold start.
///
/// Prompt time is when work STARTED, so where a transcript exists the newest
/// timestamp INSIDE it is taken as the better completion time — bounded by now,
/// and only for places a qualifying prompt already vouched for. Dating by a
/// transcript with no prompt vouching for it would re-light every place merely
/// opened, which is exactly what must not happen — and dating by that file's
/// MTIME re-lights every place whose session is still UP, on every launch,
/// which is what this used to do (see `transcript_epoch`).
pub fn backfill_worked(ctx: &Ctx) {
    let roots = crate::projects::read_projects(ctx);
    if roots.is_empty() {
        return;
    }
    let now = sysclock::now_epoch();
    let cutoff = now - BACKFILL_WINDOW_SECS;
    let mut newest: HashMap<String, i64> = HashMap::new();
    // One read per transcript, not per prompt: a busy afternoon leaves dozens of
    // history lines naming the same session file, and each read is a 256K tail.
    let mut landed: HashMap<PathBuf, Option<i64>> = HashMap::new();
    for root in worktrees_core::profile::claude_config_dirs_all() {
        for line in tail_lines(&root.join("history.jsonl"), HISTORY_TAIL_BYTES) {
            let Ok(h) = serde_json::from_str::<HistLine>(&line) else {
                continue;
            };
            let (Some(project), Some(epoch)) = (h.project, hist_epoch(&h.timestamp)) else {
                continue;
            };
            if epoch < cutoff || !is_work_prompt(h.display.as_deref().unwrap_or("")) {
                continue;
            }
            let mut stamp = epoch;
            // Refine upward to when the work actually LANDED: the prompt only
            // says when it was asked for, and a long task can finish an hour
            // later. Strictly THIS prompt's own transcript — the dir's newest
            // file would let a later `/clear` (which starts a fresh session
            // file) drag a ten-hour-old prompt up to "just finished", quietly
            // undoing the denylist above.
            if let Some(sid) = h.session_id.as_deref() {
                let cdir = worktrees_core::project::claude_dir_in(&root, &project);
                let jsonl = Path::new(&cdir).join(format!("{sid}.jsonl"));
                let m = *landed.entry(jsonl.clone()).or_insert_with(|| transcript_epoch(&jsonl));
                if let Some(m) = m {
                    if m > stamp {
                        stamp = m.min(now);
                    }
                }
            }
            let slot = newest.entry(project).or_insert(0);
            *slot = (*slot).max(stamp);
        }
    }
    let mut stamped = false;
    for (path, epoch) in newest {
        if stamp_worked(&roots, &path, epoch) {
            stamped = true;
            ctx.emit("sessions:done", serde_json::to_value(TaskDone { path, epoch }).unwrap_or(serde_json::Value::Null));
        }
    }
    // The emits above almost certainly land before the webview has registered
    // its listener (this runs at thread spawn), and a dropped event has no
    // retry. One `places:changed` re-pulls the snapshot, which carries the same
    // stamps durably — otherwise the afterglow stays dark until the ~30s safety
    // re-emit, on exactly the launch where it has the most to say.
    if stamped {
        ctx.emit("places:changed", serde_json::Value::Null);
    }
}


/// The model a live harness session names. Here rather than beside the
/// snapshot because Claude's reader caches transcript tails in `Watch`.
pub fn live_model(
    w: &Watch,
    id: &str,
    probes: &[worktrees_core::agent::ClaudeProbe],
    session: &str,
    cwd: &str,
) -> Option<String> {
    match id {
        "claude" => claude_model(w, probes, session),
        "codex" => codex_model(cwd),
        // By the place's DIR, as codex and the tick read it — never through
        // the session name, which a pi in the canonical session does not carry
        // the sidecar suffix of (a mismatch there re-listed every 3s).
        "pi" => worktrees_core::pi::running_model(session, cwd),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn v(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    /// The dwell guard: a place must be seen busy on two consecutive ticks
    /// before leaving counts as finished work. One tick is a blip (a resume, a
    /// probe written mid-transition) and must stamp nothing.
    #[test]
    fn completion_edge_needs_two_consecutive_busy_ticks() {
        let mut t = HashMap::new();
        assert!(completion_edges(&mut t, &v(&["/a"])).is_empty()); // tick 1: seen once
        assert!(completion_edges(&mut t, &[]).is_empty()); // gone after ONE tick → blip
        assert!(t.is_empty(), "a discarded blip must not linger in the counters");

        assert!(completion_edges(&mut t, &v(&["/a"])).is_empty());
        assert!(completion_edges(&mut t, &v(&["/a"])).is_empty()); // still busy on tick 2
        assert_eq!(completion_edges(&mut t, &[]), v(&["/a"]));
        assert!(completion_edges(&mut t, &[]).is_empty()); // and only ONCE
    }

    /// Independent places must not interfere: one finishing says nothing about
    /// another still working.
    #[test]
    fn completion_edges_track_places_independently() {
        let mut t = HashMap::new();
        for _ in 0..2 {
            completion_edges(&mut t, &v(&["/a", "/b"]));
        }
        assert_eq!(completion_edges(&mut t, &v(&["/b"])), v(&["/a"]));
        assert_eq!(completion_edges(&mut t, &[]), v(&["/b"]));
    }

    /// Two sessions in the same dir report the same cwd twice. The caller
    /// de-dupes; if it ever stops, the dwell guard would silently halve.
    #[test]
    fn duplicate_paths_would_double_count_the_dwell() {
        let mut t = HashMap::new();
        completion_edges(&mut t, &v(&["/a", "/a"]));
        assert_eq!(t["/a"], 2, "documents WHY the caller must dedup before this");
    }

    /// A codex turn is merged into the DOTS but must never reach the dwell
    /// counter: it is stamped from its own `task_complete`, and feeding it to
    /// `completion_edges` as well would stamp it twice — once more at the
    /// busy-exit, which an Esc also triggers.
    #[test]
    fn a_codex_turn_is_a_dot_but_never_a_completion_edge() {
        let codex = LaneTick { busy: v(&["/codex"]), waiting: v(&["/asking"]), ..Default::default() };
        let act = merge_activity(v(&["/claude", "/claude"]), vec![], [&codex]);
        assert_eq!(act.busy, v(&["/claude", "/codex"]));
        assert_eq!(act.waiting, v(&["/asking"]));
        let mut t = HashMap::new();
        completion_edges(&mut t, &act.edges);
        completion_edges(&mut t, &act.edges);
        assert!(!t.contains_key("/codex"), "codex reached the dwell counter: {t:?}");
        assert_eq!(t["/claude"], 2, "claude is counted once per tick, de-duplicated");
    }

    /// The miss this guards: pi joined `provider::PROVIDERS`, core's
    /// `place_status` read it, and the app's dot poll never did — no dot, no
    /// amber, no afterglow for any pi lane (v0.34.0). Both halves of the
    /// wiring are checked: the tick reads the harness, and the snapshot hands
    /// the tick its live sessions.
    #[test]
    fn every_harness_feeds_the_dot_poll() {
        // Its own registry, so this cannot be perturbed by — or perturb —
        // any other test's lanes. That is the whole reason `Watch` is a
        // value and not a `static`.
        let w = Watch::default();
        for p in worktrees_core::provider::PROVIDERS {
            assert!(
                !matches!(harness_feed(&w, p.id, ""), Feed::Unpolled),
                "{} is in PROVIDERS but the nav dot poll never reads it (`harness_feed`)",
                p.id
            );
            assert!(
                watch_lane(&w, p.id, "/nonexistent/drift-check", None, None, &None),
                "{} is in PROVIDERS but the snapshot never hands its sessions to the poll (`watch_lane`)",
                p.id
            );
        }
    }

    const PI_G1: &str = include_str!(
        "../../../crates/worktrees-core/tests/fixtures/pi-session/restart/2026-09-30T00-51-41-893Z_lane-abc123-g1.jsonl"
    );
    const PI_LANE: &str = "/tmp/wtfix/repo/.worktrees/lane";

    fn pi_watch(session: &str) -> HashMap<String, PiWatched> {
        HashMap::from([(
            PI_LANE.to_string(),
            PiWatched { session: session.into(), launch: None, model: None, first_file: None, settled: false },
        )])
    }

    /// A pi lane's dot, end to end through the tick: a real 0.99.1 session file
    /// (cut to end on the user's message, i.e. mid-turn) is BUSY in the merged
    /// set, a finished one is a completion dated by its own entry, and the pane
    /// is captured only while the lane is starting.
    #[test]
    fn a_busy_pi_lane_lights_the_nav_dot_and_its_finish_is_stamped() {
        use worktrees_core::pi::{lane_file_in, PiScreen};
        let d = std::env::temp_dir().join(format!("wtapp-pi-tick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("2026-09-30T00-51-41-893Z_lane-abc123-g1.jsonl");
        let mid: Vec<&str> = PI_G1.lines().collect();
        std::fs::write(&f, mid[..mid.len() - 1].join("\n") + "\n").unwrap();
        let file = |cwd: &str| lane_file_in(&d, cwd, None);

        let mut w = pi_watch("lane~agent~pi");
        // Tick 1: starting, so the pane is read — pi at its composer settles it.
        let mut asked = Vec::new();
        let t = pi_tick_in(&mut w, file, |targets| {
            asked = targets.to_vec();
            vec![(PI_LANE.to_string(), PiScreen::Working, true)]
        });
        assert_eq!(asked, vec![(PI_LANE.to_string(), "lane~agent~pi".to_string())]);
        let act = merge_activity(vec![], vec![], [&t]);
        assert_eq!(act.busy, vec![PI_LANE.to_string()], "a mid-turn pi lane is a green dot");
        assert!(act.edges.is_empty(), "pi is dated by its own turns, never the dwell counter");
        assert!(t.models_moved, "the first read names the model");
        assert!(w[PI_LANE].settled);

        // Tick 2: settled, and the file answers — no capture at all.
        let t = pi_tick_in(&mut w, file, |_| panic!("a settled lane with a turn on file was captured"));
        assert_eq!(t.busy, vec![PI_LANE.to_string()]);
        assert!(!t.models_moved);

        // The reply lands: not busy, and a completion at the reply's own time.
        std::fs::write(&f, PI_G1).unwrap();
        let t = pi_tick_in(&mut w, file, |_| panic!("captured"));
        assert!(t.busy.is_empty() && t.waiting.is_empty());
        let at = worktrees_core::sysclock::parse_iso8601("2026-09-30T00:52:19.068Z").unwrap();
        assert_eq!(t.done, vec![(PI_LANE.to_string(), at)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A RESUMED lane in `ask` mode: its file ends on a finished turn and the
    /// trust modal sits on the pane. The file alone says idle; the dot must be
    /// amber, and must stay read from the pane until pi gets past it.
    #[test]
    fn a_pi_trust_modal_is_an_amber_dot_until_the_composer_shows() {
        use worktrees_core::pi::{lane_file_in, PiScreen};
        let d = std::env::temp_dir().join(format!("wtapp-pi-modal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("2026-09-30T00-51-41-893Z_lane-abc123-g1.jsonl"), PI_G1).unwrap();
        let file = |cwd: &str| lane_file_in(&d, cwd, None);
        let mut w = pi_watch("lane~agent~pi");
        for _ in 0..2 {
            let t = pi_tick_in(&mut w, file, |_| vec![(PI_LANE.to_string(), PiScreen::TrustModal, false)]);
            assert_eq!(t.waiting, vec![PI_LANE.to_string()], "the trust modal is pi's waiting");
            assert!(t.busy.is_empty());
            assert!(!w[PI_LANE].settled, "a modal is not the composer");
        }
        let t = pi_tick_in(&mut w, file, |_| vec![(PI_LANE.to_string(), PiScreen::Other, true)]);
        assert!(t.waiting.is_empty());
        assert!(w[PI_LANE].settled);
        let _ = std::fs::remove_dir_all(&d);

        // No file at all (pi still starting, opener not yet submitted): read
        // the pane every tick, and a blank one is no dot.
        let empty = std::env::temp_dir().join(format!("wtapp-pi-none-{}", std::process::id()));
        let mut w = pi_watch("lane~agent~pi");
        for _ in 0..2 {
            let mut asked = false;
            let t = pi_tick_in(&mut w, |c| lane_file_in(&empty, c, None), |_| {
                asked = true;
                vec![(PI_LANE.to_string(), PiScreen::Other, false)]
            });
            assert!(asked && t.busy.is_empty() && t.waiting.is_empty() && t.done.is_empty());
        }
    }

    /// A re-list of the SAME live launch keeps what the tick learned about
    /// startup; a new launch starts over, modal and all — including a close +
    /// open, which recreates the SAME `~agent~pi` name: keyed on the name, a
    /// resumed lane's trust modal (ask mode) was never captured and the place
    /// stayed dark instead of amber.
    #[test]
    fn a_pi_relaunch_resets_the_startup_memory() {
        // Its OWN registry. This test used to poke the process-global
        // `PI_WATCH` and said so — "the map is process-global and shared with
        // the drift test's `pi_tick`" — which is the order-dependence
        // AGENTS.md records biting `viewer::ISSUED`. A `Watch` per test ends
        // it, and the assertions are unchanged.
        let w = Watch::default();
        let cwd = "/nonexistent/pi-relaunch";
        let s = "x~agent~pi".to_string();
        let settled = || w.pi.lock().unwrap()[cwd].settled;
        let settle = || w.pi.lock().unwrap().get_mut(cwd).unwrap().settled = true;
        pi_watch_set(&w, cwd, Some(&s), Some("101"), &None);
        settle();
        pi_watch_set(&w, cwd, Some(&s), Some("101"), &Some("m".into()));
        assert!(settled(), "same launch: kept");
        pi_watch_set(&w, cwd, Some(&s), Some("202"), &None);
        assert!(!settled(), "same name, new launch (close + open): reset");
        settle();
        pi_watch_set(&w, cwd, Some(&"y~agent~pi".to_string()), Some("202"), &None);
        assert!(!settled(), "new session: reset");
        pi_watch_set(&w, cwd, None, None, &None);
        assert!(!w.pi.lock().unwrap().contains_key(cwd));
    }

    /// The snapshot's pi model is read by the place's DIR, like codex's — never
    /// through the session name. Stripping `~agent~pi` from it gave `None` for
    /// a pi running in the canonical session while the tick (by dir) read the
    /// model: a mismatch every 3s, each one a `places:changed` and a git
    /// fan-out over every project.
    #[test]
    fn the_snapshots_pi_model_does_not_depend_on_the_session_name() {
        let root = std::env::temp_dir().join(format!("wtapp-pi-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let agent = root.join("agent");
        let cwd = root.join("place");
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.to_str().unwrap().to_string();
        let dir = worktrees_core::pi::session_dir_in(&agent, &cwd);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("2026-09-30T00-51-41-893Z_lane-abc123-g1.jsonl"), PI_G1.replace(PI_LANE, &cwd).replace("2026-09-30T", "2099-09-30T")).unwrap();
        // Dated after the place dir was made, or `current_session` rightly
        // treats it as a removed place's leftover.
        // Process-global, and read by nothing else in this crate's tests
        // except for paths that do not exist.
        std::env::set_var("PI_CODING_AGENT_DIR", &agent);
        let w = Watch::default();
        let canonical = live_model(&w, "pi", &[], "repo-place", &cwd);
        let sidecar = live_model(&w, "pi", &[], "repo-place~agent~pi", &cwd);
        std::env::remove_var("PI_CODING_AGENT_DIR");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(sidecar.as_deref(), Some("lm-studio/qwen/qwen3-coder-480b"));
        assert_eq!(canonical, sidecar, "a pi in the canonical session names the same model");
    }

    // `codex_panes_split_the_chain_per_pane` moved to core with the parser
    // (`worktrees_core::activity`), where the MCP server's reading shares it.

    /// A finished codex turn stays the rollout's newest boundary until the next
    /// one starts, so it is offered on every tick. Only a NEW epoch may reach
    /// `stamp_worked` (a git spawn each), and a later turn in the same place
    /// must still get through.
    #[test]
    fn a_codex_completion_is_stamped_once_per_turn() {
        let mut seen = HashMap::new();
        let d = |p: &str, at: i64| vec![(p.to_string(), at)];
        assert_eq!(new_dones(&mut seen, &d("/a", 100)), d("/a", 100));
        assert!(new_dones(&mut seen, &d("/a", 100)).is_empty());
        assert_eq!(new_dones(&mut seen, &d("/a", 160)), d("/a", 160));
        assert_eq!(new_dones(&mut seen, &d("/b", 100)), d("/b", 100), "places are independent");
    }

    /// The backfill's only judgement call: which history lines are WORK. Slash
    /// housekeeping lands in history.jsonl exactly like a prompt, so without the
    /// denylist a `/clear` would light a place where nothing happened — the one
    /// thing the afterglow must never do.
    #[test]
    fn work_prompts_exclude_housekeeping_slashes() {
        assert!(is_work_prompt("fix the flaky test"));
        assert!(is_work_prompt("  /commit -m wip  ")); // unknown slash = user's own command
        assert!(is_work_prompt("/close-out"));
        assert!(!is_work_prompt("/clear"));
        assert!(!is_work_prompt("/resume some-session")); // args must not smuggle it past
        assert!(!is_work_prompt(""));
        assert!(!is_work_prompt("   "));
    }

    /// Claude Code has written `timestamp` as both a number and a quoted string.
    /// Trusting one shape would silently zero the backfill on the other.
    #[test]
    fn history_epoch_accepts_both_shapes() {
        use serde_json::json;
        assert_eq!(hist_epoch(&json!(1_786_318_766_274i64)), Some(1_786_318_766));
        assert_eq!(hist_epoch(&json!("1786318766274")), Some(1_786_318_766));
        assert_eq!(hist_epoch(&json!(null)), None);
        assert_eq!(hist_epoch(&json!("not-a-number")), None);
    }

    /// A transcript line carries ISO-8601; history.jsonl carries epoch millis.
    /// One parser reads both, and a date string must never fall through to the
    /// number branch.
    #[test]
    fn entry_epoch_accepts_iso_and_epoch_millis() {
        use serde_json::json;
        assert_eq!(entry_epoch(&json!("2026-09-20T01:49:52.243Z")), Some(1_789_868_992));
        assert_eq!(entry_epoch(&json!("2026-09-20T01:49:52Z")), Some(1_789_868_992));
        assert_eq!(entry_epoch(&json!(1_786_318_766_274i64)), Some(1_786_318_766));
        assert_eq!(entry_epoch(&json!("nope")), None);
        assert_eq!(entry_epoch(&json!(null)), None);
    }

    /// ⚠ THE regression this function exists for. A transcript is dated by the
    /// timestamps inside it, never by its mtime: Claude Code rewrites a live
    /// session's `.jsonl` for hours after its last turn, and the backfill runs
    /// on every launch with a forward-only stamp — so an mtime read re-dated
    /// every still-open place to "just finished" at each restart, and the unread
    /// ring came back on all of them. The file here is written NOW, so its mtime
    /// is now and its content is hours old: the two answers cannot be confused.
    #[test]
    fn transcript_epoch_dates_a_session_by_its_content_not_its_mtime() {
        let p = std::env::temp_dir().join(format!("wt-tsx-{}.jsonl", std::process::id()));
        std::fs::write(
            &p,
            concat!(
                r#"{"type":"user","timestamp":"2026-09-20T01:40:00.000Z"}"#,
                "\n",
                r#"{"type":"assistant","timestamp":"2026-09-20T01:49:52.243Z"}"#,
                "\n",
                // No timestamp at all: skipped, not treated as a zero.
                r#"{"type":"system","subtype":"turn_duration","durationMs":210986}"#,
                "\n",
                // Out of order — a rewritten summary lands last carrying an
                // older stamp, and the newest entry must still win.
                r#"{"type":"summary","timestamp":"2026-09-20T01:45:00.000Z"}"#,
                "\n",
                "not json at all\n",
            ),
        )
        .unwrap();
        let got = transcript_epoch(&p).expect("a transcript with timestamps must date");
        assert_eq!(got, 1_789_868_992, "the NEWEST timestamp in the file, 01:49:52Z");
        let now = worktrees_core::sysclock::now_epoch();
        assert!(
            now - got > 3600,
            "transcript_epoch returned {got}, within an hour of now ({now}) — that is the \
             file's mtime, which is exactly the bug: a session idle since this morning would \
             be stamped as having just finished on every restart"
        );
        let _ = std::fs::remove_file(&p);
    }

    /// Everything unreadable degrades to "no answer", so the backfill falls back
    /// to the prompt time instead of inventing one.
    #[test]
    fn transcript_epoch_has_no_answer_for_a_file_it_cannot_date() {
        let miss = std::env::temp_dir().join(format!("wt-tsx-missing-{}.jsonl", std::process::id()));
        assert_eq!(transcript_epoch(&miss), None);
        let p = std::env::temp_dir().join(format!("wt-tsx-blank-{}.jsonl", std::process::id()));
        std::fs::write(&p, "{\"type\":\"user\"}\nnot json\n").unwrap();
        assert_eq!(transcript_epoch(&p), None);
        let _ = std::fs::remove_file(&p);
    }

    /// A tail read starts mid-line by construction; that fragment must be
    /// dropped rather than fed to the JSON parser as a whole record.
    #[test]
    fn tail_lines_drops_the_partial_first_line() {
        let p = std::env::temp_dir().join(format!("wt-tail-{}.jsonl", std::process::id()));
        std::fs::write(&p, "aaaa\nbbbb\ncccc\n").unwrap();
        assert_eq!(tail_lines(&p, 1024), vec!["aaaa", "bbbb", "cccc"]);
        // 9 bytes back = "b\ncccc\n" plus a fragment of the first line
        assert_eq!(tail_lines(&p, 9), vec!["cccc"]);
        let _ = std::fs::remove_file(&p);
    }

    /// The tail offset lands wherever it lands, and prompts contain emoji. A
    /// strict decode would throw away every line over a split character — and
    /// stay broken, since the boundary only moves as the file grows.
    #[test]
    fn tail_lines_survives_a_split_multibyte_char() {
        let p = std::env::temp_dir().join(format!("wt-tail-utf8-{}.jsonl", std::process::id()));
        std::fs::write(&p, "aaaa\n🎉bbb\ncccc\n").unwrap(); // 5 + 8 + 5 = 18 bytes
        // 11 bytes back = offset 7, two bytes into the 4-byte emoji at 5..9
        assert_eq!(tail_lines(&p, 11), vec!["cccc"]);
        let _ = std::fs::remove_file(&p);
    }
}
