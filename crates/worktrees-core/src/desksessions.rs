//! Desk sessions: conversations with Claude that run in a tmux session of
//! their own and belong to no place. Two kinds: an idea's conversations
//! (`ideasessions`, `worktrees~idea~<conv>`) and a workspace's chats
//! (`workspacechats`, `worktrees~ws~<conv>`).
//!
//! What the two share is here, once: which tmux names are a desk session's
//! ([`is_desk_session`], what every reader of a PLACE's sessions asks), the
//! record of a set of conversations over where it lives ([`Desk`]), the
//! command line and the launch with its limits ([`Launch`]), and finding a
//! transcript by the session id the record chose. What differs (the folder,
//! the files a conversation opens on, its words) stays in each kind's module.

use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A record larger than this is refused rather than read.
const RECORD_MAX: u64 = 1024 * 1024;
/// How long `claude --help` may take before the floor is called unknown.
const HELP_SECS: u64 = 15;

/// Whether tmux session `name` is a desk session: an idea conversation's or a
/// workspace chat's, and so never a place's, wherever its pane is. `~` never
/// appears in a place's session (`<prefix>-<slug>`), so the marker alone
/// decides, as `~term` does for a dock shell. EVERY reader that looks for a
/// place's session or agent asks this one predicate.
pub fn is_desk_session(name: &str) -> bool {
    crate::ideasessions::is_idea_session(name) || crate::workspacechats::is_workspace_session(name)
}

// ── the record ──────────────────────────────────────────────────────────────

/// One conversation in a record.
pub trait Entry: Serialize + DeserializeOwned + Clone {
    /// Its ULID: names its folder and its tmux session.
    fn id(&self) -> &str;
    /// Claude's own id for it, a UUID chosen before the launch.
    fn session_id(&self) -> &str;
    fn set_title(&mut self, title: String);
}

/// A record file. Keys this build does not know are kept.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(bound(serialize = "C: Serialize", deserialize = "C: DeserializeOwned"))]
pub struct Record<C> {
    #[serde(default)]
    pub conversations: Vec<C>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

impl<C> Default for Record<C> {
    fn default() -> Record<C> {
        Record { conversations: Vec::new(), other: serde_json::Map::new() }
    }
}

/// Where one record lives, and what its conversations are called.
pub struct Desk {
    /// The store's root, which the real-write guard judges.
    pub root: PathBuf,
    /// The folders down to the record, outermost first; the last holds it.
    /// Each is a real directory this engine made; a link is refused.
    pub dirs: Vec<PathBuf>,
    /// The record's file name.
    pub file: &'static str,
    /// The folders down to the conversations' own, outermost first; the last
    /// holds them. Empty: the record's folder does. Real directories too.
    pub convs: Vec<PathBuf>,
    /// Whose conversations these are, for a refusal: `the idea <id>`.
    pub owner: String,
    /// What one is called: `conversation`, `chat`.
    pub noun: &'static str,
    /// The guard against a test writing the user's real folder.
    pub refuse_real: fn(&Path) -> Result<(), String>,
    /// A lock file beside the record, held across each read-modify-write.
    /// `None`: this process's mutex alone.
    pub lock: Option<&'static str>,
}

static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// A conversation id is a ULID before it names a path or reaches argv.
pub fn check_conv(noun: &str, conv: &str) -> Result<(), String> {
    if crate::ulid::is_valid(conv) { Ok(()) } else { Err(format!("not a {noun} id: {conv:?}")) }
}

impl Desk {
    /// The folder that holds the record.
    pub fn dir(&self) -> &Path {
        self.dirs.last().map_or(self.root.as_path(), PathBuf::as_path)
    }

    /// Conversation `conv`'s own folder. The caller checked the id.
    pub fn conv_dir(&self, conv: &str) -> PathBuf {
        self.convs.last().map_or(self.dir(), PathBuf::as_path).join(conv)
    }

    /// Make the folders down to the record, refusing a link in any of them.
    pub fn make_dirs(&self) -> Result<(), String> {
        (self.refuse_real)(&self.root)?;
        for d in &self.dirs {
            crate::fsx::make_dir(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        Ok(())
    }

    /// Make conversation `conv`'s folder and everything above it.
    pub fn make_conv_dir(&self, conv: &str) -> Result<PathBuf, String> {
        check_conv(self.noun, conv)?;
        self.make_dirs()?;
        let dir = self.conv_dir(conv);
        for d in &self.convs {
            crate::fsx::make_dir(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        crate::fsx::make_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(dir)
    }

    /// The record, or an empty one when there is none. A link where the
    /// record or its folders should be is refused: they are real directories
    /// and regular files this engine made. A hand-edited record that names
    /// anything but a conversation of ours is refused whole.
    pub fn read<C: Entry>(&self) -> Result<Record<C>, String> {
        for d in &self.dirs {
            match fs::symlink_metadata(d) {
                Ok(md) if md.file_type().is_dir() => {}
                Ok(_) => return Err(format!("{} is not a folder", d.display())),
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Record::default()),
                Err(e) => return Err(format!("{}: {e}", d.display())),
            }
        }
        let p = self.dir().join(self.file);
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
        let rec: Record<C> = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        for c in &rec.conversations {
            check_conv(self.noun, c.id())?;
            if !crate::codex::is_uuid(c.session_id()) {
                return Err(format!("{}: {} {} has no usable session id", p.display(), self.noun, c.id()));
            }
        }
        Ok(rec)
    }

    fn write<C: Entry>(&self, rec: &Record<C>) -> Result<(), String> {
        self.make_dirs()?;
        let mut text = serde_json::to_string_pretty(rec).map_err(|e| e.to_string())?;
        text.push('\n');
        crate::fsx::write_atomic(&self.dir().join(self.file), text.as_bytes(), None).map_err(|e| format!("{}: {e}", self.file))
    }

    /// Read, change, write, under the lock. `f` answering `Err` writes nothing.
    pub fn edit<C: Entry, T>(&self, f: impl FnOnce(&mut Record<C>) -> Result<T, String>) -> Result<T, String> {
        (self.refuse_real)(&self.root)?;
        let _serial = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _file = match self.lock {
            Some(name) => Some(self.flock(name)?),
            None => None,
        };
        let mut rec = self.read::<C>()?;
        let out = f(&mut rec)?;
        self.write(&rec)?;
        Ok(out)
    }

    /// The lock file, held until it is dropped. Never through a link.
    fn flock(&self, name: &str) -> Result<fs::File, String> {
        use std::os::unix::fs::OpenOptionsExt;
        self.make_dirs()?;
        let p = self.dir().join(name);
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&p)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        // SAFETY: flock gets a live descriptor; closing it releases the lock.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(format!("could not lock {}", p.display()));
        }
        Ok(lock)
    }

    fn missing(&self, conv: &str) -> String {
        format!("{} has no {} {conv}", self.owner, self.noun)
    }

    /// The conversation `conv`.
    pub fn find<C: Entry>(&self, conv: &str) -> Result<C, String> {
        check_conv(self.noun, conv)?;
        self.read::<C>()?.conversations.into_iter().find(|c| c.id() == conv).ok_or_else(|| self.missing(conv))
    }

    /// Its conversations, newest first.
    pub fn list<C: Entry>(&self) -> Result<Vec<C>, String> {
        let mut v = self.read::<C>()?.conversations;
        v.sort_by(|a, b| b.id().cmp(a.id()));
        Ok(v)
    }

    /// Change conversation `conv` in place and answer it as written.
    pub fn update<C: Entry>(&self, conv: &str, f: impl FnOnce(&mut C)) -> Result<C, String> {
        check_conv(self.noun, conv)?;
        self.edit(|rec: &mut Record<C>| {
            let c = rec.conversations.iter_mut().find(|c| c.id() == conv).ok_or_else(|| self.missing(conv))?;
            f(c);
            Ok(c.clone())
        })
    }

    /// Retitle conversation `conv`. A title is one line of at most 120
    /// characters.
    pub fn rename<C: Entry>(&self, conv: &str, title: &str) -> Result<C, String> {
        check_conv(self.noun, conv)?;
        let title: String = title.split(['\n', '\r']).next().unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(120).collect();
        if title.is_empty() {
            return Err(format!("a {} needs a title", self.noun));
        }
        self.update(conv, |c: &mut C| c.set_title(title))
    }

    /// Forget conversation `conv`: its record entry and its folder. Claude's
    /// transcript is Claude's file and stays. The caller closes a live
    /// session first.
    pub fn remove<C: Entry>(&self, conv: &str) -> Result<(), String> {
        check_conv(self.noun, conv)?;
        self.edit(|rec: &mut Record<C>| {
            let before = rec.conversations.len();
            rec.conversations.retain(|c| c.id() != conv);
            if rec.conversations.len() == before { Err(self.missing(conv)) } else { Ok(()) }
        })?;
        let dir = self.conv_dir(conv);
        match fs::symlink_metadata(&dir) {
            // `remove_dir_all` removes a link inside without following it.
            Ok(md) if md.file_type().is_dir() => fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display())),
            Ok(_) => fs::remove_file(&dir).map_err(|e| format!("{}: {e}", dir.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", dir.display())),
        }
    }
}

// ── small things both kinds use ─────────────────────────────────────────────

/// `YYYY-MM-DDTHH:MM:SSZ` for now.
pub fn now_iso() -> String {
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
    let mut paths: Vec<PathBuf> =
        dirs.iter().map(|d| PathBuf::from(crate::project::claude_dir_in(claude_root, d)).join(format!("{session_id}.jsonl"))).collect();
    // A conversation resumed from another folder keeps its file in the
    // folder it was born in (`agent::transcript_path`).
    if !paths.iter().any(|p| p.is_file()) {
        paths.extend(crate::agent::find_transcript(claude_root, session_id));
    }
    paths
}

/// The registered project among `roots` that `dir` is inside of, or that is
/// inside `dir`: a desk session's working directory is neither, because a
/// place adopts the sessions whose panes sit under it
/// (`Project::live_session_in`).
pub fn overlapping<'a>(dir: &Path, roots: &'a [String]) -> Option<&'a str> {
    let mine = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    roots.iter().map(String::as_str).find(|r| {
        let theirs = fs::canonicalize(r).unwrap_or_else(|_| PathBuf::from(r));
        mine.starts_with(&theirs) || theirs.starts_with(&mine)
    })
}

// ── launching ───────────────────────────────────────────────────────────────

/// Whether Claude's `--help` lists `--session-id`: the floor a new
/// conversation needs, so its id can be chosen and recorded first.
pub fn help_has_session_id(help: &str) -> bool {
    help.split(|c: char| c.is_whitespace() || c == ',').any(|w| w == "--session-id")
}

/// Ask the installed Claude (`program`, the configured command's first word)
/// whether it takes `--session-id`. `--help` prints and exits: it starts no
/// session and runs no hook. `what` is the kind, in a refusal.
fn session_id_floor(program: &str, cwd: &Path, what: &str) -> Result<(), String> {
    let mut cmd = crate::childenv::command(program);
    cmd.arg("--help").current_dir(cwd);
    let out = crate::proc::run_deadline(cmd, HELP_SECS).map_err(|e| format!("could not run `{program} --help`: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if help_has_session_id(&text) {
        Ok(())
    } else {
        Err(format!(
            "this Claude does not take --session-id (`{program} --help` does not list it), which {what} needs \
             so its transcript can be found: update Claude Code"
        ))
    }
}

/// How a conversation is launched: fresh, or resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Fresh,
    Resume,
}

/// One launch. Every string in it is the engine's own: ids it checked, fixed
/// text, and paths of its own folders.
pub struct Launch<'a> {
    /// The pane's working directory.
    pub cwd: &'a Path,
    /// The tmux session, which is also Claude's `--name`.
    pub session: &'a str,
    /// The conversation's ULID.
    pub conv: &'a str,
    /// Claude's id for it.
    pub session_id: &'a str,
    pub how: Start,
    /// The fixed opener, or none. A resume may carry one (a fold's does).
    pub opener: Option<String>,
    /// `--settings` for this session alone (`AiLaunch::session_settings`).
    pub settings: Option<String>,
    /// `KEY=VALUE` the session's Claude is started with (`AiLaunch::env`), and
    /// every process it starts inherits: a workspace chat's name, which is how
    /// its MCP server learns which chat it serves.
    pub env: Vec<(String, String)>,
    /// What one is called: `conversation`, `chat`.
    pub noun: &'a str,
    /// One of them in a refusal: `an idea conversation`.
    pub a_kind: &'a str,
    /// Several of them in a refusal: `idea conversations`.
    pub kinds: &'a str,
}

impl Launch<'_> {
    /// The command line, before it is quoted for tmux: what the inner shell
    /// runs. The configured Claude command (`ai_cmd`, whose program must be
    /// Claude), `--name`, the session id or its resume, the prompt plugin, the
    /// session's settings, and the opener.
    pub fn line(&self, ai_cmd: &str, prompt_hook: Vec<String>) -> Result<String, String> {
        let (prefix, cmd) = self.parts(ai_cmd, prompt_hook)?;
        Ok(format!("{prefix}{cmd}"))
    }

    /// [`Launch::line`] in its two halves: the `K=v ` assignments, and the
    /// command.
    fn parts(&self, ai_cmd: &str, prompt_hook: Vec<String>) -> Result<(String, String), String> {
        check_conv(self.noun, self.conv)?;
        if !crate::codex::is_uuid(self.session_id) {
            return Err(format!("{} {} has no usable session id", self.noun, self.conv));
        }
        let adapter = crate::harness::for_cmd(ai_cmd).filter(|_| !ai_cmd.is_empty());
        if adapter.map(|a| a.provider().id) != Some(crate::provider::CLAUDE.id) {
            return Err(format!("{} run Claude, and the configured command ({ai_cmd:?}) is not Claude", self.kinds));
        }
        let mut ai = crate::profile::AiLaunch::plain(ai_cmd);
        ai.prompt_hook = prompt_hook;
        ai.session_settings = self.settings.clone();
        ai.env = self.env.clone();
        ai.opener = self.opener.clone();
        match self.how {
            Start::Fresh => ai.session_id = Some(self.session_id.to_string()),
            Start::Resume => {
                ai.resume = true;
                ai.resume_id = Some(self.session_id.to_string());
            }
        }
        Ok((ai.shell_prefix(), ai.launch_cmd(self.session)))
    }

    /// The `pane0` that runs [`Launch::line`]. The pane ends with Claude, so a
    /// conversation whose Claude exits has no session: it reads as ended.
    ///
    /// The environment goes through `env`: `exec K=v claude` would exec a
    /// program named `K=v`, and the pane would end before Claude started.
    pub fn pane0(&self, ai_cmd: &str, prompt_hook: Vec<String>) -> Result<String, String> {
        let (prefix, cmd) = self.parts(ai_cmd, prompt_hook)?;
        let inner = if prefix.is_empty() { format!("exec {cmd}") } else { format!("exec env {prefix}{cmd}") };
        Ok(format!("exec \"${{SHELL:-/bin/sh}}\" -ic {}", crate::tmux::sq(&inner)))
    }

    /// Whether [`Launch::run`] would be refused for anything but its own
    /// session: the plan window and the live-agent limit, its own session not
    /// counted. Asked before that session is ended to be launched again.
    pub fn admit(&self) -> Result<(), String> {
        let ai = crate::profile::AiLaunch::plain(&crate::config::resolve_ai_cmd(Some(crate::provider::CLAUDE.id)));
        crate::quota::gate(crate::harness::CLAUDE, &ai, crate::sysclock::now_epoch()).map_err(refusal)?;
        crate::guard::admit(Some(crate::provider::CLAUDE.id), false, &[self.session.to_string()]).map_err(refusal).map(|_| ())
    }

    /// Launch it: refused while its session runs, past the live-agent limit
    /// (a lane's words), under a spent plan window, and, for a fresh start,
    /// below Claude's `--session-id` floor. The caller checked that `cwd` is
    /// no project's ([`overlapping`]).
    pub fn run(&self) -> Result<(), String> {
        check_conv(self.noun, self.conv)?;
        if crate::tmux::session_exists(self.session) {
            return Err(format!("{} {} is already running (tmux session {})", self.noun, self.conv, self.session));
        }
        let ai_cmd = crate::config::resolve_ai_cmd(Some(crate::provider::CLAUDE.id));
        let program = ai_cmd.split_whitespace().next().unwrap_or("").to_string();
        if self.how == Start::Fresh {
            session_id_floor(&program, self.cwd, self.a_kind)?;
        }
        let ai = crate::profile::AiLaunch::plain(&ai_cmd);
        let claude = crate::harness::CLAUDE;
        crate::quota::gate(claude, &ai, crate::sysclock::now_epoch()).map_err(refusal)?;
        let exclude = [self.session.to_string()];
        let provider = Some(crate::provider::CLAUDE.id);
        crate::guard::admit(provider, false, &exclude).map_err(refusal)?;
        let pane0 = self.pane0(&ai_cmd, crate::ops::prompt_hook_for(&ai))?;
        // The count that decides, under the machine's lock, held across
        // `new-session` so parallel launches count each other (`guard`).
        let _lock = crate::guard::launch_lock(|| {});
        crate::guard::admit(provider, false, &exclude).map_err(refusal)?;
        let dir = self.cwd.to_string_lossy().into_owned();
        crate::tmux::new_session(self.session, &dir, &pane0)?;
        crate::tmux::tune_session(self.session);
        Ok(())
    }
}

fn refusal(r: crate::harness::Refusal) -> String {
    match r {
        crate::harness::Refusal::Soft(w) | crate::harness::Refusal::Hard(w) => w,
    }
}

/// End tmux session `session` (`kill-session`). Answers whether it ran.
pub fn close(session: &str) -> Result<bool, String> {
    if !crate::tmux::session_exists(session) {
        return Ok(false);
    }
    crate::tmux::kill_session(session);
    if crate::tmux::session_exists(session) {
        return Err(format!("could not end tmux session {session}"));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A launch's environment reaches Claude through `env`, never as words
    /// after `exec`; with none, the line is as it was. Run, not read: the
    /// inner command is executed by `sh` with `claude` a script that prints
    /// what it was given.
    #[test]
    fn a_launch_with_an_environment_execs_claude_with_it() {
        let dir = std::env::temp_dir().join(format!("wt-desk-env-{}", crate::ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let conv = crate::ulid::new();
        let sid = new_uuid();
        let mut l = Launch { cwd: &dir, session: "worktrees~ws~x", conv: &conv, session_id: &sid, how: Start::Fresh, opener: None, settings: None, env: vec![("WT_PROBE".into(), "a b".into())], noun: "chat", a_kind: "a chat", kinds: "chats" };
        let pane = l.pane0("claude", vec![]).unwrap();
        // The inner `-ic` string, run by a plain sh with a fake claude.
        let inner = pane.split_once(" -ic ").unwrap().1;
        let fake = dir.join("claude");
        crate::fsx::exec_fixture(&fake, "#!/bin/sh\necho \"probe=$WT_PROBE\"\n");
        let out = crate::childenv::command("sh")
            .arg("-c")
            .arg(format!("eval {inner}"))
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "probe=a b", "{}", String::from_utf8_lossy(&out.stderr));
        assert!(pane.contains("exec env WT_PROBE="), "{pane}");
        l.env.clear();
        assert!(l.pane0("claude", vec![]).unwrap().contains(" -ic 'exec claude "), "no env: as before");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shared predicate covers both kinds, and no name a place, a dock
    /// shell or a provider sidecar has.
    #[test]
    fn a_desk_session_is_an_ideas_or_a_workspaces_and_never_a_places() {
        let conv = crate::ulid::new();
        assert!(is_desk_session(&crate::ideasessions::session_name(&conv)));
        assert!(is_desk_session(&crate::workspacechats::session_name(&conv)));
        for other in ["repo-feat-ws", "repo-(main)", "repo-feat~term", "repo-feat~term~2", "repo-feat~agent~codex", "ws", "worktrees-ws-x", ""] {
            assert!(!is_desk_session(other), "{other}");
        }
    }
}
