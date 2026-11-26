//! The fold: closing a chat writes what is worth keeping of it into the
//! workspace's memory (`docs/proposals/workspace-chats.md` §3.1, §3.2, §9).
//!
//! **What one is.** `workspace_chat_fold` commits what is dirty (so the fold's
//! commit holds only the fold), holds the memory, writes the chat's
//! `fold.md`, records the fold in `chats.json`, ends the session and launches
//! the SAME conversation again on a fixed opener that reads that file. Nothing
//! is typed. The poll does the rest ([`tick`]).
//!
//! **When it lands.** When the chat's note has been written since (its revision
//! is not the one before, whatever any clock says), its Claude has been idle
//! for one tick (seen idle twice running), and its own turn in the transcript
//! has said its last words: the turn that this ATTEMPT's opener began, found by
//! the attempt's id, ended by Claude's own text and not by a stop or an error.
//! A note that is there is not enough, and neither is a session that reads
//! idle: one that has just launched does. The turn ending with the line
//! `fold.md` gives for it closes the fold with no change.
//!
//! **When it fails.** Five minutes with no new note, and five more with a note
//! and no end, on a clock that stands still while Claude waits on a prompt; or
//! its Claude exited before its last words. A tmux that does not answer, or
//! finds no server (it looks the same when one refuses), is UNKNOWN, never gone: it counts toward the five minutes and nothing else. The
//! session stays open, and what memory holds is COUNTED from its dirty tree:
//! the words never say "unchanged" without that look. Try again folds it again;
//! closing the chat keeps what was written, under its own subject.
//!
//! **One at a time in a workspace, in any engine.** The reservation is the
//! record itself (`chats::folding_at`), read and written under the workspace's
//! cross-process lock (`wslock`) together with the baseline commit. Undo and
//! the quiet commit ask the same record under the same lock.
//!
//! **Landing survives a crash at each step.** The commit carries the attempt's
//! id, so an engine that died before the receipt finds its own commit and makes
//! no second one; the receipt is written before the session is ended, and says
//! that it still is to be. A receipt that cannot be written ends nothing: the
//! next tick tries again.
//!
//! **What is running is in memory** ([`Folds`]); the record says which chats
//! were folding, and the engine takes them up again when it starts, each
//! workspace until its record has been read ([`ensure`]): one that cannot be
//! read is held, not skipped. Until the first tick has looked they read
//! `checking`.
//!
//! **Order of the locks**, outermost first: a workspace's `life_lock`, the
//! chat's own lock, `wslock`, the record's. Nothing takes an earlier one while
//! it holds a later one, and no lock but the first two is held across a launch.
//!
//! Every row here runs under the workspace's `life_lock`, and the poll only
//! TRIES it: a start that is launching is not waited for, the next tick is.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use worktrees_core::memory::{self as memory, Change};
use worktrees_core::workspacechats::{self as chats, Chat, Fold, FoldLines, FoldTurn};
use worktrees_core::workspaces::{self as store, Workspace};
use worktrees_core::tmux::Up;
use worktrees_core::{activity, desksessions, tmux, turns, ulid, wsfolder, wslock};

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
pub const ERR_UNSEEN: &str = "tmux did not answer, so whether it is running cannot be told: try again";
pub const ERR_ALREADY: &str = "this chat is folding already";

/// What a fold does outside the files: a test scripts it.
pub trait FoldIo: Send + Sync {
    /// Milliseconds since the epoch.
    fn now_ms(&self) -> u64;
    /// The day it is at this host, for a note's name.
    fn today(&self) -> String;
    /// The poll's clock, for what it counts.
    fn clock(&self) -> Duration;
    /// The sessions that are up, as tmux said it.
    fn sessions(&self) -> Up;
    /// Whether the chat could be relaunched (the limits), before it is ended.
    fn admit(&self, root: &Path, ideas: &Path, w: &Workspace, conv: &str) -> Result<(), String>;
    /// End the chat's session; whether it ran.
    fn close(&self, conv: &str) -> Result<bool, String>;
    /// Launch the conversation again, resumed on `opener`.
    fn relaunch(&self, root: &Path, ideas: &Path, w: &Workspace, conv: &str, opener: String) -> Result<String, String>;
    /// Whether the engine dies here: a test's, at each step of a landing.
    fn dies(&self, _at: &'static str) -> bool {
        false
    }
}

struct Live;

impl FoldIo for Live {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
    }
    fn today(&self) -> String {
        chats::local_date(worktrees_core::sysclock::now_epoch())
    }
    fn clock(&self) -> Duration {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }
    fn sessions(&self) -> Up {
        tmux::sessions_up()
    }
    fn admit(&self, root: &Path, ideas: &Path, w: &Workspace, conv: &str) -> Result<(), String> {
        chats::may_relaunch_at(root, ideas, w, conv)
    }
    fn close(&self, conv: &str) -> Result<bool, String> {
        chats::close(conv)
    }
    fn relaunch(&self, root: &Path, ideas: &Path, w: &Workspace, conv: &str, opener: String) -> Result<String, String> {
        chats::launch_at(root, ideas, w, conv, chats::Start::Resume, Some(opener))
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
    /// Its receipt is written; its session is still to be ended.
    Closing,
}

/// One chat's fold, running or failed.
#[derive(Clone, Debug)]
struct Run {
    workspace: String,
    /// The attempt its opener and its commit carry. Empty for a record of a
    /// build before attempts: no turn and no commit is ever that one's.
    attempt: String,
    /// The note's revision as it was asked.
    before: Option<String>,
    said: Said,
    /// Its clock: what has run of [`LIMIT`], and the last tick's reading.
    spent: Duration,
    last: Option<Duration>,
    /// A new note has been seen: the five minutes began again.
    noted: bool,
    idle: u32,
    gone: u32,
}

impl Run {
    fn new(workspace: &str, attempt: &str, before: Option<String>, said: Said) -> Run {
        Run { workspace: workspace.into(), attempt: attempt.into(), before, said, spent: Duration::ZERO, last: None, noted: false, idle: 0, gone: 0 }
    }
    fn running(&self) -> bool {
        matches!(self.said, Said::Checking | Said::Folding | Said::Waiting)
    }
}

/// What one tick sees of a fold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seen {
    /// `None`: tmux did not say.
    live: Option<bool>,
    busy: bool,
    waiting: bool,
    /// Its session is up and its Claude has written nothing to say so yet.
    starting: bool,
    /// Its note is not the one that was there before.
    new: bool,
}

/// What a tick decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Wait,
    /// Idle: see what its own turn came to.
    Ask,
    TimedOut,
    Gone,
}

/// One tick of one fold. `now` is [`FoldIo::clock`].
fn step(r: &mut Run, seen: Seen, now: Duration) -> Next {
    let since = r.last.map_or(Duration::ZERO, |l| now.saturating_sub(l));
    r.last = Some(now);
    let was_waiting = r.said == Said::Waiting;
    let up = seen.live == Some(true);
    let idle = up && !seen.busy && !seen.waiting && !seen.starting;
    r.idle = if idle { r.idle + 1 } else { 0 };
    r.gone = if seen.live == Some(false) { r.gone + 1 } else { 0 };
    if up && seen.waiting {
        r.said = Said::Waiting;
        return Next::Wait;
    }
    if !was_waiting {
        r.spent += since;
    }
    if up {
        r.said = Said::Folding;
    }
    if seen.new && !r.noted {
        r.noted = true;
        r.spent = Duration::ZERO;
    }
    if r.gone >= TICKS {
        return Next::Gone;
    }
    if r.idle >= TICKS {
        return Next::Ask;
    }
    if r.spent >= LIMIT { Next::TimedOut } else { Next::Wait }
}

/// Which workspaces' records this engine has read for folds it must take up.
#[derive(Default)]
struct Taken {
    done: HashSet<String>,
    /// Held because their record could not be read, and what it said.
    unknown: BTreeMap<String, String>,
}

/// The folds of this engine, by chat id: running, and failed ones a client is
/// still to be told of. One per engine.
pub struct Folds {
    runs: Mutex<BTreeMap<String, Run>>,
    taken: Mutex<Taken>,
    io: Arc<dyn FoldIo>,
}

impl Default for Folds {
    fn default() -> Folds {
        Folds::new(Arc::new(Live))
    }
}

impl Folds {
    pub fn new(io: Arc<dyn FoldIo>) -> Folds {
        Folds { runs: Mutex::new(BTreeMap::new()), taken: Mutex::default(), io }
    }
    fn runs(&self) -> MutexGuard<'_, BTreeMap<String, Run>> {
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The chat of workspace `ws` that is folding now.
    fn running_in(&self, ws: &str) -> Option<String> {
        self.runs().iter().find(|(_, r)| r.workspace == ws && r.running()).map(|(c, _)| c.clone())
    }
    /// Whether chat `conv` is folding now.
    pub fn is_folding(&self, conv: &str) -> bool {
        self.runs().get(conv).is_some_and(Run::running)
    }
    /// How chat `conv`'s fold stands, when there is something to say.
    pub fn of(&self, conv: &str) -> Option<FoldOf> {
        self.runs().get(conv).filter(|r| r.said != Said::Closing).map(|r| fold_of(conv, r))
    }
    /// End chat `conv`'s session; whether it ran.
    pub fn close(&self, conv: &str) -> Result<bool, String> {
        self.io.close(conv)
    }
    /// Nothing more is said of workspace `ws`'s chats until its record is read
    /// again, if it still has one.
    fn forget(&self, ws: &str) {
        self.runs().retain(|_, r| r.workspace != ws);
        let mut t = self.taken.lock().unwrap_or_else(|e| e.into_inner());
        t.done.remove(ws);
        t.unknown.remove(ws);
    }
    /// The event's map, by session.
    fn by_session(&self) -> BTreeMap<String, FoldOf> {
        self.runs().iter().filter(|(_, r)| r.said != Said::Closing).map(|(c, r)| (chats::session_name(c), fold_of(c, r))).collect()
    }
}

fn fold_of(conv: &str, r: &Run) -> FoldOf {
    let (state, why, left) = match &r.said {
        Said::Checking => ("checking", None, None),
        Said::Folding => ("folding", None, None),
        Said::Waiting => ("waiting", None, None),
        Said::Failed { why, left } => ("failed", Some(why.clone()), *left),
        Said::Closing => ("done", None, None),
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

/// Workspace `ws`'s chats are being removed, or are gone: no run of it, and no
/// hold. If it is still there after all, its record is read again and what it
/// says is taken up, so neither half can be left without the other.
pub fn forget(ctx: &Ctx, ws: &str) {
    ctx.folds.forget(ws);
    ctx.memory.release(ws);
}

/// Take up the folds an engine left: the hold again, and `checking` until a
/// tick has looked; a failed one is shown again, and one whose session was
/// still to be ended has it ended. Each workspace until its record has been
/// read: one that cannot be is HELD meanwhile, for what it cannot say. Called
/// by every row that reads or starts something a fold bears on, and by the poll.
pub fn ensure(ctx: &Ctx) {
    let Ok(f) = store::read_at(ctx.workspaces_file()) else { return };
    let mut t = ctx.folds.taken.lock().unwrap_or_else(|e| e.into_inner());
    t.done.retain(|id| f.workspaces.iter().any(|w| w.id == *id));
    for w in f.workspaces.iter().filter(|w| !t.done.contains(&w.id)).map(|w| w.id.clone()).collect::<Vec<_>>() {
        let list = match chats::list_at(ctx.workspaces_dir(), &w) {
            Ok(l) => l,
            Err(e) => {
                if t.unknown.get(&w) != Some(&e) {
                    crate::log::warn(&format!("workspace {w}: whether a chat was folding cannot be read, so its memory is held: {e}"));
                    ctx.memory.hold(&w);
                    t.unknown.insert(w, e);
                }
                continue;
            }
        };
        let was_unknown = t.unknown.remove(&w).is_some();
        let mut folding = false;
        for c in list {
            let Some(fold) = &c.fold else { continue };
            let said = match fold.state.as_str() {
                chats::FOLDING if !folding => Said::Checking,
                chats::FOLD_FAILED => Said::Failed { why: fold.why.clone().unwrap_or_default(), left: fold.left },
                chats::FOLDED if fold.closing => Said::Closing,
                _ => continue,
            };
            if said == Said::Checking {
                folding = true;
                ctx.memory.hold(&w);
            }
            let run = Run::new(&w, fold.attempt.as_deref().unwrap_or_default(), fold.before.clone(), said);
            ctx.folds.runs().entry(c.id.clone()).or_insert(run);
        }
        if was_unknown && !folding {
            ctx.memory.release(&w);
        }
        t.done.insert(w);
    }
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
    free(ctx, &id, &conv)?;
    let io = ctx.folds.io.clone();
    let at = locus_of(ctx, &id, &c)?;
    // No line or answer reaches the chat between the look and its end.
    let chat_lock = ctx.agents.place_lock(&at.lock_key());
    let _chat = chat_lock.lock().unwrap_or_else(|e| e.into_inner());
    let live = match io.sessions() {
        Up::Are(s) => s.contains(&chats::session_name(&conv)),
        // An ended chat, and the last session of all ended with it.
        Up::NoServer => false,
        Up::Unknown => return Err(ERR_UNSEEN.into()),
    };
    if live {
        quiet(ctx, &at)?;
    }
    io.admit(root, ctx.ideas_dir(), &w, &conv)?;
    let began = reserve(ctx, &w, &c, &at, live).and_then(|(opener, fold)| launch(ctx, &w, &c, live, opener, fold));
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

/// Whether no chat of workspace `id` is folding, in this engine or, by the
/// record, in any other.
fn free(ctx: &Ctx, id: &str, conv: &str) -> Result<(), String> {
    let here = ctx.folds.running_in(id);
    match here.map_or_else(|| chats::folding_at(ctx.workspaces_dir(), id), |c| Ok(Some(c)))? {
        Some(x) if x == conv => Err(ERR_ALREADY.into()),
        Some(x) => Err(format!("“{}” is folding. Close this one after.", title_of(ctx, id, &x))),
        None => Ok(()),
    }
}

/// The reservation, under the workspace's lock: whether it is free and the chat
/// still quiet, asked again inside it; the baseline commit, so the fold's holds
/// only the fold; and the attempt, in the record. What is dirty from here on is
/// the fold's.
fn reserve(ctx: &Ctx, w: &Workspace, c: &Chat, at: &Locus, live: bool) -> Result<(String, Fold), String> {
    let (root, io) = (ctx.workspaces_dir(), ctx.folds.io.clone());
    let _lock = wslock::take(root, &w.id)?;
    free(ctx, &w.id, &c.id)?;
    if live {
        quiet(ctx, at)?;
    }
    crate::memory::before_launch(ctx, w)?;
    crate::memory::memory_commit_if_dirty(ctx, &w.id, None).map_err(|e| format!("the memory could not be committed, so nothing was folded: {e}"))?;
    let attempt = ulid::new();
    // Its note is named the first time, by what the chat is called by then.
    let mut named = chats::named_at(root, &w.id, &c.id)?;
    chats::name_note(&mut named, &io.today());
    let opener = chats::write_fold_at(root, &w.id, &named, &attempt)?;
    // A relaunch is a launch: what it is told is as fresh as its rules.
    chats::write_context_at(root, &w.id, &c.id, &chats::context_text(&crate::workspacechats::context(ctx, w, &chats::policy(w)?, &c.id)))?;
    let fold = Fold {
        asked: desksessions::now_iso(),
        asked_ms: Some(io.now_ms()),
        before: chats::note_rev(root, &w.id, &named),
        attempt: Some(attempt),
        state: chats::FOLDING.into(),
        ..Fold::default()
    };
    chats::update_at(root, &w.id, &c.id, |x| {
        x.note = named.note.clone();
        x.attempts.extend(fold.attempt.clone());
        x.fold = Some(fold.clone());
    })?;
    ctx.memory.hold(&w.id);
    ctx.folds.runs().insert(c.id.clone(), Run::new(&w.id, fold.attempt.as_deref().unwrap_or_default(), fold.before.clone(), Said::Folding));
    Ok((opener, fold))
}

/// End the session that is reserved and launch it again. The reservation is
/// undone while the session still runs; once it has been ended, a relaunch that
/// fails is a failed fold.
fn launch(ctx: &Ctx, w: &Workspace, c: &Chat, live: bool, opener: String, fold: Fold) -> Result<(String, Fold), String> {
    let (root, io) = (ctx.workspaces_dir(), ctx.folds.io.clone());
    if live {
        if let Err(e) = io.close(&c.id) {
            ctx.folds.runs().remove(&c.id);
            chats::update_at(root, &w.id, &c.id, |x| x.fold = c.fold.clone())?;
            return Err(e);
        }
    }
    match io.relaunch(root, ctx.ideas_dir(), w, &c.id, opener) {
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
/// let go. One whose record cannot be written is not failed yet: it is looked
/// at again next tick. Answers whether it is.
fn fail(ctx: &Ctx, ws: &str, conv: &str, why: String) -> bool {
    let (left, holds) = holds(ctx, ws);
    let why = format!("{}. {holds}", why.trim_end_matches('.'));
    if let Err(e) = chats::update_at(ctx.workspaces_dir(), ws, conv, |c| {
        let f = c.fold.get_or_insert_with(Fold::default);
        f.state = chats::FOLD_FAILED.into();
        f.why = Some(why.clone());
        f.left = left;
    }) {
        crate::log::warn(&format!("workspace {ws}: could not record chat {conv}'s failed fold: {e}"));
        return false;
    }
    if let Some(r) = ctx.folds.runs().get_mut(conv) {
        r.said = Said::Failed { why, left };
    }
    ctx.memory.release(ws);
    true
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
    let stopped = run.as_ref().is_some_and(Run::running);
    let title = chats::named_at(ctx.workspaces_dir(), ws, &c.id).map_or_else(|_| "a chat".into(), |c| c.title);
    // With what another engine's fold may be writing, one lock for the look and
    // the commit.
    let _lock = wslock::take(ctx.workspaces_dir(), ws)?;
    if run.is_some() || c.fold.as_ref().is_some_and(|f| [chats::FOLDING, chats::FOLD_FAILED].contains(&f.state.as_str())) {
        let r = chats::update_at(ctx.workspaces_dir(), ws, &c.id, |x| {
            x.fold = Some(Fold { asked: x.fold.take().map(|f| f.asked).unwrap_or_default(), state: chats::FOLD_NONE.into(), ..Fold::default() });
        });
        if let Err(e) = r {
            crate::log::warn(&format!("workspace {ws}: could not record chat {}'s fold as stopped: {e}", c.id));
        }
    }
    if stopped {
        let done = crate::memory::memory_commit_if_dirty(ctx, ws, Some(&subject("Fold stopped", &title)));
        ctx.memory.release(ws);
        return Ok((true, done?));
    }
    if !ran || ctx.memory.is_held(ws) || !matches!(chats::folding_at(ctx.workspaces_dir(), ws), Ok(None)) {
        return Ok((false, None));
    }
    Ok((false, crate::memory::memory_commit_if_dirty(ctx, ws, Some(&subject("Chat closed", &title)))?))
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

/// What the fold's own turn came to.
fn own_turn(ctx: &Ctx, ws: &str, c: &Chat, at: &Locus, attempt: &str) -> FoldTurn {
    let Ok(folder) = wsfolder::folder_at(ctx.workspaces_dir(), ws) else { return FoldTurn::Open };
    let Some(lane) = agents::ended_lane(at).filter(|_| !attempt.is_empty()) else { return FoldTurn::Open };
    let Ok(lines) = ctx.agents.io().transcript(at, &lane) else { return FoldTurn::Open };
    chats::fold_turn(&turns::claude(&lines, turns::MAX_TURNS), &chats::fold_opener(&folder, &c.id, attempt))
}

/// The poll's part for the folds: each running one is looked at, and the ones
/// that are over are closed. `now` gets the fold map. Answers what was
/// decided, for the audit log.
pub fn tick(ctx: &Ctx, now: &mut WorkspaceSessions) -> Vec<FoldNote> {
    ensure(ctx);
    let mut notes = Vec::new();
    let running: Vec<(String, String)> = ctx.folds.runs().iter().filter(|(_, r)| r.running() || r.said == Said::Closing).map(|(c, r)| (c.clone(), r.workspace.clone())).collect();
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
    let io = ctx.folds.io.clone();
    let note = |event, commit| Some(FoldNote { workspace: ws.into(), chat: conv.into(), event, commit });
    let (closing, attempt, before) = ctx.folds.runs().get(conv).map(|r| (r.said == Said::Closing, r.attempt.clone(), r.before.clone()))?;
    if closing {
        end(ctx, ws, conv);
        return None;
    }
    // A record that cannot be read says nothing: the fold and its hold stay,
    // and the next tick asks again.
    let rec = match chats::read_record_at(root, ws) {
        Ok(r) => r,
        Err(e) => {
            crate::log::warn(&format!("workspace {ws}: chat {conv} is folding and its record cannot be read: {e}"));
            return None;
        }
    };
    let mine = |c: &Chat| c.fold.as_ref().is_some_and(|f| f.state == chats::FOLDING && f.attempt.as_deref().unwrap_or_default() == attempt);
    if !rec.conversations.iter().any(|c| c.id == conv && mine(c)) {
        // Ended, or over in another engine: nothing of this one's to decide.
        ctx.folds.runs().remove(conv);
        ctx.memory.release(ws);
        return None;
    }
    let c = chats::find_at(root, ws, conv).ok()?;
    let session = chats::session_name(conv);
    let at = locus_of(ctx, ws, &c).ok()?;
    let has = |v: &[String]| v.iter().any(|s| *s == session);
    // The poll's panes are empty when tmux did not answer: only a list it did
    // give says a session is not there.
    let live = if has(&now.live) {
        Some(true)
    } else {
        match io.sessions() {
            Up::Are(s) => Some(s.contains(&session)),
            Up::NoServer | Up::Unknown => None,
        }
    };
    let state = if live == Some(true) { ctx.agents.io().activity(&at).state } else { activity::State::None };
    let rev = chats::note_rev(root, ws, &c);
    let seen = Seen {
        live,
        busy: has(&now.busy) || state == activity::State::Busy,
        waiting: has(&now.waiting) || state == activity::State::Waiting,
        starting: state == activity::State::None,
        new: rev.is_some() && rev != before,
    };
    let (next, over, noted) = {
        let mut runs = ctx.folds.runs();
        let r = runs.get_mut(conv)?;
        let next = step(r, seen, io.clock());
        (next, r.spent >= LIMIT, r.noted)
    };
    let next = match next {
        Next::Ask => match own_turn(ctx, ws, &c, &at, &attempt) {
            FoldTurn::Finished if seen.new => return land(ctx, ws, conv, &attempt, true),
            FoldTurn::Nothing => return land(ctx, ws, conv, &attempt, seen.new),
            _ if over => Next::TimedOut,
            _ => Next::Wait,
        },
        n => n,
    };
    let why = match next {
        Next::Wait | Next::Ask => return None,
        Next::TimedOut if live.is_none() => "tmux did not answer for five minutes, so whether it was done could not be told",
        Next::TimedOut if noted => "it wrote its note and did not finish in five minutes, and the chat is still open",
        Next::TimedOut => "it wrote no note in five minutes, and the chat is still open",
        Next::Gone if noted => "its Claude exited after it wrote its note and before it was done",
        Next::Gone => "its Claude exited before it wrote a note",
    };
    if fail(ctx, ws, conv, why.into()) { note("failed", None) } else { None }
}

/// A fold is over: its commit, its receipt, and then its session. Under the
/// workspace's lock to the receipt, with the record read again inside it, so
/// two engines land it once. `wrote`: there is something to commit, and it is
/// looked for first by the attempt's id, for an engine that died after it.
fn land(ctx: &Ctx, ws: &str, conv: &str, attempt: &str, wrote: bool) -> Option<FoldNote> {
    let (root, io) = (ctx.workspaces_dir(), ctx.folds.io.clone());
    let commit = {
        // Someone is in the memory: next tick.
        let _lock = match wslock::try_take(root, ws) {
            Ok(Some(l)) => l,
            Ok(None) => return None,
            Err(e) => {
                crate::log::warn(&format!("workspace {ws}: chat {conv}'s fold could not take the memory: {e}"));
                return None;
            }
        };
        let c = chats::find_at(root, ws, conv).ok()?;
        if !c.fold.as_ref().is_some_and(|f| f.state == chats::FOLDING && f.attempt.as_deref() == Some(attempt)) {
            return None;
        }
        let c = chats::named_at(root, ws, conv).ok()?;
        if io.dies("before the commit") {
            return None;
        }
        let made = if wrote {
            memory::fold_commit_at(root, ws, attempt).and_then(|found| match found {
                Some(c) => Ok(Some(c)),
                None => memory::commit_fold_at(root, ws, &subject("fold", &c.title), attempt),
            })
        } else {
            Ok(None)
        };
        let change = match made {
            Ok(c) => c,
            Err(e) => {
                let failed = fail(ctx, ws, conv, format!("its note was written, and the memory could not be committed: {e}"));
                return failed.then(|| FoldNote { workspace: ws.into(), chat: conv.into(), event: "failed", commit: None });
            }
        };
        if io.dies("after the commit") {
            return None;
        }
        // With no commit, what memory is left with is said only if it was looked at.
        let left = if change.is_none() { holds(ctx, ws).0 } else { None };
        if let Err(e) = chats::update_at(root, ws, conv, |x| {
            let f = x.fold.get_or_insert_with(Fold::default);
            f.state = chats::FOLDED.into();
            f.closing = true;
            f.commit = change.as_ref().map(|c| c.commit.clone());
            f.files = Some(change.as_ref().map_or(0, |c| c.files.len() as u32));
            f.lines = Some(change.as_ref().map_or(FoldLines::default(), |c| FoldLines { added: c.added as u32, removed: c.removed as u32 }));
            f.left = left;
            f.why = None;
        }) {
            // No receipt: nothing is ended and nothing let go. The next tick
            // finds the commit by its attempt and writes it.
            crate::log::warn(&format!("workspace {ws}: could not record chat {conv}'s fold: {e}"));
            return None;
        }
        change.map(|c| c.commit)
    };
    if let Some(r) = ctx.folds.runs().get_mut(conv) {
        r.said = Said::Closing;
    }
    ctx.memory.release(ws);
    if !io.dies("after the receipt") {
        end(ctx, ws, conv);
    }
    Some(FoldNote { workspace: ws.into(), chat: conv.into(), event: if commit.is_some() { "landed" } else { "nothing" }, commit })
}

/// End the session of a fold whose receipt is written, and say it is ended only
/// once tmux says so: until then it is tried again, by this engine's poll or the
/// next one's start.
fn end(ctx: &Ctx, ws: &str, conv: &str) {
    let io = ctx.folds.io.clone();
    if let Err(e) = io.close(conv) {
        crate::log::warn(&format!("workspace {ws}: chat {conv} folded and its session could not be ended: {e}"));
    }
    let ended = match io.sessions() {
        Up::Are(s) => !s.contains(&chats::session_name(conv)),
        // The last one, which we have just ended.
        Up::NoServer => true,
        Up::Unknown => false,
    };
    if io.dies("after the close") || !ended {
        return;
    }
    if let Err(e) = chats::update_at(ctx.workspaces_dir(), ws, conv, |c| {
        if let Some(f) = c.fold.as_mut() {
            f.closing = false;
        }
    }) {
        crate::log::warn(&format!("workspace {ws}: could not record that chat {conv} is closed: {e}"));
        return;
    }
    ctx.folds.runs().remove(conv);
    touched(ctx, ws, conv);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
        /// tmux does not answer.
        dumb: AtomicBool,
        /// It has no server once nothing is up.
        no_server: AtomicBool,
        /// Called as a fold is admitted, before it reserves anything: once.
        admitted: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        /// Where the engine dies.
        fatal: Mutex<Option<&'static str>>,
    }

    impl FoldIo for Script {
        fn now_ms(&self) -> u64 {
            self.ms.load(Ordering::SeqCst)
        }
        fn today(&self) -> String {
            "2026-11-24".into()
        }
        fn clock(&self) -> Duration {
            Duration::from_secs(self.secs.load(Ordering::SeqCst))
        }
        fn sessions(&self) -> Up {
            let up = self.up.lock().unwrap().clone();
            match self.dumb.load(Ordering::SeqCst) {
                true => Up::Unknown,
                // With the last session goes the server; a script that says so
                // keeps one of its own.
                false if up.is_empty() && self.no_server.load(Ordering::SeqCst) => Up::NoServer,
                false => Up::Are(up.into_iter().collect()),
            }
        }
        fn dies(&self, at: &'static str) -> bool {
            *self.fatal.lock().unwrap() == Some(at)
        }
        fn admit(&self, _: &Path, _: &Path, _: &Workspace, _: &str) -> Result<(), String> {
            let hook = self.admitted.lock().unwrap().take();
            if let Some(f) = hook {
                f();
            }
            self.refuse.lock().unwrap().0.clone().map_or(Ok(()), Err)
        }
        fn close(&self, conv: &str) -> Result<bool, String> {
            if self.dumb.load(Ordering::SeqCst) {
                return Ok(false);
            }
            let was = self.up.lock().unwrap().remove(&chats::session_name(conv));
            if was {
                self.log.lock().unwrap().push("close".into());
            }
            Ok(was)
        }
        fn relaunch(&self, _: &Path, _: &Path, _: &Workspace, conv: &str, opener: String) -> Result<String, String> {
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
        /// The chat's note, written `after` ms past the script's clock, and no
        /// word from Claude that it is done.
        fn note_only(&self, conv: &str, text: &str, after: i64) {
            let p = self.base.join("ws").join(&self.ws).join(self.rec(conv).note);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, text).unwrap();
            let at = std::time::UNIX_EPOCH + Duration::from_millis((self.io.now_ms() as i64 + after) as u64);
            fs::File::options().write(true).open(&p).unwrap().set_modified(at).unwrap();
        }
        /// The note, and the fold's own turn ended with what it changed.
        fn note(&self, conv: &str, text: &str, after: i64) {
            self.note_only(conv, text, after);
            self.said(&self.opener(), "I wrote the note.\nAnd a line in the index.");
        }
        /// The newest fold's opener.
        fn opener(&self) -> String {
            self.acts().into_iter().rfind(|a| a.starts_with("Read ")).expect("a fold was launched")
        }
        fn said(&self, opener: &str, reply: &str) {
            *self.agent.lines.lock().unwrap() = turn(opener, "2027-01-15T08:00:05Z", reply);
        }
        fn up(&self, conv: &str) -> bool {
            self.io.up.lock().unwrap().contains(&chats::session_name(conv))
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

    const ON: Seen = Seen { live: Some(true), busy: false, waiting: false, starting: false, new: false };

    fn run() -> Run {
        Run::new("w", "", None, Said::Folding)
    }

    /// Idle is asked about, seen twice running; a Claude at work, on a prompt or
    /// not started is not.
    #[test]
    fn its_own_turn_is_asked_only_when_it_has_been_idle_one_tick() {
        let mut r = run();
        let new = Seen { new: true, ..ON };
        assert_eq!(step(&mut r, Seen { busy: true, ..new }, secs(0)), Next::Wait, "still writing");
        assert_eq!(step(&mut r, new, secs(3)), Next::Wait, "seen idle once");
        assert_eq!(step(&mut r, Seen { busy: true, ..new }, secs(6)), Next::Wait, "at work again");
        assert_eq!(step(&mut r, new, secs(9)), Next::Wait);
        assert_eq!(step(&mut r, new, secs(12)), Next::Ask);
        let mut r = run();
        for t in 0..5 {
            assert_eq!(step(&mut r, Seen { starting: true, ..new }, secs(t)), Next::Wait, "a Claude that has not started");
        }
    }

    #[test]
    fn the_five_minutes_stand_still_while_it_waits_on_a_prompt() {
        let mut r = run();
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
    }

    /// The note begins the five minutes again, once; they end too, and stand
    /// still on a prompt too.
    #[test]
    fn a_note_with_no_end_has_five_minutes_of_its_own() {
        let mut r = run();
        let (work, wait) = (Seen { busy: true, new: true, ..ON }, Seen { waiting: true, new: true, ..ON });
        assert_eq!(step(&mut r, Seen { new: false, ..work }, secs(0)), Next::Wait);
        assert_eq!(step(&mut r, work, secs(290)), Next::Wait, "its note at 4:50");
        assert_eq!(step(&mut r, work, secs(580)), Next::Wait, "4:50 after it");
        assert_eq!(step(&mut r, wait, secs(583)), Next::Wait);
        assert_eq!(step(&mut r, wait, secs(9000)), Next::Wait);
        assert_eq!(step(&mut r, work, secs(9001)), Next::Wait);
        assert_eq!(step(&mut r, work, secs(9010)), Next::Wait, "4:59");
        assert_eq!(step(&mut r, work, secs(9011)), Next::TimedOut);
    }

    /// Gone is what tmux SAID, twice running, note or no note. One that does not
    /// answer is unknown: it counts toward the five minutes and nothing else.
    #[test]
    fn a_tmux_that_does_not_answer_is_not_a_claude_that_is_gone() {
        let (work, gone, unknown) = (Seen { busy: true, ..ON }, Seen { live: Some(false), starting: true, ..Seen::default() }, Seen { live: None, starting: true, ..Seen::default() });
        let mut r = run();
        assert_eq!(step(&mut r, gone, secs(0)), Next::Wait);
        assert_eq!(step(&mut r, work, secs(3)), Next::Wait, "one missed reading is not that");
        assert_eq!((step(&mut r, gone, secs(6)), step(&mut r, gone, secs(9))), (Next::Wait, Next::Gone));
        let mut r = run();
        assert_eq!(step(&mut r, Seen { new: true, ..gone }, secs(0)), Next::Wait);
        assert_eq!(step(&mut r, Seen { new: true, ..gone }, secs(3)), Next::Gone, "a note is not an end");
        let mut r = run();
        for t in 0..60 {
            assert_eq!(step(&mut r, unknown, secs(t * 5)), Next::Wait, "{t}");
        }
        assert_eq!(step(&mut r, unknown, secs(300)), Next::TimedOut);
        let mut r = run();
        assert_eq!(step(&mut r, gone, secs(0)), Next::Wait);
        assert_eq!(step(&mut r, unknown, secs(3)), Next::Wait);
        assert_eq!(step(&mut r, gone, secs(6)), Next::Wait, "gone twice RUNNING");
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
        assert_eq!(d.acts(), vec!["close".to_string(), format!("Read {} and do what it says. This is fold {}.", fold_md.display(), d.rec(&conv).fold.unwrap().attempt.unwrap())]);
        assert!(fs::read_to_string(&fold_md).unwrap().contains(&d.base.join("ws").join(&d.ws).join(d.rec(&conv).note).display().to_string()));
        assert_eq!(d.rec(&conv).fold.unwrap().asked_ms, Some(d.io.now_ms()));
        assert_eq!(d.rec(&conv).fold.unwrap().before, None, "it had no note");
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
        assert!(!d.held() && now.fold.is_empty() && !d.up(&conv));
        assert!(d.tick(3).0.is_empty(), "once");
    }

    /// Whose words each turn is, as `workspace_turns` answers it.
    fn whose(d: &Desk, conv: &str) -> Vec<(String, Option<String>)> {
        let v = crate::workspacechats::workspace_turns(&d.ctx, d.ws.clone(), conv.into(), Some(20)).unwrap();
        v["turns"].as_array().unwrap().iter().map(|t| (t["user"].as_str().unwrap_or("-").to_string(), t["engine"].as_str().map(str::to_string))).collect()
    }

    /// The turns the engine sent are marked, each attempt's of every fold, and
    /// no one else's: not the same words sent again, another chat's opener,
    /// words that look like one, or one a message only quotes.
    #[test]
    fn a_turn_the_engine_sent_says_so_and_no_other_does() {
        let d = Desk::new("engine");
        let other = d.chat("Other", false);
        let conv = d.chat("Chat 2", true);
        d.fold(&conv).unwrap();
        let first = d.opener();
        // Stopped: its attempt is not the record's fold any more.
        crate::workspacechats::workspace_chat_close(&d.ctx, d.ws.clone(), conv.clone()).unwrap();
        assert_eq!(d.rec(&conv).fold.unwrap().attempt, None);
        d.fold(&conv).unwrap();
        let second = d.opener();
        assert_ne!(first, second);
        let start = chats::opener(&conv);
        let unknown = chats::fold_opener(&wsfolder::folder_at(d.ctx.workspaces_dir(), &d.ws).unwrap(), &conv, &ulid::new());
        let said = [
            (start.clone(), Some("start")), ("Hello".into(), None), (start.clone(), None), (first.clone(), Some("fold")), (first.clone(), None),
            (format!("It said: {second}"), None), (second.clone(), Some("fold")), (unknown, None), (chats::opener(&other), None),
            ("Read the fold.md and do what it says.".into(), None),
        ];
        let lines: Vec<String> = said.iter().flat_map(|(u, _)| turn(u, "2027-01-15T08:00:05Z", "Right.")).collect();
        *d.agent.lines.lock().unwrap() = lines.clone();
        let want: Vec<(String, Option<String>)> = said.iter().map(|(u, e)| (u.clone(), e.map(str::to_string))).collect();
        assert_eq!(whose(&d, &conv), want);
        // By the whole transcript, however few are asked for.
        let v = crate::workspacechats::workspace_turns(&d.ctx, d.ws.clone(), conv.clone(), Some(6)).unwrap();
        assert!(v["turns"][0].get("engine").is_none(), "the first opener's second saying: {}", v["turns"][0]);
        // What is read starts later: no first turn of it is the start.
        *d.agent.lines.lock().unwrap() = [lines[1..2].to_vec(), lines[4..].to_vec()].concat();
        assert_eq!(whose(&d, &conv)[1], (start, None));
        // A chat never folded has its start only.
        *d.agent.lines.lock().unwrap() = [turn(&chats::opener(&other), "2027-01-15T08:00:05Z", "Hi."), turn(&second, "2027-01-15T08:00:06Z", "?")].concat();
        assert_eq!(whose(&d, &other).into_iter().map(|w| w.1).collect::<Vec<_>>(), vec![Some("start".to_string()), None]);
    }

    /// A turn read whole says what its short one did.
    #[test]
    fn a_whole_turn_says_the_engine_sent_it() {
        let d = Desk::new("whole");
        let conv = d.chat("Chat 1", true);
        *d.agent.lines.lock().unwrap() = [turn(&chats::opener(&conv), "2027-01-15T08:00:05Z", "Right."), turn("Hello", "2027-01-15T08:00:06Z", "Right.")].concat();
        let short = crate::workspacechats::workspace_turns(&d.ctx, d.ws.clone(), conv.clone(), None).unwrap();
        let whole = |i: usize| crate::workspacechats::workspace_turn_text(&d.ctx, d.ws.clone(), conv.clone(), short["turns"][i]["id"].as_str().unwrap().into()).unwrap()["turn"].clone();
        assert_eq!((whole(0)["engine"].clone(), whole(1).get("engine").cloned()), (json!("start"), None));
    }

    /// A chat that was never sent a message is folded as `Chat N`, whatever a
    /// client called it: the note, the commit and the record.
    #[test]
    fn a_fold_never_takes_its_opener_for_a_name() {
        let d = Desk::new("unsent");
        d.chat("Earlier", false);
        let conv = d.chat("Chat 2", true);
        let stem = |x: &str| chats::fold_opener(&wsfolder::folder_at(d.ctx.workspaces_dir(), &d.ws).unwrap(), x, "");
        // The row refuses it, from an ask to the landing.
        let name = |x: &str, t: &str| crate::workspacechats::workspace_chat_rename(&d.ctx, d.ws.clone(), x.into(), t.into());
        let cut = format!("{}…", stem(&conv).chars().take(40).collect::<String>());
        assert!(name(&conv, &cut).is_err());
        // A record that has it already.
        let spoil = |x: &str| chats::update_at(d.ctx.workspaces_dir(), &d.ws, x, |c| c.title = format!("{}…", stem(x).chars().take(40).collect::<String>())).unwrap();
        spoil(&conv);
        d.fold(&conv).unwrap();
        assert_eq!(d.rec(&conv).note, format!("memory/notes/2026-11-24-chat-2-{}.md", conv[20..].to_lowercase()));
        assert_eq!(d.rec(&conv).title, "Chat 2");
        assert!(name(&conv, &d.opener()).is_err());
        // Named after the opener while it folds.
        spoil(&conv);
        d.note(&conv, "# Chat\n", 5);
        d.tick(3);
        assert_eq!(d.tick(3).0[0].event, "landed");
        assert_eq!((d.log()[0].subject.as_str(), d.rec(&conv).title.as_str()), ("fold: Chat 2", "Chat 2"));
        // Stopped, and closed without one.
        for (fold, subject) in [(true, "Fold stopped: Chat 3"), (false, "Chat closed: Chat 4")] {
            let x = d.chat("x", true);
            if fold {
                d.fold(&x).unwrap();
            }
            spoil(&x);
            d.put("topics/half.md", subject);
            crate::workspacechats::workspace_chat_close(&d.ctx, d.ws.clone(), x).unwrap();
            assert_eq!(d.log()[0].subject, subject);
        }
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
        // Renamed since: the note is the same file.
        chats::rename_at(d.ctx.workspaces_dir(), &d.ws, &conv, "Another name").unwrap();
        // Ended: there is no session to look at and none to attach to again.
        let r = d.fold(&conv).unwrap();
        assert_eq!((&r["reattach"], d.rec(&conv).fold.unwrap().commit), (&json!(false), None));
        for _ in 0..10 {
            assert!(d.tick(3).0.is_empty(), "the first fold's note landed the second");
        }
        assert_eq!(d.says(&conv).as_deref(), Some("folding"));
        d.note(&conv, "second\n", 5);
        d.tick(3);
        d.tick(3);
        assert_eq!(d.log()[0].subject, "fold: Another name");
        assert_eq!(d.log().iter().filter(|c| c.subject.starts_with("fold: ")).count(), 2);
        assert_eq!(fs::read_dir(d.mem("notes")).unwrap().count(), 1, "never doubled");
        assert_eq!(d.rec(&conv).note, format!("memory/notes/2026-11-24-naming-{}.md", conv[20..].to_lowercase()));
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
        let config = fs::read(memory::git_dir_at(d.ctx.workspaces_dir(), &d.ws).unwrap().join("config")).unwrap();
        fs::write(memory::git_dir_at(d.ctx.workspaces_dir(), &d.ws).unwrap().join("config"), "[").unwrap();
        assert!(d.fold(&a).unwrap_err().contains("nothing was folded"));
        unchanged("a memory that cannot be committed");
        fs::write(memory::git_dir_at(d.ctx.workspaces_dir(), &d.ws).unwrap().join("config"), config).unwrap();
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

    /// Nothing to keep is what Claude SAID, at the end of this fold's own turn:
    /// an earlier attempt's answer, a question, or words in the middle are not.
    #[test]
    fn nothing_to_keep_is_said_at_the_end_of_this_attempts_turn() {
        let d = Desk::new("nothing");
        let conv = d.chat("Chat 1", true);
        d.fold(&conv).unwrap();
        let opener = d.opener();
        let earlier = opener.replace(d.rec(&conv).fold.unwrap().attempt.as_deref().unwrap(), &ulid::new());
        for (u, r) in [
            (earlier.as_str(), "No memory changes."),
            ("something else", "No memory changes."),
            (opener.as_str(), "Should I keep the part about disks?"),
            (opener.as_str(), "No memory changes. But look at this."),
            (opener.as_str(), "I would not say: No memory changes."),
            (opener.as_str(), "You asked me not to say\nNo memory changes.\nso I will look again."),
            (opener.as_str(), "API Error: 529 overloaded"),
        ] {
            d.said(u, r);
            d.tick(3);
            assert!(d.tick(3).0.is_empty(), "{u} {r}");
        }
        d.said(&opener, "I looked.\n\n`No memory changes.`");
        let (n, _) = d.tick(3);
        assert_eq!((n[0].event, n[0].commit.clone()), ("nothing", None));
        let f = d.rec(&conv).fold.unwrap();
        assert_eq!((f.state.as_str(), f.commit, f.files, f.left, f.closing), ("done", None, Some(0), Some(0), false));
        assert_eq!(d.log().len(), 1, "no commit");
        assert!(!d.held() && !d.up(&conv));
        // Again at once, in the same second: that answer is not this one's.
        d.fold(&conv).unwrap();
        for _ in 0..4 {
            assert!(d.tick(3).0.is_empty(), "the first attempt's answer ended the second");
        }
    }

    /// A note that is there is not an end: one from the future that nobody
    /// touched, one written by a turn that was stopped or is still going.
    #[test]
    fn a_note_lands_only_when_it_was_written_and_the_turn_said_its_last_words() {
        let d = Desk::new("witness");
        let a = d.chat("A", true);
        d.fold(&a).unwrap();
        d.note(&a, "first\n", 3_600_000);
        d.tick(3);
        assert_eq!(d.tick(3).0[0].event, "landed");
        d.fold(&a).unwrap();
        d.said(&d.opener(), "I changed two files.");
        for _ in 0..4 {
            assert!(d.tick(3).0.is_empty(), "an untouched note from the future landed");
        }
        // Written now, and stopped before the index.
        d.note_only(&a, "half\n", 10);
        let mut lines = turn(&d.opener(), "2027-01-15T08:00:05Z", "The note is written. Now the index.");
        lines.push(turn("[Request interrupted by user]", "2027-01-15T08:00:09Z", "").remove(0));
        *d.agent.lines.lock().unwrap() = lines;
        for _ in 0..4 {
            assert!(d.tick(3).0.is_empty(), "a stopped turn landed");
        }
        // Between two calls: its last step is a tool's.
        let mut lines = turn(&d.opener(), "2027-01-15T08:00:05Z", "The note is written.");
        lines.push(
            json!({ "type": "assistant", "uuid": ulid::new(), "timestamp": "2027-01-15T08:00:06Z", "message": { "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Edit", "input": { "file_path": "/x" } }] } }).to_string(),
        );
        *d.agent.lines.lock().unwrap() = lines;
        assert!(d.tick(3).0.is_empty(), "a turn that is still going landed");
        // Ended by an error, not by its last words.
        d.said(&d.opener(), "API Error: 529 overloaded_error");
        assert!(d.tick(3).0.is_empty(), "a turn an error ended landed");
        // No transcript to read is no witness.
        d.agent.lines.lock().unwrap().clear();
        assert!(d.tick(3).0.is_empty());
        d.said(&d.opener(), "Two files changed.");
        assert_eq!(d.tick(3).0[0].event, "landed");
        // Never said: five minutes after the note.
        d.fold(&a).unwrap();
        d.note_only(&a, "again\n", 20);
        d.tick(3);
        let (n, now) = d.tick(LIMIT.as_secs());
        assert_eq!(n[0].event, "failed");
        assert!(now.fold[&chats::session_name(&a)].why.as_ref().unwrap().starts_with("it wrote its note and did not finish in five minutes"));
    }

    /// An engine that dies at any step of a landing leaves what the next one
    /// finishes: one commit, its own receipt, and the session ended.
    #[test]
    fn a_landing_survives_the_engine_dying_at_every_step() {
        for at in ["before the commit", "after the commit", "after the receipt", "after the close"] {
            let d = Desk::new("crash");
            let conv = d.chat("Naming", true);
            d.fold(&conv).unwrap();
            d.note(&conv, "note\n", 5);
            d.tick(3);
            *d.io.fatal.lock().unwrap() = Some(at);
            d.tick(3);
            *d.io.fatal.lock().unwrap() = None;
            // Meanwhile someone edits: not the fold's.
            let committed = d.log()[0].subject == "fold: Naming";
            assert_eq!(committed, at != "before the commit", "{at}");
            if committed {
                d.put("topics/by-hand.md", "by hand\n");
            }
            let again = Desk::engine(&d.base, &d.io, &d.agent);
            let mut notes = Vec::new();
            for _ in 0..4 {
                notes.extend(d.tick_on(&again, 3).0);
            }
            let f = d.rec(&conv).fold.unwrap();
            let folds: Vec<Change> = d.log().into_iter().filter(|c| c.subject.starts_with("fold:")).collect();
            assert_eq!(folds.len(), 1, "{at}: one commit");
            assert_eq!((f.state.as_str(), f.commit.as_deref(), f.files, f.closing), ("done", Some(folds[0].commit.as_str()), Some(1), false), "{at}: its own receipt");
            assert!(!folds[0].files.iter().any(|f| f.path.contains("by-hand")), "{at}");
            assert!(!d.up(&conv) && !again.memory.is_held(&d.ws) && again.folds.of(&conv).is_none(), "{at}: ended and let go");
        }
    }

    /// A receipt that cannot be written ends nothing and lets nothing go; the
    /// next tick writes it, for the commit that is there.
    #[test]
    fn a_receipt_that_cannot_be_written_is_tried_again() {
        use std::os::unix::fs::PermissionsExt;
        let d = Desk::new("receipt");
        let conv = d.chat("Naming", true);
        d.fold(&conv).unwrap();
        d.note(&conv, "note\n", 5);
        d.tick(3);
        let dir = wsfolder::engine_dir_at(d.ctx.workspaces_dir(), &d.ws).unwrap();
        let mode = |m: u32| fs::set_permissions(&dir, fs::Permissions::from_mode(m)).unwrap();
        mode(0o555);
        let through = fs::write(dir.join("probe"), "").is_ok();
        let (n, _) = d.tick(3);
        mode(0o700);
        if through {
            return; // root writes through the mode
        }
        assert!(n.is_empty(), "landed with no receipt");
        assert!(d.up(&conv) && d.held(), "closed or let go with no receipt");
        assert_eq!(d.says(&conv).as_deref(), Some("folding"), "still to be recorded");
        assert_eq!(d.tick(3).0[0].event, "landed");
        assert_eq!(d.rec(&conv).fold.unwrap().commit, Some(d.log()[0].commit.clone()));
        assert_eq!(d.log().iter().filter(|c| c.subject.starts_with("fold:")).count(), 1);
    }

    /// Two engines on one folder: the record is the reservation, so one fold at
    /// a time, and one that has just started refuses an Undo before any poll.
    #[test]
    fn another_engine_sees_the_fold_before_its_first_poll() {
        let d = Desk::new("engines");
        let (a, b) = (d.chat("A", true), d.chat("B", true));
        d.put("topics/x.md", "x\n");
        let edit = crate::memory::memory_commit_if_dirty(&d.ctx, &d.ws, None).unwrap().unwrap();
        let other = Desk::engine(&d.base, &d.io, &d.agent);
        ensure(&other);
        d.fold(&a).unwrap();
        assert_eq!(workspace_chat_fold(&other, d.ws.clone(), b.clone()).unwrap_err(), "“A” is folding. Close this one after.");
        assert!(d.rec(&b).fold.is_none());
        assert!(crate::memory::workspace_memory_revert(&other, d.ws.clone(), edit.commit.clone()).is_err(), "undone under another engine's fold");
        let fresh = Desk::engine(&d.base, &d.io, &d.agent);
        assert!(crate::memory::workspace_memory_revert(&fresh, d.ws.clone(), edit.commit).is_err(), "undone before its first poll");
        // B closed there leaves A's dirty tree alone.
        d.put("topics/half.md", "half of A's fold\n");
        let before = d.log().len();
        crate::workspacechats::workspace_chat_close(&other, d.ws.clone(), b).unwrap();
        assert_eq!(d.log().len(), before);
        // Both polls look: it lands once.
        d.note(&a, "note\n", 5);
        let mut n = Vec::new();
        for _ in 0..3 {
            n.extend(d.tick(3).0);
            n.extend(d.tick_on(&other, 0).0);
        }
        assert_eq!(n.iter().filter(|n| n.event == "landed").count(), 1, "{n:?}");
        assert_eq!(d.log().iter().filter(|c| c.subject.starts_with("fold:")).count(), 1);
    }

    /// The reservation is asked for and made under one lock: another engine's,
    /// made between this one's first look and its own, is seen.
    #[test]
    fn a_reservation_made_meanwhile_is_waited_for_and_seen() {
        use std::sync::mpsc;
        let d = Desk::new("race");
        let (a, b) = (d.chat("A", true), d.chat("B", true));
        let root = d.ctx.workspaces_dir();
        let (admitted, at_admit) = mpsc::channel();
        let (go, gone) = mpsc::channel::<()>();
        *d.io.admitted.lock().unwrap() = Some(Box::new(move || {
            admitted.send(()).unwrap();
            let _ = gone.recv();
        }));
        std::thread::scope(|s| {
            let (done, said) = mpsc::channel();
            let (ctx, ws, chat) = (&d.ctx, d.ws.clone(), b.clone());
            s.spawn(move || done.send(workspace_chat_fold(ctx, ws, chat).map(|_| ())));
            at_admit.recv().unwrap();
            let held = wslock::take(root, &d.ws).unwrap();
            go.send(()).unwrap();
            // It waits for the lock: without it this answers at once.
            assert!(said.recv_timeout(Duration::from_millis(400)).is_err(), "it did not wait for the lock");
            chats::update_at(root, &d.ws, &a, |c| {
                c.fold = Some(Fold { state: chats::FOLDING.into(), attempt: Some(ulid::new()), ..Fold::default() })
            })
            .unwrap();
            drop(held);
            assert_eq!(said.recv().unwrap().unwrap_err(), "“A” is folding. Close this one after.");
        });
        assert!(d.rec(&b).fold.is_none() && d.acts().is_empty(), "nothing ended, nothing launched");
    }

    /// A landing looks again under the lock: a fold another engine has finished,
    /// or asked again, since this one's tick read it is not landed here.
    #[test]
    fn a_landing_asks_again_under_the_lock() {
        let d = Desk::new("again");
        let a = d.chat("A", true);
        d.fold(&a).unwrap();
        d.note(&a, "note\n", 5);
        let other = ulid::new();
        let before = d.log().len();
        assert!(land(&d.ctx, &d.ws, &a, &other, true).is_none(), "another attempt's landing");
        let mine = d.rec(&a).fold.unwrap().attempt.unwrap();
        chats::update_at(d.ctx.workspaces_dir(), &d.ws, &a, |c| c.fold.as_mut().unwrap().state = chats::FOLDED.into()).unwrap();
        assert!(land(&d.ctx, &d.ws, &a, &mine, true).is_none(), "landed twice");
        assert_eq!(d.log().len(), before);
        assert!(d.up(&a), "and nothing is ended");
    }

    /// A record that cannot be read says nothing: at a start its workspace is
    /// held and asked again, and under a fold nothing is ended.
    #[test]
    fn an_unreadable_record_ends_no_fold_and_frees_no_memory() {
        let d = Desk::new("unreadable");
        let a = d.chat("A", true);
        d.fold(&a).unwrap();
        let p = wsfolder::engine_dir_at(d.ctx.workspaces_dir(), &d.ws).unwrap().join("chats.json");
        let good = fs::read(&p).unwrap();
        fs::write(&p, "{").unwrap();
        d.tick(3);
        assert!(d.held() && d.says(&a).is_some(), "a failed read ended the fold");
        let again = Desk::engine(&d.base, &d.io, &d.agent);
        ensure(&again);
        assert!(again.memory.is_held(&d.ws), "unknown is not free");
        assert!(workspace_chat_fold(&again, d.ws.clone(), a.clone()).is_err());
        fs::write(&p, &good).unwrap();
        ensure(&again);
        assert_eq!(again.folds.of(&a).unwrap().state, "checking", "asked again, and taken up");
        d.note(&a, "note\n", 5);
        let mut n = Vec::new();
        for _ in 0..3 {
            n.extend(d.tick_on(&again, 3).0);
        }
        assert_eq!(n[0].event, "landed");
        // One that had no fold lets go.
        fs::write(&p, "{").unwrap();
        let third = Desk::engine(&d.base, &d.io, &d.agent);
        ensure(&third);
        assert!(third.memory.is_held(&d.ws));
        chats_put(&p, &good);
        chats::update_at(d.ctx.workspaces_dir(), &d.ws, &a, |c| c.fold = None).unwrap();
        ensure(&third);
        assert!(!third.memory.is_held(&d.ws));
    }

    fn chats_put(p: &Path, text: &[u8]) {
        fs::write(p, text).unwrap();
    }

    /// A removal that fails half way leaves neither half alone: no hold with no
    /// fold, and no fold that nobody looks at.
    #[test]
    fn a_failed_removal_leaves_no_hold_without_a_fold() {
        let d = Desk::new("removal");
        let a = d.chat("A", true);
        d.fold(&a).unwrap();
        let p = wsfolder::engine_dir_at(d.ctx.workspaces_dir(), &d.ws).unwrap().join("chats.json");
        let good = fs::read(&p).unwrap();
        fs::write(&p, "{").unwrap();
        fs::write(d.base.join("ws/.removed"), "a file where the folder goes").unwrap();
        assert!(crate::workspaces::workspace_remove(&d.ctx, d.ws.clone()).is_err());
        assert!(!d.held() && d.says(&a).is_none());
        fs::write(&p, good).unwrap();
        d.io.up.lock().unwrap().clear();
        let mut n = Vec::new();
        for _ in 0..4 {
            n.extend(d.tick(3).0);
        }
        assert_eq!(n[0].event, "failed", "taken up again, and over");
        assert!(!d.held());
    }

    /// The last session of all ended takes the server with it: a chat that has
    /// ended is folded all the same, and one that lands is closed. A fold that
    /// RUNS is not taken for gone by it.
    #[test]
    fn no_server_is_the_end_of_a_chat_and_not_of_a_fold() {
        let d = Desk::new("noserver");
        d.io.no_server.store(true, Ordering::SeqCst);
        let a = d.chat("A", false);
        d.fold(&a).unwrap();
        d.note(&a, "note\n", 5);
        d.tick(3);
        assert_eq!(d.tick(3).0[0].event, "landed");
        assert!(!d.rec(&a).fold.unwrap().closing && d.ctx.folds.of(&a).is_none());
        d.fold(&a).unwrap();
        d.io.up.lock().unwrap().clear();
        for _ in 0..20 {
            assert!(d.tick(3).0.is_empty(), "taken for gone");
        }
        d.io.secs.fetch_add(LIMIT.as_secs(), Ordering::SeqCst);
        assert_eq!(d.tick(0).0[0].event, "failed");
    }

    /// tmux silent: nothing is started, and nothing is taken for gone.
    #[test]
    fn a_fold_is_not_asked_of_a_tmux_that_does_not_answer() {
        let d = Desk::new("dumb");
        let a = d.chat("A", true);
        d.io.dumb.store(true, Ordering::SeqCst);
        assert_eq!(d.fold(&a).unwrap_err(), ERR_UNSEEN);
        assert!(d.rec(&a).fold.is_none() && d.acts().is_empty());
        d.io.dumb.store(false, Ordering::SeqCst);
        d.fold(&a).unwrap();
        d.note(&a, "note\n", 5);
        d.io.dumb.store(true, Ordering::SeqCst);
        let silent = |d: &Desk| {
            let mut now = WorkspaceSessions::default();
            d.io.secs.fetch_add(3, Ordering::SeqCst);
            tick(&d.ctx, &mut now)
        };
        for _ in 0..5 {
            assert!(silent(&d).is_empty());
        }
        assert!(d.held() && d.up(&a));
        d.io.secs.fetch_add(LIMIT.as_secs(), Ordering::SeqCst);
        assert_eq!(silent(&d)[0].event, "failed");
        assert!(d.rec(&a).fold.unwrap().why.unwrap().starts_with("tmux did not answer"));
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
        assert!(d.up(&conv), "the session is left open");
        assert!(!d.held());
        assert_eq!((d.rec(&conv).fold.unwrap().state.as_str(), d.tick(3).0.len()), ("failed", 0), "said once");
        let (_, e) = run(None);
        assert_eq!(e.left, Some(0));
        assert!(e.why.unwrap().ends_with("Memory is unchanged."));
        // Memory that cannot be read: no number, and not "unchanged".
        d.fold(&conv).unwrap();
        d.sees(activity::State::Busy);
        fs::write(memory::git_dir_at(d.ctx.workspaces_dir(), &d.ws).unwrap().join("config"), "[").unwrap();
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
        fs::write(memory::git_dir_at(d.ctx.workspaces_dir(), &d.ws).unwrap().join("config"), "[").unwrap();
        d.tick(3);
        let (n, _) = d.tick(3);
        assert_eq!(n[0].event, "failed");
        assert!(d.rec(&conv).fold.unwrap().why.unwrap().contains("could not be committed"));
        assert!(d.up(&conv) && !d.held());
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
        let c = chats::find_at(d.ctx.workspaces_dir(), &other, &c.id).unwrap();
        let p = d.base.join("ws").join(&other).join(&c.note);
        fs::write(&p, "note\n").unwrap();
        // Both turns, in the one transcript the script has.
        let openers: Vec<String> = d.acts().into_iter().filter(|a| a.starts_with("Read ")).collect();
        *d.agent.lines.lock().unwrap() = openers.iter().flat_map(|o| turn(o, "2027-01-15T08:00:05Z", "Two files changed.")).collect();
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
