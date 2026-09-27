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
///
/// `codex exec` runs are excluded for the same reason: `source: "exec"`, no
/// parent, same cwd. A script or an agent running `codex exec` in the worktree
/// would otherwise become "the newest session" and hijack the dot, the
/// worked stamp and the label — and stay newest long after it exited.
///
/// Still ambiguous: two INTERACTIVE sessions in one cwd (a second `codex`
/// opened by hand beside the managed one). The newer one wins, whichever the
/// user is looking at; one provider session per place is the app's rule, and
/// this does not try to enforce it.
fn is_user_thread(meta: &serde_json::Value, cwd: &str) -> bool {
    let source = meta.get("source");
    meta.get("cwd").and_then(|c| c.as_str()) == Some(cwd)
        && meta.get("parent_thread_id").is_none_or(|p| p.is_null())
        && source.and_then(|s| s.get("subagent")).is_none()
        && source.and_then(|s| s.as_str()) != Some("exec")
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

/// Where a codex thread's CURRENT turn stands, per the newest turn-boundary
/// record in its rollout. Codex writes no status file, but since 0.15x every
/// turn is bracketed in the rollout: `task_started` when it begins, then
/// `task_complete` (carrying `completed_at`) or `turn_aborted` (Esc). Measured
/// on 0.157.1, 2026-09-26 (`findings.md`): legacy `notify` reports only the
/// completion and nothing at all for an interrupt, so the rollout is the one
/// source that can end a busy turn either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Turn {
    /// A turn is running — possibly parked on an approval prompt, which
    /// nothing codex writes tells apart from a command that is simply running.
    Busy,
    /// The last turn finished at `at` (epoch seconds, codex's own
    /// `completed_at`, else the record's own RFC3339 `timestamp` — CONTENT
    /// either way, never the file's mtime, which a live session keeps moving).
    /// `None` only when the record carries neither.
    Done { at: Option<i64>, turn_id: Option<String> },
    /// The last turn was interrupted. Not work to be told about: the user was
    /// at the prompt when it stopped.
    Aborted,
}

/// The state of the newest turn among rollout `lines` (oldest first), or
/// `None` when the lines hold no turn boundary at all — a fresh session, or a
/// tail filled by one enormous tool result, which the caller treats as "no new
/// answer" rather than "idle". Reads only record TYPES and codex's own
/// timestamps/ids; `last_agent_message` is content and is never looked at.
pub fn rollout_turn(lines: &[String]) -> Option<Turn> {
    lines.iter().rev().find_map(|l| {
        // Cheap reject before a parse: most lines are token counts and items.
        if !(l.contains("\"task_") || l.contains("\"turn_aborted\"")) {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v.get("type")?.as_str()? != "event_msg" {
            return None;
        }
        let p = v.get("payload")?;
        match p.get("type")?.as_str()? {
            "task_started" => Some(Turn::Busy),
            "task_complete" => Some(Turn::Done {
                at: p.get("completed_at").and_then(|c| c.as_i64()).or_else(|| {
                    v.get("timestamp").and_then(|t| t.as_str()).and_then(crate::sysclock::parse_iso8601)
                }),
                turn_id: p.get("turn_id").and_then(|t| t.as_str()).map(str::to_string),
            }),
            "turn_aborted" => Some(Turn::Aborted),
            _ => None,
        }
    })
}

/// The footer Codex's approval modals end on — a command, a file edit, a
/// permission grant — captured from the 0.157.1 TUI (`findings.md`).
const APPROVAL_FOOTER: &str = "Press enter to confirm or esc to cancel";
/// The footers of a plan-mode `request_user_input` modal: one question, or
/// the last of several (the key-hint strings in the 0.157.1 binary).
const QUESTION_FOOTERS: [&str; 2] = ["to submit answer", "to submit all"];

/// Whether a captured Codex pane is sitting on a modal that waits for the
/// user: an approval (command, edits, permissions) or a plan-mode question.
/// Nothing Codex writes to disk says so — the rollout's last record is the
/// same pending tool call whether the command is running or awaiting a yes,
/// and legacy `notify` has no approval event — so the screen is the witness.
///
/// Keyed on the modal's FOOTER being the bottom of the screen, not on its
/// question line anywhere in it: history can quote "Would you like to run the
/// following command?" (a diff of this very file), but only a live modal puts
/// its key hints below everything else, where the composer otherwise sits.
/// The last two lines are joined so a narrow pane that wraps the footer still
/// matches. An answered modal leaves no trace, scrollback included, so this
/// can never stick. A modal whose footer is not listed here reads as busy —
/// the quiet failure, where a false amber dot would cry wolf.
pub fn waiting_on_screen(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(2)..].join(" ");
    tail.contains(APPROVAL_FOOTER) || QUESTION_FOOTERS.iter().any(|f| tail.contains(f))
}

/// Composer content from a captured Codex TUI. Unknown layouts are not evidence
/// of submission. The model/path footer anchors the live prompt so transcript
/// messages (including old paste placeholders) cannot stand in for the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Composer {
    Empty,
    Text(String),
}

pub fn composer_on_screen(screen: &str) -> Option<Composer> {
    if waiting_on_screen(screen) {
        return None;
    }
    let lines: Vec<&str> = screen.lines().map(str::trim_end).collect();
    // The status line has a model followed by a directory. Hints can also
    // contain separators ("← for agents · ? for shortcuts"), so a separator
    // alone is not a footer. Everything below this status line is TUI hints,
    // including "tab to queue message" while a turn is running.
    let footer = lines.iter().rposition(|line| {
        let Some((model, rest)) = line.strip_prefix("  ").and_then(|l| l.split_once(" · ")) else {
            return false;
        };
        let path = rest.split(" · ").next().unwrap_or("").trim();
        !model.trim().is_empty() && (path.starts_with('/') || path.starts_with("~/") || path == "~")
    })?;
    let prompt = lines[..footer]
        .iter()
        .rposition(|l| l.starts_with("› ") || *l == "›")?;
    let mut content = lines[prompt].trim_start_matches('›').trim().to_string();
    if content.is_empty() || content == "Ask Codex to do anything" {
        return Some(Composer::Empty);
    }
    for line in &lines[prompt + 1..footer] {
        if !line.is_empty() && !line.starts_with("  ") {
            return None;
        }
        if !line.trim().is_empty() {
            content.push('\n');
            content.push_str(line.trim());
        }
    }
    Some(Composer::Text(content))
}

/// A paste chip belongs to the live composer, not to a message in history.
pub fn composer_has_paste(screen: &str) -> bool {
    let Some(Composer::Text(text)) = composer_on_screen(screen) else {
        return false;
    };
    text.split("[Pasted Content ").skip(1).any(|tail| {
        tail.split_once(" chars]")
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Only unchanged, nonempty composer content can settle. A blank or failed
/// capture, a modal, and the pre-typing empty prompt must never arm Enter.
pub fn composer_settled(previous: &str, current: &str) -> bool {
    matches!((composer_on_screen(previous), composer_on_screen(current)),
        (Some(Composer::Text(a)), Some(Composer::Text(b))) if a == b)
}

pub fn composer_submitted(screen: &str) -> bool {
    composer_on_screen(screen) == Some(Composer::Empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEND_EMPTY: &str = include_str!("../tests/fixtures/codex-send/empty.txt");
    const SEND_TYPED: &str = include_str!("../tests/fixtures/codex-send/typed.txt");
    const SEND_PASTED: &str = include_str!("../tests/fixtures/codex-send/pasted.txt");

    #[test]
    fn send_composer_decisions_use_real_captures() {
        assert!(SEND_PASTED.contains("[Pasted Content 1284 chars]"));
        assert!(composer_submitted(SEND_EMPTY));
        assert!(!composer_submitted(SEND_TYPED));
        assert!(!composer_submitted(SEND_PASTED));
        assert!(composer_has_paste(SEND_PASTED));
        assert!(!composer_has_paste(SEND_TYPED));
        assert!(composer_settled(SEND_PASTED, SEND_PASTED));
        assert!(composer_settled(SEND_TYPED, SEND_TYPED));
        assert!(!composer_settled(SEND_TYPED, SEND_PASTED));
        assert!(!composer_settled(SEND_EMPTY, SEND_EMPTY));
        assert!(!composer_submitted(""));
        assert!(!composer_submitted("› Ask Codex to do anything"));
        assert!(!composer_submitted(RUN_APPROVAL));
        assert!(!composer_settled(RUN_APPROVAL, RUN_APPROVAL));
        // History may quote both a chip and a modal. Only the live composer counts.
        let history = format!("{SEND_PASTED}\n{RUN_APPROVAL}\n{SEND_EMPTY}");
        assert!(composer_submitted(&history));
        assert!(!composer_has_paste(&history));
        // Transcript animations do not reset a settled composer.
        assert!(composer_settled(
            SEND_TYPED,
            &format!("• Working...\n{SEND_TYPED}")
        ));
    }

    #[test]
    fn send_review_busy_composer_settles_despite_queue_hint() {
        let screen = include_str!("../tests/fixtures/codex-send/review-busy-typed.txt");
        assert!(screen.contains("tab to queue message"));
        assert_eq!(composer_on_screen(screen), Some(Composer::Text("Second message: reply OK".into())));
        assert!(composer_settled(screen, screen));
        assert!(!composer_submitted(screen));
    }

    #[test]
    fn send_review_empty_composers_ignore_agent_navigation_hints() {
        for screen in [
            include_str!("../tests/fixtures/codex-send/review-idle.txt"),
            include_str!("../tests/fixtures/codex-send/review-idle-git.txt"),
            include_str!("../tests/fixtures/codex-send/review-busy-queued.txt"),
            include_str!("../tests/fixtures/codex-send/review-post-final.txt"),
        ] {
            assert!(screen.contains("← for agents · ? for shortcuts"));
            assert_eq!(composer_on_screen(screen), Some(Composer::Empty));
            assert!(composer_submitted(screen));
            assert!(!composer_settled(screen, screen));
        }
        let wrapped = include_str!("../tests/fixtures/codex-send/review-typed400.txt");
        assert!(composer_settled(wrapped, wrapped));
        assert!(!composer_submitted(wrapped));
    }

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
        // `codex exec` in the same worktree: no parent, but not a user thread.
        let exec = serde_json::json!({"cwd":"/w","source":"exec","parent_thread_id":null});
        assert!(!is_user_thread(&exec, "/w"));
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

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Shapes from the 2026-09-26 probe (0.157.1), content fields dropped.
    const STARTED: &str = r#"{"timestamp":"2026-09-26T20:31:40.648Z","type":"event_msg","payload":{"type":"task_started","turn_id":"t2","started_at":1790454700}}"#;
    const COMPLETE: &str = r#"{"timestamp":"2026-09-26T20:31:14.445Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"t1","started_at":1790454672,"completed_at":1790454674}}"#;
    const ABORTED: &str = r#"{"timestamp":"2026-09-26T20:33:06.534Z","type":"event_msg","payload":{"type":"turn_aborted","turn_id":"t3","reason":"interrupted","completed_at":1790454786}}"#;
    const TOOL: &str = r#"{"timestamp":"2026-09-26T20:31:44.814Z","type":"response_item","payload":{"type":"custom_tool_call","name":"exec"}}"#;

    #[test]
    fn a_started_turn_is_busy_until_its_boundary_lands() {
        assert_eq!(rollout_turn(&lines(&[COMPLETE, STARTED, TOOL])), Some(Turn::Busy));
        assert_eq!(
            rollout_turn(&lines(&[STARTED, TOOL, COMPLETE])),
            Some(Turn::Done { at: Some(1790454674), turn_id: Some("t1".into()) })
        );
    }

    /// A `task_complete` with no `completed_at` is dated by the record's own
    /// timestamp — still content — rather than dropped, which would leave the
    /// ring dark for a turn that did finish.
    #[test]
    fn a_completion_without_completed_at_uses_its_timestamp() {
        let bare = r#"{"timestamp":"2026-09-26T20:31:14.445Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"t1"}}"#;
        assert_eq!(
            rollout_turn(&lines(&[STARTED, bare])),
            Some(Turn::Done { at: Some(1790454674), turn_id: Some("t1".into()) })
        );
    }

    /// Esc writes `turn_aborted` and sends no notify — the case a
    /// completion-only source leaves busy forever.
    #[test]
    fn an_interrupted_turn_is_not_busy_and_not_done() {
        assert_eq!(rollout_turn(&lines(&[COMPLETE, STARTED, ABORTED])), Some(Turn::Aborted));
    }

    /// No boundary in the tail is "no answer", never "idle": a huge tool
    /// result can push the `task_started` out of the window mid-turn.
    #[test]
    fn a_tail_with_no_boundary_has_no_answer() {
        assert_eq!(rollout_turn(&lines(&[TOOL, TOOL])), None);
        assert_eq!(rollout_turn(&[]), None);
        // A content field that merely MENTIONS a record type is not one.
        let quoted = r#"{"type":"response_item","payload":{"type":"message","text":"\"task_complete\""}}"#;
        assert_eq!(rollout_turn(&lines(&[STARTED, quoted])), Some(Turn::Busy));
    }

    /// Screens captured from the 0.157.1 TUI in tmux (paths shortened).
    const RUN_APPROVAL: &str = "\
› Create an empty file named b.txt using the shell command touch b.txt.
• I’ll run touch b.txt to create the file.
• Running touch b.txt
  Would you like to run the following command?
  Environment: local
  Reason: Allow running touch b.txt to create the requested file? The filesystem sandbox is read-only.
  $ touch b.txt
› 1. Yes, proceed (y)
  2. Yes, and don't ask again for commands that start with `touch b.txt` (p)
  3. No, and tell Codex what to do differently (esc)
  Press enter to confirm or esc to cancel


";
    const EDIT_APPROVAL: &str = "\
• Edited a.txt (+1 -0)
    2 +hello
    + Show details
  Would you like to make the following edits?
  Description: Apply proposed file edits
  Destination: /w/a.txt
› 1. Yes, proceed (y)
  2. Yes, and don't ask again for these files (a)
  3. No, and tell Codex what to do differently (esc)
  Press enter to confirm or esc to cancel
";
    const QUESTION: &str = "\
› Use the request_user_input tool to ask me one short multiple-choice question.



  Question 1/1 (1 unanswered)
  Which color?

  › 1. Red                Choose red.
    2. Blue               Choose blue.
    3. None of the above  Optionally, add details in notes (tab)

  tab to add notes | enter to submit answer | esc to interrupt
";
    const IDLE: &str = "\
• Appended hello to a.txt using a file edit.
  15:54
                                                  Tip: Press ctrl+t to open the full transcript.
› Ask Codex to do anything
  GPT-6-Astra default · ~/w · Append hello to a.txt
  ? for shortcuts
";

    #[test]
    fn an_approval_or_a_question_on_screen_is_waiting() {
        assert!(waiting_on_screen(RUN_APPROVAL));
        assert!(waiting_on_screen(EDIT_APPROVAL));
        assert!(waiting_on_screen(QUESTION));
        // The last question of several ends on "submit all" instead.
        assert!(waiting_on_screen("  Question 2/2 (1 unanswered)\n  tab to add notes | enter to submit all | esc to interrupt\n"));
        assert!(!waiting_on_screen(IDLE));
        assert!(!waiting_on_screen(""));
    }

    /// History that QUOTES a modal is not a modal: only a live one puts its
    /// footer at the bottom, below where the composer otherwise sits.
    #[test]
    fn a_modal_quoted_in_history_is_not_waiting() {
        let quoted = format!("• Edited codex.rs\n{RUN_APPROVAL}\n{QUESTION}\n{IDLE}");
        assert!(!waiting_on_screen(&quoted));
    }

    /// A narrow pane wraps the footer onto two lines; the join finds it. A
    /// wrapped footer with the composer back under it is history, not a modal.
    #[test]
    fn a_wrapped_footer_still_counts() {
        assert!(waiting_on_screen("  3. No, and tell Codex what to do differently (esc)\n  Press enter to confirm or esc\n  to cancel\n"));
        assert!(!waiting_on_screen("  Press enter to confirm or esc\n  to cancel\n› Ask Codex to do anything\n  ? for shortcuts\n"));
    }
}
