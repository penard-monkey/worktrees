//! What the agent in a place is doing RIGHT NOW — busy, waiting on someone, idle
//! or not there — for both providers, derived ONCE.
//!
//! Two readers need the same answer: the app's nav dots (a 3s tick over every
//! live session) and the MCP server's `place_status` / `wait` (one place, on
//! demand). They used to be two derivations — Codex's lived only in the app —
//! so an orchestrator asking "is that Codex done?" could not be told what the
//! dot showed. Everything that DECIDES a state lives here; the app keeps its
//! scheduling (which sessions it watches, when it samples, how it stamps an
//! afterglow) and calls down.
//!
//! - **Claude** comes from its probe file (`agent::effective_state`): `busy` and
//!   `waiting` are what they say; `idle`, `shell` and a parked-away `delegated`
//!   busy are all idle for this purpose.
//! - **Codex** writes no status file. Its rollout brackets every turn
//!   (`codex::rollout_turn`), and a mid-turn session parked on an approval or a
//!   plan-mode question looks exactly like a running command there — so for a
//!   mid-turn session, and only then, the pane is captured and read with
//!   `codex::waiting_on_screen`. A pane back at a shell means codex exited,
//!   whatever the rollout's last line says.
//!
//! Cost is the app's, unchanged by the move: `codex_tail` re-reads a rollout's
//! 256K tail only once the file has GROWN (the cache moved here with it), and
//! `codex_panes` is one chained `tmux` call for however many sessions are
//! mid-turn, and none while none are.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use crate::codex::{self, Turn};
use crate::{agent, tmux, Project};

/// How much of a rollout's end is read per growth. Matches the app's transcript
/// tail, so one enormous tool result cannot hide the turn boundary before it
/// from both readers differently.
pub const ROLLOUT_TAIL_BYTES: u64 = 256 * 1024;

/// The state of the agent in one place.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Mid-turn and getting on with it.
    Busy,
    /// Mid-turn and stopped on a person: an approval, a question.
    Waiting,
    /// Running, between turns.
    Idle,
    /// No agent running in the place.
    None,
}

impl State {
    fn rank(self) -> u8 {
        match self {
            State::Busy => 0,
            State::Waiting => 1,
            State::Idle => 2,
            State::None => 3,
        }
    }
}

/// One place's activity, the same shape for both providers.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Activity {
    /// `claude`, `codex`, or null when nothing runs there.
    pub provider: Option<&'static str>,
    pub state: State,
    /// When the last turn FINISHED (epoch seconds), when the provider says.
    /// Codex dates it from its own `task_complete`; Claude's probe does not
    /// date turns, so it is null there.
    pub last_done: Option<i64>,
    /// The session it runs in — for Claude the `--name` other Claude sessions
    /// address with `SendMessage`, for Codex the tmux session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

impl Activity {
    pub fn none() -> Activity {
        Activity { provider: None, state: State::None, last_done: None, session: None }
    }
}

/// Claude's probe state, as an activity state. `delegated` (a parked-away
/// busy), `idle`, `shell` and anything newer claude writes are all idle here —
/// the rule the nav dot has always used.
pub fn claude_state(effective: &str) -> State {
    match effective {
        "busy" => State::Busy,
        "waiting" => State::Waiting,
        _ => State::Idle,
    }
}

/// What a mid-turn codex pane shows, per one capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexPane {
    /// The pane is back at a bare shell: codex exited or was killed.
    Gone,
    /// Codex is running and not asking anything.
    Running,
    /// Codex is sitting on an approval or a question (`waiting_on_screen`).
    Waiting,
}

/// A RUNNING codex session's state from its rollout's newest turn boundary and,
/// when that says mid-turn, what its pane showed. Returns the state and the
/// completion stamp (a finished turn only).
///
/// A mid-turn session with no pane answer (a tmux failure, a chain cut short)
/// reads busy: the rollout says busy, and busy is the quiet failure. A rollout
/// with no boundary at all is a fresh session: idle.
pub fn codex_state(turn: Option<&Turn>, pane: Option<CodexPane>) -> (State, Option<i64>) {
    match turn {
        Some(Turn::Busy) => match pane {
            Some(CodexPane::Waiting) => (State::Waiting, None),
            Some(CodexPane::Gone) => (State::None, None),
            _ => (State::Busy, None),
        },
        Some(Turn::Done { at, .. }) => (State::Idle, *at),
        Some(Turn::Aborted) | None => (State::Idle, None),
    }
}

/// Read the tail of a file as whole lines (the first, possibly-truncated line is
/// dropped). Decoded LOSSILY: a byte offset lands mid-character whenever the
/// boundary falls inside a multi-byte char, and a strict decode would throw the
/// whole tail away — silently, and stickily, since the boundary only moves as
/// the file grows. A file that cannot be OPENED is an empty tail (the normal
/// state for a session that has not written yet); `Err` says what failed after
/// that, for a caller that logs.
pub fn tail_lines_checked(path: &Path, max_bytes: u64) -> Result<Vec<String>, String> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Ok(Vec::new());
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let partial = len > max_bytes;
    if partial && f.seek(std::io::SeekFrom::Start(len - max_bytes)).is_err() {
        return Err(format!("tail seek failed: {}", path.display()));
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| format!("tail read failed ({e}): {}", path.display()))?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines().map(|s| s.to_string()).collect::<Vec<_>>();
    if partial && !lines.is_empty() {
        lines.remove(0);
    }
    Ok(lines)
}

/// Last answer per codex rollout: its length when read, the model it named and
/// where its newest turn stands. One cache and one tail read serve both, so a
/// streaming turn costs a single 256K read per tick, not one per question.
type TailCache = HashMap<PathBuf, (u64, Option<String>, Option<Turn>)>;
static CODEX_TAIL: Mutex<Option<TailCache>> = Mutex::new(None);

/// `path`'s (model, turn), re-read only once the file has GROWN. A tail that
/// now names neither (one enormous tool result filling it) keeps the last
/// answer for each: nothing changed because a big line landed.
pub fn codex_tail(path: &Path) -> (Option<String>, Option<Turn>) {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
        return (None, None);
    };
    let mut guard = CODEX_TAIL.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some((l, m, t)) = cache.get(path) {
        if *l == len {
            return (m.clone(), t.clone());
        }
    }
    let (pm, pt) = cache.get(path).map(|(_, m, t)| (m.clone(), t.clone())).unwrap_or((None, None));
    let lines = tail_lines_checked(path, ROLLOUT_TAIL_BYTES).unwrap_or_default();
    let m = codex::rollout_model(&lines).or(pm);
    let t = codex::rollout_turn(&lines).or(pt);
    cache.insert(path.to_path_buf(), (len, m.clone(), t.clone()));
    (m, t)
}

/// Capture the codex panes in `targets` (cwd, tmux session) in ONE `tmux`
/// call: each pane's current command, then its screen. `sessions` is the live
/// session list (newline-separated); targets missing from it are dropped first,
/// because a dead target aborts the rest of the chain. A marker precedes each
/// pane so a chain that dies part-way still attributes what it did print.
/// `=name:` is the session's current window and active pane, which is codex
/// unless the user split it — then the screen shows no codex footer and the
/// place simply reads busy.
pub fn codex_panes(sessions: &str, targets: &[(String, String)]) -> Vec<(String, CodexPane)> {
    let live: Vec<&str> = sessions.lines().filter(|l| !l.is_empty()).collect();
    let targets: Vec<(&String, String)> = targets
        .iter()
        .filter(|(_, s)| live.contains(&s.as_str()))
        .map(|(cwd, s)| (cwd, format!("={s}:")))
        .collect();
    if targets.is_empty() {
        return Vec::new();
    }
    let marks: Vec<String> = (0..targets.len()).map(|i| format!("@@ {i} @@")).collect();
    let mut args: Vec<&str> = Vec::with_capacity(targets.len() * 15);
    for (i, (_, target)) in targets.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        args.extend(["display-message", "-p", &marks[i], ";"]);
        args.extend(["display-message", "-p", "-t", target, "#{pane_current_command}", ";"]);
        args.extend(["capture-pane", "-p", "-t", target]);
    }
    let Ok(out) = tmux::tmux(&args) else {
        return Vec::new();
    };
    let cwds: Vec<&str> = targets.iter().map(|(c, _)| c.as_str()).collect();
    codex_panes_in(&String::from_utf8_lossy(&out.stdout), &cwds)
}

/// Parse `codex_panes`'s chained output: per target `i`, the line `@@ i @@`,
/// then the pane's current command, then its screen up to the next marker.
/// A target with no marker in the output (the chain died before it) is left
/// out — no answer, which `codex_state` reads as the rollout's busy.
pub fn codex_panes_in(text: &str, cwds: &[&str]) -> Vec<(String, CodexPane)> {
    let marks: Vec<String> = (0..cwds.len()).map(|i| format!("@@ {i} @@")).collect();
    let mut panes = Vec::new();
    for (i, cwd) in cwds.iter().enumerate() {
        let Some(rest) = text.split(&format!("{}\n", marks[i])).nth(1) else { continue };
        // The next marker ends this pane's block (the last one runs to EOF).
        let block = match marks.get(i + 1).and_then(|m| rest.find(m.as_str())) {
            Some(end) => &rest[..end],
            None => rest,
        };
        let (cmd, screen) = block.split_once('\n').unwrap_or((block, ""));
        let state = if tmux::is_shell_command(cmd.trim()) {
            CodexPane::Gone
        } else if codex::waiting_on_screen(screen) {
            CodexPane::Waiting
        } else {
            CodexPane::Running
        };
        panes.push((cwd.to_string(), state));
    }
    panes
}

/// The codex session name a place's managed Codex runs under: the `~agent~codex`
/// sidecar, or — for a session launched before sidecars — the canonical name
/// when that session is running codex. Answered from a pane snapshot.
pub fn codex_session_for(panes: &tmux::PaneList, canonical: &str) -> String {
    if panes.session_is_codex(canonical) {
        canonical.to_string()
    } else {
        tmux::codex_session_name(canonical)
    }
}

/// The Codex half of `place_activity`, for one place: `None` unless the place's
/// managed codex session is up AND running something other than a shell (the
/// pane is `codex …; exec "$SHELL"`, so it outlives codex).
pub fn codex_activity(panes: &tmux::PaneList, canonical: &str, path: &str) -> Option<Activity> {
    let name = codex_session_for(panes, canonical);
    if !panes.session_runs_program(&name) {
        return None;
    }
    let turn = codex::latest_rollout(path).and_then(|r| codex_tail(&r).1);
    let pane = if matches!(turn, Some(Turn::Busy)) {
        codex_panes(&name, &[(path.to_string(), name.clone())]).into_iter().next().map(|(_, p)| p)
    } else {
        None
    };
    let (state, last_done) = codex_state(turn.as_ref(), pane);
    Some(Activity { provider: Some("codex"), state, last_done, session: Some(name) })
}

/// The Claude half: the most active claude session whose cwd is `path`.
pub fn claude_activity(probes: &[agent::ClaudeProbe], path: &str) -> Option<Activity> {
    let a = agent::agents_at(probes, path).into_iter().next()?;
    let session = a.name.clone().or_else(|| a.tmux.as_deref().and_then(agent::session_name).map(str::to_string));
    Some(Activity { provider: Some("claude"), state: claude_state(&a.state), last_done: None, session })
}

/// The more active of two readings; Claude first on a tie (it is the one with
/// its own bus, so naming it is the more useful answer).
pub fn most_active(claude: Option<Activity>, codex: Option<Activity>) -> Activity {
    match (claude, codex) {
        (Some(c), Some(x)) => {
            if x.state.rank() < c.state.rank() {
                x
            } else {
                c
            }
        }
        (Some(c), None) => c,
        (None, Some(x)) => x,
        (None, None) => Activity::none(),
    }
}

/// Both providers' readings for one place, from a probe scan and a pane
/// snapshot the caller already holds (so a caller asking about several places
/// pays for each once).
pub fn place_activities(
    project: &Project,
    slug: &str,
    path: &str,
    probes: &[agent::ClaudeProbe],
    panes: Option<&tmux::PaneList>,
) -> (Option<Activity>, Option<Activity>) {
    let claude = claude_activity(probes, path);
    let codex = panes.and_then(|p| codex_activity(p, &project.session_name(slug), path));
    (claude, codex)
}

/// What the agent in one place is doing — the answer `place_status` and
/// `wait` give, and the same derivation the app's dots use.
pub fn place_activity(project: &Project, slug: &str, path: &str) -> Activity {
    let probes = agent::live_probes();
    let panes = tmux::PaneList::fetch();
    let (c, x) = place_activities(project, slug, path, &probes, panes.as_ref());
    most_active(c, x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_codex_turn_maps_to_one_state_per_case() {
        let done = Turn::Done { at: Some(100), turn_id: None };
        assert_eq!(codex_state(Some(&Turn::Busy), None), (State::Busy, None), "no pane answer is busy");
        assert_eq!(codex_state(Some(&Turn::Busy), Some(CodexPane::Running)), (State::Busy, None));
        assert_eq!(codex_state(Some(&Turn::Busy), Some(CodexPane::Waiting)), (State::Waiting, None));
        assert_eq!(
            codex_state(Some(&Turn::Busy), Some(CodexPane::Gone)),
            (State::None, None),
            "a dead codex is not busy, whatever its rollout says"
        );
        assert_eq!(codex_state(Some(&done), None), (State::Idle, Some(100)));
        assert_eq!(codex_state(Some(&Turn::Aborted), None), (State::Idle, None), "an Esc is not finished work");
        assert_eq!(codex_state(None, None), (State::Idle, None), "a fresh session is idle");
    }

    #[test]
    fn claude_states_map_the_way_the_dot_always_has() {
        assert_eq!(claude_state("busy"), State::Busy);
        assert_eq!(claude_state("waiting"), State::Waiting);
        for s in ["idle", "shell", "delegated", "something-new"] {
            assert_eq!(claude_state(s), State::Idle, "{s}");
        }
    }

    #[test]
    fn the_more_active_provider_answers() {
        let a = |p, s| Some(Activity { provider: Some(p), state: s, last_done: None, session: None });
        assert_eq!(most_active(a("claude", State::Idle), a("codex", State::Busy)).provider, Some("codex"));
        assert_eq!(most_active(a("claude", State::Waiting), a("codex", State::Waiting)).provider, Some("claude"));
        assert_eq!(most_active(None, a("codex", State::Idle)).state, State::Idle);
        assert_eq!(most_active(None, None), Activity::none());
        let v = serde_json::to_value(Activity::none()).unwrap();
        assert_eq!(v, serde_json::json!({ "provider": null, "state": "none", "last_done": null }));
    }

    /// The chained capture, two panes: each block runs from its marker to the
    /// NEXT marker (not to EOF — the first pane must not see the second's
    /// footer), its first line is the pane's command, and a bare shell there
    /// means codex is gone whatever the screen shows.
    #[test]
    fn codex_panes_split_the_chain_per_pane() {
        let modal = "  3. No, and tell Codex what to do differently (esc)\n  Press enter to confirm or esc to cancel\n";
        let idle = "› Ask Codex to do anything\n  ? for shortcuts\n";
        let text = format!("@@ 0 @@\ncodex\n{idle}@@ 1 @@\ncodex\n{modal}");
        assert_eq!(
            codex_panes_in(&text, &["/a", "/b"]),
            vec![("/a".into(), CodexPane::Running), ("/b".into(), CodexPane::Waiting)]
        );
        let text = format!("@@ 0 @@\nzsh\n{modal}@@ 1 @@\nnode\n{idle}");
        assert_eq!(
            codex_panes_in(&text, &["/a", "/b"]),
            vec![("/a".into(), CodexPane::Gone), ("/b".into(), CodexPane::Running)]
        );
        assert_eq!(codex_panes_in(&format!("@@ 0 @@\ncodex\n{idle}"), &["/a", "/b"]).len(), 1);
    }

    /// The rollout tail is re-read only when the file GROWS, and a tail that
    /// names no turn keeps the last answer.
    #[test]
    fn codex_tail_rereads_only_on_growth_and_keeps_the_last_answer() {
        let d = std::env::temp_dir().join(format!("wtact-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("r.jsonl");
        let started = r#"{"type":"event_msg","payload":{"type":"task_started"}}"#;
        std::fs::write(&f, format!("{started}\n")).unwrap();
        assert_eq!(codex_tail(&f).1, Some(Turn::Busy));
        // Same length, different bytes: served from the cache (the length IS the key).
        let same_len = r#"{"type":"event_msg","payload":{"type":"task_startex"}}"#;
        std::fs::write(&f, format!("{same_len}\n")).unwrap();
        assert_eq!(codex_tail(&f).1, Some(Turn::Busy));
        // Grew with nothing turn-shaped: the last answer stands.
        std::fs::write(&f, format!("{same_len}\n{{\"type\":\"response_item\"}}\n")).unwrap();
        assert_eq!(codex_tail(&f).1, Some(Turn::Busy));
        let done = r#"{"type":"event_msg","payload":{"type":"task_complete","completed_at":123}}"#;
        std::fs::write(&f, format!("{started}\n{done}\n")).unwrap();
        assert_eq!(codex_tail(&f).1, Some(Turn::Done { at: Some(123), turn_id: None }));
        let _ = std::fs::remove_dir_all(&d);
    }
}
