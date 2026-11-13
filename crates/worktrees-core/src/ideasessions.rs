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
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
/// A record larger than this is refused rather than read.
const RECORD_MAX: u64 = 1024 * 1024;
/// How long `claude --help` may take before the floor is called unknown.
const HELP_SECS: u64 = 15;

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

/// `.desk/<idea id>/sessions.json`. Keys this build does not know are kept.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Record {
    #[serde(default)]
    pub conversations: Vec<Conversation>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

fn check_idea(idea: &str) -> Result<(), String> {
    if crate::ulid::is_valid(idea) { Ok(()) } else { Err(format!("not an idea id: {idea:?}")) }
}

fn check_conv(conv: &str) -> Result<(), String> {
    if crate::ulid::is_valid(conv) { Ok(()) } else { Err(format!("not a conversation id: {conv:?}")) }
}

/// `.desk/<idea>`, the idea id checked first.
fn idea_desk(root: &Path, idea: &str) -> Result<PathBuf, String> {
    check_idea(idea)?;
    Ok(root.join(DESK_DIR).join(idea))
}

/// The record of `idea`, or an empty one when it has none. A link where the
/// record or its folders should be is refused: `.desk` and everything in it
/// are real directories and regular files this module made.
pub fn read_record_at(root: &Path, idea: &str) -> Result<Record, String> {
    let desk = idea_desk(root, idea)?;
    for d in [root.join(DESK_DIR), desk.clone()] {
        match fs::symlink_metadata(&d) {
            Ok(md) if md.file_type().is_dir() => {}
            Ok(_) => return Err(format!("{} is not a folder", d.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Record::default()),
            Err(e) => return Err(format!("{}: {e}", d.display())),
        }
    }
    let p = desk.join(RECORD);
    let md = match fs::symlink_metadata(&p) {
        Ok(md) => md,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Record::default()),
        Err(e) => return Err(format!("{}: {e}", p.display())),
    };
    if !md.file_type().is_file() {
        return Err(format!("{} is not a regular file", p.display()));
    }
    if md.len() > RECORD_MAX {
        return Err(format!("{} is too large to read", p.display()));
    }
    let text = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let rec: Record = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
    // A hand-edited record names nothing that is not a conversation of ours.
    for c in &rec.conversations {
        check_conv(&c.id)?;
        if !crate::codex::is_uuid(&c.session_id) {
            return Err(format!("{}: conversation {} has no usable session id", p.display(), c.id));
        }
    }
    Ok(rec)
}

fn write_record_at(root: &Path, idea: &str, rec: &Record) -> Result<(), String> {
    ideas::refuse_real(root)?;
    let desk = idea_desk(root, idea)?;
    for d in [root.join(DESK_DIR), desk.clone()] {
        crate::fsx::make_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut text = serde_json::to_string_pretty(rec).map_err(|e| e.to_string())?;
    text.push('\n');
    crate::fsx::write_atomic(&desk.join(RECORD), text.as_bytes(), None).map_err(|e| format!("{RECORD}: {e}"))
}

/// The conversation `conv` of `idea`.
pub fn find_at(root: &Path, idea: &str, conv: &str) -> Result<Conversation, String> {
    check_conv(conv)?;
    read_record_at(root, idea)?
        .conversations
        .into_iter()
        .find(|c| c.id == conv)
        .ok_or_else(|| format!("the idea {idea} has no conversation {conv}"))
}

/// The idea's conversations, newest first.
pub fn list_at(root: &Path, idea: &str) -> Result<Vec<Conversation>, String> {
    let mut v = read_record_at(root, idea)?.conversations;
    v.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(v)
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

/// `YYYY-MM-DDTHH:MM:SSZ` for now.
fn now_iso() -> String {
    let secs = crate::sysclock::now_epoch();
    let (y, m, d) = crate::attach::civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// A random (v4) UUID, lower case: Claude's session id.
pub fn new_uuid() -> String {
    let (a, b) = (crate::fsx::random_u64(), crate::fsx::random_u64());
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
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
    let mut rec = read_record_at(root, idea)?;
    let conv = crate::ulid::new();
    let desk = idea_desk(root, idea)?;
    let dir = desk.join(&conv);
    for d in [root.join(DESK_DIR), desk.clone(), dir.clone()] {
        crate::fsx::make_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    ideas::create_new(&dir.join(START), start_text(idea, &doc.idea.file, &conv).as_bytes()).map_err(|e| format!("{START}: {e}"))?;
    let n = rec.conversations.len() + 1;
    let c = Conversation {
        id: conv,
        harness: crate::provider::CLAUDE.id.to_string(),
        session_id: new_uuid(),
        title: format!("Conversation {n}"),
        started: now_iso(),
        started_from,
    };
    rec.conversations.push(c.clone());
    write_record_at(root, idea, &rec)?;
    Ok(c)
}

/// Retitle conversation `conv`. A title is one line of at most 120 characters.
pub fn rename_at(root: &Path, idea: &str, conv: &str, title: &str) -> Result<Conversation, String> {
    check_conv(conv)?;
    let title: String = title.split(['\n', '\r']).next().unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(120).collect();
    if title.is_empty() {
        return Err("a conversation needs a title".into());
    }
    let mut rec = read_record_at(root, idea)?;
    let c = rec.conversations.iter_mut().find(|c| c.id == conv).ok_or_else(|| format!("the idea {idea} has no conversation {conv}"))?;
    c.title = title;
    let out = c.clone();
    write_record_at(root, idea, &rec)?;
    Ok(out)
}

/// Forget conversation `conv`: its record entry and its folder. Claude's
/// transcript is Claude's file and stays. The caller closes a live session
/// first.
pub fn remove_at(root: &Path, idea: &str, conv: &str) -> Result<(), String> {
    check_conv(conv)?;
    ideas::refuse_real(root)?;
    let mut rec = read_record_at(root, idea)?;
    let before = rec.conversations.len();
    rec.conversations.retain(|c| c.id != conv);
    if rec.conversations.len() == before {
        return Err(format!("the idea {idea} has no conversation {conv}"));
    }
    write_record_at(root, idea, &rec)?;
    let dir = idea_desk(root, idea)?.join(conv);
    match fs::symlink_metadata(&dir) {
        // `remove_dir_all` removes a link inside without following it.
        Ok(md) if md.file_type().is_dir() => fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display())),
        Ok(_) => fs::remove_file(&dir).map_err(|e| format!("{}: {e}", dir.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

/// Where Claude keeps conversation `session_id`'s transcript for a session
/// started in `root`: `<claude root>/projects/<root, mangled>/<id>.jsonl`.
/// Claude mangles the directory it sees, which is the RESOLVED one (a link
/// in the path, macOS's `/tmp`), so the canonical path is asked first.
pub fn transcript_paths(claude_root: &Path, root: &Path, session_id: &str) -> Vec<PathBuf> {
    if !crate::codex::is_uuid(session_id) {
        return Vec::new();
    }
    let mut dirs = Vec::new();
    if let Ok(c) = fs::canonicalize(root) {
        dirs.push(c.to_string_lossy().into_owned());
    }
    let raw = root.to_string_lossy().into_owned();
    if !dirs.contains(&raw) {
        dirs.push(raw);
    }
    dirs.iter().map(|d| PathBuf::from(crate::project::claude_dir_in(claude_root, d)).join(format!("{session_id}.jsonl"))).collect()
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
    let mine = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    for r in roots {
        let theirs = fs::canonicalize(r).unwrap_or_else(|_| PathBuf::from(r));
        if mine.starts_with(&theirs) || theirs.starts_with(&mine) {
            return Err(format!(
                "the ideas folder {} is inside the registered project {r} (or holds it): conversations are not started there, \
                 because the project's places would take them for their own. Point {} at a folder outside every project.",
                root.display(),
                ideas::OVERRIDE_ENV
            ));
        }
    }
    Ok(())
}

// ── launching ───────────────────────────────────────────────────────────────

/// Whether Claude's `--help` lists `--session-id`: the floor a new
/// conversation needs, so its id can be chosen and recorded first.
pub fn help_has_session_id(help: &str) -> bool {
    help.split(|c: char| c.is_whitespace() || c == ',').any(|w| w == "--session-id")
}

/// Ask the installed Claude (`program`, the configured command's first word)
/// whether it takes `--session-id`. `--help` prints and exits: it starts no
/// session and runs no hook.
fn session_id_floor(program: &str, cwd: &Path) -> Result<(), String> {
    let mut cmd = crate::childenv::command(program);
    cmd.arg("--help").current_dir(cwd);
    let out = crate::proc::run_deadline(cmd, HELP_SECS).map_err(|e| format!("could not run `{program} --help`: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if help_has_session_id(&text) {
        Ok(())
    } else {
        Err(format!(
            "this Claude does not take --session-id (`{program} --help` does not list it), which an idea conversation needs \
             so its transcript can be found: update Claude Code"
        ))
    }
}

/// How a conversation is launched: fresh, on its opener, or resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Fresh,
    Resume,
}

/// The `pane0` that runs Claude for conversation `c` of `idea` in tmux
/// session `session`: the configured Claude command (`ai_cmd`, whose program
/// must be Claude), `--name`, the session id or its resume, the prompt
/// plugin, and on a fresh start the opener. The pane ends with Claude, so a
/// conversation whose Claude exits has no session: it reads as ended.
pub fn pane0(ai_cmd: &str, idea: &str, c: &Conversation, session: &str, how: Start, prompt_hook: Vec<String>) -> Result<String, String> {
    let line = launch_line(ai_cmd, idea, c, session, how, prompt_hook)?;
    Ok(format!("exec \"${{SHELL:-/bin/sh}}\" -ic {}", crate::tmux::sq(&format!("exec {line}"))))
}

/// [`pane0`]'s command line, before it is quoted for tmux: what the inner
/// shell runs.
pub fn launch_line(ai_cmd: &str, idea: &str, c: &Conversation, session: &str, how: Start, prompt_hook: Vec<String>) -> Result<String, String> {
    check_idea(idea)?;
    check_conv(&c.id)?;
    if !crate::codex::is_uuid(&c.session_id) {
        return Err(format!("conversation {} has no usable session id", c.id));
    }
    let adapter = crate::harness::for_cmd(ai_cmd).filter(|_| !ai_cmd.is_empty());
    if adapter.map(|a| a.provider().id) != Some(crate::provider::CLAUDE.id) {
        return Err(format!("idea conversations run Claude, and the configured command ({ai_cmd:?}) is not Claude"));
    }
    let mut ai = crate::profile::AiLaunch::plain(ai_cmd);
    ai.prompt_hook = prompt_hook;
    match how {
        Start::Fresh => {
            ai.session_id = Some(c.session_id.clone());
            ai.opener = Some(opener(idea, &c.id));
        }
        Start::Resume => {
            ai.resume = true;
            ai.resume_id = Some(c.session_id.clone());
        }
    }
    Ok(format!("{}{}", ai.shell_prefix(), ai.launch_cmd(session)))
}

/// Launch conversation `conv` of `idea` in `root`: refused while its session
/// runs, past the live-agent limit (a lane's words), below Claude's
/// `--session-id` floor (a fresh start), and for an ideas folder inside a
/// registered project. Answers the tmux session's name.
pub fn launch_at(root: &Path, idea: &str, conv: &str, how: Start) -> Result<String, String> {
    check_idea(idea)?;
    check_conv(conv)?;
    check_root(root, &crate::registry::read_lenient().roots())?;
    let c = find_at(root, idea, conv)?;
    let session = session_name(&c.id);
    if crate::tmux::session_exists(&session) {
        return Err(format!("conversation {conv} is already running (tmux session {session})"));
    }
    let ai_cmd = crate::config::resolve_ai_cmd(Some(crate::provider::CLAUDE.id));
    let program = ai_cmd.split_whitespace().next().unwrap_or("").to_string();
    if how == Start::Fresh {
        session_id_floor(&program, root)?;
    }
    let mut ai = crate::profile::AiLaunch::plain(&ai_cmd);
    let claude = crate::harness::CLAUDE;
    crate::quota::gate(claude, &ai, crate::sysclock::now_epoch()).map_err(refusal)?;
    let exclude = [session.clone()];
    let provider = Some(crate::provider::CLAUDE.id);
    crate::guard::admit(provider, false, &exclude).map_err(refusal)?;
    ai.prompt_hook = crate::ops::prompt_hook_for(&ai);
    let pane0 = pane0(&ai_cmd, idea, &c, &session, how, ai.prompt_hook)?;
    // The count that decides, under the machine's lock, held across
    // `new-session` so parallel launches count each other (`guard`).
    let _lock = crate::guard::launch_lock(|| {});
    crate::guard::admit(provider, false, &exclude).map_err(refusal)?;
    let dir = root.to_string_lossy().into_owned();
    crate::tmux::new_session(&session, &dir, &pane0)?;
    crate::tmux::tune_session(&session);
    Ok(session)
}

fn refusal(r: crate::harness::Refusal) -> String {
    match r {
        crate::harness::Refusal::Soft(w) | crate::harness::Refusal::Hard(w) => w,
    }
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
    let session = session_name(conv);
    if !crate::tmux::session_exists(&session) {
        return Ok(false);
    }
    crate::tmux::kill_session(&session);
    if crate::tmux::session_exists(&session) {
        return Err(format!("could not end tmux session {session}"));
    }
    Ok(true)
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
        assert!(pane.starts_with("exec \"${SHELL:-/bin/sh}\" -ic 'exec claude --name"), "{pane}");
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
