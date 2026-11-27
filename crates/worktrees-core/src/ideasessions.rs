//! Idea sessions: conversations with Claude about one idea
//! (`docs/proposals/idea-board-phases.md` §7.3, phase S).
//!
//! An idea session is NOT a place. It runs in its own tmux session,
//! `worktrees~idea~<conversation ULID>` ([`session_name`]), with the ideas
//! folder as its working directory. `~` never appears in a place's session
//! (`<prefix>-<slug>`, the prefix `[a-z0-9_-]`, the slug a git ref), so no
//! place reader can take it for one, as the dock's `~term` sidecars already
//! rely on (`tmux::SHELL_SIDECAR_MARKER`).
//!
//! **The record** is one file per idea, `ideas/.desk/<idea id>/sessions.json`
//! ([`Record`]), written with `fsx::write_atomic`. The ideas store lists only
//! top-level `.md` names not starting with `.`, so `.desk/` is never an idea.
//! The idea's own text gets nothing: a conversation is not part of it.
//!
//! **What reaches Claude.** A new conversation writes `.desk/<idea>/<conv>/
//! start.md` from a fixed template ([`start_text`]: ids and the idea's file
//! name, never its text), takes a history copy of the idea ("we can see where
//! it started"), chooses Claude's session id and records it, and only then
//! launches `claude --name <session> --session-id <uuid> --plugin-dir
//! <prompt plugin> '<opener>'`, the opener being the fixed
//! `Read .desk/<idea>/<conv>/start.md and begin.` ([`opener`]). Every id is
//! checked ([`crate::ulid::is_valid`], [`crate::codex::is_uuid`]) before it
//! names a path or reaches argv. A resume is `--resume '<session id>'` and
//! no opener: the conversation already has it. Nothing is ever typed into
//! the pane to start it.
//!
//! **No guidance plugin and no planning plugin**: those reach a Claude only
//! through `ops::ai_launch_for`, which takes a project, and this launcher has
//! none. The prompt plugin (`ops::prompt_hook_for`) is on, because the chat
//! reads a tall permission prompt through its record.
//!
//! **The live-agent limit** (`[limits] max_live_agents`) counts an idea
//! session like a lane, and a start past it is refused with a lane's words:
//! this launcher asks `guard::admit` twice, the second time under the
//! machine's launch lock, as `ops::launch` does.

use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::desksessions::{self as desk, Desk, Launch};
pub use crate::desksessions::{help_has_session_id, new_uuid, transcript_paths, Start};
use crate::ideas;

/// The folder under the ideas folder that holds conversations' records and
/// files. Starts with `.`, so the store never lists it.
pub const DESK_DIR: &str = ".desk";
/// The record's file name in `.desk/<idea id>/`.
pub const RECORD: &str = "sessions.json";
/// The file each conversation opens on, in `.desk/<idea id>/<conv>/`.
pub const START: &str = "start.md";
/// What every idea session's tmux name carries, and no place's can.
pub const MARKER: &str = "~idea~";
/// The tmux name's head.
pub const SESSION_PREFIX: &str = "worktrees~idea~";

/// The tmux session of conversation `conv`.
pub fn session_name(conv: &str) -> String {
    format!("{SESSION_PREFIX}{conv}")
}

/// Whether tmux session `name` is an idea session's. The marker alone
/// decides, as `~term` does for a dock shell.
pub fn is_idea_session(name: &str) -> bool {
    name.contains(MARKER)
}

/// The conversation an idea session's tmux name names, when it is one this
/// module made (a ULID after the prefix).
pub fn conv_of_session(name: &str) -> Option<&str> {
    name.strip_prefix(SESSION_PREFIX).filter(|c| crate::ulid::is_valid(c))
}

/// The fixed opener: a file name made of two ULIDs the engine checked.
pub fn opener(idea: &str, conv: &str) -> String {
    format!("Read {DESK_DIR}/{idea}/{conv}/{START} and begin.")
}

/// [`opener`], as the turns are read by it.
pub fn start_opener(idea: &str, conv: &str) -> crate::turns::Opener {
    crate::turns::Opener { engine: crate::turns::Engine::Start, text: opener(idea, conv) }
}

/// One conversation, as the record keeps it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    /// The conversation's ULID: names its folder and its tmux session.
    pub id: String,
    /// `claude` (the only harness phase S launches).
    pub harness: String,
    /// Claude's own id for it, chosen here and written BEFORE the launch.
    pub session_id: String,
    pub title: String,
    /// When it started, `YYYY-MM-DDTHH:MM:SSZ`.
    pub started: String,
    /// The history copy taken as it started, relative to the ideas folder.
    pub started_from: String,
}

impl desk::Entry for Conversation {
    fn id(&self) -> &str {
        &self.id
    }
    fn session_id(&self) -> &str {
        &self.session_id
    }
    fn set_title(&mut self, title: String) {
        self.title = title;
    }
}

/// `.desk/<idea id>/sessions.json`. Keys this build does not know are kept.
pub type Record = desk::Record<Conversation>;

fn check_idea(idea: &str) -> Result<(), String> {
    if crate::ulid::is_valid(idea) { Ok(()) } else { Err(format!("not an idea id: {idea:?}")) }
}

fn check_conv(conv: &str) -> Result<(), String> {
    desk::check_conv("conversation", conv)
}

/// `.desk/<idea>` and its record, the idea id checked first.
fn idea_desk(root: &Path, idea: &str) -> Result<Desk, String> {
    check_idea(idea)?;
    let all = root.join(DESK_DIR);
    Ok(Desk {
        root: root.to_path_buf(),
        dirs: vec![all.clone(), all.join(idea)],
        file: RECORD,
        convs: Vec::new(),
        owner: format!("the idea {idea}"),
        noun: "conversation",
        refuse_real: ideas::refuse_real,
        lock: None,
    })
}

/// The record of `idea`, or an empty one when it has none ([`Desk::read`]).
pub fn read_record_at(root: &Path, idea: &str) -> Result<Record, String> {
    idea_desk(root, idea)?.read()
}

/// The conversation `conv` of `idea`.
pub fn find_at(root: &Path, idea: &str, conv: &str) -> Result<Conversation, String> {
    check_conv(conv)?;
    idea_desk(root, idea)?.find(conv)
}

/// The idea's conversations, newest first.
pub fn list_at(root: &Path, idea: &str) -> Result<Vec<Conversation>, String> {
    idea_desk(root, idea)?.list()
}

/// What `start.md` says: the fixed template, filled with the idea's id, its
/// file name now, and the conversation's own folder. It names the id as well
/// as the file because a heading change renames the file mid-conversation.
pub fn start_text(idea: &str, file: &str, conv: &str) -> String {
    format!(
        "# This conversation\n\
         \n\
         This conversation is about ONE idea: the markdown document in this folder whose\n\
         frontmatter says `id: {idea}`. Its file is `{file}` now.\n\
         \n\
         1. Read `{file}`. If it is not there (a new heading renames the file), find the `.md`\n\
            file in this folder whose frontmatter says `id: {idea}` and use that one.\n\
         2. Talk the idea through with the person who opened this conversation: ask, suggest,\n\
            sharpen. Start by saying in a few lines what you read and what you would ask first.\n\
         3. Change the idea's file only when they ask you to. Keep its frontmatter, and its\n\
            `id:` line exactly as it is.\n\
         4. Never write anywhere else in this folder, except inside `{DESK_DIR}/{idea}/{conv}/`,\n\
            which is this conversation's own folder.\n"
    )
}

/// A new conversation on `idea`, up to the launch: the history copy, the
/// conversation's folder and `start.md`, and its record entry with the
/// session id Claude will be given. Nothing is launched.
pub fn prepare_at(root: &Path, idea: &str) -> Result<Conversation, String> {
    check_idea(idea)?;
    ideas::refuse_real(root)?;
    let doc = ideas::read_at(root, idea)?;
    if doc.merged.is_some() {
        return Err(format!("that idea was merged into “{}”: talk about that one", doc.idea.title));
    }
    let copy = ideas::history_copy_at(root, idea, doc.content.as_bytes())?;
    let started_from = copy.strip_prefix(root).unwrap_or(&copy).to_string_lossy().into_owned();
    let desk = idea_desk(root, idea)?;
    desk.read::<Conversation>()?;
    let conv = crate::ulid::new();
    let dir = desk.make_conv_dir(&conv)?;
    ideas::create_new(&dir.join(START), start_text(idea, &doc.idea.file, &conv).as_bytes()).map_err(|e| format!("{START}: {e}"))?;
    desk.edit(|rec: &mut Record| {
        let c = Conversation {
            id: conv,
            harness: crate::provider::CLAUDE.id.to_string(),
            session_id: new_uuid(),
            title: format!("Conversation {}", rec.conversations.len() + 1),
            started: desk::now_iso(),
            started_from,
        };
        rec.conversations.push(c.clone());
        Ok(c)
    })
}

/// Retitle conversation `conv`. A title is one line of at most 120 characters.
pub fn rename_at(root: &Path, idea: &str, conv: &str, title: &str) -> Result<Conversation, String> {
    check_conv(conv)?;
    idea_desk(root, idea)?.rename(conv, title)
}

/// Forget conversation `conv`: its record entry and its folder. Claude's
/// transcript is Claude's file and stays. The caller closes a live session
/// first.
pub fn remove_at(root: &Path, idea: &str, conv: &str) -> Result<(), String> {
    check_conv(conv)?;
    idea_desk(root, idea)?.remove::<Conversation>(conv)
}

/// The idea whose record has conversation `conv`: the folder
/// `.desk/<idea>/<conv>/` that `prepare_at` made, looked for under each idea
/// in `.desk/`, real directories only. The poll's question for a live
/// session (`ideas:sessions`' `idea_of`): one `readdir`, one `lstat` per idea.
pub fn idea_of_conv(root: &Path, conv: &str) -> Option<String> {
    if !crate::ulid::is_valid(conv) {
        return None;
    }
    let desk = root.join(DESK_DIR);
    if !fs::symlink_metadata(&desk).ok()?.file_type().is_dir() {
        return None;
    }
    fs::read_dir(&desk).ok()?.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).filter(|n| crate::ulid::is_valid(n)).find(|idea| {
        fs::symlink_metadata(desk.join(idea)).is_ok_and(|m| m.file_type().is_dir())
            && fs::symlink_metadata(desk.join(idea).join(conv)).is_ok_and(|m| m.file_type().is_dir())
    })
}

// ── the ideas folder must not be a place's ──────────────────────────────────

/// Refuse an ideas folder inside a registered project (or one holding a
/// project): every conversation's pane is cwd'd there, and a place adopts the
/// sessions whose panes sit under it (`Project::live_session_in`). Inside a
/// project, an idea session would be taken for that place's.
pub fn check_root(root: &Path, roots: &[String]) -> Result<(), String> {
    match desk::overlapping(root, roots) {
        None => Ok(()),
        Some(r) => Err(format!(
            "the ideas folder {} is inside the registered project {r} (or holds it): conversations are not started there, \
             because the project's places would take them for their own. Point {} at a folder outside every project.",
            root.display(),
            ideas::OVERRIDE_ENV
        )),
    }
}

// ── launching ───────────────────────────────────────────────────────────────

/// The launch of conversation `c` of `idea` in `root`: on its opener when
/// fresh, with none when resumed.
fn launch<'a>(root: &'a Path, idea: &str, c: &'a Conversation, session: &'a str, how: Start) -> Result<Launch<'a>, String> {
    check_idea(idea)?;
    Ok(Launch {
        cwd: root,
        session,
        conv: &c.id,
        session_id: &c.session_id,
        how,
        opener: (how == Start::Fresh).then(|| opener(idea, &c.id)),
        settings: None,
        env: Vec::new(),
        noun: "conversation",
        a_kind: "an idea conversation",
        kinds: "idea conversations",
    })
}

/// The `pane0` that runs Claude for conversation `c` of `idea` in tmux
/// session `session` ([`Launch::pane0`]).
pub fn pane0(ai_cmd: &str, idea: &str, c: &Conversation, session: &str, how: Start, prompt_hook: Vec<String>) -> Result<String, String> {
    launch(Path::new(""), idea, c, session, how)?.pane0(ai_cmd, prompt_hook)
}

/// [`pane0`]'s command line, before it is quoted for tmux: what the inner
/// shell runs.
pub fn launch_line(ai_cmd: &str, idea: &str, c: &Conversation, session: &str, how: Start, prompt_hook: Vec<String>) -> Result<String, String> {
    launch(Path::new(""), idea, c, session, how)?.line(ai_cmd, prompt_hook)
}

/// Launch conversation `conv` of `idea` in `root` ([`Launch::run`]), refused
/// for an ideas folder inside a registered project. Answers the tmux
/// session's name.
pub fn launch_at(root: &Path, idea: &str, conv: &str, how: Start) -> Result<String, String> {
    check_idea(idea)?;
    check_conv(conv)?;
    check_root(root, &crate::registry::read_lenient().roots())?;
    let c = find_at(root, idea, conv)?;
    let session = session_name(&c.id);
    launch(root, idea, &c, &session, how)?.run()?;
    Ok(session)
}

/// A new conversation: [`prepare_at`], then [`launch_at`]. A launch that is
/// refused takes the conversation back out (record entry and folder; the
/// history copy stays, as every copy does), so a refusal leaves nothing.
pub fn start_at(root: &Path, idea: &str) -> Result<(Conversation, String), String> {
    check_idea(idea)?;
    check_root(root, &crate::registry::read_lenient().roots())?;
    let c = prepare_at(root, idea)?;
    match launch_at(root, idea, &c.id, Start::Fresh) {
        Ok(session) => Ok((c, session)),
        Err(e) => {
            let _ = remove_at(root, idea, &c.id);
            Err(e)
        }
    }
}

/// End conversation `conv`'s session (`kill-session`). Its record and
/// Claude's transcript stay. Answers whether a session was running.
pub fn close(conv: &str) -> Result<bool, String> {
    check_conv(conv)?;
    desk::close(&session_name(conv))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-ideasess-{name}-{}", crate::ulid::new()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_session_name_is_never_a_places_and_names_its_conversation() {
        let conv = crate::ulid::new();
        let s = session_name(&conv);
        assert!(is_idea_session(&s));
        assert_eq!(conv_of_session(&s), Some(conv.as_str()));
        assert!(!crate::tmux::is_shell_sidecar(&s), "not a dock shell");
        assert!(!crate::provider::is_sidecar(&s), "not a provider sidecar");
        assert_eq!(conv_of_session("worktrees~idea~../x"), None);
        assert!(!is_idea_session("repo-feat-idea"), "a place's session never carries `~`");
    }

    #[test]
    fn a_new_conversation_writes_its_record_start_file_and_history_copy() {
        let root = tmp("prepare");
        let doc = ideas::create_at(&root, "Jellyfin sharing").unwrap();
        let id = doc.idea.id.clone().unwrap();
        let c = prepare_at(&root, &id).unwrap();
        assert!(crate::ulid::is_valid(&c.id));
        assert!(crate::codex::is_uuid(&c.session_id));
        assert_eq!(c.title, "Conversation 1");
        assert_eq!(c.harness, "claude");
        assert!(c.started_from.starts_with(&format!(".history/{id}/")), "{}", c.started_from);
        assert_eq!(fs::read_to_string(root.join(&c.started_from)).unwrap(), doc.content, "the copy is the idea as it was");
        let start = fs::read_to_string(root.join(DESK_DIR).join(&id).join(&c.id).join(START)).unwrap();
        assert_eq!(start, start_text(&id, &doc.idea.file, &c.id));
        assert!(start.contains(&format!("id: {id}")) && start.contains(&doc.idea.file));
        assert!(!start.contains("Jellyfin sharing"), "start.md carries no text of the idea");
        let rec = read_record_at(&root, &id).unwrap();
        assert_eq!(rec.conversations, vec![c.clone()]);
        // The store never lists `.desk` or its files as ideas.
        let listed = ideas::list_at(&root).unwrap();
        assert_eq!(listed.ideas.len(), 1);
        assert!(listed.skipped.is_empty(), "{:?}", listed.skipped);
        let c2 = prepare_at(&root, &id).unwrap();
        assert_eq!(c2.title, "Conversation 2");
        assert_eq!(list_at(&root, &id).unwrap().iter().map(|c| c.id.clone()).collect::<Vec<_>>(), vec![c2.id.clone(), c.id.clone()], "newest first");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_conversation_names_its_idea_by_its_folder() {
        let root = tmp("ideaof");
        let id = ideas::create_at(&root, "x").unwrap().idea.id.unwrap();
        let c = prepare_at(&root, &id).unwrap();
        assert_eq!(idea_of_conv(&root, &c.id), Some(id));
        assert_eq!(idea_of_conv(&root, &crate::ulid::new()), None);
        assert_eq!(idea_of_conv(&root, "../x"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_and_remove_keep_to_the_record_and_the_conversations_folder() {
        let root = tmp("remove");
        let id = ideas::create_at(&root, "x").unwrap().idea.id.unwrap();
        let c = prepare_at(&root, &id).unwrap();
        let keep = prepare_at(&root, &id).unwrap();
        assert_eq!(rename_at(&root, &id, &c.id, "  Who can see what\nsecond line").unwrap().title, "Who can see what");
        assert!(rename_at(&root, &id, &c.id, " \n").is_err());
        remove_at(&root, &id, &c.id).unwrap();
        assert!(!root.join(DESK_DIR).join(&id).join(&c.id).exists());
        assert!(root.join(DESK_DIR).join(&id).join(&keep.id).join(START).exists());
        assert_eq!(read_record_at(&root, &id).unwrap().conversations, vec![keep]);
        assert!(remove_at(&root, &id, &c.id).is_err(), "gone already");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn every_id_is_checked_before_it_names_a_path_or_reaches_argv() {
        let root = tmp("ids");
        for bad in ["../../etc", "01JC8Z4K3M9QX2T7VBN5R6WYH", "", "01jc8z4k3m9qx2t7vbn5r6wyhd"] {
            assert!(read_record_at(&root, bad).is_err(), "{bad:?}");
            assert!(prepare_at(&root, bad).is_err(), "{bad:?}");
            assert!(find_at(&root, &crate::ulid::new(), bad).is_err(), "{bad:?}");
            assert!(close(bad).is_err(), "{bad:?}");
        }
        let c = Conversation {
            id: crate::ulid::new(),
            harness: "claude".into(),
            session_id: "not-a-uuid; rm -rf ~".into(),
            title: "t".into(),
            started: String::new(),
            started_from: String::new(),
        };
        assert!(pane0("claude", &crate::ulid::new(), &c, "s", Start::Fresh, vec![]).is_err());
        // A hand-edited record naming a path instead of an id is refused whole.
        let idea = crate::ulid::new();
        let desk = root.join(DESK_DIR).join(&idea);
        fs::create_dir_all(&desk).unwrap();
        fs::write(desk.join(RECORD), r#"{"conversations":[{"id":"../x","harness":"claude","session_id":"0","title":"","started":"","started_from":""}]}"#).unwrap();
        assert!(read_record_at(&root, &idea).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_link_where_the_desk_should_be_is_refused() {
        let root = tmp("link");
        let elsewhere = tmp("link-target");
        std::os::unix::fs::symlink(&elsewhere, root.join(DESK_DIR)).unwrap();
        let idea = crate::ulid::new();
        assert!(read_record_at(&root, &idea).unwrap_err().contains("not a folder"));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&elsewhere);
    }

    #[test]
    fn the_launch_carries_ids_and_flags_and_the_fixed_opener_only() {
        let idea = crate::ulid::new();
        let c = Conversation {
            id: crate::ulid::new(),
            harness: "claude".into(),
            session_id: new_uuid(),
            title: "Talk".into(),
            started: String::new(),
            started_from: String::new(),
        };
        let s = session_name(&c.id);
        let hook = vec!["--plugin-dir".to_string(), "'/data/claude-prompt'".to_string()];
        let fresh = launch_line("claude", &idea, &c, &s, Start::Fresh, hook.clone()).unwrap();
        let want = format!("claude --name '{s}' --session-id '{}' --plugin-dir '/data/claude-prompt' 'Read .desk/{idea}/{}/start.md and begin.'", c.session_id, c.id);
        assert_eq!(fresh, want);
        assert!(!fresh.contains("Talk"), "the title never reaches argv");
        let resumed = launch_line("claude", &idea, &c, &s, Start::Resume, hook.clone()).unwrap();
        assert_eq!(resumed, format!("claude --name '{s}' --resume '{}' --plugin-dir '/data/claude-prompt'", c.session_id), "no opener and no new id on a resume");
        let pane = pane0("claude", &idea, &c, &s, Start::Fresh, hook).unwrap();
        assert!(pane.starts_with("exec env DISABLE_AUTO_UPDATE=true \"${SHELL:-/bin/sh}\" -ic 'unset DISABLE_AUTO_UPDATE; exec claude --name"), "{pane}");
        assert!(pane0("codex", &idea, &c, &s, Start::Fresh, vec![]).is_err(), "Claude only");
    }

    #[test]
    fn the_floor_reads_session_id_off_claudes_help() {
        assert!(help_has_session_id("  --session-id <uuid>   Use a specific session ID"));
        assert!(help_has_session_id("  -r, --resume [value]\n  --session-id, --other"));
        assert!(!help_has_session_id("  --session-identity <x>\n  --resume"));
    }

    #[test]
    fn an_ideas_folder_inside_a_registered_project_is_refused() {
        let proj = tmp("proj");
        let inside = proj.join("notes");
        fs::create_dir_all(&inside).unwrap();
        let outside = tmp("outside");
        let roots = vec![proj.to_string_lossy().into_owned()];
        assert!(check_root(&inside, &roots).unwrap_err().contains("inside the registered project"));
        assert!(check_root(&proj, &roots).is_err(), "the project's own root");
        assert!(check_root(&outside, &roots).is_ok());
        let holder = proj.parent().unwrap().to_path_buf();
        assert!(check_root(&holder, &roots).is_err(), "a folder that holds a project");
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&outside);
    }

    /// `Project::live_session_in` (adoption by pane cwd: `close`, `rm`,
    /// `doctor`'s session scan, the snapshot, the agent rows) and the agents a
    /// launch would end (`agents_in`) never take an idea session for a
    /// place's, even with its pane inside the place — the folder is refused
    /// there anyway (`check_root`).
    #[test]
    fn no_place_reader_adopts_an_idea_session() {
        let s = session_name(&crate::ulid::new());
        for cmd in ["claude", "2.1.322"] {
            let panes = crate::tmux::PaneList::from_panes(vec![crate::tmux::Pane {
                session: s.clone(),
                path: "/r/repo/.worktrees/feat".into(),
                cmd: cmd.into(),
                id: Some("%9".into()),
                ..crate::tmux::Pane::default()
            }]);
            assert_eq!(panes.session_in("/r/repo/.worktrees/feat", "claude", None), None, "{cmd}");
            assert_eq!(panes.session_in("/r/repo", "claude", None), None, "{cmd}: (main)");
            assert!(panes.agents_in("/r/repo/.worktrees/feat", None).is_empty(), "{cmd}");
        }
    }

    #[test]
    fn a_uuid_is_v4_and_passes_the_launch_check() {
        let u = new_uuid();
        assert!(crate::codex::is_uuid(&u), "{u}");
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"), "{u}");
        assert_ne!(u, new_uuid());
    }

    #[test]
    fn a_resumed_conversation_is_looked_for_where_it_began() {
        let claude = tmp("tx-resumed");
        let sid = new_uuid();
        let born = PathBuf::from(crate::project::claude_dir_in(&claude, "/elsewhere")).join(format!("{sid}.jsonl"));
        fs::create_dir_all(born.parent().unwrap()).unwrap();
        fs::write(&born, "{}\n").unwrap();
        let p = transcript_paths(&claude, Path::new("/no/such/root"), &sid);
        assert_eq!(p.last(), Some(&born), "{p:?}");
        fs::remove_file(&born).unwrap();
        assert!(transcript_paths(&claude, Path::new("/no/such/root"), &sid).iter().all(|p| !p.is_file()));
        let _ = fs::remove_dir_all(&claude);
    }

    #[test]
    fn transcripts_are_looked_for_under_the_resolved_folder_first() {
        let root = tmp("tx");
        let sid = new_uuid();
        let p = transcript_paths(Path::new("/c"), &root, &sid);
        let canon = fs::canonicalize(&root).unwrap();
        assert_eq!(p[0], PathBuf::from(crate::project::claude_dir_in(Path::new("/c"), &canon.to_string_lossy())).join(format!("{sid}.jsonl")));
        assert!(transcript_paths(Path::new("/c"), &root, "x").is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
