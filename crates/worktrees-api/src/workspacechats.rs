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

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{json, Value};
use worktrees_core::workspacechats::{self as chats, Chat, Context, PlaceLine, ProjectContext, ProjectRead, Reads, Start};
use worktrees_core::workspaces::{self as store, Workspace};
use worktrees_core::{activity, agent, desksessions, harness, ideas, registry, store as declared, sysclock, tmux, wsfolder, Project};

use crate::agents::{self, Locus};
use crate::watch::WorkspaceSessions;
use crate::Ctx;

/// The lock the life of workspace `id`'s chats holds.
pub fn life_lock(ctx: &Ctx, id: &str) -> Arc<Mutex<()>> {
    ctx.agents.place_lock(&format!("ws-life:{id}"))
}

fn workspace(ctx: &Ctx, id: &str) -> Result<Workspace, String> {
    store::read_at(ctx.workspaces_file())?.workspaces.into_iter().find(|w| w.id == id).ok_or_else(|| format!("no workspace {id}"))
}

fn locus_of(ctx: &Ctx, id: &str, c: &Chat) -> Result<Locus, String> {
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
}

fn state_of(live: bool, a: &activity::Activity) -> &'static str {
    match (live, a.state) {
        (false, _) => "ended",
        (true, activity::State::Busy) => "busy",
        (true, activity::State::Waiting) => "waiting",
        (true, _) => "running",
    }
}

fn listed(ctx: &Ctx, id: &str, c: Chat, live: &std::collections::HashSet<String>) -> Result<Listed, String> {
    let session = chats::session_name(&c.id);
    let up = live.contains(&session);
    let a = if up { ctx.agents.io().activity(&locus_of(ctx, id, &c)?) } else { activity::Activity::none() };
    Ok(Listed { state: state_of(up, &a), session, note_exists: chats::note_exists(ctx.workspaces_dir(), id, &c), chat: c })
}

/// `workspace_chats`: the workspace's chats, newest first, each with its
/// state, from `chats.json` and tmux — no git call — and what they read.
pub fn workspace_chats(ctx: &Ctx, id: String) -> Result<Chats, String> {
    let w = workspace(ctx, &id)?;
    let root = ctx.workspaces_dir();
    let live: std::collections::HashSet<String> = tmux::session_names().into_iter().collect();
    Ok(Chats {
        chats: chats::list_at(root, &id)?.into_iter().map(|c| listed(ctx, &id, c, &live)).collect::<Result<_, _>>()?,
        projects: chats::project_reads(&w, &registry::read_lenient()),
        folder: wsfolder::folder_at(root, &id)?.to_string_lossy().into_owned(),
        name: w.name,
        id,
    })
}

// ── context.md ──────────────────────────────────────────────────────────────

/// What the agents of a place are doing, in words: `claude busy`.
fn agent_words(readings: Vec<(&'static dyn harness::Adapter, activity::Activity)>) -> String {
    let words: Vec<String> = readings
        .into_iter()
        .filter(|(_, a)| a.state != activity::State::None)
        .map(|(h, a)| format!("{} {}", h.provider().id, format!("{:?}", a.state).to_lowercase()))
        .collect();
    words.join(", ")
}

/// The places of the project at `root`: names, branches and declared notes,
/// with what their agents are doing. One `git worktree list`, and no git call
/// per place.
fn places(root: &str, panes: Option<&tmux::PaneList>, probes: &[agent::ClaudeProbe]) -> Result<Vec<PlaceLine>, String> {
    if !Path::new(root).is_dir() {
        return Err("its folder is not there".into());
    }
    let project = Project::discover(Path::new(root)).map_err(|e| e.msg)?;
    let decls = declared::read_lenient(&project.main_root);
    let (now, word) = (sysclock::now_epoch(), worktrees_core::project::adopt_ai_word());
    let scan = harness::Scan { probes, panes };
    Ok(project
        .place_index()
        .into_iter()
        .map(|p| {
            let d = decls.places.get(&p.slug);
            let up = panes.is_some_and(|ps| project.live_session_in(&p.slug, &p.path, &project.place_panes(&p.slug, ps), &word).is_some());
            PlaceLine {
                branch: p.branch.unwrap_or_else(|| "(detached)".into()),
                lifecycle: declared::reconcile(d, up, now),
                note: d.and_then(|d| d.note.clone()).unwrap_or_default(),
                agent: agent_words(harness::place_activities(&project, &p.slug, &p.path, &scan)),
                slug: p.slug,
            }
        })
        .collect())
}

/// The workspace as its chat `conv` is told it (`context.md`). A project that
/// is not read is named, and nothing more is said of it.
fn context(ctx: &Ctx, w: &Workspace, conv: &str) -> Context {
    let panes = tmux::PaneList::fetch();
    let probes = agent::live_probes_in(panes.as_ref());
    let projects = chats::project_reads(w, &registry::read_lenient())
        .into_iter()
        .map(|read| {
            let places = if read.reads == Reads::No { Ok(Vec::new()) } else { places(&read.root, panes.as_ref(), &probes) };
            ProjectContext { read, places }
        })
        .collect();
    let idx = ideas::list_at(ctx.ideas_dir()).ok();
    let title = |id: &str| idx.as_ref()?.lookup(id).ok().map(|(d, _)| d.title.clone());
    Context {
        workspace: w.name.clone(),
        written: desksessions::now_iso(),
        projects,
        plan: w.plan.as_ref().map(|p| (p.clone(), title(p))),
        ideas: w.ideas.iter().map(|i| (i.clone(), title(i))).collect(),
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
    crate::memory::before_launch(ctx, &w)?;
    let (c, session) = chats::start_at(ctx.workspaces_dir(), &w, |conv| chats::context_text(&context(ctx, &w, conv)))?;
    Ok(json!({ "id": id, "chat": c, "session": session }))
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
    if tmux::session_exists(&chats::session_name(&conv)) {
        return Err(format!("chat {conv} is already running (tmux session {})", chats::session_name(&conv)));
    }
    crate::memory::before_launch(ctx, &w)?;
    chats::write_context_at(root, &id, &conv, &chats::context_text(&context(ctx, &w, &conv)))?;
    let session = chats::launch_at(root, &w, &conv, Start::Resume, None)?;
    touched(ctx, &id, &conv);
    Ok(json!({ "id": id, "conv": conv, "session": session }))
}

/// `workspace_chat_close`: end its tmux session. The record, its files and
/// Claude's transcript stay. This is "close without folding".
pub fn workspace_chat_close(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    let lock = life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    chats::find_at(ctx.workspaces_dir(), &id, &conv)?;
    let was = chats::close(&conv)?;
    if was {
        touched(ctx, &id, &conv);
    }
    Ok(json!({ "id": id, "conv": conv, "closed": was }))
}

/// `workspace_chat_remove`: close it if it runs, then forget it: its record
/// entry and its folder. Its note stays, in the memory; Claude's transcript
/// is Claude's file.
pub fn workspace_chat_remove(ctx: &Ctx, id: String, conv: String) -> Result<Value, String> {
    let lock = life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    chats::find_at(ctx.workspaces_dir(), &id, &conv)?;
    chats::close(&conv)?;
    chats::remove_at(ctx.workspaces_dir(), &id, &conv)?;
    Ok(json!({ "id": id, "conv": conv, "removed": true }))
}

/// `workspace_chat_rename`: a new title, one line.
pub fn workspace_chat_rename(ctx: &Ctx, id: String, conv: String, title: String) -> Result<Value, String> {
    let c = chats::rename_at(ctx.workspaces_dir(), &id, &conv, &title)?;
    Ok(json!({ "id": id, "chat": c }))
}

/// End every live chat of workspace `id`: what removing it does first. By the
/// live sessions and their folders as well as the record, so a record that
/// does not read cannot leave one running. The caller holds [`life_lock`].
pub fn close_all(ctx: &Ctx, id: &str) -> Result<(), String> {
    let root = ctx.workspaces_dir();
    let mut convs: Vec<String> = tmux::session_names()
        .iter()
        .filter_map(|s| chats::conv_of_session(s).map(str::to_string))
        .filter(|c| chats::workspace_of_conv(root, c).as_deref() == Some(id))
        .collect();
    convs.extend(chats::list_at(root, id).unwrap_or_default().into_iter().map(|c| c.id));
    for conv in convs {
        chats::close(&conv)?;
    }
    Ok(())
}

/// Stamp `last_activity`. A stamp that could not be written is logged and
/// never fails the act it follows.
fn touched(ctx: &Ctx, id: &str, conv: &str) {
    if let Err(e) = chats::touch_at(ctx.workspaces_dir(), id, conv) {
        crate::log::warn(&format!("workspace {id}: could not stamp chat {conv}: {e}"));
    }
}

/// The poll's part: push `workspaces:sessions` when it changed (and on the
/// first tick, whatever it holds), and stamp the chats a turn or a session
/// ended in.
pub fn tick(ctx: &Ctx, now: WorkspaceSessions, last: &mut Option<WorkspaceSessions>) {
    let now = if now.live.is_empty() { now } else { now.with_chats(ctx.workspaces_dir()) };
    if last.as_ref() == Some(&now) {
        return;
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
}

// ── the chat: the agent rows' bodies on the chat's session ──────────────────

pub fn workspace_turns(ctx: &Ctx, id: String, conv: String, n: Option<u32>) -> Result<Value, String> {
    agents::turns_at(ctx, &locus(ctx, &id, &conv)?, None, n)
}

pub fn workspace_turn_text(ctx: &Ctx, id: String, conv: String, turn_id: String) -> Result<Value, String> {
    agents::turn_text_at(ctx, &locus(ctx, &id, &conv)?, turn_id)
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
    struct Ended(Option<Vec<String>>, std::path::PathBuf);

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
                .with_agents(agents::Agents::new(Arc::new(Ended(t, dir.join(&ws)))))
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
            assert!(dir.join(&id).join("chats.json").exists(), "the folder is back");
            assert_eq!(store::read_at(&file).unwrap().workspaces.len(), 1);
            let r = crate::workspaces::workspace_remove(&ctx, id.clone()).unwrap();
            assert_eq!(r.moved_to.as_deref(), dir.join(".removed").join(&id).to_str());
        }
        let _ = std::fs::remove_dir_all(&top);
    }
}
