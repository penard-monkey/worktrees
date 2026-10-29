//! Kimi Code (`kimi`): what worktrees reads of it, and how it types into it.
//! Every shape here was measured on kimi 2.2.0 (`docs/kimi-manual-checks.md`,
//! fixtures under `tests/fixtures/kimi-*`), never taken from its docs.
//!
//! - **Sessions** live in `<home>/sessions/wd_<slug>_<hash>/session_<uuid>/`,
//!   one workspace dir per launch directory. `state.json` names the session's
//!   `cwd`; `agents/main/wire.jsonl` is an append-only event log, and it is
//!   where every state comes from: `agent.turn.started` / `agent.turn.ended`
//!   bracket a turn, and an `interaction.request` (approval, question,
//!   user_tool) stays open until its `interaction.resolved` — so "waiting on
//!   you" is ON DISK, unlike Codex. A session dir appears at the first
//!   submitted message, not at launch.
//! - **The screen** is read only for what the log cannot say: the trust modal
//!   (it comes before any session exists) and whether the composer is there to
//!   type into. Every modal kimi draws REPLACES its composer box, so "is the
//!   composer the last thing on screen" is the one rule `send` and the opener
//!   need.
//! - **Nothing is written** under kimi's home. The model catalog lives in
//!   `kimimodels`; per-place files worktrees owns live in `kimiplace`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::activity::{self, Activity, State};
use crate::harness::SendOutcome;
use crate::tmux;

/// kimi's data root: `$KIMI_CODE_HOME`, else `~/.kimi-code` (kimi's own rule).
pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("KIMI_CODE_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(h);
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".kimi-code")
}

/// The `kimi` binary: `$WORKTREES_KIMI_BIN` (authoritative: one that is not
/// executable means none), the PATH, then the installer's own `<home>/bin/kimi`
/// — the installer adds that dir to an interactive shell's rc only, which a
/// GUI's login-shell PATH does not read.
pub fn kimi_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WORKTREES_KIMI_BIN").filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return crate::profile::is_exec(&p).then_some(p);
    }
    crate::profile::bin_on_path("kimi").or_else(|| {
        let p = home().join("bin").join("kimi");
        crate::profile::is_exec(&p).then_some(p)
    })
}

/// `kimi --version`'s output as a bare version: the first `N.N[.N…]` word, so
/// a launcher that prints `kimi, version 2.2.0` still reads `2.2.0`. `None`
/// when nothing in it looks like a version.
pub fn version_from(out: &str) -> Option<String> {
    out.split(|c: char| c.is_whitespace() || c == ',')
        .map(|w| w.trim_start_matches('v'))
        .find(|w| {
            let parts: Vec<&str> = w.split('.').collect();
            parts.len() >= 2 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
}

/// kimi's version, asked of the binary a pane would run — for Settings only:
/// it starts a kimi process, so nothing on a dialog's path calls it.
/// `Err` says why there is none, for the caller to log.
pub fn version() -> Result<String, String> {
    let bin = kimi_bin().ok_or("kimi is not installed")?;
    let mut cmd = crate::childenv::command(&bin);
    cmd.arg("--version").current_dir("/").stdin(std::process::Stdio::null());
    let out = crate::proc::run_deadline(cmd, 10).map_err(|e| format!("kimi --version: {e}"))?;
    if !out.status.success() {
        return Err(format!("kimi --version exited {}", out.status.code().unwrap_or(-1)));
    }
    version_from(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| "kimi --version printed no version".into())
}

/// kimi's `slugifyWorkDirName`: the workspace dir's readable half. Lowercased,
/// runs outside `[a-z0-9._-]` folded to `-`, trimmed, at most 40 characters.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    let mut dash = false;
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            s.push(c);
            dash = false;
        } else if !dash {
            s.push('-');
            dash = true;
        }
    }
    let trimmed: String = s.trim_matches('-').chars().take(40).collect();
    let trimmed = trimmed.trim_matches('-').to_string();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "workspace".into()
    } else {
        trimmed
    }
}

/// A session id as kimi mints them (`session_<uuid>`), and the only shape that
/// may reach `-S` argv.
pub fn valid_session_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("session_") else { return false };
    rest.len() == 36 && rest.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// One kimi session for a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiSession {
    pub id: String,
    pub dir: PathBuf,
    /// `createdAt` / `updatedAt`, epoch MILLISECONDS, from `state.json`.
    pub created_ms: i64,
    pub updated_ms: i64,
}

impl KimiSession {
    pub fn wire(&self) -> PathBuf {
        self.dir.join("agents").join("main").join("wire.jsonl")
    }
}

/// `state.json`'s (id, cwd, createdAt, updatedAt).
pub fn parse_state(text: &str) -> Option<(String, String, i64, i64)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let id = v.get("id")?.as_str()?.to_string();
    let cwd = v.get("cwd")?.as_str()?.to_string();
    let created = v.get("createdAt").and_then(|t| t.as_i64()).unwrap_or(0);
    let updated = v.get("updatedAt").and_then(|t| t.as_i64()).unwrap_or(created);
    Some((id, cwd, created, updated))
}

fn same_dir(a: &str, b: &str) -> bool {
    let (a, b) = (Path::new(a), Path::new(b));
    a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// Every session under `root` (kimi's `sessions/`) whose `cwd` is `cwd`. Only
/// the `wd_<slug>_*` workspace dirs are opened — the slug comes from the
/// directory's basename, and the `cwd` check settles two places whose names
/// slug alike.
pub fn sessions_in(root: &Path, cwd: &str) -> Vec<KimiSession> {
    let base = Path::new(cwd.trim_end_matches('/')).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let prefix = format!("wd_{}_", slug(&base));
    let Ok(workspaces) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out = Vec::new();
    for ws in workspaces.flatten() {
        if !ws.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        let Ok(sessions) = std::fs::read_dir(ws.path()) else { continue };
        for s in sessions.flatten() {
            let dir = s.path();
            let Ok(text) = std::fs::read_to_string(dir.join("state.json")) else { continue };
            let Some((id, scwd, created_ms, updated_ms)) = parse_state(&text) else { continue };
            if valid_session_id(&id) && same_dir(&scwd, cwd) {
                out.push(KimiSession { id, dir, created_ms, updated_ms });
            }
        }
    }
    out
}

/// Sessions for `cwd` in workspace dirs whose name is NOT the slug
/// `sessions_in` derives — what a divergence from kimi's own
/// `slugifyWorkDirName` (a newer kimi, a name the port gets wrong) would
/// leave unseen: no resume, no dot, and nothing else saying why. Opens every
/// workspace dir, so `doctor --kimi` asks, never the poll.
pub fn sessions_missed(root: &Path, cwd: &str) -> usize {
    let base = Path::new(cwd.trim_end_matches('/')).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let prefix = format!("wd_{}_", slug(&base));
    let Ok(workspaces) = std::fs::read_dir(root) else { return 0 };
    workspaces
        .flatten()
        .filter(|ws| {
            let n = ws.file_name().to_string_lossy().into_owned();
            n.starts_with("wd_") && !n.starts_with(&prefix)
        })
        .filter_map(|ws| std::fs::read_dir(ws.path()).ok())
        .flat_map(|d| d.flatten())
        .filter_map(|s| std::fs::read_to_string(s.path().join("state.json")).ok())
        .filter_map(|t| parse_state(&t))
        .filter(|(id, scwd, ..)| valid_session_id(id) && same_dir(scwd, cwd))
        .count()
}

/// When the place's directory was made, in epoch ms — `None` where the
/// filesystem cannot say. A place removed and re-created under the same slug
/// has the same path, so the same kimi workspace: without this, `-c` and every
/// read here would pick up the dead lane's conversation.
fn place_born_ms(cwd: &str) -> Option<i64> {
    let t = std::fs::metadata(cwd).ok()?.created().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
}

/// The place's current session: the most recently updated one created no
/// earlier than the place itself. `None` when kimi has written nothing here.
pub fn current_session_since(root: &Path, cwd: &str, born_ms: Option<i64>) -> Option<KimiSession> {
    sessions_in(root, cwd)
        .into_iter()
        .filter(|s| born_ms.is_none_or(|b| s.created_ms >= b))
        .max_by(|a, b| (a.updated_ms, &a.id).cmp(&(b.updated_ms, &b.id)))
}

pub fn sessions_root() -> PathBuf {
    home().join("sessions")
}

pub fn current_session(cwd: &str) -> Option<KimiSession> {
    current_session_since(&sessions_root(), cwd, place_born_ms(cwd))
}

// ── the wire log ─────────────────────────────────────────────────────────────

/// Where a session's newest turn stands, from its wire log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KimiTurn {
    /// A turn is running.
    Busy,
    /// A turn is running and stopped on an approval, a question or a tool
    /// that asks the user (an `interaction.request` with no resolution yet).
    Waiting,
    /// The last turn finished (`outcome: done`) at `at`, epoch seconds — the
    /// record's own `time`, never the file's mtime.
    Done { at: Option<i64> },
    /// Esc (`outcome: aborted`), or anything else that ended a turn without
    /// finishing it. Not finished work.
    Aborted,
}

fn ty(v: &serde_json::Value) -> &str {
    v.get("type").and_then(|t| t.as_str()).unwrap_or("")
}

/// The newest turn among `lines` (oldest first), or `None` when they hold no
/// event at all. A tail with events but no turn boundary is a turn that began
/// before the tail did — an ended turn always has its end within reach, since
/// only a handful of bookkeeping records follow `agent.turn.ended`.
pub fn wire_turn(lines: &[String]) -> Option<KimiTurn> {
    let mut resolved: Vec<String> = Vec::new();
    let mut open = false;
    let mut any = false;
    for l in lines.iter().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        any = true;
        let id = || v.get("id").and_then(|i| i.as_str()).map(str::to_string);
        match ty(&v) {
            "interaction.resolved" => resolved.extend(id()),
            "interaction.request" => {
                if id().is_some_and(|i| !resolved.contains(&i)) {
                    open = true;
                }
            }
            "agent.turn.ended" => {
                return Some(match v.get("outcome").and_then(|o| o.as_str()) {
                    Some("done") if !open => KimiTurn::Done { at: v.get("time").and_then(|t| t.as_i64()).map(|ms| ms / 1000) },
                    _ if open => KimiTurn::Waiting,
                    _ => KimiTurn::Aborted,
                });
            }
            "agent.turn.started" => return Some(if open { KimiTurn::Waiting } else { KimiTurn::Busy }),
            _ => {}
        }
    }
    any.then_some(if open { KimiTurn::Waiting } else { KimiTurn::Busy })
}

/// The model a session is on: the newest `modelAlias` among `config.update`
/// (a `/model` switch, written AT the switch), `profile.bind` (the session's
/// start) and `llm.request` (every request). kimi's own alias, so it can be
/// handed straight back to `-m`.
pub fn wire_model(lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|l| {
        if !l.contains("\"modelAlias\"") {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        match ty(&v) {
            "config.update" | "profile.bind" | "llm.request" => v.get("modelAlias")?.as_str().map(str::to_string),
            _ => None,
        }
    })
}

/// Whitespace collapsed, so a wrapped composer and a logged prompt compare.
pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a typed text is recognised by: its first 80 characters, normalised.
pub fn send_prefix(typed: &str) -> String {
    normalize(typed).chars().take(80).collect()
}

fn starts(text: &str, prefix: &str) -> bool {
    !prefix.is_empty() && normalize(text).starts_with(prefix)
}

/// `turn.prompt` records — the user's submitted messages, written when each
/// one's turn STARTS (a message queued behind a running turn has none yet) —
/// whose text starts with `prefix`.
pub fn prompts_in(lines: &[String], prefix: &str) -> usize {
    lines
        .iter()
        .filter(|l| l.contains("\"turn.prompt\""))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| ty(v) == "turn.prompt")
        .filter_map(|v| {
            let parts = v.get("input")?.as_array()?;
            Some(parts.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""))
        })
        .filter(|t| starts(t, prefix))
        .count()
}

/// Whether `lines` hold any user prompt at all.
pub fn any_prompt(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("\"turn.prompt\""))
}

#[derive(Debug, Clone, Default)]
struct TailInfo {
    model: Option<String>,
    turn: Option<KimiTurn>,
}

type TailCache = HashMap<PathBuf, (u64, TailInfo)>;
static KIMI_TAIL: Mutex<Option<TailCache>> = Mutex::new(None);

/// A wire log's (model, turn), re-read only once the file has GROWN — the
/// shape of `activity::codex_tail`. A tail that names no model keeps the last
/// one read: one long tool result filled the window, nothing switched.
pub fn session_tail(path: &Path) -> (Option<String>, Option<KimiTurn>) {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
        return (None, None);
    };
    let mut guard = KIMI_TAIL.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some((l, info)) = cache.get(path) {
        if *l == len {
            return (info.model.clone(), info.turn.clone());
        }
    }
    let prev = cache.get(path).map(|(_, i)| i.clone()).unwrap_or_default();
    let lines = activity::tail_lines_checked(path, activity::ROLLOUT_TAIL_BYTES).unwrap_or_default();
    let info = TailInfo { model: wire_model(&lines).or(prev.model), turn: wire_turn(&lines).or(prev.turn) };
    cache.insert(path.to_path_buf(), (len, info.clone()));
    (info.model, info.turn)
}

fn tail(path: &Path) -> Vec<String> {
    activity::tail_lines_checked(path, activity::ROLLOUT_TAIL_BYTES).unwrap_or_default()
}

// ── the screen ───────────────────────────────────────────────────────────────

/// What one capture of a kimi pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KimiScreen {
    /// The pane is back at a shell: kimi exited.
    Gone,
    /// The composer box is the last thing drawn: kimi takes typed input
    /// (mid-turn it queues it).
    Composer,
    /// "Trust this folder?" — before any session exists. Its highlighted
    /// choice is **Trust and continue**, so an Enter typed into it trusts the
    /// place and lets a repo's `.mcp.json` start its servers.
    Trust,
    /// Any other block kimi closes with a rule where the composer was: an
    /// approval, a question, a picker. Typed keys answer it.
    Modal,
    /// Nothing recognisable yet (startup, a blank pane).
    Other,
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.chars().all(|c| c == '─')
}

fn is_box_bottom(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('╰') && t.ends_with('╯')
}

fn is_box_top(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('╭') && t.ends_with('╮')
}

/// Read a kimi screen by POSITION: whichever comes last, the composer box's
/// bottom edge or a full rule closing a block. History can quote a trust
/// prompt or an approval; it cannot draw below the composer.
pub fn read_screen(screen: &str) -> KimiScreen {
    let lines: Vec<&str> = screen.lines().collect();
    let rule = lines.iter().rposition(|l| is_rule(l));
    let boxed = lines.iter().rposition(|l| is_box_bottom(l));
    match (rule, boxed) {
        (_, Some(b)) if rule.is_none_or(|r| b > r) => KimiScreen::Composer,
        (Some(r), _) => {
            let start = lines[..r].iter().rposition(|l| is_rule(l)).map_or(0, |i| i + 1);
            if lines[start..r].iter().any(|l| l.trim() == "Trust this folder?") {
                KimiScreen::Trust
            } else {
                KimiScreen::Modal
            }
        }
        _ => KimiScreen::Other,
    }
}

/// kimi's composer, read by position: the input inside the last box, and the
/// messages kimi has queued behind a running turn, which it lists right above
/// the box (`❯ <text>` lines, anchored by its `↑ to edit · ctrl-s to steer`
/// hint — without the hint there is no queue, so a transcript line quoting
/// `❯` can never be read as one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composer {
    pub input: String,
    pub queued: Vec<String>,
}

pub fn read_composer(screen: &str) -> Option<Composer> {
    if read_screen(screen) != KimiScreen::Composer {
        return None;
    }
    let lines: Vec<&str> = screen.lines().collect();
    let bottom = lines.iter().rposition(|l| is_box_bottom(l))?;
    let top = lines[..bottom].iter().rposition(|l| is_box_top(l))?;
    let inner: Vec<String> = lines[top + 1..bottom]
        .iter()
        .map(|l| {
            let t = l.trim();
            let t = t.strip_prefix('│').unwrap_or(t);
            let t = t.strip_suffix('│').unwrap_or(t).trim();
            t.strip_prefix('>').unwrap_or(t).trim().to_string()
        })
        .collect();
    let input = normalize(&inner.join(" "));
    let mut queued = Vec::new();
    let mut above = lines[..top].iter().rev().map(|l| l.trim()).skip_while(|t| t.is_empty());
    if above.next().is_some_and(|t| t.starts_with('↑') && t.contains("to edit")) {
        for t in above {
            match t.strip_prefix("❯ ") {
                Some(text) => queued.push(normalize(text)),
                None => break,
            }
        }
    }
    queued.reverse();
    Some(Composer { input, queued })
}

/// One capture of a kimi pane, read: its current command first (a shell means
/// kimi is gone — the pane is `kimi …; exec "$SHELL"`), then the screen.
pub fn screen_from(cmd: &str, screen: &str) -> KimiScreen {
    if tmux::is_shell_command(cmd) {
        KimiScreen::Gone
    } else {
        read_screen(screen)
    }
}


/// A kimi lane's state from its session's newest turn and one capture of its
/// pane. The screen answers first where only it can: a shell (kimi exited) and
/// the trust modal, which comes before any session file. A modal kimi drew
/// mid-turn is waiting; one opened between turns (the user in `/model`) is not
/// — nobody is waiting on the user there, though `send` still refuses it. The
/// log answers the rest, and a composer on screen overrules a log that says
/// "waiting": every modal replaces the composer.
pub fn kimi_state(turn: Option<&KimiTurn>, screen: Option<KimiScreen>) -> (State, Option<i64>) {
    match screen {
        Some(KimiScreen::Gone) => return (State::None, None),
        Some(KimiScreen::Trust) => return (State::Waiting, None),
        Some(KimiScreen::Modal) if matches!(turn, Some(KimiTurn::Busy | KimiTurn::Waiting)) => {
            return (State::Waiting, None)
        }
        _ => {}
    }
    match turn {
        Some(KimiTurn::Waiting) if screen == Some(KimiScreen::Composer) => (State::Busy, None),
        Some(KimiTurn::Waiting) => (State::Waiting, None),
        Some(KimiTurn::Busy) => (State::Busy, None),
        Some(KimiTurn::Done { at }) => (State::Idle, *at),
        Some(KimiTurn::Aborted) | None => (State::Idle, None),
    }
}

/// The kimi session name for a place: its `~agent~kimi` sidecar, or the
/// canonical session when kimi is what runs there.
pub fn session_for(panes: &tmux::PaneList, canonical: &str) -> String {
    crate::provider::KIMI.session_name(canonical, panes.canonical_provider(canonical).id, false)
}

/// The kimi half of `place_activity` for one place: `None` unless the place's
/// kimi session is up and running something other than a shell.
pub fn kimi_activity(panes: &tmux::PaneList, canonical: &str, path: &str) -> Option<Activity> {
    let name = session_for(panes, canonical);
    let pending = opener_pending(path);
    if !panes.session_runs_program(&name) {
        // The pane's shell is still starting kimi (it sources the user's rc
        // first): with an opener on its way the lane is about to work, and
        // "nobody is there" would let `wait` answer before it began.
        return before_start(name.clone(), pending, panes.has_session(&name));
    }
    let turn = current_session(path).and_then(|s| session_tail(&s.wire()).1);
    let capture = activity::capture_one(&name);
    let screen = capture.as_ref().map(|(cmd, text)| screen_from(cmd, text));
    let (state, last_done) = lane_state(turn.as_ref(), screen, pending);
    let reason = capture.as_ref().filter(|_| state == State::Waiting).and_then(|(_, text)| activity::waiting_reason("kimi", text));
    Some(Activity { provider: Some("kimi"), state, last_done, session: Some(name), reason })
}

/// The reading for a kimi session whose pane is still a shell: busy while an
/// opener is on its way into a session that exists, else nobody there.
pub fn before_start(session: String, pending: bool, session_up: bool) -> Option<Activity> {
    (pending && session_up).then(|| Activity {
        provider: Some("kimi"),
        state: State::Busy,
        last_done: None,
        session: Some(session),
        reason: None,
    })
}

/// THE kimi reading for a running lane: its log's newest turn, one capture of
/// its pane, and whether its opener is pending. `place_status`/`wait`
/// (`kimi_activity`) and the app's dot tick (`kimi_tick_in`) both call this,
/// each with a capture of its own — never a reading without one, or a kimi
/// that died mid-turn (`Gone`) stays busy on the dot while `place_status`
/// says it is gone.
pub fn lane_state(turn: Option<&KimiTurn>, screen: Option<KimiScreen>, pending: bool) -> (State, Option<i64>) {
    with_pending_opener(kimi_state(turn, screen), pending)
}

/// A lane whose opener has not been typed yet is not idle: it is about to
/// start the turn its brief asks for, and `wait until: idle` answering in the
/// gap would hand an orchestrator a lane that has done nothing. Everything
/// else stands — the trust modal (waiting on the user) above all.
pub fn with_pending_opener(reading: (State, Option<i64>), pending: bool) -> (State, Option<i64>) {
    match reading {
        (State::Idle, _) if pending => (State::Busy, None),
        r => r,
    }
}

// ── the pending-opener marker ────────────────────────────────────────────────

fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// `$XDG_STATE_HOME/worktrees/kimi-opener/<hash of the place path>`: written
/// by the launch that starts a helper, removed by the helper when it is done.
pub fn opener_pending_path(wt: &str) -> Option<PathBuf> {
    Some(crate::init::state_dir()?.join("kimi-opener").join(format!("{:016x}", fnv1a(wt.trim_end_matches('/')))))
}

pub fn mark_opener_pending(wt: &str) {
    if let Some(p) = opener_pending_path(wt) {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, wt);
    }
}

pub fn clear_opener_pending(wt: &str) {
    if let Some(p) = opener_pending_path(wt) {
        let _ = std::fs::remove_file(p);
    }
}

/// A marker younger than the helper's whole bound: one a helper that died
/// without clearing it leaves behind stops counting on its own.
pub fn opener_pending(wt: &str) -> bool {
    let Some(p) = opener_pending_path(wt) else { return false };
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_millis() < u128::from(OPENER_WAIT_MS + OPENER_START_GRACE_MS))
}

/// The model the place's current kimi session is on, from its wire log.
pub fn running_model(path: &str) -> Option<String> {
    session_tail(&current_session(path)?.wire()).0
}

/// What a place's current session says, for a caller that polls it (the app's
/// dot tick): which wire log (path + length, so "has it moved" is a
/// comparison), its model and its newest turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneFile {
    pub path: PathBuf,
    pub len: u64,
    pub model: Option<String>,
    pub turn: Option<KimiTurn>,
}

pub fn lane_file(path: &str) -> Option<LaneFile> {
    let s = current_session(path)?;
    let wire = s.wire();
    let len = std::fs::metadata(&wire).ok()?.len();
    let (model, turn) = session_tail(&wire);
    Some(LaneFile { path: wire, len, model, turn })
}

// ── typing into it ───────────────────────────────────────────────────────────

/// Where a receipt is looked for: the place's current session id and how many
/// of its prompts start with the prefix. A first message CREATES the session,
/// so a receipt is either more matching prompts in the same session or any in
/// a new one.
pub type Receipts = (Option<String>, usize);

pub fn receipts(cwd: &str, prefix: &str) -> Receipts {
    match current_session(cwd) {
        Some(s) => {
            let n = prompts_in(&tail(&s.wire()), prefix);
            (Some(s.id), n)
        }
        None => (None, 0),
    }
}

fn received(before: &Receipts, now: &Receipts) -> bool {
    match (&before.0, &now.0) {
        (b, n) if b == n => now.1 > before.1,
        (_, Some(_)) => now.1 > 0,
        _ => false,
    }
}

/// Press Enter on text already typed into kimi's composer, and confirm it —
/// `pi::submit_pi`'s loop with kimi's receipts. Settling watches the COMPOSER
/// only (mid-turn the transcript streams). Every Enter needs a fresh screen
/// whose composer holds OUR text, settled; any modal ends it with no key
/// pressed; a failed capture never authorizes a keypress or counts as a
/// receipt. Receipts: a `turn.prompt` starting with `prefix` (`Submitted`), or
/// the composer empty and kimi's queue holding it (`Queued` — kimi keeps that
/// queue in memory and logs the prompt only when its turn starts).
#[allow(clippy::too_many_arguments)]
pub fn submit_kimi(
    prefix: &str,
    exact: Option<&str>,
    baseline: Receipts,
    mut capture: impl FnMut() -> Option<String>,
    mut receipts: impl FnMut() -> Receipts,
    mut enter: impl FnMut() -> Result<(), String>,
    now: impl Fn() -> u64,
    mut sleep: impl FnMut(u64),
) -> SendOutcome {
    use crate::harness::{SEND_ENTER_TRIES, SEND_POLL_MS, SEND_STABLE_MS, SEND_TIMEOUT_MS, SEND_VERIFY_MS};
    let start = now();
    let mut previous: Option<String> = None;
    let mut queued_before = 0;
    let mut stable_since = start;
    let mut tries = 0;
    let mut last_enter: Option<u64> = None;
    let ours = |t: &str| {
        let shown = normalize(t.trim_end_matches("...").trim_end_matches('…'));
        shown.chars().count() >= prefix.chars().count().min(24) && (shown.starts_with(prefix) || prefix.starts_with(&shown))
    };
    loop {
        let time = now();
        if time.saturating_sub(start) >= SEND_TIMEOUT_MS {
            return SendOutcome::Unconfirmed;
        }
        match capture() {
            Some(screen) => {
                // After an Enter the log answers first: an approval that opens
                // within one poll of the turn starting is a DELIVERED message
                // whose turn is now waiting, not an unconfirmed one.
                if tries > 0 && received(&baseline, &receipts()) {
                    return SendOutcome::Submitted;
                }
                if matches!(read_screen(&screen), KimiScreen::Trust | KimiScreen::Modal) {
                    return SendOutcome::Modal;
                }
                let composer = read_composer(&screen);
                if tries > 0 {
                    if composer.as_ref().is_some_and(|c| c.input.is_empty() && c.queued.iter().filter(|t| ours(t)).count() > queued_before) {
                        return SendOutcome::Queued;
                    }
                } else if let Some(c) = &composer {
                    queued_before = c.queued.iter().filter(|t| ours(t)).count();
                }
                let input = composer.map(|c| c.input);
                // With the whole text known (the opener), only that text may be
                // submitted: anything the user typed after it in the gap before
                // Enter would ride along as the same turn. `send` matches its
                // prefix, since a long message wraps in the composer.
                let typed = input.as_deref().is_some_and(|i| match exact {
                    Some(t) => normalize(i) == normalize(t),
                    None => starts(i, prefix),
                });
                if !typed || previous != input {
                    stable_since = time;
                } else if time.saturating_sub(stable_since) >= SEND_STABLE_MS
                    && last_enter.is_none_or(|at| time.saturating_sub(at) >= SEND_VERIFY_MS)
                {
                    if tries == SEND_ENTER_TRIES {
                        return SendOutcome::Unconfirmed;
                    }
                    if let Err(e) = enter() {
                        return SendOutcome::EnterFailed(e);
                    }
                    tries += 1;
                    last_enter = Some(time);
                    stable_since = time;
                }
                previous = input;
            }
            None => {
                previous = None;
                stable_since = time;
            }
        }
        sleep(SEND_POLL_MS);
    }
}

// ── the deferred opener ──────────────────────────────────────────────────────

/// How long the opener waits for kimi to be ready (a trust modal answered by
/// hand can take a while) before it gives up and says so.
pub const OPENER_WAIT_MS: u64 = 10 * 60 * 1000;
const OPENER_POLL_MS: u64 = 500;
/// How long a pane may still read as a shell before kimi counts as not having
/// started at all (kimi 2.2.0 is up in under a second).
const OPENER_START_GRACE_MS: u64 = 30_000;

/// How a deferred opener went. Every outcome but `Delivered` leaves the brief
/// unread, and is logged with its reason.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenerOutcome {
    /// Typed, submitted, and seen in the wire log (`turn.prompt`).
    Delivered,
    /// Typed and submitted, but kimi queued it behind a turn already running.
    Queued,
    /// kimi exited before it was ready.
    Gone,
    /// Someone started the conversation first: a prompt exists that is not
    /// ours. The opener is not added on top of it.
    TookOver,
    /// kimi was never at an empty composer within the bound.
    TimedOut(&'static str),
    /// Typed, but the submit could not be confirmed (a modal opened, Enter
    /// failed, or no receipt came).
    Unconfirmed(String),
    /// Typing failed outright.
    Failed(String),
}

impl OpenerOutcome {
    pub fn line(&self) -> String {
        match self {
            Self::Delivered => "delivered".into(),
            Self::Queued => "delivered (queued behind a running turn)".into(),
            Self::Gone => "not delivered: kimi exited before it was ready".into(),
            Self::TookOver => "not delivered: the conversation was started by hand first".into(),
            Self::TimedOut(why) => format!("not delivered: gave up waiting ({why})"),
            Self::Unconfirmed(why) => format!("typed but not confirmed: {why}"),
            Self::Failed(why) => format!("not delivered: {why}"),
        }
    }
}

/// Type `opener` into kimi once it sits at an EMPTY composer, never into a
/// non-empty one and never over a modal; then confirm it with `submit_kimi`.
/// Waits through startup, the trust modal and a user typing, up to `wait_ms`.
/// Seams: `capture` gives (pane command, screen); `prompts` answers whether
/// ANY prompt exists for the place yet and the receipts for `opener`.
#[allow(clippy::too_many_arguments)]
pub fn deliver_opener(
    opener: &str,
    wait_ms: u64,
    mut capture: impl FnMut() -> Option<(String, String)>,
    mut started: impl FnMut() -> bool,
    mut receipts: impl FnMut() -> Receipts,
    mut type_text: impl FnMut(&str) -> Result<(), String>,
    mut enter: impl FnMut() -> Result<(), String>,
    now: impl Fn() -> u64,
    mut sleep: impl FnMut(u64),
) -> OpenerOutcome {
    let start = now();
    let prefix = send_prefix(opener);
    let mut last = "kimi never drew its composer";
    // The helper is started by the pane's own shell just BEFORE it runs kimi,
    // so a shell on the first captures is kimi not having started yet. Only a
    // shell after kimi was seen — or one that never gave way — is an exit.
    let mut seen = false;
    // The last time the pane answered at all: a closed session answers
    // nothing, and that is an exit too, not a reason to wait out the bound.
    let mut answered = start;
    loop {
        if now().saturating_sub(start) >= wait_ms {
            return OpenerOutcome::TimedOut(last);
        }
        let Some((cmd, screen)) = capture() else {
            if now().saturating_sub(answered) >= OPENER_START_GRACE_MS {
                return OpenerOutcome::Gone;
            }
            sleep(OPENER_POLL_MS);
            continue;
        };
        answered = now();
        {
            let read = screen_from(&cmd, &screen);
            if read == KimiScreen::Gone && !seen {
                if now().saturating_sub(start) >= OPENER_START_GRACE_MS {
                    return OpenerOutcome::Gone;
                }
                sleep(OPENER_POLL_MS);
                continue;
            }
            seen = true;
            match read {
                KimiScreen::Gone => return OpenerOutcome::Gone,
                KimiScreen::Trust => last = "the trust prompt was never answered",
                KimiScreen::Modal => last = "a prompt stayed open",
                KimiScreen::Other => {}
                KimiScreen::Composer => {
                    if started() {
                        return OpenerOutcome::TookOver;
                    }
                    match read_composer(&screen) {
                        Some(c) if c.input.is_empty() && c.queued.is_empty() => {
                            let baseline = receipts();
                            if let Err(e) = type_text(opener) {
                                return OpenerOutcome::Failed(e);
                            }
                            let t0 = now();
                            return match submit_kimi(
                                &prefix,
                                Some(opener),
                                baseline,
                                || capture().map(|(_, s)| s),
                                &mut receipts,
                                &mut enter,
                                || now().saturating_sub(t0),
                                &mut sleep,
                            ) {
                                SendOutcome::Submitted => OpenerOutcome::Delivered,
                                SendOutcome::Queued => OpenerOutcome::Queued,
                                SendOutcome::Modal => OpenerOutcome::Unconfirmed("a prompt opened while submitting".into()),
                                SendOutcome::EnterFailed(e) => OpenerOutcome::Unconfirmed(format!("Enter failed: {e}")),
                                _ => OpenerOutcome::Unconfirmed("no turn.prompt appeared".into()),
                            };
                        }
                        _ => last = "the composer was never empty",
                    }
                }
            }
        }
        sleep(OPENER_POLL_MS);
    }
}

/// The `worktrees` binary that has the opener helper (`kimi-opener --probe`
/// answers), or `None`. Asked per launch, not cached: the app outlives a CLI
/// upgrade, and an older CLI on PATH would answer "Unknown command".
pub fn opener_bin() -> Option<PathBuf> {
    let bin = crate::profile::worktrees_bin()?;
    let mut cmd = crate::childenv::command(&bin);
    cmd.args(["kimi-opener", "--probe"]).current_dir("/").stdin(std::process::Stdio::null());
    let out = crate::proc::run_deadline(cmd, 5).ok()?;
    (out.status.success() && String::from_utf8_lossy(&out.stdout).starts_with(OPENER_PROBE)).then_some(bin)
}

/// What `kimi-opener --probe` prints.
pub const OPENER_PROBE: &str = "kimi-opener 1";

/// `worktrees kimi-opener --pane %N --dir <place> --since <epoch ms>` — the
/// helper a fresh kimi launch with a brief starts in its own pane
/// (`ops::deferred_opener`). Types `ops::BRIEF_OPENER` (fixed text, never an
/// argument) once kimi is at an empty composer, and logs what happened.
pub fn cmd_opener(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--probe") {
        println!("{OPENER_PROBE}");
        return 0;
    }
    let val = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let (Some(pane), Some(dir)) = (val("--pane").and_then(|p| tmux::PaneId::parse(&p)), val("--dir")) else {
        eprintln!("usage: worktrees kimi-opener --pane %N --dir <place> --since <epoch ms>");
        return 2;
    };
    let since: i64 = val("--since").and_then(|s| s.parse().ok()).unwrap_or(0);
    let wait = std::env::var("WORKTREES_KIMI_OPENER_WAIT_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(OPENER_WAIT_MS);
    let opener = crate::ops::BRIEF_OPENER;
    let prefix = send_prefix(opener);
    let root = sessions_root();
    let t0 = std::time::Instant::now();
    let outcome = deliver_opener(
        opener,
        wait,
        || activity::capture_target(pane.as_str()),
        // Any prompt in a session made since the launch: the user went first.
        || sessions_in(&root, &dir).iter().any(|s| s.created_ms >= since && any_prompt(&tail(&s.wire()))),
        || receipts(&dir, &prefix),
        |t| tmux::send_literal(&pane, t),
        || tmux::press_enter(&pane),
        || t0.elapsed().as_millis() as u64,
        |ms| std::thread::sleep(std::time::Duration::from_millis(ms)),
    );
    let place = if dir.contains("/.worktrees/") {
        Path::new(&dir).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        "(main)".to_string()
    };
    clear_opener_pending(&dir);
    if let Some(common) = crate::codex::git_common_dir(&dir) {
        log_opener(&common, &place, &outcome);
    }
    0
}

/// Where opener outcomes are logged: the repo's private git dir, beside the
/// agent-switch lock. One line per launch; it never holds the opener's text
/// (fixed anyway) or anything kimi wrote.
pub fn opener_log_path(git_common: &str) -> PathBuf {
    Path::new(git_common).join("worktrees-kimi-opener.log")
}

pub fn log_opener(git_common: &str, place: &str, outcome: &OpenerOutcome) {
    use std::io::Write;
    let line = format!("{} {place}: {}\n", crate::sysclock::stamp_now(), outcome.line());
    crate::logsink::log("info", &format!("kimi opener for {place}: {}", outcome.line()));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(opener_log_path(git_common)) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// The most recent opener line for `place`, for `doctor --kimi`.
pub fn last_opener_line(git_common: &str, place: &str) -> Option<String> {
    let text = std::fs::read_to_string(opener_log_path(git_common)).ok()?;
    let key = format!(" {place}: ");
    text.lines().rev().find(|l| l.contains(&key)).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name),
        )
        .unwrap()
    }

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn a_version_is_the_version_whatever_the_launcher_says_around_it() {
        assert_eq!(version_from("2.2.0\n").as_deref(), Some("2.2.0"));
        assert_eq!(version_from("kimi, version 2.2.0").as_deref(), Some("2.2.0"));
        assert_eq!(version_from("kimi-code v2.10.1 (darwin-arm64)").as_deref(), Some("2.10.1"));
        assert_eq!(version_from("no version here"), None);
        assert_eq!(version_from("1."), None);
    }

    #[test]
    fn slugs_are_kimis() {
        assert_eq!(slug("feat"), "feat");
        assert_eq!(slug("Naruto Shippuuden"), "naruto-shippuuden");
        assert_eq!(slug("(main)"), "main");
        assert_eq!(slug("a..b"), "a..b");
        assert_eq!(slug("..."), "...");
        assert_eq!(slug("--"), "workspace");
        assert_eq!(slug(""), "workspace");
        assert_eq!(slug(&"x".repeat(50)).len(), 40);
    }

    #[test]
    fn a_session_id_is_kimis_shape_or_nothing() {
        assert!(valid_session_id("session_c8ce7b2e-8f8a-4cd2-88e8-c0ca9bbbbf5e"));
        for bad in ["c8ce7b2e-8f8a-4cd2-88e8-c0ca9bbbbf5e", "session_x", "session_c8ce7b2e-8f8a-4cd2-88e8-c0ca9bbbbf5e'; rm", ""] {
            assert!(!valid_session_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_place_session_is_its_cwds_newest_and_none_older_than_the_place() {
        let t = std::env::temp_dir().join(format!("kimi-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let place = t.join("repo/.worktrees/feat");
        std::fs::create_dir_all(&place).unwrap();
        let cwd = place.to_string_lossy().into_owned();
        let other = t.join("elsewhere/feat").to_string_lossy().into_owned();
        let mk = |ws: &str, id: &str, cwd: &str, created: i64, updated: i64| {
            let d = t.join("sessions").join(ws).join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("state.json"),
                serde_json::json!({"id": id, "cwd": cwd, "createdAt": created, "updatedAt": updated}).to_string(),
            )
            .unwrap();
        };
        let a = "session_00000000-0000-0000-0000-00000000000a";
        let b = "session_00000000-0000-0000-0000-00000000000b";
        let c = "session_00000000-0000-0000-0000-00000000000c";
        let d = "session_00000000-0000-0000-0000-00000000000d";
        mk("wd_feat_111111111111", a, &cwd, 100, 500);
        mk("wd_feat_111111111111", b, &cwd, 200, 300);
        // Same slug, another directory: never this place's.
        mk("wd_feat_222222222222", c, &other, 300, 900);
        // A dead lane's session from before the place was (re)made.
        mk("wd_feat_111111111111", d, &cwd, 10, 1000);
        let root = t.join("sessions");
        assert_eq!(current_session_since(&root, &cwd, Some(50)).map(|s| s.id).as_deref(), Some(a));
        assert_eq!(current_session_since(&root, &cwd, None).map(|s| s.id).as_deref(), Some(d));
        assert_eq!(current_session_since(&root, &cwd, Some(1_000)), None);
        assert_eq!(sessions_in(&root, &other).len(), 1);
        assert_eq!(sessions_missed(&root, &cwd), 0, "every one of them is under the derived slug");
        // A workspace dir named some other way (kimi changed its slug rule):
        // the discovery misses it, and doctor can say so.
        mk("wd_FEAT-renamed_333333333333", "session_00000000-0000-0000-0000-00000000000e", &cwd, 400, 400);
        assert_eq!(sessions_missed(&root, &cwd), 1);
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_turn_reads_from_the_wire_log() {
        let w = lines(&fixture("kimi-wire/2.2.0/turns.jsonl"));
        // Through the first turn's end: done, dated by the record (ms → s).
        let first_end = w.iter().position(|l| l.contains("\"agent.turn.ended\"")).unwrap();
        assert_eq!(wire_turn(&w[..=first_end]), Some(KimiTurn::Done { at: Some(1792431731) }));
        // An approval open mid-turn is waiting; resolved, the turn is busy again.
        let req = w.iter().position(|l| l.contains("\"interaction.request\"")).unwrap();
        assert_eq!(wire_turn(&w[..=req]), Some(KimiTurn::Waiting));
        assert_eq!(wire_turn(&w[..=req + 1]), Some(KimiTurn::Busy));
        // The whole log ends on an Esc: aborted, not finished work.
        assert_eq!(wire_turn(&w), Some(KimiTurn::Aborted));
        // A tail that starts mid-turn, with no boundary in it, is a running turn.
        assert_eq!(wire_turn(&lines(r#"{"type":"llm.request","modelAlias":"m"}"#)), Some(KimiTurn::Busy));
        assert_eq!(wire_turn(&[]), None);
    }

    #[test]
    fn a_question_is_waiting_too() {
        let w = lines(&fixture("kimi-wire/2.2.0/question.jsonl"));
        let req = w.iter().position(|l| l.contains("\"kind\":\"question\"")).unwrap();
        assert_eq!(wire_turn(&w[..=req]), Some(KimiTurn::Waiting));
        assert_eq!(wire_turn(&w), Some(KimiTurn::Done { at: Some(1792461769) }));
    }

    #[test]
    fn the_model_is_the_newest_alias_and_a_switch_counts_at_once() {
        let w = lines(&fixture("kimi-wire/2.2.0/turns.jsonl"));
        assert_eq!(wire_model(&w).as_deref(), Some("kimi-code/kimi-for-coding-highspeed"));
        let s = lines(&fixture("kimi-wire/2.2.0/model-switch.jsonl"));
        // `config.update` is written at the switch, before any request.
        assert_eq!(wire_model(&s).as_deref(), Some("kimi-code/kimi-for-coding"));
    }

    #[test]
    fn prompts_are_counted_by_prefix() {
        let w = lines(&fixture("kimi-wire/2.2.0/turns.jsonl"));
        assert_eq!(prompts_in(&w, &send_prefix("Run ls, then reply DONE.")), 1);
        assert_eq!(prompts_in(&w, &send_prefix("nothing like it")), 0);
        assert!(any_prompt(&w));
        assert!(!any_prompt(&lines(r#"{"type":"metadata"}"#)));
    }

    #[test]
    fn every_measured_screen_reads_as_what_it_is() {
        let s = |n: &str| read_screen(&fixture(&format!("kimi-screen/2.2.0/{n}.txt")));
        assert_eq!(s("idle"), KimiScreen::Composer);
        assert_eq!(s("busy-typed"), KimiScreen::Composer);
        assert_eq!(s("busy-queued"), KimiScreen::Composer);
        assert_eq!(s("composer-typed"), KimiScreen::Composer);
        assert_eq!(s("trust"), KimiScreen::Trust);
        assert_eq!(s("approval"), KimiScreen::Modal);
        assert_eq!(s("question"), KimiScreen::Modal);
        assert_eq!(read_screen(""), KimiScreen::Other);
        assert_eq!(screen_from("sh", &fixture("kimi-screen/2.2.0/idle.txt")), KimiScreen::Gone);
        assert_eq!(screen_from("kimi", &fixture("kimi-screen/2.2.0/idle.txt")), KimiScreen::Composer);
    }

    #[test]
    fn a_trust_prompt_quoted_in_history_is_not_the_modal() {
        let mut screen = fixture("kimi-screen/2.2.0/trust.txt");
        screen.push_str(&fixture("kimi-screen/2.2.0/composer-typed.txt"));
        assert_eq!(read_screen(&screen), KimiScreen::Composer);
    }

    #[test]
    fn the_composer_and_the_queue_read_by_position() {
        let c = |n: &str| read_composer(&fixture(&format!("kimi-screen/2.2.0/{n}.txt")));
        assert_eq!(c("idle"), Some(Composer { input: String::new(), queued: vec![] }));
        assert_eq!(c("composer-typed").unwrap().input, "Read .planning/brief.md and begin.");
        assert_eq!(
            c("busy-typed").unwrap().input,
            "[worktrees: message from place \"probe\"] reply QUEUED-OK"
        );
        let q = c("busy-queued").unwrap();
        assert_eq!(q.input, "");
        assert_eq!(q.queued, vec!["[worktrees: message from place \"probe\"] reply QUEUED-OK".to_string()]);
        assert_eq!(c("approval"), None);
        assert_eq!(c("trust"), None);
    }

    #[test]
    fn state_is_the_log_with_the_screen_where_only_it_knows() {
        use KimiScreen::*;
        let done = KimiTurn::Done { at: Some(7) };
        assert_eq!(kimi_state(None, Some(Trust)), (State::Waiting, None), "no session yet, trust modal up");
        assert_eq!(kimi_state(Some(&done), Some(Gone)), (State::None, None));
        assert_eq!(kimi_state(Some(&KimiTurn::Busy), Some(Composer)), (State::Busy, None));
        assert_eq!(kimi_state(Some(&KimiTurn::Waiting), Some(Modal)), (State::Waiting, None));
        assert_eq!(kimi_state(Some(&KimiTurn::Waiting), None), (State::Waiting, None), "the log alone");
        assert_eq!(kimi_state(Some(&KimiTurn::Busy), Some(Modal)), (State::Waiting, None));
        assert_eq!(kimi_state(Some(&KimiTurn::Waiting), Some(Composer)), (State::Busy, None), "a composer means no modal");
        assert_eq!(kimi_state(Some(&done), Some(Modal)), (State::Idle, Some(7)), "a picker between turns is no one waiting");
        assert_eq!(kimi_state(Some(&done), Some(Composer)), (State::Idle, Some(7)));
        assert_eq!(kimi_state(Some(&KimiTurn::Aborted), Some(Composer)), (State::Idle, None));
        assert_eq!(kimi_state(None, Some(Composer)), (State::Idle, None), "a fresh lane before its first message");
    }

    // ── submit_kimi / deliver_opener on virtual time ────────────────────────

    use std::cell::{Cell, RefCell};

    fn composer(input: &str, queued: &[&str]) -> String {
        let mut s = String::from(" ● some history\n ────────────\n");
        for q in queued {
            s.push_str(&format!("   ❯ {q}\n"));
        }
        if !queued.is_empty() {
            s.push_str("   ↑ to edit · ctrl-s to steer immediately\n");
        }
        s.push_str(" ╭──────────╮\n");
        s.push_str(&format!(" │ > {input:<40}│\n"));
        s.push_str(" ╰──────────╯\n K2.7 Code Highspeed\n");
        s
    }

    #[test]
    fn a_send_submits_when_its_prompt_reaches_the_log() {
        let clock = Cell::new(0u64);
        let enters = Cell::new(0);
        let typed = "[worktrees: message from place \"a\"] hello";
        let out = submit_kimi(
            &send_prefix(typed),
            None,
            (Some("s1".into()), 0),
            || Some(if enters.get() == 0 { composer(typed, &[]) } else { composer("", &[]) }),
            || (Some("s1".into()), enters.get()),
            || {
                enters.set(enters.get() + 1);
                Ok(())
            },
            || clock.get(),
            |ms| clock.set(clock.get() + ms),
        );
        assert_eq!(out, SendOutcome::Submitted);
        assert_eq!(enters.get(), 1);
    }

    #[test]
    fn a_first_message_creates_the_session_and_that_is_the_receipt() {
        assert!(received(&(None, 0), &(Some("s".into()), 1)));
        assert!(received(&(Some("old".into()), 3), &(Some("new".into()), 1)));
        assert!(!received(&(Some("old".into()), 3), &(Some("old".into()), 3)));
        assert!(!received(&(Some("old".into()), 3), &(Some("new".into()), 0)));
    }

    #[test]
    fn a_busy_send_is_queued_when_kimi_lists_it() {
        let clock = Cell::new(0u64);
        let enters = Cell::new(0);
        let typed = "[worktrees: message from place \"probe\"] reply QUEUED-OK";
        let out = submit_kimi(
            &send_prefix(typed),
            None,
            (Some("s".into()), 0),
            || Some(if enters.get() == 0 { composer(typed, &[]) } else { composer("", &[typed]) }),
            || (Some("s".into()), 0),
            || {
                enters.set(enters.get() + 1);
                Ok(())
            },
            || clock.get(),
            |ms| clock.set(clock.get() + ms),
        );
        assert_eq!(out, SendOutcome::Queued);
    }

    /// The message reached kimi's log, and its turn opened an approval before
    /// the next poll: that is a delivered message, not "typed but unconfirmed"
    /// (which tells the orchestrator to check and resend).
    #[test]
    fn a_receipt_wins_over_an_approval_that_opens_right_after_enter() {
        let clock = Cell::new(0u64);
        let enters = Cell::new(0);
        let typed = "[worktrees: message from place \"a\"] run ls";
        let out = submit_kimi(
            &send_prefix(typed),
            None,
            (Some("s".into()), 0),
            || Some(if enters.get() == 0 { composer(typed, &[]) } else { fixture("kimi-screen/2.2.0/approval.txt") }),
            || (Some("s".into()), enters.get()),
            || {
                enters.set(enters.get() + 1);
                Ok(())
            },
            || clock.get(),
            |ms| clock.set(clock.get() + ms),
        );
        assert_eq!(out, SendOutcome::Submitted);
    }

    #[test]
    fn a_modal_stops_a_send_with_no_key_pressed() {
        let clock = Cell::new(0u64);
        let enters = Cell::new(0);
        let out = submit_kimi(
            "x",
            None,
            (None, 0),
            || Some(fixture("kimi-screen/2.2.0/approval.txt")),
            || (None, 0),
            || {
                enters.set(enters.get() + 1);
                Ok(())
            },
            || clock.get(),
            |ms| clock.set(clock.get() + ms),
        );
        assert_eq!(out, SendOutcome::Modal);
        assert_eq!(enters.get(), 0);
    }

    struct Pane {
        screens: RefCell<Vec<(String, String)>>,
        typed: RefCell<Vec<String>>,
        enters: Cell<usize>,
    }

    impl Pane {
        fn new(screens: Vec<(&str, String)>) -> Pane {
            Pane {
                screens: RefCell::new(screens.into_iter().map(|(c, s)| (c.to_string(), s)).collect()),
                typed: RefCell::new(Vec::new()),
                enters: Cell::new(0),
            }
        }
        /// The next scripted screen, the last one repeating.
        fn next(&self) -> Option<(String, String)> {
            let mut s = self.screens.borrow_mut();
            if s.len() > 1 {
                Some(s.remove(0))
            } else {
                s.first().cloned()
            }
        }
    }

    fn run_opener(pane: &Pane, started: bool, wait: u64) -> OpenerOutcome {
        let clock = Cell::new(0u64);
        let opener = crate::ops::BRIEF_OPENER;
        deliver_opener(
            opener,
            wait,
            || {
                // Once typed, the composer shows the opener until Enter, then clears.
                if !pane.typed.borrow().is_empty() {
                    let input = if pane.enters.get() == 0 { opener } else { "" };
                    return Some(("kimi".into(), composer(input, &[])));
                }
                pane.next()
            },
            || started,
            || (pane.enters.get() > 0).then(|| "s".to_string()).map_or((None, 0), |id| (Some(id), 1)),
            |t| {
                pane.typed.borrow_mut().push(t.to_string());
                Ok(())
            },
            || {
                pane.enters.set(pane.enters.get() + 1);
                Ok(())
            },
            || clock.get(),
            |ms| clock.set(clock.get() + ms),
        )
    }

    /// Review of #509: the user starts typing in the gap between the helper's
    /// keys and its Enter. The composer then holds the opener AND their words,
    /// and Enter would submit both as one turn. The opener's whole text is
    /// known, so only an exact match may be submitted.
    #[test]
    fn the_opener_never_submits_a_composer_the_user_typed_into() {
        let clock = Cell::new(0u64);
        let typed = Cell::new(false);
        let enters = Cell::new(0);
        let opener = crate::ops::BRIEF_OPENER;
        let glued = format!("{opener} and also delete everything");
        let out = deliver_opener(
            opener,
            60_000,
            || Some(("kimi".into(), if typed.get() { composer(&glued, &[]) } else { fixture("kimi-screen/2.2.0/idle.txt") })),
            || false,
            || (None, 0),
            |_| {
                typed.set(true);
                Ok(())
            },
            || {
                enters.set(enters.get() + 1);
                Ok(())
            },
            || clock.get(),
            |ms| clock.set(clock.get() + ms),
        );
        assert_eq!(enters.get(), 0, "Enter was pressed on the user's text");
        assert!(matches!(out, OpenerOutcome::Unconfirmed(_)), "{out:?}");
    }

    #[test]
    fn the_opener_waits_out_startup_and_the_trust_modal_then_types_once() {
        let pane = Pane::new(vec![
            ("kimi", String::new()),
            ("kimi", fixture("kimi-screen/2.2.0/trust.txt")),
            ("kimi", fixture("kimi-screen/2.2.0/idle.txt")),
        ]);
        assert_eq!(run_opener(&pane, false, OPENER_WAIT_MS), OpenerOutcome::Delivered);
        assert_eq!(*pane.typed.borrow(), vec![crate::ops::BRIEF_OPENER.to_string()]);
        assert_eq!(pane.enters.get(), 1);
    }

    #[test]
    fn the_opener_never_types_into_a_non_empty_composer_and_gives_up_bounded() {
        let pane = Pane::new(vec![("kimi", composer("the user is typing", &[]))]);
        assert_eq!(run_opener(&pane, false, 30_000), OpenerOutcome::TimedOut("the composer was never empty"));
        assert!(pane.typed.borrow().is_empty());
        assert_eq!(pane.enters.get(), 0);
    }

    #[test]
    fn the_opener_never_types_over_a_modal() {
        let pane = Pane::new(vec![("kimi", fixture("kimi-screen/2.2.0/trust.txt"))]);
        assert_eq!(run_opener(&pane, false, 30_000), OpenerOutcome::TimedOut("the trust prompt was never answered"));
        assert!(pane.typed.borrow().is_empty());
        let pane = Pane::new(vec![("kimi", fixture("kimi-screen/2.2.0/approval.txt"))]);
        assert_eq!(run_opener(&pane, false, 30_000), OpenerOutcome::TimedOut("a prompt stayed open"));
        assert_eq!(pane.enters.get(), 0);
    }

    #[test]
    fn the_opener_steps_aside_when_the_conversation_started_by_hand() {
        let pane = Pane::new(vec![("kimi", fixture("kimi-screen/2.2.0/idle.txt"))]);
        assert_eq!(run_opener(&pane, true, 30_000), OpenerOutcome::TookOver);
        assert!(pane.typed.borrow().is_empty());
    }

    #[test]
    fn the_opener_gives_up_when_kimi_exits() {
        let pane = Pane::new(vec![("kimi", String::new()), ("zsh", String::new())]);
        assert_eq!(run_opener(&pane, false, 60_000), OpenerOutcome::Gone);
        assert!(pane.typed.borrow().is_empty());
    }

    /// The helper starts before kimi does: a shell on its first look is kimi
    /// still starting, not kimi gone.
    #[test]
    fn the_opener_waits_for_kimi_to_start() {
        let pane = Pane::new(vec![
            ("zsh", String::new()),
            ("zsh", String::new()),
            ("kimi", fixture("kimi-screen/2.2.0/idle.txt")),
        ]);
        assert_eq!(run_opener(&pane, false, 60_000), OpenerOutcome::Delivered);
        // …and one that never starts is given up on, bounded.
        let pane = Pane::new(vec![("zsh", String::new())]);
        assert_eq!(run_opener(&pane, false, OPENER_WAIT_MS), OpenerOutcome::Gone);
        assert!(pane.typed.borrow().is_empty());
    }

    /// Found live: right after `new` the pane is the shell sourcing its rc,
    /// and a reading of "nobody there" let `wait` answer before kimi began.
    #[test]
    fn a_lane_still_starting_with_its_opener_pending_is_busy() {
        assert_eq!(before_start("s".into(), true, true).map(|a| a.state), Some(State::Busy));
        assert_eq!(before_start("s".into(), false, true), None, "no opener: a shell is a shell");
        assert_eq!(before_start("s".into(), true, false), None, "no session at all");
    }

    #[test]
    fn a_pending_opener_holds_an_idle_lane_busy_and_nothing_else() {
        assert_eq!(with_pending_opener((State::Idle, None), true), (State::Busy, None));
        assert_eq!(with_pending_opener((State::Idle, Some(5)), false), (State::Idle, Some(5)));
        assert_eq!(with_pending_opener((State::Waiting, None), true), (State::Waiting, None), "the trust modal still says so");
        assert_eq!(with_pending_opener((State::None, None), true), (State::None, None), "kimi exited");
    }

    #[test]
    fn the_opener_log_keeps_one_line_per_outcome_and_finds_the_place() {
        let t = std::env::temp_dir().join(format!("kimi-opener-log-{}", std::process::id()));
        std::fs::create_dir_all(&t).unwrap();
        let g = t.to_string_lossy().into_owned();
        log_opener(&g, "feat", &OpenerOutcome::TimedOut("the trust prompt was never answered"));
        log_opener(&g, "other", &OpenerOutcome::Delivered);
        let last = last_opener_line(&g, "feat").unwrap();
        assert!(last.ends_with("feat: not delivered: gave up waiting (the trust prompt was never answered)"), "{last}");
        assert!(last_opener_line(&g, "nope").is_none());
        let _ = std::fs::remove_dir_all(&t);
    }
}
