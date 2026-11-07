//! What Claude asked permission for, as its PermissionRequest hook saw it:
//! the whole of a prompt taller than its pane (`feat/actionable-prompts`, the
//! user's decision of 2026-10-31).
//!
//! Claude (2.1.314 and 2.1.316, measured) writes a tool call to its transcript
//! only once its permission is answered, so while the prompt is up this hook
//! is the one whole source of a diff or command that scrolled off. `worktrees
//! agent-hook permission-request`, in the `claude-prompt` plugin every Claude
//! launch gets (`guidance::materialize_prompt_in`, `ops::ai_launch_for`),
//! saves the tool and its input here; serve's `agent_prompt` shows it as
//! `detail` beside a prompt only when the screen agrees with it
//! (`prompt::Clipped::agrees`), and, with the hook's `cwd`, as `asked` on any
//! readable Claude prompt it agrees with (`prompt::call_agrees`).
//!
//! **The hook never decides.** It prints nothing and exits 0 whatever
//! happens: a worktrees bug must never become a Claude permission error. Its
//! command line ends `>/dev/null 2>&1 || true` for what this code cannot
//! reach (a missing or broken binary, a signal), and the launch adds it only
//! for a CLI that answers [`HOOK_VERSION`] ([`hook_bin`], `guard_bin`'s rule).
//!
//! **What it saves is the user's command text and file contents,** so it is
//! short-lived: `$XDG_STATE_HOME/worktrees/pending/<session id>.json`, 0600
//! in a 0700 directory, written with `fsx::write_atomic`; at most [`KEEP`]
//! records of at most [`MAX_INPUT`] bytes each and [`MAX_FILE`] in all;
//! removed by Claude's SessionEnd, by `agent_prompt` once no prompt is on the
//! session's screen, by an answer that used it, and by any later hook once
//! [`STALE_SECS`] old. Nothing here logs it, and no row audits it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The most prompts kept per session: a Claude that queues several
/// permissions (parallel tool calls, sub-agents) fires the hook for each.
pub const KEEP: usize = 4;
/// The most one tool input may take, as JSON. A larger one is not saved, and
/// its prompt stays `readable: false` when it does not fit the pane.
pub const MAX_INPUT: usize = 512 * 1024;
/// The most a session's file may take.
pub const MAX_FILE: usize = 1024 * 1024;
/// The most the hook reads from stdin; anything longer is drained unread.
const MAX_STDIN: usize = MAX_FILE + 64 * 1024;
/// A file nobody removed (a Claude that crashed before SessionEnd) goes this
/// long after its last write.
pub const STALE_SECS: u64 = 24 * 3600;
/// The most a plan's own file may take to be read ([`plan_file`]).
pub const MAX_PLAN: u64 = 256 * 1024;
/// What `worktrees agent-hook --version` prints first: the launch adds the
/// hook only for a CLI that has it ([`hook_bin`]).
pub const HOOK_VERSION: &str = "worktrees-agent-hook 1";

/// One permission prompt, as the hook saw it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Asked {
    /// Claude's tool name: `Edit`, `Write`, `Bash`, `ExitPlanMode`, …
    pub tool: String,
    /// The tool's whole input.
    pub input: Value,
    /// When the hook ran, in ms since the epoch.
    pub at: u64,
    /// The hook's `cwd`: where Claude was when it asked. Absent in a record
    /// an older hook wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    asked: Vec<Asked>,
}

/// The fields of a hook's stdin this reads.
#[derive(Deserialize)]
struct HookIn {
    session_id: String,
    hook_event_name: String,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<Value>,
    #[serde(default)]
    cwd: Option<String>,
}

/// `$XDG_STATE_HOME/worktrees/pending`.
pub fn dir() -> Option<PathBuf> {
    Some(crate::init::state_dir()?.join("pending"))
}

/// A session id this module will put in a file name: Claude's are UUIDs.
/// Anything else (a `/`, a `..`, nothing at all) is no session.
pub fn session_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn path_in(dir: &Path, session: &str) -> Option<PathBuf> {
    session_ok(session).then(|| dir.join(format!("{session}.json")))
}

/// `dir`, made 0700 if missing and kept 0700; refused when it is not a real
/// directory (a link included).
fn ensure(dir: &Path) -> Result<(), String> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let md = fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    if !md.is_dir() {
        return Err(format!("{}: not a directory", dir.display()));
    }
    if md.permissions().mode() & 0o777 != 0o700 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Read at most `max` bytes of a regular file that is not a link.
fn read_bounded(path: &Path, max: u64) -> Option<Vec<u8>> {
    let md = fs::symlink_metadata(path).ok()?;
    if !md.is_file() || md.len() > max {
        return None;
    }
    let mut buf = Vec::new();
    fs::File::open(path).ok()?.take(max + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= max).then_some(buf)
}

/// What the session's hook saved, oldest first. Empty when there is nothing,
/// or nothing that reads.
pub fn asked_in(dir: &Path, session: &str) -> Vec<Asked> {
    let Some(path) = path_in(dir, session) else { return Vec::new() };
    read_bounded(&path, MAX_FILE as u64)
        .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
        .map(|s| s.asked)
        .unwrap_or_default()
}

/// Write the session's records, or remove its file when there are none.
fn save(dir: &Path, session: &str, asked: &[Asked]) -> Result<(), String> {
    let path = path_in(dir, session).ok_or("not a session id")?;
    if asked.is_empty() {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    ensure(dir)?;
    let bytes = serde_json::to_vec(&Saved { asked: asked.to_vec() }).map_err(|e| e.to_string())?;
    crate::fsx::write_atomic(&path, &bytes, Some(fs::Permissions::from_mode(0o600))).map_err(|e| e.to_string())
}

/// The PermissionRequest hook's work: save `stdin`'s tool and input for its
/// session, keeping the newest [`KEEP`] within [`MAX_FILE`], then sweep files
/// older than [`STALE_SECS`]. Every refusal is an `Err` the caller drops.
pub fn record_in(dir: &Path, stdin: &[u8], now_ms: u64) -> Result<(), String> {
    let h: HookIn = serde_json::from_slice(stdin).map_err(|e| e.to_string())?;
    if h.hook_event_name != "PermissionRequest" {
        return Err(format!("not a permission request: {}", h.hook_event_name));
    }
    if !session_ok(&h.session_id) {
        return Err("not a session id".into());
    }
    let tool = h.tool_name.filter(|t| !t.is_empty() && t.len() <= 128).ok_or("no tool name")?;
    let input = h.tool_input.filter(Value::is_object).ok_or("no tool input")?;
    if serde_json::to_vec(&input).map_err(|e| e.to_string())?.len() > MAX_INPUT {
        return Err("the tool input is over the size this keeps".into());
    }
    let mut asked = asked_in(dir, &h.session_id);
    // A working directory is a path; anything else in its place is no cwd.
    let cwd = h.cwd.filter(|c| !c.is_empty() && c.len() <= 4096 && c.starts_with('/') && !c.chars().any(char::is_control));
    asked.push(Asked { tool, input, at: now_ms, cwd });
    let over = asked.len().saturating_sub(KEEP);
    asked.drain(..over);
    while asked.len() > 1 && serde_json::to_vec(&Saved { asked: asked.clone() }).map_or(0, |b| b.len()) > MAX_FILE {
        asked.remove(0);
    }
    save(dir, &h.session_id, &asked)?;
    sweep_in(dir, std::time::SystemTime::now());
    Ok(())
}

/// The SessionEnd hook's work: the session's file goes.
pub fn end_in(dir: &Path, stdin: &[u8]) -> Result<(), String> {
    let h: HookIn = serde_json::from_slice(stdin).map_err(|e| e.to_string())?;
    if h.hook_event_name != "SessionEnd" {
        return Err(format!("not a session end: {}", h.hook_event_name));
    }
    save(dir, &h.session_id, &[])
}

/// Forget one record of the session (an answer used it), or every record
/// older than `before_ms` (no prompt is on its screen any more). The file
/// goes with its last record.
pub fn forget_in(dir: &Path, session: &str, which: Forget) {
    let asked = asked_in(dir, session);
    let keep: Vec<Asked> = match which {
        Forget::One(one) => asked.iter().filter(|a| **a != one).cloned().collect(),
        Forget::Before(ms) => asked.iter().filter(|a| a.at >= ms).cloned().collect(),
    };
    if keep.len() != asked.len() {
        let _ = save(dir, session, &keep);
    }
}

/// Which records [`forget_in`] drops.
#[derive(Clone, Debug, PartialEq)]
pub enum Forget {
    One(Asked),
    Before(u64),
}

/// Whether any session has something saved: what lets a caller skip looking
/// up which session a pane runs when there is nothing to forget.
pub fn any_in(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut rd| rd.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".json"))))
}

/// Remove the files no hook or reader has touched for [`STALE_SECS`].
fn sweep_in(dir: &Path, now: std::time::SystemTime) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten().take(1024) {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(session) = name.strip_suffix(".json").filter(|s| session_ok(s)) else { continue };
        let old = fs::symlink_metadata(e.path())
            .and_then(|m| m.modified())
            .is_ok_and(|t| now.duration_since(t).is_ok_and(|d| d.as_secs() > STALE_SECS));
        if old {
            let _ = save(dir, session, &[]);
        }
    }
}

/// `worktrees agent-hook <permission-request | session-end | --version>`:
/// Claude's hooks from the `claude-prompt` plugin. Prints nothing (but the
/// version) and returns 0 whatever happens: no decision, no error, no panic.
pub fn cmd_agent_hook(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("--version") => println!("{HOOK_VERSION}"),
        Some(event @ ("permission-request" | "session-end")) => {
            std::panic::set_hook(Box::new(|_| {}));
            let _ = std::panic::catch_unwind(|| hook(event, dir().as_deref(), &mut std::io::stdin()));
        }
        _ => {}
    }
    0
}

/// One hook run, its result dropped: what `cmd_agent_hook` does, testable.
pub fn hook(event: &str, dir: Option<&Path>, stdin: &mut dyn Read) {
    let mut input = Vec::new();
    let read = stdin.take(MAX_STDIN as u64 + 1).read_to_end(&mut input);
    if read.is_err() || input.len() > MAX_STDIN {
        // Drained, so Claude's write never meets a closed pipe; not kept.
        let _ = std::io::copy(stdin, &mut std::io::sink());
        return;
    }
    let Some(dir) = dir else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let _ = match event {
        "permission-request" => record_in(dir, &input, now),
        _ => end_in(dir, &input),
    };
}

/// The `worktrees` the hook's command would run — only one that HAS the
/// hook (`agent-hook --version` answers [`HOOK_VERSION`]'s name), so an
/// older CLI on PATH never meets an argument it does not know. Run from `/`.
pub fn hook_bin() -> Option<PathBuf> {
    let bin = crate::profile::worktrees_bin()?;
    let mut cmd = crate::childenv::command(&bin);
    cmd.args(["agent-hook", "--version"]).current_dir("/").stdin(std::process::Stdio::null());
    let out = crate::proc::run_deadline(cmd, 5).ok()?;
    let name = HOOK_VERSION.split(' ').next().unwrap_or(HOOK_VERSION);
    let first = String::from_utf8_lossy(&out.stdout).lines().next().map(str::to_string);
    (out.status.success() && first.is_some_and(|l| l.split(' ').next() == Some(name))).then_some(bin)
}

/// The plan file a plan approval's footer names (`~/.claude/plans/<name>.md`),
/// for a session whose hook saved no plan (one launched before the hook):
/// `(its path, its text)`. Read-only and bounded ([`MAX_PLAN`]), and only a
/// regular file directly under `~/.claude/plans/` — a name of one component,
/// no link, its canonical path under the canonical plans directory
/// (`safepath::safe_under`).
pub fn plan_file(shown: &str) -> Option<(String, String)> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    plan_file_in(Path::new(&home), shown)
}

pub fn plan_file_in(home: &Path, shown: &str) -> Option<(String, String)> {
    let name = shown.strip_prefix("~/.claude/plans/")?;
    if name.contains('/') || !name.ends_with(".md") {
        return None;
    }
    let path = crate::safepath::safe_under(&home.join(".claude").join("plans"), name)?;
    let text = String::from_utf8(read_bounded(&path, MAX_PLAN)?).ok()?;
    Some((path.to_string_lossy().into_owned(), text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    macro_rules! hook_fx {
        ($name:literal) => {
            include_bytes!(concat!("../tests/fixtures/claude-screen/2.1.316/", $name))
        };
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(tag: &str) -> Scratch {
        let d = std::env::temp_dir().join(format!("wtpending-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Scratch(d)
    }

    /// The hook's `cwd` is kept beside the call (`asked.cwd`): a path, or
    /// nothing — and a record an older hook wrote reads without it.
    #[test]
    fn the_hooks_working_directory_is_kept_when_it_is_a_path() {
        let d = scratch("cwd");
        record_in(d.path(), hook_fx!("hook-bash.json"), 1).unwrap();
        let sid = "dde2366a-1223-4dc0-9584-dd8aa90ffc11";
        assert_eq!(asked_in(d.path(), sid)[0].cwd.as_deref(), Some("/private/tmp/claude-501/wtph/repo"));
        let mut odd: serde_json::Value = serde_json::from_slice(hook_fx!("hook-bash.json")).unwrap();
        for bad in ["relative/dir", "", "/tmp/x\nforged"] {
            odd["cwd"] = bad.into();
            record_in(d.path(), &serde_json::to_vec(&odd).unwrap(), 2).unwrap();
            assert_eq!(asked_in(d.path(), sid).last().unwrap().cwd, None, "{bad:?}");
        }
        odd.as_object_mut().unwrap().remove("cwd");
        record_in(d.path(), &serde_json::to_vec(&odd).unwrap(), 3).unwrap();
        assert_eq!(asked_in(d.path(), sid).last().unwrap().cwd, None);
        // A file from before `cwd` existed.
        fs::write(d.path().join(format!("{sid}.json")), br#"{"asked":[{"tool":"Bash","input":{"command":"ls"},"at":1}]}"#).unwrap();
        assert_eq!(asked_in(d.path(), sid).len(), 1);
    }

    fn mode(p: &Path) -> u32 {
        fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
    }

    const SID: &str = "dde2366a-1223-4dc0-9584-dd8aa90ffc11";

    fn request(session: &str, tool: &str, input: Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "session_id": session, "hook_event_name": "PermissionRequest", "tool_name": tool, "tool_input": input,
        }))
        .unwrap()
    }

    /// Claude 2.1.316's own PermissionRequest stdin: its tool and its whole
    /// input, keyed by its session, 0600 in a 0700 directory made for it.
    #[test]
    fn a_permission_request_is_kept_whole_and_private_by_session() {
        let t = scratch("kept");
        let dir = t.path().join("state/worktrees/pending");
        record_in(&dir, hook_fx!("hook-edit.json"), 1_000).unwrap();
        let got = asked_in(&dir, SID);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].tool, "Edit");
        let fx: Value = serde_json::from_slice(hook_fx!("hook-edit.json")).unwrap();
        assert_eq!(got[0].input, fx["tool_input"], "the whole input");
        assert_eq!(got[0].at, 1_000);
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(format!("{SID}.json"))), 0o600);
        assert!(any_in(&dir));
        assert!(asked_in(&dir, "another-session").is_empty());

        record_in(&dir, hook_fx!("hook-exit-plan.json"), 2_000).unwrap();
        assert_eq!(asked_in(&dir, SID).iter().map(|a| a.tool.as_str()).collect::<Vec<_>>(), vec!["Edit", "ExitPlanMode"]);
        // Its SessionEnd removes the file.
        end_in(&dir, hook_fx!("hook-session-end.json")).unwrap();
        assert!(asked_in(&dir, SID).is_empty());
        assert!(!dir.join(format!("{SID}.json")).exists());
        assert!(!any_in(&dir));
    }

    /// Bounded: the newest KEEP records, an input over MAX_INPUT never
    /// kept, and the file within MAX_FILE by dropping the oldest.
    #[test]
    fn a_session_keeps_its_last_few_prompts_within_bounds() {
        let t = scratch("bounds");
        let dir = t.path().join("pending");
        for i in 0..6u64 {
            record_in(&dir, &request(SID, "Bash", serde_json::json!({ "command": format!("echo {i}") })), i).unwrap();
        }
        assert_eq!(asked_in(&dir, SID).iter().map(|a| a.at).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        let huge = "x".repeat(MAX_INPUT + 1);
        assert!(record_in(&dir, &request(SID, "Write", serde_json::json!({ "content": huge })), 9).is_err());
        assert_eq!(asked_in(&dir, SID).len(), 4, "nothing dropped for a refused input");
        let big = "y".repeat(MAX_INPUT - 64);
        for i in 10..13u64 {
            record_in(&dir, &request(SID, "Write", serde_json::json!({ "content": big })), i).unwrap();
        }
        let kept = asked_in(&dir, SID);
        assert!(fs::metadata(dir.join(format!("{SID}.json"))).unwrap().len() as usize <= MAX_FILE);
        assert_eq!(kept.last().map(|a| a.at), Some(12), "the newest is always kept");
        assert!(kept.len() < KEEP);
    }

    /// A session id is never a path, a file over its bound is never read, and
    /// a link where the file belongs is never followed.
    #[test]
    fn a_session_id_is_never_a_path_and_a_file_is_never_followed() {
        let t = scratch("path");
        let dir = t.path().join("pending");
        for bad in ["", "../x", "a/b", ".hidden", &"a".repeat(65)] {
            assert!(record_in(&dir, &request(bad, "Bash", serde_json::json!({"command": "ls"})), 1).is_err(), "{bad:?}");
            assert!(asked_in(&dir, bad).is_empty());
        }
        assert!(!t.path().join("x.json").exists());
        fs::create_dir_all(&dir).unwrap();
        let elsewhere = t.path().join("elsewhere.json");
        fs::write(&elsewhere, br#"{"asked":[{"tool":"Bash","input":{"command":"ls"},"at":1}]}"#).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join(format!("{SID}.json"))).unwrap();
        assert!(asked_in(&dir, SID).is_empty(), "a link is not this session's file");
    }

    /// The hook never decides and never fails: bad stdin, another event, no
    /// state dir, a state dir that is a file, a stdin past its bound — each is
    /// nothing saved, no panic, and the caller (`cmd_agent_hook`) returns 0.
    #[test]
    fn the_hook_swallows_every_failure() {
        let t = scratch("swallow");
        let dir = t.path().join("pending");
        let run = |event: &str, dir: Option<&Path>, input: &[u8]| hook(event, dir, &mut std::io::Cursor::new(input.to_vec()));
        run("permission-request", Some(&dir), b"not json");
        run("permission-request", Some(&dir), hook_fx!("hook-session-end.json"));
        run("session-end", Some(&dir), hook_fx!("hook-edit.json"));
        run("permission-request", None, hook_fx!("hook-edit.json"));
        assert!(!dir.exists(), "nothing was written");
        let file = t.path().join("a-file");
        fs::write(&file, "").unwrap();
        run("permission-request", Some(&file.join("pending")), hook_fx!("hook-edit.json"));
        let mut long = br#"{"session_id":"x","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":""#.to_vec();
        long.resize(MAX_STDIN + 10, b'a');
        run("permission-request", Some(&dir), &long);
        assert!(!dir.exists());
        // And it does save when it can.
        run("permission-request", Some(&dir), hook_fx!("hook-bash.json"));
        assert_eq!(asked_in(&dir, SID).len(), 1);
        assert_eq!(cmd_agent_hook(&["nonsense".into()]), 0);
    }

    /// Forgetting: one record an answer used, everything older than a
    /// moment, a stale file on the next hook — and the file with its last
    /// record.
    #[test]
    fn records_are_forgotten_once_their_prompt_is_gone() {
        let t = scratch("forget");
        let dir = t.path().join("pending");
        for (i, c) in ["a", "b", "c"].iter().enumerate() {
            record_in(&dir, &request(SID, "Bash", serde_json::json!({ "command": c })), 100 * (i as u64 + 1)).unwrap();
        }
        let b = asked_in(&dir, SID)[1].clone();
        forget_in(&dir, SID, Forget::One(b));
        assert_eq!(asked_in(&dir, SID).iter().map(|a| a.at).collect::<Vec<_>>(), vec![100, 300]);
        forget_in(&dir, SID, Forget::Before(200));
        assert_eq!(asked_in(&dir, SID).iter().map(|a| a.at).collect::<Vec<_>>(), vec![300]);
        forget_in(&dir, SID, Forget::Before(301));
        assert!(!dir.join(format!("{SID}.json")).exists());

        let other = "0b0b0b0b-0000-0000-0000-000000000000";
        record_in(&dir, &request(other, "Bash", serde_json::json!({ "command": "old" })), 1).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(STALE_SECS + 60);
        fs::File::options().write(true).open(dir.join(format!("{other}.json"))).unwrap().set_modified(old).unwrap();
        record_in(&dir, &request(SID, "Bash", serde_json::json!({ "command": "new" })), 2).unwrap();
        assert!(asked_in(&dir, other).is_empty(), "swept");
        assert_eq!(asked_in(&dir, SID).len(), 1);
    }

    /// The plan-file fallback reads one regular file directly under
    /// `~/.claude/plans/`, bounded — never a path that leaves it.
    #[test]
    fn a_plan_file_is_read_only_from_claudes_plans_and_bounded() {
        let t = scratch("plan");
        let home = t.path().join("home");
        let plans = home.join(".claude/plans");
        fs::create_dir_all(&plans).unwrap();
        fs::write(plans.join("a-plan.md"), "# Plan\n1. one\n").unwrap();
        fs::write(home.join("secret.md"), "secret").unwrap();
        let (path, text) = plan_file_in(&home, "~/.claude/plans/a-plan.md").expect("the plan");
        assert_eq!(text, "# Plan\n1. one\n");
        assert!(path.ends_with("/.claude/plans/a-plan.md"), "{path}");
        for bad in [
            "~/.claude/plans/../../secret.md",
            "~/.claude/plans/sub/a-plan.md",
            "~/.claude/plans/a-plan.txt",
            "/etc/passwd",
            "~/secret.md",
            "~/.claude/plans/missing.md",
        ] {
            assert!(plan_file_in(&home, bad).is_none(), "{bad}");
        }
        std::os::unix::fs::symlink(home.join("secret.md"), plans.join("linked.md")).unwrap();
        assert!(plan_file_in(&home, "~/.claude/plans/linked.md").is_none(), "a link is not followed");
        fs::write(plans.join("huge.md"), "z".repeat(MAX_PLAN as usize + 1)).unwrap();
        assert!(plan_file_in(&home, "~/.claude/plans/huge.md").is_none(), "bounded");
    }
}
