//! The workspace-chat rows (`docs/proposals/workspace-chats.md` §3, §5):
//! chats with Claude that belong to a workspace and to no project, and the
//! chat rows that drive them.
//!
//! Every row is addressed by the workspace's id and, past the list and the
//! start, a chat id from that workspace's `chats.json`
//! (`worktrees_core::workspacechats`). Serve declares them `HostConsole` on
//! the objects `workspace(id)` and `workspace_chat(conv, id)`: no device
//! grant reaches them.
//!
//! **The chat rows reuse the agent rows' bodies** (`agents::*_at`) on a
//! [`Locus::Workspace`], as the idea rows do on theirs: the same screen
//! reads, proofs and paste, the same `typed_line_ok`, empty-composer and
//! no-prompt checks, one lock per chat. `workspace_send` and
//! `workspace_answer_text` are `idea_send`'s and `idea_answer_text`'s bodies
//! behind one more console-only door (the owner, 2026-11-24). No slash
//! commands and no attachments.
//!
//! **The life of one workspace's chats is serial** ([`life_lock`]): a start,
//! a resume, a close, a delete and the workspace's removal hold one lock, so
//! a chat is never launched into a folder that is being moved away.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{json, Value};
use worktrees_core::workspacechats::{self as chats, Chat, Context, DocRead, ProjectContext, ProjectRead, Reads, Start};
use worktrees_core::workspaces::{self as store, Workspace};
use worktrees_core::{activity, agent, desksessions, registry, tmux, turns, wsfolder};

use crate::agents::{self, Locus};
use crate::watch::WorkspaceSessions;
use crate::Ctx;

/// The lock the life of workspace `id`'s chats holds.
pub fn life_lock(ctx: &Ctx, id: &str) -> Arc<Mutex<()>> {
    ctx.agents.place_lock(&format!("ws-life:{id}"))
}

pub(crate) fn workspace(ctx: &Ctx, id: &str) -> Result<Workspace, String> {
    store::read_at(ctx.workspaces_file())?.workspaces.into_iter().find(|w| w.id == id).ok_or_else(|| format!("no workspace {id}"))
}

pub(crate) fn locus_of(ctx: &Ctx, id: &str, c: &Chat) -> Result<Locus, String> {
    Ok(Locus::Workspace { session: chats::session_name(&c.id), dir: wsfolder::folder_at(ctx.workspaces_dir(), id)?, session_id: c.session_id.clone() })
}

/// The chat `conv` of workspace `id`, as a locus the agent bodies run on.
fn locus(ctx: &Ctx, id: &str, conv: &str) -> Result<Locus, String> {
    locus_of(ctx, id, &chats::find_at(ctx.workspaces_dir(), id, conv)?)
}

// ── the list ────────────────────────────────────────────────────────────────

/// One chat as the list shows it.
#[derive(Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub chat: Chat,
    /// Its tmux session.
    pub session: String,
    /// `running` (its session is up), `busy`, `waiting` (on the person) or
    /// `ended` (no session: closed, or its Claude exited).
    pub state: &'static str,
    /// Its note is on disk, at `note`.
    pub note_exists: bool,
    /// How its fold stands now, when there is something to say: the event's
    /// entry for it (`WorkspaceSessions::fold`). `fold` is the record's.
    pub fold_now: Option<crate::watch::FoldOf>,
    /// Whether a LIVE chat still has what a launch now would give it:
    /// `current`, `stale` (it keeps its launch's rules until it is resumed; it
    /// is never ended for this), or `unknown` (launched by an earlier build,
    /// or the list of projects cannot be read). Null for an ended one.
    pub access: Option<&'static str>,
    /// What would be different, with `stale`.
    pub access_changes: Vec<chats::AccessChange>,
}

/// `workspace_chats`' answer.
#[derive(Serialize)]
pub struct Chats {
    pub id: String,
    pub name: String,
    /// The workspace's own folder, which may not exist yet.
    pub folder: String,
    /// Newest first.
    pub chats: Vec<Listed>,
    /// What a chat of this workspace reads, project by project, and why not:
    /// the same list its `context.md` is written from.
    pub projects: Vec<ProjectRead>,
    /// The workspace's plan and linked ideas, with where each is and whether a
    /// chat reads it: also what `context.md` is written from.
    pub documents: Vec<DocRead>,
    /// What the rules stop and what they do not, to show beside these lists.
    pub limits: &'static str,
}

fn state_of(live: bool, a: &activity::Activity) -> &'static str {
    match (live, a.state) {
        (false, _) => "ended",
        (true, activity::State::Busy) => "busy",
        (true, activity::State::Waiting) => "waiting",
        (true, _) => "running",
    }
}

/// Whether chat `c` is up, by the sessions `live` and its record. A fold
/// that landed writes its receipt and then ends the session
/// (`workspacefolds::land`): until the end is done its row is closed, as the
/// receipt says, and the app draws it so (`wschats.ts` `rowOf`).
fn up_in(live: &std::collections::HashSet<String>, c: &Chat) -> bool {
    live.contains(&chats::session_name(&c.id)) && !c.fold.as_ref().is_some_and(|f| f.closing)
}

fn listed(ctx: &Ctx, id: &str, mut c: Chat, live: &std::collections::HashSet<String>, given: Option<&chats::Access>) -> Result<Listed, String> {
    let session = chats::session_name(&c.id);
    let up = up_in(live, &c);
    let a = if up { ctx.agents.io().activity(&locus_of(ctx, id, &c)?) } else { activity::Activity::none() };
    // The snapshot is the record's; a client is told what it comes to.
    let was = c.access.take();
    let changes = match (up, &was, given) {
        (true, Some(then), Some(now)) => Some(chats::access_changes(then, now)),
        _ => None,
    };
    let access = match (up, &changes) {
        (false, _) => None,
        (true, None) => Some("unknown"),
        (true, Some(c)) if c.is_empty() => Some("current"),
        (true, Some(_)) => Some("stale"),
    };
    Ok(Listed { access, access_changes: changes.unwrap_or_default(), state: state_of(up, &a), session, note_exists: chats::note_exists(ctx.workspaces_dir(), id, &c), fold_now: ctx.folds.of(&c.id), chat: c })
}

/// `workspace_chats`: the workspace's chats, newest first, each with its
/// state, from `chats.json` and tmux — no git call — and what they read.
pub fn workspace_chats(ctx: &Ctx, id: String) -> Result<Chats, String> {
    crate::workspacefolds::ensure(ctx);
    let w = workspace(ctx, &id)?;
    let root = ctx.workspaces_dir();
    let reg = chats::policy(&w);
    let none = registry::Registry::default();
    let src = chats::Sources { root, ideas: ctx.ideas_dir(), reg: reg.as_ref().unwrap_or(&none) };
    // What a launch would get now: what each live chat is compared with.
    let given = reg.as_ref().ok().and_then(|_| chats::access(&w, &src).ok());
    // The record first, then tmux. A landing writes its receipt and then ends
    // the session; read the other way round, a landing between the two reads
    // answered `done` beside a session listed before it ended.
    let record = chats::list_at(root, &id)?;
    let live: std::collections::HashSet<String> = tmux::session_names().into_iter().collect();
    Ok(Chats {
        chats: record.into_iter().map(|c| listed(ctx, &id, c, &live, given.as_ref())).collect::<Result<_, _>>()?,
        projects: if reg.is_ok() { chats::project_reads(&w, src.reg) } else { chats::project_reads_unknown(&w) },
        documents: chats::doc_reads(&w, &src),
        limits: chats::LIMITS,
        folder: wsfolder::folder_at(root, &id)?.to_string_lossy().into_owned(),
        name: w.name,
        id,
    })
}

// ── context.md ──────────────────────────────────────────────────────────────

/// The workspace as its chat `conv` is told it (`context.md`). A project that
/// is not read is named, and nothing more is said of it.
pub(crate) fn context(ctx: &Ctx, w: &Workspace, reg: &registry::Registry, conv: &str) -> Context {
    let panes = tmux::PaneList::fetch();
    let probes = agent::live_probes_in(panes.as_ref());
    let projects = chats::project_reads(w, reg)
        .into_iter()
        .map(|read| {
            let places = if read.reads == Reads::No { Ok(Vec::new()) } else { chats::places(&read.root, panes.as_ref(), &probes) };
            ProjectContext { read, places }
        })
        .collect();
    Context {
        workspace: w.name.clone(),
        written: desksessions::now_iso(),
        projects,
        docs: chats::doc_reads(w, &chats::Sources { root: ctx.workspaces_dir(), ideas: ctx.ideas_dir(), reg }),
        notes: chats::earlier_notes(ctx.workspaces_dir(), &w.id, Some(conv)),
    }
}

// ── the life of a chat ──────────────────────────────────────────────────────

/// `workspace_chat_start`: a new chat. Its folder, `start.md`, `context.md`,
/// the record (with the session id), then the launch. A refused launch (the
/// live-agent limit, a spent window, a Claude below the floor) leaves nothing
/// of the chat; the memory it made stays (a memory is never deleted here).
pub fn workspace_chat_start(ctx: &Ctx, id: String) -> Result<Value, String> {
    let lock = life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    let w = workspace(ctx, &id)?;
    // Before anything is made: a chat is not started without its rules.
    let reg = chats::policy(&w)?;
    crate::workspacefolds::ensure(ctx);
    crate::memory::before_launch(ctx, &w)?;
    let (c, session) = chats::start_at(ctx.workspaces_dir(), ctx.ideas_dir(), &w, |conv| chats::context_text(&context(ctx, &w, &reg, conv)))?;
    Ok(json!({ "id": id, "chat": c, "session": session }))
}

/// `workspace_chat_tidy`: Tidy…, a new chat on the engine's tidy file
/// (`chats::tidy_text`), launched on the same fixed opener as any chat. No
/// text of the client's reaches it: the row takes the workspace's id alone.
/// Refused, before anything is made, with no index, while a chat folds into
/// the memory (in this engine or by the record), and while another tidy chat
/// runs; then as any start is (the live-agent limit, a spent window), which
/// leaves nothing. What it writes is committed as any chat's work is (the
/// quiet minute), in History, where each commit can be undone.
pub fn workspace_chat_tidy(ctx: &Ctx, id: String) -> Result<Value, String> {
    let lock = life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    let w = workspace(ctx, &id)?;
    let reg = chats::policy(&w)?;
    crate::workspacefolds::ensure(ctx);
    tidy_may(ctx, &id, &|s| tmux::session_exists(s))?;
    crate::memory::before_launch(ctx, &w)?;
    let (c, session) = chats::start_kind_at(ctx.workspaces_dir(), ctx.ideas_dir(), &w, chats::Kind::Tidy, |conv| chats::context_text(&context(ctx, &w, &reg, conv)))?;
    Ok(json!({ "id": id, "chat": c, "session": session }))
}

/// Whether Tidy… may start now: the record's rule (`chats::may_tidy_at`) and
/// this engine's own folds and holds, which the record may not have yet.
fn tidy_may(ctx: &Ctx, id: &str, live: &dyn Fn(&str) -> bool) -> Result<(), String> {
    if ctx.memory.is_held(id) || chats::list_at(ctx.workspaces_dir(), id)?.iter().any(|c| ctx.folds.is_folding(&c.id)) {
        return Err(chats::ERR_TIDY_FOLDING.into());
    }
    chats::may_tidy_at(ctx.workspaces_dir(), id, live)
}

/// `workspace_chat_resume`: rewrite `context.md`, then launch an ended chat
/// again, `--resume '<its session id>'` under the same tmux name, with no
/// opener.
pub fn workspace_chat_resume(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    let lock = life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    let w = workspace(ctx, &id)?;
    let root = ctx.workspaces_dir();
    chats::find_at(root, &id, &conv)?;
    let reg = chats::policy(&w)?;
    crate::workspacefolds::ensure(ctx);
    if ctx.folds.is_folding(&conv) {
        return Err(crate::workspacefolds::ERR_ALREADY.into());
    }
    if tmux::session_exists(&chats::session_name(&conv)) {
        return Err(format!("chat {conv} is already running (tmux session {})", chats::session_name(&conv)));
    }
    crate::memory::before_launch(ctx, &w)?;
    chats::write_context_at(root, &id, &conv, &chats::context_text(&context(ctx, &w, &reg, &conv)))?;
    let session = chats::launch_at(root, ctx.ideas_dir(), &w, &conv, Start::Resume, None)?;
    touched(ctx, &id, &conv);
    Ok(json!({ "id": id, "conv": conv, "session": session }))
}

/// `workspace_chat_close`: end its tmux session. The record, its files and
/// Claude's transcript stay. This is "close without folding", and, while it
/// folds, Stop the fold: what is left in the memory is committed under its own
/// subject (`workspacefolds::closed`). `stopped_fold` and `commit` say so.
pub fn workspace_chat_close(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    let lock = life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    crate::workspacefolds::ensure(ctx);
    let c = chats::find_at(ctx.workspaces_dir(), &id, &conv)?;
    let was = ctx.folds.close(&conv)?;
    let (stopped, change) = crate::workspacefolds::closed(ctx, &id, &c, was)?;
    if was || stopped {
        touched(ctx, &id, &conv);
    }
    Ok(json!({ "id": id, "conv": conv, "closed": was, "stopped_fold": stopped, "commit": change.map(|c| c.commit) }))
}

/// `workspace_chat_remove`: close it if it runs (a fold is stopped, as
/// `workspace_chat_close` stops one), then forget it: its record
/// entry and its folder. Its note stays, in the memory; Claude's transcript
/// is Claude's file.
pub fn workspace_chat_remove(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    let lock = life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    crate::workspacefolds::ensure(ctx);
    let c = chats::find_at(ctx.workspaces_dir(), &id, &conv)?;
    let was = ctx.folds.close(&conv)?;
    let (stopped, change) = crate::workspacefolds::closed(ctx, &id, &c, was)?;
    chats::remove_at(ctx.workspaces_dir(), &id, &conv)?;
    Ok(json!({ "id": id, "conv": conv, "removed": true, "stopped_fold": stopped, "commit": change.map(|c| c.commit) }))
}

/// `workspace_chat_rename`: a new title, one line. Refused for what the engine
/// launched the chat on (`chats::is_opener`).
pub fn workspace_chat_rename(ctx: &Ctx, id: String, conv: String, title: String) -> Result<Value, String> {
    let c = chats::rename_at(ctx.workspaces_dir(), &id, &conv, &title)?;
    Ok(json!({ "id": id, "chat": c }))
}

/// End every live chat of workspace `id`: what removing it does first. By the
/// live sessions and their folders as well as the record, so a record that
/// does not read cannot leave one running. The caller holds [`life_lock`].
pub fn close_all(ctx: &Ctx, id: &str) -> Result<(), String> {
    let root = ctx.workspaces_dir();
    let of = chats::workspaces_of_convs(root);
    let mut convs: Vec<String> = tmux::session_names()
        .iter()
        .filter_map(|s| chats::conv_of_session(s).map(str::to_string))
        .filter(|c| of.get(c).is_some_and(|w| w == id))
        .collect();
    crate::workspacefolds::ensure(ctx);
    let recorded = chats::list_at(root, id).unwrap_or_default();
    convs.extend(recorded.iter().map(|c| c.id.clone()));
    for conv in convs {
        ctx.folds.close(&conv)?;
    }
    // A fold is stopped with its chat, and what it wrote goes with the folder
    // as a commit of its own.
    for c in recorded.iter().filter(|c| ctx.folds.is_folding(&c.id)) {
        if let Err(e) = crate::workspacefolds::closed(ctx, id, c, true) {
            crate::log::warn(&format!("workspace {id}: what chat {}'s fold left could not be committed: {e}", c.id));
        }
    }
    crate::workspacefolds::forget(ctx, id);
    Ok(())
}

/// Stamp `last_activity`. A stamp that could not be written is logged and
/// never fails the act it follows.
pub(crate) fn touched(ctx: &Ctx, id: &str, conv: &str) {
    if let Err(e) = chats::touch_at(ctx.workspaces_dir(), id, conv) {
        crate::log::warn(&format!("workspace {id}: could not stamp chat {conv}: {e}"));
    }
}

/// The poll's part: look at the folds, push `workspaces:sessions` when it
/// changed (and on the first tick, whatever it holds), and stamp the chats a
/// turn or a session ended in. Answers the folds it decided.
pub fn tick(ctx: &Ctx, now: WorkspaceSessions, last: &mut Option<WorkspaceSessions>) -> Vec<crate::workspacefolds::FoldNote> {
    let mut now = if now.live.is_empty() { now } else { now.with_chats(ctx.workspaces_dir()) };
    let folded = crate::workspacefolds::tick(ctx, &mut now);
    if last.as_ref() == Some(&now) {
        return folded;
    }
    if let Some(before) = last.as_ref() {
        for c in now.ended_since(before) {
            // Not one that was just deleted, with its workspace or alone.
            if chats::find_at(ctx.workspaces_dir(), &c.workspace, &c.chat).is_ok() {
                touched(ctx, &c.workspace, &c.chat);
            }
        }
    }
    ctx.emit("workspaces:sessions", serde_json::to_value(&now).unwrap_or(Value::Null));
    *last = Some(now);
    folded
}

// ── the chat: the agent rows' bodies on the chat's session ──────────────────

/// The chat, and the lines it was launched on.
fn opened(ctx: &Ctx, id: &str, conv: &str) -> Result<(Locus, Vec<turns::Opener>), String> {
    let c = chats::find_at(ctx.workspaces_dir(), id, conv)?;
    Ok((locus_of(ctx, id, &c)?, chats::openers(&wsfolder::folder_at(ctx.workspaces_dir(), id)?, &c)))
}

/// A turn the engine sent says so (`engine`: `start`, `fold`): it is not the
/// person's, and no client needs to know it by its words.
pub fn workspace_turns(ctx: &Ctx, id: String, conv: String, n: Option<u32>) -> Result<Value, String> {
    let (at, openers) = opened(ctx, &id, &conv)?;
    agents::turns_at(ctx, &at, None, n, &openers)
}

pub fn workspace_turn_text(ctx: &Ctx, id: String, conv: String, turn_id: String) -> Result<Value, String> {
    let (at, openers) = opened(ctx, &id, &conv)?;
    agents::turn_text_at(ctx, &at, turn_id, &openers)
}

pub fn workspace_tool_call(ctx: &Ctx, id: String, conv: String, call_id: String) -> Result<Value, String> {
    agents::tool_call_at(ctx, &locus(ctx, &id, &conv)?, call_id)
}

pub fn workspace_prompt(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    agents::prompt_at(ctx, &locus(ctx, &id, &conv)?)
}

pub fn workspace_answer(ctx: &Ctx, id: String, conv: String, choice: u32, fingerprint: String) -> Result<Value, String> {
    let v = agents::answer_at(ctx, &locus(ctx, &id, &conv)?, choice, fingerprint)?;
    touched(ctx, &id, &conv);
    Ok(v)
}

pub fn workspace_answer_text(ctx: &Ctx, id: String, conv: String, choice: u32, fingerprint: String, text: String) -> Result<Value, String> {
    let v = agents::answer_text_at(ctx, &locus(ctx, &id, &conv)?, choice, fingerprint, text)?;
    touched(ctx, &id, &conv);
    Ok(v)
}

pub fn workspace_answer_many(ctx: &Ctx, id: String, conv: String, choices: Vec<u32>, fingerprint: String) -> Result<Value, String> {
    let v = agents::answer_many_at(ctx, &locus(ctx, &id, &conv)?, choices, fingerprint)?;
    touched(ctx, &id, &conv);
    Ok(v)
}

/// `workspace_send`: `agent_send`'s body, into the chat's own session.
pub fn workspace_send(ctx: &Ctx, id: String, conv: String, text: String) -> Result<Value, String> {
    let at = locus(ctx, &id, &conv)?;
    let session = chats::session_name(&conv);
    let v = agents::send_at(ctx, &at, session, text)?;
    touched(ctx, &id, &conv);
    Ok(v)
}

pub fn workspace_interrupt(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    agents::interrupt_at(ctx, &locus(ctx, &id, &conv)?)
}

/// `workspace_term_open`: attach a terminal to the chat's own session, as
/// `idea_term_open` does to a conversation's. The client names the chat and
/// never a session. Answers the session and the attach's id, which the
/// `term_*` rows then take.
pub fn workspace_term_open(ctx: &Ctx, id: String, conv: String, cols: u16, rows: u16, on_bytes: Arc<dyn crate::ByteSink>) -> Result<(String, u32), String> {
    let c = chats::find_at(ctx.workspaces_dir(), &id, &conv)?;
    let session = chats::session_name(&c.id);
    let term = crate::term::term_open(session.clone(), cols, rows, on_bytes, &ctx.terminals)?;
    Ok((session, term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use worktrees_core::harness;

    /// A fold that landed writes its receipt and then ends its session
    /// (`workspacefolds::land`). In between, and while that end is tried
    /// again, the row reads ended, as its receipt says: never `done` and
    /// `running` at once (the macOS CI flake of 2026-11-27).
    #[test]
    fn a_landed_fold_whose_session_is_still_up_reads_ended() {
        let mut c: Chat = serde_json::from_value(serde_json::json!({ "id": worktrees_core::ulid::new(), "harness": "claude", "session_id": "s", "title": "t", "started": "" })).unwrap();
        let live: std::collections::HashSet<String> = [chats::session_name(&c.id)].into();
        assert!(up_in(&live, &c), "a chat with no fold is up while its session is");
        c.fold = Some(chats::Fold { state: chats::FOLDED.into(), closing: true, ..chats::Fold::default() });
        assert!(!up_in(&live, &c), "its receipt is written: it is closed");
        c.fold.as_mut().unwrap().closing = false;
        assert!(up_in(&live, &c), "a resumed chat after its fold is up again");
        assert!(!up_in(&Default::default(), &c));
    }

    /// The snapshot's agent rows never list a workspace chat as a place's
    /// agent, even with its pane inside the place.
    #[test]
    fn a_places_agent_rows_never_list_a_workspace_chat() {
        let s = chats::session_name(&worktrees_core::ulid::new());
        let panes = tmux::PaneList::from_panes(vec![tmux::Pane {
            session: s.clone(),
            path: "/r/repo".into(),
            cmd: "claude".into(),
            id: Some("%3".into()),
            ..tmux::Pane::default()
        }]);
        let rows = crate::workspace::agent_session_names(Some(&panes), "repo-(main)", "repo-(main)", false, "/r/repo", Some("/r/repo/.worktrees"));
        assert!(rows.iter().all(|(_, name, up)| !up && *name != s), "{:?}", rows.iter().map(|r| (&r.1, r.2)).collect::<Vec<_>>());
    }

    /// No lanes, and a transcript read by id. Anything else is a failure.
    struct Ended(Option<Vec<String>>, std::path::PathBuf, bool);

    impl agents::AgentIo for Ended {
        fn lanes(&self, _: &Locus) -> Vec<agents::Lane> {
            Vec::new()
        }
        fn activity(&self, _: &Locus) -> activity::Activity {
            activity::Activity::none()
        }
        fn capture(&self, _: &agents::Lane) -> Option<String> {
            panic!("an ended chat has no screen to read")
        }
        fn press(&self, _: &agents::Lane, _: tmux::Key) -> Result<(), String> {
            panic!("no key reaches an ended chat")
        }
        fn paste(&self, _: &agents::Lane, _: &str) -> Result<(), String> {
            panic!("no text reaches an ended chat")
        }
        fn deliver(&self, _: &Locus, _: &agents::Lane, _: &str) -> Result<harness::SendOutcome, String> {
            panic!("no text reaches an ended chat")
        }
        fn transcript(&self, at: &Locus, lane: &agents::Lane) -> Result<Vec<String>, String> {
            let d = at.desk().expect("a desk session");
            assert!(matches!(at, Locus::Workspace { .. }) && lane.pane.is_empty() && d.dir == self.1, "read by id, in the workspace's folder");
            self.0.clone().ok_or_else(|| agents::ERR_TRANSCRIPT_GONE.to_string())
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
        fn command(&self, _: &agents::Lane, _: &str, _: &str) -> Result<harness::SendOutcome, String> {
            panic!("no commands")
        }
        fn terminal_activity(&self, _: &agents::Lane) -> Result<u64, String> {
            Err("none".into())
        }
        fn sleep(&self, _: u64) {}
        fn session_up(&self, _: &str) -> bool {
            self.2
        }
    }

    /// A chat whose session is up with no Claude in it (pane 0 is still the
    /// shell: an rc file's question, 2026-11-26) is not ended: it says
    /// Claude has not started, and a send says so too, never "no agent".
    #[test]
    fn a_chat_whose_claude_never_started_says_so_and_is_not_ended() {
        let top = std::env::temp_dir().join(format!("wt-wschat-shell-{}", worktrees_core::ulid::new()));
        let dir = top.join("workspaces");
        let ws = worktrees_core::ulid::new();
        let c = chats::prepare_at(&dir, &ws, |_| String::new()).unwrap();
        let ctx = Ctx::new(top.join("cfg"), Arc::new(crate::tests::Recorder::default()))
            .with_agents(agents::Agents::new(Arc::new(Ended(None, dir.join(&ws), true))))
            .with_workspaces_dir(dir.clone());
        let t = workspace_turns(&ctx, ws.clone(), c.id.clone(), None).unwrap();
        assert_eq!((t["ended"].clone(), t["not_started"].clone(), t["reason"].clone()), (json!(false), json!(true), json!(agents::ERR_NOT_STARTED)), "{t}");
        assert_eq!(workspace_send(&ctx, ws.clone(), c.id.clone(), "hello".into()), Err(agents::ERR_NOT_STARTED.to_string()));
        assert_eq!(workspace_prompt(&ctx, ws.clone(), c.id.clone()).unwrap()["waiting"], json!(false));
        let _ = std::fs::remove_dir_all(&top);
    }

    /// An ended chat reads from its transcript, found by the record's id in
    /// the workspace's folder, and takes no text. A chat of another workspace
    /// is not this one's, and two chats hold two locks.
    #[test]
    fn an_ended_chat_reads_from_its_transcript_and_takes_no_text() {
        let top = std::env::temp_dir().join(format!("wt-wschat-ended-{}", worktrees_core::ulid::new()));
        let dir = top.join("workspaces");
        let ws = worktrees_core::ulid::new();
        let c = chats::prepare_at(&dir, &ws, |_| String::new()).unwrap();
        let d = chats::prepare_at(&dir, &ws, |_| String::new()).unwrap();
        let lines: Vec<String> = include_str!("../../worktrees-core/tests/fixtures/transcripts/claude-2.1.309.jsonl").lines().map(str::to_string).collect();
        let ctx = |t: Option<Vec<String>>| {
            Ctx::new(top.join("cfg"), Arc::new(crate::tests::Recorder::default()))
                .with_agents(agents::Agents::new(Arc::new(Ended(t, dir.join(&ws), false))))
                .with_workspaces_dir(dir.clone())
        };
        let t = workspace_turns(&ctx(Some(lines)), ws.clone(), c.id.clone(), None).unwrap();
        assert_eq!(t["ended"], json!(true), "{t}");
        assert!(!t["turns"].as_array().unwrap().is_empty(), "{t}");
        let gone = workspace_turns(&ctx(None), ws.clone(), c.id.clone(), None).unwrap();
        assert_eq!((gone["ended"].clone(), gone["reason"].clone()), (json!(true), json!(agents::ERR_TRANSCRIPT_GONE)));
        assert!(workspace_send(&ctx(None), ws.clone(), c.id.clone(), "hello".into()).is_err());
        assert_eq!(chats::find_at(&dir, &ws, &c.id).unwrap(), c, "a refused send is no activity");
        assert!(workspace_turns(&ctx(None), worktrees_core::ulid::new(), c.id.clone(), None).is_err(), "another workspace's id");
        let x = ctx(None);
        let key = |id: &str| locus(&x, &ws, id).unwrap().lock_key();
        assert_eq!(key(&c.id), format!("ws:worktrees~ws~{}", c.id));
        assert_ne!(key(&c.id), key(&d.id));
        let _ = std::fs::remove_dir_all(&top);
    }

    /// The poll pushes the first tick and every change, and nothing between;
    /// a chat whose turn ended is stamped, once.
    #[test]
    fn the_poll_pushes_changes_and_stamps_a_turn_that_ended() {
        let dir = std::env::temp_dir().join(format!("wt-wschat-tick-{}", worktrees_core::ulid::new()));
        let ws = worktrees_core::ulid::new();
        let c = chats::prepare_at(&dir, &ws, |_| String::new()).unwrap();
        chats::update_at(&dir, &ws, &c.id, |c| c.last_activity = "2020-01-01T00:00:00Z".into()).unwrap();
        let rec = Arc::new(crate::tests::Recorder::default());
        let ctx = Ctx::new(dir.join("cfg"), rec.clone()).with_workspaces_dir(dir.clone());
        let s = chats::session_name(&c.id);
        let busy = WorkspaceSessions { live: vec![s.clone()], busy: vec![s.clone()], ..Default::default() };
        let idle = WorkspaceSessions { live: vec![s.clone()], ..Default::default() };
        let stamp = || chats::find_at(&dir, &ws, &c.id).unwrap().last_activity;
        let mut last = None;
        tick(&ctx, WorkspaceSessions::default(), &mut last);
        tick(&ctx, WorkspaceSessions::default(), &mut last);
        tick(&ctx, busy.clone(), &mut last);
        tick(&ctx, busy.clone(), &mut last);
        assert_eq!(stamp(), "2020-01-01T00:00:00Z", "a turn still running is not stamped");
        tick(&ctx, idle.clone(), &mut last);
        assert!(stamp() > c.started.chars().take(4).collect::<String>(), "{}", stamp());
        let events: Vec<_> = rec.events.lock().unwrap().clone();
        assert_eq!(events.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), ["workspaces:sessions"; 3], "{events:?}");
        assert_eq!(events[1].1["chat_of"][&s], json!({ "workspace": ws, "chat": c.id }));
        // A chat deleted between two ticks is nobody's to stamp.
        chats::remove_at(&dir, &ws, &c.id).unwrap();
        tick(&ctx, WorkspaceSessions::default(), &mut last);
        assert!(chats::list_at(&dir, &ws).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Tidy… is refused while this engine holds the memory for a fold, even
    /// before the record says so; and with the record's rule otherwise.
    #[test]
    fn a_tidy_is_refused_while_this_engine_holds_the_memory() {
        let top = std::env::temp_dir().join(format!("wt-wschat-tidy-{}", worktrees_core::ulid::new()));
        let dir = top.join("workspaces");
        let ctx = Ctx::new(top.join("cfg"), Arc::new(crate::tests::Recorder::default())).with_workspaces_dir(dir.clone());
        let ws = worktrees_core::ulid::new();
        let none = |_: &str| false;
        assert_eq!(tidy_may(&ctx, &ws, &none).err().as_deref(), Some(chats::ERR_TIDY_NO_INDEX));
        worktrees_core::memory::init_at(&dir, &ws, "Home").unwrap();
        assert_eq!(tidy_may(&ctx, &ws, &none), Ok(()));
        ctx.memory.hold(&ws);
        assert_eq!(tidy_may(&ctx, &ws, &none).err().as_deref(), Some(chats::ERR_TIDY_FOLDING));
        ctx.memory.release(&ws);
        assert_eq!(tidy_may(&ctx, &ws, &none), Ok(()));
        let _ = std::fs::remove_dir_all(&top);
    }

    /// Removing a workspace that is not on the list touches no folder, and one
    /// whose list cannot be written gets its folder back.
    #[test]
    fn a_removal_that_fails_puts_the_folder_back() {
        let top = std::env::temp_dir().join(format!("wt-wschat-rm-{}", worktrees_core::ulid::new()));
        let dir = top.join("workspaces");
        let file = top.join("cfg").join("workspaces.json");
        let ctx = Ctx::new(top.join("cfg"), Arc::new(crate::tests::Recorder::default())).with_workspaces_dir(dir.clone()).with_workspaces_file(file.clone());
        let id = store::create_at(&file, "Home").unwrap().id;
        chats::prepare_at(&dir, &id, |_| String::new()).unwrap();
        let stranger = worktrees_core::ulid::new();
        chats::prepare_at(&dir, &stranger, |_| String::new()).unwrap();
        assert_eq!(crate::workspaces::workspace_remove(&ctx, stranger.clone()).unwrap().moved_to, None);
        assert!(dir.join(&stranger).exists());
        use std::os::unix::fs::PermissionsExt;
        let mode = |m: u32| std::fs::set_permissions(file.parent().unwrap(), std::fs::Permissions::from_mode(m)).unwrap();
        mode(0o555);
        let r = crate::workspaces::workspace_remove(&ctx, id.clone());
        mode(0o755);
        // Root (a container) writes through the mode, so there it succeeds.
        if r.is_err() {
            assert!(dir.join(&id).join("chats").exists() && dir.join(".engine").join(&id).join("chats.json").exists(), "both halves are back");
            assert_eq!(store::read_at(&file).unwrap().workspaces.len(), 1);
            let r = crate::workspaces::workspace_remove(&ctx, id.clone()).unwrap();
            assert_eq!(r.moved_to.as_deref(), dir.join(".removed").join(&id).to_str());
        }
        let _ = std::fs::remove_dir_all(&top);
    }
}
