//! Small, read-only Codex session check for deciding whether `resume --last`
//! has a conversation in this exact worktree. Codex owns these files.

use std::io::{BufRead, BufReader};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn sessions_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex"))
        .join("sessions")
}

/// Rollout files live under `sessions/<year>/<month>/<day>/`. Read only the
/// first JSONL record (`session_meta`) of each file, never transcript content.
pub fn session_present(cwd: &str) -> bool {
    fn walk(dir: &Path, depth: u8, cwd: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else { return false };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 3 {
                if walk(&path, depth + 1, cwd) { return true; }
            } else if depth == 3 && path.extension().is_some_and(|x| x == "jsonl") {
                let Ok(file) = std::fs::File::open(path) else { continue };
                let mut first = String::new();
                if BufReader::new(file).read_line(&mut first).is_ok() {
                    let meta = serde_json::from_str::<serde_json::Value>(&first).ok();
                    if meta.as_ref().and_then(|v| v.pointer("/payload/cwd")).and_then(|v| v.as_str()) == Some(cwd) {
                        return true;
                    }
                }
            }
        }
        false
    }
    walk(&sessions_dir(), 0, cwd)
}

/// Day directories to search, newest first, when looking for a place's rollout.
/// A codex session left open for longer than this keeps writing into the file
/// it STARTED in, so its model simply goes unshown — the cheap failure, versus
/// listing every day dir ever written on a 3s poll.
const ROLLOUT_DAYS: usize = 14;

/// Whether a rollout's `session_meta` payload is a session the user is TALKING
/// to in `cwd`. Codex also writes rollouts for its own sub-agents (the
/// auto-review "guardian": `source: {subagent: …}`, `parent_thread_id` set) in
/// the same cwd, and those start AFTER the session that spawned them — so
/// without this the newest rollout is routinely the reviewer, and the label
/// names `codex-auto-review` instead of the model the user chose.
fn is_user_thread(meta: &serde_json::Value, cwd: &str) -> bool {
    meta.get("cwd").and_then(|c| c.as_str()) == Some(cwd)
        && meta.get("parent_thread_id").is_none_or(|p| p.is_null())
        && meta.get("source").and_then(|s| s.get("subagent")).is_none()
}

/// The newest rollout file for a user thread in `cwd` (see `is_user_thread`),
/// looking back `ROLLOUT_DAYS` day dirs. Rollout names begin with their start
/// time, so the first match in descending name order is the newest session.
pub fn latest_rollout(cwd: &str) -> Option<PathBuf> {
    fn sorted_desc(dir: &Path, dirs: bool) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut v: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() == dirs).collect();
        v.sort_by(|a, b| b.cmp(a));
        v
    }
    let mut days = Vec::new();
    'walk: for year in sorted_desc(&sessions_dir(), true) {
        for month in sorted_desc(&year, true) {
            for day in sorted_desc(&month, true) {
                days.push(day);
                if days.len() == ROLLOUT_DAYS {
                    break 'walk;
                }
            }
        }
    }
    days.iter().flat_map(|d| sorted_desc(d, false)).find(|path| {
        path.extension().is_some_and(|x| x == "jsonl") && user_thread_cwd(path).as_deref() == Some(cwd)
    })
}

/// Per rollout: the cwd of the user thread it records, or `None` for a
/// sub-agent's (or a first line that does not parse). A rollout's first line
/// never changes once written, and it carries codex's whole base prompt (20KB+),
/// so it is read once per file rather than once per lookup on the poll.
static THREAD_CWD: Mutex<Option<HashMap<PathBuf, Option<String>>>> = Mutex::new(None);

fn user_thread_cwd(path: &Path) -> Option<String> {
    let mut guard = THREAD_CWD.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(v) = cache.get(path) {
        return v.clone();
    }
    let mut first = String::new();
    BufReader::new(std::fs::File::open(path).ok()?).read_line(&mut first).ok()?;
    // Codex creates the file a few seconds before it finishes writing the
    // first line. A partial line is "not yet", not "not a match" — caching
    // it would hide the session it is about to become.
    if !first.ends_with('\n') {
        return None;
    }
    let cwd = serde_json::from_str::<serde_json::Value>(&first).ok().and_then(|v| {
        let meta = v.get("payload")?;
        let cwd = meta.get("cwd")?.as_str()?;
        is_user_thread(meta, cwd).then(|| cwd.to_string())
    });
    cache.insert(path.to_path_buf(), cwd.clone());
    cwd
}

/// The model named by the newest `turn_context` or `thread_settings_applied`
/// among rollout `lines` (oldest first). A turn's context carries the model it
/// ran on; the settings event is written the moment `/model` is used, before
/// any turn, so a switch shows without waiting for the next message.
pub fn rollout_model(lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        let m = match v.get("type")?.as_str()? {
            "turn_context" => v.pointer("/payload/model")?,
            "event_msg" if v.pointer("/payload/type")?.as_str()? == "thread_settings_applied" => {
                v.pointer("/payload/thread_settings/model")?
            }
            _ => return None,
        };
        let m = m.as_str()?;
        (!m.is_empty()).then(|| m.to_string())
    })
}

// ── permissions ─────────────────────────────────────────────────────────────

/// How much a Worktrees-launched Codex may do without asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permissions {
    /// Codex's own default: it asks.
    Ask,
    /// `--approve-for-me`: a reviewer decides each request, inside the
    /// workspace-write sandbox. The default — the analogue of Claude's auto mode.
    AutoReview,
    /// `--dangerously-bypass-approvals-and-sandbox`.
    Full,
}

impl Permissions {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "ask" => Some(Self::Ask),
            "auto-review" => Some(Self::AutoReview),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::AutoReview => "auto-review",
            Self::Full => "full",
        }
    }
}

/// The app's setting, pushed in-process (the app links this crate). Wins over
/// env and config so a Settings change applies to the very next launch.
static PERMISSIONS_OVERRIDE: Mutex<Option<Permissions>> = Mutex::new(None);

pub fn set_permissions_override(p: Option<Permissions>) {
    *PERMISSIONS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = p;
}

/// app setting > `$WORKTREES_CODEX_PERMISSIONS` > `codex_permissions` user
/// config > auto-review. An unknown value falls through rather than failing a
/// launch.
pub fn permissions() -> Permissions {
    if let Some(p) = *PERMISSIONS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
        return p;
    }
    let env = std::env::var("WORKTREES_CODEX_PERMISSIONS").ok();
    let cfg = crate::config::user_cfg("codex_permissions");
    permissions_from(env.as_deref(), cfg.as_deref())
}

pub fn permissions_from(env: Option<&str>, cfg: Option<&str>) -> Permissions {
    env.and_then(Permissions::parse)
        .or_else(|| cfg.and_then(Permissions::parse))
        .unwrap_or(Permissions::AutoReview)
}

/// Codex arguments (already shell-quoted) for `mode` in a worktree whose git
/// common dir is `git_common`.
///
/// Auto-review needs two holes in the workspace-write sandbox, both measured
/// live (codex exec, 0.157.1) in a linked worktree:
/// - the git COMMON dir must be writable, or `git add`/`commit` die on
///   `<main>/.git/worktrees/<name>/index.lock` ("Operation not permitted") —
///   a linked worktree's index and every object live outside it;
/// - network, or `git push`/`gh` cannot resolve github.com.
/// Without a common dir (not a repo?) the flag is omitted rather than guessed.
pub fn permission_flags(mode: Permissions, git_common: Option<&str>) -> Vec<String> {
    let q = crate::profile::shell_quote;
    match mode {
        Permissions::Ask => Vec::new(),
        Permissions::Full => vec!["--dangerously-bypass-approvals-and-sandbox".into()],
        Permissions::AutoReview => {
            let mut v = vec!["--approve-for-me".to_string()];
            if let Some(dir) = git_common.filter(|d| !d.is_empty()) {
                let toml = format!("sandbox_workspace_write.writable_roots=[{}]", toml_str(dir));
                v.push("-c".into());
                v.push(q(&toml));
            }
            v.push("-c".into());
            v.push("sandbox_workspace_write.network_access=true".into());
            v
        }
    }
}

/// A TOML basic string.
fn toml_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The repo's git common dir as a real path (the sandbox compares real paths:
/// on macOS `/tmp` is `/private/tmp`).
pub fn git_common_dir(wt: &str) -> Option<String> {
    let raw = crate::git::git_out(wt, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let p = std::fs::canonicalize(raw.trim()).ok()?;
    Some(p.to_string_lossy().into_owned())
}

/// Everything `ops::ai_launch_for` adds to a Codex launch in `wt`.
pub fn launch_flags(wt: &str) -> Vec<String> {
    let mode = permissions();
    let common = if mode == Permissions::AutoReview { git_common_dir(wt) } else { None };
    permission_flags(mode, common.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shapes copied from real session_meta lines: codex's auto-reviewer shares
    /// the cwd and starts later, so it must not be taken for the user's thread.
    #[test]
    fn a_subagent_rollout_is_not_the_users_thread() {
        let user = serde_json::json!({"cwd":"/w","source":"cli","thread_source":"user","parent_thread_id":null});
        let vscode = serde_json::json!({"cwd":"/w","source":"vscode"});
        let guardian = serde_json::json!({"cwd":"/w","source":{"subagent":{"other":"guardian"}},
            "thread_source":"guardian_review","parent_thread_id":"01a0cc95"});
        assert!(is_user_thread(&user, "/w"));
        assert!(is_user_thread(&vscode, "/w"));
        assert!(!is_user_thread(&guardian, "/w"));
        assert!(!is_user_thread(&user, "/elsewhere"));
    }

    /// Codex creates the rollout, then writes its 20KB first line a few seconds
    /// later. A lookup in that gap must not be remembered as "not this place".
    #[test]
    fn a_half_written_first_line_is_asked_again() {
        let p = std::env::temp_dir().join(format!("wt-rollout-{}.jsonl", std::process::id()));
        let line = r#"{"type":"session_meta","payload":{"cwd":"/w","source":"cli"}}"#;
        std::fs::write(&p, &line[..20]).unwrap();
        assert_eq!(user_thread_cwd(&p), None);
        std::fs::write(&p, format!("{line}\n")).unwrap();
        assert_eq!(user_thread_cwd(&p).as_deref(), Some("/w"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn rollout_model_is_the_last_turn_context() {
        let lines: Vec<String> = [
            r#"{"type":"session_meta","payload":{"cwd":"/w"}}"#,
            r#"{"type":"turn_context","payload":{"model":"gpt-6"}}"#,
            r#"{"type":"turn_context","payload":{"model":"gpt-6-astra"}}"#,
            r#"{"type":"event_msg","payload":{"model":"nope"}}"#,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(rollout_model(&lines).as_deref(), Some("gpt-6-astra"));
        assert_eq!(rollout_model(&lines[..1]), None);
    }

    /// Shape from a real mid-session `/model`: the settings event lands at the
    /// switch, 42s before the next turn's context in the sampled rollout.
    #[test]
    fn a_model_switch_shows_before_the_next_turn() {
        let lines: Vec<String> = [
            r#"{"type":"turn_context","payload":{"model":"gpt-6-astra"}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#,
            r#"{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"model":"gpt-6-sol","model_provider_id":"openai"}}}"#,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(rollout_model(&lines).as_deref(), Some("gpt-6-sol"));
    }

    #[test]
    fn permissions_resolve_env_then_config_then_auto_review() {
        assert_eq!(permissions_from(None, None), Permissions::AutoReview);
        assert_eq!(permissions_from(None, Some("ask")), Permissions::Ask);
        assert_eq!(permissions_from(Some("full"), Some("ask")), Permissions::Full);
        // garbage falls through, it never fails a launch
        assert_eq!(permissions_from(Some("yolo"), Some("ask")), Permissions::Ask);
        assert_eq!(permissions_from(Some(""), None), Permissions::AutoReview);
    }

    #[test]
    fn auto_review_opens_the_git_common_dir_and_network() {
        assert_eq!(permission_flags(Permissions::Ask, Some("/r/.git")), Vec::<String>::new());
        assert_eq!(permission_flags(Permissions::Full, None), vec!["--dangerously-bypass-approvals-and-sandbox"]);
        assert_eq!(
            permission_flags(Permissions::AutoReview, Some("/r/.git")).join(" "),
            r#"--approve-for-me -c 'sandbox_workspace_write.writable_roots=["/r/.git"]' -c sandbox_workspace_write.network_access=true"#
        );
        // no common dir → no guessed root, still network
        assert_eq!(
            permission_flags(Permissions::AutoReview, None).join(" "),
            "--approve-for-me -c sandbox_workspace_write.network_access=true"
        );
        // a quote in the path survives both TOML and the shell
        assert_eq!(
            permission_flags(Permissions::AutoReview, Some("/a'b\"c/.git"))[2],
            r#"'sandbox_workspace_write.writable_roots=["/a'\''b\"c/.git"]'"#
        );
    }
}
