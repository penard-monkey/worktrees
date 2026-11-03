//! Restart the agent sessions worktrees launched so they run the installed
//! CLI, each resuming its own conversation (`docs/proposals/harness-restart.md`).
//!
//! `harness_sessions` (read) lists, per tool, the sessions in worktrees places
//! that run an OLDER build than the one installed, with what each is doing,
//! and the older Claude sessions outside worktrees that only a person can
//! restart. `restart_harness_sessions` (admin) restarts the idle ones, or with
//! `force` the busy and waiting ones too.
//!
//! **Which build a session runs.** Claude writes it: its probe's `version` is
//! the running process's. Codex does not — a resumed Codex appends to the
//! thread's rollout, whose `cli_version` stays the one that STARTED the
//! thread — so a Codex session is older when its process started before the
//! installed executable was last replaced (its ctime), or runs another file.
//!
//! **Which conversation it resumes.** Claude's probe's `sessionId`
//! (`--resume '<id>'`, never the `-r` picker); for Codex the rollout its
//! process holds open (`codex::held_thread`). No id, no restart.
//!
//! **How.** `respawn-pane -k` on the agent's own pane, with the command
//! `open` would build (`ops::restart_command`): the pane, its session, the
//! session's other panes and windows and every attached client stay. Each
//! restart re-reads its lane under the place's agent lock first — a session
//! that turned busy since the list was read is not inactive any more — and
//! is then proved on the new process (Claude's probe: same `sessionId`, the
//! installed version; Codex: a new process holding the same rollout).
//!
//! Every pane, process and file goes through [`RestartIo`], so the rules are
//! tested against scripted lanes and never a tmux server.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use worktrees_core::activity::State;
use worktrees_core::tmux::{self, PaneId, PaneList};
use worktrees_core::{agent, codex, harness, procinfo, provider, Project};

use crate::Ctx;

/// What `kimi`/`pi` answer: their restart was not measured
/// (`docs/proposals/harness-restart.md`).
pub const ERR_UNSUPPORTED: &str = "restarting sessions is supported for Claude and Codex only";
/// How long a restart is watched for its new version.
const PROVE_FOR: Duration = Duration::from_secs(20);
const PROVE_EVERY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    pub fn parse(s: &str) -> Result<Tool, String> {
        match s {
            "claude" => Ok(Tool::Claude),
            "codex" => Ok(Tool::Codex),
            "kimi" | "pi" => Err(ERR_UNSUPPORTED.into()),
            _ => Err("unknown harness".into()),
        }
    }
    pub fn id(self) -> &'static str {
        self.provider().id
    }
    fn provider(self) -> &'static provider::Provider {
        match self {
            Tool::Claude => provider::CLAUDE,
            Tool::Codex => provider::CODEX,
        }
    }
}

/// The executable worktrees launches for a tool, as installed now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Installed {
    /// Canonical: what a running process's executable is compared with.
    pub path: PathBuf,
    /// When the file was last created or replaced (`procinfo::ctime`).
    pub ctime: Option<i64>,
    pub version: Option<String>,
}

/// One agent session in a worktrees place.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Session {
    /// The project's root, and its folder name for display.
    pub project: String,
    pub project_name: String,
    pub slug: String,
    /// The tmux session it runs in.
    pub session: String,
    /// The build it runs, when it is known: always for Claude, for Codex only
    /// when it is the installed one.
    pub version: Option<String>,
    /// `idle`, `busy` or `waiting` (on a prompt).
    pub state: &'static str,
    /// Why it cannot be restarted, when it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip)]
    pub older: bool,
    #[serde(skip)]
    pub path: String,
    /// `%N`: the agent's pane.
    #[serde(skip)]
    pub pane: String,
    /// The agent's process: Claude's probe pid, Codex's job leader.
    #[serde(skip)]
    pub pid: i32,
    #[serde(skip)]
    pub resume_id: Option<String>,
}

/// An older session outside worktrees, restarted only by hand.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Outside {
    pub version: Option<String>,
}

/// What one read of the machine found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub sessions: Vec<Session>,
    pub outside: Vec<Outside>,
}

/// `harness_sessions`' answer.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Sessions {
    pub tool: &'static str,
    pub installed_version: Option<String>,
    /// The worktrees sessions that run an older build.
    pub sessions: Vec<Session>,
    pub outside: Vec<Outside>,
}

/// One session's restart, as `restart_harness_sessions` answers it.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Restart {
    pub project: String,
    pub project_name: String,
    pub slug: String,
    pub session: String,
    /// `restarted`, `skipped` or `failed`.
    pub outcome: &'static str,
    /// The version the restarted session was seen to run; `None` while unproved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// `restart_harness_sessions`' answer.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Restarted {
    pub tool: &'static str,
    pub force: bool,
    pub installed_version: Option<String>,
    pub results: Vec<Restart>,
    pub restarted: usize,
    pub skipped: usize,
    pub failed: usize,
    /// The projects whose `(main)` agent was restarted, named so a client can
    /// say the orchestrator went too.
    pub mains: Vec<String>,
}

/// How a restart went once the pane was respawned.
#[derive(Clone, Debug, PartialEq)]
pub enum Proof {
    /// The new process runs this version (and the same conversation).
    Runs(Option<String>),
    /// Not seen within the deadline.
    Unconfirmed,
}

/// Everything the rows read and do. [`Live`] is the machine; a test scripts it.
pub trait RestartIo: Send + Sync {
    fn installed(&self, tool: Tool) -> Option<Installed>;
    /// Every `tool` session in the places of `roots`, older or not, and the
    /// older ones outside worktrees.
    fn find(&self, roots: &[String], tool: Tool, installed: &Installed) -> Found;
    /// `lane` read again now: `None` when its pane is gone or runs no agent.
    fn reread(&self, lane: &Session, tool: Tool, installed: &Installed) -> Option<Session>;
    /// Whether this process (serve) runs inside `lane`'s pane.
    fn hosts_us(&self, lane: &Session) -> bool;
    /// Relaunch `lane`'s agent in its pane, resuming its conversation.
    fn respawn(&self, lane: &Session, tool: Tool) -> Result<(), String>;
    /// Wait for the relaunched agent, until `deadline`.
    fn prove(&self, lane: &Session, tool: Tool, installed: &Installed, deadline: Instant) -> Proof;
}

// ── the rules, pure ──────────────────────────────────────────────────────────

/// A Claude session's build, from its probe: older when the installed version
/// is newer than the one the probe says it runs.
pub fn claude_older(running: Option<&str>, installed: &Installed) -> bool {
    match (running, installed.version.as_deref()) {
        (Some(r), Some(i)) => crate::harness_updates::newer(i, r),
        _ => false,
    }
}

/// A Codex session's build, from its process: older when it started before
/// the installed executable was last replaced, or runs another executable —
/// unless that executable is the interpreter of an npm install (`node`),
/// whose script is the installed file. Unknown facts say nothing.
pub fn codex_older(exe: Option<&Path>, start: Option<i64>, installed: &Installed) -> bool {
    let Some(exe) = exe else { return false };
    let started_before = matches!((start, installed.ctime), (Some(s), Some(c)) if s < c);
    let launcher = exe.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == "node" || n.starts_with("node"));
    let same = std::fs::canonicalize(exe).map_or(false, |e| e == installed.path);
    started_before || (!same && !launcher)
}

/// The three states a client is shown: what the nav dot would say, with
/// anything that is not plainly idle or waiting counted as busy.
pub fn bucket(state: State) -> &'static str {
    match state {
        State::Idle => "idle",
        State::Waiting => "waiting",
        _ => "busy",
    }
}

/// The read row, from what `io` finds in `roots`. `everyone` is a caller who
/// sees every project: only it is told about sessions outside worktrees,
/// which belong to no project and so to none a scoped device was granted.
pub fn sessions_with(io: &dyn RestartIo, roots: &[String], tool: Tool, everyone: bool) -> Sessions {
    let Some(installed) = io.installed(tool) else {
        return Sessions { tool: tool.id(), installed_version: None, sessions: Vec::new(), outside: Vec::new() };
    };
    let found = io.find(roots, tool, &installed);
    Sessions {
        tool: tool.id(),
        installed_version: installed.version.clone(),
        sessions: found.sessions.into_iter().filter(|s| s.older).collect(),
        outside: if everyone { found.outside } else { Vec::new() },
    }
}

fn result(lane: &Session, outcome: &'static str, version: Option<String>, why: Option<String>) -> Restart {
    Restart {
        project: lane.project.clone(),
        project_name: lane.project_name.clone(),
        slug: lane.slug.clone(),
        session: lane.session.clone(),
        outcome,
        version,
        why,
    }
}

/// Why a lane read again under its lock is not restarted now, if it is not.
fn refusal(listed: &Session, now: Option<&Session>, force: bool) -> Option<String> {
    let Some(now) = now else { return Some("it is no longer running".into()) };
    if now.pid != listed.pid {
        return Some("it was restarted since the list was read".into());
    }
    if !now.older {
        return Some("it already runs the installed version".into());
    }
    if let Some(why) = &now.why {
        return Some(why.clone());
    }
    if !force && now.state != "idle" {
        return Some(match now.state {
            "waiting" => "it is waiting on a prompt now".into(),
            _ => "it turned busy since the list was read".into(),
        });
    }
    None
}

/// The action row, from what `io` finds and does. Each lane is re-read and
/// respawned under `lock(path)`; the proofs run together afterwards, with no
/// lock held.
pub fn restart_with(io: &dyn RestartIo, roots: &[String], lock: &dyn Fn(&str) -> Arc<Mutex<()>>, tool: Tool, force: bool) -> Result<Restarted, String> {
    let installed = io.installed(tool).ok_or_else(|| format!("{} is not installed", tool.provider().label))?;
    if installed.version.is_none() {
        return Err(format!("the installed {} did not report its version", tool.provider().label));
    }
    let found = io.find(roots, tool, &installed);
    let mut results: Vec<Option<Restart>> = Vec::new();
    let mut respawned: Vec<(usize, Session)> = Vec::new();
    for lane in found.sessions.iter().filter(|s| s.older) {
        if let Some(why) = &lane.why {
            results.push(Some(result(lane, "skipped", None, Some(why.clone()))));
            continue;
        }
        if !force && lane.state != "idle" {
            let why = if lane.state == "waiting" { "it is waiting on a prompt" } else { "it is busy" };
            results.push(Some(result(lane, "skipped", None, Some(why.into()))));
            continue;
        }
        let place = lock(&lane.path);
        let _held = place.lock().unwrap_or_else(|e| e.into_inner());
        let now = io.reread(lane, tool, &installed);
        if let Some(why) = refusal(lane, now.as_ref(), force) {
            results.push(Some(result(lane, "skipped", None, Some(why))));
            continue;
        }
        let now = now.expect("refusal checked it");
        if io.hosts_us(&now) {
            results.push(Some(result(&now, "skipped", None, Some("Worktrees' own server runs in this session".into()))));
            continue;
        }
        match io.respawn(&now, tool) {
            Ok(()) => {
                respawned.push((results.len(), now));
                results.push(None);
            }
            Err(e) => results.push(Some(result(&now, "failed", None, Some(e)))),
        }
    }
    let deadline = Instant::now() + PROVE_FOR;
    let proofs: Vec<(usize, Restart)> = std::thread::scope(|s| {
        let jobs: Vec<_> = respawned
            .iter()
            .map(|(i, lane)| {
                let installed = &installed;
                s.spawn(move || {
                    let r = match io.prove(lane, tool, installed, deadline) {
                        Proof::Runs(v) => result(lane, "restarted", v, None),
                        Proof::Unconfirmed => result(lane, "restarted", None, Some("its new version did not show within 20 s".into())),
                    };
                    (*i, r)
                })
            })
            .collect();
        jobs.into_iter().map(|j| j.join().expect("a proof panicked")).collect()
    });
    for (i, r) in proofs {
        results[i] = Some(r);
    }
    let results: Vec<Restart> = results.into_iter().flatten().collect();
    let count = |o: &str| results.iter().filter(|r| r.outcome == o).count();
    let mut mains: Vec<String> = results
        .iter()
        .filter(|r| r.outcome == "restarted" && r.slug == "(main)")
        .map(|r| r.project_name.clone())
        .collect();
    mains.dedup();
    Ok(Restarted {
        tool: tool.id(),
        force,
        installed_version: installed.version.clone(),
        restarted: count("restarted"),
        skipped: count("skipped"),
        failed: count("failed"),
        results,
        mains,
    })
}

// ── the rows ─────────────────────────────────────────────────────────────────

/// `harness_sessions {tool}`: older sessions of `tool`, live, in the
/// caller's projects (`read_projects` is the caller's scope). The ones
/// outside worktrees belong to no project, so only a caller who sees every
/// project is told about them: to a scoped device they would be sessions of
/// projects it was not granted.
pub fn sessions(ctx: &Ctx, tool: String) -> Result<Sessions, String> {
    let tool = Tool::parse(&tool)?;
    Ok(sessions_with(&Live, &crate::projects::read_projects(ctx), tool, ctx.scope().is_all()))
}

/// `restart_harness_sessions {tool, force}`, in the caller's projects only:
/// `read_projects` answers with the caller's scope (serve's `Ctx::scoped`),
/// so a device granted one project restarts that project's agents and no
/// other's. A `force` left out is `false`: the inactive sessions only.
pub fn restart(ctx: &Ctx, tool: String, force: Option<bool>) -> Result<Restarted, String> {
    let tool = Tool::parse(&tool)?;
    let force = force.unwrap_or(false);
    let agents = ctx.agents.clone();
    let out = restart_with(&Live, &crate::projects::read_projects(ctx), &|path| agents.place_lock(path), tool, force)?;
    crate::log::log(
        "info",
        &format!("restart {} force={force}: {} restarted, {} skipped, {} failed", out.tool, out.restarted, out.skipped, out.failed),
    );
    Ok(out)
}

// ── the machine ──────────────────────────────────────────────────────────────

/// The machine: tmux, the probes, the processes.
pub struct Live;

type VersionKey = (PathBuf, i64, u64);
/// `--version` of an installed executable, by (path, ctime, size): asked once
/// per install, so a list costs no spawn once the version is known.
static VERSIONS: Mutex<Vec<(VersionKey, Option<String>)>> = Mutex::new(Vec::new());

fn installed_version(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    let key = (path.to_path_buf(), meta.ctime(), meta.len());
    if let Some((_, v)) = VERSIONS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| *k == key) {
        return v.clone();
    }
    let mut c = worktrees_core::childenv::command(path);
    c.arg("--version");
    if let Some(home) = std::env::var_os("HOME") {
        c.current_dir(home);
    }
    let v = worktrees_core::proc::run_deadline(c, 5)
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| crate::harness_updates::version(&String::from_utf8_lossy(&o.stdout)));
    let mut cache = VERSIONS.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(k, _)| k.0 != key.0);
    cache.push((key, v.clone()));
    v
}

/// One agent pane in a place, before its build is known.
struct Candidate {
    project: Arc<Project>,
    project_name: String,
    slug: String,
    path: String,
    session: String,
    pane: worktrees_core::tmux::Pane,
}

fn under(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{root}/"))
}

impl Live {
    /// The `tool` panes of every place under `roots`, by the agent rows' own
    /// rule (`agents::live_agent_sessions`, then the pane in the place that
    /// runs the harness). A project with no pane under it is not even
    /// discovered.
    fn candidates(roots: &[String], tool: Tool, panes: &PaneList) -> Vec<Candidate> {
        let word = tool.provider().match_word;
        let ai_word = worktrees_core::project::adopt_ai_word();
        let mut out = Vec::new();
        for root in roots {
            if !panes.panes().iter().any(|p| under(&p.path, root)) {
                continue;
            }
            let Ok(project) = Project::discover(Path::new(root)) else { continue };
            let project = Arc::new(project);
            let name = Path::new(&project.main_root).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for (slug, path) in project.place_dirs() {
                if !panes.panes().iter().any(|p| under(&p.path, &path)) {
                    continue;
                }
                let exclude = (slug == "(main)").then(|| project.wt_root.clone());
                for (adapter, session) in crate::agents::live_agent_sessions(&project, &slug, panes, &ai_word) {
                    if adapter.provider().id != tool.id() {
                        continue;
                    }
                    let Some(pane) = panes.agent_pane_of(&session, &path, exclude.as_deref(), word) else { continue };
                    out.push(Candidate {
                        project: project.clone(),
                        project_name: name.clone(),
                        slug: slug.clone(),
                        path: path.clone(),
                        session,
                        pane: pane.clone(),
                    });
                }
            }
        }
        out
    }

    fn read(roots: &[String], tool: Tool, installed: &Installed, only_pane: Option<&str>) -> Found {
        let Some(panes) = PaneList::fetch() else { return Found::default() };
        let mut probes = agent::live_probes();
        agent::locate(&mut probes, &panes);
        let mut cands = Self::candidates(roots, tool, &panes);
        if let Some(id) = only_pane {
            cands.retain(|c| c.pane.id.as_deref() == Some(id));
        }
        let jobs = if tool == Tool::Codex {
            let ttys: Vec<&str> = cands.iter().filter_map(|c| c.pane.tty.as_deref()).collect();
            tmux::foreground_jobs(&ttys)
        } else {
            Default::default()
        };
        let scan = harness::Scan { probes: &probes, panes: Some(&panes) };
        let mut claimed: HashSet<i32> = HashSet::new();
        let mut sessions = Vec::new();
        for c in &cands {
            let id = c.pane.id.clone().unwrap_or_default();
            let (pid, version, older, resume_id) = match tool {
                Tool::Claude => {
                    let lane = crate::agents::Lane { harness: tool.id(), session: c.session.clone(), pane: id.clone() };
                    let Some(p) = crate::agents::lane_probe(&probes, &lane) else { continue };
                    claimed.insert(p.pid);
                    let sid = p.session_id.clone().filter(|s| codex::is_uuid(s));
                    (p.pid, p.version.clone(), claude_older(p.version.as_deref(), installed), sid)
                }
                Tool::Codex => {
                    let Some(job) = c.pane.tty.as_deref().and_then(|t| jobs.get(t)) else { continue };
                    let exe = procinfo::exe(job.leader);
                    let older = codex_older(exe.as_deref(), procinfo::start_epoch(job.leader), installed);
                    let open: Vec<PathBuf> = job.members.iter().flat_map(|m| procinfo::open_paths(*m)).collect();
                    let version = (!older).then(|| installed.version.clone()).flatten();
                    (job.leader, version, older, codex::held_thread(&open, &c.path))
                }
            };
            // Activity only for the sessions a client is shown.
            let state = if older {
                let canonical = c.project.session_name(&c.slug);
                tool_adapter(tool).activity(&scan, &canonical, &c.path).map_or("busy", |a| bucket(a.state))
            } else {
                "idle"
            };
            let why = resume_id.is_none().then(|| match tool {
                Tool::Claude => "its session file names no conversation, so it cannot be resumed exactly".to_string(),
                Tool::Codex => "it holds no conversation file open, so it cannot be resumed exactly".to_string(),
            });
            sessions.push(Session {
                project: c.project.main_root.clone(),
                project_name: c.project_name.clone(),
                slug: c.slug.clone(),
                session: c.session.clone(),
                version,
                state,
                why,
                older,
                path: c.path.clone(),
                pane: id,
                pid,
                resume_id,
            });
        }
        // Older Claude sessions no place's lane owns: VS Code, a terminal,
        // another pane. Counted only; background (parked) sessions are not
        // sessions a person restarts.
        let outside = if tool == Tool::Claude && only_pane.is_none() {
            probes
                .iter()
                .filter(|p| !claimed.contains(&p.pid))
                .filter(|p| p.kind.as_deref().is_none_or(|k| k == "interactive"))
                .filter(|p| claude_older(p.version.as_deref(), installed))
                .map(|p| Outside { version: p.version.clone() })
                .collect()
        } else {
            Vec::new()
        };
        Found { sessions, outside }
    }
}

fn tool_adapter(tool: Tool) -> &'static dyn harness::Adapter {
    harness::by_id(tool.id()).expect("claude and codex are registered harnesses")
}

impl RestartIo for Live {
    fn installed(&self, tool: Tool) -> Option<Installed> {
        let bin = crate::harness_updates::tool_bin(tool.id())?;
        let path = std::fs::canonicalize(&bin).unwrap_or(bin);
        Some(Installed { ctime: procinfo::ctime(&path), version: installed_version(&path), path })
    }

    fn find(&self, roots: &[String], tool: Tool, installed: &Installed) -> Found {
        Self::read(roots, tool, installed, None)
    }

    fn reread(&self, lane: &Session, tool: Tool, installed: &Installed) -> Option<Session> {
        Self::read(std::slice::from_ref(&lane.project), tool, installed, Some(&lane.pane)).sessions.into_iter().next()
    }

    fn hosts_us(&self, lane: &Session) -> bool {
        let shell = PaneList::fetch().and_then(|pl| pl.pane(&lane.pane).and_then(|p| p.pid.as_deref()?.parse::<i32>().ok()));
        let line = procinfo::lineage(std::process::id() as i32);
        line.contains(&lane.pid) || shell.is_some_and(|s| line.contains(&s))
    }

    fn respawn(&self, lane: &Session, tool: Tool) -> Result<(), String> {
        let pane = PaneId::parse(&lane.pane).ok_or("the agent's pane has no id")?;
        let id = lane.resume_id.as_deref().ok_or("no conversation to resume")?;
        let project = Project::discover(Path::new(&lane.project)).map_err(|e| e.msg)?;
        let mut ui = worktrees_core::ui::CaptureUi::default();
        let cmd = worktrees_core::ops::restart_command(&project, &mut ui, &lane.path, &lane.slug, tool.id(), id, &lane.session)?;
        for w in ui.warnings() {
            crate::log::log("warn", &format!("restart {} {}: {w}", tool.id(), lane.session));
        }
        tmux::respawn_pane(&pane, &lane.path, &cmd)
    }

    fn prove(&self, lane: &Session, tool: Tool, installed: &Installed, deadline: Instant) -> Proof {
        loop {
            if let Some(now) = Self::read(std::slice::from_ref(&lane.project), tool, installed, Some(&lane.pane)).sessions.into_iter().next() {
                if now.pid != lane.pid && !now.older && now.resume_id.is_some() && now.resume_id == lane.resume_id {
                    return Proof::Runs(now.version);
                }
            }
            if Instant::now() >= deadline {
                return Proof::Unconfirmed;
            }
            std::thread::sleep(PROVE_EVERY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn lane(slug: &str, state: &'static str, older: bool) -> Session {
        Session {
            project: "/r".into(),
            project_name: "r".into(),
            slug: slug.into(),
            session: format!("r-{slug}"),
            version: Some(if older { "2.1.316" } else { "2.1.317" }.into()),
            state,
            why: None,
            older,
            path: format!("/r/.worktrees/{slug}"),
            pane: format!("%{}", slug.len()),
            pid: 100 + slug.len() as i32,
            resume_id: Some("beba5e74-3ece-4f92-b5c6-72da058fca41".into()),
        }
    }

    /// A scripted machine: what the list finds, what each re-read finds (by
    /// slug), and a record of every respawn.
    #[derive(Default)]
    struct Script {
        found: Vec<Session>,
        outside: Vec<Outside>,
        now: Mutex<Vec<Session>>,
        respawned: Mutex<Vec<String>>,
        fail: Vec<String>,
        hosts: Vec<String>,
        locked: Mutex<Vec<String>>,
        rereads: AtomicUsize,
    }

    impl RestartIo for Script {
        fn installed(&self, _: Tool) -> Option<Installed> {
            Some(Installed { path: "/bin/claude".into(), ctime: Some(10), version: Some("2.1.317".into()) })
        }
        fn find(&self, _: &[String], _: Tool, _: &Installed) -> Found {
            Found { sessions: self.found.clone(), outside: self.outside.clone() }
        }
        fn reread(&self, l: &Session, _: Tool, _: &Installed) -> Option<Session> {
            self.rereads.fetch_add(1, Ordering::SeqCst);
            self.now.lock().unwrap().iter().find(|s| s.slug == l.slug).cloned()
        }
        fn hosts_us(&self, l: &Session) -> bool {
            self.hosts.contains(&l.slug)
        }
        fn respawn(&self, l: &Session, _: Tool) -> Result<(), String> {
            if self.fail.contains(&l.slug) {
                return Err("tmux respawn-pane exited 1".into());
            }
            self.respawned.lock().unwrap().push(l.slug.clone());
            Ok(())
        }
        fn prove(&self, l: &Session, _: Tool, i: &Installed, _: Instant) -> Proof {
            if l.slug == "slow" { Proof::Unconfirmed } else { Proof::Runs(i.version.clone()) }
        }
    }

    fn run(s: &Script, force: bool) -> Restarted {
        let lock = |p: &str| {
            s.locked.lock().unwrap().push(p.to_string());
            Arc::new(Mutex::new(()))
        };
        restart_with(s, &["/r".into()], &lock, Tool::Claude, force).unwrap()
    }

    fn outcome<'a>(r: &'a Restarted, slug: &str) -> (&'a str, Option<&'a str>) {
        let x = r.results.iter().find(|x| x.slug == slug).unwrap_or_else(|| panic!("{slug} missing from {:?}", r.results));
        (x.outcome, x.why.as_deref())
    }

    #[test]
    fn the_list_shows_older_sessions_only_with_their_activity() {
        let s = Script {
            found: vec![lane("a", "idle", true), lane("bb", "busy", true), lane("ccc", "waiting", true), lane("new", "idle", false)],
            outside: vec![Outside { version: Some("2.1.310".into()) }],
            ..Script::default()
        };
        let got = sessions_with(&s, &["/r".into()], Tool::Claude, true);
        assert_eq!(got.installed_version.as_deref(), Some("2.1.317"));
        let rows: Vec<(&str, &str)> = got.sessions.iter().map(|x| (x.slug.as_str(), x.state)).collect();
        assert_eq!(rows, [("a", "idle"), ("bb", "busy"), ("ccc", "waiting")]);
        assert_eq!(got.outside, [Outside { version: Some("2.1.310".into()) }]);
        let wire = serde_json::to_value(&got).unwrap();
        assert!(wire["sessions"][0].get("pane").is_none() && wire["sessions"][0].get("resume_id").is_none(), "{wire}");
        // A scoped device: its projects' sessions, and none outside them.
        let scoped = sessions_with(&s, &["/r".into()], Tool::Claude, false);
        assert_eq!(scoped.sessions.len(), 3);
        assert!(scoped.outside.is_empty(), "sessions outside worktrees belong to no project a device was granted");
    }

    #[test]
    fn claude_is_older_by_its_probe_version() {
        let i = Installed { version: Some("2.1.317".into()), ..Installed::default() };
        assert!(claude_older(Some("2.1.316"), &i));
        assert!(!claude_older(Some("2.1.317"), &i));
        assert!(!claude_older(Some("2.1.318"), &i), "a newer session is not older");
        assert!(!claude_older(None, &i), "no version says nothing");
        assert!(!claude_older(Some("2.1.316"), &Installed::default()));
    }

    #[test]
    fn codex_is_older_by_its_process_not_its_rollout() {
        let dir = std::env::temp_dir().join(format!("wt-codex-older-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ext-old")).unwrap();
        let installed_bin = dir.join("codex");
        std::fs::write(&installed_bin, "").unwrap();
        let other = dir.join("ext-old/codex");
        std::fs::write(&other, "").unwrap();
        let path = std::fs::canonicalize(&installed_bin).unwrap();
        let i = Installed { path: path.clone(), ctime: Some(1_000), version: Some("0.160.1".into()) };
        assert!(!codex_older(Some(&path), Some(1_500), &i), "the installed file, started after it was installed");
        assert!(codex_older(Some(&path), Some(900), &i), "started before the file was replaced");
        assert!(codex_older(Some(&other), Some(1_500), &i), "another copy of codex");
        assert!(!codex_older(Some(Path::new("/usr/local/bin/node")), Some(1_500), &i), "npm's launcher, after the install");
        assert!(codex_older(Some(Path::new("/usr/local/bin/node")), Some(900), &i), "npm's launcher, before it");
        assert!(codex_older(Some(Path::new("/x/codex (deleted)")), Some(1_500), &i), "Linux names a replaced file so");
        assert!(!codex_older(None, Some(900), &i), "no process facts say nothing");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn inactive_restarts_idle_sessions_only() {
        let found = vec![lane("a", "idle", true), lane("bb", "busy", true), lane("ccc", "waiting", true), lane("new", "idle", false)];
        let s = Script { now: Mutex::new(found.clone()), found, ..Script::default() };
        let r = run(&s, false);
        assert_eq!(*s.respawned.lock().unwrap(), ["a"]);
        assert_eq!(outcome(&r, "a"), ("restarted", None));
        assert_eq!(outcome(&r, "bb"), ("skipped", Some("it is busy")));
        assert_eq!(outcome(&r, "ccc"), ("skipped", Some("it is waiting on a prompt")));
        assert!(r.results.iter().all(|x| x.slug != "new"), "a current session is not touched or listed");
        assert_eq!((r.restarted, r.skipped, r.failed), (1, 2, 0));
        assert_eq!(r.results.iter().find(|x| x.slug == "a").unwrap().version.as_deref(), Some("2.1.317"));
        assert_eq!(*s.locked.lock().unwrap(), ["/r/.worktrees/a"], "only the session acted on is locked");
    }

    #[test]
    fn force_restarts_busy_and_waiting_too_and_names_main() {
        let found = vec![lane("(main)", "busy", true), lane("bb", "waiting", true), lane("ccc", "idle", true)];
        let s = Script { now: Mutex::new(found.clone()), found, ..Script::default() };
        let r = run(&s, true);
        assert_eq!(*s.respawned.lock().unwrap(), ["(main)", "bb", "ccc"]);
        assert_eq!((r.restarted, r.skipped, r.failed), (3, 0, 0));
        assert_eq!(r.mains, ["r"]);
        assert!(r.force);
    }

    /// The re-read under the lock decides, not the list: a session that turned
    /// busy since is skipped without force; one restarted or gone since is
    /// skipped either way; the lock is taken BEFORE the re-read.
    #[test]
    fn the_lane_is_read_again_under_its_lock() {
        let found = vec![lane("a", "idle", true), lane("bb", "idle", true), lane("ccc", "idle", true), lane("dddd", "idle", true)];
        let mut now = found.clone();
        now[0].state = "busy";
        now[1].pid = 999;
        now[2].older = false;
        now.remove(3);
        let s = Script { now: Mutex::new(now.clone()), found: found.clone(), ..Script::default() };
        let r = run(&s, false);
        assert!(s.respawned.lock().unwrap().is_empty(), "{r:?}");
        assert_eq!(outcome(&r, "a"), ("skipped", Some("it turned busy since the list was read")));
        assert_eq!(outcome(&r, "bb"), ("skipped", Some("it was restarted since the list was read")));
        assert_eq!(outcome(&r, "ccc"), ("skipped", Some("it already runs the installed version")));
        assert_eq!(outcome(&r, "dddd"), ("skipped", Some("it is no longer running")));
        assert_eq!(s.rereads.load(Ordering::SeqCst), 4);
        assert_eq!(s.locked.lock().unwrap().len(), 4, "every re-read held its place's lock");
        // With force, the one that turned busy goes; the others still do not.
        let s = Script { now: Mutex::new(now), found, ..Script::default() };
        let r = run(&s, true);
        assert_eq!(*s.respawned.lock().unwrap(), ["a"]);
        assert_eq!(outcome(&r, "a").0, "restarted");
    }

    #[test]
    fn failures_unproved_restarts_and_our_own_session_are_said() {
        let mut found = vec![lane("a", "idle", true), lane("bb", "idle", true), lane("slow", "idle", true), lane("us", "idle", true)];
        found.push(Session { why: Some("it holds no conversation file open, so it cannot be resumed exactly".into()), ..lane("nofile", "idle", true) });
        let s = Script { now: Mutex::new(found.clone()), found, fail: vec!["bb".into()], hosts: vec!["us".into()], ..Script::default() };
        let r = run(&s, false);
        assert_eq!(outcome(&r, "a"), ("restarted", None));
        assert_eq!(outcome(&r, "bb"), ("failed", Some("tmux respawn-pane exited 1")));
        assert_eq!(outcome(&r, "slow"), ("restarted", Some("its new version did not show within 20 s")));
        assert_eq!(outcome(&r, "us"), ("skipped", Some("Worktrees' own server runs in this session")));
        assert_eq!(outcome(&r, "nofile").0, "skipped");
        assert_eq!(*s.respawned.lock().unwrap(), ["a", "slow"]);
        assert_eq!((r.restarted, r.skipped, r.failed), (2, 2, 1));
        assert!(r.mains.is_empty());
    }

    #[test]
    fn only_claude_and_codex_are_restarted() {
        assert_eq!(Tool::parse("claude"), Ok(Tool::Claude));
        assert_eq!(Tool::parse("codex"), Ok(Tool::Codex));
        assert_eq!(Tool::parse("pi"), Err(ERR_UNSUPPORTED.to_string()));
        assert_eq!(Tool::parse("kimi"), Err(ERR_UNSUPPORTED.to_string()));
        assert_eq!(Tool::parse("--all"), Err("unknown harness".to_string()));
    }
}
