//! The fold: closing a chat writes what is worth keeping of it into the
//! workspace's memory (`docs/proposals/workspace-chats.md` §3.1, §3.2, §9).
//!
//! **What one is.** `workspace_chat_fold` commits what is dirty (so the fold's
//! commit holds only the fold), holds the memory, writes the chat's
//! `fold.md`, records the fold in `chats.json`, ends the session and launches
//! the SAME conversation again on a fixed opener that reads that file. Nothing
//! is typed. The poll does the rest ([`tick`]).
//!
//! **When it lands.** When the chat's note is NEWER than the fold's start and
//! its Claude has been idle for one tick (seen idle twice running). "The note
//! exists" is not the test: on a second fold of one chat it exists before
//! Claude has read a word, and a session that has just launched reads idle.
//! Claude saying there is nothing to keep, in `fold.md`'s words and in this
//! fold's own turn, with no new note, closes it with no change.
//!
//! **When it fails.** Five minutes with no new note, on a clock that does not
//! run while Claude waits on a prompt, or its Claude exited without one. The
//! session stays open, the hold is let go, and what memory holds is COUNTED
//! from its dirty tree: the words never say "unchanged" without that look.
//!
//! **One at a time in a workspace**, so a fold's commit is one chat's work.
//!
//! **What is running is in memory** ([`Folds`]); `chats.json` says which chats
//! were folding, and the engine takes them up again when it starts
//! ([`ensure`]), hold and all. Until the first tick has looked they read
//! `checking`. The record is in the chat's own working directory, so it only
//! ever says WHICH chats to look at: nothing in it names a path or reaches git.
//!
//! Every row here runs under the workspace's `life_lock`, and the poll only
//! TRIES it: a start that is launching is not waited for, the next tick is.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use worktrees_core::memory::{self as memory, Change};
use worktrees_core::workspacechats::{self as chats, Chat, Fold, FoldLines};
use worktrees_core::workspaces::{self as store, Workspace};
use worktrees_core::{activity, desksessions, tmux, turns, wsfolder};

use crate::agents::{self, Locus};
use crate::watch::{FoldOf, WorkspaceSessions};
use crate::workspacechats::{life_lock, locus_of, touched, workspace};
use crate::Ctx;

/// How long a fold may go with no new note, the time it waits on a prompt
/// aside.
pub const LIMIT: Duration = Duration::from_secs(5 * 60);
/// Ticks running a Claude must be seen idle, or gone, to count as that.
const TICKS: u32 = 2;

pub const ERR_WORKING: &str = "it is working: stop it or wait";
pub const ERR_WAITING: &str = "it is waiting on you: answer it first";
pub const ERR_STARTING: &str = "it has not started yet: wait for it, or close it without folding";
pub const ERR_ALREADY: &str = "this chat is folding already";

/// What a fold does outside the files: a test scripts it.
pub trait FoldIo: Send + Sync {
    /// Milliseconds since the epoch.
    fn now_ms(&self) -> u64;
    /// The poll's clock, for what it counts.
    fn clock(&self) -> Duration;
    fn live(&self, session: &str) -> bool;
    /// Whether the chat could be relaunched (the limits), before it is ended.
    fn admit(&self, root: &Path, w: &Workspace, conv: &str) -> Result<(), String>;
    /// End the chat's session; whether it ran.
    fn close(&self, conv: &str) -> Result<bool, String>;
    /// Launch the conversation again, resumed on `opener`.
    fn relaunch(&self, root: &Path, w: &Workspace, conv: &str, opener: String) -> Result<String, String>;
}

struct Live;

impl FoldIo for Live {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
    }
    fn clock(&self) -> Duration {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }
    fn live(&self, session: &str) -> bool {
        tmux::session_exists(session)
    }
    fn admit(&self, root: &Path, w: &Workspace, conv: &str) -> Result<(), String> {
        chats::may_relaunch_at(root, w, conv)
    }
    fn close(&self, conv: &str) -> Result<bool, String> {
        chats::close(conv)
    }
    fn relaunch(&self, root: &Path, w: &Workspace, conv: &str, opener: String) -> Result<String, String> {
        chats::launch_at(root, w, conv, chats::Start::Resume, Some(opener))
    }
}

/// How a fold stands, as a client reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Said {
    /// Taken up after an engine restart, and not looked at yet.
    Checking,
    Folding,
    /// On a prompt: the clock stands still.
    Waiting,
    Failed { why: String, left: Option<u32> },
}

/// One chat's fold, running or failed.
#[derive(Clone, Debug)]
struct Run {
    workspace: String,
    /// When it was asked, as the note's mtime is read.
    asked_ms: u64,
    said: Said,
    /// Its clock: what has run of [`LIMIT`], and the last tick's reading.
    spent: Duration,
    last: Option<Duration>,
    idle: u32,
    gone: u32,
}

impl Run {
    fn new(workspace: &str, asked_ms: u64, said: Said) -> Run {
        Run { workspace: workspace.into(), asked_ms, said, spent: Duration::ZERO, last: None, idle: 0, gone: 0 }
    }
    fn failed(&self) -> bool {
        matches!(self.said, Said::Failed { .. })
    }
}

/// What one tick sees of a fold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seen {
    live: bool,
    busy: bool,
    waiting: bool,
    /// Its session is up and its Claude has written nothing to say so yet.
    starting: bool,
    note_ms: Option<u64>,
}

/// What a tick decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Wait,
    Land,
    /// Idle, with no new note: see whether it said there was nothing.
    Ask,
    TimedOut,
    Gone,
}

/// One tick of one fold. `now` is [`FoldIo::clock`].
fn step(r: &mut Run, seen: Seen, now: Duration) -> Next {
    let since = r.last.map_or(Duration::ZERO, |l| now.saturating_sub(l));
    r.last = Some(now);
    let was_waiting = r.said == Said::Waiting;
    let idle = seen.live && !seen.busy && !seen.waiting && !seen.starting;
    r.idle = if idle { r.idle + 1 } else { 0 };
    r.gone = if seen.live { 0 } else { r.gone + 1 };
    if seen.live && seen.waiting {
        r.said = Said::Waiting;
        return Next::Wait;
    }
    if !was_waiting {
        r.spent += since;
    }
    if seen.live {
        r.said = Said::Folding;
    }
    let new = seen.note_ms.is_some_and(|m| m > r.asked_ms);
    if new {
        return if r.idle >= TICKS || r.gone >= TICKS { Next::Land } else { Next::Wait };
    }
    if r.gone >= TICKS {
        return Next::Gone;
    }
    if r.spent >= LIMIT {
        return Next::TimedOut;
    }
    if r.idle >= TICKS {
        return Next::Ask;
    }
    Next::Wait
}

/// The folds of this engine, by chat id: running, and failed ones a client is
/// still to be told of. One per engine.
pub struct Folds {
    runs: Mutex<BTreeMap<String, Run>>,
    taken_up: OnceLock<()>,
    io: Arc<dyn FoldIo>,
}

impl Default for Folds {
    fn default() -> Folds {
        Folds::new(Arc::new(Live))
    }
}

impl Folds {
    pub fn new(io: Arc<dyn FoldIo>) -> Folds {
        Folds { runs: Mutex::new(BTreeMap::new()), taken_up: OnceLock::new(), io }
    }
    fn runs(&self) -> MutexGuard<'_, BTreeMap<String, Run>> {
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The chat of workspace `ws` that is folding now.
    fn running_in(&self, ws: &str) -> Option<String> {
        self.runs().iter().find(|(_, r)| r.workspace == ws && !r.failed()).map(|(c, _)| c.clone())
    }
    /// Whether chat `conv` is folding now.
    pub fn is_folding(&self, conv: &str) -> bool {
        self.runs().get(conv).is_some_and(|r| !r.failed())
    }
    /// How chat `conv`'s fold stands, when there is something to say.
    pub fn of(&self, conv: &str) -> Option<FoldOf> {
        self.runs().get(conv).map(|r| fold_of(conv, r))
    }
    /// End chat `conv`'s session; whether it ran.
    pub fn close(&self, conv: &str) -> Result<bool, String> {
        self.io.close(conv)
    }
    /// Workspace `ws` is gone: nothing more is said of its chats.
    pub fn forget(&self, ws: &str) {
        self.runs().retain(|_, r| r.workspace != ws);
    }
    /// The event's map, by session.
    fn by_session(&self) -> BTreeMap<String, FoldOf> {
        self.runs().iter().map(|(c, r)| (chats::session_name(c), fold_of(c, r))).collect()
    }
}

fn fold_of(conv: &str, r: &Run) -> FoldOf {
    let (state, why, left) = match &r.said {
        Said::Checking => ("checking", None, None),
        Said::Folding => ("folding", None, None),
        Said::Waiting => ("waiting", None, None),
        Said::Failed { why, left } => ("failed", Some(why.clone()), *left),
    };
    FoldOf { workspace: r.workspace.clone(), chat: conv.to_string(), state: state.into(), why, left }
}

/// What the poll decided of a fold, for the audit log.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FoldNote {
    pub workspace: String,
    pub chat: String,
    /// `landed`, `nothing` (it changed no memory) or `failed`.
    pub event: &'static str,
    /// The commit it landed as.
    pub commit: Option<String>,
}

/// Take up the folds a restart left, once per engine: the hold again, and
/// `checking` until a tick has looked. A failed one is shown again. Called by
/// every row that reads or starts something a fold bears on, and by the poll.
pub fn ensure(ctx: &Ctx) {
    ctx.folds.taken_up.get_or_init(|| {
        let Ok(f) = store::read_at(ctx.workspaces_file()) else { return };
        for w in f.workspaces {
            for c in chats::list_at(ctx.workspaces_dir(), &w.id).unwrap_or_default() {
                let Some(fold) = &c.fold else { continue };
                let said = match fold.state.as_str() {
                    chats::FOLDING if ctx.folds.running_in(&w.id).is_none() => Said::Checking,
                    chats::FOLD_FAILED => Said::Failed { why: fold.why.clone().unwrap_or_default(), left: fold.left },
                    _ => continue,
                };
                if said == Said::Checking {
                    ctx.memory.hold(&w.id);
                }
                ctx.folds.runs().insert(c.id.clone(), Run::new(&w.id, asked_ms(fold), said));
            }
        }
    });
}

/// When a fold was asked. A record with no usable time (an agent's edit)
/// reads as just now: its note must still be written to land it.
fn asked_ms(f: &Fold) -> u64 {
    f.asked_ms.unwrap_or_else(|| Live.now_ms())
}

fn title_of(ctx: &Ctx, ws: &str, conv: &str) -> String {
    chats::find_at(ctx.workspaces_dir(), ws, conv).map(|c| c.title).unwrap_or_else(|_| "Another chat".into())
}

// ── the rows ────────────────────────────────────────────────────────────────

/// Why chat `c` cannot be folded now, by what its Claude is doing.
fn quiet(ctx: &Ctx, at: &Locus) -> Result<(), String> {
    match ctx.agents.io().activity(at).state {
        activity::State::Busy => return Err(ERR_WORKING.into()),
        activity::State::Waiting => return Err(ERR_WAITING.into()),
        _ => {}
    }
    // The pane as well: the folder-trust question comes before Claude says
    // anything of itself.
    if agents::prompt_at(ctx, at)?["waiting"] == json!(true) {
        return Err(ERR_WAITING.into());
    }
    if ctx.agents.io().activity(at).state == activity::State::None {
        return Err(ERR_STARTING.into());
    }
    Ok(())
}

/// `workspace_chat_fold`: fold chat `conv` into the workspace's memory, and
/// close it when that lands. An ended chat is folded too (Fold now, and a
/// failed fold's Try again): its note is rewritten.
///
/// `reattach`: a session was ended and launched again under the same name, so
/// a terminal attached to it has ended, and the client opens it again with
/// `workspace_term_open`.
pub fn workspace_chat_fold(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    let lock = life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    ensure(ctx);
    let w = workspace(ctx, &id)?;
    let root = ctx.workspaces_dir();
    let c = chats::find_at(root, &id, &conv)?;
    match ctx.folds.running_in(&id) {
        Some(x) if x == conv => return Err(ERR_ALREADY.into()),
        Some(x) => return Err(format!("“{}” is folding. Close this one after.", title_of(ctx, &id, &x))),
        None => {}
    }
    let io = ctx.folds.io.clone();
    let at = locus_of(ctx, &id, &c)?;
    // No line or answer reaches the chat between the look and its end.
    let chat_lock = ctx.agents.place_lock(&at.lock_key());
    let _chat = chat_lock.lock().unwrap_or_else(|e| e.into_inner());
    let live = io.live(&chats::session_name(&conv));
    if live {
        quiet(ctx, &at)?;
    }
    io.admit(root, &w, &conv)?;
    crate::memory::before_launch(ctx, &w)?;
    crate::memory::memory_commit_if_dirty(ctx, &id, None).map_err(|e| format!("the memory could not be committed, so nothing was folded: {e}"))?;
    ctx.memory.hold(&id);
    let began = begin(ctx, &w, &c, live);
    match began {
        Ok((session, fold)) => {
            touched(ctx, &id, &conv);
            Ok(json!({ "id": id, "conv": conv, "session": session, "fold": fold, "reattach": live }))
        }
        Err(e) => {
            ctx.memory.release(&id);
            Err(e)
        }
    }
}

/// The fold's own part, with the memory held: the file, the record, and the
/// relaunch. A record is undone while the session still runs; once it has been
/// ended, a relaunch that fails is a failed fold.
fn begin(ctx: &Ctx, w: &Workspace, c: &Chat, live: bool) -> Result<(String, Fold), String> {
    let (root, io) = (ctx.workspaces_dir(), ctx.folds.io.clone());
    let opener = chats::write_fold_at(root, &w.id, c)?;
    let asked = io.now_ms();
    let fold = Fold { asked: desksessions::now_iso(), asked_ms: Some(asked), state: chats::FOLDING.into(), ..Fold::default() };
    let before = c.fold.clone();
    chats::update_at(root, &w.id, &c.id, |x| x.fold = Some(fold.clone()))?;
    if live {
        if let Err(e) = io.close(&c.id) {
            let _ = chats::update_at(root, &w.id, &c.id, |x| x.fold = before);
            return Err(e);
        }
    }
    ctx.folds.runs().insert(c.id.clone(), Run::new(&w.id, asked, Said::Folding));
    match io.relaunch(root, w, &c.id, opener) {
        Ok(session) => Ok((session, fold)),
        Err(e) => {
            let why = format!("it could not be launched again: {e}");
            fail(ctx, &w.id, &c.id, why.clone());
            Err(why)
        }
    }
}

/// `1 file changed`, as the failure says it: from the dirty tree, or that it
/// could not be read.
fn holds(ctx: &Ctx, ws: &str) -> (Option<u32>, String) {
    match memory::status_at(ctx.workspaces_dir(), ws) {
        Ok(s) if s.dirty.is_empty() => (Some(0), "Memory is unchanged.".into()),
        Ok(s) => {
            let n = s.dirty.len();
            (Some(n as u32), format!("Memory may hold part of it: {n} file{} changed.", if n == 1 { "" } else { "s" }))
        }
        Err(e) => (None, format!("Whether memory holds part of it could not be read: {e}")),
    }
}

/// Mark chat `conv`'s fold failed: its session is left as it is, and the hold
/// let go.
fn fail(ctx: &Ctx, ws: &str, conv: &str, why: String) {
    let (left, holds) = holds(ctx, ws);
    let why = format!("{}. {holds}", why.trim_end_matches('.'));
    if let Err(e) = chats::update_at(ctx.workspaces_dir(), ws, conv, |c| {
        let f = c.fold.get_or_insert_with(Fold::default);
        f.state = chats::FOLD_FAILED.into();
        f.why = Some(why.clone());
        f.left = left;
    }) {
        crate::log::warn(&format!("workspace {ws}: could not record chat {conv}'s failed fold: {e}"));
    }
    if let Some(r) = ctx.folds.runs().get_mut(conv) {
        r.said = Said::Failed { why, left };
    }
    ctx.memory.release(ws);
}

/// A title as a commit's subject may carry it after `head`.
fn subject(head: &str, title: &str) -> String {
    format!("{head}: {title}")
}

/// What is left of a chat that is closed without its fold: its session is
/// ended by the caller FIRST. A running fold is stopped (its hold let go) and
/// what it wrote committed under `Fold stopped`; a chat that only left changes
/// has them committed under `Chat closed`, unless another chat's fold holds
/// the memory: that dirty tree is the fold's. The caller holds `life_lock`.
/// Answers whether a fold was stopped, and the commit if there was one.
pub fn closed(ctx: &Ctx, ws: &str, c: &Chat, ran: bool) -> Result<(bool, Option<Change>), String> {
    let run = ctx.folds.runs().remove(&c.id);
    let stopped = run.as_ref().is_some_and(|r| !r.failed());
    if run.is_some() || c.fold.as_ref().is_some_and(|f| [chats::FOLDING, chats::FOLD_FAILED].contains(&f.state.as_str())) {
        let r = chats::update_at(ctx.workspaces_dir(), ws, &c.id, |x| {
            x.fold = Some(Fold { asked: x.fold.take().map(|f| f.asked).unwrap_or_default(), state: chats::FOLD_NONE.into(), ..Fold::default() });
        });
        if let Err(e) = r {
            crate::log::warn(&format!("workspace {ws}: could not record chat {}'s fold as stopped: {e}", c.id));
        }
    }
    if stopped {
        let done = crate::memory::memory_commit_if_dirty(ctx, ws, Some(&subject("Fold stopped", &c.title)));
        ctx.memory.release(ws);
        return Ok((true, done?));
    }
    if !ran || ctx.memory.is_held(ws) {
        return Ok((false, None));
    }
    Ok((false, crate::memory::memory_commit_if_dirty(ctx, ws, Some(&subject("Chat closed", &c.title)))?))
}

/// After an undo or a redo: which chats' folds are undone now, in their
/// records. One git call, whatever the workspace holds.
pub fn mark_reverted(ctx: &Ctx, ws: &str) {
    let root = ctx.workspaces_dir();
    let marked = memory::undone_at(root, ws).and_then(|undone| {
        for c in chats::list_at(root, ws)? {
            let Some(f) = &c.fold else { continue };
            let Some(commit) = &f.commit else { continue };
            let now = undone.contains(commit);
            if f.reverted != now {
                chats::update_at(root, ws, &c.id, |x| {
                    if let Some(f) = x.fold.as_mut() {
                        f.reverted = now;
                    }
                })?;
            }
        }
        Ok(())
    });
    if let Err(e) = marked {
        crate::log::warn(&format!("workspace {ws}: could not mark its chats' undone folds: {e}"));
    }
}

// ── the poll ────────────────────────────────────────────────────────────────

/// Whether the fold's own turn says there was nothing to keep.
fn nothing_said(ctx: &Ctx, ws: &str, c: &Chat, at: &Locus, asked_ms: u64) -> bool {
    let Ok(folder) = wsfolder::folder_at(ctx.workspaces_dir(), ws) else { return false };
    let Some(lane) = agents::ended_lane(at) else { return false };
    let Ok(lines) = ctx.agents.io().transcript(at, &lane) else { return false };
    turns::claude(&lines, 1).last().is_some_and(|t| chats::said_nothing(t, &chats::fold_opener(&folder, &c.id), asked_ms))
}

/// The poll's part for the folds: each running one is looked at, and the ones
/// that are over are closed. `now` gets the fold map. Answers what was
/// decided, for the audit log.
pub fn tick(ctx: &Ctx, now: &mut WorkspaceSessions) -> Vec<FoldNote> {
    ensure(ctx);
    let mut notes = Vec::new();
    let running: Vec<(String, String)> = ctx.folds.runs().iter().filter(|(_, r)| !r.failed()).map(|(c, r)| (c.clone(), r.workspace.clone())).collect();
    for (conv, ws) in running {
        let lock = life_lock(ctx, &ws);
        // A start, a close or a fold of this workspace is under way: next tick.
        let _life = match lock.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => continue,
        };
        notes.extend(look(ctx, now, &ws, &conv));
    }
    now.fold = ctx.folds.by_session();
    notes
}

/// One fold's tick, under the workspace's `life_lock`.
fn look(ctx: &Ctx, now: &WorkspaceSessions, ws: &str, conv: &str) -> Option<FoldNote> {
    let root = ctx.workspaces_dir();
    let note = |event, commit| Some(FoldNote { workspace: ws.into(), chat: conv.into(), event, commit });
    // Ended while the lock was waited for, or recorded as over by another
    // engine: nothing of this one's to decide.
    let c = chats::find_at(root, ws, conv).ok().filter(|c| c.fold.as_ref().is_some_and(|f| f.state == chats::FOLDING));
    let Some(c) = c.filter(|_| ctx.folds.is_folding(conv)) else {
        if ctx.folds.runs().remove(conv).is_some() {
            ctx.memory.release(ws);
        }
        return None;
    };
    let session = chats::session_name(conv);
    let at = locus_of(ctx, ws, &c).ok()?;
    let has = |v: &[String]| v.iter().any(|s| *s == session);
    let live = has(&now.live);
    let state = if live { ctx.agents.io().activity(&at).state } else { activity::State::None };
    let seen = Seen {
        live,
        busy: has(&now.busy) || state == activity::State::Busy,
        waiting: has(&now.waiting) || state == activity::State::Waiting,
        starting: state == activity::State::None,
        note_ms: chats::note_mtime_ms(root, ws, &c),
    };
    let (next, asked) = {
        let mut runs = ctx.folds.runs();
        let r = runs.get_mut(conv)?;
        (step(r, seen, ctx.folds.io.clock()), r.asked_ms)
    };
    match next {
        Next::Wait => None,
        Next::Land => match crate::memory::memory_commit_if_dirty(ctx, ws, Some(&subject("fold", &c.title))) {
            Ok(change) => {
                let commit = change.as_ref().map(|c| c.commit.clone());
                land(ctx, ws, &c, change);
                note(if commit.is_some() { "landed" } else { "nothing" }, commit)
            }
            Err(e) => {
                fail(ctx, ws, conv, format!("its note was written, and the memory could not be committed: {e}"));
                note("failed", None)
            }
        },
        Next::Ask if nothing_said(ctx, ws, &c, &at, asked) => {
            land(ctx, ws, &c, None);
            note("nothing", None)
        }
        Next::Ask => None,
        Next::TimedOut => {
            fail(ctx, ws, conv, "it wrote no note in five minutes, and the chat is still open".into());
            note("failed", None)
        }
        Next::Gone => {
            fail(ctx, ws, conv, "its Claude exited before it wrote a note".into());
            note("failed", None)
        }
    }
}

/// A fold is over: its record, its session, its hold.
fn land(ctx: &Ctx, ws: &str, c: &Chat, change: Option<Change>) {
    let root = ctx.workspaces_dir();
    // With no commit, what memory is left with is said only if it was looked at.
    let left = if change.is_none() { holds(ctx, ws).0 } else { None };
    if let Err(e) = chats::update_at(root, ws, &c.id, |x| {
        let f = x.fold.get_or_insert_with(Fold::default);
        f.state = chats::FOLDED.into();
        f.commit = change.as_ref().map(|c| c.commit.clone());
        f.files = Some(change.as_ref().map_or(0, |c| c.files.len() as u32));
        f.lines = Some(change.as_ref().map_or(FoldLines::default(), |c| FoldLines { added: c.added as u32, removed: c.removed as u32 }));
        f.left = left;
        f.why = None;
    }) {
        crate::log::warn(&format!("workspace {ws}: could not record chat {}'s fold: {e}", c.id));
    }
    if let Err(e) = ctx.folds.io.close(&c.id) {
        crate::log::warn(&format!("workspace {ws}: chat {} folded and its session could not be ended: {e}", c.id));
    }
    ctx.folds.runs().remove(&c.id);
    ctx.memory.release(ws);
    touched(ctx, ws, &c.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use worktrees_core::{fsx, ulid};

    /// The outside, scripted: the two clocks, which sessions are up, and what
    /// was ended and launched, in order.
    #[derive(Default)]
    struct Script {
        ms: AtomicU64,
        secs: AtomicU64,
        up: Mutex<HashSet<String>>,
        log: Mutex<Vec<String>>,
        refuse: Mutex<(Option<String>, Option<String>)>,
    }

    impl FoldIo for Script {
        fn now_ms(&self) -> u64 {
            self.ms.load(Ordering::SeqCst)
        }
        fn clock(&self) -> Duration {
            Duration::from_secs(self.secs.load(Ordering::SeqCst))
        }
        fn live(&self, session: &str) -> bool {
            self.up.lock().unwrap().contains(session)
        }
        fn admit(&self, _: &Path, _: &Workspace, _: &str) -> Result<(), String> {
            self.refuse.lock().unwrap().0.clone().map_or(Ok(()), Err)
        }
        fn close(&self, conv: &str) -> Result<bool, String> {
            let was = self.up.lock().unwrap().remove(&chats::session_name(conv));
            if was {
                self.log.lock().unwrap().push("close".into());
            }
            Ok(was)
        }
        fn relaunch(&self, _: &Path, _: &Workspace, conv: &str, opener: String) -> Result<String, String> {
            if let Some(e) = self.refuse.lock().unwrap().1.clone() {
                return Err(e);
            }
            self.log.lock().unwrap().push(opener);
            let s = chats::session_name(conv);
            self.up.lock().unwrap().insert(s.clone());
            Ok(s)
        }
    }

    /// The agent: its state, whether a prompt is up, and its transcript.
    struct Agent {
        state: Mutex<activity::State>,
        screen: Mutex<String>,
        lines: Mutex<Vec<String>>,
    }

    impl agents::AgentIo for Agent {
        fn lanes(&self, at: &Locus) -> Vec<agents::Lane> {
            agents::ended_lane(at).into_iter().map(|l| agents::Lane { pane: "%1".into(), ..l }).collect()
        }
        fn activity(&self, _: &Locus) -> activity::Activity {
            activity::Activity { state: *self.state.lock().unwrap(), ..activity::Activity::none() }
        }
        fn capture(&self, _: &agents::Lane) -> Option<String> {
            Some(self.screen.lock().unwrap().clone())
        }
        fn press(&self, _: &agents::Lane, _: tmux::Key) -> Result<(), String> {
            panic!("a fold presses no key")
        }
        fn paste(&self, _: &agents::Lane, _: &str) -> Result<(), String> {
            panic!("a fold types nothing")
        }
        fn deliver(&self, _: &Locus, _: &agents::Lane, _: &str) -> Result<worktrees_core::harness::SendOutcome, String> {
            panic!("a fold types nothing")
        }
        fn transcript(&self, _: &Locus, _: &agents::Lane) -> Result<Vec<String>, String> {
            Ok(self.lines.lock().unwrap().clone())
        }
        fn asked(&self, _: &agents::Lane) -> Vec<worktrees_core::pending::Asked> {
            Vec::new()
        }
        fn forget(&self, _: &agents::Lane, _: worktrees_core::pending::Forget) {}
        fn plan_file(&self, _: &str) -> Option<(String, String)> {
            None
        }
        fn codex_models(&self) -> Vec<(String, usize)> {
            Vec::new()
        }
        fn commands_of(&self, _: &Locus, _: &agents::Lane) -> (Option<String>, Vec<worktrees_core::slash::Command>) {
            (None, Vec::new())
        }
        fn command(&self, _: &agents::Lane, _: &str, _: &str) -> Result<worktrees_core::harness::SendOutcome, String> {
            panic!("no commands")
        }
        fn terminal_activity(&self, _: &agents::Lane) -> Result<u64, String> {
            Err("none".into())
        }
        fn sleep(&self, _: u64) {}
    }

    const TRUST: &str = include_str!("../../worktrees-core/tests/fixtures/claude-screen/2.1.314/trust.e.txt");

    struct Desk {
        base: PathBuf,
        ctx: Ctx,
        io: Arc<Script>,
        agent: Arc<Agent>,
        ws: String,
    }

    impl Desk {
        fn new(tag: &str) -> Desk {
            let base = std::env::temp_dir().join(format!("wt-fold-{tag}-{}-{:x}", std::process::id(), fsx::random_u64()));
            fs::create_dir_all(&base).unwrap();
            let base = fs::canonicalize(base).unwrap();
            let io = Arc::new(Script::default());
            io.ms.store(1_800_000_000_000, Ordering::SeqCst);
            let agent = Arc::new(Agent { state: Mutex::new(activity::State::Idle), screen: Mutex::default(), lines: Mutex::default() });
            let file = base.join("cfg/workspaces.json");
            let ws = store::create_at(&file, "Home").unwrap().id;
            let ctx = Desk::engine(&base, &io, &agent);
            memory::init_at(&base.join("ws"), &ws, "Home").unwrap();
            Desk { base, ctx, io, agent, ws }
        }
        /// An engine over the same files: a restart is a second one.
        fn engine(base: &Path, io: &Arc<Script>, agent: &Arc<Agent>) -> Ctx {
            Ctx::new(base.join("cfg"), Arc::new(crate::tests::Recorder::default()))
                .with_agents(agents::Agents::new(agent.clone()))
                .with_workspaces_dir(base.join("ws"))
                .with_workspaces_file(base.join("cfg/workspaces.json"))
                .with_folds(Folds::new(io.clone()))
        }
        fn chat(&self, title: &str, live: bool) -> String {
            let c = chats::prepare_at(self.ctx.workspaces_dir(), &self.ws, |_| String::new()).unwrap();
            chats::rename_at(self.ctx.workspaces_dir(), &self.ws, &c.id, title).unwrap();
            if live {
                self.io.up.lock().unwrap().insert(chats::session_name(&c.id));
            }
            c.id
        }
        fn rec(&self, conv: &str) -> Chat {
            chats::find_at(self.ctx.workspaces_dir(), &self.ws, conv).unwrap()
        }
        fn fold(&self, conv: &str) -> Result<Value, String> {
            workspace_chat_fold(&self.ctx, self.ws.clone(), conv.into())
        }
        fn mem(&self, path: &str) -> PathBuf {
            self.base.join("ws").join(&self.ws).join("memory").join(path)
        }
        fn put(&self, path: &str, text: &str) {
            fs::create_dir_all(self.mem(path).parent().unwrap()).unwrap();
            fs::write(self.mem(path), text).unwrap();
        }
        /// The chat's note, written `after` ms past the script's clock.
        fn note(&self, conv: &str, text: &str, after: i64) {
            let p = self.base.join("ws").join(&self.ws).join(self.rec(conv).note);
            fs::write(&p, text).unwrap();
            let at = std::time::UNIX_EPOCH + Duration::from_millis((self.io.now_ms() as i64 + after) as u64);
            fs::File::options().write(true).open(&p).unwrap().set_modified(at).unwrap();
        }
        fn sees(&self, s: activity::State) {
            *self.agent.state.lock().unwrap() = s;
        }
        /// One poll tick, `secs` after the last.
        fn tick_on(&self, ctx: &Ctx, secs: u64) -> (Vec<FoldNote>, WorkspaceSessions) {
            self.io.secs.fetch_add(secs, Ordering::SeqCst);
            let mut live: Vec<String> = self.io.up.lock().unwrap().iter().cloned().collect();
            live.sort();
            let mut now = WorkspaceSessions { live, ..Default::default() };
            let notes = tick(ctx, &mut now);
            (notes, now)
        }
        fn tick(&self, secs: u64) -> (Vec<FoldNote>, WorkspaceSessions) {
            self.tick_on(&self.ctx, secs)
        }
        fn says(&self, conv: &str) -> Option<String> {
            self.ctx.folds.of(conv).map(|f| f.state)
        }
        fn log(&self) -> Vec<Change> {
            memory::log_at(self.ctx.workspaces_dir(), &self.ws, None, None, None).unwrap().changes
        }
        fn held(&self) -> bool {
            self.ctx.memory.is_held(&self.ws)
        }
        fn acts(&self) -> Vec<String> {
            self.io.log.lock().unwrap().clone()
        }
    }

    impl Drop for Desk {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    const ON: Seen = Seen { live: true, busy: false, waiting: false, starting: false, note_ms: None };

    /// The hole this closes: on a second fold the note is there before Claude
    /// has read a word, and a session that has just launched reads idle.
    #[test]
    fn an_old_note_never_lands_and_a_new_one_lands_after_one_idle_tick() {
        let mut r = Run::new("w", 1000, Said::Folding);
        let old = Seen { note_ms: Some(1000), ..ON };
        for t in 0..20 {
            assert_ne!(step(&mut r, old, secs(t)), Next::Land, "{t}");
        }
        let new = Seen { note_ms: Some(1001), ..ON };
        let mut r = Run::new("w", 1000, Said::Folding);
        assert_eq!(step(&mut r, Seen { busy: true, ..new }, secs(0)), Next::Wait, "still writing");
        assert_eq!(step(&mut r, new, secs(3)), Next::Wait, "seen idle once");
        assert_eq!(step(&mut r, Seen { busy: true, ..new }, secs(6)), Next::Wait, "at work again");
        assert_eq!(step(&mut r, new, secs(9)), Next::Wait);
        assert_eq!(step(&mut r, new, secs(12)), Next::Land);
        // A Claude that has not started reads as nothing at all.
        let mut r = Run::new("w", 1000, Said::Folding);
        for t in 0..5 {
            assert_eq!(step(&mut r, Seen { starting: true, ..new }, secs(t)), Next::Wait);
        }
        // With the note written and its Claude gone, what it wrote lands.
        let mut r = Run::new("w", 1000, Said::Folding);
        let gone = Seen { live: false, starting: true, ..new };
        assert_eq!((step(&mut r, gone, secs(0)), step(&mut r, gone, secs(3))), (Next::Wait, Next::Land));
    }

    #[test]
    fn the_five_minutes_stand_still_while_it_waits_on_a_prompt() {
        let mut r = Run::new("w", 1000, Said::Folding);
        let (work, wait) = (Seen { busy: true, ..ON }, Seen { waiting: true, ..ON });
        assert_eq!(step(&mut r, work, secs(0)), Next::Wait);
        assert_eq!(step(&mut r, work, secs(240)), Next::Wait, "four minutes of work");
        // An hour on a prompt: from the tick that saw it to the one that did not.
        assert_eq!((step(&mut r, wait, secs(243)), r.said.clone()), (Next::Wait, Said::Waiting));
        assert_eq!(step(&mut r, wait, secs(3843)), Next::Wait);
        assert_eq!((step(&mut r, work, secs(3846)), r.said.clone()), (Next::Wait, Said::Folding));
        assert_eq!(r.spent, secs(240));
        assert_eq!(step(&mut r, work, secs(3905)), Next::Wait, "4:59");
        assert_eq!(step(&mut r, work, secs(3906)), Next::TimedOut, "5:00");
        // A note that is there is never timed out: it is being finished.
        let mut r = Run::new("w", 1000, Said::Folding);
        assert_eq!(step(&mut r, work, secs(0)), Next::Wait);
        assert_eq!(step(&mut r, Seen { note_ms: Some(2000), ..work }, secs(900)), Next::Wait);
        // Gone with no note, seen twice; one missed reading is not that.
        let mut r = Run::new("w", 1000, Said::Folding);
        let gone = Seen { live: false, starting: true, ..Seen::default() };
        assert_eq!(step(&mut r, gone, secs(0)), Next::Wait);
        assert_eq!(step(&mut r, work, secs(3)), Next::Wait);
        assert_eq!((step(&mut r, gone, secs(6)), step(&mut r, gone, secs(9))), (Next::Wait, Next::Gone));
    }

    /// The whole of one: what was dirty is committed before and apart, the
    /// file and the record are written, the session is ended and launched on
    /// the opener, and it lands with the counts of its own commit.
    #[test]
    fn a_fold_lands_as_its_own_commit_and_closes_the_chat() {
        let d = Desk::new("land");
        let conv = d.chat("Storage layout", true);
        d.put("topics/by-hand.md", "by hand\n");
        let r = d.fold(&conv).unwrap();
        assert_eq!((&r["reattach"], &r["fold"]["state"], &r["session"]), (&json!(true), &json!("folding"), &json!(chats::session_name(&conv))), "{r}");
        assert_eq!(d.log()[0].subject, "Edited: topics/by-hand.md");
        assert!(d.held());
        let fold_md = d.base.join("ws").join(&d.ws).join("chats").join(&conv).join("fold.md");
        assert_eq!(d.acts(), vec!["close".to_string(), format!("Read {} and do what it says.", fold_md.display())]);
        assert!(fs::read_to_string(&fold_md).unwrap().contains(&d.base.join("ws").join(&d.ws).join(d.rec(&conv).note).display().to_string()));
        assert_eq!(d.rec(&conv).fold.unwrap().asked_ms, Some(d.io.now_ms()));
        d.sees(activity::State::Busy);
        let (n, now) = d.tick(3);
        assert!(n.is_empty());
        assert_eq!((now.fold[&chats::session_name(&conv)].state.as_str(), now.fold.len()), ("folding", 1));
        d.note(&conv, "# Storage\n\n## Asked\n", 5);
        d.put("MEMORY.md", "# Memory: Home\n- The NAS.\n");
        assert!(d.tick(3).0.is_empty(), "still at work");
        d.sees(activity::State::Idle);
        assert!(d.tick(3).0.is_empty(), "idle once");
        let (n, now) = d.tick(3);
        let top = d.log().remove(0);
        assert_eq!(top.subject, "fold: Storage layout");
        assert_eq!(n, vec![FoldNote { workspace: d.ws.clone(), chat: conv.clone(), event: "landed", commit: Some(top.commit.clone()) }]);
        assert!(!top.files.iter().any(|f| f.path.contains("by-hand")), "only the fold: {:?}", top.files);
        let f = d.rec(&conv).fold.unwrap();
        assert_eq!((f.state.as_str(), f.commit, f.files, f.reverted), ("done", Some(top.commit), Some(2), false));
        assert_eq!(f.lines, Some(FoldLines { added: top.added as u32, removed: top.removed as u32 }));
        assert!(!d.held() && now.fold.is_empty() && !d.io.live(&chats::session_name(&conv)));
        assert!(d.tick(3).0.is_empty(), "once");
    }

    /// Fold now on a chat folded before: its note is rewritten, and the old one
    /// is not taken for the new however long Claude reads.
    #[test]
    fn the_same_chat_folded_twice_waits_for_its_note_again() {
        let d = Desk::new("twice");
        let conv = d.chat("Naming", true);
        d.fold(&conv).unwrap();
        d.note(&conv, "first\n", 5);
        d.tick(3);
        assert_eq!(d.tick(3).0[0].event, "landed");
        d.io.ms.fetch_add(60_000, Ordering::SeqCst);
        // Ended: there is no session to look at and none to attach to again.
        let r = d.fold(&conv).unwrap();
        assert_eq!((&r["reattach"], d.rec(&conv).fold.unwrap().commit), (&json!(false), None));
        for _ in 0..10 {
            assert!(d.tick(3).0.is_empty(), "the first fold's note landed the second");
        }
        assert_eq!(d.says(&conv).as_deref(), Some("folding"));
        d.note(&conv, "second\n", 5);
        d.tick(3);
        assert_eq!(d.log()[0].subject, "fold: Naming");
        assert_eq!(d.log().iter().filter(|c| c.subject == "fold: Naming").count(), 2);
        assert_eq!(fs::read_dir(d.mem("notes")).unwrap().count(), 1, "never doubled");
    }

    #[test]
    fn a_fold_is_refused_while_it_works_waits_or_starts_and_one_at_a_time() {
        let d = Desk::new("refused");
        let (a, b) = (d.chat("Release order", true), d.chat("Subtitles", true));
        let unchanged = |why: &str| {
            assert!(!d.held() && d.rec(&a).fold.is_none() && d.acts().is_empty(), "{why}");
        };
        d.sees(activity::State::Busy);
        assert_eq!(d.fold(&a).unwrap_err(), ERR_WORKING);
        unchanged("busy");
        d.sees(activity::State::Waiting);
        assert_eq!(d.fold(&a).unwrap_err(), ERR_WAITING);
        unchanged("waiting");
        // The folder-trust question: nothing is written yet that says waiting.
        d.sees(activity::State::None);
        *d.agent.screen.lock().unwrap() = TRUST.into();
        assert_eq!(d.fold(&a).unwrap_err(), ERR_WAITING);
        unchanged("on a prompt");
        d.agent.screen.lock().unwrap().clear();
        assert_eq!(d.fold(&a).unwrap_err(), ERR_STARTING);
        unchanged("starting");
        d.sees(activity::State::Idle);
        d.io.refuse.lock().unwrap().0 = Some("2 agents are already live".into());
        assert_eq!(d.fold(&a).unwrap_err(), "2 agents are already live");
        unchanged("the limit, asked before anything is ended");
        d.io.refuse.lock().unwrap().0 = None;
        // A memory that cannot be committed first: its commit would not be the
        // fold's alone, so nothing is folded.
        let config = fs::read(d.mem(".git/config")).unwrap();
        fs::write(d.mem(".git/config"), "[").unwrap();
        assert!(d.fold(&a).unwrap_err().contains("nothing was folded"));
        unchanged("a memory that cannot be committed");
        fs::write(d.mem(".git/config"), config).unwrap();
        d.fold(&a).unwrap();
        assert_eq!(d.fold(&a).unwrap_err(), ERR_ALREADY);
        assert_eq!(d.fold(&b).unwrap_err(), "“Release order” is folding. Close this one after.");
        assert!(d.rec(&b).fold.is_none());
        // Another workspace's chats are not in its way.
        let other = store::create_at(&d.base.join("cfg/workspaces.json"), "Other").unwrap().id;
        let c = chats::prepare_at(d.ctx.workspaces_dir(), &other, |_| String::new()).unwrap();
        workspace_chat_fold(&d.ctx, other, c.id).unwrap();
    }

    /// Ended and not launched again: a failed fold, said, with the hold gone.
    #[test]
    fn a_relaunch_that_is_refused_is_a_failed_fold() {
        let d = Desk::new("relaunch");
        let conv = d.chat("X", true);
        d.io.refuse.lock().unwrap().1 = Some("no tmux".into());
        assert!(d.fold(&conv).unwrap_err().contains("no tmux"));
        let f = d.rec(&conv).fold.unwrap();
        assert_eq!((f.state.as_str(), f.left), ("failed", Some(0)));
        assert!(f.why.unwrap().ends_with("no tmux. Memory is unchanged."));
        assert!(!d.held());
        assert_eq!(d.says(&conv).as_deref(), Some("failed"));
        // Try again.
        d.io.refuse.lock().unwrap().1 = None;
        d.fold(&conv).unwrap();
        assert_eq!(d.says(&conv).as_deref(), Some("folding"));
    }

    fn turn(user: &str, at: &str, reply: &str) -> Vec<String> {
        vec![
            json!({ "type": "user", "uuid": ulid::new(), "timestamp": at, "message": { "role": "user", "content": user } }).to_string(),
            json!({ "type": "assistant", "uuid": ulid::new(), "timestamp": at, "message": { "role": "assistant", "content": [{ "type": "text", "text": reply }] } }).to_string(),
        ]
    }

    /// Nothing to keep is what Claude SAID in this fold's own turn: an earlier
    /// fold's answer, a question, or silence is not it.
    #[test]
    fn nothing_to_keep_is_said_in_this_folds_turn() {
        let d = Desk::new("nothing");
        let conv = d.chat("Chat 1", true);
        d.fold(&conv).unwrap();
        let opener = d.acts()[1].clone();
        // 2027-01-15T08:00:00Z is the script's 1_800_000_000_000.
        let set = |u: &str, at: &str, r: &str| *d.agent.lines.lock().unwrap() = turn(u, at, r);
        for (u, at, r) in [
            (opener.as_str(), "2027-01-15T07:59:59Z", "No memory changes."),
            ("something else", "2027-01-15T08:00:05Z", "No memory changes."),
            (opener.as_str(), "2027-01-15T08:00:05Z", "Should I keep the part about disks?"),
            (opener.as_str(), "2027-01-15T08:00:05Z", "No memory changes. But look at this."),
        ] {
            set(u, at, r);
            d.tick(3);
            assert!(d.tick(3).0.is_empty(), "{u} {at} {r}");
        }
        set(&opener, "2027-01-15T08:00:05Z", "I looked.\n\nNo memory changes.");
        let (n, _) = d.tick(3);
        assert_eq!((n[0].event, n[0].commit.clone()), ("nothing", None));
        let f = d.rec(&conv).fold.unwrap();
        assert_eq!((f.state.as_str(), f.commit, f.files, f.left), ("done", None, Some(0), Some(0)));
        assert_eq!(d.log().len(), 1, "no commit");
        assert!(!d.held() && !d.io.live(&chats::session_name(&conv)));
    }

    /// The words count what memory holds, or say it could not be read: never
    /// "unchanged" without the look.
    #[test]
    fn a_fold_with_no_note_fails_and_says_what_memory_holds() {
        let d = Desk::new("fails");
        let conv = d.chat("X", true);
        let run = |dirty: Option<&str>| {
            d.fold(&conv).unwrap();
            d.sees(activity::State::Busy);
            if let Some(p) = dirty {
                d.put(p, "half\n");
            }
            d.tick(3);
            let (n, now) = d.tick(LIMIT.as_secs());
            d.sees(activity::State::Idle);
            let e = now.fold[&chats::session_name(&conv)].clone();
            (n, e)
        };
        let (n, e) = run(Some("topics/half.md"));
        assert_eq!((n[0].event, n[0].commit.clone()), ("failed", None));
        assert_eq!((e.state.as_str(), e.left), ("failed", Some(1)));
        assert_eq!(e.why.as_deref(), Some("it wrote no note in five minutes, and the chat is still open. Memory may hold part of it: 1 file changed."));
        assert!(d.io.live(&chats::session_name(&conv)), "the session is left open");
        assert!(!d.held());
        assert_eq!((d.rec(&conv).fold.unwrap().state.as_str(), d.tick(3).0.len()), ("failed", 0), "said once");
        let (_, e) = run(None);
        assert_eq!(e.left, Some(0));
        assert!(e.why.unwrap().ends_with("Memory is unchanged."));
        // Memory that cannot be read: no number, and not "unchanged".
        d.fold(&conv).unwrap();
        d.sees(activity::State::Busy);
        fs::write(d.mem(".git/config"), "[").unwrap();
        d.tick(3);
        let (_, now) = d.tick(LIMIT.as_secs());
        let e = &now.fold[&chats::session_name(&conv)];
        assert_eq!(e.left, None);
        assert!(e.why.as_ref().unwrap().contains("could not be read") && !e.why.as_ref().unwrap().contains("unchanged"), "{e:?}");
    }

    #[test]
    fn a_claude_that_exits_with_no_note_fails_its_fold_at_once() {
        let d = Desk::new("exits");
        let conv = d.chat("X", true);
        d.fold(&conv).unwrap();
        d.io.up.lock().unwrap().clear();
        assert!(d.tick(3).0.is_empty());
        let (n, now) = d.tick(3);
        assert_eq!(n[0].event, "failed");
        assert!(now.fold[&chats::session_name(&conv)].why.as_ref().unwrap().starts_with("its Claude exited"));
    }

    /// A memory that cannot be committed as it lands is a failed fold, with the
    /// session left open and its note on disk.
    #[test]
    fn a_fold_whose_commit_fails_is_failed_and_not_lost() {
        let d = Desk::new("commit");
        let conv = d.chat("X", true);
        d.fold(&conv).unwrap();
        d.note(&conv, "note\n", 5);
        fs::write(d.mem(".git/config"), "[").unwrap();
        d.tick(3);
        let (n, _) = d.tick(3);
        assert_eq!(n[0].event, "failed");
        assert!(d.rec(&conv).fold.unwrap().why.unwrap().contains("could not be committed"));
        assert!(d.io.live(&chats::session_name(&conv)) && !d.held());
    }

    /// An engine restart mid-fold: the hold again and `checking`, before any
    /// tick; then the tick decides. A failed one is still shown.
    #[test]
    fn a_restart_takes_the_fold_up_again() {
        let d = Desk::new("restart");
        let (conv, failed) = (d.chat("Naming", true), d.chat("Old", false));
        chats::update_at(d.ctx.workspaces_dir(), &d.ws, &failed, |c| {
            c.fold = Some(Fold { state: chats::FOLD_FAILED.into(), why: Some("it wrote no note".into()), left: Some(2), ..Fold::default() });
        })
        .unwrap();
        d.fold(&conv).unwrap();
        d.note(&conv, "note\n", 5);
        let again = Desk::engine(&d.base, &d.io, &d.agent);
        assert!(!again.memory.is_held(&d.ws));
        ensure(&again);
        assert!(again.memory.is_held(&d.ws), "the hold is retaken before anything can commit");
        assert_eq!(again.folds.of(&conv).unwrap().state, "checking");
        let f = again.folds.of(&failed).unwrap();
        assert_eq!((f.state.as_str(), f.why.as_deref(), f.left), ("failed", Some("it wrote no note"), Some(2)));
        assert_eq!(workspace_chat_fold(&again, d.ws.clone(), failed).unwrap_err(), "“Naming” is folding. Close this one after.");
        let (n, now) = d.tick_on(&again, 3);
        assert!(n.is_empty());
        assert_eq!(now.fold[&chats::session_name(&conv)].state, "folding", "the first tick has looked");
        let (n, _) = d.tick_on(&again, 3);
        assert_eq!((n[0].event, again.memory.is_held(&d.ws)), ("landed", false));
        // The engine it was taken from finds it over and lets go: no second
        // commit, no second record.
        let (n, _) = d.tick(3);
        assert!(n.is_empty() && !d.held() && d.says(&conv).is_none());
    }

    /// Its session gone with the machine: `checking` until it is sure.
    #[test]
    fn a_restart_with_no_session_left_decides_by_the_note() {
        let d = Desk::new("reboot");
        let (a, b) = (d.chat("A", true), ());
        let _ = b;
        d.fold(&a).unwrap();
        d.io.up.lock().unwrap().clear();
        let again = Desk::engine(&d.base, &d.io, &d.agent);
        ensure(&again);
        let (n, now) = d.tick_on(&again, 3);
        assert!(n.is_empty());
        assert_eq!(now.fold[&chats::session_name(&a)].state, "checking");
        assert_eq!(d.tick_on(&again, 3).0[0].event, "failed");
    }

    /// Stop: what it wrote is committed under its own words, and the record
    /// says it was not folded.
    #[test]
    fn stopping_commits_what_is_left_under_its_own_subject() {
        let d = Desk::new("stop");
        let conv = d.chat("Naming", true);
        d.fold(&conv).unwrap();
        d.put("topics/half.md", "half\n");
        let was = d.ctx.folds.close(&conv).unwrap();
        let (stopped, change) = closed(&d.ctx, &d.ws, &d.rec(&conv), was).unwrap();
        assert_eq!((stopped, change.unwrap().subject.as_str()), (true, "Fold stopped: Naming"));
        assert!(!d.held() && d.says(&conv).is_none());
        assert_eq!(d.rec(&conv).fold.unwrap().state, "none");
        assert!(d.tick(3).0.is_empty());
        // Through the row, and a delete does the same first.
        let conv = d.chat("Second", true);
        d.fold(&conv).unwrap();
        d.put("topics/half.md", "more\n");
        let r = crate::workspacechats::workspace_chat_close(&d.ctx, d.ws.clone(), conv.clone()).unwrap();
        assert_eq!((&r["closed"], &r["stopped_fold"], r["commit"].as_str().map(memory::is_sha)), (&json!(true), &json!(true), Some(true)), "{r}");
        d.fold(&conv).unwrap();
        d.put("topics/half.md", "again\n");
        let r = crate::workspacechats::workspace_chat_remove(&d.ctx, d.ws.clone(), conv.clone()).unwrap();
        assert_eq!(r["stopped_fold"], json!(true));
        assert_eq!(d.log()[0].subject, "Fold stopped: Second");
        assert!(!d.held() && d.says(&conv).is_none());
        // Nothing left to commit is no commit.
        let conv = d.chat("Third", true);
        d.fold(&conv).unwrap();
        let r = crate::workspacechats::workspace_chat_close(&d.ctx, d.ws.clone(), conv).unwrap();
        assert_eq!((&r["stopped_fold"], &r["commit"]), (&json!(true), &Value::Null));
    }

    /// Closed without folding: its changes are committed under their own
    /// subject, but never out of another chat's fold.
    #[test]
    fn closing_without_folding_commits_what_it_left_but_not_a_folds_work() {
        let d = Desk::new("close");
        let (a, b, c) = (d.chat("A", true), d.chat("B", true), d.chat("C", true));
        let close = |x: &str| crate::workspacechats::workspace_chat_close(&d.ctx, d.ws.clone(), x.to_string()).unwrap();
        d.put("topics/a.md", "a\n");
        let r = close(&a);
        assert_eq!((&r["closed"], &r["stopped_fold"]), (&json!(true), &json!(false)));
        assert_eq!((d.log()[0].subject.as_str(), d.rec(&a).fold), ("Chat closed: A", None));
        // Already ended: what is dirty is not its doing.
        d.put("topics/a.md", "by hand\n");
        assert_eq!(close(&a)["commit"], Value::Null);
        d.fold(&b).unwrap();
        d.put("topics/b.md", "half of B's fold\n");
        assert_eq!(close(&c)["commit"], Value::Null);
        assert!(d.held(), "B still holds its memory");
        d.note(&b, "note\n", 5);
        d.tick(3);
        d.tick(3);
        assert!(d.log()[0].files.iter().any(|f| f.path == "topics/b.md"), "{:?}", d.log()[0]);
        // A failed fold that is closed is simply not folded.
        chats::update_at(d.ctx.workspaces_dir(), &d.ws, &a, |x| x.fold = Some(Fold { state: chats::FOLD_FAILED.into(), ..Fold::default() })).unwrap();
        close(&a);
        assert_eq!(d.rec(&a).fold.unwrap().state, "none");
    }

    /// Undo and Redo are the memory's; the chat's record follows them.
    #[test]
    fn an_undone_fold_says_so_and_a_redone_one_no_longer() {
        let d = Desk::new("undo");
        let conv = d.chat("Naming", true);
        d.fold(&conv).unwrap();
        assert!(crate::memory::workspace_memory_revert(&d.ctx, d.ws.clone(), d.log()[0].commit.clone()).unwrap_err().contains("folding"));
        d.note(&conv, "note\n", 5);
        d.tick(3);
        d.tick(3);
        let commit = d.rec(&conv).fold.unwrap().commit.unwrap();
        let undo = crate::memory::workspace_memory_revert(&d.ctx, d.ws.clone(), commit.clone()).unwrap().change.unwrap();
        assert!(d.rec(&conv).fold.unwrap().reverted);
        crate::memory::workspace_memory_revert(&d.ctx, d.ws.clone(), undo.commit).unwrap();
        let f = d.rec(&conv).fold.unwrap();
        assert_eq!((f.reverted, f.commit), (false, Some(commit)));
    }

    /// The poll never waits for a start or a close: it looks again next tick.
    #[test]
    fn a_tick_passes_over_a_workspace_that_is_busy_with_its_life() {
        let d = Desk::new("lock");
        let conv = d.chat("X", true);
        d.fold(&conv).unwrap();
        d.note(&conv, "note\n", 5);
        // Another workspace's fold, asked for in the same second.
        let other = store::create_at(&d.base.join("cfg/workspaces.json"), "Other").unwrap().id;
        let c = chats::prepare_at(d.ctx.workspaces_dir(), &other, |_| String::new()).unwrap();
        workspace_chat_fold(&d.ctx, other.clone(), c.id.clone()).unwrap();
        let p = d.base.join("ws").join(&other).join(&c.note);
        fs::write(&p, "note\n").unwrap();
        let at = std::time::UNIX_EPOCH + Duration::from_millis(d.io.now_ms() + 5);
        fs::File::options().write(true).open(&p).unwrap().set_modified(at).unwrap();
        // The FIRST the poll comes to is locked; the one after it is not.
        let (locked, free) = if conv < c.id { (d.ws.clone(), other) } else { (other, d.ws.clone()) };
        let lock = life_lock(&d.ctx, &locked);
        let held = lock.lock().unwrap();
        let mut landed = Vec::new();
        for _ in 0..4 {
            landed.extend(d.tick(3).0.into_iter().map(|n| n.workspace));
        }
        assert_eq!(landed, vec![free]);
        drop(held);
        d.tick(3);
        assert_eq!(d.tick(3).0[0].event, "landed");
    }

    /// The record is in the chat's own cwd. One an agent rewrote names no
    /// commit, and a fold it called over is let go, never acted on.
    #[test]
    fn a_record_an_agent_rewrote_reaches_no_git_and_holds_nothing() {
        let d = Desk::new("forged");
        let conv = d.chat("X", true);
        chats::update_at(d.ctx.workspaces_dir(), &d.ws, &conv, |c| {
            c.fold = Some(Fold { state: "done".into(), commit: Some("--upload-pack=x".into()), ..Fold::default() });
        })
        .unwrap();
        assert_eq!(d.rec(&conv).fold.unwrap().commit, None);
        mark_reverted(&d.ctx, &d.ws);
        d.fold(&conv).unwrap();
        chats::update_at(d.ctx.workspaces_dir(), &d.ws, &conv, |c| c.fold.as_mut().unwrap().state = "done".into()).unwrap();
        let before = d.log().len();
        d.put("topics/x.md", "x\n");
        assert!(d.tick(3).0.is_empty());
        assert!(!d.held() && d.says(&conv).is_none() && d.log().len() == before);
    }
}
