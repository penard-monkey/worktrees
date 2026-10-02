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
    /// Something other than a shell runs in the place's own session and no
    /// harness reports on it — a program typed into the pane by hand, or an
    /// agent that has not started reporting yet. Not `none`: "nobody is
    /// working there" would be a guess, and `wait until: idle` must not
    /// answer on it (`Activity::reason` says why).
    Unknown,
}

impl State {
    fn rank(self) -> u8 {
        match self {
            State::Busy => 0,
            State::Waiting => 1,
            State::Idle => 2,
            // Something running beats nothing running; any harness's own
            // reading beats a program nobody reports on.
            State::Unknown => 3,
            State::None => 4,
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
    /// Why the state is what it is, when the state alone would mislead —
    /// only `unknown` carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Activity {
    pub fn none() -> Activity {
        Activity { provider: None, state: State::None, last_done: None, session: None, reason: None }
    }

    /// `program` runs in `session` and no harness reports on it.
    pub fn unknown(session: &str, program: &str) -> Activity {
        Activity {
            provider: None,
            state: State::Unknown,
            last_done: None,
            session: Some(session.to_string()),
            reason: Some(format!(
                "a program this place did not launch is running in {session} ({program}), and no agent \
                 reports on it — it was typed into the pane, or an agent there has not started reporting. \
                 Ask the user; never type into it."
            )),
        }
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

/// Capture the panes in `targets` (key, tmux session) in ONE `tmux` call:
/// each pane's current command, then its screen — (key, command, screen) per
/// pane that answered. `sessions` is the live session list (newline-separated);
/// targets missing from it are dropped first, because a dead target aborts the
/// rest of the chain. A marker precedes each pane so a chain that dies part-way
/// still attributes what it did print. `=name:` is the session's current window
/// and active pane. Harness-neutral: codex and pi each read the screens their
/// own way (`codex_panes`, `pi::pi_panes`).
pub fn capture_chain(sessions: &str, targets: &[(String, String)]) -> Vec<(String, String, String)> {
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
    chain_blocks_in(&String::from_utf8_lossy(&out.stdout), &cwds)
}

/// Parse `capture_chain`'s output: per target `i`, the line `@@ i @@`, then
/// the pane's current command, then its screen up to the next marker. A target
/// with no marker in the output (the chain died before it) is left out — no
/// answer, which each harness's state function reads its own way.
pub fn chain_blocks_in(text: &str, cwds: &[&str]) -> Vec<(String, String, String)> {
    let marks: Vec<String> = (0..cwds.len()).map(|i| format!("@@ {i} @@")).collect();
    let mut blocks = Vec::new();
    for (i, cwd) in cwds.iter().enumerate() {
        let Some(rest) = text.split(&format!("{}\n", marks[i])).nth(1) else { continue };
        // The next marker ends this pane's block (the last one runs to EOF).
        let block = match marks.get(i + 1).and_then(|m| rest.find(m.as_str())) {
            Some(end) => &rest[..end],
            None => rest,
        };
        let (cmd, screen) = block.split_once('\n').unwrap_or((block, ""));
        blocks.push((cwd.to_string(), cmd.trim().to_string(), screen.to_string()));
    }
    blocks
}

/// Capture the codex panes in `targets` (cwd, tmux session) in one `tmux` call
/// (`capture_chain`). `=name:` is codex unless the user split the window —
/// then the screen shows no codex footer and the place simply reads busy.
pub fn codex_panes(sessions: &str, targets: &[(String, String)]) -> Vec<(String, CodexPane)> {
    capture_chain(sessions, targets).into_iter().map(|(cwd, cmd, screen)| (cwd, codex_pane(&cmd, &screen))).collect()
}

/// `codex_panes`'s parse, for the tests: `chain_blocks_in`, read as codex.
/// A target the chain never reached is left out — no answer, which
/// `codex_state` reads as the rollout's busy.
pub fn codex_panes_in(text: &str, cwds: &[&str]) -> Vec<(String, CodexPane)> {
    chain_blocks_in(text, cwds).into_iter().map(|(cwd, cmd, screen)| (cwd, codex_pane(&cmd, &screen))).collect()
}

fn codex_pane(cmd: &str, screen: &str) -> CodexPane {
    if tmux::is_shell_command(cmd) {
        CodexPane::Gone
    } else if codex::waiting_on_screen(screen) {
        CodexPane::Waiting
    } else {
        CodexPane::Running
    }
}

/// The codex session name a place's managed Codex runs under: the `~agent~codex`
/// sidecar, or — for a session launched before sidecars — the canonical name
/// when that session is running codex. Answered from a pane snapshot.
pub fn codex_session_for(panes: &tmux::PaneList, canonical: &str) -> String {
    crate::provider::CODEX.session_name(canonical, panes.canonical_provider(canonical).id, false)
}

/// The pi session name for a place, by the same rule: its `~agent~pi`
/// sidecar, or the canonical session when pi is what runs there (a pi typed
/// into the place's own pane — `PaneList::canonical_provider` names it from
/// the tty's foreground leader, since tmux only sees `node`).
pub fn pi_session_for(panes: &tmux::PaneList, canonical: &str) -> String {
    crate::provider::PI.session_name(canonical, panes.canonical_provider(canonical).id, false)
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
    Some(Activity { provider: Some("codex"), state, last_done, session: Some(name), reason: None })
}

/// The Claude half: the most active claude session whose cwd is `path`.
pub fn claude_activity(probes: &[agent::ClaudeProbe], path: &str) -> Option<Activity> {
    let a = agent::agents_at(probes, path).into_iter().next()?;
    let session = a.name.clone().or_else(|| a.tmux.as_deref().and_then(agent::session_name).map(str::to_string));
    Some(Activity { provider: Some("claude"), state: claude_state(&a.state), last_done: None, session, reason: None })
}

/// The most active of any number of readings (one per harness, in registry
/// order); the EARLIER reading wins a tie — Claude first, since it is the one
/// with its own bus, so naming it is the more useful answer.
pub fn most_active(readings: impl IntoIterator<Item = Activity>) -> Activity {
    readings.into_iter().fold(None, |best: Option<Activity>, x| match best {
        Some(b) if b.state.rank() <= x.state.rank() => Some(b),
        _ => Some(x),
    })
    .unwrap_or_else(Activity::none)
}

/// The ONE answer for a place from its harness readings — `place_status`'s
/// `activity` and `wait`'s, never two derivations. The most active reading;
/// or, when no harness reports at all while the place's own session runs a
/// program, `unknown` rather than `none` — an orchestrator told "nobody is
/// working there" acts on it.
pub fn place_answer(readings: impl IntoIterator<Item = Activity>, panes: Option<&tmux::PaneList>, canonical: &str) -> Activity {
    let mut readings = readings.into_iter().peekable();
    if readings.peek().is_none() {
        if let Some(program) = panes.and_then(|p| p.program_in(canonical)) {
            return Activity::unknown(canonical, program);
        }
    }
    most_active(readings)
}

/// What the agent in one place is doing — the answer `place_status` and
/// `wait` give, and the same derivation the app's dots use.
pub fn place_activity(project: &Project, slug: &str, path: &str) -> Activity {
    let probes = agent::live_probes();
    let panes = tmux::PaneList::fetch();
    let scan = crate::harness::Scan { probes: &probes, panes: panes.as_ref() };
    let readings = crate::harness::place_activities(project, slug, path, &scan);
    place_answer(readings.into_iter().map(|(_, x)| x), panes.as_ref(), &project.session_name(slug))
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
        let a = |p, s| Activity { provider: Some(p), state: s, last_done: None, session: None, reason: None };
        assert_eq!(most_active([a("claude", State::Idle), a("codex", State::Busy)]).provider, Some("codex"));
        assert_eq!(most_active([a("claude", State::Waiting), a("codex", State::Waiting)]).provider, Some("claude"));
        assert_eq!(most_active([a("codex", State::Idle)]).state, State::Idle);
        assert_eq!(most_active([]), Activity::none());
        // N-ary: the first of the most active wins, wherever it sits.
        let three = [a("claude", State::Idle), a("codex", State::Waiting), a("third", State::Waiting)];
        assert_eq!(most_active(three).provider, Some("codex"));
        let three = [a("claude", State::Idle), a("codex", State::Idle), a("third", State::Busy)];
        assert_eq!(most_active(three).provider, Some("third"));
        let v = serde_json::to_value(Activity::none()).unwrap();
        assert_eq!(v, serde_json::json!({ "provider": null, "state": "none", "last_done": null }));
    }

    /// No harness reports, but the place's own session runs a program: that
    /// is `unknown`, with why — never `none`, which an orchestrator reads as
    /// "nobody is working there" (pi typed into a Claude lane answered `none`
    /// mid-turn, and `wait` returned at once). Any harness's reading wins, and
    /// a session back at its shell is still `none`.
    #[test]
    fn an_unclaimed_program_in_the_place_session_is_unknown_not_none() {
        let panes = |cmd: &str| tmux::PaneList::from_rows(vec![("repo-feat".into(), "/w/feat".into(), cmd.into())]);
        let got = place_answer([], Some(&panes("vim")), "repo-feat");
        assert_eq!(got.state, State::Unknown);
        assert_eq!(got.provider, None);
        assert_eq!(got.session.as_deref(), Some("repo-feat"));
        let why = got.reason.as_deref().unwrap_or("");
        assert!(why.contains("did not launch") && why.contains("(vim)"), "{why}");
        let v = serde_json::to_value(&got).unwrap();
        assert_eq!(v["state"], "unknown");
        assert!(v["reason"].is_string(), "{v}");
        assert_eq!(place_answer([], Some(&panes("zsh")), "repo-feat"), Activity::none(), "a shell is nobody");
        assert_eq!(place_answer([], None, "repo-feat"), Activity::none(), "no tmux, nothing to see");
        assert_eq!(place_answer([], Some(&panes("vim")), "repo-other"), Activity::none(), "another place's session");
        let claude = Activity { provider: Some("claude"), state: State::Idle, last_done: None, session: None, reason: None };
        assert_eq!(place_answer([claude.clone()], Some(&panes("vim")), "repo-feat"), claude, "a harness that reports wins");
        // Ranked between the live states and nothing at all.
        assert!(State::Idle.rank() < State::Unknown.rank() && State::Unknown.rank() < State::None.rank());
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

    #[test]
    fn real_codex_busy_and_modal_screens_do_not_read_as_idle() {
        let started = include_str!("../tests/fixtures/codex-send/probe-task-started.jsonl");
        let turn = codex::rollout_turn(&started.lines().map(str::to_string).collect::<Vec<_>>());
        assert_eq!(turn, Some(Turn::Busy));
        for (screen, expected) in [
            (include_str!("../tests/fixtures/codex-send/probe-busy.txt"), State::Busy),
            (include_str!("../tests/fixtures/codex-send/probe-approval.txt"), State::Waiting),
            (include_str!("../tests/fixtures/codex-send/probe-question-modal.txt"), State::Waiting),
            (include_str!("../tests/fixtures/codex-send/probe-question-banner-empty.txt"), State::Busy),
        ] {
            for command in ["codex", "node"] {
                let capture = format!("@@ 0 @@\n{command}\n{screen}");
                let panes = codex_panes_in(&capture, &["/scratch"]);
                assert_eq!(panes.len(), 1);
                assert_eq!(codex_state(turn.as_ref(), Some(panes[0].1)), (expected, None));
            }
        }
    }

    /// The rollout tail is re-read only when the file GROWS. Proven with a
    /// same-length rewrite that WOULD answer differently if read (`task_started`
    /// and `turn_aborted` are both 12 bytes): the cached Busy must still be
    /// served. Then growth is read, and answers Done.
    #[test]
    fn codex_tail_rereads_only_on_growth() {
        let d = std::env::temp_dir().join(format!("wtact-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("r.jsonl");
        let started = r#"{"type":"event_msg","payload":{"type":"task_started"}}"#;
        let aborted = r#"{"type":"event_msg","payload":{"type":"turn_aborted"}}"#;
        assert_eq!(started.len(), aborted.len());
        assert_eq!(codex::rollout_turn(&[aborted.to_string()]), Some(Turn::Aborted), "it WOULD answer differently");
        std::fs::write(&f, format!("{started}\n")).unwrap();
        assert_eq!(codex_tail(&f).1, Some(Turn::Busy));
        std::fs::write(&f, format!("{aborted}\n")).unwrap();
        assert_eq!(codex_tail(&f).1, Some(Turn::Busy), "same length: served from the cache, not re-read");
        let done = r#"{"type":"event_msg","payload":{"type":"task_complete","completed_at":123}}"#;
        std::fs::write(&f, format!("{started}\n{done}\n")).unwrap();
        assert_eq!(codex_tail(&f).1, Some(Turn::Done { at: Some(123), turn_id: None }), "growth is read");
        let _ = std::fs::remove_dir_all(&d);
    }
}
