//! Projects and their snapshots: the nav's whole data source.
//!
//! One `snapshot` per project — core's live `ls`, overlaid with the DECLARED
//! store and a reconciled `lifecycle_effective` — and `list_workspace`, which
//! fans those out across every registered project.
//!
//! Several input checks live here rather than in the dialog that offers them
//! (plan §III.3a R2). `valid_project_name` says why in its own comment: the
//! frontend checks the same shapes for the inline hint, and a text box is
//! never evidence. The same reasoning now extends one layer out — a client
//! that is not our WebView reaches these functions directly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use worktrees_core::{git, harness, provider, store, sysclock, tmux, Project};

use crate::log::log as applog;
use crate::projects::{project_roots, read_projects};
use crate::watch::Watch;
use crate::Ctx;


/// One harness's session in one place, as the snapshot reports it.
pub struct AgentSession {
    pub id: &'static str,
    pub name: String,
    pub up: bool,
    pub model: Option<String>,
}

/// Every harness's session for a place, in registry order.
///
/// A non-default harness (Codex) runs in its `~agent~<id>` sidecar, or — for a
/// session launched before sidecars — in the canonical session when that
/// session is running it. The default harness (Claude) owns the canonical
/// session historically: it is up in its sidecar, or in the place's primary
/// session when no other harness claims that one.
pub fn agent_sessions_for(
    w: &Watch,
    panes: Option<&tmux::PaneList>,
    probes: &[worktrees_core::agent::ClaudeProbe],
    canonical: &str,
    primary_name: &str,
    primary_up: bool,
    place_path: &str,
) -> Vec<AgentSession> {
    let owner = panes.map_or(provider::CLAUDE.id, |p| p.canonical_provider(canonical).id);
    let has = |name: &str| panes.is_some_and(|p| p.has_session(name));
    let others: Vec<String> = provider::PROVIDERS
        .iter()
        .filter(|p| !p.canonical_default)
        .map(|p| p.session_name(canonical, owner, false))
        .collect();
    harness::ALL
        .iter()
        .map(|a| {
            let p = a.provider();
            let (name, up) = if p.canonical_default {
                let sidecar = p.sidecar_name(canonical);
                let sidecar_up = has(&sidecar);
                let up = sidecar_up || (primary_up && owner == p.id && !others.iter().any(|n| n == primary_name));
                let name = if sidecar_up { sidecar } else if up { primary_name.to_string() } else { canonical.to_string() };
                (name, up)
            } else {
                let name = p.session_name(canonical, owner, false);
                let up = has(&name);
                (name, up)
            };
            // The model each live agent last answered with — `null` until its
            // first reply, and never looked up for a session that is down.
            let model = if up { crate::watch::live_model(w, p.id, probes, &name, place_path) } else { None };
            AgentSession { id: p.id, name, up, model }
        })
        .collect()
}


/// One repo's merged snapshot: core's live `ls` + DECLARED store overlay +
/// reconciled `lifecycle_effective` per place.
pub fn snapshot(w: &Watch, repo: &str) -> Result<serde_json::Value, String> {
    let project = Project::discover(Path::new(repo)).map_err(|e| e.msg)?;
    let mut v = serde_json::to_value(project.ls()).map_err(|e| e.to_string())?;
    // Unborn HEAD (git init, no commits): the repo lists fine but no worktree can
    // be created from it. Carried on the snapshot so the nav can offer the first
    // commit instead of letting `new` fail on an invalid object name.
    v["unborn"] = serde_json::Value::Bool(!git::has_commits(&project.main_root));
    let store = store::read_lenient(repo);
    let now = sysclock::now_epoch();
    // Read the declarations ONCE per snapshot, not per place. This runs on the
    // 3s poll, so it stays a single small JSON read — no per-profile filesystem
    // probes here (those live in `profiles_info`, which is called on sheet-open).
    let profiles = worktrees_core::profile::read_lenient();
    // What this repo's NEXT launch would use — so a session started under a
    // different (or since-unbound) profile reads as stale rather than merely
    // naming whatever it started with.
    // Resolved against the set already loaded above — not a second read of the
    // same file in the same tick.
    let effective = worktrees_core::profile::resolve_profile_id_in(&profiles, repo);
    let agent_panes = tmux::PaneList::fetch();
    let probes = worktrees_core::agent::live_probes();
    if let Some(places) = v.get_mut("places").and_then(|p| p.as_array_mut()) {
        for place in places.iter_mut() {
            let slug = place.get("slug").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let canonical = project.session_name(&slug);
            let primary_up = place.pointer("/tmux_session/up").and_then(|b| b.as_bool()).unwrap_or(false);
            let primary_name = place.pointer("/tmux_session/name").and_then(|s| s.as_str()).unwrap_or(&canonical).to_string();
            let place_path = place.get("path").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let sessions = agent_sessions_for(w, agent_panes.as_ref(), &probes, &canonical, &primary_name, primary_up, &place_path);
            // Watched only while the harness itself runs in the session: the
            // pane is `<cli> …; exec "$SHELL"`, so an agent that exited or was
            // killed leaves the session up with a shell in it, and a file
            // ending mid-turn (nothing writes an abort on a kill) would
            // otherwise hold the place green indefinitely. Every harness, not
            // a named one: `watch_lane` is where each is wired.
            for s in &sessions {
                let running = s.up && agent_panes.as_ref().is_some_and(|panes| panes.session_runs_program(&s.name));
                let launch = agent_panes.as_ref().and_then(|panes| panes.session_launch(&s.name));
                crate::watch::watch_lane(w, s.id, &place_path, running.then_some(&s.name), launch, &s.model);
            }
            let tmux_up = sessions.iter().any(|s| s.up);
            // Profiles are claude's recipe, so only a live Claude can be stale.
            let claude_up = sessions.iter().any(|s| s.up && s.id == provider::CLAUDE.id);
            // `tmux_session` stays on the default harness's session when it is
            // up (a stray pair keeps Claude's, which is what the nav names);
            // otherwise it follows the first live harness.
            let default_up = sessions.iter().any(|s| s.up && s.id == harness::default_adapter().provider().id);
            if let Some(live) = sessions.iter().find(|s| s.up).filter(|_| !default_up) {
                place["tmux_session"] = serde_json::json!({ "name": live.name, "up": true });
            }
            place["agent_sessions"] = serde_json::Value::Object(
                sessions
                    .into_iter()
                    .map(|s| (s.id.to_string(), serde_json::json!({ "name": s.name, "up": s.up, "model": s.model })))
                    .collect(),
            );
            let decl = store.places.get(&slug);
            place["declared"] = decl
                .map(|d| serde_json::to_value(d).unwrap_or(serde_json::Value::Null))
                .unwrap_or(serde_json::Value::Null);
            place["lifecycle_effective"] = serde_json::Value::String(store::reconcile(decl, tmux_up, now));

            // What a LIVE session is actually running, versus the profile as
            // edited since. Both derived from the launch stamp ops writes when a
            // session is created — a place with no stamp simply has no badge.
            let (mut pname, mut stale) = (serde_json::Value::Null, false);
            if let Some(d) = decl {
                if let Some(pid) = d.profile_id.as_deref() {
                    // A profile deleted mid-session must not make the badge
                    // vanish — the session is still running it. Name it as gone
                    // rather than showing nothing.
                    let name = profiles
                        .profiles
                        .get(pid)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| format!("{pid} (deleted)"));
                    pname = serde_json::Value::String(name);
                    // Only meaningful while the session is up: a closed place
                    // picks up the current profile on its next launch, so calling
                    // it "stale" would be noise.
                    if claude_up {
                        let edited = profiles
                            .profiles
                            .get(pid)
                            .map(|p| d.profile_epoch.unwrap_or(0) < p.updated_epoch)
                            .unwrap_or(true); // deleted counts as changed
                        // A REBIND is the edit a user most expects the badge to
                        // cover: the session is running one profile while the
                        // repo is now bound to another.
                        let rebound = effective.as_deref() != Some(pid);
                        stale = edited || rebound;
                    }
                } else if claude_up && effective.is_some() {
                    // Launched unprofiled, but a profile is bound now.
                    pname = serde_json::Value::Null;
                    stale = true;
                }
            }
            place["profile_name"] = pname;
            place["profile_stale"] = serde_json::Value::Bool(stale);
        }
    }
    Ok(v)
}

/// Single-repo snapshot (kept for direct use / back-compat).
pub fn list_places(ctx: &Ctx, repo: String) -> Result<serde_json::Value, String> {
    snapshot(&ctx.watch, &repo)
}


#[derive(Serialize)]
pub struct ProjectView {
    pub root: String,
    pub ok: bool,
    pub error: Option<String>,
    pub snapshot: Option<serde_json::Value>,
    /// The registry's name for it — what an agent elsewhere addresses it by.
    pub name: Option<String>,
    pub private: bool,
}
#[derive(Serialize)]
pub struct Workspace {
    pub projects: Vec<ProjectView>,
}




/// Every tracked project with its snapshot (or an error if it's gone/broken —
/// one dead repo greys its node without blanking the rest).
pub fn list_workspace(ctx: &Ctx) -> Result<Workspace, String> {
    // Fan out per project — each snapshot() is an independent git sweep, so
    // serial across projects stacked their latencies. Bounded: each snapshot
    // itself runs up to 16 concurrent git calls (place_json_par), so cap
    // projects-in-flight to keep the product (~4×16 processes) sane.
    let roots = read_projects(ctx);
    let reg = worktrees_core::registry::read_lenient();
    // One registry for the whole fan-out: each thread below resolves models
    // into the same cache, which is the point of it being per-process.
    let watch = ctx.watch.as_ref();
    let mut projects: Vec<ProjectView> = Vec::with_capacity(roots.len());
    for chunk in roots.chunks(4) {
        let batch: Vec<ProjectView> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .cloned()
                .map(|root| {
                    let entry = reg.by_root(&root).cloned();
                    s.spawn(move || {
                        let (name, private) = entry.map_or((None, false), |e| (Some(e.name), e.private));
                        match snapshot(&watch, &root) {
                            Ok(sn) => ProjectView { root, ok: true, error: None, snapshot: Some(sn), name, private },
                            Err(e) => ProjectView { root, ok: false, error: Some(e), snapshot: None, name, private },
                        }
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        projects.extend(batch);
    }
    Ok(Workspace { projects })
}

/// Add a git repo to the workspace (stored by its canonical main root, so a
/// subdir resolves to the repo and dedupes).
pub fn add_project(ctx: &Ctx, dir: String) -> Result<Workspace, String> {
    let project = Project::discover(Path::new(&dir)).map_err(|e| e.msg)?;
    crate::projects::import_app_projects(ctx);
    worktrees_core::registry::add(&project.main_root)?;
    list_workspace(ctx)
}

/// What a picked folder actually IS, before we try to add it. The Add-project
/// path needs to tell "not a repo" (offer `git init`) from "repo with no
/// commits" (offer a first commit) apart — `add_project`'s flat error string
/// can't carry that, and neither state is a dead end.
#[derive(Serialize)]
pub struct DirProbe {
    pub exists: bool,
    pub is_git: bool,
    pub has_commits: bool,
    /// The repo's `.worktrees.toml` has a `[plan]` section: Add existing
    /// pre-sets its planning choice to full. A suggestion, never applied.
    pub plan_suggested: bool,
}

pub fn probe_dir(dir: String) -> Result<DirProbe, String> {
    let exists = Path::new(&dir).is_dir();
    let is_git = exists && git::git_ok(&dir, &["rev-parse", "--is-inside-work-tree"]);
    let has_commits = is_git && git::has_commits(&dir);
    let plan_suggested = is_git && worktrees_core::planning::repo_suggests(Path::new(&dir));
    Ok(DirProbe { exists, is_git, has_commits, plan_suggested })
}

/// A project name becomes ONE path component under the chosen location, so it
/// may not steer one — the same rule `worktrees_core::sync::valid_name` applies
/// to a name that becomes a directory on a hub, plus a leading dot (nobody
/// typing a project name means "make it hidden").
///
/// The frontend checks the same shapes live, for the inline hint. This is the
/// one that decides: the field is a text box, and a text box is never evidence.
pub fn valid_project_name(n: &str) -> Result<(), String> {
    if n.is_empty() {
        return Err("give the project a name".into());
    }
    if n == "." || n == ".." {
        return Err("'.' and '..' are not names".into());
    }
    if n.starts_with('.') {
        return Err("a name starting with '.' would make a hidden folder".into());
    }
    if n.contains('/') {
        return Err("a name cannot contain '/' — the folder above it goes in Location".into());
    }
    if n.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a name cannot contain spaces or control characters".into());
    }
    Ok(())
}

/// A LEADING `~` (alone or before `/`) → `$HOME`. The Location field is typed
/// and pasted into, and `~/workspace` is what a person writes; `~user` is a
/// shell feature this deliberately is not, so it stays a literal directory name
/// rather than becoming a path to somebody else's home.
pub fn expand_home(p: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return PathBuf::from(p);
    }
    if p == "~" {
        return PathBuf::from(home);
    }
    match p.strip_prefix("~/") {
        Some(rest) => PathBuf::from(home).join(rest),
        None => PathBuf::from(p),
    }
}

/// Make a project that does not exist yet: `<location>/<name>`, created,
/// `git init`-ed with a first commit, and added to the workspace.
///
/// The dialog's fields are re-validated HERE — name shape, `~` expansion, and
/// the one refusal that matters: a target that already exists is never written
/// into. "Exists and is a repo" is answered differently from "exists", because
/// the first has an obvious next step (Add existing…) and the second is a
/// collision the user has to resolve.
///
/// Everything after the mkdir is the EXISTING path: `init_repo` (git init +
/// empty first commit + `add_project`), so a project created here is the same
/// object as one added from disk.
pub fn create_project(ctx: &Ctx, location: String, name: String) -> Result<Workspace, String> {
    let name = name.trim();
    valid_project_name(name)?;
    let loc = location.trim();
    if loc.is_empty() {
        return Err("give a location — the folder the project will be created in".into());
    }
    let base = expand_home(loc);
    // A relative location would resolve against the APP PROCESS's cwd — `/` for
    // a launchd GUI — and the resulting "permission denied" reads like a bug
    // rather than a rule. The rule is the honest answer.
    if !base.is_absolute() {
        return Err(format!(
            "location must be an absolute path (or start with ~) — got '{loc}'"
        ));
    }
    let target = base.join(name);
    crate::projects::forbid_new_path(&project_roots(ctx), &target)?;
    if target.exists() {
        return Err(if target.join(".git").exists() {
            format!(
                "{} already exists and is a git repo — add it with “Add existing…” instead.",
                target.display()
            )
        } else {
            format!("{} already exists — pick another name or location.", target.display())
        });
    }
    std::fs::create_dir_all(&target).map_err(|e| {
        applog("error", &format!("create_project mkdir {}: {e}", target.display()));
        format!("cannot create {}: {e}", target.display())
    })?;
    let dir = target.to_string_lossy().into_owned();
    applog("info", &format!("create_project {dir}"));
    // A failure here leaves the (empty) directory behind ON PURPOSE: removing a
    // path we have just been told we cannot fully write to is the wrong instinct,
    // and the message says where it is so the user can finish or delete it.
    let ws = init_repo(ctx, dir.clone()).map_err(|e| {
        applog("error", &format!("create_project init {dir}: {e}"));
        format!("created {dir}, but setting it up as a git repo failed: {e}")
    })?;
    ctx.emit("places:changed", serde_json::Value::Null);
    Ok(ws)
}

/// Running clones by the id the dialog chose, so `clone_cancel` can reach one.
/// A struct (not a bare static) so the tests own the map they assert against —
/// the `_in` seam AGENTS.md asks of process-global state.
#[derive(Default)]
pub struct CloneJobs(Mutex<HashMap<String, Arc<AtomicBool>>>);

impl CloneJobs {
    /// Register `id`; the guard unregisters it however the clone ends. A second
    /// clone under a live id is refused rather than sharing the first's flag.
    fn start<'a>(&'a self, id: &str) -> Result<(Arc<AtomicBool>, CloneJobGuard<'a>), String> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if m.contains_key(id) {
            return Err(format!("a clone with id {id} is already running"));
        }
        let flag = Arc::new(AtomicBool::new(false));
        m.insert(id.to_string(), flag.clone());
        Ok((flag, CloneJobGuard { jobs: self, id: id.to_string() }))
    }
    /// Cancel every running clone; returns their ids. For app exit: a clone
    /// left running would outlive the window as an orphaned `git` writing a
    /// folder nobody will add — cancelled, core removes that folder.
    pub fn cancel_all(&self) -> Vec<String> {
        let m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for f in m.values() {
            f.store(true, Ordering::SeqCst);
        }
        m.keys().cloned().collect()
    }
    pub fn is_idle(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
    /// `false` when nothing runs under `id` (already finished — not an error).
    pub fn cancel(&self, id: &str) -> bool {
        let m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.get(id).map(|f| f.store(true, Ordering::SeqCst)).is_some()
    }
}

pub struct CloneJobGuard<'a> {
    jobs: &'a CloneJobs,
    id: String,
}

impl Drop for CloneJobGuard<'_> {
    fn drop(&mut self) {
        self.jobs.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

pub static CLONE_JOBS: std::sync::LazyLock<CloneJobs> = std::sync::LazyLock::new(CloneJobs::default);

#[derive(Serialize)]
pub struct CloneDone {
    pub workspace: Workspace,
    /// The project's root as the workspace stores it (canonical) — what the
    /// dialog selects. `dir` is where the clone was written, as asked.
    pub root: String,
    pub dir: String,
    pub has_submodules: bool,
}

/// "Clone from URL…": clone into `<parent>/<name>` and add the result exactly
/// as "Add existing…" would — through `add_project`, the one door.
///
/// All of the clone is `worktrees_core::clone` (the CLI's `worktrees clone` is
/// the same code): the URL rules, the existing-target refusal, the no-prompt
/// environment, and the cleanup that removes only the directory this clone
/// created. Here is only the glue: progress over a `Channel` (the `sync_apply`
/// mechanism), a cancel flag `clone_cancel` can reach, and the blocking clone
/// moved off the async runtime.
///
/// An `Err` is core's `CloneError` (`{kind, message}`) so the dialog can tell a
/// cancel — which is not an error to show — from a failure.
///
/// Nothing the repository declares runs here or in `add_project` (ADR 0001):
/// registering reads no `.worktrees.toml` argv, and the first thing that could
/// run anything at all is a place the user creates later.
/// `on_progress` is a plain callback rather than a transport type: Tauri's
/// `Channel<CloneProgress>` and the server's framed JSON are both one of
/// these, and the handler never needed to know which. (It is NOT a
/// `ByteSink` — that carries a pty's raw bytes; this is typed progress.)
///
/// The body is fully blocking, where it used to put only the clone itself on
/// `spawn_blocking` and finish on the async executor. The wrapper now wraps
/// the whole call instead, which moves `add_project`'s git fan-out off the
/// executor as well — the direction AGENTS.md's async-handler rule points.
pub fn clone_project(
    ctx: &Ctx,
    id: String,
    url: String,
    parent: String,
    name: Option<String>,
    on_progress: &mut dyn FnMut(worktrees_core::clone::CloneProgress),
) -> Result<CloneDone, worktrees_core::clone::CloneError> {
    use worktrees_core::clone::{self as wclone, CloneError, CloneErrorKind};
    let invalid = |m: String| CloneError { kind: CloneErrorKind::Invalid, message: m };
    let src = wclone::parse_source(&url).map_err(invalid)?;
    let folder = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| src.name.clone());
    let target = wclone::plan_target(&parent, &folder)?;
    crate::projects::forbid_new_path(&project_roots(ctx), &target).map_err(invalid)?;
    // Never the raw URL: `https://user:TOKEN@host/…` is a legal thing to paste.
    let shown = wclone::redact_url(&src.url);
    applog("info", &format!("clone_project start {shown} -> {}", target.display()));
    let started = std::time::Instant::now();
    let src2 = src.clone();
    let t2 = target.clone();
    // Registered BEFORE the blocking task is scheduled, so a Cancel pressed in
    // the first instant still finds the flag.
    let (flag, guard) = CLONE_JOBS.start(&id).map_err(invalid)?;
    let out = {
        let _guard = guard;
        // A client that went away must not stop the clone. That is the
        // TRANSPORT's problem now, not this layer's: `on_progress` cannot
        // fail, and the Tauri wrapper swallows a closed channel after logging
        // it once, exactly as this function used to inline.
        wclone::clone_repo(&src2, &t2, &wclone::CloneOpts::default(), &flag, Some(on_progress))
    };
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            applog(
                if e.kind == CloneErrorKind::Cancelled { "info" } else { "warn" },
                &format!("clone_project {:?} {shown} ({:.1}s): {}", e.kind, started.elapsed().as_secs_f32(), e.message),
            );
            return Err(e);
        }
    };
    applog("info", &format!("clone_project ok {} ({:.1}s)", out.dir, started.elapsed().as_secs_f32()));
    // Half-success must not read as failure OR success: the files are there,
    // only the bookkeeping failed — say both, and how to finish by hand.
    let workspace = add_project(ctx, out.dir.clone()).map_err(|e| {
        applog("error", &format!("clone_project cloned but add_project failed {}: {e}", out.dir));
        CloneError {
            kind: CloneErrorKind::Other,
            message: format!(
                "Cloned to {}, but adding it to the workspace failed: {e}\nAdd it with “Add existing…” and pick that folder.",
                out.dir
            ),
        }
    })?;
    ctx.emit("places:changed", serde_json::Value::Null);
    let root = Project::discover(Path::new(&out.dir)).map(|p| p.main_root).unwrap_or_else(|_| out.dir.clone());
    Ok(CloneDone { workspace, root, dir: out.dir, has_submodules: out.has_submodules })
}

/// Cancel the clone running under `id`. `false` when none is (it finished
/// first) — the dialog then simply receives that clone's own result.
pub fn clone_cancel(id: String) -> Result<bool, String> {
    Ok(CLONE_JOBS.cancel(&id))
}

/// `git init` + a first commit, then add the repo to the workspace. The commit
/// is not optional politeness: without it HEAD is unborn and the very next thing
/// the user does (new worktree) fails on an invalid object name.
/// `--allow-empty` keeps it a pure bootstrap — it permits an empty commit rather
/// than demanding one, so the seeded `.gitignore` (below) rides along in it.
pub fn init_repo(ctx: &Ctx, dir: String) -> Result<Workspace, String> {
    if !Path::new(&dir).is_dir() {
        return Err(format!("{dir} is not a directory"));
    }
    crate::projects::forbid_new_path(&project_roots(ctx), Path::new(&dir))?;
    if !git::git_ok(&dir, &["rev-parse", "--is-inside-work-tree"]) {
        git::git_status_captured(&dir, &["init"]).map_err(|e| {
            applog("error", &format!("git init failed in {dir}: {e}"));
            e
        })?;
    }
    if !git::has_commits(&dir) {
        // Only on the bootstrap path: a repo we are about to make the FIRST
        // commit in is one nobody has opinions about yet. A repo that already
        // has history is the user's, and seeding files into it is uninvited.
        if seed_gitignore(&dir)? {
            git::git_status_captured(&dir, &["add", ".gitignore"])?;
        }
        first_commit(&dir)?;
    }
    add_project(ctx, dir)
}

/// Bootstrap commit for a repo that IS tracked but has an unborn HEAD.
pub fn create_initial_commit(ctx: &Ctx, repo: String) -> Result<Workspace, String> {
    if git::has_commits(&repo) {
        return list_workspace(ctx);
    }
    first_commit(&repo)?;
    list_workspace(ctx)
}

/// The `.gitignore` a project created HERE starts with. Returns whether it was
/// written — an existing `.gitignore` belongs to the user and is never touched,
/// never appended to, so this is create-or-nothing.
///
/// The two app-state entries are not a preference: `.worktrees.places.json` is
/// per-MACHINE declared state and `worktrees sync` exists to ferry it between
/// machines out-of-band, so committing it is the wrong shape — and this repo's
/// own `.gitignore` says the same. Without them a brand-new project's very first
/// `git status` is dirty with a file the app itself just wrote. The planning
/// docs ride along because they are working memory by construction.
pub fn seed_gitignore(dir: &str) -> Result<bool, String> {
    let path = Path::new(dir).join(".gitignore");
    if path.exists() {
        return Ok(false);
    }
    let body = "\
# worktrees — per-machine app state (moved between machines by `worktrees sync`, not git)
/.worktrees.places.json
/.worktrees/

# planning docs — local working memory
/task_plan.md
/findings.md
/progress.md
";
    std::fs::write(&path, body).map_err(|e| {
        applog("error", &format!("seeding .gitignore in {dir}: {e}"));
        format!("cannot write {}: {e}", path.display())
    })?;
    Ok(true)
}

/// The bootstrap commit. Git refuses to commit without an identity, and its
/// stderr says exactly which `git config` line is missing — pass it through
/// rather than paraphrasing.
pub fn first_commit(dir: &str) -> Result<(), String> {
    git::git_status_captured(dir, &["commit", "--allow-empty", "-m", "Initial commit"]).map_err(|e| {
        applog("error", &format!("initial commit failed in {dir}: {e}"));
        e
    })
}

pub fn remove_project(ctx: &Ctx, root: String) -> Result<Workspace, String> {
    crate::projects::import_app_projects(ctx);
    worktrees_core::registry::remove(&root)?;
    list_workspace(ctx)
}

/// Re-order the workspace's projects (nav drag). The registry's order IS the
/// nav order. The frontend's list is a snapshot that can be stale by the time
/// the drop lands, so core treats it as a PREFERENCE, under its lock: roots it
/// no longer has are ignored and roots the dragger never saw are kept,
/// appended (`registry::reorder`). A drag can reorder; it cannot delete.
pub fn reorder_projects(ctx: &Ctx, roots: Vec<String>) -> Result<Workspace, String> {
    crate::projects::import_app_projects(ctx);
    worktrees_core::registry::reorder(&roots)?;
    list_workspace(ctx)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_cancel_reaches_a_running_clone_and_only_while_it_runs() {
        let jobs = CloneJobs::default();
        assert!(!jobs.cancel("a"), "nothing running yet");
        let (flag, guard) = jobs.start("a").unwrap();
        assert!(jobs.start("a").is_err(), "a live id is not shared");
        assert!(!flag.load(Ordering::SeqCst));
        assert!(jobs.cancel("a"));
        assert!(flag.load(Ordering::SeqCst), "cancel set THIS clone's flag");
        drop(guard);
        assert!(!jobs.cancel("a"), "a finished clone is unregistered");
        assert!(jobs.start("a").is_ok(), "and its id is free again");
    }

    #[test]
    fn exit_cancels_every_running_clone() {
        let jobs = CloneJobs::default();
        assert!(jobs.cancel_all().is_empty() && jobs.is_idle());
        let (a, ga) = jobs.start("a").unwrap();
        let (b, gb) = jobs.start("b").unwrap();
        let mut ids = jobs.cancel_all();
        ids.sort();
        assert_eq!(ids, ["a", "b"]);
        assert!(a.load(Ordering::SeqCst) && b.load(Ordering::SeqCst));
        assert!(!jobs.is_idle());
        drop((ga, gb));
        assert!(jobs.is_idle());
    }

    /// The snapshot's per-harness sessions, pinned against the two-provider
    /// formula they replaced (kept here verbatim as the reference), across a
    /// legacy canonical Codex, both sidecars, a primary Claude, a shell, and
    /// no tmux at all.
    #[test]
    fn agent_sessions_match_the_two_provider_rule_they_replaced() {
        fn old(panes: Option<&tmux::PaneList>, canonical: &str, primary: &str, primary_up: bool) -> Vec<(String, bool)> {
            let legacy_codex = panes.is_some_and(|p| p.canonical_provider(canonical).id == "codex");
            let codex_name = if legacy_codex { canonical.to_string() } else { format!("{canonical}~agent~codex") };
            let codex_up = panes.is_some_and(|p| p.has_session(&codex_name));
            let sidecar = format!("{canonical}~agent~claude");
            let sidecar_up = panes.is_some_and(|p| p.has_session(&sidecar));
            let claude_up = sidecar_up || (primary_up && !legacy_codex && primary != codex_name);
            let claude_name = if sidecar_up { sidecar } else if claude_up { primary.to_string() } else { canonical.to_string() };
            vec![(claude_name, claude_up), (codex_name, codex_up)]
        }
        let rows = |r: &[(&str, &str)]| {
            tmux::PaneList::from_rows(r.iter().map(|(s, c)| (s.to_string(), "/w".to_string(), c.to_string())).collect())
        };
        let c = "p-feat";
        let cases: Vec<(Option<tmux::PaneList>, &str, bool)> = vec![
            (None, c, false),
            (None, c, true),
            (Some(rows(&[])), c, false),
            (Some(rows(&[(c, "claude")])), c, true),
            (Some(rows(&[(c, "node")])), c, true),
            (Some(rows(&[(c, "zsh")])), c, true),
            (Some(rows(&[(c, "codex")])), c, true),
            (Some(rows(&[("p-feat~agent~codex", "codex")])), c, false),
            (Some(rows(&[("p-feat~agent~codex", "node")])), c, false),
            (Some(rows(&[("p-feat~agent~claude", "claude")])), c, false),
            (Some(rows(&[("p-feat~agent~claude", "claude"), (c, "zsh")])), c, true),
            (Some(rows(&[("adopted", "claude")])), "adopted", true),
            (Some(rows(&[("p-feat~agent~codex", "codex")])), "p-feat~agent~codex", true),
        ];
        for (panes, primary, up) in &cases {
            let got: Vec<(String, bool)> = agent_sessions_for(&Watch::default(), panes.as_ref(), &[], c, primary, *up, "/nonexistent")
                .into_iter()
                .map(|s| (s.name, s.up))
                .collect();
            // Claude and Codex exactly as the formula had them; pi, which the
            // formula predates, is its sidecar and nothing else.
            assert_eq!(&got[..2], &old(panes.as_ref(), c, primary, *up)[..], "primary {primary} up={up}");
            let pi_up = panes.as_ref().is_some_and(|p| p.has_session("p-feat~agent~pi"));
            assert_eq!(got[2], ("p-feat~agent~pi".to_string(), pi_up), "primary {primary} up={up}");
            let kimi_up = panes.as_ref().is_some_and(|p| p.has_session("p-feat~agent~kimi"));
            assert_eq!(got[3], ("p-feat~agent~kimi".to_string(), kimi_up), "primary {primary} up={up}");
        }
        let kimi = rows(&[("p-feat~agent~kimi", "kimi")]);
        let got = agent_sessions_for(&Watch::default(), Some(&kimi), &[], c, c, false, "/nonexistent");
        assert!(got[3].up && !got[0].up && !got[1].up && !got[2].up, "kimi's sidecar is kimi's");
        let pi = rows(&[("p-feat~agent~pi", "node")]);
        let got = agent_sessions_for(&Watch::default(), Some(&pi), &[], c, c, false, "/nonexistent");
        assert!(got[2].up && !got[0].up && !got[1].up, "a node pane in pi's sidecar is pi, not Claude");
        // pi typed into the place's OWN session: tmux says `node`, the tty's
        // foreground leader says `pi`. The nav dot's lane is pi's, at the
        // canonical name — before, Claude claimed it and pi_tick never watched.
        let typed = tmux::PaneList::from_panes(vec![tmux::Pane {
            session: c.to_string(),
            path: "/w".to_string(),
            cmd: "node".to_string(),
            fg: Some("pi".to_string()),
            ..tmux::Pane::default()
        }]);
        let got = agent_sessions_for(&Watch::default(), Some(&typed), &[], c, c, true, "/nonexistent");
        let up: Vec<(&str, &str, bool)> = got.iter().map(|s| (s.id, s.name.as_str(), s.up)).collect();
        assert_eq!(
            up,
            [("claude", c, false), ("codex", "p-feat~agent~codex", false), ("pi", c, true), ("kimi", "p-feat~agent~kimi", false)]
        );
        let ids: Vec<&str> = agent_sessions_for(&Watch::default(), None, &[], c, c, false, "").iter().map(|s| s.id).collect();
        assert_eq!(ids, ["claude", "codex", "pi", "kimi"], "one entry per harness, registry order");
    }

    /// The dialog validates as you type; this is the check that DECIDES. Every
    /// rejected shape here is one that would otherwise become a path component
    /// somewhere the user did not point at — or a folder they cannot see.
    #[test]
    fn a_new_projects_name_may_not_steer_a_path() {
        for ok in ["worktrees", "casa-del-valle", "proj_2", "a"] {
            assert!(valid_project_name(ok).is_ok(), "{ok} should be a name");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "/abs", "two words", "tab\there", "..\\x/y"] {
            assert!(valid_project_name(bad).is_err(), "{bad:?} should be refused");
        }
        // the message is the UI's inline error, so it has to name the problem
        assert!(valid_project_name("a/b").unwrap_err().contains("Location"));
        assert!(valid_project_name(".x").unwrap_err().contains("hidden"));
    }

    /// The Location field is TYPED. `~/workspace` is what a person writes, and
    /// it is also this dialog's own default — so the expansion is on the path
    /// every created project goes through, not a convenience.
    #[test]
    fn a_typed_location_expands_only_its_own_leading_tilde() {
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(expand_home("~"), PathBuf::from(&home));
        assert_eq!(expand_home("~/workspace"), PathBuf::from(&home).join("workspace"));
        assert_eq!(expand_home("~/a/b"), PathBuf::from(&home).join("a/b"));
        // NOT a shell: another user's home, and a tilde anywhere but the front,
        // are literal directory names
        assert_eq!(expand_home("~other/x"), PathBuf::from("~other/x"));
        assert_eq!(expand_home("/tmp/~/x"), PathBuf::from("/tmp/~/x"));
        assert_eq!(expand_home("/Users/dp/work"), PathBuf::from("/Users/dp/work"));
        // …and everything that comes out relative (including those literals) is
        // refused by create_project: a GUI process's cwd is `/`, and resolving a
        // typed "workspace" against it turns a rule into a permission error.
        assert!(!expand_home("workspace").is_absolute());
        assert!(!expand_home("~other/x").is_absolute());
    }

    /// A project created here should answer its first `git status` with
    /// "nothing to commit" — the app writes `.worktrees.places.json` into it
    /// moments later, and an untracked file the tool itself made is noise the
    /// user did not choose. The entries are ANCHORED (`/…`) so they mean the
    /// project root, not any nested directory that happens to share the name.
    ///
    /// The other half of the rule is restraint: seeding is create-or-nothing,
    /// because an existing `.gitignore` is a file the user (or a template, or a
    /// `git clone`) authored, and appending to it is a decision that is not
    /// ours. `init_repo` is reachable for a directory that is already a repo.
    #[test]
    fn a_seeded_gitignore_is_written_once_and_never_over_the_users() {
        let dir = std::env::temp_dir().join(format!("wt-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().to_string();
        let path = dir.join(".gitignore");

        assert!(seed_gitignore(&d).unwrap(), "an absent .gitignore is written");
        let body = std::fs::read_to_string(&path).unwrap();
        for entry in [
            "/.worktrees.places.json",
            "/.worktrees/",
            "/task_plan.md",
            "/findings.md",
            "/progress.md",
        ] {
            assert!(body.lines().any(|l| l == entry), "missing {entry} in:\n{body}");
        }

        // second call: the file it finds is the one it wrote, and it still keeps
        // its hands off — so bootstrapping twice cannot double the entries
        assert!(!seed_gitignore(&d).unwrap(), "an existing .gitignore is left alone");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body, "idempotent");

        // …and the same holds for a file we did NOT write, byte for byte
        let mine = "node_modules\n";
        std::fs::write(&path, mine).unwrap();
        assert!(!seed_gitignore(&d).unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            mine,
            "the user's .gitignore is never touched or appended to",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
