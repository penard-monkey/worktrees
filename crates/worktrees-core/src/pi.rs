//! pi (`@earendil-works/pi-coding-agent`) as a harness: where its session file
//! is, what it says, and what its screen says when the file cannot.
//!
//! Everything here was measured, and each fact names what it is pinned to
//! (docs/proposals/pi-harness.md §1.3, §3; fixtures in `tests/fixtures/pi-*`):
//!
//! - **Session file.** `--session-id <id>` creates or reopens EXACTLY
//!   `<session dir>/<iso ts>_<id>.jsonl`; `--session-dir` wins over a repo's
//!   `.pi/settings.json` `sessionDir`, which pi reads BEFORE trust is decided —
//!   so worktrees always passes the dir it will read, and a repo cannot move it.
//!   The default dir is `<agent dir>/sessions/--<cwd, leading / dropped, / \ :
//!   → ->--` (0.99.1 `session-manager.js::getDefaultSessionDirPath`).
//! - **No file until the first USER message.** 0.99.1 keeps setup entries
//!   (model, thinking level, system prompt) in memory and creates the file only
//!   once the session holds a user or assistant message
//!   (`session-manager.js::_hasConversation`; 0.87.1, which the proposal
//!   measured, waited for the first REPLY). So a lane launched with the opener
//!   has a file ending on its user message almost at once — busy, correctly —
//!   and the only windows the file cannot see are pi's startup before the
//!   opener is submitted and a trust modal holding it back. The screen covers
//!   those (the composer border, and the modal).
//! - **State keys on the newest MESSAGE entry**, never the last line (a
//!   `usage`/`label`/`custom` entry arrives without a turn) and never the
//!   file's mtime (the AGENTS.md transcript rule). A failed request is written
//!   as `assistant stopReason:error` and then taken back out of context by a
//!   `context_edit` while pi retries; the FINAL failure has no `context_edit`
//!   after it. "error, then context_edit" is still busy.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::activity::{self, Activity, State};
use crate::tmux;

/// pi's session-id charset, from its own `assertValidSessionId` (0.99.1):
/// `^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?$`.
pub fn valid_session_id(id: &str) -> bool {
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    let ends = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    id.chars().all(ok) && ends(id.chars().next()) && ends(id.chars().last())
}

/// FNV-1a, 64-bit. Stable across Rust versions and platforms, unlike
/// `DefaultHasher` — a session id derived from it has to mean the same file
/// next month.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Everything of a place's pi session id but the generation number:
/// `<canonical sanitised>-<6 hex of the UNSANITISED canonical>-g`.
///
/// Sanitising alone is not an identity: the slug is `basename(worktree dir)`,
/// which may hold any character but `/`, and main's slug is literally `(main)`
/// — `worktrees-(main)` is a real canonical name and pi rejects it as an id.
/// Mapping to pi's charset collapses `a b` and `a-b` into one id, so the hash of
/// the name BEFORE mapping keeps two places apart.
pub fn session_stem(canonical: &str) -> String {
    let mut s = String::with_capacity(canonical.len());
    for c in canonical.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' };
        if c == '-' && s.ends_with('-') {
            continue;
        }
        s.push(c);
    }
    let base = s.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let base = if base.is_empty() { "place" } else { base };
    format!("{base}-{:06x}-g", fnv1a(canonical) & 0xff_ffff)
}

/// A place's pi session id at `generation`. A fresh launch bumps the
/// generation; a resume reuses it (`harness::Pi::prepare`).
pub fn session_id(canonical: &str, generation: u32) -> String {
    format!("{}{generation}", session_stem(canonical))
}

/// pi's config dir: `$PI_CODING_AGENT_DIR`, else `~/.pi/agent` (pi's own
/// `getAgentDir`). Read-only as far as worktrees is concerned.
pub fn agent_dir() -> PathBuf {
    match std::env::var("PI_CODING_AGENT_DIR").ok().filter(|s| !s.is_empty()) {
        Some(d) => expand_tilde(&d),
        None => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".pi/agent"),
    }
}

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(p),
    }
}

/// The session dir pi would pick by default for `cwd` — and the one worktrees
/// passes as `--session-dir`, so the reader and the writer agree by
/// construction rather than by a repo's `sessionDir` not being set.
pub fn session_dir_in(agent_dir: &Path, cwd: &str) -> PathBuf {
    let trimmed = cwd.strip_prefix('/').or_else(|| cwd.strip_prefix('\\')).unwrap_or(cwd);
    let mangled: String = trimmed.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
    agent_dir.join("sessions").join(format!("--{mangled}--"))
}

pub fn session_dir(cwd: &str) -> PathBuf {
    session_dir_in(&agent_dir(), cwd)
}

/// The file pi writes for session `id` in `dir` (`<ts>_<id>.jsonl`), if it has
/// written one yet.
pub fn session_file(dir: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("_{id}.jsonl");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(&suffix))
        .map(|e| e.path())
        .max()
}

/// The newest-generation session file for the place whose ids share `stem`,
/// and its generation. Used by the activity reader, which runs every tick over
/// every live pi lane and so does not read the declared store: the highest
/// generation on disk is the place's current session except in the window
/// between a fresh launch and its first user message — pi's startup, or a
/// trust modal holding the opener back — exactly when the screen, not the
/// file, answers (`pi_state`).
pub fn latest_session_file(dir: &Path, stem: &str) -> Option<(u32, PathBuf)> {
    let marker = format!("_{stem}");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = &name[name.find(&marker)? + marker.len()..];
            let gen: u32 = rest.strip_suffix(".jsonl")?.parse().ok()?;
            Some((gen, e.path()))
        })
        .max()
}

/// Where a pi session's newest turn stands, from its JSONL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiTurn {
    /// A turn is running: the newest message is the user's, a tool result, an
    /// assistant asking for a tool, or an error pi has taken back to retry.
    Busy,
    /// The last turn finished (`stop`/`length`) at `at` — the entry's own
    /// timestamp, CONTENT, never the file's mtime.
    Done { at: Option<i64> },
    /// Esc: `stopReason: aborted`. Not finished work.
    Aborted,
    /// The retries ran out: an `error` with no `context_edit` after it. The
    /// turn is over and produced nothing.
    Failed,
}

/// The newest turn among session `lines` (oldest first), or `None` when they
/// hold no message pi counts as a turn (a header only, or a resumed session's
/// `system` entry with nothing after it and nothing before in the tail).
pub fn session_turn(lines: &[String]) -> Option<PiTurn> {
    // The ids later `context_edit`s took back out of context. Keyed on the
    // edit's `targetId`, not on "some edit came after": an edit that removed
    // something else (a compaction, an extension) says nothing about the error.
    let mut retracted: Vec<String> = Vec::new();
    for l in lines.iter().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("context_edit") => retracted.extend(v.get("targetId").and_then(|t| t.as_str()).map(str::to_string)),
            Some("message") => {
                let m = v.get("message");
                let role = m.and_then(|m| m.get("role")).and_then(|r| r.as_str()).unwrap_or("");
                let stop = m.and_then(|m| m.get("stopReason")).and_then(|r| r.as_str()).unwrap_or("");
                return Some(match (role, stop) {
                    // A resume writes the prompt sections as a `system` message;
                    // it is not a turn, and neither is an extension's own role.
                    ("assistant", "stop" | "length") => PiTurn::Done {
                        at: v.get("timestamp").and_then(|t| t.as_str()).and_then(crate::sysclock::parse_iso8601),
                    },
                    ("assistant", "aborted") => PiTurn::Aborted,
                    ("assistant", "error")
                        if v.get("id").and_then(|i| i.as_str()).is_some_and(|id| retracted.iter().any(|r| r == id)) =>
                    {
                        PiTurn::Busy
                    }
                    ("assistant", "error") => PiTurn::Failed,
                    ("assistant", _) | ("user", _) | ("toolResult", _) => PiTurn::Busy,
                    _ => continue,
                });
            }
            _ => {}
        }
    }
    None
}

/// The model a session is on: the newest of the last assistant message's
/// `provider/model` and the last `model_change` — whichever was APPENDED
/// later, which is what the user just saw (a `/model` switch writes its
/// `model_change` at once, before any reply). `backend/model`, pi's own
/// spelling, so it can be handed straight back to `--model`.
pub fn session_model(lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|l| {
        if !(l.contains("\"model_change\"") || l.contains("\"assistant\"")) {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        let (backend, model) = match v.get("type")?.as_str()? {
            "model_change" => (v.get("provider")?.as_str()?, v.get("modelId")?.as_str()?),
            "message" => {
                let m = v.get("message")?;
                if m.get("role")?.as_str()? != "assistant" {
                    return None;
                }
                (m.get("provider")?.as_str()?, m.get("model")?.as_str()?)
            }
            _ => return None,
        };
        Some(format!("{backend}/{model}"))
    })
}

type TailCache = HashMap<PathBuf, (u64, Option<String>, Option<PiTurn>)>;
static PI_TAIL: Mutex<Option<TailCache>> = Mutex::new(None);

/// `path`'s (model, turn), re-read only once the file has GROWN — the same
/// shape and reason as `activity::codex_tail`.
pub fn session_tail(path: &Path) -> (Option<String>, Option<PiTurn>) {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
        return (None, None);
    };
    let mut guard = PI_TAIL.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some((l, m, t)) = cache.get(path) {
        if *l == len {
            return (m.clone(), t.clone());
        }
    }
    let (pm, pt) = cache.get(path).map(|(_, m, t)| (m.clone(), t.clone())).unwrap_or((None, None));
    let lines = activity::tail_lines_checked(path, activity::ROLLOUT_TAIL_BYTES).unwrap_or_default();
    let m = session_model(&lines).or(pm);
    let t = session_turn(&lines).or(pt);
    cache.insert(path.to_path_buf(), (len, m.clone(), t.clone()));
    (m, t)
}

/// What one capture of a pi pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiScreen {
    /// The pane is back at a bare shell: pi exited or was killed.
    Gone,
    /// The composer's top border reads `── <spinner> Working` or `Retrying`.
    Working,
    /// The project-trust modal is up. Its highlighted choice is **Trust**, so
    /// an Enter typed into it trusts the repo — this is pi's only "waiting".
    TrustModal,
    /// Anything else: pi at its prompt, or a screen we do not recognise.
    Other,
}

fn is_rule(line: &str) -> bool {
    let t = line.trim_end();
    !t.is_empty() && t.chars().all(|c| c == '─')
}

/// Read a pi screen by POSITION, never by text found anywhere: history can
/// quote "Working" or a trust prompt, the composer border cannot.
///
/// The composer is two rules with the input between them, followed by the
/// cwd/usage footer. When a turn runs, the TOP rule is replaced by the status
/// line itself (`── ⠙ Working ───…`, `── ⠼ Retrying (3/3) in 7s… ───`) —
/// measured on 0.87.1 (`fixtures/pi-screen/working.txt`: the status line sits
/// directly on the input, with no plain rule above it). So: find the bottom
/// rule (the last full rule), walk up past the input to the first line
/// starting with `─`, and read that line.
///
/// The trust modal has no composer: its last full rule closes a block whose
/// last text line is the `↑↓ navigate  enter select` footer, under a
/// `Trust project folder?` heading.
pub fn read_screen(screen: &str) -> PiScreen {
    let lines: Vec<&str> = screen.lines().collect();
    let Some(bottom) = lines.iter().rposition(|l| is_rule(l)) else {
        return PiScreen::Other;
    };
    let above: Vec<&str> = lines[..bottom].iter().rev().copied().filter(|l| !l.trim().is_empty()).collect();
    if above.first().is_some_and(|l| l.contains("enter select") && l.contains("navigate")) {
        let block_start = lines[..bottom].iter().rposition(|l| is_rule(l)).map_or(0, |i| i + 1);
        if lines[block_start..bottom].iter().any(|l| l.trim() == "Trust project folder?") {
            return PiScreen::TrustModal;
        }
    }
    let Some(top) = lines[..bottom].iter().rposition(|l| l.starts_with('─')) else {
        return PiScreen::Other;
    };
    if border_is_busy(lines[top]) {
        return PiScreen::Working;
    }
    PiScreen::Other
}

/// Whether a composer top border carries a working status. It FAILS SAFE: pi
/// 0.99.1 puts any status it likes there (`custom-editor.js::renderTopBorder`)
/// — `Working`, `Retrying (n/3)`, `Compacting context…`, an extension's own
/// working message, or on a narrow pane a spinner with no word at all
/// (`renderSpinnerInBorder`) — so this does not match words. The one
/// non-status thing an idle border carries is the input's overflow label
/// (` ↑ N more `); with that removed, anything left that is not rule is a
/// status, and a status means a turn is running.
pub fn border_is_busy(line: &str) -> bool {
    let t = line.trim_end();
    let stripped = match (t.find(" ↑ "), t.find(" more ")) {
        (Some(a), Some(b)) if a < b && t[a + " ↑ ".len()..b].chars().all(|c| c.is_ascii_digit()) => {
            format!("{}{}", &t[..a], &t[b + " more ".len()..])
        }
        _ => t.to_string(),
    };
    !stripped.chars().all(|c| c == '─' || c == ' ')
}

/// A pi lane's state from its session file's newest turn and one capture of
/// its pane. The screen answers first — it is the only witness of the gap
/// before the first user message (no file yet) and of the trust modal — and
/// the file answers the rest.
///
/// A modal is `Waiting` whatever the file says: a RESUMED session in `ask`
/// mode has a file that ends idle and a modal on screen. No capture at all
/// (a tmux failure) leaves the file's answer standing, busy included.
pub fn pi_state(turn: Option<&PiTurn>, screen: Option<PiScreen>) -> (State, Option<i64>) {
    match screen {
        Some(PiScreen::Gone) => return (State::None, None),
        Some(PiScreen::TrustModal) => return (State::Waiting, None),
        Some(PiScreen::Working) => return (State::Busy, None),
        _ => {}
    }
    match turn {
        Some(PiTurn::Busy) => (State::Busy, None),
        Some(PiTurn::Done { at }) => (State::Idle, *at),
        Some(PiTurn::Aborted | PiTurn::Failed) | None => (State::Idle, None),
    }
}

/// Capture one pi pane: its current command, then its screen, in ONE `tmux`
/// call. `None` when tmux could not answer.
fn capture(session: &str) -> Option<PiScreen> {
    let target = format!("={session}:");
    let out = tmux::tmux(&[
        "display-message", "-p", "-t", &target, "#{pane_current_command}", ";", "capture-pane", "-p", "-t", &target,
    ])
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (cmd, screen) = text.split_once('\n').unwrap_or((&text, ""));
    Some(if tmux::is_shell_command(cmd.trim()) { PiScreen::Gone } else { read_screen(screen) })
}

/// The pi half of `place_activity` for one place: `None` unless the place's
/// `~agent~pi` session is up and running something other than a shell (the
/// pane is `pi …; exec "$SHELL"`, so it outlives pi, and pi runs as `node`).
pub fn pi_activity(panes: &tmux::PaneList, canonical: &str, path: &str) -> Option<Activity> {
    let name = crate::provider::PI.sidecar_name(canonical);
    if !panes.session_runs_program(&name) {
        return None;
    }
    let turn = latest_session_file(&session_dir(path), &session_stem(canonical)).and_then(|(_, f)| session_tail(&f).1);
    let (state, last_done) = pi_state(turn.as_ref(), capture(&name));
    Some(Activity { provider: Some("pi"), state, last_done, session: Some(name) })
}

/// The model the place's current pi session is on, from its file.
pub fn running_model(canonical: &str, path: &str) -> Option<String> {
    let (_, f) = latest_session_file(&session_dir(path), &session_stem(canonical))?;
    session_tail(&f).0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn a_session_id_is_valid_for_pi_and_unique_per_place() {
        let main = session_id("worktrees-(main)", 3);
        assert!(main.starts_with("worktrees-main-") && main.ends_with("-g3"), "{main}");
        assert!(valid_session_id(&main), "{main}");
        // Sanitising alone would collapse these; the hash of the raw name does not.
        assert_ne!(session_stem("p-a b"), session_stem("p-a-b"));
        for raw in ["(main)", "--", "a..b", "ü", "x~agent~pi", ".hidden", "trailing-"] {
            let id = session_id(raw, 1);
            assert!(valid_session_id(&id), "{raw} -> {id}");
        }
        assert!(session_id("", 1).starts_with("place-"));
        // Stable: the same place names the same file next week.
        assert_eq!(session_stem("worktrees-(main)"), session_stem("worktrees-(main)"));
        assert!(!valid_session_id("worktrees-(main)"));
        assert!(!valid_session_id("-a"));
        assert!(!valid_session_id("a~agent~pi"));
    }

    #[test]
    fn the_session_dir_is_pis_own_mangling() {
        let d = session_dir_in(Path::new("/h/.pi/agent"), "/Users/me/.cache/wt/repo");
        assert_eq!(d, PathBuf::from("/h/.pi/agent/sessions/--Users-me-.cache-wt-repo--"));
        let d = session_dir_in(Path::new("/a"), "/x/y:z");
        assert_eq!(d, PathBuf::from("/a/sessions/--x-y-z--"));
    }

    #[test]
    fn session_files_are_found_by_id_and_by_newest_generation() {
        let d = std::env::temp_dir().join(format!("wtpi-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let stem = session_stem("p-feat");
        for (ts, g) in [("2026-09-29T10-00-00-000Z", 2), ("2026-09-29T09-00-00-000Z", 10), ("2026-09-29T11-00-00-000Z", 9)] {
            std::fs::write(d.join(format!("{ts}_{stem}{g}.jsonl")), "").unwrap();
        }
        // A near-miss stem (another place) and a non-numeric tail are ignored.
        std::fs::write(d.join(format!("2026-09-29T12-00-00-000Z_{}99.jsonl", session_stem("p-other"))), "").unwrap();
        std::fs::write(d.join(format!("2026-09-29T12-00-00-000Z_{stem}x.jsonl")), "").unwrap();
        let (g, f) = latest_session_file(&d, &stem).unwrap();
        assert_eq!(g, 10, "numeric, not lexical, and not by timestamp");
        assert!(f.to_string_lossy().ends_with(&format!("{stem}10.jsonl")));
        assert!(session_file(&d, &format!("{stem}2")).is_some());
        assert!(session_file(&d, &format!("{stem}3")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    const TWO_TURNS: &str = include_str!("../tests/fixtures/pi-session/two-turns.jsonl");
    const TOOL_ESC: &str = include_str!("../tests/fixtures/pi-session/tool-esc.jsonl");
    const REFUSED: &str = include_str!("../tests/fixtures/pi-session/refused.jsonl");

    /// Replays a real session (0.87.1) line by line: every prefix of the file
    /// is a moment a reader could have sampled.
    #[test]
    fn a_real_session_reads_busy_until_its_reply_lands() {
        let all = lines(TOOL_ESC);
        let mut seen = Vec::new();
        for n in 1..=all.len() {
            seen.push(session_turn(&all[..n]).map(|t| match t {
                PiTurn::Busy => "busy",
                PiTurn::Done { .. } => "done",
                PiTurn::Aborted => "aborted",
                PiTurn::Failed => "failed",
            }));
        }
        // header, model_change, thinking, system: no turn yet.
        assert_eq!(&seen[..4], &[None, None, None, None]);
        // user → assistant(toolUse) → toolResult → assistant(stop): busy ×3, then done.
        assert_eq!(&seen[4..8], &[Some("busy"), Some("busy"), Some("busy"), Some("done")]);
        // Then an Esc'd turn.
        assert_eq!(&seen[8..10], &[Some("busy"), Some("aborted")]);
        assert_eq!(seen.last().unwrap(), &Some("done"));
        match session_turn(&all).unwrap() {
            PiTurn::Done { at } => assert!(at.unwrap() > 1_790_000_000),
            t => panic!("{t:?}"),
        }
    }

    #[test]
    fn an_error_taken_back_is_a_retry_and_the_last_one_ends_the_turn() {
        let all = lines(REFUSED);
        let idx = |pred: &dyn Fn(&str) -> bool| all.iter().position(|l| pred(l)).unwrap();
        let first_error = idx(&|l| l.contains("\"stopReason\":\"error\""));
        // The error alone (before its context_edit lands) reads as failed…
        assert_eq!(session_turn(&all[..=first_error]), Some(PiTurn::Failed));
        // …and once pi takes it back to retry, busy again.
        assert_eq!(session_turn(&all[..=first_error + 1]), Some(PiTurn::Busy));
        // The final error has no context_edit after it.
        assert!(all.last().unwrap().contains("\"error\""));
        assert_eq!(session_turn(&all), Some(PiTurn::Failed));
        // An edit that took back something ELSE is not a retry of this error.
        let mut other = all.clone();
        other.push(r#"{"type":"context_edit","id":"ffff0000","targetId":"not-the-error","replacement":null}"#.into());
        assert_eq!(session_turn(&other), Some(PiTurn::Failed));
    }

    #[test]
    fn non_message_entries_are_not_turns_and_the_model_is_the_newest_appended() {
        let mut all = lines(TWO_TURNS);
        let done = session_turn(&all);
        assert!(matches!(done, Some(PiTurn::Done { .. })));
        all.push(r#"{"type":"usage","timestamp":"2026-09-29T17:00:00.000Z"}"#.into());
        all.push(r#"{"type":"label","timestamp":"2026-09-29T17:00:01.000Z"}"#.into());
        assert_eq!(session_turn(&all), done, "a usage/label entry is not a turn");
        assert_eq!(session_model(&all).as_deref(), Some("lm-studio/qwen3.6-27b"));
        // A /model switch writes model_change at once — it wins before any reply.
        all.push(r#"{"type":"model_change","provider":"kimi-coding","modelId":"k3"}"#.into());
        assert_eq!(session_model(&all).as_deref(), Some("kimi-coding/k3"));
        assert_eq!(session_model(&lines(REFUSED)).as_deref(), Some("lm-refused/qwen3.6-27b"));
        assert_eq!(session_turn(&[]), None);
    }

    fn screen(name: &str) -> &'static str {
        match name {
            "start-idle" => include_str!("../tests/fixtures/pi-screen/start-idle.txt"),
            "turn-done" => include_str!("../tests/fixtures/pi-screen/turn-done.txt"),
            "working" => include_str!("../tests/fixtures/pi-screen/working.txt"),
            "aborted" => include_str!("../tests/fixtures/pi-screen/aborted.txt"),
            "retrying" => include_str!("../tests/fixtures/pi-screen/retrying.txt"),
            "trust-modal" => include_str!("../tests/fixtures/pi-screen/trust-modal.txt"),
            "steering-typed" => include_str!("../tests/fixtures/pi-screen/steering-typed.txt"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn screens_read_by_position() {
        for (name, want) in [
            ("start-idle", PiScreen::Other),
            ("turn-done", PiScreen::Other),
            ("aborted", PiScreen::Other),
            ("working", PiScreen::Working),
            ("retrying", PiScreen::Working),
            // Typed input in the composer does not hide the status border.
            ("steering-typed", PiScreen::Working),
            ("trust-modal", PiScreen::TrustModal),
        ] {
            assert_eq!(read_screen(screen(name)), want, "{name}");
        }
        // History that QUOTES the status line or the modal is not either.
        let quoted = screen("turn-done").replacen(
            " Done. I ran ls",
            "── ⠙ Working ──\n Trust project folder?\n ↑↓ navigate  enter select  escape/ctrl+c cancel\n Done. I ran ls",
            1,
        );
        assert_eq!(read_screen(&quoted), PiScreen::Other);
        assert_eq!(read_screen(""), PiScreen::Other);
    }

    /// 0.99.1's border shapes, from `custom-editor.js::renderTopBorder`: a
    /// status the reader has never heard of, a spinner with no word on a narrow
    /// pane, and an IDLE border carrying only the input's overflow label.
    #[test]
    fn a_border_status_reads_busy_whatever_it_says_and_an_overflow_label_does_not() {
        let w = |t: &str| border_is_busy(t);
        assert!(w("── ⠙ Working ────────────"));
        assert!(w("── ⠼ Retrying (3/3) in 7s... (escape to cancel) ────"));
        assert!(w("── ⠋ Compacting context… ──────────"), "a status this build never saw is still a status");
        assert!(w("── ⠋ Auto-compacting… ─────── ↑ 4 more ─────"), "a status beside an overflow label");
        assert!(w("───⠋─────"), "spinner only, no word (narrow pane)");
        assert!(!w("──────────────────"));
        assert!(!w("─────────── ↑ 12 more ───────────"), "idle, with typed input overflowing");
        assert!(!w("────────────────── "));
        // Through the full screen reader, positionally.
        let base = include_str!("../tests/fixtures/pi-screen/turn-done.txt");
        let bottom_rule = base.lines().rev().find(|l| is_rule(l)).unwrap().to_string();
        let with_top = |top: &str| {
            let mut lines: Vec<String> = base.lines().map(str::to_string).collect();
            let b = lines.iter().rposition(|l| is_rule(l)).unwrap();
            let t = lines[..b].iter().rposition(|l| l.starts_with('─')).unwrap();
            lines[t] = top.to_string();
            lines.join("\n")
        };
        assert_eq!(read_screen(&with_top("── ⠋ Compacting context… ───────")), PiScreen::Working);
        assert_eq!(read_screen(&with_top("──⠋────────")), PiScreen::Working);
        assert_eq!(read_screen(&with_top("──────── ↑ 3 more ────────")), PiScreen::Other);
        assert!(!bottom_rule.is_empty());
    }

    /// Re-pinned on pi 0.99.1 (2026-09-29, live, `lm-studio/qwen3.6-27b`):
    /// the same readers on the version this was built against.
    #[test]
    fn pi_0_99_1_screens_and_sessions_read_as_measured() {
        for (screen, want) in [
            (include_str!("../tests/fixtures/pi-screen/0.99.1/working.txt"), PiScreen::Working),
            (include_str!("../tests/fixtures/pi-screen/0.99.1/retrying.txt"), PiScreen::Working),
            (include_str!("../tests/fixtures/pi-screen/0.99.1/trust-modal.txt"), PiScreen::TrustModal),
            (include_str!("../tests/fixtures/pi-screen/0.99.1/done.txt"), PiScreen::Other),
            (include_str!("../tests/fixtures/pi-screen/0.99.1/aborted.txt"), PiScreen::Other),
        ] {
            assert_eq!(read_screen(screen), want);
        }
        // Launch → tool turn → a second turn → Esc; model from the file.
        let turns = lines(include_str!("../tests/fixtures/pi-session/0.99.1/turns-esc-resume.jsonl"));
        assert_eq!(session_turn(&turns), Some(PiTurn::Aborted));
        assert_eq!(session_model(&turns).as_deref(), Some("lm-studio/qwen3.6-27b"));
        // 0.99.1 writes the file at the first USER message: its first
        // conversational entry is the user's, and a prefix ending there is busy.
        let first_user = turns.iter().position(|l| l.contains("\"role\":\"user\"")).unwrap();
        assert_eq!(session_turn(&turns[..=first_user]), Some(PiTurn::Busy));
        // A refused host: busy through every retry, failed at the end.
        let refused = lines(include_str!("../tests/fixtures/pi-session/0.99.1/refused.jsonl"));
        let retry = refused.iter().rposition(|l| l.contains("\"context_edit\"")).unwrap();
        assert_eq!(session_turn(&refused[..=retry]), Some(PiTurn::Busy));
        assert_eq!(session_turn(&refused), Some(PiTurn::Failed));
    }

    #[test]
    fn the_screen_answers_first_and_the_file_the_rest() {
        let done = PiTurn::Done { at: Some(5) };
        // Before the first user message lands: no file, a Working border.
        assert_eq!(pi_state(None, Some(PiScreen::Working)), (State::Busy, None));
        assert_eq!(pi_state(None, Some(PiScreen::Other)), (State::Idle, None));
        // A resumed session in ask mode: idle file, modal on screen.
        assert_eq!(pi_state(Some(&done), Some(PiScreen::TrustModal)), (State::Waiting, None));
        assert_eq!(pi_state(Some(&PiTurn::Busy), Some(PiScreen::Gone)), (State::None, None), "a dead pi is not busy");
        assert_eq!(pi_state(Some(&done), Some(PiScreen::Other)), (State::Idle, Some(5)));
        assert_eq!(pi_state(Some(&PiTurn::Busy), None), (State::Busy, None), "no capture leaves the file's answer");
        assert_eq!(pi_state(Some(&PiTurn::Aborted), Some(PiScreen::Other)), (State::Idle, None), "Esc is not finished work");
        assert_eq!(pi_state(Some(&PiTurn::Failed), Some(PiScreen::Other)), (State::Idle, None));
    }
}
