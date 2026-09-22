// worktrees UI — Tauri backend. Uses worktrees-core as a LIBRARY (in-process; no
// subprocess, no WORKTREES_BIN). Two jobs of its own:
//   1. state    — core computes derived `ls`; core::store owns the declared sidecar;
//                 the app merges them + reconciles lifecycle_effective for the UI.
//   2. PTY host — attaches to a live tmux session for the place's canonical
//                 shell, and OWNS the dock's scratch shells outright (no tmux).
// See DESIGN.md / MIGRATION.md.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager, State};
use worktrees_core::ui::CaptureUi;
use worktrees_core::{git, mcpsetup, mention, ops, store, sync, sysclock, tmux, Project, Ui};

// The documentation viewer: one supervised child, N places, and a port. Its own
// file because it is the only part of this backend that can hand a document to
// something outside the app, and every rule in it is a refusal — the security
// surface is worth reading in one piece.
mod docserver;
mod viewer;

// ── app log ──────────────────────────────────────────────────────────────────
// Plain append-only file at the platform's log location (macOS: ~/Library/Logs/
// <identifier>/app.log — Console.app finds it). Deliberately AppHandle-free so
// the panic hook can use it. Every op failure, frontend error, and panic lands
// here; Settings → Logs opens the folder / tails it.

const APP_IDENT: &str = "net.casadelvalle.worktrees";

fn log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Logs").join(APP_IDENT)
    } else {
        // XDG-ish fallback (linux dev)
        PathBuf::from(home).join(".local/share").join(APP_IDENT).join("logs")
    }
}

fn log_file() -> PathBuf {
    log_dir().join("app.log")
}

/// epoch → "YYYY-MM-DD HH:MM:SS" UTC (civil-from-days; avoids a chrono dep).
fn fmt_utc(epoch: i64) -> String {
    let (days, secs) = (epoch.div_euclid(86_400), epoch.rem_euclid(86_400));
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{y:04}-{mth:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

fn applog(level: &str, msg: &str) {
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = log_file();
    // single-slot rotation at ~1MB so the log can't grow unbounded
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("app.log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ts = fmt_utc(sysclock::now_epoch());
        let _ = writeln!(f, "{ts}Z [{level}] {msg}");
    }
}

#[derive(Serialize)]
struct LogInfo {
    dir: String,
    file: String,
}

#[tauri::command]
async fn log_info() -> Result<LogInfo, String> {
    Ok(LogInfo { dir: log_dir().to_string_lossy().into(), file: log_file().to_string_lossy().into() })
}

/// Frontend errors land in the same file, tagged `ui:`.
#[tauri::command]
async fn log_event(level: String, msg: String) -> Result<(), String> {
    let lv = match level.as_str() {
        "error" | "warn" | "info" => level.as_str(),
        _ => "info",
    };
    let mut m = msg;
    m.truncate(4000);
    applog(lv, &format!("ui: {m}"));
    Ok(())
}

/// The CHANGELOG ships inside the binary — the "What's new" sheet renders the
/// sections between the last-seen and current versions with zero network.
#[derive(Serialize)]
struct ChangelogInfo {
    version: String,
    changelog: String,
}

#[tauri::command]
async fn get_changelog() -> Result<ChangelogInfo, String> {
    Ok(ChangelogInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        changelog: include_str!("../../../CHANGELOG.md").to_string(),
    })
}

#[tauri::command]
async fn log_tail(lines: Option<usize>) -> Result<String, String> {
    let n = lines.unwrap_or(200).min(2000);
    let text = std::fs::read_to_string(log_file()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(n);
    Ok(all[start..].join("\n"))
}

// ── AI profiles + the skill store ────────────────────────────────────────────
//
// Thin wrappers: core owns profiles.json and the skill store (the CLI reads the
// same files), so these must never keep their own copy of that state.

#[tauri::command]
async fn profiles_info(repo: String) -> Result<serde_json::Value, String> {
    // Filesystem probes per profile (`ever_launched`) live here rather than in
    // `snapshot`, because this is called on sheet-open and that is on the 3s poll.
    serde_json::to_value(worktrees_core::profile::info_for(&repo)).map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_profile(profile: serde_json::Value) -> Result<String, String> {
    let p: worktrees_core::profile::Profile =
        serde_json::from_value(profile).map_err(|e| format!("invalid profile: {e}"))?;
    worktrees_core::profile::save(p).inspect_err(|e| applog("error", &format!("save_profile: {e}")))
}

#[tauri::command]
async fn new_profile_id(name: String) -> Result<String, String> {
    let taken: Vec<String> = worktrees_core::profile::read_lenient().profiles.keys().cloned().collect();
    Ok(worktrees_core::profile::new_id_from(&name, &taken))
}

/// What deleting a profile actually costs, reported rather than done silently.
#[derive(Serialize)]
struct ProfileRemoval {
    /// The materialized dir is LEFT ON DISK — it holds the session transcripts.
    dir: Option<String>,
    /// The keychain item's service name, when it was recorded — so the message
    /// can name what to look for instead of sending the user hunting.
    keychain_service: Option<String>,
    /// A keychain item may remain. worktrees never touches credentials (that is
    /// the invariant that made this whole feature safe), so it cannot delete one
    /// either — the UI tells the user where it is instead of pretending.
    keychain_hint: bool,
}

#[tauri::command]
async fn delete_profile(id: String) -> Result<ProfileRemoval, String> {
    let dir = worktrees_core::profile::profile_dir(&id).map(|d| d.to_string_lossy().into_owned());
    let launched = worktrees_core::profile::ever_launched(&id);
    let keychain_service = worktrees_core::profile::remove(&id)
        .inspect_err(|e| applog("error", &format!("delete_profile: {e}")))?;
    Ok(ProfileRemoval { dir, keychain_service, keychain_hint: launched })
}

#[tauri::command]
async fn set_project_profile(repo: String, id: Option<String>) -> Result<(), String> {
    worktrees_core::profile::assign(&repo, id.as_deref())
        .inspect_err(|e| applog("error", &format!("set_project_profile: {e}")))
}

#[tauri::command]
async fn set_default_profile(id: Option<String>) -> Result<(), String> {
    worktrees_core::profile::set_default(id.as_deref())
        .inspect_err(|e| applog("error", &format!("set_default_profile: {e}")))
}

#[tauri::command]
async fn skills_list() -> Result<serde_json::Value, String> {
    serde_json::to_value(worktrees_core::skillstore::list()).map_err(|e| e.to_string())
}

/// Read a candidate skill directory WITHOUT installing it — the review step.
#[tauri::command]
async fn skill_inspect(path: String) -> Result<serde_json::Value, String> {
    let i = worktrees_core::skillstore::inspect(Path::new(&path))?;
    Ok(serde_json::json!({
        "name": i.name, "description": i.description,
        "capabilities": i.capabilities, "files": i.files, "bytes": i.bytes,
        "skill_md": i.skill_md,
    }))
}

#[tauri::command]
async fn skill_install_local(path: String) -> Result<serde_json::Value, String> {
    let e = worktrees_core::skillstore::install_local(Path::new(&path))
        .inspect_err(|e| applog("error", &format!("skill_install_local: {e}")))?;
    serde_json::to_value(e).map_err(|e| e.to_string())
}

/// Clone, inspect, discard. Installs nothing — see `skill_install_git`.
#[tauri::command]
async fn skill_preview_git(url: String, rev: Option<String>) -> Result<serde_json::Value, String> {
    let p = worktrees_core::skillstore::preview_git(&url, rev.as_deref().unwrap_or(""))
        .inspect_err(|e| applog("warn", &format!("skill_preview_git: {e}")))?;
    serde_json::to_value(p).map_err(|e| e.to_string())
}

/// Install at the sha the user reviewed. Refuses if the branch moved.
#[tauri::command]
async fn skill_install_git(
    url: String,
    rev: Option<String>,
    sha: String,
    name: String,
) -> Result<serde_json::Value, String> {
    let e = worktrees_core::skillstore::install_git_pinned(
        &url,
        rev.as_deref().unwrap_or(""),
        &sha,
        &name,
    )
    .inspect_err(|e| applog("error", &format!("skill_install_git: {e}")))?;
    serde_json::to_value(e).map_err(|e| e.to_string())
}

#[tauri::command]
async fn skill_remove(name: String) -> Result<Vec<String>, String> {
    worktrees_core::skillstore::remove(&name)
        .inspect_err(|e| applog("error", &format!("skill_remove: {e}")))
}

// ── state: core-derived places + declared overlay + reconciled lifecycle ─────

/// One repo's merged snapshot: core's live `ls` + DECLARED store overlay +
/// reconciled `lifecycle_effective` per place.
fn snapshot(repo: &str) -> Result<serde_json::Value, String> {
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
    if let Some(places) = v.get_mut("places").and_then(|p| p.as_array_mut()) {
        for place in places.iter_mut() {
            let slug = place.get("slug").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let canonical = project.session_name(&slug);
            let legacy_codex = agent_panes.as_ref().is_some_and(|panes| panes.session_is_codex(&canonical));
            let codex_name = if legacy_codex { canonical.clone() } else { tmux::codex_session_name(&canonical) };
            let codex_up = agent_panes.as_ref().is_some_and(|panes| panes.has_session(&codex_name));
            let primary_up = place.pointer("/tmux_session/up").and_then(|b| b.as_bool()).unwrap_or(false);
            let primary_name = place.pointer("/tmux_session/name").and_then(|s| s.as_str()).unwrap_or(&canonical).to_string();
            let claude_sidecar = tmux::claude_session_name(&canonical);
            let sidecar_up = agent_panes.as_ref().is_some_and(|panes| panes.has_session(&claude_sidecar));
            let claude_up = sidecar_up || (primary_up && !legacy_codex && primary_name != codex_name);
            let claude_name = if sidecar_up { claude_sidecar } else if claude_up { primary_name } else { canonical.clone() };
            place["agent_sessions"] = serde_json::json!({
                "claude": { "name": claude_name, "up": claude_up },
                "codex": { "name": codex_name, "up": codex_up }
            });
            let tmux_up = claude_up || codex_up;
            if !claude_up && codex_up {
                place["tmux_session"] = serde_json::json!({ "name": codex_name, "up": true });
            }
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
#[tauri::command]
async fn list_places(repo: String) -> Result<serde_json::Value, String> {
    snapshot(&repo)
}

// ── multi-project workspace (tracked in the app config dir) ──────────────────

#[derive(Serialize)]
struct ProjectView {
    root: String,
    ok: bool,
    error: Option<String>,
    snapshot: Option<serde_json::Value>,
}
#[derive(Serialize)]
struct Workspace {
    projects: Vec<ProjectView>,
}

fn projects_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("projects.json"))
}
fn read_projects(app: &AppHandle) -> Vec<String> {
    projects_file(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
}
fn write_projects(app: &AppHandle, list: &[String]) -> Result<(), String> {
    let p = projects_file(app)?;
    let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(p, json).map_err(|e| e.to_string())
}

/// Every tracked project with its snapshot (or an error if it's gone/broken —
/// one dead repo greys its node without blanking the rest).
#[tauri::command]
async fn list_workspace(app: AppHandle) -> Result<Workspace, String> {
    // Fan out per project — each snapshot() is an independent git sweep, so
    // serial across projects stacked their latencies. Bounded: each snapshot
    // itself runs up to 16 concurrent git calls (place_json_par), so cap
    // projects-in-flight to keep the product (~4×16 processes) sane.
    let roots = read_projects(&app);
    let mut projects: Vec<ProjectView> = Vec::with_capacity(roots.len());
    for chunk in roots.chunks(4) {
        let batch: Vec<ProjectView> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .cloned()
                .map(|root| {
                    s.spawn(move || match snapshot(&root) {
                        Ok(sn) => ProjectView { root, ok: true, error: None, snapshot: Some(sn) },
                        Err(e) => ProjectView { root, ok: false, error: Some(e), snapshot: None },
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
#[tauri::command]
async fn add_project(app: AppHandle, dir: String) -> Result<Workspace, String> {
    let project = Project::discover(Path::new(&dir)).map_err(|e| e.msg)?;
    let root = project.main_root.clone();
    let mut roots = read_projects(&app);
    if !roots.contains(&root) {
        roots.push(root);
        write_projects(&app, &roots)?;
    }
    list_workspace(app).await
}

/// What a picked folder actually IS, before we try to add it. The Add-project
/// path needs to tell "not a repo" (offer `git init`) from "repo with no
/// commits" (offer a first commit) apart — `add_project`'s flat error string
/// can't carry that, and neither state is a dead end.
#[derive(Serialize)]
struct DirProbe {
    exists: bool,
    is_git: bool,
    has_commits: bool,
}

#[tauri::command]
async fn probe_dir(dir: String) -> Result<DirProbe, String> {
    let exists = Path::new(&dir).is_dir();
    let is_git = exists && git::git_ok(&dir, &["rev-parse", "--is-inside-work-tree"]);
    let has_commits = is_git && git::has_commits(&dir);
    Ok(DirProbe { exists, is_git, has_commits })
}

/// A project name becomes ONE path component under the chosen location, so it
/// may not steer one — the same rule `worktrees_core::sync::valid_name` applies
/// to a name that becomes a directory on a hub, plus a leading dot (nobody
/// typing a project name means "make it hidden").
///
/// The frontend checks the same shapes live, for the inline hint. This is the
/// one that decides: the field is a text box, and a text box is never evidence.
fn valid_project_name(n: &str) -> Result<(), String> {
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
fn expand_home(p: &str) -> PathBuf {
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
#[tauri::command]
async fn create_project(app: AppHandle, location: String, name: String) -> Result<Workspace, String> {
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
    let ws = init_repo(app.clone(), dir.clone()).await.map_err(|e| {
        applog("error", &format!("create_project init {dir}: {e}"));
        format!("created {dir}, but setting it up as a git repo failed: {e}")
    })?;
    let _ = app.emit("places:changed", ());
    Ok(ws)
}

/// `git init` + a first commit, then add the repo to the workspace. The commit
/// is not optional politeness: without it HEAD is unborn and the very next thing
/// the user does (new worktree) fails on an invalid object name.
/// `--allow-empty` keeps it a pure bootstrap — it permits an empty commit rather
/// than demanding one, so the seeded `.gitignore` (below) rides along in it.
#[tauri::command]
async fn init_repo(app: AppHandle, dir: String) -> Result<Workspace, String> {
    if !Path::new(&dir).is_dir() {
        return Err(format!("{dir} is not a directory"));
    }
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
    add_project(app, dir).await
}

/// Bootstrap commit for a repo that IS tracked but has an unborn HEAD.
#[tauri::command]
async fn create_initial_commit(app: AppHandle, repo: String) -> Result<Workspace, String> {
    if git::has_commits(&repo) {
        return list_workspace(app).await;
    }
    first_commit(&repo)?;
    list_workspace(app).await
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
fn seed_gitignore(dir: &str) -> Result<bool, String> {
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
fn first_commit(dir: &str) -> Result<(), String> {
    git::git_status_captured(dir, &["commit", "--allow-empty", "-m", "Initial commit"]).map_err(|e| {
        applog("error", &format!("initial commit failed in {dir}: {e}"));
        e
    })
}

#[tauri::command]
async fn remove_project(app: AppHandle, root: String) -> Result<Workspace, String> {
    let mut roots = read_projects(&app);
    roots.retain(|r| r != &root);
    write_projects(&app, &roots)?;
    list_workspace(app).await
}

/// Re-order the workspace's projects (nav drag). `projects.json` IS the order —
/// there is no separate order field — so this rewrites the file.
///
/// The frontend's list is a snapshot that can be stale by the time the drop
/// lands (another window added a project, a `remove_project` raced it), so the
/// incoming list is treated as a PREFERENCE, not the truth: roots the file no
/// longer has are dropped, and roots the frontend never saw are kept, appended
/// in their existing order. A drag can reorder the workspace; it must not be
/// able to delete from it.
fn merge_project_order(current: &[String], want: Vec<String>) -> Vec<String> {
    let known: std::collections::HashSet<&String> = current.iter().collect();
    let mut seen = std::collections::HashSet::new();
    let mut next: Vec<String> = want
        .into_iter()
        .filter(|r| known.contains(r) && seen.insert(r.clone()))
        .collect();
    next.extend(current.iter().filter(|r| !seen.contains(*r)).cloned());
    next
}

#[tauri::command]
async fn reorder_projects(app: AppHandle, roots: Vec<String>) -> Result<Workspace, String> {
    let current = read_projects(&app);
    let next = merge_project_order(&current, roots);
    if next != current {
        write_projects(&app, &next)?;
    }
    list_workspace(app).await
}

// ── UI settings (app-global; ui-state.json in app-config-dir) ────────────────
// Kept SEPARATE from per-repo declared state (.worktrees.places.json). Free-form
// JSON so the frontend owns the schema; a corrupt/absent file → null (defaults).

fn ui_state_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("ui-state.json"))
}

#[derive(Serialize)]
struct SettingsInfo {
    dir: String,
    file: String,
}

/// Config dir + the ui-state.json path — for Settings → Data (reveal the file).
#[tauri::command]
async fn settings_info(app: AppHandle) -> Result<SettingsInfo, String> {
    let file = ui_state_file(&app)?;
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(SettingsInfo { dir: dir.to_string_lossy().into(), file: file.to_string_lossy().into() })
}

#[tauri::command]
async fn get_settings(app: AppHandle) -> Result<Option<serde_json::Value>, String> {
    let p = ui_state_file(&app)?;
    match std::fs::read(&p) {
        Ok(b) => Ok(serde_json::from_slice(&b).ok()),
        Err(_) => Ok(None),
    }
}

#[tauri::command]
async fn set_settings(app: AppHandle, settings: serde_json::Value) -> Result<(), String> {
    let p = ui_state_file(&app)?;
    let json = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(p, json).map_err(|e| e.to_string())
}

const LIFECYCLE_LABELS: [&str; 4] = ["closed", "saved", "archived", "abandoned"];

/// Set — or with an empty label CLEAR — a place's declared lifecycle.
///
/// Clearing exists because the nav's tier groups are draggable and two of them
/// (Active, Idle) are DERIVED by `store::reconcile` rather than declared. There
/// is no label that means "idle"; the closest true statement is "no declared
/// label, let the live state speak", and that is what dropping a row on Idle
/// writes. Faking it by stamping `last_opened_epoch` would lie to the Recent
/// lens about when the place was last used.
#[tauri::command]
async fn set_lifecycle(repo: String, slug: String, label: String) -> Result<(), String> {
    if !label.is_empty() && !LIFECYCLE_LABELS.contains(&label.as_str()) {
        return Err(format!("invalid lifecycle label: {label}"));
    }
    store::edit(&repo, &slug, |d| {
        d.lifecycle = if label.is_empty() { None } else { Some(label) }
    })
}

#[tauri::command]
async fn set_pin(repo: String, slug: String, on: bool) -> Result<(), String> {
    store::edit(&repo, &slug, |d| d.pinned = Some(on))
}

#[tauri::command]
async fn set_note(repo: String, slug: String, note: String) -> Result<(), String> {
    store::edit(&repo, &slug, |d| {
        d.note = if note.trim().is_empty() { None } else { Some(note) }
    })
}

/// Drop a reference to one place into ANOTHER place's Claude session.
///
/// The nav drag's landing. What arrives is the same `@worktrees:place://…`
/// token the `@` menu completes, built in Rust from `worktrees_core::mention`
/// so there is exactly one implementation of it — the client matches a mention
/// against its cached resource list by exact string equality, so a frontend
/// copy that drifted by a character would reference the wrong place silently
/// rather than erroring.
///
/// `into_session` rather than a slug because the receiving place may be on an
/// ADOPTED session whose name is not the canonical one; the frontend already
/// holds the real name from `ls`.
#[tauri::command]
async fn drop_reference(
    repo: String,
    slug: String,
    into_slug: String,
    into_session: String,
) -> Result<String, String> {
    let project = Project::discover(std::path::Path::new(&repo)).map_err(|e| e.msg)?;
    let places = project.place_index();
    let uri = mention::uri_for(&places, &slug)
        .ok_or_else(|| format!("no such place: {slug}"))?;
    // `mcpsetup::claude_json_path()` rather than hand-building it from $HOME:
    // that module owns where claude's config lives.
    let server = mention::server_name_for(&repo, &into_slug, &mcpsetup::claude_json_path())?;
    let token = mention::mention(&server, &uri);
    // Spaces on BOTH sides. The client's extractor requires whitespace (or
    // start-of-input) before the `@`, and this cannot see the prompt to know
    // whether there already is any; the trailing one closes the `\b` and
    // dismisses the completion popup the `@` opens as it arrives.
    //
    // Addressed by the AI's pane, not by an index — see `tmux::ai_pane` for the
    // three ordinary ways pane 0 turns out not to be Claude.
    tmux::paste_to_ai(&into_session, "claude", &format!(" {token} "))?;
    applog("info", &format!("drop_reference: {token} -> {into_session}"));
    Ok(token)
}

/// Rename a place's LABEL. Empty clears it, which is how the UI goes back to
/// showing the slug — see `Declared::title` for why this is a label and not a
/// rename of the worktree.
#[tauri::command]
async fn set_title(repo: String, slug: String, title: String) -> Result<(), String> {
    store::edit(&repo, &slug, |d| {
        let t = title.trim();
        d.title = if t.is_empty() { None } else { Some(t.to_string()) }
    })
}

/// Stamp last-opened (drives the `idle` window). Called when a place is opened.
#[tauri::command]
async fn touch_place(repo: String, slug: String) -> Result<(), String> {
    store::edit(&repo, &slug, |d| d.last_opened_epoch = Some(sysclock::now_epoch()))
}

/// Stamp "the user has SEEN this place's afterglow", forward-only.
///
/// The epoch comes from the FRONTEND, not from `now_epoch()`: the frontend is
/// what knows the moment the dwell expired, and it has already written the same
/// number into its optimistic map. Two clocks for one fact is how an ack and the
/// dot it is acking end up disagreeing by a second.
///
/// Reads before it writes, like `stamp_worked`: selecting a place is a gesture
/// the user makes all day, and an unconditional `store::edit` would rewrite
/// `.worktrees.places.json` on every one of them. A stale-or-equal epoch is a
/// no-op, NOT an error — the caller fire-and-forgets it.
///
/// ⚠ Touches `last_seen_epoch` and nothing else. `last_worked_epoch` is the
/// fact being acknowledged and `last_opened_epoch` means "I entered here";
/// moving either from this path would erase the signal or lie to the Recent lens.
#[tauri::command]
async fn mark_seen(repo: String, slug: String, epoch: i64) -> Result<(), String> {
    if store::read_lenient(&repo)
        .places
        .get(&slug)
        .and_then(|d| d.last_seen_epoch)
        .unwrap_or(0)
        >= epoch
    {
        return Ok(()); // already seen at or after this moment
    }
    store::edit(&repo, &slug, |d| {
        if d.last_seen_epoch.unwrap_or(0) < epoch {
            d.last_seen_epoch = Some(epoch);
        }
    })
}

// ── mutating ops via core (create/switch/rm from the UI) ─────────────────────

/// Outcome of a core op: exit code + the op's own messages (the loud guards),
/// surfaced to the UI verbatim.
#[derive(Serialize)]
struct CmdResult {
    ok: bool,
    code: i32,
    output: String,
    /// For `new_place` only: the ACTUAL slug core landed the place on (branch
    /// slugified, origin/ stripped, holder-reuse applied). The frontend selects
    /// this instead of re-deriving and guessing wrong. `None` for every other op.
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<String>,
    /// For `close_place` only: core stopped short (`diag::EXIT_NEEDS_CONFIRM`)
    /// because killing this place's session needs the user's word — it was
    /// ADOPTED, so `kill-session` takes a whole session the user did not start
    /// through this tool. The value is the session that would die, passed
    /// STRUCTURALLY so the frontend names it in its own confirm without parsing
    /// core's prose. `None` for every other op and outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    needs_confirm: Option<String>,
    /// Messages the op emitted at warn/error severity. `output` flattens every
    /// severity into one blob and the UI only shows it when the op FAILED, so a
    /// warning on a successful op used to vanish — which is the failure mode
    /// `doctor` exists to end (proposal §7).
    warnings: Vec<String>,
}

fn run_op<F: FnOnce(&Project, &mut CaptureUi) -> i32>(op: &str, repo: &str, f: F) -> Result<CmdResult, String> {
    let project = Project::discover(Path::new(repo)).map_err(|e| {
        applog("error", &format!("{op} repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    // Guard A for the app. The CLI refuses every mutating command inside a tree
    // that arrived on a sync hub (main.rs's one dispatch choke point); the app
    // calls the same `ops::cmd_*` IN-PROCESS and so never passes it. This is the
    // app's equivalent choke point — every op-shaped command goes through here.
    //
    // Reported as a FAILED CmdResult rather than an Err: the frontend renders op
    // failures from `output`, and an Err would surface as a thrown invoke with no
    // banner of its own.
    if let Some(msg) = sync::hub_copy_refusal(Path::new(&project.main_root)) {
        applog("warn", &format!("{op} refused repo={repo}: {msg}"));
        return Ok(CmdResult {
            ok: false,
            code: 1,
            output: msg,
            slug: None,
            needs_confirm: None,
            warnings: Vec::new(),
        });
    }
    let mut ui = CaptureUi::default();
    let code = f(&project, &mut ui);
    let warnings = ui.warnings();
    if code == 0 {
        applog("info", &format!("{op} ok repo={repo}"));
        // A SUCCESSFUL op can still warn (a declared file absent from main, a
        // drifted copy). Log it separately — otherwise the only record of it is
        // an `output` string the UI never renders on success.
        if !warnings.is_empty() {
            applog("warn", &format!("{op} warnings repo={repo}: {}", warnings.join(" | ")));
        }
    } else {
        applog("warn", &format!("{op} rc={code} repo={repo}: {}", ui.lines.join(" | ")));
    }
    Ok(CmdResult { ok: code == 0, code, output: ui.lines.join("\n"), slug: None, needs_confirm: None, warnings })
}

/// Create a worktree (`new`). `--no-attach`: the session is created (pane 0 AI,
/// pane 1 shell) but the app embeds it via its own PTY rather than attaching.
#[tauri::command]
async fn new_place(
    repo: String,
    branch: String,
    base: Option<String>,
    name: Option<String>,
    provider: Option<String>,
) -> Result<CmdResult, String> {
    let provider = provider.unwrap_or_else(|| "claude".into());
    if provider != "claude" && provider != "codex" { return Err("provider must be claude or codex".into()); }
    if provider == "codex" && worktrees_core::profile::codex_bin().is_none() {
        return Err("Codex CLI is not installed. Install it, then sign in with `codex login`.".into());
    }
    let branch_log = branch.clone();
    let name = name.filter(|s| !s.is_empty());
    // Resolve the FINAL slug BEFORE the op — wt_for_branch reflects the holder
    // that reuse targets, and the derived-slug dir doesn't exist yet either way.
    // After the op the holder may have moved (branch now in the new dir), so the
    // pre-op resolution is what the frontend should select.
    let resolved_slug = Project::discover(Path::new(&repo))
        .ok()
        .map(|p| p.resolve_new_slug(&branch, name.as_deref()));
    let mut args: Vec<String> = vec![branch];
    if let Some(b) = base.filter(|s| !s.is_empty()) {
        args.push(b);
    }
    if let Some(n) = name {
        args.push("--name".into());
        args.push(n);
    }
    args.push("--ai".into());
    args.push(provider);
    args.push("--no-attach".into());
    // Single-pane like `open_place` below: Claude gets the full width and the
    // scratch shell lives in the dock's Terminal tab (which is also where deps
    // get installed — `--no-spare` suppresses the auto-install pane).
    args.push("--no-spare".into());
    let mut r = run_op(&format!("new {branch_log}"), &repo, |p, ui| ops::cmd_new(p, ui, &args))?;
    if r.ok {
        r.slug = resolved_slug;
    }
    Ok(r)
}

/// Move a place to another branch (`switch <slug> <branch> [base]`). `-y` skips
/// the inside-a-worktree ambiguity prompt (the UI targets a place explicitly).
#[tauri::command]
async fn switch_place(
    repo: String,
    slug: String,
    branch: String,
    base: Option<String>,
) -> Result<CmdResult, String> {
    let slug_log = slug.clone();
    let mut args: Vec<String> = vec![slug, branch];
    if let Some(b) = base.filter(|s| !s.is_empty()) {
        args.push(b);
    }
    args.push("-y".into());
    run_op(&format!("switch {slug_log}"), &repo, |p, ui| ops::cmd_switch(p, ui, &args))
}

/// Branches the status-bar switcher offers, plus the base a NEW branch would be
/// cut from. `switch` is DWIM (local → switch, remote-only → track, otherwise
/// create), so the control is a combobox and not a `<select>`: picking from the
/// list and typing a name that doesn't exist yet are both first-class.
#[derive(Serialize)]
struct BranchList {
    branches: Vec<String>,
    current: String,
    default_base: String,
}

#[tauri::command]
async fn list_branches(repo: String, slug: String) -> Result<BranchList, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| e.msg)?;
    let current = p.wt_branch(&p.place_dir(&slug));
    Ok(BranchList { branches: p.branch_names(), current, default_base: p.default_base() })
}

/// Enter a place: ensure its tmux session exists (create if down) WITHOUT attaching
/// — the app embeds it via its own PTY. Worktrees go through `open` (reuses the
/// existing launch path); the main checkout is launched directly since `open` only
/// targets worktrees under `.worktrees/`.
#[tauri::command]
async fn open_place(repo: String, slug: String, fresh: Option<bool>, provider: Option<String>) -> Result<CmdResult, String> {
    let provider = provider.unwrap_or_else(|| "claude".into());
    if provider != "claude" && provider != "codex" { return Err("provider must be claude or codex".into()); }
    if provider == "codex" && worktrees_core::profile::codex_bin().is_none() {
        return Err("Codex CLI is not installed. Install it, then sign in with `codex login`.".into());
    }
    run_op(&format!("open {slug} fresh={}", fresh.unwrap_or(false)), &repo, move |p, ui| {
        // Auto-resume: if this place already has a Claude Code conversation on
        // disk, launch the AI pane with the resume arg (-r) instead of cold.
        // `fresh` (right-click "Open fresh") skips it. Gated on the configured
        // AI actually being Claude — appending -r to an arbitrary ai_cmd breaks it.
        let wt = p.place_dir(&slug);
        let resume = !fresh.unwrap_or(false) && match provider.as_str() {
            "claude" => p.claude_session_present(&wt),
            "codex" => worktrees_core::codex::session_present(&wt),
            _ => false,
        };
        if slug == "(main)" {
            if !worktrees_core::tmux::have_tmux() {
                ui.error("tmux not found");
                return 1;
            }
            let session = ops::agent_session_name(&p.session_name("(main)"), &provider);
            let mut ai_cmd = provider.clone();
            if resume && !ai_cmd.is_empty() {
                ai_cmd = ops::resume_command(&ai_cmd);
            }
            // Propagate launch's rc: a failed new-session must reach the UI
            // banner / app.log, not silently report success. Single-pane
            // (spare_shell=false): Claude gets full width; the scratch shell
            // lives in the dock's Terminal tab.
            let ai = ops::ai_launch_for(p, ui, &p.main_root, &ai_cmd);
            ops::launch(p, ui, &p.main_root, &session, "", &ai, false, false)
        } else {
            let mut args = vec![slug, "--no-attach".into(), "--no-spare".into()];
            args.push("--ai".into());
            args.push(provider.clone());
            if resume {
                args.push("-r".into());
            }
            ops::cmd_open(p, ui, &args)
        }
    })
}

/// End a place's tmux session — the worktree stays (right-click "Close session").
///
/// `yes` is the user's word, collected by the frontend's two-click arm. Without
/// it core refuses to kill an ADOPTED session (one found by pane cwd, not by
/// this tool's name — a whole tmux session someone started by hand, or one left
/// under a previous prefix) and returns `EXIT_NEEDS_CONFIRM`. Passing `-y`
/// unconditionally, as `remove_place` does, would restore exactly the
/// promptless adopted-kill that confirmation exists to prevent.
///
/// `session` is the name the arm DISPLAYED — the frontend sends back the one it
/// asked about, and core kills only that. The arm is armed at one moment and
/// clicked at another; without the name, `yes` would authorize whatever a fresh
/// resolution finds at click time, which can be a different session entirely.
#[tauri::command]
async fn close_place(
    repo: String,
    slug: String,
    yes: bool,
    session: Option<String>,
    provider: Option<String>,
    shells: State<'_, Shells>,
) -> Result<CmdResult, String> {
    if provider.as_deref().is_some_and(|p| p != "claude" && p != "codex") {
        return Err("provider must be claude or codex".into());
    }
    // core cmd_close sweeps this place's tmux-era sidecars; the owned dock
    // shells are app state, so they're swept here — same rule as before, the
    // dock's scratch shells die with the place.
    let slug_log = slug.clone();
    let mut args = vec![slug.clone()];
    if let Some(p) = &provider { args.push("--ai".into()); args.push(p.clone()); }
    if yes {
        args.push("-y".into());
    }
    let expect = session.filter(|s| !s.is_empty());
    // The consented session goes in the log line too: when a kill is questioned
    // later, the record must say WHICH session the user was asked about.
    let op = match &expect {
        Some(s) => format!("close {slug_log} yes={yes} session={s}"),
        None => format!("close {slug_log} yes={yes}"),
    };
    if let Some(s) = expect {
        args.push("--session".into());
        args.push(s);
    }
    let mut r = run_op(&op, &repo, |p, ui| ops::cmd_close(p, ui, &args))?;
    if r.code == worktrees_core::diag::EXIT_NEEDS_CONFIRM {
        // Not a failure — a question. Resolve the session core balked at the
        // same way core did, so the UI can name it verbatim. This is a SECOND
        // resolution and can differ from core's (the session may have exited in
        // between) — harmless now that the answer comes back name-bound: the
        // name shown is the name core is held to, and `None` means there is
        // nothing left to ask about (the frontend treats it as "already gone").
        r.needs_confirm = Project::discover(Path::new(&repo)).ok().and_then(|p| ops::place_session(&p, &slug));
    } else if r.ok && provider.is_none() {
        kill_place_shells(&shells, &repo, &slug);
    }
    Ok(r)
}

// ── update check (Settings → Version) ────────────────────────────────────────

const REPO_SLUG: &str = "penard-monkey/worktrees";

#[derive(Serialize)]
struct UpdateInfo {
    app_version: String,
    cli_version: Option<String>,
    cli_path: Option<String>,
    latest: Option<String>, // release tag, e.g. "v0.2.0"
}

/// Latest release tag via the releases/latest REDIRECT (install.sh's trick —
/// no API, no rate limit, no auth). None when offline or no releases exist.
fn latest_release_tag() -> Option<String> {
    let out = std::process::Command::new("curl")
        .args([
            "-fsSLI", "--max-time", "6", "-o", "/dev/null", "-w", "%{url_effective}",
            &format!("https://github.com/{REPO_SLUG}/releases/latest"),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout);
    let url = url.trim();
    if !url.contains("/tag/") {
        return None;
    }
    url.rsplit("/tag/").next().map(String::from)
}

/// The hard-deadline subprocess runner, now core's (`proc.rs`).
///
/// It moved when the automation runner needed the identical thing: a headless
/// `claude -p` with no pane to Ctrl-C and nobody watching. Imported rather than
/// re-exported so every call site below reads exactly as it did.
use worktrees_core::proc::run_deadline;

// ── origin fetch: shared machinery for the watcher + the manual verb ─────────
// The app's FIRST background network actor, so every call is hardened the same
// way regardless of who triggers it.

/// Fetch a single project's origin. Runs `git fetch --prune origin` in the MAIN
/// ROOT only — every worktree shares the repo's ref store, so one fetch keeps the
/// whole project's ahead/behind fresh. Hardening: `GIT_TERMINAL_PROMPT=0` +
/// `GIT_SSH_COMMAND=ssh -oBatchMode=yes` so a credential/host-key prompt can
/// never hang a thread forever (the GUI has no tty under launchd), and a ~60s
/// deadline (run_deadline) so a wedged network can't pin the thread either.
/// Result → app.log only (info ok / warn + git's stderr) — background work never
/// pops a user-facing banner. `Err` carries git's stderr for the manual command.
fn fetch_origin_root(main_root: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C")
        .arg(main_root)
        .args(["fetch", "--prune", "origin"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
    match run_deadline(cmd, 60) {
        Ok(out) if out.status.success() => {
            applog("info", &format!("fetch ok repo={main_root}"));
            Ok(())
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            applog("warn", &format!("fetch rc={} repo={main_root}: {stderr}", out.status.code().unwrap_or(-1)));
            Err(if stderr.is_empty() { format!("git fetch exited {}", out.status.code().unwrap_or(-1)) } else { stderr })
        }
        Err(e) => {
            applog("warn", &format!("fetch failed repo={main_root}: {e}"));
            Err(e.to_string())
        }
    }
}

/// Auto-fetch interval in SECONDS (0 = off). The settings blob is backend-opaque
/// (get_settings/set_settings just shuttle JSON), so the watcher thread can't read
/// it — the frontend pushes the value here via `set_fetch_interval` after every
/// settings hydration and change.
static FETCH_INTERVAL_SECS: AtomicU64 = AtomicU64::new(0);

/// Frontend → backend sync of the auto-fetch interval. `mins` is 0 (off) or one
/// of the offered cadences (5 / 15 / 60); stored in whole seconds for the watcher.
#[tauri::command]
async fn set_fetch_interval(mins: u64) -> Result<(), String> {
    FETCH_INTERVAL_SECS.store(mins.saturating_mul(60), Ordering::Relaxed);
    Ok(())
}

/// Manual "Fetch origin" verb (project right-click). Discovers the repo from any
/// path under it, then fetches its main root via the shared fn. The error string
/// is git's own stderr so the frontend's fail() shows the real reason.
#[tauri::command]
async fn fetch_origin(root: String) -> Result<(), String> {
    let project = Project::discover(Path::new(&root)).map_err(|e| e.msg)?;
    fetch_origin_root(&project.main_root)
}

// ── Claude working state (nav busy/waiting dots) ─────────────────────────────
// The probe reader lives in core (`worktrees_core::agent`) so the MCP server's
// `place_status` and these dots read ONE truth. This is the app's projection of
// it: which worktree PATHS have a busy / waiting claude right now.

/// Scan the live probes and bucket their `cwd`s by state. Returns
/// (busy_cwds, waiting_cwds). Paths are pushed as-is (the probe cwd and a
/// place's `path` both derive from the same worktree dir, so the frontend
/// matches raw). Any I/O or parse failure degrades to empty.
fn claude_activity() -> (Vec<String>, Vec<String>) {
    let mut busy = Vec::new();
    let mut waiting = Vec::new();
    for probe in worktrees_core::agent::live_probes() {
        match worktrees_core::agent::effective_state(&probe).as_str() {
            "busy" => busy.push(probe.cwd),
            "waiting" => waiting.push(probe.cwd),
            _ => {} // idle / shell / parked-away busy (`delegated`) → no dot
        }
    }
    (busy, waiting)
}

/// Payload for `sessions:busy` — PATHS (worktree dirs), keyed to a place's `path`.
#[derive(Serialize, Clone)]
struct ClaudeActivity {
    busy: Vec<String>,
    waiting: Vec<String>,
}

// ── the unsent prompt (nav ✎, ⌘K, Home) ─────────────────────────────────────
// A place can be waiting on YOU with nothing in the probe file to say so: text
// typed at claude's prompt and never sent leaves `status: "idle"`, exactly like
// an empty pane. The screen is the only witness, so this samples it.
//
// COST, deliberately bounded: ONE extra `tmux` spawn per 15s, whatever the
// session count — every pane is captured in a single invocation by chaining
// commands with `;` as its own argv element, and the session list is reused
// from the fingerprint the tick loop already fetched. `%N` pane ids are
// server-global, so `-t %N` addresses a pane without naming its session.

/// How many rows above the cursor to capture. The prompt box is the bottom of
/// the pane and the parser needs the box's TOP edge in frame (that edge is what
/// tells an input box from a `❯` selector menu), so this has to clear the
/// longest wrapped draft anyone types. `-S -20` starts 20 rows INTO the
/// scrollback and runs to the bottom of the visible pane, so what comes back is
/// the whole screen plus a little history — a few KB per pane. Too few costs a
/// draft, never a wrong one.
const DRAFT_CAPTURE_LINES: &str = "-20";

/// Payload row for `sessions:drafts` / `list_drafts`.
#[derive(Serialize, Clone, PartialEq, Debug)]
struct Draft {
    /// The session's cwd — the same key as `sessions:busy` and a place's `path`.
    path: String,
    text: String,
    /// The session is mid-turn, so claude will SEND this when the turn ends
    /// rather than sit on it. A different sentence in the UI, not a different
    /// signal.
    queued: bool,
}

/// Payload for `sessions:drafts` — the CURRENT set, reconciled every sample.
#[derive(Serialize, Clone)]
struct Drafts {
    drafts: Vec<Draft>,
}

/// Capture every live claude pane in one `tmux` call and parse the unsent text
/// out of each. `sessions` is `tmux::session_fingerprint()`'s output (sorted
/// names, one per line) — passed in rather than re-fetched so the pass costs a
/// single spawn.
///
/// Panes whose session is not in that list are dropped BEFORE the call: a
/// `capture-pane` on a dead pane makes tmux print an error and abandon the rest
/// of the chain, which would silently blank every pane after it.
fn scan_drafts(sessions: &str) -> Result<Vec<Draft>, String> {
    let live: Vec<&str> = sessions.lines().filter(|l| !l.is_empty()).collect();
    let probes = worktrees_core::agent::live_probes();
    // (cwd, pane id, queued) for the panes worth capturing.
    let mut targets: Vec<(String, String, bool)> = Vec::new();
    for p in &probes {
        let state = worktrees_core::agent::effective_state(p);
        // `shell` is not a claude prompt at all; `delegated` is a session whose
        // turn is somewhere else, and whose box is not the user's to answer.
        if state == "shell" || state == "delegated" {
            continue;
        }
        let Some(tmux) = p.tmux.as_deref() else { continue }; // started outside tmux → no screen
        let (Some(pane), Some(sess)) = (
            worktrees_core::agent::pane_id(tmux),
            worktrees_core::agent::session_name(tmux),
        ) else {
            continue;
        };
        if !live.contains(&sess) {
            continue;
        }
        targets.push((p.cwd.clone(), pane.to_string(), state == "busy"));
    }
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    // `display-message -p` expands `#{…}` AND eats `%` in its format, so the
    // marker is built from the target's index and nothing else.
    let marks: Vec<String> = (0..targets.len()).map(|i| format!("@@ {i} @@")).collect();
    let mut args: Vec<&str> = Vec::with_capacity(targets.len() * 10);
    for (i, (_, pane, _)) in targets.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        // Marker FIRST: a chain that dies part-way then leaves the surviving
        // segments still correctly attributed to their panes.
        args.extend(["display-message", "-p", &marks[i], ";"]);
        // `-e` keeps the styling: claude paints its own suggested follow-up
        // into the box in DIM, and the text alone cannot tell it from typing
        // (`agent::draft_from_screen` reads the attribute and strips the rest).
        args.extend(["capture-pane", "-e", "-p", "-t", pane, "-S", DRAFT_CAPTURE_LINES]);
    }
    let out = worktrees_core::tmux::tmux(&args).map_err(|e| format!("tmux: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "tmux capture-pane failed".into() } else { err });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut drafts: Vec<Draft> = Vec::new();
    for (i, (path, _, queued)) in targets.iter().enumerate() {
        let Some(rest) = text.split(&format!("{}\n", marks[i])).nth(1) else { continue };
        // The next marker ends this pane's screen (the last one runs to EOF).
        let screen = match marks.get(i + 1).and_then(|m| rest.find(m.as_str())) {
            Some(end) => &rest[..end],
            None => rest,
        };
        if let Some(t) = worktrees_core::agent::draft_from_screen(screen) {
            drafts.push(Draft { path: path.clone(), text: t, queued: *queued });
        }
    }
    // Two claude sessions can share a cwd (a fork, or a second one started by
    // hand). The row has one ✎, so keep one draft per path — the busy one
    // first, since "queued" is the more surprising of the two sentences.
    drafts.sort_by(|a, b| a.path.cmp(&b.path).then(b.queued.cmp(&a.queued)));
    drafts.dedup_by(|a, b| a.path == b.path);
    Ok(drafts)
}

/// Mount-time pull for `sessions:drafts`. The event is change-gated and only
/// fires every 15s, so a frontend that mounts (or reloads) between two changes
/// would otherwise show nothing until the user typed something.
#[tauri::command]
async fn list_drafts() -> Result<Vec<Draft>, String> {
    let fp = worktrees_core::tmux::session_fingerprint();
    Ok(scan_drafts(&fp).unwrap_or_default())
}

/// Draft TEXT is something a person typed, so it never reaches the log — same
/// rule the usage metrics live under. `applog` records counts only.
///
/// `WORKTREES_TRACE_DRAFTS=<path substring>` is the one exception, for proving
/// the pipeline end to end against a scratch place. It is a FILTER rather than
/// an on/off switch on purpose: `app.log` is keyed to `APP_IDENT`, which the
/// dev/sandbox build shares with the installed app, and a session that scans
/// every config root would otherwise write the user's real drafts into the
/// file they paste into bug reports.
fn draft_trace_filter() -> Option<String> {
    std::env::var("WORKTREES_TRACE_DRAFTS").ok().filter(|v| !v.is_empty())
}

/// `WORKTREES_TRACE_SPAWNS=1` logs what the draft pass costs in subprocesses —
/// the app cannot be measured with the CLI's PATH shim (`fixup_gui_path` puts
/// the real tmux back in front), so the counter has to live here.
fn trace_spawns() -> bool {
    std::env::var("WORKTREES_TRACE_SPAWNS").is_ok_and(|v| v == "1")
}

// ── task-completed stamps (the nav's decaying "afterglow" dot) ───────────────
// A place stops being busy → Claude finished a task there. That instant is worth
// keeping: the green dot vanishing is currently the end of all visibility, and
// "which places did work recently" has no answer at all once it goes.
//
// Why the busy-EXIT edge and not "a session exists": opening or resuming a
// session leaves its probe at `status: "idle"`, so this signal already excludes
// mere presence — the constraint the whole feature hangs on. The dwell guard
// below is the only other filter needed.
//
// The stamp is a monotonic FACT, so it gets its own event (`sessions:done`)
// rather than riding on `sessions:busy`, which is a live SET that reconciles to
// current truth every tick. Merging the two would make a completion something
// the next tick could retract.

/// Consecutive 3s ticks a place must be observed busy before its exit counts as
/// a finished task. Two ticks (~3–6s) discards startup/resume blips; no real
/// prompt turns around that fast.
const DONE_DWELL_TICKS: u32 = 2;

/// Payload for `sessions:done` — one place finished a task at `epoch`.
#[derive(Serialize, Clone)]
struct TaskDone {
    path: String,
    epoch: i64,
}

/// Advance the dwell counters by one tick and return the paths that just
/// FINISHED: busy for at least `DONE_DWELL_TICKS` consecutive observations, and
/// no longer busy now. Pure, so the ordering (measure exits, then drop, then
/// count) is testable without a Claude session — it is the one piece here with
/// genuinely tricky sequencing, and there is no fake `claude` to drive it.
///
/// `busy_now` must be de-duplicated; a repeated path would double-count.
///
/// busy→waiting is an exit too: the work finished, and amber merely out-ranks
/// the ember in the dot until the question is answered. (busy→dead-pid is also
/// an exit — a killed session is indistinguishable from a finished one here,
/// which the busy-edge design accepts.)
///
/// Ticks are only as regular as the loop: the inline auto-fetch pass can stall
/// it for up to ~60s per repo, so a task that starts and ends inside a stall is
/// missed entirely. A miss, never a false stamp — and the next cold start's
/// backfill picks it up from history.
fn completion_edges(busy_ticks: &mut HashMap<String, u32>, busy_now: &[String]) -> Vec<String> {
    let exits: Vec<String> = busy_ticks
        .iter()
        .filter(|(p, t)| **t >= DONE_DWELL_TICKS && !busy_now.contains(p))
        .map(|(p, _)| p.clone())
        .collect();
    busy_ticks.retain(|p, _| busy_now.contains(p)); // sub-dwell blips drop unstamped
    for p in busy_now {
        *busy_ticks.entry(p.clone()).or_insert(0) += 1;
    }
    exits
}

/// Resolve a session cwd → (repo root, store slug), or `None` when the path is
/// not inside a TRACKED project. The guard matters: `claude_activity` reports
/// every live session on the machine, and without it a Claude run in some
/// unrelated clone would drop a `.worktrees.places.json` into that repo.
///
/// The slug comes from the session's WORKTREE TOP, never from the cwd itself.
/// A session's cwd is wherever the user happened to be — `<place>/app` is
/// entirely normal — and slugging that basename would both invent a phantom
/// store entry ("app") and miss the place that actually did the work. Store
/// keys are `basename(worktree_dir)`, or `(main)` for the main root
/// (`project::place_json`), so the cwd has to be resolved back to that dir
/// first. `--show-toplevel` answers exactly that, including for linked
/// worktrees, and it normalizes a symlinked spelling on the way.
fn place_key_for(path: &str, roots: &[String]) -> Option<(String, String)> {
    let project = Project::discover(Path::new(path)).ok()?;
    let root = project.main_root;
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    let root_c = canon(&root);
    if !roots.iter().any(|r| canon(r) == root_c) {
        return None; // untracked repo — not ours to write in
    }
    let top = git::git_out(path, &["rev-parse", "--show-toplevel"])?;
    let top_c = canon(top.trim());
    if top_c == root_c {
        return Some((root, "(main)".to_string()));
    }
    let slug = top_c.file_name()?.to_string_lossy().into_owned();
    Some((root, slug))
}

/// What the frontend is told when a Claude session asks for a document.
///
/// `repo`/`slug` and not just the path: the pane that renders a document is
/// mounted under a SELECTED place, so the request has to name the place before
/// it can name the file.
#[derive(Clone, Serialize)]
struct OpenDoc {
    repo: String,
    slug: String,
    path: String,
}

/// Serve whatever a session has dropped in `~/.cache/worktrees/inbox`.
///
/// Runs on the 3 s watcher tick. The directory is empty essentially always, so
/// the steady-state cost is one `read_dir` of an empty directory — cheaper than
/// the tmux fingerprint the same tick already pays for.
///
/// Every request is re-validated here even though the writer canonicalised it.
/// It arrived from another process; `guard_under_projects` is the same check
/// every file-reading command makes, and this is the one entry point where the
/// asker is not the user's own click.
fn drain_inbox(app: &AppHandle) {
    let reqs = worktrees_core::inbox::drain(sysclock::now_epoch());
    if reqs.is_empty() {
        return;
    }
    let roots = read_projects(app);
    for r in reqs {
        // Not under a registered project → refused, and SAID so. A silent drop
        // here is a request that vanished with no way to tell whether the app
        // saw it (CLAUDE.md: never swallow errors).
        if guard_under_projects(app, &r.path).is_err() {
            applog("warn", &format!("inbox: refused {} (pid {}) — outside every registered project", r.path, r.pid));
            continue;
        }
        // `place_key_for` runs git in the path it is given, so it needs the
        // DIRECTORY — `git -C <a file>` is not a thing.
        let Some(parent) = Path::new(&r.path).parent().map(|p| p.to_string_lossy().to_string()) else {
            continue;
        };
        let Some((repo, slug)) = place_key_for(&parent, &roots) else {
            applog("warn", &format!("inbox: {} is in no tracked place", r.path));
            continue;
        };
        applog("info", &format!("inbox: open {} in {repo}:{slug} (pid {})", r.path, r.pid));
        let _ = app.emit("app:open-doc", OpenDoc { repo, slug, path: r.path });
        // Front and centre — "let me see X" means show it to me NOW. A window
        // that loads the document behind the terminal the request was typed in
        // is a feature you have to go looking for.
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            if let Err(e) = w.set_focus() {
                applog("warn", &format!("inbox: could not focus the window: {e}"));
            }
        }
    }
}

/// Stamp `last_worked_epoch` forward-only. Returns true when the store actually
/// moved, so callers only emit an event for a real change. Never fatal: an
/// untracked path is silent (expected), a write failure is logged.
fn stamp_worked(roots: &[String], path: &str, epoch: i64) -> bool {
    let Some((repo, slug)) = place_key_for(path, roots) else {
        return false;
    };
    if store::read_lenient(&repo)
        .places
        .get(&slug)
        .and_then(|d| d.last_worked_epoch)
        .unwrap_or(0)
        >= epoch
    {
        return false; // already know about newer work here
    }
    match store::edit(&repo, &slug, |d| {
        if d.last_worked_epoch.unwrap_or(0) < epoch {
            d.last_worked_epoch = Some(epoch);
        }
    }) {
        Ok(()) => true,
        Err(e) => {
            applog("warn", &format!("worked stamp {path}: {e}"));
            false
        }
    }
}

/// How far back the startup backfill looks. Matches the UI's afterglow horizon —
/// anything older renders as an empty dot slot, so reading it would be waste.
const BACKFILL_WINDOW_SECS: i64 = 12 * 3600;
/// Tail of `history.jsonl` to read. The file is append-only and grows without
/// bound (MBs), but 12h of prompts is a few KB; this is slack for pasted blobs.
const HISTORY_TAIL_BYTES: u64 = 512 * 1024;
/// Tail of a session TRANSCRIPT to read when dating its last turn. Transcripts
/// reach tens of MB (one line per message, tool results included), and only the
/// newest timestamp is wanted — but a single line can be a pasted file or a
/// whole tool result, so this is slack for a few of those rather than a line
/// count. A tail that lands inside one enormous line yields no whole line at
/// all, which `transcript_epoch` reports as "no answer" rather than a wrong one.
const TRANSCRIPT_TAIL_BYTES: u64 = 256 * 1024;
/// Slash commands that are session HOUSEKEEPING, not a task. They land in
/// history.jsonl exactly like a prompt, and without this a `/clear` ten minutes
/// ago would light a place where nothing was done. Unknown slash commands are
/// deliberately NOT filtered — a user's own `/close-out` or `/commit` is work.
const NON_WORK_SLASH: &[&str] = &[
    "/clear", "/exit", "/quit", "/help", "/status", "/config", "/model", "/login", "/logout",
    "/cost", "/usage", "/resume", "/compact", "/doctor", "/context", "/permissions", "/mcp",
    "/memory", "/hooks", "/agents", "/add-dir", "/export", "/todos", "/ide", "/statusline",
    "/bug", "/vim", "/terminal-setup", "/release-notes",
];

#[derive(serde::Deserialize)]
struct HistLine {
    display: Option<String>,
    /// Claude Code has written this as both a JSON number and a quoted string
    /// across versions; parsed leniently below rather than trusted as one shape.
    #[serde(default)]
    timestamp: serde_json::Value,
    project: Option<String>,
    /// Names THIS prompt's transcript file, which is how the completion time is
    /// refined without letting unrelated activity in the same place count.
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

fn hist_epoch(v: &serde_json::Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        .map(|ms| ms / 1000)
}

/// When a transcript entry was written. Transcript lines carry ISO-8601
/// (`2026-09-20T01:49:52.243Z`); `history.jsonl`'s epoch-millis shape is
/// accepted too, tried second so a date string is never mistaken for a number.
fn entry_epoch(v: &serde_json::Value) -> Option<i64> {
    v.as_str().and_then(parse_iso8601).or_else(|| hist_epoch(v))
}

/// When a session transcript's newest entry was written, per the timestamps the
/// FILE carries. `None` when the file is missing, unreadable, or its tail holds
/// no whole line with a timestamp.
///
/// ⚠ Deliberately not the file's mtime, which this used to read and which is
/// not a fact about the work. Claude Code keeps rewriting a live session's
/// `.jsonl` long after its last turn: measured across every transcript touched
/// in a day, 24 of 28 had an mtime running from 12 minutes to 34 HOURS ahead of
/// the last entry inside the file. Since the stamp is forward-only and the
/// backfill runs on every launch, reading the mtime re-dated every place with a
/// still-open session to "just finished" at each restart — which the unread
/// rule then renders as a place shouting for attention it has already had, and
/// which the nav reads as the row's age and sort key.
///
/// The max over the tail rather than the last line with a timestamp: a
/// transcript is not strictly ordered (a rewritten summary lands at the end
/// carrying an older stamp), and the caller bounds the answer by `now` anyway.
fn transcript_epoch(path: &Path) -> Option<i64> {
    tail_lines(path, TRANSCRIPT_TAIL_BYTES)
        .iter()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("timestamp").and_then(entry_epoch))
        .max()
}

fn is_work_prompt(display: &str) -> bool {
    let t = display.trim();
    if t.is_empty() {
        return false;
    }
    if !t.starts_with('/') {
        return true;
    }
    let head = t.split_whitespace().next().unwrap_or(t);
    !NON_WORK_SLASH.contains(&head)
}

/// Read the tail of a file as whole lines (the first, possibly-truncated line is
/// dropped). Returns empty on any I/O failure — a backfill is a nicety.
///
/// Decoded LOSSILY on purpose. Seeking to a byte offset lands mid-character
/// whenever the boundary falls inside a multi-byte char — routine in a file full
/// of pasted prompts — and a strict decode would throw away the whole tail, not
/// just the fragment that is discarded anyway. Worse, the boundary only moves as
/// the file grows, so a strict failure would be silent AND sticky.
fn tail_lines(path: &Path, max_bytes: u64) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let partial = len > max_bytes;
    if partial && f.seek(std::io::SeekFrom::Start(len - max_bytes)).is_err() {
        applog("warn", &format!("tail seek failed: {}", path.display()));
        return Vec::new();
    }
    let mut buf = Vec::new();
    if let Err(e) = f.read_to_end(&mut buf) {
        applog("warn", &format!("tail read failed ({e}): {}", path.display()));
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines().map(|s| s.to_string()).collect::<Vec<_>>();
    if partial && !lines.is_empty() {
        lines.remove(0);
    }
    lines
}

/// Backfill afterglow from `history.jsonl` at startup, so a machine where the
/// app was closed all night still shows what was worked on. Live observation
/// (the poll thread) can only see completions while the app runs; this is the
/// half that survives a cold start.
///
/// Prompt time is when work STARTED, so where a transcript exists the newest
/// timestamp INSIDE it is taken as the better completion time — bounded by now,
/// and only for places a qualifying prompt already vouched for. Dating by a
/// transcript with no prompt vouching for it would re-light every place merely
/// opened, which is exactly what must not happen — and dating by that file's
/// MTIME re-lights every place whose session is still UP, on every launch,
/// which is what this used to do (see `transcript_epoch`).
fn backfill_worked(handle: &AppHandle) {
    let roots = read_projects(handle);
    if roots.is_empty() {
        return;
    }
    let now = sysclock::now_epoch();
    let cutoff = now - BACKFILL_WINDOW_SECS;
    let mut newest: HashMap<String, i64> = HashMap::new();
    // One read per transcript, not per prompt: a busy afternoon leaves dozens of
    // history lines naming the same session file, and each read is a 256K tail.
    let mut landed: HashMap<PathBuf, Option<i64>> = HashMap::new();
    for root in worktrees_core::profile::claude_config_dirs_all() {
        for line in tail_lines(&root.join("history.jsonl"), HISTORY_TAIL_BYTES) {
            let Ok(h) = serde_json::from_str::<HistLine>(&line) else {
                continue;
            };
            let (Some(project), Some(epoch)) = (h.project, hist_epoch(&h.timestamp)) else {
                continue;
            };
            if epoch < cutoff || !is_work_prompt(h.display.as_deref().unwrap_or("")) {
                continue;
            }
            let mut stamp = epoch;
            // Refine upward to when the work actually LANDED: the prompt only
            // says when it was asked for, and a long task can finish an hour
            // later. Strictly THIS prompt's own transcript — the dir's newest
            // file would let a later `/clear` (which starts a fresh session
            // file) drag a ten-hour-old prompt up to "just finished", quietly
            // undoing the denylist above.
            if let Some(sid) = h.session_id.as_deref() {
                let cdir = worktrees_core::project::claude_dir_in(&root, &project);
                let jsonl = Path::new(&cdir).join(format!("{sid}.jsonl"));
                let m = *landed.entry(jsonl.clone()).or_insert_with(|| transcript_epoch(&jsonl));
                if let Some(m) = m {
                    if m > stamp {
                        stamp = m.min(now);
                    }
                }
            }
            let slot = newest.entry(project).or_insert(0);
            *slot = (*slot).max(stamp);
        }
    }
    let mut stamped = false;
    for (path, epoch) in newest {
        if stamp_worked(&roots, &path, epoch) {
            stamped = true;
            let _ = handle.emit("sessions:done", TaskDone { path, epoch });
        }
    }
    // The emits above almost certainly land before the webview has registered
    // its listener (this runs at thread spawn), and a dropped event has no
    // retry. One `places:changed` re-pulls the snapshot, which carries the same
    // stamps durably — otherwise the afterglow stays dark until the ~30s safety
    // re-emit, on exactly the launch where it has the most to say.
    if stamped {
        let _ = handle.emit("places:changed", ());
    }
}

// ── Claude plan usage (nav footer widget) ────────────────────────────────────
// Same bars Claude Code's /usage panel shows: the 5h session window, the weekly
// all-models window, and any model-scoped weekly bucket (e.g. "Fable").
//
// Primary source is the OAuth usage endpoint the TUI itself calls — free GET, no
// quota, but undocumented and unversioned, so EVERY step degrades instead of
// failing: the authoritative field is `limits[]` and the rest of the payload is
// full of experimental nulls we deliberately ignore. Token comes from the macOS
// Keychain via `security` (shelled out — house style, and it keeps the secret out
// of our address space longer than a lib would). Claude Code rotates the token
// ~hourly, so a 401 buys exactly one retry with a freshly re-read token.
//
// Fallback is the statusline widget's local snapshot (`~/.claude/widgets/
// rate_limits.json`, written by whatever statusline script the user runs) —
// session + weekly only, no model bucket, and only as fresh as the last Claude
// Code session, hence the file mtime as `fetched_at` and the dimmed UI.
//
// Missing data is NOT an error: `source: "unavailable"` with no limits just
// hides the widget. The reason still lands in app.log.
//
// And a failed fetch degrades before it hides — see `usage_degraded`. Hiding is
// the answer only when there is no reading left anywhere, because the widget
// disappearing reads as "something is broken in the app" when what happened is
// "one GET came back 429".

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Claude Code's own UA. Load-bearing: a foreign UA lands in a much more
/// aggressive rate-limit bucket on this endpoint.
const USAGE_UA: &str = "claude-code/2.1.220";
/// Minimum gap between real fetches. The frontend polls at 180s; this is the
/// floor that also covers window-focus pulls and a re-mount storm.
const USAGE_TTL_SECS: i64 = 120;
/// The same floor for a FAILED attempt. Without it the TTL only throttled the
/// happy path — nothing was cached on failure, so every focus pull went straight
/// back out to an endpoint that had just refused us. On 2026-09-19 that turned
/// one 429 into 15 real fetches inside a single minute (⌘-tabbing re-runs the
/// poll effect AND fires the `focus` listener), which is how a rate limit gets
/// held open by the thing complaining about it. Half the poll period, so a
/// recovered endpoint is still picked up by the next pull rather than the one
/// after it.
const USAGE_FAIL_TTL_SECS: i64 = 60;
/// How long the last good reading may stand in for a live one. The bars measure
/// 5h and 7d windows, so a reading of this age is still the right shape — but it
/// is bounded, because a percentage from before a window rolled over is not
/// stale data, it is wrong data.
///
/// The same bargain `STATUS_STALE_MAX_SECS` strikes for the service-status
/// probe, at the same 30 minutes and with the same inclusive bound — this
/// widget simply took longer to learn it. Deliberately a SECOND constant and
/// not a shared one: they answer for different data, and a future change to one
/// window must not silently move the other.
const USAGE_STALE_MAX_SECS: i64 = 1800;

#[derive(Serialize, Clone)]
struct UsageLimit {
    kind: String,     // session | weekly_all | weekly_scoped
    label: String,    // "Session" | "Weekly" | model display name ("Fable")
    percent: f64,
    severity: String, // normal | warning | … (rendered as a color tier)
    resets_at: Option<i64>, // unix seconds
}

#[derive(Serialize, Clone)]
struct UsageInfo {
    source: String, // oauth | cached | statusline | unavailable
    fetched_at: i64,
    limits: Vec<UsageLimit>,
}

/// Last SUCCESSFUL oauth answer, with the epoch it was fetched at (see TTL).
static USAGE_CACHE: Mutex<Option<UsageInfo>> = Mutex::new(None);
/// Epoch of the last FAILED real attempt — the negative half of the TTL below.
static USAGE_FAIL_AT: Mutex<Option<i64>> = Mutex::new(None);

/// ISO-8601 (`2026-08-04T00:00:00Z`, `…+00:00`, optional fraction) → unix
/// seconds. days_from_civil, the inverse of fmt_utc's civil_from_days — same
/// reason: no chrono dep for two date conversions.
fn parse_iso8601(s: &str) -> Option<i64> {
    if s.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse::<i64>().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if mo > 2 { mo - 3 } else { mo + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut epoch = days * 86_400 + h * 3600 + mi * 60 + se;
    // trailing zone: `Z`, nothing, or ±HH:MM (after an optional .fraction)
    let rest = s[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let sign = rest.chars().next().unwrap_or('Z');
    if sign == '+' || sign == '-' {
        let oh: i64 = rest.get(1..3)?.parse().ok()?;
        let om: i64 = rest.get(4..6).and_then(|m| m.parse::<i64>().ok()).unwrap_or(0);
        let off = oh * 3600 + om * 60;
        epoch += if sign == '-' { off } else { -off };
    }
    Some(epoch)
}

/// The Claude Code OAuth access token, straight out of the login Keychain item.
/// None on any failure (not macOS, item absent, locked keychain, shape changed).
fn claude_oauth_token() -> Option<String> {
    let mut cmd = std::process::Command::new("/usr/bin/security");
    cmd.args(["find-generic-password", "-s", "Claude Code-credentials", "-w"]);
    let out = run_deadline(cmd, 6).ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("claudeAiOauth")?.get("accessToken")?.as_str().map(String::from)
}

/// GET the usage endpoint → (http status, body). curl, not a HTTP crate: the app
/// already shells out to curl for the release check and this keeps the dep tree
/// (and the TLS stack) exactly where it is.
fn usage_get(token: &str) -> Result<(u16, String), String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-s", "--max-time", "10", "-w", "\n%{http_code}"])
        .arg("-H")
        .arg(format!("Authorization: Bearer {token}"))
        .arg("-H")
        .arg("anthropic-beta: oauth-2025-04-20")
        .arg("-H")
        .arg(format!("User-Agent: {USAGE_UA}"))
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg(USAGE_URL);
    let out = run_deadline(cmd, 15).map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("curl exited {}", out.status.code().unwrap_or(-1)));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    // -w appended "\n<code>" AFTER the body; the body itself may contain newlines.
    let cut = text.rfind('\n').ok_or("curl produced no status line")?;
    let code: u16 = text[cut + 1..].trim().parse().map_err(|_| "curl produced no status code".to_string())?;
    Ok((code, text[..cut].to_string()))
}

/// `limits[]` → our rows. Unknown kinds and unparseable entries are SKIPPED, not
/// fatal — the endpoint ships experimental buckets we've never seen. `None` only
/// when there is no `limits` array at all (i.e. the shape moved under us).
fn parse_usage_limits(body: &str) -> Option<Vec<UsageLimit>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let arr = v.get("limits")?.as_array()?;
    let mut out = Vec::new();
    for e in arr {
        let kind = e.get("kind").and_then(|k| k.as_str()).unwrap_or_default();
        let label = match kind {
            "session" => "Session".to_string(),
            "weekly_all" => "Weekly".to_string(),
            // the model bar (e.g. "Fable") — no name, no row
            "weekly_scoped" => match e.pointer("/scope/model/display_name").and_then(|d| d.as_str()) {
                Some(n) => n.to_string(),
                None => continue,
            },
            _ => continue,
        };
        let percent = match e.get("percent").and_then(|p| p.as_f64()) {
            Some(p) => p,
            None => continue,
        };
        out.push(UsageLimit {
            kind: kind.to_string(),
            label,
            percent,
            severity: e.get("severity").and_then(|s| s.as_str()).unwrap_or("normal").to_string(),
            resets_at: e.get("resets_at").and_then(|r| r.as_str()).and_then(parse_iso8601),
        });
    }
    Some(out)
}

fn usage_from_oauth(now: i64) -> Result<UsageInfo, String> {
    let token = claude_oauth_token().ok_or("keychain: no Claude Code credentials")?;
    let (mut code, mut body) = usage_get(&token)?;
    if code == 401 {
        // token rotated under us (Claude Code refreshes ~hourly) — re-read once
        let fresh = claude_oauth_token().ok_or("keychain: re-read failed after 401")?;
        let retry = usage_get(&fresh)?;
        code = retry.0;
        body = retry.1;
    }
    if code != 200 {
        return Err(format!("usage endpoint http {code}"));
    }
    let limits = parse_usage_limits(&body).ok_or("usage response carries no `limits` array")?;
    Ok(UsageInfo { source: "oauth".into(), fetched_at: now, limits })
}

/// The statusline widget's local snapshot: `{"five_hour":{"used_percentage":46,
/// "resets_at":<epoch secs>}, "seven_day":{…}}`. No severity and no model bucket
/// — the frontend dims the whole widget for this source.
fn usage_from_statusline() -> Option<UsageInfo> {
    let home = std::env::var("HOME").ok()?;
    let path = Path::new(&home).join(".claude/widgets/rate_limits.json");
    let fetched_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_else(sysclock::now_epoch);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    let mut limits = Vec::new();
    for (key, kind, label) in [("five_hour", "session", "Session"), ("seven_day", "weekly_all", "Weekly")] {
        let bucket = match v.get(key) {
            Some(b) => b,
            None => continue,
        };
        let percent = match bucket.get("used_percentage").and_then(|p| p.as_f64()) {
            Some(p) => p,
            None => continue,
        };
        limits.push(UsageLimit {
            kind: kind.into(),
            label: label.into(),
            percent,
            severity: "normal".into(),
            resets_at: bucket.get("resets_at").and_then(|r| r.as_i64()),
        });
    }
    if limits.is_empty() {
        return None;
    }
    Some(UsageInfo { source: "statusline".into(), fetched_at, limits })
}

/// Is a real fetch still being held off after a failure? Its own function so
/// the boundary is assertable — the whole defect was a floor that existed for
/// successes and not for failures, and a silently drifting constant here brings
/// it straight back.
fn usage_in_backoff(now: i64, failed_at: Option<i64>) -> bool {
    failed_at.is_some_and(|t| now - t < USAGE_FAIL_TTL_SECS)
}

/// The answer when there is no live one: the best reading we still hold, marked
/// as not-live so the frontend dims it.
///
/// This exists because "the endpoint said no" used to mean the widget VANISHED.
/// A 429 that lasted four minutes took the rail tile with it, and a reading 130
/// seconds old — indistinguishable from a fresh one at the resolution of a 4px
/// bar — was thrown away to show nothing at all. Degrading to it keeps the
/// answer on screen and keeps the layout still.
///
/// The cached reading wins while it is inside the ceiling, even against a
/// FRESHER statusline snapshot — the one place this does not simply prefer
/// newer data, and deliberately.
///
/// The snapshot is often the newer of the two: the statusline rewrites
/// `rate_limits.json` on every Claude Code prompt, while the cached reading is
/// by construction at least `USAGE_TTL_SECS` old by the time this runs. But it
/// is a POORER SHAPE — two rows instead of three, no severity tiers, no model
/// bucket. Preferring it would drop a bar and lose the amber tier for the
/// length of an outage and then put them back, which is the layout moving
/// exactly when this function exists to hold it still. The percentages here
/// measure 5h and 7d windows; between a reading two minutes old and one five
/// seconds old there is nothing to choose, and the ceiling already guarantees
/// the older one is not misleading. So: same shape as the live widget for as
/// long as that is honest, and the snapshot as the fallback for when it is not.
fn usage_degraded(now: i64, cached: Option<UsageInfo>, snapshot: Option<UsageInfo>) -> UsageInfo {
    match cached.filter(|c| now - c.fetched_at <= USAGE_STALE_MAX_SECS) {
        Some(c) => UsageInfo { source: "cached".into(), ..c },
        None => snapshot
            .unwrap_or(UsageInfo { source: "unavailable".into(), fetched_at: now, limits: Vec::new() }),
    }
}

/// Plan-usage bars for the nav footer. Never Errs on "no data" — the widget is
/// ambient, and a banner for a background poll would be worse than a blank
/// corner; the reason goes to app.log instead.
#[tauri::command]
async fn claude_usage() -> Result<UsageInfo, String> {
    let now = sysclock::now_epoch();
    let cached = USAGE_CACHE.lock().unwrap().clone();
    if let Some(c) = cached.as_ref() {
        if now - c.fetched_at < USAGE_TTL_SECS {
            return Ok(c.clone());
        }
    }
    // The negative half of the TTL. Note it is checked AFTER the positive one:
    // a failure never shortens the life of a good reading.
    if usage_in_backoff(now, *USAGE_FAIL_AT.lock().unwrap()) {
        // Silent on purpose: one log line per real attempt, not one per pull.
        // The suppressed pulls ARE the bug this is fixing, and logging them
        // would reproduce its shape in the file used to diagnose it.
        return Ok(usage_degraded(now, cached, usage_from_statusline()));
    }
    match usage_from_oauth(now) {
        Ok(info) => {
            *USAGE_CACHE.lock().unwrap() = Some(info.clone());
            *USAGE_FAIL_AT.lock().unwrap() = None;
            Ok(info)
        }
        Err(why) => {
            *USAGE_FAIL_AT.lock().unwrap() = Some(now);
            // Re-read the cache rather than degrading from the copy taken at
            // the top: curl just spent up to 15 seconds, and two pulls overlap
            // ROUTINELY here — the focus listener and the interval both invoke
            // this command, which is the doubled-pull shape the backoff above
            // exists to damp. If the other one succeeded while we were out, the
            // cache now holds a reading fresher than anything we could degrade
            // to, and the frontend takes whichever response lands LAST: serving
            // the pre-fetch copy would dim a live widget — or hide it outright
            // on a cold start — for up to a full poll period, purely because
            // the loser of a race answered second.
            let cached = USAGE_CACHE.lock().unwrap().clone();
            // …and if that reading is inside the positive TTL, it is not a
            // fallback at all. Answer exactly as the check at the top of this
            // function would have, had we arrived a moment later.
            if let Some(c) = cached.as_ref() {
                if now - c.fetched_at < USAGE_TTL_SECS {
                    applog("warn", &format!("claude_usage: oauth unavailable: {why} — another pull succeeded, serving that"));
                    return Ok(c.clone());
                }
            }
            let out = usage_degraded(now, cached, usage_from_statusline());
            // What the user will SEE, next to why — the old pair of lines said
            // "widget hidden" unconditionally, which was about to stop being
            // true for most failures.
            let shown = match out.source.as_str() {
                "unavailable" => "widget hidden".to_string(),
                src => format!("showing {src} reading from {}s ago", now - out.fetched_at),
            };
            applog("warn", &format!("claude_usage: oauth unavailable: {why} — {shown}"));
            Ok(out)
        }
    }
}

// ── Claude service status (status.claude.com) ───────────────────────────────
// Statuspage's public summary: unauthenticated, ~2KB, no credentials anywhere
// near it. The page carries SIX components and this app depends on TWO — a
// place's session is Claude Code, the usage bars are the API — so the severity
// below is computed from the WATCHED components only and NEVER from the page's
// own top-level `status.indicator`, which covers all six. That is not a
// hypothetical: the most recent incident on the page while this was written was
// "Issues with Google Play subscriptions", which would have lit a badge in the
// nav for something no worktree can feel.
//
// Matched by ID first: `k8w3r06qmzrp` outlives "Claude API (api.anthropic.com)"
// being retitled, and a rename is exactly the change nobody here would notice.
// The name prefix is the fallback, and the name the page gives is what the UI
// shows — it is the string the status page itself uses, so a user comparing the
// two sees the same words.
//
// Missing data is NOT an error, for the same reason as `claude_usage` above: a
// probe whose whole job is "is Claude broken" must not raise a dialog when the
// network is down. `source: "unavailable"` renders nothing and the reason goes
// to app.log.

const STATUS_URL: &str = "https://status.claude.com/api/v2/summary.json";
/// The status page's URL for a human, carried in the payload so the frontend
/// never hardcodes a second copy of it.
const STATUS_PAGE: &str = "https://status.claude.com";
/// (component id, name prefix). Order is the display order.
const STATUS_WATCHED: [(&str, &str); 2] = [
    ("yyzkbfz2thpt", "Claude Code"),
    ("k8w3r06qmzrp", "Claude API"),
];
/// Minimum gap between real fetches. Longer than usage's 120s: a status page
/// changes on human timescales (an incident is minutes old before it is
/// posted), and this is a poll nobody asked for.
const STATUS_TTL_SECS: i64 = 240;
/// How long an EXPIRED answer may still be served when a fetch fails. A pull
/// that lands before the network is back — waking from a closed lid is the
/// common one, and the window-focus pull makes it likely — would otherwise turn
/// a live incident into silence and the indicator would blink out and back.
/// Bounded, because the opposite failure is worse: a cached "Claude Code is
/// down" held over a long disconnection would still be on screen hours after
/// the outage ended. Past this, silence is the honest answer.
const STATUS_STALE_MAX_SECS: i64 = 1800;

#[derive(Serialize, Clone, PartialEq)]
struct StatusComponent {
    name: String,
    /// Statuspage's own vocabulary, verbatim: operational | degraded_performance
    /// | partial_outage | major_outage | under_maintenance.
    status: String,
}

#[derive(Serialize, Clone, PartialEq)]
struct StatusIncident {
    name: String,
    /// investigating | identified | monitoring | resolved
    status: String,
    /// The latest update's body, which is the one sentence worth showing.
    body: Option<String>,
    updated_at: Option<i64>,
    url: Option<String>,
}

#[derive(Serialize, Clone, PartialEq)]
struct ClaudeStatus {
    source: String, // live | unavailable
    fetched_at: i64,
    /// none | degraded | down — the only field the indicator's visibility reads.
    severity: String,
    components: Vec<StatusComponent>,
    /// How many components the page carries in total. Only the footer line
    /// "watching 2 of 6" reads it — but that line is the one place the UI
    /// admits its own scope, and a hardcoded 6 would quietly start lying.
    total: usize,
    incident: Option<StatusIncident>,
    page_url: String,
}

static STATUS_CACHE: Mutex<Option<ClaudeStatus>> = Mutex::new(None);

/// A component's status → 0 none / 1 degraded / 2 down. An UNKNOWN value counts
/// as degraded rather than as operational: if Statuspage adds a state, the
/// honest default is "something is off", not silence.
fn status_rank(s: &str) -> u8 {
    match s {
        "operational" => 0,
        "major_outage" => 2,
        _ => 1,
    }
}

fn status_word(rank: u8) -> &'static str {
    match rank {
        0 => "none",
        2 => "down",
        _ => "degraded",
    }
}

/// `summary.json` → our payload. Pure, so the mapping is unit-tested against a
/// captured body rather than against the live page.
fn status_parse(body: &str, now: i64) -> Result<ClaudeStatus, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("bad json: {e}"))?;
    let comps = v
        .get("components")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "no components array".to_string())?;

    let mut watched_ids: Vec<String> = Vec::new();
    let mut out: Vec<StatusComponent> = Vec::new();
    for (id, prefix) in STATUS_WATCHED {
        let hit = comps
            .iter()
            .find(|c| c.get("id").and_then(|x| x.as_str()) == Some(id))
            .or_else(|| {
                comps.iter().find(|c| {
                    c.get("name")
                        .and_then(|x| x.as_str())
                        .is_some_and(|n| n.starts_with(prefix))
                })
            });
        let Some(c) = hit else { continue };
        watched_ids.push(c.get("id").and_then(|x| x.as_str()).unwrap_or(id).to_string());
        out.push(StatusComponent {
            name: c.get("name").and_then(|x| x.as_str()).unwrap_or(prefix).to_string(),
            status: c
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("operational")
                .to_string(),
        });
    }
    // Every watched component gone from the page is the shape moving under us,
    // not an all-clear: say so rather than reporting calm.
    if out.is_empty() {
        return Err("no watched component on the page".into());
    }

    let rank = out.iter().map(|c| status_rank(&c.status)).max().unwrap_or(0);

    // `incidents` in summary.json is the UNRESOLVED set. We take the first one
    // that names a watched component — the page lists newest first, and a second
    // simultaneous incident is detail the link covers.
    let incident = v
        .get("incidents")
        .and_then(|i| i.as_array())
        .and_then(|arr| {
            arr.iter().find(|inc| {
                inc.get("components")
                    .and_then(|c| c.as_array())
                    .is_some_and(|cs| {
                        cs.iter().any(|c| {
                            c.get("id")
                                .and_then(|x| x.as_str())
                                .is_some_and(|id| watched_ids.iter().any(|w| w == id))
                        })
                    })
            })
        })
        .map(|inc| {
            let latest = inc
                .get("incident_updates")
                .and_then(|u| u.as_array())
                .and_then(|a| a.first());
            StatusIncident {
                name: inc
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("Incident")
                    .to_string(),
                status: inc
                    .get("status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("investigating")
                    .to_string(),
                body: latest
                    .and_then(|u| u.get("body"))
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
                updated_at: latest
                    .and_then(|u| u.get("updated_at").or_else(|| u.get("created_at")))
                    .and_then(|x| x.as_str())
                    .and_then(parse_iso8601)
                    .or_else(|| {
                        inc.get("updated_at")
                            .and_then(|x| x.as_str())
                            .and_then(parse_iso8601)
                    }),
                url: inc
                    .get("shortlink")
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
            }
        });

    Ok(ClaudeStatus {
        source: "live".into(),
        fetched_at: now,
        severity: status_word(rank).into(),
        components: out,
        total: comps.len(),
        incident,
        page_url: STATUS_PAGE.into(),
    })
}

/// GET the status summary → (http status, body). curl, same reasoning as
/// `usage_get`: the dep tree and the TLS stack stay where they are.
fn status_get() -> Result<(u16, String), String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-s", "--max-time", "10", "-w", "\n%{http_code}", STATUS_URL]);
    let out = run_deadline(cmd, 15).map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("curl exited {}", out.status.code().unwrap_or(-1)));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let cut = text.rfind('\n').ok_or("curl produced no status line")?;
    let code: u16 = text[cut + 1..]
        .trim()
        .parse()
        .map_err(|_| "curl produced no status code".to_string())?;
    Ok((code, text[..cut].to_string()))
}

/// Is Claude itself broken right now. Never Errs — see the block comment above.
#[tauri::command]
async fn claude_status() -> Result<ClaudeStatus, String> {
    let now = sysclock::now_epoch();
    let cached = STATUS_CACHE.lock().unwrap().clone();
    if let Some(c) = cached.as_ref() {
        if now - c.fetched_at < STATUS_TTL_SECS {
            return Ok(c.clone());
        }
    }
    // A failed fetch falls back to the expired entry while it is young enough
    // to still be true (see STATUS_STALE_MAX_SECS), and to silence after that.
    let fallback = |why: String| match stale_or_silence(cached, now) {
        Some(c) => {
            applog("warn", &format!("claude_status: {why} — serving the last answer"));
            c
        }
        None => fallback_hard(now, why),
    };
    match status_get() {
        Ok((200, body)) => match status_parse(&body, now) {
            Ok(info) => {
                *STATUS_CACHE.lock().unwrap() = Some(info.clone());
                Ok(info)
            }
            // A body we cannot read is NOT a transient failure — the page's
            // shape moved — so this one does not fall back to a cached answer
            // that the next fetch will never be able to refresh.
            Err(why) => Ok(fallback_hard(now, why)),
        },
        Ok((code, _)) => Ok(fallback(format!("http {code}"))),
        Err(why) => Ok(fallback(why)),
    }
}

/// What a FAILED fetch should serve: the expired cache while it is young enough
/// to still be true, or `None` for silence. Pure, so the boundary is a test and
/// not a claim.
fn stale_or_silence(cached: Option<ClaudeStatus>, now: i64) -> Option<ClaudeStatus> {
    match cached {
        Some(c) if now - c.fetched_at <= STATUS_STALE_MAX_SECS => Some(c),
        _ => None,
    }
}

/// Silence, with the reason logged. Used when the page answered but we could not
/// read it: serving a stale entry there would keep a possibly-resolved incident
/// on screen with no prospect of it ever being corrected.
fn fallback_hard(now: i64, why: String) -> ClaudeStatus {
    applog("warn", &format!("claude_status: {why}"));
    ClaudeStatus {
        source: "unavailable".into(),
        fetched_at: now,
        severity: "none".into(),
        components: Vec::new(),
        total: 0,
        incident: None,
        page_url: STATUS_PAGE.into(),
    }
}

/// Find the installed CLI: whatever `command -v` resolves in the USER'S login
/// shell (zsh reads ~/.zprofile; plain sh would not — the app may be launched
/// from Finder with a bare PATH), then the common install dirs. Chatty profiles
/// are filtered by taking the last absolute-path line. Returns (path, version).
fn cli_binary() -> Option<(String, String)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut candidates: Vec<String> = Vec::new();
    let mut probe = std::process::Command::new(&shell);
    probe.args(["-lc", "command -v -- worktrees"]);
    if let Ok(out) = run_deadline(probe, 5) {
        if out.status.success() {
            if let Some(p) = String::from_utf8_lossy(&out.stdout)
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| l.starts_with('/'))
            {
                candidates.push(p.to_string());
            }
        }
    }
    candidates.push(format!("{home}/.local/bin/worktrees"));
    candidates.push(format!("{home}/bin/worktrees"));
    candidates.push("/opt/homebrew/bin/worktrees".into());
    candidates.push("/usr/local/bin/worktrees".into());
    for path in candidates {
        let mut c = std::process::Command::new(&path);
        c.arg("--version");
        if let Ok(out) = run_deadline(c, 5) {
            if out.status.success() {
                let full = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // "worktrees 0.2.0" → "0.2.0"
                let v = full.rsplit(' ').next().unwrap_or(&full).to_string();
                return Some((path, v));
            }
        }
    }
    None
}

#[tauri::command]
async fn check_update() -> Result<UpdateInfo, String> {
    let (cli_path, cli_version) = match cli_binary() {
        Some((p, v)) => (Some(p), Some(v)),
        None => (None, None),
    };
    Ok(UpdateInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        cli_version,
        cli_path,
        latest: latest_release_tag(),
    })
}

/// Update the installed CLI via the PINNED-TAG installer — install.sh stays the
/// single source of truth for download/checksum/replace. Hardening (review):
/// the webview PROPOSES a tag but this side re-resolves latest and requires an
/// exact match (no webview-driven downgrade / stale pin); the script downloads
/// to a temp file with a CHECKED curl (a piped `curl | bash` masks download
/// failure as success — pipeline status is the last command's); and success is
/// declared only when the re-probed CLI actually reports the new version.
#[tauri::command]
async fn update_cli(tag: String) -> Result<CmdResult, String> {
    let ok_tag = tag.starts_with('v')
        && tag.len() > 1
        && tag[1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !ok_tag {
        return Err(format!("suspicious tag '{tag}'"));
    }
    match latest_release_tag() {
        Some(latest) if latest == tag => {}
        Some(latest) => return Err(format!("{tag} is not the latest release ({latest}) — re-check for updates")),
        None => return Err("could not re-verify the latest release (offline?)".into()),
    }

    let url = format!("https://raw.githubusercontent.com/{REPO_SLUG}/{tag}/install.sh");
    let script = std::env::temp_dir().join(format!("worktrees-install-{}.sh", std::process::id()));
    let mut fetch = std::process::Command::new("curl");
    fetch
        .args(["-fsSL", "--connect-timeout", "10", "--max-time", "120", "-o"])
        .arg(&script)
        .arg(&url);
    let f = run_deadline(fetch, 150).map_err(|e| {
        applog("error", &format!("update_cli {tag}: installer download failed: {e}"));
        format!("installer download failed: {e}")
    })?;
    if !f.status.success() {
        applog("error", &format!("update_cli {tag}: installer download rc={}", f.status.code().unwrap_or(-1)));
        let _ = std::fs::remove_file(&script);
        return Ok(CmdResult {
            ok: false,
            code: f.status.code().unwrap_or(-1),
            output: format!("installer download failed\n{}", String::from_utf8_lossy(&f.stderr)),
            slug: None,
            needs_confirm: None,
            warnings: Vec::new(),
        });
    }

    let mut run = std::process::Command::new("bash");
    run.arg(&script).env("WORKTREES_INSTALL_VERSION", &tag);
    // replace the binary that's actually resolved, not blindly ~/.local/bin
    if let Some((path, _)) = cli_binary() {
        if let Some(parent) = Path::new(&path).parent() {
            run.env("WORKTREES_INSTALL_DIR", parent);
        }
    }
    let out = run_deadline(run, 600).map_err(|e| {
        applog("error", &format!("update_cli {tag}: installer run failed: {e}"));
        format!("installer run failed: {e}")
    })?;
    let _ = std::fs::remove_file(&script);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));

    // the real success signal: the resolved CLI now reports the new version
    let verified = cli_binary().map(|(_, v)| format!("v{v}") == tag).unwrap_or(false);
    if !out.status.success() || !verified {
        applog("warn", &format!("update_cli {tag}: rc={} verified={verified}", out.status.code().unwrap_or(-1)));
    }
    if out.status.success() && !verified {
        text.push_str("\n! installer exited 0 but the resolved CLI does not report the new version — check PATH shadowing.");
    }
    Ok(CmdResult {
        ok: out.status.success() && verified,
        code: out.status.code().unwrap_or(-1),
        output: text,
        slug: None,
        needs_confirm: None,
        warnings: Vec::new(),
    })
}

// ── AI command config (Settings → Commands) ──────────────────────────────────
// Phase 1: read-only visibility of the engine's ai_cmd / ai_resume_arg, which
// live in the SHARED CLI config (~/.config/worktrees/config). The app exposes
// nothing to edit yet (no core config writer). All resolution goes through
// worktrees-core's pub fns so the app never re-implements precedence.

#[derive(Serialize)]
struct AiConfig {
    /// Effective resolved ai_cmd (flag=None → env > config > default `claude`).
    ai_cmd: String,
    /// Effective resolved resume arg (env > config > default `-r`).
    ai_resume_arg: String,
    /// The shared config file path (respects $XDG_CONFIG_HOME).
    path: String,
    /// Whether that config file actually exists on disk.
    exists: bool,
}

/// The engine's effective AI command + resume arg, plus the shared config path.
/// Read-only (Phase 1). The frontend shows it under the ai_auto_resume checkbox
/// and offers a reveal button (reveal the PARENT dir when the file is absent —
/// revealItemInDir rejects a non-existent path).
#[tauri::command]
async fn get_ai_config() -> Result<AiConfig, String> {
    let path = worktrees_core::config::config_path();
    Ok(AiConfig {
        ai_cmd: worktrees_core::config::resolve_ai_cmd(None),
        ai_resume_arg: worktrees_core::config::resolve_ai_resume_arg(),
        path: path.to_string_lossy().into_owned(),
        exists: path.exists(),
    })
}

// ── Claude MCP setup (Settings → Claude, and the Home nudge) ─────────────────
// Thin wrappers over `worktrees_core::mcpsetup`, which holds the whole rule set
// (what counts as installed, which scopes we write, why we shell out to `claude`
// instead of editing ~/.claude.json). The CLI's `worktrees mcp --status` runs the
// SAME functions, so the two surfaces cannot disagree about the verdict.
//
// `repo` is optional and normally the project in focus: it is what makes the
// local and project scopes checkable. Home has no repo and passes none — the
// user scope, which is the only one we ever write, does not need it.

/// Is the worktrees MCP server wired into this machine's claude?
///
/// Cheap by construction — it parses `~/.claude.json` rather than running
/// `claude mcp list`, which health-checks by launching every server in the cwd
/// (~1.1s, and a false ✘ whenever the cwd is not a repo). Safe to call on a
/// sheet open or a Home render; still `async`, like every command here, because
/// it touches the filesystem.
///
/// Logged, unlike most read-only commands. `mcp_install`/`mcp_uninstall` already
/// applog, but the state that decides whether the offer EVER appears is
/// computed here — and every non-`absent` answer is a silent one, by design. A
/// user reporting "it never offered" leaves no other trace: without this line
/// the only way to tell `absent` (card suppressed by something in the UI) from
/// `cli-missing`/`elsewhere` (card correctly withheld) is to ask them to open
/// Settings and read it out.
#[tauri::command]
async fn mcp_status(repo: Option<String>) -> Result<worktrees_core::mcpsetup::Status, String> {
    let s = worktrees_core::mcpsetup::status(repo.as_deref());
    applog(
        "info",
        &format!(
            "mcp_status repo={} -> state={:?} found_in={:?} claude={} worktrees={}",
            repo.as_deref().unwrap_or("-"),
            s.state,
            s.found_in,
            s.claude_bin.as_deref().unwrap_or("-"),
            s.worktrees_bin.as_deref().unwrap_or("-"),
        ),
    );
    Ok(s)
}

/// Wire it in (or repair, or re-install with a different `--mutations`).
///
/// Spawns `claude mcp add`, so it is slow enough to need a busy state in the UI
/// and is deadline-guarded in core. The returned `Outcome` carries the RE-READ
/// status, which is the real success signal — the frontend renders that rather
/// than assuming the click worked.
#[tauri::command]
async fn mcp_install(repo: Option<String>, mutations: bool) -> Result<worktrees_core::mcpsetup::Outcome, String> {
    let r = worktrees_core::mcpsetup::install(repo.as_deref(), mutations);
    match &r {
        Ok(o) => applog(
            if o.ok { "info" } else { "warn" },
            &format!("mcp_install mutations={mutations}: ok={} state={:?}", o.ok, o.status.state),
        ),
        Err(e) => applog("error", &format!("mcp_install mutations={mutations}: {e}")),
    }
    r
}

/// Remove the user-scope server, so the panel is not a one-way door.
#[tauri::command]
async fn mcp_uninstall(repo: Option<String>) -> Result<worktrees_core::mcpsetup::Outcome, String> {
    let r = worktrees_core::mcpsetup::uninstall(repo.as_deref());
    if let Err(e) = &r {
        applog("error", &format!("mcp_uninstall: {e}"));
    }
    r
}

#[tauri::command]
async fn codex_mcp_status() -> Result<worktrees_core::codexmcp::Status, String> {
    Ok(worktrees_core::codexmcp::status())
}

#[tauri::command]
async fn codex_mcp_install(mutations: bool) -> Result<worktrees_core::codexmcp::Outcome, String> {
    worktrees_core::codexmcp::install(mutations)
}

#[tauri::command]
async fn codex_mcp_uninstall() -> Result<worktrees_core::codexmcp::Outcome, String> {
    worktrees_core::codexmcp::uninstall()
}

// ── per-project config surface (the Project sheet, proposal §10) ─────────────
// Read-only config view + the four verbs (doctor / relink / provision / init).
//
// Everything here is ON DEMAND. `doctor` in particular must NEVER be wired into
// the 3s poll thread: `places:changed` already triggers a full `list_workspace`
// (up to 16 concurrent git calls per project), and a per-place filesystem probe
// on top of that would put the project config on exactly the hot path §8 keeps
// it off. The frontend runs it when the sheet opens, after a repair, and on a
// slow (~5 min) timer.

#[derive(Serialize)]
struct ProjectFileView {
    path: String,
    /// `"link"` | `"copy"` — the per-entry mode (§3).
    mode: String,
}

#[derive(Serialize)]
struct ProjectPortsView {
    stride: u32,
    max_slots: u32,
    /// `(NAME, base_port)`, in the config's own (BTreeMap-sorted) order.
    base: Vec<(String, u32)>,
}

#[derive(Serialize)]
struct ProjectComposeView {
    /// The `-f` list in DOCKER's order (a later file overrides an earlier one).
    /// A LIST because `projcfg::Compose` is one: `down -v` only removes volumes
    /// declared in the files it is handed, so the base compose file has to travel
    /// with the worktree override. `file = "…"` is still the one-element form.
    files: Vec<String>,
    /// The `{prefix}`/`{slug}` template, unexpanded — this is the declaration.
    project: String,
}

/// `[docs]` as the sheet shows it. Present here for the same reason every other
/// section is: a declared section the sheet does not mention leaves the user
/// unable to tell whether it took effect — and `[docs]` is the one section
/// whose effect is on ANOTHER surface (the Docs tab), so it is the easiest to
/// write wrong and never notice.
#[derive(Serialize)]
struct ProjectDocsView {
    /// Declared order, which is also listing order (`docs::index_with`).
    paths: Vec<String>,
    /// Where reading starts, when declared.
    index: Option<String>,
}

/// The direct analogue of `get_ai_config`: read-only visibility of a config the
/// app does not (yet) edit. Never a `CmdResult` — the sheet renders structure.
#[derive(Serialize)]
struct ProjectConfigView {
    /// Absolute path of `.worktrees.toml` — where it IS, or where it would go.
    path: String,
    exists: bool,
    files: Vec<ProjectFileView>,
    ports: Option<ProjectPortsView>,
    compose: Option<ProjectComposeView>,
    docs: Option<ProjectDocsView>,
    /// A fatal parse/validation failure, rendered as its `file:line: message`.
    /// This is the most important thing the sheet can say when set: it is *why*
    /// every op in the repo is refusing.
    error: Option<String>,
    /// Non-fatal parse findings, already rendered as `file:line: message`. Today
    /// that is exactly `projcfg`'s `unknown-key` family — an unknown key, an
    /// unknown table, or a wrong-shaped `[project]`. (Prefix disagreement is a
    /// `doctor` finding, not a parse one: it needs the repo, not the file.)
    warnings: Vec<String>,
}

#[tauri::command]
async fn project_config_read(repo: String) -> Result<ProjectConfigView, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("project_config_read repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let main = Path::new(&p.main_root);
    let path = main.join(worktrees_core::projcfg::CONFIG_FILE);
    let mut view = ProjectConfigView {
        path: path.to_string_lossy().into_owned(),
        exists: path.exists(),
        files: Vec::new(),
        ports: None,
        compose: None,
        docs: None,
        error: None,
        warnings: Vec::new(),
    };
    match worktrees_core::projcfg::load(main) {
        Ok((Some(cfg), findings)) => {
            view.files = cfg
                .files
                .iter()
                .map(|f| ProjectFileView {
                    path: f.path.as_str().to_string(),
                    mode: match f.mode {
                        worktrees_core::projcfg::Mode::Link => "link",
                        worktrees_core::projcfg::Mode::Copy => "copy",
                    }
                    .to_string(),
                })
                .collect();
            view.ports = cfg.ports.as_ref().map(|x| ProjectPortsView {
                stride: x.stride,
                max_slots: x.max_slots,
                base: x.base.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            });
            view.compose = cfg.compose.as_ref().map(|c| ProjectComposeView {
                files: c.files.iter().map(|f| f.as_str().to_string()).collect(),
                project: c.project.clone(),
            });
            view.docs = cfg.docs.as_ref().map(|d| ProjectDocsView {
                paths: d.paths.iter().map(|p| p.as_str().to_string()).collect(),
                index: d.index.as_ref().map(|i| i.as_str().to_string()),
            });
            view.warnings = findings.iter().map(|f| f.message.clone()).collect();
        }
        // No config is a healthy repo, not a broken one (§2.4).
        Ok((None, _)) => {}
        Err(e) => {
            applog("warn", &format!("project_config_read repo={repo}: {e}"));
            view.error = Some(e.to_string());
        }
    }
    Ok(view)
}

/// `doctor`'s TYPED report — badges need structure, so this is deliberately not
/// a `CmdResult` (§10). `findings` is `diag::Finding` verbatim, so the app's
/// vocabulary of severities and codes is the CLI's by construction.
#[derive(Serialize)]
struct DoctorReport {
    /// `0` clean · `1` a guard only the user can fix (bad worktree name, an
    /// unreadable config) · `2` findings present.
    code: i32,
    schema_version: u32,
    findings: Vec<worktrees_core::diag::Finding>,
    /// Set when the run produced no report at all (`code == 1`): the guard
    /// message, already in app.log. Never swallowed.
    error: Option<String>,
}

/// Report drift for one place, or for every place in the project (`slug: None`).
///
/// Runs the REAL `cmd_doctor` in `--json` mode and parses the line it emits,
/// rather than re-assembling findings here: the CLI and the app must never
/// disagree about what "drift" means.
#[tauri::command]
async fn doctor(repo: String, slug: Option<String>) -> Result<DoctorReport, String> {
    let project = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("doctor repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let mut args: Vec<String> = Vec::new();
    if let Some(s) = slug.filter(|s| !s.trim().is_empty()) {
        args.push(s);
    }
    args.push("--json".into());
    let mut ui = CaptureUi::default();
    let code = ops::cmd_doctor(&project, &mut ui, &args);
    let parsed = ui
        .lines
        .iter()
        .rev()
        .find_map(|l| serde_json::from_str::<worktrees_core::diag::Report>(l).ok());
    match parsed {
        Some(r) => {
            if !r.findings.is_empty() {
                applog("info", &format!("doctor rc={code} repo={repo}: {} finding(s)", r.findings.len()));
            }
            Ok(DoctorReport { code, schema_version: r.schema_version, findings: r.findings, error: None })
        }
        None => {
            // rc 1 before the report was emitted (a usage guard, or a config that
            // does not parse). The lines ARE the diagnosis.
            let msg = ui.lines.join("\n");
            applog("warn", &format!("doctor rc={code} repo={repo}: {msg}"));
            Ok(DoctorReport {
                code,
                schema_version: worktrees_core::diag::SCHEMA_VERSION,
                findings: Vec::new(),
                error: Some(if msg.is_empty() { format!("doctor exited {code}") } else { msg }),
            })
        }
    }
}

/// One place's health verdict — `doctor`'s shape, for `status`.
///
/// `report` is `health::Report` verbatim, so the sheet's vocabulary of verdicts
/// and reasons is the CLI's by construction; `error` carries the guard message
/// when `cmd_status` exited before emitting any JSON (never swallowed).
#[derive(Serialize)]
struct HealthReport {
    /// `0` a report was emitted · `1` usage / unknown / unregistered place.
    code: i32,
    report: Option<worktrees_core::health::Report>,
    error: Option<String>,
}

/// Run the REAL `cmd_status --json` in-process for one place and parse the line
/// it emits → `(exit code, report, the captured lines)`.
///
/// Factored out because TWO commands need the same verdict and must not each
/// grow their own parse: `place_health` renders it, and `ai_status_report`
/// embeds its JSON in the prompt it hands claude. A second copy would be a
/// second answer to "what did the check say" — the same reason `place_health`
/// shells through core at all instead of re-assembling facts here.
///
/// `None` for the report means `cmd_status` exited on a guard before emitting
/// any JSON (unknown or unregistered place); the lines ARE the diagnosis then.
fn status_of(project: &Project, slug: &str) -> (i32, Option<worktrees_core::health::Report>, String) {
    let args: Vec<String> = vec![slug.to_string(), "--json".into()];
    let mut ui = CaptureUi::default();
    let code = ops::cmd_status(project, &mut ui, &args);
    let parsed = ui
        .lines
        .iter()
        .rev()
        .find_map(|l| serde_json::from_str::<worktrees_core::health::Report>(l).ok());
    (code, parsed, ui.lines.join("\n"))
}

/// Health verdict for one place. Runs the REAL `cmd_status` in `--json` mode
/// in-process and parses the line it emits, for the same reason `doctor` does:
/// the CLI and the app must never disagree about what "at risk" means.
#[tauri::command]
async fn place_health(repo: String, slug: String) -> Result<HealthReport, String> {
    let project = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("place_health repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let (code, parsed, lines) = status_of(&project, &slug);
    match parsed {
        Some(r) => Ok(HealthReport { code, report: Some(r), error: None }),
        None => {
            // rc 1 before the report was emitted (an unknown or unregistered
            // place). The captured lines ARE the diagnosis.
            applog("warn", &format!("place_health rc={code} repo={repo} slug={slug}: {lines}"));
            Ok(HealthReport {
                code,
                report: None,
                error: Some(if lines.is_empty() { format!("status exited {code}") } else { lines }),
            })
        }
    }
}

// ── project automations: the dock's fourth tab ───────────────────────────────
// Thin wrappers over `worktrees_core::automation` + `::runs`, which own the
// sidecar, the ledger and the runner. Nothing here keeps a second copy of any
// of that — the CLI reads the same files, and a tab that disagreed with
// `worktrees automations ls` would be the bug this layering exists to prevent.
//
// The VIEWS are explicit structs rather than `serde_json::Value` for the reason
// every other view struct here is: the frontend's types are read off these
// fields, and an `Option` that vanished when it was `None` would make a run
// that failed indistinguishable from one that has not finished.

/// One run, as a LIST row: everything but the two bulky halves (`facts`,
/// `report_md`). `findings` is a COUNT here — the rows say "3 findings", and
/// shipping the findings themselves down the 2s poll would be the list paying
/// for the run view's data.
#[derive(Serialize)]
struct RunSummaryView {
    id: String,
    automation: String,
    trigger: String,
    started_epoch: i64,
    finished_epoch: Option<i64>,
    status: String,
    findings: usize,
    dropped: usize,
    actions: usize,
    seconds: Option<u64>,
    error: Option<String>,
}

impl RunSummaryView {
    fn of(r: &worktrees_core::runs::Run) -> RunSummaryView {
        RunSummaryView {
            id: r.id.clone(),
            automation: r.automation.clone(),
            trigger: r.trigger.as_str().to_string(),
            started_epoch: r.started_epoch,
            finished_epoch: r.finished_epoch,
            status: r.status.as_str().to_string(),
            findings: r.findings.len(),
            dropped: r.dropped.len(),
            actions: r.actions.len(),
            seconds: r.seconds,
            error: r.error.clone(),
        }
    }
}

/// One definition plus the one fact the row on the right shows.
#[derive(Serialize)]
struct AutomationView {
    slug: String,
    name: String,
    brief: String,
    /// `automation::When`, verbatim: `{"kind":"manual"}` /
    /// `{"kind":"daily","at":"08:00"}` / `{"kind":"weekly","day":"mon","at":"09:00"}`.
    /// The frontend renders it as words and posts it back unchanged, so the
    /// vocabulary has exactly one home.
    when: serde_json::Value,
    scope: String,
    tier: String,
    enabled: bool,
    created_epoch: i64,
    last_run: Option<RunSummaryView>,
}

/// A run in FULL, minus `facts`. `facts` is a `health::Report` per place — a few
/// KB × places — and no surface renders it; a view that carried it would put
/// that on every open of the run view for nothing. Add it when something needs
/// it, rather than shipping it against a future that may not come.
#[derive(Serialize)]
struct RunView {
    id: String,
    automation: String,
    trigger: String,
    started_epoch: i64,
    finished_epoch: Option<i64>,
    status: String,
    profile: Option<String>,
    places: Vec<String>,
    skipped: Vec<worktrees_core::runs::Skipped>,
    findings: Vec<worktrees_core::runs::Finding>,
    dropped: Vec<worktrees_core::runs::Dropped>,
    actions: Vec<worktrees_core::runs::Action>,
    report_md: Option<String>,
    turns: Option<u32>,
    seconds: Option<u64>,
    error: Option<String>,
    seen_epoch: Option<i64>,
}

impl RunView {
    fn of(r: worktrees_core::runs::Run) -> RunView {
        RunView {
            id: r.id,
            automation: r.automation,
            trigger: r.trigger.as_str().to_string(),
            started_epoch: r.started_epoch,
            finished_epoch: r.finished_epoch,
            status: r.status.as_str().to_string(),
            profile: r.profile,
            places: r.places,
            skipped: r.skipped,
            findings: r.findings,
            dropped: r.dropped,
            actions: r.actions,
            report_md: r.report_md,
            turns: r.turns,
            seconds: r.seconds,
            error: r.error,
            seen_epoch: r.seen_epoch,
        }
    }
}

/// What the modal sends. Every field optional: an edit that names nothing
/// changes nothing (core's `Patch`), and `when` arrives as the same JSON the
/// view handed out.
#[derive(Deserialize)]
struct AutomationPatch {
    name: Option<String>,
    brief: Option<String>,
    when: Option<serde_json::Value>,
    scope: Option<String>,
    tier: Option<String>,
    enabled: Option<bool>,
}

fn automation_view(
    slug: &str,
    a: &worktrees_core::automation::Automation,
    last: Option<&worktrees_core::runs::Run>,
) -> AutomationView {
    AutomationView {
        slug: slug.to_string(),
        name: a.name.clone(),
        brief: a.brief.clone(),
        // Infallible in practice (`When` is a closed enum of strings), and a
        // serialisation failure must not take the whole list down — the row
        // degrades to "when I ask" rather than the tab showing nothing.
        when: serde_json::to_value(&a.when).unwrap_or_else(|_| serde_json::json!({ "kind": "manual" })),
        scope: a.scope.as_str().to_string(),
        tier: a.tier.as_str().to_string(),
        enabled: a.enabled,
        created_epoch: a.created_epoch,
        last_run: last.map(RunSummaryView::of),
    }
}

/// `Project::discover`, with the failure logged — the shape `place_health` uses.
fn project_at(repo: &str, what: &str) -> Result<Project, String> {
    Project::discover(Path::new(repo)).map_err(|e| {
        applog("error", &format!("{what} repo={repo}: discover failed: {}", e.msg));
        e.msg
    })
}

#[tauri::command]
async fn list_automations(repo: String) -> Result<Vec<AutomationView>, String> {
    let project = project_at(&repo, "list_automations")?;
    // `read_reporting`, not `read_lenient`: an entry this binary cannot read is
    // dropped from the list, and a tab that showed one fewer row without saying
    // why would be exactly the silent failure `diag.rs` forbids.
    let (store, warnings) = worktrees_core::automation::read_reporting(&project.main_root);
    for w in &warnings {
        applog("warn", &format!("list_automations repo={repo}: {w}"));
    }
    let last = worktrees_core::runs::last_by_automation(&project.main_root);
    Ok(store
        .automations
        .iter()
        .map(|(slug, a)| automation_view(slug, a, last.get(slug)))
        .collect())
}

#[tauri::command]
async fn upsert_automation(
    repo: String,
    slug: Option<String>,
    patch: AutomationPatch,
) -> Result<AutomationView, String> {
    let project = project_at(&repo, "upsert_automation")?;
    let when = match patch.when {
        // The vocabulary is core's. A `when` the enum does not know is refused
        // HERE with the parse error rather than silently becoming `manual` —
        // a schedule that quietly turned into "when I ask" is a job that never
        // runs and never says so.
        Some(v) => Some(
            serde_json::from_value::<worktrees_core::automation::When>(v)
                .map_err(|e| format!("when: {e}"))?,
        ),
        None => None,
    };
    let scope = match patch.scope.as_deref() {
        Some(s) => Some(
            worktrees_core::automation::Scope::parse(s)
                .ok_or_else(|| format!("unknown scope: {s}"))?,
        ),
        None => None,
    };
    let tier = match patch.tier.as_deref() {
        Some(t) => {
            Some(worktrees_core::automation::Tier::parse(t).ok_or_else(|| format!("unknown tier: {t}"))?)
        }
        None => None,
    };
    let core_patch = worktrees_core::automation::Patch {
        name: patch.name,
        brief: patch.brief,
        when,
        scope,
        tier,
        enabled: patch.enabled,
    };
    let mut ui = CaptureUi::default();
    let (slug, entry) =
        worktrees_core::automation::upsert(&project.main_root, &mut ui, slug.as_deref(), core_patch)
            .inspect_err(|e| applog("error", &format!("upsert_automation repo={repo}: {e}")))?;
    // The rename warning ("the slug stays '…'") is core's, and it is the kind of
    // thing a user discovers later as a command that does not resolve.
    for w in ui.warnings() {
        applog("warn", &format!("upsert_automation repo={repo} slug={slug}: {w}"));
    }
    let last = worktrees_core::runs::last_by_automation(&project.main_root);
    Ok(automation_view(&slug, &entry, last.get(&slug)))
}

#[tauri::command]
async fn delete_automation(repo: String, slug: String) -> Result<(), String> {
    let project = project_at(&repo, "delete_automation")?;
    worktrees_core::automation::delete(&project.main_root, &slug)
        .inspect_err(|e| applog("error", &format!("delete_automation repo={repo} slug={slug}: {e}")))
}

/// What a click on "Run now" gets back — immediately, whatever the run then does.
#[derive(Serialize)]
struct RunStarted {
    /// The id the caller polls `list_runs` for. Empty when `already_running`:
    /// the run in flight is not this caller's, and handing back an id it did not
    /// start would make a double click look like two runs.
    id: String,
    already_running: bool,
}

/// Start a run and answer with its id. The run itself is MINUTES of `claude -p`.
///
/// In-process on a `std::thread`, not `Command::spawn` of the CLI the way
/// `mcp.rs` does it: the app LINKS core, and the CLI binary may be absent
/// entirely — `update_cli` exists precisely because it can be. A spawn would
/// make the tab silently stop working on a machine that never ran `install.sh`.
///
/// The id is minted HERE, before the thread, for the same reason the MCP tool
/// mints it before spawning: the caller needs an answer in milliseconds, and
/// `RunOpts.id` is the seam core already has for that. The lock is likewise
/// checked here — core's own check happens inside the run, which is too late to
/// answer with.
#[tauri::command]
async fn run_automation(repo: String, slug: String) -> Result<RunStarted, String> {
    let project = project_at(&repo, "run_automation")?;
    let store = worktrees_core::automation::read_lenient(&project.main_root);
    if !store.automations.contains_key(&slug) {
        return Err(format!("no such automation: {slug}"));
    }
    if worktrees_core::automation::is_running(&project.main_root, &slug) {
        return Ok(RunStarted { id: String::new(), already_running: true });
    }
    // Ask the AI seam NOW, on the invoke: a project whose ai_cmd is not claude
    // (or whose profile fails to materialize) is refused by the runner BEFORE
    // it writes a ledger entry, so from the thread below the tab would only
    // ever see an id that never appears. Here the refusal is the invoke's
    // error and reaches the toast.
    let mut pre = CaptureUi::default();
    worktrees_core::automation::preflight(&project, &mut pre)
        .inspect_err(|e| applog("error", &format!("run_automation repo={repo} slug={slug}: {e}")))?;
    for w in pre.warnings() {
        applog("warn", &format!("run_automation repo={repo} slug={slug}: {w}"));
    }
    let dir = worktrees_core::runs::ensure_ledger_dir(&project.main_root)
        .inspect_err(|e| applog("error", &format!("run_automation repo={repo}: {e}")))?;
    let id = worktrees_core::runs::new_id(&dir, &slug, worktrees_core::runs::run_now());
    let thread_id = id.clone();
    let (thread_repo, thread_slug) = (repo.clone(), slug.clone());
    // A run writes `running` to its ledger entry before anything slow (see
    // `run_inner` step 2), so the tab's poll sees the spinner without this
    // thread reporting anything at all. What it reports is the END: a `failed`
    // run's reason is in the entry, but the WARNINGS on the way there are only
    // ever said out loud, and `app.log` is where the app says things.
    thread::spawn(move || {
        let mut ui = CaptureUi::default();
        let code = worktrees_core::automation::run(
            &project,
            &mut ui,
            &thread_slug,
            worktrees_core::automation::RunOpts {
                trigger: worktrees_core::runs::Trigger::Manual,
                id: Some(thread_id.clone()),
                ..Default::default()
            },
        );
        // `warnings()` is Warn AND above, so the runner's `ui.error` lines —
        // the reason a `failed` run failed — come through here too.
        for w in ui.warnings() {
            applog("warn", &format!("run_automation {thread_slug} ({thread_id}): {w}"));
        }
        applog(
            // rc 2 is "findings" (`doctor`'s convention), not a failure.
            if code == 1 { "warn" } else { "info" },
            &format!("run_automation repo={thread_repo} {thread_slug} ({thread_id}) exited {code}"),
        );
    });
    Ok(RunStarted { id, already_running: false })
}

#[tauri::command]
async fn list_runs(repo: String, automation: Option<String>) -> Result<Vec<RunSummaryView>, String> {
    let project = project_at(&repo, "list_runs")?;
    Ok(worktrees_core::runs::list(&project.main_root, automation.as_deref())
        .iter()
        .map(RunSummaryView::of)
        .collect())
}

#[tauri::command]
async fn get_run(repo: String, id: String) -> Result<RunView, String> {
    let project = project_at(&repo, "get_run")?;
    worktrees_core::runs::read(&project.main_root, &id)
        .map(RunView::of)
        .inspect_err(|e| applog("warn", &format!("get_run repo={repo} id={id}: {e}")))
}

/// Press one of a run's proposals. The closed tool set, the slug check and the
/// `Action` record are all core's (`runs::apply_proposal`) — this is the human
/// press the whole report-only tier is built around.
#[tauri::command]
async fn apply_proposal(
    repo: String,
    run_id: String,
    finding: usize,
    proposal: usize,
) -> Result<worktrees_core::runs::Action, String> {
    let project = project_at(&repo, "apply_proposal")?;
    let mut ui = CaptureUi::default();
    let act = worktrees_core::runs::apply_proposal(&project, &mut ui, &run_id, finding, proposal)
        .inspect_err(|e| {
            applog("error", &format!("apply_proposal repo={repo} run={run_id}: {e}"))
        })?;
    // `ok: false` is a call that RAN and was refused — the button renders the
    // reason under itself, and the log keeps it too.
    if !act.ok {
        applog("warn", &format!("apply_proposal repo={repo} run={run_id}: {}", act.output));
    }
    Ok(act)
}

// ── "Ask Claude": the headless one-shot status report ────────────────────────
// The app's FIRST headless AI path. Every other launch in this codebase goes
// through tmux (`ops::launch`), which means a pane, an interactive shell and a
// human at the other end. This one reuses the profile seam and bypasses the
// pane entirely — so the two things a pane gave us for free (you can SEE what
// was launched, and you can Ctrl-C it) have to be replaced by the guard below
// and by `run_deadline`'s hard 180s.

/// What the frontend caches under `declared.status_report`.
#[derive(Serialize)]
struct AiStatusReport {
    text: String,
    epoch: i64,
    /// The verdict from the SAME `cmd_status` run whose JSON went into the
    /// prompt — so a cached read can say what the machine thought at the time,
    /// beside what claude thought about it.
    verdict: String,
}

/// The "is this really claude" guard, now core's (`profile.rs`).
///
/// Two headless callers need it — this command and `automation::run` — and the
/// reasoning it encodes (read the COMPOSED command, never `match_word`, because
/// `ai_launch_for` fails closed with a `printf` sentinel that keeps the word)
/// is the kind that must have exactly one home. The test below still drives it.
use worktrees_core::profile::claude_launch_check;

/// The prompt, composed from the health report and the worktree's path.
///
/// Pure and unit-tested because it is the whole interface to the model: the JSON
/// goes in VERBATIM (claude must not have to re-derive facts the check already
/// measured), and the ban on running commands is what keeps a headless run —
/// which cannot answer a permission prompt — from stalling on a tool it will
/// never be granted. Reads are allowed, which is why the planning files are
/// named: they are where a worktree says what it was FOR, and no git fact does.
fn status_prompt(report_json: &str, path: &str) -> String {
    format!(
        "{report_json}\n\
         \n\
         You are reporting on the git worktree at {path}. The JSON above is its \
         measured state. If task_plan.md, findings.md, progress.md or ROADMAP.md \
         exist there, read them.\n\
         \n\
         Answer in under 250 words:\n\
         (1) what was this worktree for,\n\
         (2) what state did the work end in,\n\
         (3) recommend exactly one of: resume / push-then-abandon / abandon, with \
         one line of justification.\n\
         \n\
         Do not run commands; the git facts are already in the JSON."
    )
}

/// Last `n` lines of a subprocess's stderr, for an error the user has to read.
/// Whole-stderr in a toast is unreadable and whole-stderr in an `Err` string is
/// worse; the tail is where a CLI puts the reason.
fn stderr_tail(bytes: &[u8], n: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Hand this worktree's measured state to the repo's AI profile, headless, and
/// cache the paragraph that comes back.
///
/// App-only on purpose: there is no CLI verb for it (the seam would support one
/// — see the spec's out-of-scope list — but a headless spawn with a 180s
/// deadline is a very different contract from a CLI that a user can Ctrl-C).
/// NEVER runs on its own: the sheet has a button, and nothing else calls this.
#[tauri::command]
async fn ai_status_report(repo: String, slug: String) -> Result<AiStatusReport, String> {
    let project = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("ai_status_report repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    // `place_dir`, not `wt_root.join(slug)`: `(main)` is a place too, and it
    // lives at the main checkout rather than under `.worktrees/`.
    let wt_path = project.place_dir(&slug);

    // THE seam (ops.rs): the same call every interactive launch makes, so this
    // report runs under exactly the profile a session in this place would.
    let mut ui = CaptureUi::default();
    let ai = ops::ai_launch_for(
        &project,
        &mut ui,
        &wt_path,
        &worktrees_core::config::resolve_ai_cmd(None),
    );
    // The seam's own voice — a skipped skill, or the fail-closed reason. Never
    // swallowed: without this, a broken profile's guard message below would be
    // the ONLY trace, and it deliberately points at this log.
    for w in ui.warnings() {
        applog("warn", &format!("ai_status_report repo={repo} slug={slug}: {w}"));
    }
    if let Err(msg) = claude_launch_check(&ai.cmd, &ai.match_word) {
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: {msg}"));
        return Err(msg);
    }

    let (code, report, lines) = status_of(&project, &slug);
    let Some(report) = report else {
        let why = if lines.is_empty() { format!("status exited {code}") } else { lines };
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: no status report: {why}"));
        return Err(format!("could not read this worktree's status: {why}"));
    };
    let verdict = report.verdict.clone();
    let json = serde_json::to_string(&report).map_err(|e| {
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: encode failed: {e}"));
        e.to_string()
    })?;
    let prompt = status_prompt(&json, &wt_path);

    // ⚠ `exec` is load-bearing. `run_deadline` kills its DIRECT child and only
    // that (one pid, no process group). Without `exec` the direct child is
    // `sh`, and a timed-out claude survives it as an orphan — still burning
    // turns, still holding any MCP servers the profile started. `exec` makes sh
    // replace itself, so the deadline's kill lands on claude.
    //
    // `sh -c` because `ai.cmd` is an already-composed, shell-quoted flag string
    // (profile::claude_launch). Env goes through `Command::env`, NOT inlined as
    // a `K=V ` prefix: that separation is the point of `AiLaunch`.
    let line = format!("exec {} -p {} --max-turns 12", ai.cmd, worktrees_core::tmux::sq(&prompt));
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", &line]).current_dir(&wt_path);
    for (k, v) in &ai.env {
        cmd.env(k, v); // CLAUDE_CONFIG_DIR, when a profile applies
    }
    let out = run_deadline(cmd, 180).map_err(|e| {
        let msg = if e.kind() == std::io::ErrorKind::TimedOut {
            "claude timed out after 180s".to_string()
        } else {
            format!("could not run claude: {e}")
        };
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: {msg}"));
        msg
    })?;

    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let tail = stderr_tail(&out.stderr, 12);
        let rc = out.status.code().unwrap_or(-1);
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: claude rc={rc}: {tail}"));
        return Err(if tail.is_empty() { format!("claude exited {rc}") } else { format!("claude exited {rc}:\n{tail}") });
    }
    // A real failure shape, not a curiosity: claude can exit 0 having printed
    // nothing (a denied tool, an empty turn budget). Caching that would leave a
    // blank "Claude's read" section that looks like a successful answer, and the
    // Re-run button would be the only way to discover it was not one.
    if text.is_empty() {
        let tail = stderr_tail(&out.stderr, 12);
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: claude exited 0 with no output: {tail}"));
        return Err(if tail.is_empty() {
            "claude exited 0 but printed nothing".into()
        } else {
            format!("claude exited 0 but printed nothing:\n{tail}")
        });
    }

    let epoch = sysclock::now_epoch();
    // The declared sidecar, via the same locked + atomic edit every other
    // declared write uses. `extra` round-trips unknown keys by design, so this
    // needs no schema surgery — and it is NOT `ui-state.json`, which the
    // frontend owns whole-blob (a backend write there is erased by the next
    // settings save).
    //
    // ⚠ `last_worked_epoch` is deliberately untouched. That stamp means "claude
    // finished a task HERE", and it drives the nav's afterglow; a report ABOUT a
    // worktree is not work IN it, and stamping it would relight a cold place.
    let stamp = serde_json::json!({ "text": text, "epoch": epoch, "verdict": verdict });
    store::edit(&repo, &slug, |d| {
        d.extra.insert("status_report".into(), stamp);
    })
    .map_err(|e| {
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: store write failed: {e}"));
        e
    })?;
    applog("info", &format!("ai_status_report repo={repo} slug={slug}: {} chars, verdict={verdict}", text.len()));
    Ok(AiStatusReport { text, epoch, verdict })
}

/// Re-apply the file plan (`relink [<wt>|--all] [--force]`). `force` is the
/// documented escape hatch for a shadowing regular file — core writes a `.bak`
/// alongside before it touches one (§7), so the drifted content is never the
/// only casualty.
#[tauri::command]
async fn relink(repo: String, slug: Option<String>, force: bool) -> Result<CmdResult, String> {
    let target = slug.filter(|s| !s.trim().is_empty());
    let mut args: Vec<String> = vec![target.clone().unwrap_or_else(|| "--all".into())];
    if force {
        args.push("--force".into());
    }
    let label = target.as_deref().unwrap_or("--all").to_string();
    run_op(&format!("relink {label}"), &repo, |p, ui| ops::cmd_relink(p, ui, &args))
}

/// Allocate/repair port slots (`provision [<wt>|--all]`). `--reallocate` is
/// deliberately NOT exposed: §6 makes moving a slot under a running stack a
/// refuse-by-default, and a GUI button is the last place that decision belongs.
#[tauri::command]
async fn provision(repo: String, slug: Option<String>) -> Result<CmdResult, String> {
    let target = slug.filter(|s| !s.trim().is_empty());
    let args: Vec<String> = vec![target.clone().unwrap_or_else(|| "--all".into())];
    let label = target.as_deref().unwrap_or("--all").to_string();
    run_op(&format!("provision {label}"), &repo, |p, ui| ops::cmd_provision(p, ui, &args))
}

// ── sync (courier sync of a project through a mounted hub) ───────────────────
// The engine is `worktrees_core::sync`; these three commands are the app's whole
// surface on it. They are `async fn` like every other command here — each one
// shells out to rsync, and a sync handler runs on the main thread and freezes
// the window for the duration (CLAUDE.md).
//
// The hub is deliberately NOT a parameter: it comes from the same chain the CLI
// uses ($WORKTREES_SYNC_HUB → `[sync] hub` → a single mounted volume), so the
// app and the terminal can never disagree about where a project is pushed.

/// `repo` = a project in the workspace; `name` = a project that exists only on
/// the hub (adoption — the machine has no tree for it, so there is no root to
/// pass). Exactly one is expected; `repo` wins if both arrive, since a caller
/// that knows a root is not adopting.
fn sync_request(
    repo: Option<&str>,
    name: Option<&str>,
    direction: &str,
    with_sessions: bool,
) -> Result<sync::SyncRequest, String> {
    let source = match (repo, name) {
        (Some(r), _) => {
            let p = Project::discover(Path::new(r)).map_err(|e| e.msg)?;
            sync::SyncSource::Root(PathBuf::from(&p.main_root))
        }
        (None, Some(n)) => sync::SyncSource::HubName(n.to_string()),
        (None, None) => return Err("sync needs a project: pass repo, or name to import one from the hub".into()),
    };
    Ok(sync::SyncRequest {
        source,
        direction: sync::SyncDirection::parse(direction)?,
        hub: None,
        with_sessions: Some(with_sessions),
    })
}

/// `sync status` for one project, plus whether this checkout IS a hub copy —
/// the menu disables the whole sync group there (pushing FROM a copy sends it
/// back over the original; pulling INTO one is hub→hub).
#[derive(Serialize)]
struct SyncStatusView {
    #[serde(flatten)]
    status: sync::StatusJson,
    hub_copy: Option<String>,
}

/// Cheap enough for a menu-open: one `rsync --version`, one hub resolve, one
/// manifest read, two git calls. No rsync transfer of any kind.
#[tauri::command]
async fn sync_status(repo: String) -> Result<SyncStatusView, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("sync_status repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let root = PathBuf::from(&p.main_root);
    let mut ui = CaptureUi::default();
    let status = sync::sync_status_data(&mut ui, Some(&root), None, true);
    Ok(SyncStatusView { status, hub_copy: sync::hub_copy_refusal(&root) })
}

/// The hub's own status: what has been pushed to this drive, from anywhere.
/// This is the ONE sync command with no project — the import picker calls it on
/// a machine whose workspace may be empty, which is precisely the machine that
/// needs it (`sync status` outside a repo, the CLI's adoption listing).
///
/// The walk is core's: `sync_status_data(from: None)` already lists every
/// manifest on the hub, so nothing here re-implements it. `hub_error` carries
/// the reason there is no hub, because "no projects" and "no drive" are
/// different answers and the picker must not show them the same way.
#[tauri::command]
async fn sync_hub_list() -> Result<sync::StatusJson, String> {
    let mut ui = CaptureUi::default();
    let out = sync::sync_status_data(&mut ui, None, None, true);
    if let Some(e) = &out.hub_error {
        applog("info", &format!("sync_hub_list: no hub ({e})"));
    }
    Ok(out)
}

/// What a sync WOULD do — the modal's whole content. Two rsync dry passes plus
/// (on a pull) the live-session lookup, so it is seconds on a real tree.
///
/// `name` (with no `repo`) is an adoption preview: the destination in the
/// answer comes from the hub manifest, and it is the only thing telling the
/// user where a tree they have never had is about to appear.
#[tauri::command]
async fn sync_preview(
    repo: Option<String>,
    name: Option<String>,
    direction: String,
    with_sessions: bool,
) -> Result<sync::SyncPreviewOut, String> {
    let req = sync_request(repo.as_deref(), name.as_deref(), &direction, with_sessions)?;
    sync::sync_preview(&req).inspect_err(|e| {
        let who = repo.clone().or_else(|| name.clone()).unwrap_or_default();
        applog("warn", &format!("sync_preview {direction} {who}: {e}"));
    })
}

/// Apply it. `confirmed` is the user's answer to the modal's live-session
/// warning — core re-lists the sessions here and refuses again if the answer no
/// longer covers what is running.
///
/// Mapped into `CmdResult` (not `Result::Err`) so the frontend's `runCmd` shows
/// it the way it shows every other op: `needs_confirm` carries the live sessions
/// NEWLINE-separated, structurally, so the modal names them without parsing prose.
///
/// `on_progress` streams rsync's own progress while the transfer runs — the same
/// `Channel` mechanism `term_open` uses for pty bytes. An 8.5G first push is
/// minutes long, and a button that says "Pushing…" for four minutes is
/// indistinguishable from a hung one. Core throttles to ~15 events/sec and
/// always sends the last one, so this end can simply forward.
#[tauri::command]
async fn sync_apply(
    app: AppHandle,
    repo: Option<String>,
    name: Option<String>,
    direction: String,
    with_sessions: bool,
    confirmed: bool,
    install: bool,
    on_progress: Channel<sync::SyncProgress>,
) -> Result<CmdResult, String> {
    let req = sync_request(repo.as_deref(), name.as_deref(), &direction, with_sessions)?;
    let repo = repo.or_else(|| name.clone()).unwrap_or_default();
    let op = format!("sync {direction}");
    // A closed channel (the window went away mid-sync) is not a reason to stop
    // the transfer or to lose the result — but it is not swallowed either: it is
    // logged ONCE, then the sink goes quiet instead of logging per event.
    let mut sink_dead = false;
    let mut sink = |p: sync::SyncProgress| {
        if sink_dead {
            return;
        }
        if let Err(e) = on_progress.send(p) {
            sink_dead = true;
            applog("warn", &format!("{op} progress channel closed repo={repo}: {e}"));
        }
    };
    let outcome = sync::sync_apply(&req, confirmed, install, Some(&mut sink));
    match outcome {
        Err(e) => {
            applog("warn", &format!("{op} rc=1 repo={repo}: {e}"));
            Ok(CmdResult {
                ok: false,
                code: 1,
                output: e,
                slug: None,
                needs_confirm: None,
                warnings: Vec::new(),
            })
        }
        Ok(out) if out.needs_confirm => {
            let live = out.live_sessions.join(", ");
            applog("info", &format!("{op} needs confirm repo={repo}: {live}"));
            Ok(CmdResult {
                ok: false,
                code: worktrees_core::diag::EXIT_NEEDS_CONFIRM,
                output: format!(
                    "{} live tmux session(s) in {}: {live} — a pull mirrors over the tree they are using.",
                    out.live_sessions.len(),
                    out.dst
                ),
                slug: None,
                needs_confirm: Some(out.live_sessions.join("\n")),
                warnings: out.warnings,
            })
        }
        Ok(out) => {
            applog(
                "info",
                &format!(
                    "{op} ok repo={repo}: {} sent/updated, {} deleted → {}",
                    out.plan.sends, out.plan.deletes, out.dst
                ),
            );
            if !out.warnings.is_empty() {
                applog("warn", &format!("{op} warnings repo={repo}: {}", out.warnings.join(" | ")));
            }
            let mut lines = vec![format!(
                "sync {} — {}: {} sent/updated, {} deleted",
                out.direction, out.name, out.plan.sends, out.plan.deletes
            )];
            // An adoption just created a tree this workspace has never seen. The
            // app's whole surface hangs off a project ROW, so a transfer that
            // stopped here would leave the user with files and no way to reach
            // them — the add is part of the import, not a follow-up.
            //
            // It goes through `add_project` itself (the same discover + dedupe +
            // persist the "Add project" button runs), so an import cannot admit
            // something the button would have refused.
            if out.adopting {
                match add_project(app.clone(), out.dst.clone()).await {
                    Ok(_) => lines.push(format!("added to the workspace: {}", out.dst)),
                    Err(e) => {
                        // Half-success is the one outcome that must never read as
                        // either a success or a plain failure: the FILES are
                        // there, and only the bookkeeping failed. Say both, and
                        // say what to do about it.
                        applog(
                            "error",
                            &format!("{op} transferred but add_project failed dst={}: {e}", out.dst),
                        );
                        let _ = app.emit("places:changed", ());
                        return Ok(CmdResult {
                            ok: false,
                            code: 1,
                            output: format!(
                                "{}\nThe transfer finished — the files are at {}.\nAdding it to the workspace failed: {e}\nAdd it by hand with “Add project” and pick that folder.",
                                lines.join("\n"),
                                out.dst
                            ),
                            slug: None,
                            needs_confirm: None,
                            warnings: out.warnings,
                        });
                    }
                }
            }
            // A pull rewrites the tree under every place in it — branches, dirty
            // counts and sessions are all potentially different now. Emitted
            // AFTER the workspace add, so the refresh it triggers is the one
            // that already contains the imported project.
            if req.direction == sync::SyncDirection::Pull {
                let _ = app.emit("places:changed", ());
            }
            Ok(CmdResult {
                ok: true,
                code: 0,
                output: lines.join("\n"),
                slug: None,
                needs_confirm: None,
                warnings: out.warnings,
            })
        }
    }
}

#[derive(Serialize)]
struct SuggestedFile {
    path: String,
    /// The class that fails SILENTLY (§1.2) — flagged louder in the UI.
    credential: bool,
}

/// What `worktrees init` would suggest, WITHOUT writing anything.
#[derive(Serialize)]
struct InitSuggestion {
    /// Where `.worktrees.toml` would be written.
    path: String,
    /// A config is already there (then `qualifies` is irrelevant — nothing is
    /// suggested over an existing file).
    exists: bool,
    /// Anything at all to configure.
    qualifies: bool,
    files: Vec<SuggestedFile>,
    credentials: usize,
    ports: bool,
    compose: bool,
    /// Existing worktrees already missing at least one suggested file.
    stale_places: Vec<String>,
    /// A walk bound was hit, so the suggestion may be incomplete.
    truncated: bool,
    /// The rendered file, comments and all — the sheet shows it before writing.
    toml: String,
    /// §9's dismissal key: a repo that later gains a credential file gets a NEW
    /// hash and correctly re-suggests, which a boolean "dismissed" could never do.
    /// Hashed from a CANONICAL PROJECTION of the suggestion, not from `toml` —
    /// see `suggestion_key` for which differences are deliberately invisible here.
    hash: String,
}

/// A 64-bit string mixer, FNV-1a **in shape only**: the multiplier below is
/// `0x1000_0000_01b3`, which is NOT the FNV-64 prime (`0x100_0000_01b3`). It is
/// deliberately byte-for-byte the same mixer as `init.rs`'s `fnv1a`, so the CLI's
/// once-only marker and the app's dismissal key can never disagree about what
/// "the same suggestion" means. Renamed off `fnv1a_hex` because claiming FNV
/// while using a different constant is how the next reader gets misled.
///
/// Not a security boundary — it only answers "is this the suggestion I already
/// declined?". If core's constant is ever corrected, correct this one in the same
/// change (both are pre-release, so no stored marker survives either way).
fn digest_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{h:016x}")
}

/// The dismissal key's INPUT: a canonical projection of the suggestion, not the
/// rendered TOML.
///
/// Hashing the rendered file made the key sensitive to things that are not a
/// change in what is being suggested. `init::walk` picks the `[compose] file`
/// from a `read_dir`-ordered candidate list, and truncates its hit list at
/// `MAX_HITS`/`MAX_ENTRIES` before sorting — so on a repo with two compose files
/// (or one big enough to truncate) the same checkout can render two different
/// TOMLs. A flapping key resurrects a banner the user already dismissed.
///
/// So: sort the file list, reduce compose to the boolean that actually drives the
/// suggestion, and drop the port numbers (they come from the same heuristic scan).
/// What survives is exactly §9's contract — "a repo that later gains a credential
/// file re-suggests" — and nothing else.
///
/// ⚠ This cannot fix the remaining case: when the walk truncates, the SET of files
/// can differ between runs, and no projection of a wrong set is stable. That fix
/// belongs in `worktrees_core::init::walk` (sort each `read_dir` before the
/// bound is applied).
fn suggestion_key(s: &InitSuggestion) -> String {
    let mut lines: Vec<String> = s
        .files
        .iter()
        .map(|f| format!("file\t{}\t{}", if f.credential { "cred" } else { "plain" }, f.path))
        .collect();
    lines.sort();
    lines.push(format!("ports\t{}", s.ports));
    lines.push(format!("compose\t{}", s.compose));
    lines.push(format!("truncated\t{}", s.truncated));
    digest_hex(&lines.join("\n"))
}

#[tauri::command]
async fn init_suggest(repo: String) -> Result<InitSuggestion, String> {
    use worktrees_core::init;
    let p = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("init_suggest repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let main = Path::new(&p.main_root);
    let path = main.join(worktrees_core::projcfg::CONFIG_FILE);
    let facts = init::probe(main, Path::new(p.wt_root_dir()));
    let sug = init::detect(&facts);
    let qualifies = !sug.is_empty();
    let toml = if qualifies { init::render(&sug) } else { String::new() };
    let mut view = InitSuggestion {
        path: path.to_string_lossy().into_owned(),
        exists: path.exists(),
        qualifies,
        files: sug
            .files
            .iter()
            .map(|c| SuggestedFile { path: c.rel.clone(), credential: c.kind == init::Kind::Credential })
            .collect(),
        credentials: sug.credentials(),
        ports: sug.ports.is_some(),
        compose: sug.compose.is_some(),
        stale_places: sug.stale_places.clone(),
        truncated: sug.truncated,
        hash: String::new(),
        toml,
    };
    // Derived from the projection, never from `view.toml` — see `suggestion_key`.
    view.hash = suggestion_key(&view);
    Ok(view)
}

/// Write the suggested `.worktrees.toml`.
///
/// `-y` carries the consent: `CaptureUi::confirm` always answers NO (a
/// programmatic caller must never have a config appear under it), so without the
/// flag this command could only ever print. The SHEET confirms first — that is
/// where the human says yes. NOT `--force`: an existing config is still refused,
/// loudly, exactly as on the CLI.
#[tauri::command]
async fn init_write(repo: String) -> Result<CmdResult, String> {
    let args: Vec<String> = vec!["-y".into()];
    run_op("init", &repo, |p, ui| ops::cmd_init(p, ui, &args))
}

// ── diagnostics (Settings → Logs → Copy diagnostics) ─────────────────────────
// A single clipboard-ready plaintext block for bug reports. Entirely OFFLINE:
// no check_update / no network — versions come from the compiled-in constant +
// the local CLI probe, environment from the (already fixed-up) PATH, and tool
// versions via short-deadline `which`/`--version` shell-outs.

/// `which <tool>` (resolved path) + first line of `<tool> --version`, each under
/// a short deadline so a wedged tool can't stall the button. Returns the two as
/// display strings ("(not found)" when absent).
fn tool_report(tool: &str) -> (String, String) {
    let mut which = std::process::Command::new("which");
    which.arg(tool);
    let path = match run_deadline(which, 10) {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "(not found)".to_string(),
    };
    let mut ver = std::process::Command::new(tool);
    ver.arg("--version");
    let version = match run_deadline(ver, 10) {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string()
        }
        _ => "(unknown)".to_string(),
    };
    (path, version)
}

/// Assemble the diagnostics block. `async` (a login-shell CLI probe can take
/// ~5s and two ~10s tool probes — must never run on the main thread).
#[tauri::command]
async fn diagnostics(app: AppHandle) -> Result<String, String> {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let (cli_path, cli_version) = match cli_binary() {
        Some((p, v)) => (p, v),
        None => ("(not found)".to_string(), "(unknown)".to_string()),
    };
    let path = std::env::var("PATH").unwrap_or_default();
    let (git_path, git_version) = tool_report("git");
    let (tmux_path, tmux_version) = tool_report("tmux");
    // The documentation viewer's BROWSER BUNDLE, by STAT — never by running
    // anything. "It should be there and it is not" belongs here, on demand,
    // beside the other tools, and NOT on any path the app takes at launch: a
    // probe that decides whether the Docs tab exists would let a missing file
    // change what the app looks like before the user has asked for anything.
    // `tool_report` is the wrong shape for the same reason — it runs
    // `<tool> --version`, and this is a JavaScript file.
    let viewer_path = match viewer::bundle_path(app.path().resource_dir().ok().as_deref()) {
        Some(p) => p.to_string_lossy().into_owned(),
        None => "(no browser bundle — the Docs tab lists and reads without it)".to_string(),
    };

    let ai_cmd = worktrees_core::config::resolve_ai_cmd(None);
    let ai_resume_arg = worktrees_core::config::resolve_ai_resume_arg();
    let cfg_path = worktrees_core::config::config_path();
    let cfg_exists = cfg_path.exists();

    let log_tail = {
        let text = std::fs::read_to_string(log_file()).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        let start = all.len().saturating_sub(200);
        all[start..].join("\n")
    };

    let block = format!(
        "worktrees diagnostics\n\
         =====================\n\
         app version : {app_version}\n\
         cli version : {cli_version}\n\
         cli path    : {cli_path}\n\
         \n\
         PATH        : {path}\n\
         git         : {git_version} @ {git_path}\n\
         tmux        : {tmux_version} @ {tmux_path}\n\
         docs viewer : {viewer_path}\n\
         \n\
         core config\n\
         -----------\n\
         ai_cmd        : {ai_cmd}\n\
         ai_resume_arg : {ai_resume_arg}\n\
         config file   : {cfg} ({exists})\n\
         \n\
         log (last 200 lines)\n\
         --------------------\n\
         {log_tail}\n",
        cfg = cfg_path.to_string_lossy(),
        exists = if cfg_exists { "exists" } else { "absent" },
    );
    Ok(block)
}

/// The https web home of a project's `origin` remote, or None when there is no
/// origin (or it is a local path / exotic protocol). This is the ONLY thing the
/// backend knows about the remote; which PAGE to open — the repo home, or a
/// branch's tree — is decided in the frontend (`remote.ts::remoteWebUrl`) from
/// the snapshot it already holds (`Place.upstream`), so the rule lives once.
/// The UI opens the result via the opener plugin.
#[tauri::command]
async fn remote_url(repo: String) -> Result<Option<String>, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| e.msg)?;
    let Some(remote) = worktrees_core::git::git_out(&p.main_root, &["remote", "get-url", "origin"])
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    Ok(normalize_remote(&remote))
}

/// `git@host:owner/repo(.git)` / `ssh://git@host/…` / `http(s)://host/…` → the
/// https web base; None for exotic remotes (local paths, other protocols).
fn normalize_remote(remote: &str) -> Option<String> {
    let r = remote.trim();
    let r = r.strip_suffix(".git").unwrap_or(r);
    if let Some(rest) = r.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        Some(format!("https://{host}/{path}"))
    } else if let Some(rest) = r.strip_prefix("ssh://git@") {
        // the authority may carry a port (host:2222/owner/repo) — strip it
        let (auth, path) = rest.split_once('/')?;
        let host = auth.split(':').next().unwrap_or(auth);
        Some(format!("https://{host}/{path}"))
    } else if r.starts_with("https://") || r.starts_with("http://") {
        Some(r.to_string())
    } else {
        None
    }
}

/// Open a place in the user's editor (`editor_cmd` from Settings, e.g. `code`).
/// The command is the user's own configured tool — same trust model as ai_cmd.
/// Run through `/bin/sh -c` so quoted/spaced commands work (`open -a "Visual
/// Studio Code"`); the path is passed as a positional arg ($0) so it never needs
/// quoting inside the command string.
#[tauri::command]
async fn open_editor(path: String, cmd: String) -> Result<(), String> {
    if cmd.trim().is_empty() {
        return Err("no editor configured (Settings → Editor command)".into());
    }
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", &format!("{cmd} \"$0\""), &path])
        .spawn()
        .map_err(|e| format!("couldn't launch '{cmd}': {e}"))?;
    // reap in the background — a dropped Child is never waited on (zombie per click)
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Open a place's tmux session in the user's external terminal app
/// (`terminal_cmd` from Settings, e.g. `ghostty -e tmux attach -t {session}`).
/// Every `{session}` token is replaced with the SINGLE-QUOTED session name —
/// session names carry parens (`<prefix>-(main)`) that break unquoted sh — then
/// the whole command runs via `/bin/sh -c`. Same trust model as editor_cmd.
#[tauri::command]
async fn open_terminal(cmd: String, session: String) -> Result<(), String> {
    if cmd.trim().is_empty() {
        return Err("no terminal command configured (Settings → Terminal command)".into());
    }
    let quoted = worktrees_core::tmux::sq(&session);
    let line = cmd.replace("{session}", &quoted);
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", &line])
        .spawn()
        .map_err(|e| format!("couldn't launch '{cmd}': {e}"))?;
    // reap in the background — a dropped Child is never waited on (zombie per click)
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

// ── file browser (right dock, Files tab) ─────────────────────────────────────
// Browse + view + edit the worktree's files. Every path is validated to live
// UNDER a registered project root (worktrees nest under their main root), so the
// UI can never read/write arbitrary files. Unlike git/tmux we DON'T shell these
// FS reads out — they carry no repo semantics — but the listing calls
// `git check-ignore` so the tree respects .gitignore (best-effort: no git / not
// a repo → show everything).

#[derive(Serialize)]
struct FsEntry {
    name: String,
    path: String,
    is_dir: bool,
    /// Only ever true when the caller asked for ignored entries — the filtered
    /// listing has nothing to mark, since everything ignored is already gone.
    ignored: bool,
    /// Symlink target exactly as written — a relative link stays relative,
    /// because that is what is on disk and what the user would `ls -l`. None
    /// for every ordinary entry.
    link: Option<String>,
    /// Why the tree must not follow this link, or None when it may. A reason
    /// code rather than a bool: all three render inert, but "outside the
    /// workspace" is a false statement about a link that merely dangles, and
    /// the row's tooltip is the only place a user learns why nothing happens.
    link_block: Option<&'static str>,
}

#[derive(Serialize)]
struct FileContent {
    content: String,
    truncated: bool,
    binary: bool,
    /// mtime (ms since epoch) — the dock echoes it back on save as a
    /// compare-and-swap token so a stale buffer can't clobber a newer edit.
    mtime: u64,
    /// Full size on disk (NOT the length of `content`, which is capped) — the
    /// viewer header shows it, and it's the only honest number for a file that
    /// came back truncated or binary.
    size: u64,
}

/// An image (or any small blob) as base64, for the viewer's `data:` URI. Kept
/// separate from `read_file` so the text path never pays for the encode.
#[derive(Serialize)]
struct FileBlob {
    b64: String,
    /// Bytes actually encoded (== `size` unless the cap truncated the read).
    size: u64,
    truncated: bool,
    mtime: u64,
}

/// How one path differs from the branch's base, coarsely. Four classes is what
/// a tinted NAME can carry on its own; a fifth would need a second channel.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum ChangeKind {
    Modified,
    Added,
    Untracked,
    Deleted,
}

#[derive(Serialize, PartialEq, Eq, Debug)]
struct FileChange {
    /// Absolute, canonical — the same shape `list_dir` hands the tree, so the
    /// frontend can look a row up by `entry.path` with no normalizing.
    path: String,
    status: ChangeKind,
}

/// Base-branch candidates, most authoritative first — the remote's view before
/// the local one, since `origin/main` is what moves on fetch.
///
/// Shared by `changed_files` and `file_diff` ON PURPOSE, so the two can never
/// resolve different BASES. Verified across eight repo shapes (committed-only,
/// uncommitted-only, base moved on, no origin, no main/master, unrelated
/// histories, rename): the loops always settle on the same candidate, even
/// though one breaks on the first `diff <cand>...HEAD` that succeeds and the
/// other on the first `merge-base` that does — they fail under identical
/// conditions.
///
/// A shared base is NOT a promise the two always agree on a given file, and one
/// case makes that visible: a file committed on the branch and then changed BACK
/// in the working tree. `changed_files` marks it (the worktree differs from
/// HEAD) while `file_diff` vs base correctly finds nothing, so a lit row sits
/// above "no changes". Both are true — the branch touched it, and it now equals
/// the base — and the HEAD toggle shows the revert. The set semantics genuinely
/// differ: commit-range ∪ working-tree status here, merge-base vs worktree there.
const BASE_CANDS: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

/// Everything in one worktree that differs from its branch's base.
#[derive(Serialize)]
struct ChangeSet {
    /// The repo top-level these paths hang off, canonicalized. Returned rather
    /// than assumed: the tree walks ancestors to cascade a marker upward, and it
    /// needs to know where to stop without trusting the `root` it passed in
    /// (a place path can be a symlink; every path here is resolved).
    root: String,
    files: Vec<FileChange>,
}

/// File mtime in ms since the epoch (0 if unavailable) — a cheap change token.
fn file_mtime_ms(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Canonicalize `path` and require it under some registered project root.
/// canonicalize() also means the path must EXIST — we only ever browse/edit
/// files the tree surfaced, never create arbitrary ones.
fn guard_under_projects(app: &AppHandle, path: &str) -> Result<PathBuf, String> {
    let canon = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
    // Lazily, so a match on the first root does not pay to canonicalize the
    // rest — one of which can be a dead network mount that blocks on stat.
    let hit = read_projects(app)
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .any(|r| canon == r || canon.starts_with(&r));
    if hit {
        return Ok(canon);
    }
    Err(format!("path outside workspace: {path}"))
}

/// Registered project roots, canonicalized (unreadable ones simply drop out).
/// Built once per listing, unlike the guard's lazy scan: a directory of links
/// would otherwise re-read the projects file per entry.
fn project_roots(app: &AppHandle) -> Vec<PathBuf> {
    read_projects(app).iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect()
}

/// The containment test `guard_under_projects` applies, against an ALREADY
/// canonical path. Split out so the tree can ask the same question about a
/// symlink target without going through a command that would reject it.
fn under_roots(roots: &[PathBuf], canon: &Path) -> bool {
    roots.iter().any(|r| canon == r || canon.starts_with(r))
}

/// One symlink as the tree needs it: `(is_dir, target, why_not_to_follow)`.
///
/// `is_dir` follows the link, because the shape a user means by `_tmp/` is the
/// target's. `target` is what the link literally says — a relative link stays
/// relative, matching `ls -l`.
///
/// The block reason is the guard's own question asked ahead of time, plus the
/// one thing the guard does not ask: `.git`. The listing drops it by NAME, so
/// without this a repo shipping `ln -s .git g` would hand the tree a browsable
/// caret straight into it — the whole point of not following links is that a
/// repo's own contents do not get to choose what the app opens.
fn classify_symlink(p: &Path, roots: &[PathBuf]) -> (bool, Option<String>, Option<&'static str>) {
    let is_dir = std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false);
    let target = std::fs::read_link(p).map(|t| t.to_string_lossy().to_string()).ok();
    let block = match std::fs::canonicalize(p) {
        // Dangling. Every command behind the row canonicalizes too, so it is
        // just as inert as one pointing away — for a different reason.
        Err(_) => Some("missing"),
        Ok(c) if !under_roots(roots, &c) => Some("outside"),
        Ok(c) if c.components().any(|s| s.as_os_str() == ".git") => Some("git"),
        Ok(_) => None,
    };
    (is_dir, target, block)
}

/// Immediate children of `path` (one level; the tree lazy-expands). Dirs first,
/// then case-insensitive by name.
///
/// `.git` is always dropped. Gitignored entries are dropped too UNLESS
/// `show_ignored` asks for them, in which case they are returned flagged rather
/// than filtered — the tree dims them. The toggle exists because the files a
/// session actually produces (build output, and this repo's own gitignored
/// working notes) are exactly the ones the filtered listing hides.
///
/// Symlinks are stat'd THROUGH for their shape (so a link to a directory reads
/// as one) and carry their target plus `link_block`, the reason the tree must
/// not follow them. Following is left to the tree, which does not: the guard
/// canonicalizes, so a link out of the workspace is unlistable by construction,
/// and honouring one would let a repo's own contents choose what the app reads.
#[tauri::command]
async fn list_dir(app: AppHandle, path: String, show_ignored: Option<bool>) -> Result<Vec<FsEntry>, String> {
    let dir = guard_under_projects(&app, &path)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let mut entries: Vec<FsEntry> = Vec::new();
    // Built on the first symlink seen, not up front: most directories have none
    // and would pay a projects-file read plus a canonicalize per root for it.
    let mut roots: Option<Vec<PathBuf>> = None;
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        let ft = e.file_type().ok();
        let p = e.path();
        // read_dir's file_type is an lstat: it answers about the LINK, so a
        // symlink to a directory came back `is_dir: false` — a file glyph, no
        // caret, and a click that tried to open it as a file. Stat the target
        // for the shape; a broken link has none and stays a file.
        let (is_dir, link, link_block) = match ft {
            Some(t) if t.is_symlink() => {
                classify_symlink(&p, roots.get_or_insert_with(|| project_roots(&app)))
            }
            other => (other.map(|t| t.is_dir()).unwrap_or(false), None, None),
        };
        let path = p.to_string_lossy().to_string();
        entries.push(FsEntry { name, path, is_dir, ignored: false, link, link_block });
    }
    // One `git check-ignore --stdin` batch for the whole directory (NUL-safe).
    let ignored = git_check_ignore(&dir, entries.iter().map(|e| e.path.as_str()));
    if show_ignored.unwrap_or(false) {
        for e in &mut entries {
            e.ignored = ignored.contains(&e.path);
        }
    } else {
        entries.retain(|e| !ignored.contains(&e.path));
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Paths git would ignore (best-effort). `-z` in/out keeps it robust to spaces
/// and newlines in filenames. Empty set on any git failure → nothing filtered.
fn git_check_ignore<'a>(dir: &Path, paths: impl Iterator<Item = &'a str>) -> std::collections::HashSet<String> {
    use std::process::{Command, Stdio};
    let mut stdin_buf = Vec::new();
    for p in paths {
        stdin_buf.extend_from_slice(p.as_bytes());
        stdin_buf.push(0);
    }
    if stdin_buf.is_empty() {
        return Default::default();
    }
    let child = Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(_) => return Default::default(),
    };
    // Write stdin on its own thread while the parent drains stdout below — a big
    // directory's path list can exceed the pipe buffer, and writing all of it
    // before reading would deadlock (git blocks on stdout, we block on stdin).
    if let Some(mut si) = child.stdin.take() {
        thread::spawn(move || {
            let _ = si.write_all(&stdin_buf);
            // drop closes the pipe → git sees EOF
        });
    }
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return Default::default(),
    };
    out.stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect()
}

/// Every path under `root` that differs from the branch's BASE — committed on
/// this branch *and* uncommitted (staged, unstaged, untracked). The Files tab
/// tints those rows and cascades the mark up through their directories.
///
/// THREE spawns per call, and the call is per REFRESH, not per directory: the
/// frontend derives a set once and answers every row from it. Doing this the way
/// `list_dir` does `check-ignore` — a batch per listing — would be one git
/// process per open node per poll tick, and the tree re-lists every open node.
///
/// A path that is not a repo (or has no git) gets an empty set, not an error:
/// nothing differs from a base that does not exist, and the tree must still list.
#[tauri::command]
async fn changed_files(app: AppHandle, root: String) -> Result<ChangeSet, String> {
    let dir = guard_under_projects(&app, &root)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    let cwd = dir.to_string_lossy().to_string();
    // Porcelain paths are relative to the repo TOP-LEVEL, never to `-C`. For a
    // place that IS the worktree root they agree; resolving it anyway is what
    // keeps a root one level down from marking the wrong rows.
    let top = match git::git_out(&cwd, &["rev-parse", "--show-toplevel"]) {
        Some(t) if !t.is_empty() => std::fs::canonicalize(&t).unwrap_or_else(|_| PathBuf::from(t)),
        _ => return Ok(ChangeSet { root: cwd, files: Vec::new() }),
    };
    let mut map: BTreeMap<String, ChangeKind> = BTreeMap::new();
    // Committed on this branch. THREE dots — the diff is against the merge base,
    // so a base branch that has moved on since doesn't light up every file those
    // other commits touched. Candidates follow core's `base_ref()` precedence
    // (the remote's view first, since `origin/main` is what moves on fetch); the
    // first range git can resolve wins, and a repo with none of them — an unborn
    // HEAD, a repo with no main/master — just gets the uncommitted half.
    for cand in BASE_CANDS {
        if let Some(out) = git::git_out(&cwd, &["diff", "--name-status", "-z", &format!("{cand}...HEAD")]) {
            for (p, k) in parse_name_status_z(&out) {
                map.insert(p, k);
            }
            break;
        }
    }
    // Uncommitted. `--untracked-files=all` is not optional: the default
    // `-unormal` collapses a whole new directory into one `dir/` record, which
    // would mark the directory and leave every file inside it unmarked.
    //
    // Applied SECOND so it wins on collision — it describes the disk as it is
    // now. A file added in a commit and then deleted has to read `deleted`
    // (there is no row to tint, only a ghost to draw), not `added`.
    if let Some(out) = git::git_out(&cwd, &["status", "--porcelain", "-z", "--untracked-files=all"]) {
        for (p, k) in parse_status_z(&out) {
            map.insert(p, k);
        }
    }
    let files = map
        .into_iter()
        .map(|(rel, status)| FileChange { path: top.join(&rel).to_string_lossy().to_string(), status })
        .collect();
    Ok(ChangeSet { root: top.to_string_lossy().to_string(), files })
}

/// `git status --porcelain -z --untracked-files=all` → (repo-relative, kind).
///
/// `-z` records are NUL-terminated `XY <path>`, and a rename/copy puts the
/// ORIGINAL path in the NEXT record instead of after a ` -> ` — which is the
/// whole reason for `-z`: no C-quoting, and no ambiguity with a filename that
/// contains an arrow. A rename's original is GONE from disk, so it is reported
/// deleted; that is what gives the tree a ghost row to draw for it.
fn parse_status_z(out: &str) -> Vec<(String, ChangeKind)> {
    let mut it = out.split('\0').filter(|r| !r.is_empty());
    let mut v = Vec::new();
    while let Some(rec) = it.next() {
        // "XY path": two status columns, a space, then at least one path byte.
        if rec.len() < 4 {
            continue;
        }
        let (x, y) = (rec.as_bytes()[0], rec.as_bytes()[1]);
        // Byte 3 is a char boundary — the two columns and the space are ASCII.
        let path = rec[3..].to_string();
        match (x, y) {
            (b'?', _) => v.push((path, ChangeKind::Untracked)),
            (b'!', _) => {} // ignored — only ever emitted with --ignored, never asked for
            (b'R', _) | (b'C', _) => {
                let orig = it.next().unwrap_or("").to_string();
                v.push((path, ChangeKind::Added));
                // A copy leaves its source where it was; a rename does not.
                if x == b'R' && !orig.is_empty() {
                    v.push((orig, ChangeKind::Deleted));
                }
            }
            // Before the `A` arm on purpose: `AD` is staged-added then removed
            // from the worktree, and the row that would carry `added` is gone.
            (b'D', _) | (_, b'D') => v.push((path, ChangeKind::Deleted)),
            (b'A', _) => v.push((path, ChangeKind::Added)),
            // M, T, U and anything a future git adds: it differs, which is all
            // four classes are for.
            _ => v.push((path, ChangeKind::Modified)),
        }
    }
    v
}

/// `git diff --name-status -z <range>` → (repo-relative, kind). Fields are
/// NUL-separated, a status token then its path — except `R…`/`C…`, which are
/// followed by TWO paths (old, then new).
fn parse_name_status_z(out: &str) -> Vec<(String, ChangeKind)> {
    let mut it = out.split('\0').filter(|r| !r.is_empty());
    let mut v = Vec::new();
    while let Some(tok) = it.next() {
        let letter = tok.as_bytes()[0];
        match letter {
            b'R' | b'C' => {
                let old = it.next().unwrap_or("").to_string();
                let new = it.next().unwrap_or("").to_string();
                if !new.is_empty() {
                    v.push((new, ChangeKind::Added));
                }
                if letter == b'R' && !old.is_empty() {
                    v.push((old, ChangeKind::Deleted));
                }
            }
            _ => {
                // A trailing status with no path means truncated output; stop
                // rather than pairing it with the next record's status token.
                let Some(path) = it.next() else { break };
                let kind = match letter {
                    b'A' => ChangeKind::Added,
                    b'D' => ChangeKind::Deleted,
                    _ => ChangeKind::Modified,
                };
                v.push((path.to_string(), kind));
            }
        }
    }
    v
}

/// One file's diff, as the viewer's two-column renderer consumes it.
#[derive(Serialize)]
struct FileDiff {
    /// Echo of what was asked for ("base" | "head").
    ///
    /// Informational only — the toggle's race is handled on the frontend by the
    /// fetch effect's `alive` flag, which cleanup trips when `diffBase` changes,
    /// so a stale response is dropped before it can be stored. This field is NOT
    /// what does that; it is here so a logged or inspected response says which
    /// question it answered.
    against: String,
    /// The ref the base resolved to, for the header. "vs base" without saying
    /// WHICH base is a claim the reader has to take on faith, and the answer
    /// differs per repo (`origin/main`, `master`, or nothing at all).
    base_label: String,
    /// git's unified diff at full context. EMPTY means the file is identical to
    /// the base — deliberately distinct from a patch that parses to no rows.
    patch: String,
    /// There is no BEFORE for this path, so every line of it is new: either git
    /// has never seen it, or the repo has no commits yet (an unborn HEAD, where
    /// a staged file is tracked but has no committed version to diff against).
    /// No git output at all in that case — the frontend builds all-added rows
    /// from the content it already read. Spawning `git diff --no-index
    /// /dev/null` would be a second process for something we derive for free.
    untracked: bool,
    binary: bool,
    /// The patch hit the byte cap below and is cut short at a line boundary.
    truncated: bool,
}

/// Byte cap on a patch. Full context means the payload is roughly BOTH copies of
/// the file, so this is the pair-wise twin of `read_file`'s 1 MiB — and it
/// crosses the IPC bridge as one string.
const DIFF_MAX: usize = 2_000_000;

/// Cut `patch` to at most `DIFF_MAX` bytes, at a LINE boundary. Mid-line would
/// hand the parser half a line of source and it would render as real content.
fn trim_patch(patch: &str) -> (String, bool) {
    if patch.len() <= DIFF_MAX {
        return (patch.to_string(), false);
    }
    let mut cut = DIFF_MAX;
    while cut > 0 && !patch.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &patch[..cut];
    // No newline in the first 2 MB means one enormous line; there is no boundary
    // to fall back to, so the char boundary is the best available cut.
    let end = head.rfind('\n').map(|i| i + 1).unwrap_or(cut);
    (patch[..end].to_string(), true)
}

/// Does this patch say "binary" rather than showing lines?
fn patch_is_binary(patch: &str) -> bool {
    patch.lines().any(|l| l.starts_with("Binary files ") || l == "GIT binary patch")
}

/// One file's diff against its branch's base (or against HEAD).
///
/// `--unified=1000000` is the whole design: full context means ONE payload
/// carries both complete versions of the file *and* their alignment, so the
/// frontend needs no diff algorithm and can syntax-highlight each side as a
/// whole file. Hunk-sized context would mis-colour every line whose block
/// comment or string opened above the hunk.
///
/// A path outside a repo, or with no git, returns an empty diff rather than an
/// error: the tree listed the file perfectly well, and a banner over it would
/// be reporting the absence of a feature as a failure.
#[tauri::command]
async fn file_diff(app: AppHandle, path: String, against: Option<String>) -> Result<FileDiff, String> {
    let f = guard_under_projects(&app, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    file_diff_for(&f, against.as_deref() == Some("head"))
}

/// The git half of `file_diff`, split out from the command so it can be tested
/// against real repos: the command itself needs an `AppHandle` for the path
/// guard, which a unit test has no way to build. Both of the states worth
/// pinning here — an unborn HEAD, and a git invocation that genuinely fails —
/// used to render as "no changes", so they get tests rather than trust.
fn file_diff_for(f: &Path, want_head: bool) -> Result<FileDiff, String> {
    let against = if want_head { "head" } else { "base" }.to_string();
    let none = |label: &str, untracked: bool| FileDiff {
        against: against.clone(),
        base_label: label.to_string(),
        patch: String::new(),
        untracked,
        binary: false,
        truncated: false,
    };
    let dir = f.parent().unwrap_or(f).to_string_lossy().to_string();
    let top = match git::git_out(&dir, &["rev-parse", "--show-toplevel"]) {
        Some(t) if !t.is_empty() => std::fs::canonicalize(&t).unwrap_or_else(|_| PathBuf::from(t)),
        _ => return Ok(none("", false)),
    };
    let cwd = top.to_string_lossy().to_string();
    // Relative to the repo TOP, because that is where `-C` points below. A place
    // one level down from the worktree root would otherwise ask git about a path
    // that does not exist and get an empty diff for a file that clearly differs.
    let rel = f.strip_prefix(&top).unwrap_or(f).to_string_lossy().to_string();

    if !git::git_ok(&cwd, &["ls-files", "--error-unmatch", "--", &rel]) {
        // No label: there is no ref to name, and the viewer says "new file"
        // rather than trying to phrase the absence of one.
        return Ok(none("", true));
    }
    // An unborn HEAD is the other way to have no BEFORE, and it does not look
    // like one: a `git add`ed file in a fresh repo IS tracked, so the check
    // above passes, every `merge-base` then fails, the fallback lands on `HEAD`
    // — and `git diff HEAD` exits 128 on `fatal: bad revision 'HEAD'`. That used
    // to arrive as an empty patch and render as "no changes vs HEAD" for a file
    // the tree had just tinted `added`. Every line of it is new; say so.
    if !git::has_commits(&cwd) {
        return Ok(none("", true));
    }

    // `merge-base`, not the `cand...HEAD` range syntax `changed_files` uses:
    // that range compares two COMMITS, and what the viewer needs is the base
    // against the WORKING TREE, so the uncommitted half shows up too. Same
    // candidates, same precedence, so the two always agree on which base.
    let (base_rev, base_label) = if want_head {
        ("HEAD".to_string(), "HEAD".to_string())
    } else {
        BASE_CANDS
            .iter()
            .find_map(|cand| {
                git::git_out(&cwd, &["merge-base", cand, "HEAD"])
                    .filter(|s| !s.is_empty())
                    .map(|sha| (sha, (*cand).to_string()))
            })
            // No main/master anywhere: HEAD is the only base there is, and
            // saying so beats silently diffing against something else.
            .unwrap_or_else(|| ("HEAD".to_string(), "HEAD".to_string()))
    };

    // `None` is a NON-ZERO EXIT, and it must not become an empty patch: empty is
    // documented and rendered as "identical to the base", so collapsing the two
    // turns any git failure into a confident, wrong "no changes". `git diff`
    // without `--exit-code` returns 0 whether or not there are differences, so a
    // non-zero exit here is always a real failure and belongs in a banner.
    let patch = git::git_out(
        &cwd,
        &["diff", "--no-ext-diff", "--no-color", "--unified=1000000", &base_rev, "--", &rel],
    )
    .ok_or_else(|| format!("could not diff {rel} against {base_label}"))?;
    if patch_is_binary(&patch) {
        return Ok(FileDiff { against, base_label, patch: String::new(), untracked: false, binary: true, truncated: false });
    }
    let (patch, truncated) = trim_patch(&patch);
    Ok(FileDiff { against, base_label, patch, untracked: false, binary: false, truncated })
}

/// The Docs tab's index for one place, plus the ref its staleness is measured
/// against.
///
/// `base` is NOT `Place::upstream`, and the difference is the whole reason this
/// field exists. Core measures `behind` against the project's BASE ref
/// (`project.rs`: `rev-list --left-right --count <base_ref>...HEAD`, where
/// `base_ref()` is `origin/main` when a fetch has brought one, else the local
/// base) — deliberately, because "how far from main" is the question, and
/// upstream told a different story on any branch that had merged main in. The
/// `upstream` field is informational and on a feature branch it names
/// `origin/<that branch>`. So a header reading "25 behind {upstream}" would
/// print the wrong ref on every pushed branch — a silently-wrong staleness
/// signal, in the one surface built to stop exactly that.
#[derive(Serialize)]
struct DocsIndex {
    /// e.g. `origin/main`. Named in the header beside `behind`, because a bare
    /// "25 behind" is the same silent error one level up.
    base: String,
    /// `.worktrees.toml` in THIS PLACE did not parse. The index still lists, by
    /// convention, and the pane says so — a broken config must never be a blank
    /// Docs tab, which is the "never a dead server for the whole place" rule
    /// (`ViewErrorBoundary`) applied one level up.
    config_error: Option<String>,
    entries: Vec<worktrees_core::docs::DocEntry>,
    truncated: bool,
}

/// Walk one place for its documentation (`worktrees_core::docs`).
///
/// `async fn` like every other handler — a sync one runs on the main thread,
/// and this reads up to 2,000 files' heads plus two `show-ref`s.
///
/// The walk itself refuses symlinks and `.worktrees`/`.git`/`node_modules`/
/// `target`/`dist`; this is the other half of that boundary, the same
/// `guard_under_projects` contract `list_dir` and `read_file` are held to. A
/// path outside every registered project is refused before anything is read.
///
/// A project that cannot be discovered is not a failure: the index still
/// lists, with an empty `base`, and the header simply says "N behind" without
/// naming a ref. Losing the ref name is survivable; losing the index is not.
///
/// ⚠ **`[docs]` is read from THIS PLACE, not from the main worktree** — the one
/// consumer of `projcfg` that does, and the divergence is deliberate.
/// `project_prefix`, materialize, `[ports]` and `[compose]` all read
/// `main_root` because they describe the PROJECT: one prefix, one port stride,
/// one compose file list, whatever branch you are standing on. `[docs]`
/// describes *content that exists on a branch*, and the Docs tab's whole
/// premise is that content differs per place.
///
/// Read main's instead and the feature defeats itself. A restructure that lands
/// `[docs] paths = ["handbook"]` on main would, on the seven of eleven places
/// that have not rebased, declare a directory their branch does not have — and
/// every one of them would show an EMPTY index while their `docs/` sat there
/// unlisted. A place whose branch predates the key gets the convention, which
/// is what its tree actually looks like. Both states correct for their branch,
/// which is the sentence this whole feature exists to make true.
///
/// It grants a branch no power it did not have: `Docs` is two `RelPath` fields
/// (Layer A already applied), the walk re-checks containment (Layer B), and the
/// place directory itself was guarded above. The worst a hostile branch can do
/// is point the listing at a different directory inside its own worktree.
#[tauri::command]
async fn list_docs(app: AppHandle, repo: String, root: String) -> Result<DocsIndex, String> {
    let dir = guard_under_projects(&app, &root)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    let base = Project::discover(Path::new(&repo)).map(|p| p.base_ref()).unwrap_or_default();
    // A config that does not parse falls back to the convention and SAYS so.
    // `load` already honours WORKTREES_NO_PROJECT_CONFIG, so the audit switch
    // turns `[docs]` off with the rest of the project rung — no new code path.
    let (cfg, config_error) = match worktrees_core::projcfg::load(&dir) {
        Ok((c, _findings)) => (c, None),
        Err(e) => {
            applog("warn", &format!("list_docs root={root}: {e}"));
            (None, Some(e.to_string()))
        }
    };
    let idx = worktrees_core::docs::index_with(&dir, cfg.as_ref().and_then(|c| c.docs.as_ref()));
    Ok(DocsIndex { base, config_error, entries: idx.entries, truncated: idx.truncated })
}

/// The Plan dock tab: the place's planning-with-files summary (goal, phases,
/// progress, errors) plus its brief. `worktrees_core::plan::summarize` never
/// fails — an absent or unreadable plan is `source: "none"` — so the only
/// error here is the guard. Read-only: the session owns these files.
#[tauri::command]
async fn place_plan(app: AppHandle, root: String) -> Result<worktrees_core::plan::PlanSummary, String> {
    let dir = guard_under_projects(&app, &root)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    Ok(worktrees_core::plan::summarize(&dir))
}

/// The Plan tab's "Generate plan": paste `ops::PLAN_PROMPT` into the place's
/// Claude session and leave it there for the user to send.
///
/// The app still writes no plan — the SESSION does, when the user presses
/// Enter on a prompt they can read first. That is why this is a paste and not
/// a `send-keys … Enter`, and why `paste_to_ai` has no trailing newline. The
/// text is the fixed constant; nothing from the frontend reaches the pane
/// except which session to put it in.
///
/// `session` rather than a slug for the same reason as `drop_reference`: the
/// place may be on an ADOPTED session whose name is not the canonical one, and
/// the frontend already holds the real name from `ls`.
#[tauri::command]
async fn plan_prompt(session: String, provider: Option<String>) -> Result<(), String> {
    // Addressed by the AI's pane, not by an index (`tmux::ai_pane`), and an
    // honest error when no Claude is there rather than a paste onto a shell.
    let ai_word = provider.as_deref().unwrap_or("claude");
    if ai_word != "claude" && ai_word != "codex" { return Err("unknown agent provider".into()); }
    tmux::paste_to_ai(&session, &ai_word, ops::PLAN_PROMPT)?;
    applog("info", &format!("plan_prompt: pasted into {session}"));
    Ok(())
}

/// The staleness facts, as the frontend already holds them in `Place`.
///
/// Passed in rather than recomputed. Every field is in the `Place` the dock is
/// rendering from at the moment the button is pressed, and re-deriving them here
/// would mean a second git fan-out per click — for numbers that would then be
/// allowed to DISAGREE with the ones on screen two inches away. `base` is the
/// exception and stays ours (`Project::base_ref`), because it is the one fact
/// `Place` does not carry and the one the header gets wrong by guessing
/// (§11.4: it is the base ref, never `upstream`).
#[derive(Deserialize)]
struct ViewerPlace {
    branch: Option<String>,
    behind: Option<i64>,
    dirty: Option<bool>,
    dirty_files: Option<u32>,
    last_commit_subject: Option<String>,
    last_commit_epoch: Option<i64>,
}

/// Open one place's documentation in the user's real browser.
///
/// Returns a URL for the frontend to hand to `openUrl` — the same plugin path
/// `FilesPane` already uses for an `https:` link in a document. Nothing is
/// opened from here: the app stays a control surface, and the one thing this
/// command owns is deciding whether there is a URL worth opening at all.
///
/// `async fn` like every other handler, and for a harder reason than most: it
/// binds a port, walks the place, and writes a few hundred files. On the main
/// thread that is a frozen window. It is also what puts this call inside a
/// tokio runtime, which is what `docserver::start` needs in order to spawn its
/// accept loop.
///
/// **Nothing about this runs at launch.** A missing browser bundle surfaces
/// here, on the click, as one failed invoke — the Docs tab lists and reads
/// every document with no server at all, which is what phases 1 and 2 shipped.
/// The frontend's optimism (`tmuxOk`'s shape) is the other half of that rule.
///
/// The tree it serves is DERIVED, never the repo's own files: a viewer that
/// edits what it is viewing is not a viewer, and the `click` directives that
/// make a diagram navigable have to bake in a port and a token that do not
/// exist until this function has run (§5.2's answer, "A′").
#[tauri::command]
async fn open_docs_viewer(
    app: AppHandle,
    v: State<'_, viewer::Viewer>,
    repo: String,
    root: String,
    slug: String,
    path: Option<String>,
    place: ViewerPlace,
) -> Result<String, String> {
    let dir = guard_under_projects(&app, &root)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    // The document, if one was named, is guarded on its own terms rather than
    // trusted because the place was: `path` is frontend-supplied, and the index
    // lookup below happens against a walk that has not run yet.
    let want = match path.as_deref() {
        None => None,
        Some(p) => Some(guard_under_projects(&app, p)?.to_string_lossy().into_owned()),
    };
    let base = Project::discover(Path::new(&repo)).map(|p| p.base_ref()).unwrap_or_default();
    let (cfg, config_error) = match worktrees_core::projcfg::load(&dir) {
        Ok((c, _findings)) => (c, None),
        Err(e) => (None, Some(e.to_string())),
    };
    if let Some(e) = &config_error {
        applog("warn", &format!("open_docs_viewer root={root}: {e}"));
    }
    let docs_cfg = cfg.as_ref().and_then(|c| c.docs.as_ref()).cloned();
    // BEFORE the index walk, deliberately: the tick compares this digest against
    // a later one, and a document written between the two walks has to read as a
    // change rather than as something already derived. `viewer::Request::
    // fingerprint` has the long version.
    let fingerprint = worktrees_core::docs::fingerprint_with(&dir, docs_cfg.as_ref());
    let idx = worktrees_core::docs::index_with(&dir, docs_cfg.as_ref());

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    // `resource_dir` is absent in `tauri dev` (there is no bundle), which is
    // what `WORKTREES_VIEWER_BIN` exists for.
    let resource_dir = app.path().resource_dir().ok();
    let req = viewer::Request {
        root: &dir,
        slug: &slug,
        path: want.as_deref(),
        stale: worktrees_core::derive::Staleness {
            place: slug.clone(),
            branch: place.branch,
            behind: place.behind,
            base,
            dirty: place.dirty,
            dirty_files: place.dirty_files,
            last_commit_subject: place.last_commit_subject,
            last_commit_epoch: place.last_commit_epoch,
            // The transform may not read the clock; this is not the transform.
            //
            // The same instant twice, and they mean different things: the facts
            // above were captured NOW (the frontend read them off the `Place` it
            // is rendering), and this copy is being written NOW. The tick moves
            // the second one alone, which is what makes the two separable at
            // all — see `Staleness::derived_epoch`.
            now_epoch: sysclock::now_epoch(),
            derived_epoch: sysclock::now_epoch(),
        },
        docs: docs_cfg,
        fingerprint,
    };
    match viewer::open(&v, &config_dir, resource_dir.as_deref(), &idx, &req) {
        Ok(opened) => {
            applog("info", &format!("open_docs_viewer slug={slug} entries={}", idx.entries.len()));
            // An image that was referenced and deliberately not copied is a
            // broken image in the reader's browser with no other explanation
            // anywhere — the open succeeded, so there is no error to carry it.
            for n in &opened.notes {
                applog("warn", &format!("open_docs_viewer slug={slug}: {n}"));
            }
            Ok(opened.url)
        }
        Err(e) => {
            // Never swallowed: the viewer is the one part of this feature the
            // user cannot see failing anywhere else.
            applog("error", &format!("open_docs_viewer slug={slug} root={root}: {e}"));
            Err(e)
        }
    }
}

/// File contents for the viewer. Capped (default 1 MiB) and binary-guarded
/// (a NUL byte in the read slice → `binary: true`, empty content). The frontend
/// shows a "binary / open in editor" placeholder instead of garbage.
#[tauri::command]
async fn read_file(app: AppHandle, path: String, max_bytes: Option<u64>) -> Result<FileContent, String> {
    let f = guard_under_projects(&app, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    // `max_bytes` is frontend-controlled: clamp it. An unclamped `cap + 1`
    // overflows on u64::MAX — panicking in a debug build, and in release
    // wrapping to 0, which would report a non-empty file as empty and NOT
    // truncated. Saturating keeps the failure mode "reads everything".
    let cap = max_bytes.unwrap_or(1_000_000).min(u64::MAX - 1);
    // Bounded read: `take(cap+1)` never allocates more than the cap even for a
    // multi-GB file the user clicks by accident (video, core dump, tarball).
    let file = std::fs::File::open(&f).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > cap;
    let slice = &bytes[..bytes.len().min(cap as usize)];
    let mtime = file_mtime_ms(&f);
    let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(bytes.len() as u64);
    if slice.contains(&0) {
        return Ok(FileContent { content: String::new(), truncated, binary: true, mtime, size });
    }
    Ok(FileContent {
        content: String::from_utf8_lossy(slice).to_string(),
        truncated,
        binary: false,
        mtime,
        size,
    })
}

/// Is `path` still a readable file inside the workspace? Answers the question
/// `files_open` restore has to ask before it reopens a remembered path.
///
/// **Returns `Ok(false)` where every other FS command returns `Err`** — that is
/// the whole point of it existing rather than the frontend calling `read_file`
/// and catching. A remembered file can be deleted, renamed, gitignored or left
/// behind by a branch switch between visits, and `FileView` routes a failed
/// read to `onError`, i.e. the app's error banner. Restoring through a command
/// that *errors* would therefore greet you with a banner for the entirely
/// ordinary act of deleting a file you once had open — which is exactly why
/// `PlacePanels` refused to remember the open file at all until this existed.
///
/// So: outside the workspace, missing, or not-a-file all collapse to `false`.
/// The `Err` arm is left in the signature for an IPC-level failure only; the
/// guard's own rejection is deliberately swallowed.
#[tauri::command]
async fn file_readable(app: AppHandle, path: String) -> Result<bool, String> {
    Ok(guard_under_projects(&app, &path).map(|f| f.is_file()).unwrap_or(false))
}

/// Raw bytes as base64 — the viewer builds a `data:` URI from it to show an
/// image inline. Same path guard as every other FS command. The cap is smaller
/// than `read_file`'s (base64 inflates 4/3, and this crosses the IPC bridge as
/// one string): a bigger image reports `truncated` and the viewer refuses to
/// render a half-decoded file rather than showing a corrupt one.
#[tauri::command]
async fn read_file_base64(app: AppHandle, path: String, max_bytes: Option<u64>) -> Result<FileBlob, String> {
    let f = guard_under_projects(&app, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    // 4 MiB: base64 inflates 4/3 and the result crosses the IPC bridge as one
    // string, so a markdown doc full of images can hold several of these at
    // once. Clamped for the same overflow reason as `read_file`.
    let cap = max_bytes.unwrap_or(4_000_000).min(u64::MAX - 1);
    let file = std::fs::File::open(&f).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > cap;
    if truncated {
        bytes.truncate(cap as usize);
    }
    Ok(FileBlob {
        b64: b64_encode(&bytes),
        size: bytes.len() as u64,
        truncated,
        mtime: file_mtime_ms(&f),
    })
}

/// Standard base64 (RFC 4648, padded). Hand-rolled: the app pulls in no base64
/// crate for what is one table and a three-byte loop.
fn b64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Save an edit. The file must already exist (guard canonicalizes) — the dock
/// edits files it surfaced, it doesn't create new ones. `expected_mtime` is a
/// compare-and-swap guard: if the file changed on disk since the dock read it
/// (Claude edited it in another pane), the save is refused rather than silently
/// clobbering. The write is atomic (temp file + rename) so a crash mid-save
/// can't leave a half-written file, and preserves the file's mode bits.
///
/// Returns the SAVED file's mtime, which is the `expected_mtime` for the next
/// save in the same sitting. Without it the editor would still hold the mtime
/// it read before this write, and its second save would be refused as a
/// conflict with its own first one — the guard firing on the one writer it is
/// not there to stop.
#[tauri::command]
async fn write_file(app: AppHandle, path: String, content: String, expected_mtime: Option<u64>) -> Result<u64, String> {
    let f = guard_under_projects(&app, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    if let Some(exp) = expected_mtime {
        if file_mtime_ms(&f) != exp {
            return Err("file changed on disk since you opened it — reload to see the latest".into());
        }
    }
    let dir = f.parent().ok_or("no parent directory")?;
    let base = f.file_name().and_then(|n| n.to_str()).unwrap_or("edit");
    let tmp = dir.join(format!(".{base}.wt-tmp"));
    std::fs::write(&tmp, content.as_bytes()).map_err(|e| e.to_string())?;
    if let Ok(meta) = std::fs::metadata(&f) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions()); // keep the exec bit etc.
    }
    std::fs::rename(&tmp, &f).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(file_mtime_ms(&f))
}

// ── dock terminal: scratch-shell sidecar sessions ────────────────────────────
// The single-pane place session (Claude only) pairs with one or more SIDECAR
// tmux sessions for the dock's Terminal tab — bare keep-alive shells cwd'd in
// the worktree, one per shell tab: `<session>~term`, `<session>~term~2`, … Each
// is persistent + `tmux attach`-able like any place session, and excluded from
// worktree adoption (tmux::session_in) so none can masquerade as the AI
// session. Torn down with the place — core cmd_close/cmd_rm sweep them.

/// A place's CANONICAL session name + worktree cwd, both derived backend-side
/// from `repo` + `slug`. The dock never supplies a session name or path — that
/// keeps the webview from naming/killing arbitrary tmux sessions or opening a
/// shell outside the workspace, and keeps sidecar names STABLE (a place's
/// canonical name doesn't change when it briefly runs under an adopted session).
fn place_session_cwd(repo: &str, slug: &str) -> Result<(String, String), String> {
    let p = Project::discover(Path::new(repo)).map_err(|e| e.msg)?;
    Ok((p.session_name(slug), p.place_dir(slug)))
}

/// A place's dock shells, with liveness. The dock restores its Terminal tabs
/// from this; shells don't outlive the app (see `Shells`), so after a restart
/// it's empty and the dock opens a fresh one. `dead` matters because the
/// `shell:exit` event is transient — a shell that exits while the dock is
/// closed has no listener, and without this flag the reopened dock would render
/// the corpse as a live tab (replayed scrollback, EIO on the first keypress).
#[derive(Serialize)]
struct ShellTab {
    index: u32,
    dead: bool,
}

#[tauri::command]
async fn list_shell_sessions(repo: String, slug: String, shells: State<'_, Shells>) -> Result<Vec<ShellTab>, String> {
    let mut map = shells.0.lock().unwrap();
    let mut tabs: Vec<ShellTab> = map
        .iter_mut()
        .filter(|((r, s, _), _)| r == &repo && s == &slug)
        .map(|((_, _, i), sh)| ShellTab { index: *i, dead: matches!(sh.child.try_wait(), Ok(Some(_))) })
        .collect();
    tabs.sort_unstable_by_key(|t| t.index);
    Ok(tabs)
}

/// End one shell tab — kills the process, unlike `shell_detach`.
#[tauri::command]
async fn close_shell_session(
    app: AppHandle,
    repo: String,
    slug: String,
    index: u32,
    keep_cwd: Option<bool>,
    shells: State<'_, Shells>,
) -> Result<(), String> {
    let key: ShellKey = (repo.clone(), slug.clone(), index);
    let keep = keep_cwd.unwrap_or(false);
    // A RESTART keeps this tab's scrollback, so persist it up to the second
    // before the corpse is reaped. This is the one moment where "as of up to 15
    // seconds ago" would be visible in the same breath as the action that caused
    // it — and the dead shell's last output is usually the thing being restarted
    // to get away from.
    if keep {
        flush_one_scrollback(&app, &shells, &key);
    }
    kill_shell(&shells, &key);
    // A tab the user CLOSED forgets where it was, exactly as it drops its name
    // (App.tsx `closeTab`) — otherwise the next tab to take this index would
    // open in a directory it never visited. A tab being RESTARTED goes through
    // this same command to reap the corpse and asks to KEEP its directory: it
    // is the same tab, and restarting a shell that died in a subdirectory only
    // to land at the place root is the exact papercut this feature removes.
    if !keep {
        edit_cwds(&app, |map| forget_tab(map, &repo, &slug, index));
        // …and its scrollback and command history with it, for the same reason
        // and with more at stake: the next tab to take this index has never been
        // here, and inheriting someone else's output is worse than inheriting
        // their directory.
        if let Ok(root) = term_hist_dir(&app) {
            forget_tab_history(&root, &key);
        }
    }
    Ok(())
}

// ── dock shell cwd memory ────────────────────────────────────────────────────
// A dock shell dies with the app, so its tab used to reopen at the place root
// however deep you had cd'd. We remember the DIRECTORY of each tab — nothing
// else: no history, no scrollback, no environment.
//
// Its own file on purpose. `ui-state.json` is written WHOLE-BLOB by the
// frontend (`set_settings` takes the entire settings object), so anything the
// backend wrote into it would be erased by the next settings save.

/// `"<repo>|<slug>"` → tab index → last known directory. Same key scheme as the
/// frontend's `term_tab_names`, which this shadows one-for-one.
type CwdMap = BTreeMap<String, BTreeMap<u32, String>>;

/// Serialises writers to `shell-cwds.json`: the slow sampler and the shell
/// commands both read-modify-write, and interleaving them would lose an update.
static CWD_FILE_LOCK: Mutex<()> = Mutex::new(());

fn cwd_key(repo: &str, slug: &str) -> String {
    format!("{repo}|{slug}")
}

fn cwd_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("shell-cwds.json"))
}

/// A missing or corrupt file is an empty memory, never an error: the worst it
/// can cost is one shell opening at the place root.
fn read_cwds_at(path: &Path) -> CwdMap {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn read_cwds(app: &AppHandle) -> CwdMap {
    cwd_file(app).map(|p| read_cwds_at(&p)).unwrap_or_default()
}

/// Read-modify-write under `CWD_FILE_LOCK`. `f` returns false to skip the write —
/// the sampler runs every few seconds and an idle shell must not churn the disk.
fn edit_cwds_at(path: &Path, f: impl FnOnce(&mut CwdMap) -> bool) {
    let _guard = CWD_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = read_cwds_at(path);
    if !f(&mut map) {
        return;
    }
    map.retain(|_, tabs| !tabs.is_empty());
    let json = match serde_json::to_vec_pretty(&map) {
        Ok(j) => j,
        Err(e) => return applog("error", &format!("shell-cwds encode failed: {e}")),
    };
    // Write-then-rename, because readers do NOT take this lock: `shell_open`
    // reads the file while holding the shell registry, and a plain `fs::write`
    // is a truncate followed by a write — a read landing in between gets half a
    // JSON document, which parses as "no memory at all".
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, path)) {
        applog("error", &format!("shell-cwds write failed: {e}"));
        let _ = std::fs::remove_file(&tmp);
    }
}

/// The directory remembered for one tab, if any.
fn remembered_dir(map: &CwdMap, repo: &str, slug: &str, index: u32) -> Option<String> {
    map.get(&cwd_key(repo, slug)).and_then(|tabs| tabs.get(&index)).cloned()
}

/// Drop one tab's directory; true if there was one. Empty places are swept by
/// `edit_cwds_at`, so a place whose last tab closes leaves no husk.
fn forget_tab(map: &mut CwdMap, repo: &str, slug: &str, index: u32) -> bool {
    map.get_mut(&cwd_key(repo, slug)).is_some_and(|tabs| tabs.remove(&index).is_some())
}

fn edit_cwds(app: &AppHandle, f: impl FnOnce(&mut CwdMap) -> bool) {
    match cwd_file(app) {
        Ok(path) => edit_cwds_at(&path, f),
        // A read that can't find the file is just "no memory yet"; a WRITE that
        // can't resolve the config dir is a real failure and has to say so.
        Err(e) => applog("error", &format!("shell-cwds path unavailable: {e}")),
    }
}

/// A live process's working directory, straight from the OS — no subprocess and
/// no shell integration. macOS has no `/proc`, so it goes through libproc
/// (`libc` is already a dependency for the `kill(pid,0)` probes). The shell-side
/// alternative, OSC 7, is a non-starter here: Apple's `/etc/zshrc` only emits it
/// when `TERM_PROGRAM` is `Apple_Terminal`, so we would have to lie about
/// `TERM_PROGRAM` or edit the user's rc files.
fn proc_cwd(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let want = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
        // Returns bytes written. Anything short of the whole struct (dead pid,
        // EPERM) means there is no path to be had — not a truncated one.
        let got = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut info as *mut libc::proc_vnodepathinfo).cast(),
                want,
            )
        };
        if got != want {
            return None;
        }
        // libc spells this MAXPATHLEN buffer `[[c_char; 32]; 32]` (a workaround
        // for the old rustc it supports), so flatten it back to bytes.
        let raw = &info.pvi_cdir.vip_path;
        let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), std::mem::size_of_val(raw)) };
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        (end > 0).then(|| PathBuf::from(String::from_utf8_lossy(&bytes[..end]).into_owned()))
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// The pid of a shell that is STILL RUNNING, or `None`.
///
/// The liveness half is not defensive tidiness, it is the whole point. A shell
/// that exits on its own is deliberately left in the registry so the tab can
/// survive and offer a restart — and `list_shell_sessions` calls `try_wait` on
/// every dock mount, which REAPS it. `Child::process_id` then keeps handing
/// back a pid that the OS is free to hand to something else (macOS wraps at
/// 99999, days on a normal machine). Sampling that pid would read a stranger's
/// directory and file it under this tab.
fn live_pid(child: &mut (dyn Child + Send + Sync)) -> Option<u32> {
    match child.try_wait() {
        Ok(None) => child.process_id(), // still running
        _ => None,                      // exited, or we cannot tell — either way, don't sample
    }
}

/// Fold a sample into the stored map; true if anything actually moved.
///
/// MERGES rather than replaces, and that is the whole subtlety: after a restart
/// a tab exists as a NAME long before it is re-spawned, so the map holds
/// entries with no live shell behind them. A wholesale rewrite from the live
/// set would wipe exactly the directories this feature exists to keep.
fn merge_cwds(map: &mut CwdMap, sample: Vec<(ShellKey, String)>) -> bool {
    let mut changed = false;
    for ((repo, slug, index), path) in sample {
        let tabs = map.entry(cwd_key(&repo, &slug)).or_default();
        if tabs.get(&index) != Some(&path) {
            tabs.insert(index, path);
            changed = true;
        }
    }
    changed
}

/// Record where every live dock shell currently is. Called on a slow tick — so
/// a crash or a force-quit still leaves a recent answer — and once more on the
/// way out, before the exit sweep kills the shells.
fn save_shell_cwds(app: &AppHandle, shells: &Shells) {
    let live: Vec<(ShellKey, u32)> = {
        let mut map = shells.0.lock().unwrap();
        map.iter_mut().filter_map(|(k, sh)| live_pid(&mut *sh.child).map(|pid| (k.clone(), pid))).collect()
    };
    let sample: Vec<(ShellKey, String)> = live
        .into_iter()
        .filter_map(|(key, pid)| proc_cwd(pid).map(|p| (key, p.to_string_lossy().into_owned())))
        .collect();
    if sample.is_empty() {
        return; // no shells (or no cwd to be read) — don't even open the file
    }
    edit_cwds(app, |map| {
        // Membership is re-checked HERE, under the file lock, because the
        // registry lock was released before the `proc_cwd` reads above. A tab
        // closed in that gap has already had its entry forgotten, and merging
        // the reading we took a moment earlier would put it straight back — so
        // the next tab to reuse that index would open somewhere it has never
        // been, which is exactly what `close_shell_session` promises cannot
        // happen. (No lock-order hazard: nothing that holds the registry ever
        // waits on this file lock — `shell_open` only READS the file, and reads
        // don't take it.)
        let open = shells.0.lock().unwrap();
        let fresh: Vec<(ShellKey, String)> = sample.into_iter().filter(|(k, _)| open.contains_key(k)).collect();
        drop(open);
        merge_cwds(map, fresh)
    });
}

/// Which `repo|slug` keys no longer name a place on disk. `place_dir` resolves
/// one, or returns `None` when the PROJECT itself is unreachable — a repo that
/// has been deleted or moved takes all of its places with it.
fn vanished_keys(keys: &[String], mut place_dir: impl FnMut(&str, &str) -> Option<String>) -> Vec<String> {
    keys.iter()
        .filter(|key| {
            // repo roots are absolute paths and a slug is a directory basename,
            // so the LAST separator is the real one
            match key.rsplit_once('|') {
                Some((repo, slug)) => !place_dir(repo, slug).is_some_and(|d| Path::new(&d).is_dir()),
                None => true, // not a key this app writes — it cannot name a live place
            }
        })
        .cloned()
        .collect()
}

/// Forget every place whose worktree is gone. The app cleans up after its own
/// `remove_place`, but `worktrees rm` from a terminal leaves the entry behind,
/// and the key is only `repo|slug` — so a slug that is later recreated (a
/// recycled branch name) would inherit the previous life's directories wherever
/// those paths still exist in the new checkout. Sweeping at startup shrinks
/// that to the case where the place is removed and recreated with the app
/// closed, which is accepted: what survives is a real directory inside the
/// worktree, so the shell opens somewhere that exists rather than anywhere
/// misleading.
fn forget_vanished_places(app: &AppHandle) {
    let keys: Vec<String> = read_cwds(app).keys().cloned().collect();
    if keys.is_empty() {
        return;
    }
    let mut projects: HashMap<String, Option<Project>> = HashMap::new();
    let gone = vanished_keys(&keys, |repo, slug| {
        projects.entry(repo.to_string()).or_insert_with(|| Project::discover(Path::new(repo)).ok());
        projects[repo].as_ref().map(|p| p.place_dir(slug))
    });
    if gone.is_empty() {
        return;
    }
    applog("info", &format!("shell-cwds: forgetting {} place(s) that no longer exist", gone.len()));
    edit_cwds(app, |map| {
        let before = map.len();
        map.retain(|k, _| !gone.contains(k));
        map.len() != before
    });
}

/// Where a tab should reopen: its last known directory if that still exists,
/// else the place root. No subtree restriction — if you cd'd out of the
/// worktree, out of the worktree is where you were.
fn pick_start_dir(saved: Option<&str>, place_dir: &str) -> String {
    match saved {
        Some(p) if Path::new(p).is_dir() => p.to_string(),
        _ => place_dir.to_string(),
    }
}

// ── dock shell history: scrollback + per-tab commands ────────────────────────
// What a tab keeps between app RUNS, beyond the directory above: the output it
// was showing, and the commands it ran. One directory per tab holds both:
//
//   <app config>/term-history/<slug>-<index>-<hash>/
//       meta.json        which tab this is, and the width the bytes were laid out at
//       scrollback.bin   the replay ring, verbatim
//       zdotdir/         the generated ZDOTDIR, and the .zsh_history zsh writes there
//
// Its own tree, for the same reason `shell-cwds.json` is its own file:
// `ui-state.json` is written WHOLE-BLOB by the frontend, so nothing the backend
// writes may go near it.
//
// The directory NAME is only a name. A repo path is absolute and a slug can be
// anything a branch name can be, so the stem is a hash — and `meta.json` names
// the tab it belongs to, so a collision, a directory left by an older scheme, or
// a half-written one reads as "not mine" rather than as someone else's
// scrollback. Nothing in here is ever an error: the worst any failure costs is a
// tab that opens blank, which is what every tab did before this existed.
//
// It is also the only place this app writes a user's terminal OUTPUT to disk —
// anything a script echoed, secrets included — so the tree is created 0700 and
// the files 0600, and Settings says so next to the switch that turns it on.

/// For the directory NAME only — never for identity (see `read_scrollback`).
/// FNV-1a 64, inline rather than a crate for eight lines. Not a security
/// boundary: it names a file.
fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A readable, filesystem-safe stem for one tab. The slug is decoration — it is
/// what makes the directory browsable — so it is clamped hard; the hash carries
/// all of the uniqueness.
fn tab_stem(key: &ShellKey) -> String {
    let (repo, slug, index) = key;
    let safe: String = slug
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '-' })
        .take(32)
        .collect();
    format!("{safe}-{index}-{:016x}", fnv1a64(&format!("{repo}|{slug}|{index}")))
}

/// How long an untouched tab's history is kept. This tree holds CONTENT, and
/// unlike the cwd map there is nothing about it that shrinks on its own, so it
/// needs a horizon as well as the sweeps.
const TERM_HIST_MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;

/// Serialises writers, exactly as `CWD_FILE_LOCK` does for the directories: the
/// slow sampler, the shell commands and the sweeps all touch this tree.
static TERM_HIST_LOCK: Mutex<()> = Mutex::new(());

/// Frontend-owned preferences the shell path needs. Set by
/// `set_term_history_opts` on hydration and on every change — the same shape as
/// `FETCH_INTERVAL_SECS`, and for the same reason: the backend must never READ
/// `ui-state.json` to find them out, because it must never write it.
static PERSIST_SCROLLBACK: AtomicBool = AtomicBool::new(true);
static PER_TAB_HISTORY: AtomicBool = AtomicBool::new(true);

/// `<app config>/term-history`, created 0700 on demand.
fn term_hist_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?.join("term-history");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    private_perms(&dir, 0o700);
    Ok(dir)
}

/// Tighten a path we just created. Best-effort: a filesystem that cannot express
/// the mode is not a reason to refuse to remember a tab.
fn private_perms(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

fn tab_dir(root: &Path, key: &ShellKey) -> PathBuf {
    root.join(tab_stem(key))
}

/// Which tab a directory belongs to, and the WIDTH its bytes were laid out at.
///
/// `cols` is not bookkeeping. Raw bytes replayed into a different grid do not
/// re-wrap, and the cursor-relative redraws in them were computed for a width
/// that no longer exists — which is the defect ROADMAP.md parks under
/// "terminal-state serialization". Recording the width lets the pane hand the
/// recording the grid it was written for and let xterm reflow it back.
///
/// It is the pty's CURRENT width, not the width the oldest byte was written at —
/// a byte ring has no single true width. It converges: quit at 174, restore at
/// 100, and the next save records 100.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct TabMeta {
    repo: String,
    slug: String,
    index: u32,
    cols: u16,
    at: i64,
}

fn read_meta(tab: &Path) -> Option<TabMeta> {
    serde_json::from_slice(&std::fs::read(tab.join("meta.json")).ok()?).ok()
}

/// Write-then-rename, 0600. Readers deliberately do NOT take `TERM_HIST_LOCK` —
/// `shell_open` reads this tree while holding the shell registry, and nothing
/// holding the registry may wait on a file lock — so a plain write (a truncate
/// followed by a write) would let a read land on half a document.
fn hist_write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let r = std::fs::write(&tmp, bytes).and_then(|_| {
        private_perms(&tmp, 0o600);
        std::fs::rename(&tmp, path)
    });
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// Drop a leading partial line.
///
/// Only correct for a ring that has actually ROLLED — see `rolled_ring` below,
/// which is the only caller and exists to state that condition.
/// Bounded, so a ring with no newline at all is left alone rather than emptied.
fn trim_to_line_start(b: &[u8]) -> &[u8] {
    const LOOK: usize = 4096;
    match b[..b.len().min(LOOK)].iter().position(|&c| c == b'\n') {
        Some(i) => &b[i + 1..],
        None => b,
    }
}

/// What of a ring is worth saving.
///
/// A ring at the CAP has been drained from the front, so it begins wherever
/// `drain` happened to cut — routinely the tail of an escape sequence whose
/// introducer is gone, which replays as a few stray characters at the top. That
/// leading fragment is dropped.
///
/// A ring BELOW the cap has never rolled: it begins where the shell began, and
/// its first line is a real one. Trimming that would throw away a genuine line
/// on every save — and worse, it COMPOUNDS, because the next restore seeds from
/// the trimmed copy and the save after that trims the new first line. One line
/// per restart, silently, off the oldest end. The length test is what separates
/// the two cases, and it is exact: nothing else can put a ring at the cap.
fn rolled_ring(bytes: &[u8]) -> &[u8] {
    if bytes.len() >= SHELL_RING {
        trim_to_line_start(bytes)
    } else {
        bytes
    }
}

/// This tab's saved scrollback and the metadata it was saved with, or `None`.
///
/// `meta.json` is the identity, not the directory name: anything that does not
/// name exactly this tab answers `None`. Never an error, and never another tab's
/// output.
fn read_scrollback(root: &Path, key: &ShellKey) -> Option<(Vec<u8>, TabMeta)> {
    let tab = tab_dir(root, key);
    let meta = read_meta(&tab)?;
    if meta.repo != key.0 || meta.slug != key.1 || meta.index != key.2 {
        return None;
    }
    let bytes = std::fs::read(tab.join("scrollback.bin")).ok()?;
    Some((bytes, meta))
}

/// Stamp a tab's identity without touching its scrollback.
///
/// Called when a tab's `zdotdir` is generated, so a tab whose shell is closed
/// before the first flush still has the `meta.json` every sweep here reads. A
/// directory with no meta is one nothing can judge, so it would never be
/// collected.
fn ensure_meta(root: &Path, key: &ShellKey, cols: u16) {
    let tab = tab_dir(root, key);
    if read_meta(&tab).is_some() {
        return;
    }
    write_meta(&tab, key, cols);
}

fn write_meta(tab: &Path, key: &ShellKey, cols: u16) {
    let meta = TabMeta {
        repo: key.0.clone(),
        slug: key.1.clone(),
        index: key.2,
        cols,
        at: sysclock::now_epoch(),
    };
    match serde_json::to_vec_pretty(&meta) {
        Ok(j) => {
            if let Err(e) = hist_write_atomic(&tab.join("meta.json"), &j) {
                applog("error", &format!("term-history meta write failed: {e}"));
            }
        }
        Err(e) => applog("error", &format!("term-history meta encode failed: {e}")),
    }
}

/// Persist one tab's ring at the width it is currently being written at.
fn write_scrollback(root: &Path, key: &ShellKey, bytes: &[u8], cols: u16) {
    let tab = tab_dir(root, key);
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = std::fs::create_dir_all(&tab) {
        return applog("error", &format!("term-history dir failed: {e}"));
    }
    private_perms(&tab, 0o700);
    // bytes THEN meta: meta is what makes the directory readable, so writing it
    // last means a crash mid-save leaves the PREVIOUS pair, never a new meta
    // pointing at a half-written ring.
    if let Err(e) = hist_write_atomic(&tab.join("scrollback.bin"), rolled_ring(bytes)) {
        return applog("error", &format!("term-history write failed: {e}"));
    }
    write_meta(&tab, key, cols);
}

/// Everything one tab keeps between runs. Called when the tab is CLOSED — a
/// RESTART keeps it (it is the same tab, and the dead shell's output is usually
/// the thing you wanted to read), and so does closing the place, which keeps tab
/// names and therefore has to keep what they hold.
fn forget_tab_history(root: &Path, key: &ShellKey) {
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_dir_all(tab_dir(root, key));
}

/// Every tab of one place. Used when the place is REMOVED.
fn forget_place_history(root: &Path, repo: &str, slug: &str) {
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && read_meta(&p).is_some_and(|m| m.repo == repo && m.slug == slug) {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
}

/// Which stored tabs should go: the ones whose place no longer exists, and the
/// ones nobody has touched in a month.
///
/// Split out from the filesystem so the decision can be tested directly, the way
/// `vanished_keys` is. `place_dir` answers `None` when the PROJECT itself is
/// unreachable — a repo that has been deleted or moved takes its places with it.
fn stale_history<'a>(
    metas: &'a [(PathBuf, TabMeta)],
    now: i64,
    mut place_dir: impl FnMut(&str, &str) -> Option<String>,
) -> Vec<&'a PathBuf> {
    metas
        .iter()
        .filter(|(_, m)| {
            now - m.at > TERM_HIST_MAX_AGE_SECS
                || !place_dir(&m.repo, &m.slug).is_some_and(|d| Path::new(&d).is_dir())
        })
        .map(|(p, _)| p)
        .collect()
}

/// Cold-start sweep. Same reasoning as `forget_vanished_places` — a
/// `worktrees rm` from a terminal never tells the app — plus the age horizon,
/// because this tree holds content rather than a path.
fn sweep_term_history(app: &AppHandle) {
    let Ok(root) = term_hist_dir(app) else { return };
    let Ok(rd) = std::fs::read_dir(&root) else { return };
    let metas: Vec<(PathBuf, TabMeta)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        // no readable meta = nothing to judge it by; left alone rather than
        // guessed at (`ensure_meta` is what keeps that set empty)
        .filter_map(|p| read_meta(&p).map(|m| (p, m)))
        .collect();
    if metas.is_empty() {
        return;
    }
    let mut projects: HashMap<String, Option<Project>> = HashMap::new();
    let gone = stale_history(&metas, sysclock::now_epoch(), |repo, slug| {
        projects.entry(repo.to_string()).or_insert_with(|| Project::discover(Path::new(repo)).ok());
        projects[repo].as_ref().map(|p| p.place_dir(slug))
    });
    if gone.is_empty() {
        return;
    }
    applog("info", &format!("term-history: sweeping {} tab(s)", gone.len()));
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for p in gone {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// One shell's ring, ready to write once the registry lock is gone.
type ScrollJob = (ShellKey, Arc<Mutex<VecDeque<u8>>>, u16);

/// Persist every live shell's ring.
///
/// Dirty-gated, so a dozen idle tabs cost no I/O at all. The snapshot is taken
/// under the REGISTRY lock and written after it is released: nothing that holds
/// the registry may wait on a file lock, which is the same rule `save_shell_cwds`
/// follows and the reason `shell_open` only ever reads this tree.
///
/// The flag is cleared BEFORE the ring is read, not after it is written. Output
/// landing in that window re-sets it and costs one redundant write next tick;
/// clearing it afterwards would instead drop the last chunk of a shell that then
/// went quiet — the one direction that loses something.
///
/// Deliberately NOT filtered on `live_pid`, unlike `save_shell_cwds`. A shell
/// that exited is kept in the registry precisely so its tab survives and can
/// offer a restart, and its ring is exactly what the user wanted to read; a dead
/// process has no cwd to sample, but it has plenty of scrollback.
fn save_shell_scrollback(app: &AppHandle, shells: &Shells) {
    if !PERSIST_SCROLLBACK.load(Ordering::Relaxed) {
        return;
    }
    let jobs: Vec<ScrollJob> = {
        let map = shells.0.lock().unwrap();
        map.iter()
            .filter(|(_, sh)| sh.dirty.swap(false, Ordering::Relaxed))
            .map(|(k, sh)| (k.clone(), sh.ring.clone(), sh.size.cols))
            .collect()
    };
    if jobs.is_empty() {
        return;
    }
    let Ok(root) = term_hist_dir(app) else { return };
    for (key, ring, cols) in jobs {
        let bytes: Vec<u8> = ring.lock().unwrap().iter().copied().collect();
        write_scrollback(&root, &key, &bytes, cols);
    }
}

/// Persist ONE tab now, whatever its dirty flag says.
///
/// Used by the restart path: closing a dead shell to make room for a new one in
/// the same tab is the one moment where "up to 15 seconds ago" is visible in the
/// same breath as the action that caused it.
fn flush_one_scrollback(app: &AppHandle, shells: &Shells, key: &ShellKey) {
    if !PERSIST_SCROLLBACK.load(Ordering::Relaxed) {
        return;
    }
    let job = {
        let map = shells.0.lock().unwrap();
        map.get(key).map(|sh| {
            sh.dirty.store(false, Ordering::Relaxed);
            (sh.ring.clone(), sh.size.cols)
        })
    };
    let (Some((ring, cols)), Ok(root)) = (job, term_hist_dir(app)) else { return };
    let bytes: Vec<u8> = ring.lock().unwrap().iter().copied().collect();
    write_scrollback(&root, key, &bytes, cols);
}

/// `at` on the machine's LOCAL wall clock.
///
/// The seam below is the one thing this app writes for a person to read where
/// it sits rather than for a log, and app.log's UTC — which CLAUDE.md already
/// warns about when cross-referencing — is the wrong register for it. libc,
/// because chrono is not a dependency and an offset is all `fmt_utc` lacks.
fn fmt_local(at: i64) -> String {
    #[cfg(unix)]
    {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let t = at as libc::time_t;
        if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return fmt_utc(at);
        }
        return fmt_utc(at + tm.tm_gmtoff as i64);
    }
    #[cfg(not(unix))]
    fmt_utc(at)
}

/// Opens the seam, and nothing else emits it: `\x18` cancels any control
/// sequence the ring's tail was cut inside, `\x1b\\` closes an unterminated
/// OSC/DCS. Without them a seam can be swallowed as the argument of a
/// half-written sequence — the one failure that reads as "the feature did not
/// run". It doubles as the marker `trim_trailing_seam` finds.
const SEAM_HEAD: &[u8] = b"\x18\x1b\\";

/// What separates restored output from the shell about to start under it. Dim,
/// so it reads as chrome rather than as something a command printed.
fn restored_seam(at: i64) -> Vec<u8> {
    let mut out = SEAM_HEAD.to_vec();
    out.extend_from_slice(format!("\r\n\x1b[2m── restored · {} ──\x1b[0m\r\n", fmt_local(at)).as_bytes());
    out
}

/// Drop a seam that is the LAST thing in the buffer — one a previous restore
/// wrote and nothing has been typed under since. A seam with a session's output
/// after it is left alone, because it still marks a real boundary.
///
/// The test is exact rather than a search: a seam is FIXED WIDTH (`fmt_utc` is),
/// so the buffer ends with one precisely when the last `n` bytes begin with
/// `SEAM_HEAD`. Scanning a window instead finds a seam that has work under it —
/// which trims a boundary that should have stayed, and which is what the second
/// half of `a_seam_nothing_was_typed_under_is_replaced_not_followed` caught.
fn trim_trailing_seam(bytes: &[u8]) -> &[u8] {
    let n = restored_seam(0).len();
    if bytes.len() < n {
        return bytes;
    }
    let cut = bytes.len() - n;
    if &bytes[cut..cut + SEAM_HEAD.len()] == SEAM_HEAD {
        &bytes[..cut]
    } else {
        bytes
    }
}

/// The bytes a restoring tab is handed first: what it had, then a seam saying
/// where that ended. Empty when there is nothing saved, so `replay` stays 0 and
/// the pane keeps treating the shell's own first output as live.
///
/// This goes into the RING as well as down the channel — which is a reversal
/// worth stating, because the tidy-looking rule is the opposite. A re-attach
/// replays the ring and nothing else, so a seam kept out of it is visible only
/// until the first tab flip; under React StrictMode the pane re-attaches before
/// anyone has seen anything, so the seam was effectively dead. Being in the ring
/// makes it persistable, which is why it carries an ABSOLUTE time rather than an
/// age that would freeze at "2h ago" forever, and why a seam still sitting at
/// the very end is REPLACED rather than followed — otherwise three launches with
/// nothing typed between them stack three of them.
fn restore_payload(bytes: &[u8], at: i64) -> Vec<u8> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = trim_trailing_seam(bytes).to_vec();
    out.extend_from_slice(&restored_seam(at));
    out
}

// ── per-tab command history ─────────────────────────────────────────────────
// zsh is the whole reason this is not simply `HISTFILE`. macOS's `/etc/zshrc`
// sets `HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history` UNCONDITIONALLY for every
// interactive shell — measured: export HISTFILE, ask an interactive zsh what it
// is, and it answers `~/.zsh_history`. `ZDOTDIR` is the one lever that file
// honours.
//
// The cost is that ZDOTDIR also moves where zsh looks for `.zshenv`,
// `.zprofile`, `.zshrc` and `.zlogin`, so this directory carries a shim for each
// that sources the user's real one. We generate our own files and never touch
// theirs — the same rule that ruled out OSC 7 for the working directory.
//
// Two things the shims have to get right, and a naive version of this gets both
// wrong:
//
//   A user's own `~/.zshenv` may set ZDOTDIR — `export ZDOTDIR=$HOME/.config/zsh`
//   is the standard dotfiles layout. Sourced blind, that hijacks the rest of the
//   chain: zsh would read THEIR .zshrc, `/etc/zshrc` would put HISTFILE in THEIR
//   directory, and the tab would quietly share the global history again. So the
//   shim hands ZDOTDIR back before sourcing theirs, reads whatever they set, and
//   then takes the chain back.
//
//   Their `.zshrc` may reference `$ZDOTDIR` itself (`source $ZDOTDIR/aliases.zsh`
//   is how every plugin manager lays out a config). So ZDOTDIR is restored to
//   THEIR value before their rc is sourced — which also means every child shell
//   from that point on gets a normal environment and writes the normal
//   `~/.zsh_history`, rather than inheriting this tab's.
//
// HISTFILE is then set explicitly, last, from an env var rather than a path baked
// into a generated file: last so it beats a user rc that sets HISTFILE itself,
// and explicitly so this does not depend on an Apple-specific line in /etc.

const SHIM_HEADER: &str = "# Generated by worktrees — per-tab shell history. Edits are overwritten.\n";

/// `$ZDOTDIR/.zshenv`: the load-bearing shim. Runs before anything else of the
/// user's and is the only place that can discover a ZDOTDIR they set themselves.
const ZSHENV_SHIM: &str = r#"_WT_Z=${ZDOTDIR}
unset ZDOTDIR
[ -r "$HOME/.zshenv" ] && . "$HOME/.zshenv"
_WT_USER_Z=${ZDOTDIR:-$HOME}
export ZDOTDIR=$_WT_Z
"#;

const ZPROFILE_SHIM: &str = r#"[ -r "${_WT_USER_Z:-$HOME}/.zprofile" ] && . "${_WT_USER_Z:-$HOME}/.zprofile"
"#;

/// `$ZDOTDIR/.zshrc`: hand the chain back, source theirs, then take the history.
///
/// `INC_APPEND_HISTORY` is not a nicety. The app SIGHUPs its shells on the way
/// out (portable-pty's `Child::kill()` sends SIGHUP, not SIGKILL), and zsh's
/// default is to flush history only on a clean exit — without this a force-quit
/// takes the whole session's commands with it, which is precisely the case this
/// feature exists to survive. `SHARE_HISTORY` is deliberately NOT set: each tab
/// has its own file, so there is nothing to share, and it would change arrow-key
/// behaviour the user never asked us to touch.
const ZSHRC_SHIM: &str = r#"if [ "${_WT_USER_Z:-$HOME}" = "$HOME" ]; then unset ZDOTDIR; else export ZDOTDIR=$_WT_USER_Z; fi
[ -r "${_WT_USER_Z:-$HOME}/.zshrc" ] && . "${_WT_USER_Z:-$HOME}/.zshrc"
[ -n "$WORKTREES_HISTFILE" ] && HISTFILE="$WORKTREES_HISTFILE"
HISTSIZE=10000
SAVEHIST=10000
setopt INC_APPEND_HISTORY
unset _WT_Z _WT_USER_Z
"#;

/// `$ZDOTDIR/.zlogin`: only reached by a NON-interactive login shell, because
/// `.zshrc` has restored ZDOTDIR by the time an interactive one gets here and
/// zsh then finds the user's own directly.
const ZLOGIN_SHIM: &str = r#"[ -r "${_WT_USER_Z:-$HOME}/.zlogin" ] && . "${_WT_USER_Z:-$HOME}/.zlogin"
"#;

/// (Re)write the four shims. Regenerated on every spawn so an app upgrade
/// refreshes them and a user who deletes the directory gets it back.
fn write_zdotdir(zdot: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(zdot)?;
    private_perms(zdot, 0o700);
    for (name, body) in [
        (".zshenv", ZSHENV_SHIM),
        (".zprofile", ZPROFILE_SHIM),
        (".zshrc", ZSHRC_SHIM),
        (".zlogin", ZLOGIN_SHIM),
    ] {
        std::fs::write(zdot.join(name), format!("{SHIM_HEADER}{body}"))?;
    }
    Ok(())
}

/// Only worth saying once a run — it is a property of the user's `$SHELL`, not
/// of any one tab, and a spawn happens every time a tab is activated.
static UNSUPPORTED_SHELL_LOGGED: std::sync::Once = std::sync::Once::new();

/// Point this tab's shell at its own command history. Answers which scheme was
/// applied, for the tests; the caller ignores it.
fn apply_tab_history_env(
    cmd: &mut CommandBuilder,
    root: &Path,
    key: &ShellKey,
    shell_bin: &str,
) -> Option<&'static str> {
    let name = Path::new(shell_bin).file_name().and_then(|s| s.to_str()).unwrap_or("");
    let tab = tab_dir(root, key);
    match name {
        "zsh" => {
            let zdot = tab.join("zdotdir");
            if let Err(e) = write_zdotdir(&zdot) {
                applog("error", &format!("per-tab history: {e}"));
                return None;
            }
            cmd.env("WORKTREES_HISTFILE", zdot.join(".zsh_history"));
            cmd.env("ZDOTDIR", &zdot);
            Some("zdotdir")
        }
        // bash reads HISTFILE from the environment and its rc files almost never
        // set it, so the simple lever is the right one here. There is no ZDOTDIR
        // equivalent, and `--rcfile` does not apply to the login shell this must
        // stay.
        "bash" | "sh" => {
            if std::fs::create_dir_all(&tab).is_err() {
                return None;
            }
            private_perms(&tab, 0o700);
            cmd.env("HISTFILE", tab.join(".bash_history"));
            Some("histfile")
        }
        // fish and the rest keep whatever history they already had. Better than a
        // half-working scheme that silently loses commands — and the one log line
        // is what makes "why is there no history here" answerable.
        other => {
            let other = other.to_string();
            UNSUPPORTED_SHELL_LOGGED.call_once(|| {
                applog(
                    "info",
                    &format!("per-tab history: {other} is not supported; shells keep their own history"),
                );
            });
            None
        }
    }
}

// ── commands: preferences and the Data pane ─────────────────────────────────

/// Push the two terminal-history preferences down from the frontend.
///
/// The settings blob is backend-opaque and frontend-owned, so the shell path
/// cannot look them up — the frontend pushes instead, on hydration and on every
/// change. Same shape as `set_fetch_interval`, and for the same reason.
#[tauri::command]
async fn set_term_history_opts(persist_scrollback: bool, per_tab_history: bool) -> Result<(), String> {
    PERSIST_SCROLLBACK.store(persist_scrollback, Ordering::Relaxed);
    PER_TAB_HISTORY.store(per_tab_history, Ordering::Relaxed);
    Ok(())
}

/// Bytes under one directory, recursively. This tree is only ever
/// `<tab>/{meta.json, scrollback.bin, zdotdir/*}`, so the recursion is shallow.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// What Settings → Data shows beside "Clear saved terminal history".
#[derive(Serialize)]
struct TermHistoryInfo {
    dir: String,
    bytes: u64,
    tabs: usize,
}

#[tauri::command]
async fn term_history_info(app: AppHandle) -> Result<TermHistoryInfo, String> {
    let root = term_hist_dir(&app)?;
    let (mut bytes, mut tabs) = (0u64, 0usize);
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                tabs += 1;
                bytes += dir_bytes(&p);
            }
        }
    }
    Ok(TermHistoryInfo { dir: root.to_string_lossy().into_owned(), bytes, tabs })
}

/// Forget every tab's saved output and command history.
///
/// The live rings go too. Leaving them would have the next sampler tick write
/// the same bytes straight back, which would make the button a lie — but what is
/// already PAINTED in an open terminal stays, because xterm owns that and
/// wiping someone's visible screen is not what this asks for.
///
/// A tab's `zdotdir` is emptied rather than removed: a running shell holds
/// `HISTFILE` inside it, and deleting the directory would leave that shell
/// silently unable to record anything until it was restarted. `meta.json` stays
/// for the same practical reason — it is identity and width, never user
/// content, and a directory without it is one no sweep can ever collect.
#[tauri::command]
async fn term_history_clear(app: AppHandle, shells: State<'_, Shells>) -> Result<(), String> {
    // Scoped: nothing that holds the registry may wait on the file lock below.
    {
        let map = shells.0.lock().unwrap();
        for sh in map.values() {
            sh.ring.lock().unwrap().clear();
            sh.dirty.store(false, Ordering::Relaxed);
        }
    }
    let root = term_hist_dir(&app)?;
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(rd) = std::fs::read_dir(&root) else { return Ok(()) };
    let mut cleared = 0usize;
    for e in rd.flatten() {
        let tab = e.path();
        if !tab.is_dir() {
            continue;
        }
        cleared += 1;
        let _ = std::fs::remove_file(tab.join("scrollback.bin"));
        let zdot = tab.join("zdotdir");
        if zdot.is_dir() {
            let _ = hist_write_atomic(&zdot.join(".zsh_history"), b"");
        } else {
            let _ = std::fs::remove_file(tab.join(".bash_history"));
        }
    }
    applog("info", &format!("term-history: cleared {cleared} tab(s) at the user's request"));
    Ok(())
}

/// Remove a place (`rm <slug> -y` [+ --branch/--force]); the UI confirms first.
#[tauri::command]
async fn remove_place(
    app: AppHandle,
    repo: String,
    slug: String,
    del_branch: bool,
    force: bool,
    shells: State<'_, Shells>,
) -> Result<CmdResult, String> {
    let slug_log = slug.clone();
    let slug_sweep = slug.clone();
    let mut args: Vec<String> = vec![slug, "-y".into()];
    if del_branch {
        args.push("--branch".into());
    }
    if force {
        args.push("--force".into());
    }
    // Resolved BEFORE the removal, because it cannot be afterwards: the derived
    // tree is keyed by a hash of the canonical directory, and `canonicalize`
    // needs the directory to exist. A place nobody opened in the browser has no
    // tree and this resolves to one that is not there, which is fine.
    let viewer_root = Project::discover(Path::new(&repo))
        .ok()
        .map(|p| p.place_dir(&slug_sweep))
        .and_then(|d| std::fs::canonicalize(d).ok());
    // cmd_rm sweeps this place's dock shell sidecars itself (core, only once the
    // removal proceeds past its dirty/confirm guards — a refused rm keeps them).
    let r = run_op(&format!("rm {slug_log}"), &repo, move |p, ui| ops::cmd_rm(p, ui, &args))?;
    if r.ok {
        kill_place_shells(&shells, &repo, &slug_sweep);
        // …and the documents the viewer derived from it. `cmd_rm` deleted the
        // originals; without this the derived copy is the last readable one and
        // it is on a port.
        if let (Ok(cfg), Some(root)) = (app.path().app_config_dir(), viewer_root) {
            viewer::forget_place(&app.state::<viewer::Viewer>(), &cfg, &slug_sweep, &root);
        }
        // The place is gone for good, so its remembered directories are too.
        // (A `close` deliberately does NOT do this: closing keeps the tab names,
        // so it has to keep what those tabs point at.)
        edit_cwds(&app, |map| map.remove(&cwd_key(&repo, &slug_sweep)).is_some());
        if let Ok(root) = term_hist_dir(&app) {
            forget_place_history(&root, &repo, &slug_sweep);
        }
    }
    Ok(r)
}

// ── local usage metrics (ui-events.jsonl in the app config dir) ──────────────
// What gets clicked, and where the hours go — so the UI can shed the controls
// nobody uses. Everything stays on this Mac; nothing is ever sent anywhere.
//
// An event is THREE fixed things and a clock: a control KEY (a name baked into
// the source), a SURFACE (a fixed enum) and a timestamp. It is never a place, a
// slug, a path, a branch, a note, a title or anything else the user typed. The
// frontend is what decides the key — and `valid_token` below is the belt: this
// side refuses to STORE anything that does not look like a hand-written
// identifier, so a leak has to get past a hard parse rather than past a review.
//
// Its own file, for the same reason `shell-cwds.json` is its own file: the
// frontend writes `ui-state.json` whole-blob, so a backend write into it would
// be erased by the next settings save.

/// One line of `ui-events.jsonl`. `ms` is dwell only — an `act` has no duration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct UiEvent {
    /// unix MILLIseconds (the frontend's clock; a day bucket needs no more)
    t: i64,
    /// `act` (a click or a chord) · `dwell` (foreground ms on one surface)
    k: String,
    key: String,
    s: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    ms: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Is this a name someone WROTE, rather than something someone typed?
///
/// Deliberately an allowlist, not the brief's "no `/`, no spaces" denylist: a
/// denylist has to imagine every shape user text arrives in, and slugs, branch
/// names and note text are exactly the shapes nobody imagines. Keys and
/// surfaces are hand-written identifiers — `nav.row.select`, `chord.cmd-b`,
/// `dock.files` — so ASCII alphanumerics plus `.`, `-` and `_` is the whole
/// vocabulary, and 64 chars is far more than any of them needs.
///
/// It cannot tell `ui-tweaks` from `dock-files`, and it is not meant to: it is
/// the last line, not the first. The first is that the frontend resolves keys
/// from `data-track` attributes, which are constants in the source.
fn valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn valid_event(e: &UiEvent) -> bool {
    matches!(e.k.as_str(), "act" | "dwell") && valid_token(&e.key) && valid_token(&e.s) && e.ms >= 0 && e.t > 0
}

/// Serialises appenders against the rotation + the clear, all of which
/// read-modify-write the same two files.
static UI_EVENTS_LOCK: Mutex<()> = Mutex::new(());

/// Past this the file rotates to `.1` (one generation, then it is dropped).
/// A month of heavy use is a few hundred KB, so 4 MB is a year of headroom and
/// still small enough that `ui_usage` can read both generations on demand.
const UI_EVENTS_MAX: u64 = 4 * 1024 * 1024;

fn ui_events_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("ui-events.jsonl"))
}

fn ui_events_append_at(path: &Path, events: &[UiEvent]) {
    let _guard = UI_EVENTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let kept: Vec<&UiEvent> = events.iter().filter(|e| valid_event(e)).collect();
    let dropped = events.len() - kept.len();
    if dropped > 0 {
        // Loud, because the only way to reach it is a frontend that started
        // building a key out of something it read off the screen.
        applog("warn", &format!("ui-events: refused {dropped} event(s) that did not look like fixed keys"));
    }
    if kept.is_empty() {
        return;
    }
    if std::fs::metadata(path).map(|m| m.len() > UI_EVENTS_MAX).unwrap_or(false) {
        let _ = std::fs::rename(path, path.with_extension("jsonl.1"));
    }
    let mut buf = String::new();
    for e in kept {
        match serde_json::to_string(e) {
            Ok(line) => {
                buf.push_str(&line);
                buf.push('\n');
            }
            Err(e) => applog("warn", &format!("ui-events encode failed: {e}")),
        }
    }
    // Append, not write-then-rename: a lost tail costs a few clicks out of a
    // histogram, and a metric may never be the reason the UI stalls.
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(buf.as_bytes()) {
                applog("warn", &format!("ui-events write failed: {e}"));
            }
        }
        Err(e) => applog("warn", &format!("ui-events open failed: {e}")),
    }
}

/// Append a batch. NEVER fails the UI: a metric that can produce an error toast
/// is a metric that changes the behaviour it is measuring.
#[tauri::command]
async fn ui_events_append(app: AppHandle, events: Vec<UiEvent>) -> Result<(), String> {
    match ui_events_path(&app) {
        Ok(path) => ui_events_append_at(&path, &events),
        Err(e) => applog("warn", &format!("ui-events path unavailable: {e}")),
    }
    Ok(())
}

// ── aggregation ─────────────────────────────────────────────────────────────

/// Which LOCAL day an event landed in, as a day number since the epoch. The
/// offset comes from the frontend (`-new Date().getTimezoneOffset()`), because
/// the backend has no timezone database and the user reads the heatmap in the
/// only timezone that matters — theirs. `div_euclid`, not `/`: a pre-1970 or
/// west-of-UTC value must floor, not truncate toward zero.
fn day_index(t_ms: i64, tz_offset_min: i32) -> i64 {
    (t_ms.div_euclid(1000) + i64::from(tz_offset_min) * 60).div_euclid(86_400)
}

/// Day number → `YYYY-MM-DD`. `fmt_utc` already does civil-from-days; taking
/// its date half at midnight of that day is the same arithmetic.
fn day_label(day: i64) -> String {
    fmt_utc(day * 86_400)[..10].to_string()
}

#[derive(Serialize, PartialEq, Debug)]
struct ActRow {
    key: String,
    total: u64,
    per_day: Vec<u64>,
}

#[derive(Serialize, PartialEq, Debug)]
struct DwellRow {
    surface: String,
    ms_total: i64,
    per_day: Vec<i64>,
}

#[derive(Serialize, PartialEq, Debug)]
struct UiUsage {
    /// oldest first, exactly `days` entries, gaps filled with zeros
    days: Vec<String>,
    acts: Vec<ActRow>,
    dwell: Vec<DwellRow>,
    file: String,
}

fn read_events(path: &Path) -> Vec<UiEvent> {
    let mut out = Vec::new();
    // Both generations, oldest first. A half-written last line (a crash mid
    // append) is one dropped event, not a failed read — hence per-line parsing.
    for p in [path.with_extension("jsonl.1"), path.to_path_buf()] {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        out.extend(text.lines().filter_map(|l| serde_json::from_str::<UiEvent>(l).ok()).filter(valid_event));
    }
    out
}

/// The whole report, from a set of events. Split out from the command so the
/// tests can drive it without a clock or an AppHandle.
fn usage_from(events: &[UiEvent], now_ms: i64, days: u32, tz_offset_min: i32, file: String) -> UiUsage {
    let n = days.clamp(1, 90) as usize;
    let last = day_index(now_ms, tz_offset_min);
    let first = last - (n as i64 - 1);

    let mut acts: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut dwell: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for e in events {
        // A key the file has EVER seen gets a row, even when every one of its
        // events is older than the range — a row of zeros is not noise here,
        // it is the answer: that control has not been touched in a month.
        // (Dwell is the opposite: a surface at zero is a row of nothing, and
        // the bar chart drops it.)
        if e.k == "act" {
            acts.entry(e.key.clone()).or_insert_with(|| vec![0; n]);
        }
        let d = day_index(e.t, tz_offset_min);
        if d < first || d > last {
            continue;
        }
        let col = (d - first) as usize;
        match e.k.as_str() {
            "act" => acts.entry(e.key.clone()).or_insert_with(|| vec![0; n])[col] += 1,
            "dwell" => dwell.entry(e.s.clone()).or_insert_with(|| vec![0; n])[col] += e.ms,
            _ => {}
        }
    }

    let mut act_rows: Vec<ActRow> =
        acts.into_iter().map(|(key, per_day)| ActRow { total: per_day.iter().sum(), key, per_day }).collect();
    // Descending by total, then by key — a stable order, so two runs of the same
    // data draw the same table and a row does not jump between refreshes.
    act_rows.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.key.cmp(&b.key)));

    let mut dwell_rows: Vec<DwellRow> =
        dwell.into_iter().map(|(surface, per_day)| DwellRow { ms_total: per_day.iter().sum(), surface, per_day }).collect();
    dwell_rows.sort_by(|a, b| b.ms_total.cmp(&a.ms_total).then_with(|| a.surface.cmp(&b.surface)));

    UiUsage {
        days: (first..=last).map(day_label).collect(),
        acts: act_rows,
        dwell: dwell_rows,
        file,
    }
}

/// Settings → Usage reads this on open and on every range change. No cache: a
/// month of heavy use is a few hundred KB and the whole pass is one read plus a
/// map, which is cheaper than any invalidation rule would be to get right.
#[tauri::command]
async fn ui_usage(app: AppHandle, days: u32, tz_offset_min: i32) -> Result<UiUsage, String> {
    let path = ui_events_path(&app)?;
    let events = read_events(&path);
    let now_ms = sysclock::now_epoch() * 1000;
    Ok(usage_from(&events, now_ms, days, tz_offset_min, path.to_string_lossy().into()))
}

/// Both generations. The button that calls this is two-click armed.
#[tauri::command]
async fn ui_events_clear(app: AppHandle) -> Result<(), String> {
    let path = ui_events_path(&app)?;
    let _guard = UI_EVENTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for p in [path.with_extension("jsonl.1"), path.clone()] {
        if let Err(e) = std::fs::remove_file(&p) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(format!("could not remove {}: {e}", p.display()));
            }
        }
    }
    Ok(())
}

// ── PTY host: attach to a live tmux session ─────────────────────────────────

struct Term {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
struct Terminals(Mutex<HashMap<u32, Term>>);

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// Attach to an EXISTING tmux session inside a PTY and stream its bytes to the
/// frontend. We never create or own a shell — tmux owns the shells, panes, and
/// scrollback; this app is just another tmux client. Closing detaches (the
/// session survives and stays `tmux attach`-able from a bare terminal).
#[tauri::command]
async fn term_open(
    session: String,
    cols: u16,
    rows: u16,
    on_bytes: Channel<InvokeResponseBody>,
    terms: State<'_, Terminals>,
) -> Result<u32, String> {
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;

    // Sizing: tune the session (window-size latest + aggressive-resize) so a
    // smaller co-attached client can't clamp us — the clamp left stale painted
    // cells ("undeletable" artifacts) outside the redrawn region. Covers
    // sessions that predate the tuning-at-create in ops::launch.
    worktrees_core::tmux::tune_session(&session);
    // -u (global flag, must precede the subcommand): declare this client
    // UTF-8-capable. Without it tmux sniffs LC_ALL/LC_CTYPE/LANG, and a
    // GUI-launched app has none — tmux then draws every non-ASCII cell as "_".
    // Always safe here: the receiving end is xterm.js, which is always UTF-8.
    let mut cmd = CommandBuilder::new("tmux");
    cmd.args(["-u", "attach-session", "-t", &session]);
    cmd.env("TERM", "xterm-256color");

    let child = pair.slave.spawn_command(cmd).map_err(|e| {
        applog("error", &format!("term_open {session}: tmux attach spawn failed: {e}"));
        e.to_string()
    })?;
    drop(pair.slave); // parent doesn't need the slave handle after spawn

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = stop.clone();

    // Reader thread → frontend. Raw binary (no JSON eval of the byte stream).
    thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            if stop_reader.load(Ordering::Relaxed) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: the tmux client exited (detached)
                Ok(n) => {
                    if on_bytes.send(InvokeResponseBody::Raw(buf[..n].to_vec())).is_err() {
                        break; // frontend gone
                    }
                }
                Err(_) => break,
            }
        }
    });

    terms.0.lock().unwrap().insert(id, Term { master: pair.master, writer, child, stop });
    Ok(id)
}

#[tauri::command]
async fn term_write(id: u32, data: Vec<u8>, terms: State<'_, Terminals>) -> Result<(), String> {
    let mut map = terms.0.lock().unwrap();
    let term = map.get_mut(&id).ok_or("no such terminal")?;
    term.writer.write_all(&data).map_err(|e| e.to_string())?;
    term.writer.flush().map_err(|e| e.to_string())
}

#[tauri::command]
async fn term_resize(id: u32, cols: u16, rows: u16, terms: State<'_, Terminals>) -> Result<(), String> {
    let map = terms.0.lock().unwrap();
    let term = map.get(&id).ok_or("no such terminal")?;
    term.master
        .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

/// Detach, never kill the session. Killing the `tmux attach-session` CLIENT
/// process drops the client → tmux detaches it; the session (and its shells /
/// AI CLI) live on. The killed client also closes the slave, so the reader
/// thread hits EOF and exits.
#[tauri::command]
async fn term_close(id: u32, terms: State<'_, Terminals>) -> Result<(), String> {
    if let Some(mut term) = terms.0.lock().unwrap().remove(&id) {
        term.stop.store(true, Ordering::Relaxed);
        let _ = term.child.kill(); // kills the CLIENT = detach, not the session
    }
    Ok(())
}

// ── dock shells: PTYs this app OWNS ─────────────────────────────────────────
// The place's canonical session stays tmux (Claude lives there; it must survive
// quit and stay `tmux attach`-able from a bare terminal). The dock's scratch
// shells do NOT: they used to be `<session>~term[~n]` tmux sidecars that a
// second process then attached, which meant tmux swallowed C-b, scrollback went
// through copy-mode, a co-attached client could clamp the size (the whole
// `tune_session` + aggressive-resize dance), and the tab was simply dead when
// tmux wasn't installed. One owned PTY per tab fixes all four.
//
// What tmux WAS providing for free is survival across a detach — the dock
// closing, a tab flip, a place switch — so the registry keeps the process alive
// independently of the webview, and a ring buffer replays what was missed. Only
// quitting the app (or closing the tab) ends a shell.

/// Replay window per shell. Enough for a build log's tail; small enough that a
/// dozen idle tabs don't add up to anything.
const SHELL_RING: usize = 256 * 1024;

struct Shell {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    stop: Arc<AtomicBool>,
    /// Everything the shell has written, capped. Replayed on re-attach.
    ring: Arc<Mutex<VecDeque<u8>>>,
    /// The attached webview channel, if any. `None` = running unwatched.
    sink: Arc<Mutex<Option<Channel<InvokeResponseBody>>>>,
    /// Attach generation. Detach names the attach it is undoing, because a
    /// STALE detach must be a no-op: under React StrictMode the same pane
    /// mounts, unmounts, and mounts again — and unmount №1's detach can arrive
    /// AFTER mount №2's attach installed its channel. Keyed detach alone would
    /// clear the new sink and freeze a live pane (the tmux design was immune:
    /// every attach had its own id).
    gen: u64,
    /// The size the pty is actually at, so a request to be the size it already
    /// is can be dropped. See `next_pty_size`.
    size: PtySize,
    /// Has the ring moved since it was last persisted? Lets the slow sampler
    /// skip a tab nobody is using, which is most tabs most of the time.
    dirty: Arc<AtomicBool>,
}

/// The size to hand `MasterPty::resize`, or `None` when the pty is already
/// there. `cur` is updated on the way through, so the next request is compared
/// against what was actually applied.
///
/// Resizing a pty to its own size is not the harmless no-op it looks like. It
/// is at best a wasted ioctl; on any platform whose TIOCSWINSZ does not compare
/// first (macOS and Linux both do, today) it is a SIGWINCH — and zsh answers
/// every SIGWINCH by redrawing the prompt and whatever half-typed line is
/// sitting on it, which lands in the replay ring. Measured on a real login
/// zsh: one full redraw per size CHANGE, nothing at all for a repeat.
///
/// That matters because the ring is raw bytes replayed into a FRESH xterm on
/// every re-attach, and those redraws are cursor-relative — rendered at a width
/// other than the one they were computed for they stop overwriting each other
/// and stack up instead. One column off already leaves residue; replaying a
/// 174-column ring into an 80-column terminal stacked a pasted path four deep
/// and left the pane scrolled to the bottom, which is the bug this guards.
fn next_pty_size(cur: &mut PtySize, cols: u16, rows: u16) -> Option<PtySize> {
    let want = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
    if *cur == want {
        return None;
    }
    *cur = want;
    Some(want)
}

impl Shell {
    /// Resize the pty unless it is already that size.
    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        match next_pty_size(&mut self.size, cols, rows) {
            Some(want) => self.master.resize(want).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }
}

/// (repo, slug, 1-based tab index) — the webview never names a shell, same rule
/// as the tmux sessions.
type ShellKey = (String, String, u32);

#[derive(Default)]
struct Shells(Mutex<HashMap<ShellKey, Shell>>);

fn kill_shell(shells: &Shells, key: &ShellKey) {
    if let Some(mut sh) = shells.0.lock().unwrap().remove(key) {
        sh.stop.store(true, Ordering::Relaxed);
        let _ = sh.child.kill();
    }
}

/// Every dock shell of one place. Called when the place is closed or removed —
/// core's `kill_shell_sidecars` handles the tmux era, but it can't see this map.
fn kill_place_shells(shells: &Shells, repo: &str, slug: &str) {
    let keys: Vec<ShellKey> = shells
        .0
        .lock()
        .unwrap()
        .keys()
        .filter(|(r, s, _)| r == repo && s == slug)
        .cloned()
        .collect();
    for k in &keys {
        kill_shell(shells, k);
    }
}

/// What `shell_open` answers: the attach generation `shell_detach` must
/// present, how many bytes were pushed down the channel FIRST, and the grid
/// those bytes were laid out for.
///
/// `replay` is a live shell's ring on a re-attach — and, since tabs started
/// keeping their scrollback between runs, a fresh spawn's RESTORED ring plus its
/// seam. Both are recordings; only a spawn with nothing saved still answers 0.
///
/// The frontend needs it so it can tell the replay from live output.
/// The replay is a recording, and a recording must not be answered: any
/// terminal query in it (vim's DA2 / cursor-position / colour / cursor-blink
/// burst, say — every `git commit` without `-m` leaves one in the ring) is
/// re-issued to a FRESH xterm on every re-attach, and xterm dutifully replies
/// down the pty, where zsh echoes the printable tail of each reply onto the
/// prompt as if typed. The pane mutes its `onData` until the replay is parsed;
/// it cannot key that on "arrived before `shell_open` returned", because Tauri
/// hands a large channel payload to the page through a separate fetch that can
/// land AFTER the invoke's own response. What IS ordered is the channel: the
/// snapshot is its first message whenever `replay > 0`.
///
/// `replay_cols` is the width the recording was written at. Raw bytes do not
/// re-wrap, so replaying them into a different grid stacks every full line — the
/// defect ROADMAP.md parks under "terminal-state serialization". The pane is
/// always attaching a BRAND-NEW xterm, so it can hand the recording the grid it
/// was written for and let xterm reflow it back afterwards.
#[derive(Serialize)]
struct ShellAttach {
    gen: u64,
    replay: usize,
    replay_cols: Option<u16>,
}

/// Start (or re-attach to) the dock shell for `index` and stream it to
/// `on_bytes`; see `ShellAttach` for what it answers.
/// Idempotent: a second call for a live shell just swaps the sink and replays —
/// which is exactly what a tab flip or a dock re-open does.
///
/// The registry lock is held for the WHOLE body, spawn included. Two calls for
/// the same key can be in flight at once (StrictMode double-mounts the pane's
/// effect), and the old check-unlock-spawn-insert shape let both see an empty
/// slot: two shells spawned, the second insert winning, the first leaked as an
/// orphan no sweep could see. The spawn is a few ms and there's one webview —
/// serializing every shell command through it is the cheap correct answer.
#[tauri::command]
async fn shell_open(
    app: AppHandle,
    repo: String,
    slug: String,
    index: u32,
    cols: u16,
    rows: u16,
    on_bytes: Channel<InvokeResponseBody>,
    shells: State<'_, Shells>,
) -> Result<ShellAttach, String> {
    let key: ShellKey = (repo.clone(), slug.clone(), index);
    let mut map = shells.0.lock().unwrap();
    if let Some(sh) = map.get_mut(&key) {
        // The grid the ring was written for, read BEFORE the resize below moves
        // it. This is the pane's reflow input (see `ShellAttach::replay_cols`) —
        // a tab flip after a ⌘B or a window drag is exactly the case where the
        // recording and the terminal about to render it disagree.
        let was_cols = sh.size.cols;
        // ring THEN sink, the same order the reader takes them — otherwise a
        // write landing mid-replay is either sent twice or dropped
        let ring = sh.ring.lock().unwrap();
        let mut sink = sh.sink.lock().unwrap();
        let snapshot: Vec<u8> = ring.iter().copied().collect();
        let replay = snapshot.len();
        if replay > 0 {
            let _ = on_bytes.send(InvokeResponseBody::Raw(snapshot));
        }
        *sink = Some(on_bytes);
        drop(sink);
        drop(ring);
        // Replay THEN resize, and only if the size really moved. The order is
        // the ring/sink one above (a resize before the snapshot would let the
        // SIGWINCH redraw land in neither); the suppression is what keeps a tab
        // flip from feeding the ring a redraw per attach.
        let _ = sh.resize(cols, rows);
        sh.gen += 1;
        return Ok(ShellAttach { gen: sh.gen, replay, replay_cols: (replay > 0).then_some(was_cols) });
    }

    let (session, cwd) = place_session_cwd(&repo, &slug)?;
    // One-time cleanup for anyone upgrading: their `<session>~term*` sidecars
    // are orphans now — nothing will ever attach them again. Cheap and
    // idempotent, and only reached when this place has no shell yet. (Runs
    // under the registry lock — a tmux round-trip, but only on first open.)
    if worktrees_core::tmux::have_tmux() {
        worktrees_core::tmux::kill_shell_sidecars(&session);
    }

    let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
    let pair = native_pty_system().openpty(size).map_err(|e| e.to_string())?;

    // A LOGIN shell, like Terminal.app: it sources the user's profile, so the
    // shell has the real PATH even though this process was launched by launchd
    // with a bare one (fixup_gui_path covers our own shell-outs, not this).
    let shell_bin = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = CommandBuilder::new(&shell_bin);
    cmd.arg("-l");
    // Only the SPAWN path consults the memory: a re-attach above returned long
    // ago, and that shell's cwd is whatever the user has since cd'd to.
    let saved = remembered_dir(&read_cwds(&app), &repo, &slug, index);
    cmd.cwd(pick_start_dir(saved.as_deref(), &cwd));
    cmd.env("TERM", "xterm-256color");
    // This tab's own command history, where the shell can be pointed at one.
    // Both preferences are read ONCE here: with either on, the tree is resolved
    // (and created); with both off, nothing on disk is touched at all, which is
    // what a switch that is off should mean.
    let want_hist = PER_TAB_HISTORY.load(Ordering::Relaxed);
    let want_scroll = PERSIST_SCROLLBACK.load(Ordering::Relaxed);
    let hist_root = (want_hist || want_scroll).then(|| term_hist_dir(&app).ok()).flatten();
    if want_hist {
        if let Some(root) = hist_root.as_deref() {
            // Stamped only when a per-tab shell directory was really created:
            // that directory is the one thing here no scrollback flush would
            // write a meta for, and a directory with no meta is one no sweep can
            // ever collect.
            if apply_tab_history_env(&mut cmd, root, &key, &shell_bin).is_some() {
                ensure_meta(root, &key, cols);
            }
        }
    }

    let child = pair.slave.spawn_command(cmd).map_err(|e| {
        applog("error", &format!("shell_open {slug}#{index}: spawn {shell_bin} failed: {e}"));
        e.to_string()
    })?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    // A tab that ran before this app did starts with its own history: the ring
    // saved on the last run, replayed into the fresh xterm exactly as a tab flip
    // replays a live one.
    //
    // Sent BEFORE `sink` exists and before the reader thread is spawned, so
    // nothing the new shell writes can get in front of it — its early output
    // sits in the pty buffer meanwhile with no reader to forward it. The
    // ordering is true by construction here, not by a lock.
    let restored = want_scroll.then(|| hist_root.as_deref().and_then(|root| read_scrollback(root, &key))).flatten();
    let mut seed = VecDeque::<u8>::with_capacity(8192);
    let (mut replay, mut replay_cols) = (0usize, None);
    if let Some((bytes, meta)) = restored {
        let payload = restore_payload(&bytes, meta.at);
        if !payload.is_empty() {
            replay = payload.len();
            replay_cols = Some(meta.cols);
            // Ring and channel get the SAME bytes, seam included — a re-attach
            // replays the ring and nothing else, so anything left out of it is
            // gone the first time the tab is flipped away from. See
            // `restore_payload`.
            seed.extend(payload.iter().copied());
            let _ = on_bytes.send(InvokeResponseBody::Raw(payload));
        }
    }

    let ring = Arc::new(Mutex::new(seed));
    let sink = Arc::new(Mutex::new(Some(on_bytes)));
    let stop = Arc::new(AtomicBool::new(false));
    let dirty = Arc::new(AtomicBool::new(false));

    let (r_ring, r_sink, r_stop, r_dirty) =
        (ring.clone(), sink.clone(), stop.clone(), dirty.clone());
    let exit_key = key.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            if r_stop.load(Ordering::Relaxed) {
                return; // closed deliberately — no exit event
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    let mut ring = r_ring.lock().unwrap();
                    ring.extend(chunk.iter().copied());
                    let overflow = ring.len().saturating_sub(SHELL_RING);
                    ring.drain(..overflow);
                    r_dirty.store(true, Ordering::Relaxed);
                    // held together with the ring (see the re-attach comment)
                    if let Some(ch) = r_sink.lock().unwrap().as_ref() {
                        if ch.send(InvokeResponseBody::Raw(chunk.to_vec())).is_err() {
                            break; // webview gone
                        }
                    }
                }
                Err(_) => break,
            }
        }
        // The shell itself exited (`exit`, or it died). The tab stays — the
        // frontend offers a restart rather than silently vanishing.
        if !r_stop.load(Ordering::Relaxed) {
            let (repo, slug, index) = exit_key;
            let _ = app.emit("shell:exit", serde_json::json!({ "repo": repo, "slug": slug, "index": index }));
        }
    });

    map.insert(key, Shell { master: pair.master, writer, child, stop, ring, sink, gen: 1, size, dirty });
    Ok(ShellAttach { gen: 1, replay, replay_cols })
}

#[tauri::command]
async fn shell_write(repo: String, slug: String, index: u32, data: Vec<u8>, shells: State<'_, Shells>) -> Result<(), String> {
    let mut map = shells.0.lock().unwrap();
    let sh = map.get_mut(&(repo, slug, index)).ok_or("no such shell")?;
    sh.writer.write_all(&data).map_err(|e| e.to_string())?;
    sh.writer.flush().map_err(|e| e.to_string())
}

#[tauri::command]
async fn shell_resize(repo: String, slug: String, index: u32, cols: u16, rows: u16, shells: State<'_, Shells>) -> Result<(), String> {
    let mut map = shells.0.lock().unwrap();
    let sh = map.get_mut(&(repo, slug, index)).ok_or("no such shell")?;
    sh.resize(cols, rows)
}

/// Stop streaming; the shell keeps running. This is the unmount path — a tab
/// flip or ⌘J must not kill what you left building. `gen` names the attach
/// being undone: a detach that lost the race to a newer attach is a no-op
/// instead of clearing the newcomer's sink (see `Shell::gen`).
#[tauri::command]
async fn shell_detach(repo: String, slug: String, index: u32, gen: u64, shells: State<'_, Shells>) -> Result<(), String> {
    let map = shells.0.lock().unwrap();
    if let Some(sh) = map.get(&(repo, slug, index)) {
        if sh.gen == gen {
            *sh.sink.lock().unwrap() = None;
        }
    }
    Ok(())
}

/// GUI-launched apps inherit launchd's bare PATH (/usr/bin:/bin:…) — no
/// homebrew, no ~/.local/bin — so the engine's tmux/git shell-outs fail even
/// though they work in every terminal (tmux is homebrew-installed: every place
/// looks dead and Enter errors). Resolve the user's real PATH from their login
/// shell once at startup (marker-wrapped so chatty profiles can't corrupt it;
/// deadline-guarded so a hung profile can't block launch).
///
/// The usual install dirs are ALWAYS appended, not just when the probe fails:
/// a login shell whose profile never runs `brew shellenv` reports a PATH with no
/// /opt/homebrew/bin in it, and a brew-installed tmux would stay invisible even
/// though the probe "worked". Order is shell PATH → standard dirs → the original
/// PATH as a safety net, so the user's own resolution still wins; duplicate
/// entries are harmless.
///
/// Re-entrant on purpose: `tmux_check(refresh = true)` calls it again to pick up
/// a tmux installed after launch.
fn fixup_gui_path() {
    let current = std::env::var("PATH").unwrap_or_default();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = std::process::Command::new(&shell);
    cmd.args(["-lc", r#"printf '\n__WTPATH__%s' "$PATH""#]);
    let from_shell = run_deadline(cmd, 5)
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .rsplit("__WTPATH__")
                .next()
                .map(|p| p.trim().to_string())
        })
        .filter(|p| !p.is_empty());
    let home = std::env::var("HOME").unwrap_or_default();
    let std_dirs = format!("{home}/.local/bin:{home}/bin:/opt/homebrew/bin:/usr/local/bin");
    // dups harmless; `current` kept as the final safety net
    let path = match from_shell {
        Some(p) => format!("{p}:{std_dirs}:{current}"),
        None => format!("{std_dirs}:{current}"),
    };
    std::env::set_var("PATH", path);
    // The Codex VS Code extension bundles a usable CLI but does not always
    // put it on the GUI app's PATH. Its parent directory also lets the tmux
    // launch use the same `codex` command that MCP setup resolves below.
    if worktrees_core::profile::bin_on_path("codex").is_none() {
        if let Some(dir) = worktrees_core::profile::codex_bin().and_then(|p| p.parent().map(Path::to_path_buf)) {
            let current = std::env::var("PATH").unwrap_or_default();
            std::env::set_var("PATH", format!("{}:{current}", dir.display()));
        }
    }
}

/// Is tmux reachable right now? `refresh = true` re-resolves the GUI PATH first,
/// so a tmux installed AFTER the app launched is picked up without a restart
/// (startup resolves PATH exactly once). Async is not optional here: the
/// login-shell probe inside `fixup_gui_path` is deadline-guarded at 5s, and a
/// sync handler would spend all of it frozen on the main thread.
#[tauri::command]
async fn tmux_check(refresh: bool) -> Result<bool, String> {
    if !refresh {
        return Ok(worktrees_core::tmux::have_tmux());
    }
    let before = worktrees_core::tmux::have_tmux();
    fixup_gui_path();
    let after = worktrees_core::tmux::have_tmux();
    if after && !before {
        applog(
            "info",
            &format!("tmux_check: tmux found after PATH refresh; PATH={}", std::env::var("PATH").unwrap_or_default()),
        );
    }
    Ok(after)
}

/// App-wide zoom — WKWebView's `setPageZoom`, driven by ⌘+ / ⌘− / ⌘0.
///
/// This is deliberately NOT tauri's `zoomHotkeysEnabled`. That option injects a
/// script (`tauri/src/webview/scripts/zoom-hotkey.js`) whose `window` keydown
/// listener never checks `defaultPrevented` — so it would fire ALONGSIDE the
/// frontend's own handler — keeps the level in a script-local variable that
/// desyncs from any programmatic call, steps by a coarse 0.2, and forgets the
/// level on every restart. The frontend owns the step table and persists it in
/// `ui-state.json`; this command is the one line of platform underneath.
///
/// Page zoom is what makes the knob reach the TERMINAL: it shrinks the CSS-px
/// viewport, TerminalPane's ResizeObserver refits, and the tmux pane is resized
/// to the new cols/rows. `--ui-rem` alone can never do that (tokens.css keeps
/// `--term-size` independent on purpose).
#[tauri::command]
async fn set_zoom(window: tauri::WebviewWindow, factor: f64) -> Result<(), String> {
    // A NaN or a wild factor here is a blank window the user cannot zoom back
    // out of — the clamp is the frontend's, repeated because this is an IPC
    // boundary and the frontend is not the only possible caller.
    if !factor.is_finite() {
        return Err("zoom factor is not a finite number".into());
    }
    let factor = factor.clamp(0.5, 3.0);
    window.set_zoom(factor).map_err(|e| {
        applog("error", &format!("set_zoom({factor}): {e}"));
        e.to_string()
    })
}

/// Same launchd-bare-env problem as PATH, but for locale: GUI-launched apps
/// have no LC_ALL/LC_CTYPE/LANG, so everything we spawn (tmux server via the
/// engine's shell-outs, shells inside sessions) runs locale-less. The embedded
/// tmux client is already covered by `-u` in term_open; this covers the server
/// side when this app is the first tmux invocation.
fn fixup_gui_locale() {
    let has_utf8 = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .any(|v| v.to_uppercase().contains("UTF-8") || v.to_uppercase().contains("UTF8"));
    if !has_utf8 {
        std::env::set_var("LANG", "en_US.UTF-8");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // panics land in the log too (chained: the default stderr hook still runs)
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        applog("panic", &info.to_string());
        prev_hook(info);
    }));
    fixup_gui_path();
    fixup_gui_locale();
    applog(
        "info",
        &format!("startup v{} PATH={}", env!("CARGO_PKG_VERSION"), std::env::var("PATH").unwrap_or_default()),
    );
    tauri::Builder::default()
        // `open_path` (the Files tab's right-click → Open) needs BOTH halves,
        // and JSON cannot carry the reason: `opener:allow-open-path` in
        // capabilities/default.json plus a path scope, because the plugin's
        // `is_path_allowed` ANDs the fs scope with "some allowed entry names a
        // path" — the permission on its own allows nothing. The scope is `**`,
        // and tauri.conf.json turns `requireLiteralLeadingDot` off so that
        // pattern also covers a path with a dot component: every worktree this
        // app opens lives under `.worktrees/`, and with the unix default the
        // invoke would reject exactly those — silently.
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(Terminals::default())
        .manage(Shells::default())
        .manage(viewer::Viewer::default())
        .setup(|app| {
            // A crash is the one exit `RunEvent::Exit` never sees, and it
            // leaves the derived trees on disk: COPIES of the user's documents
            // — the ones §4.3 says carry a client's signed agreement — outside
            // the repo's own gitignore, in a directory Spotlight and Time
            // Machine both index. Sweeping them at startup closes the half of
            // that a shutdown hook structurally cannot.
            //
            // This does not break the rule that nothing about the viewer runs
            // at launch: it binds no port, reads no configuration and cannot
            // fail in a way that matters — it empties one directory this app
            // owns, ignoring whatever will not go. The server itself is still
            // unreachable from startup; the first press of the button is what
            // starts it.
            if let Ok(dir) = app.path().app_config_dir() {
                viewer::cleanup(&dir);
            }
            // macOS 26 floats a Writing Tools affordance (AppKit's Campo
            // lightweight UI) over any selection, and hovering the one it puts
            // over the webview trips an assertion inside AppKit —
            // NSCampoLightweightUIController.m:1429, three times in app.log.
            // The NSException that assertion raises unwinds out through tao's
            // `sendEvent:` override, which is `extern "C"` and therefore
            // nounwind, so Rust turns the foreign unwind into an abort: SIGABRT
            // mid-keystroke, tmux sessions untouched. The only trace is a bare
            // "panic in a function that cannot unwind" with NO panic line before
            // it — because no Rust panic ever happened.
            //
            // Selecting text is routine here (the tab-rename box selects on
            // focus, the terminal selects on drag), so take the affordance away.
            // AppKit asks the view for `allowsWritingToolsAffordance`; WKWebView
            // answers YES and offers no setter, and the settable knob
            // (`WKWebViewConfiguration.writingToolsBehavior`) only exists before
            // the webview is built — tauri creates this window from
            // tauri.conf.json, so we never hold that configuration. Overriding
            // the getter on wry's own WKWebView subclass is the small version:
            // one added method, our instances only, no Apple class touched.
            #[cfg(target_os = "macos")]
            for (_, win) in app.webview_windows() {
                // Every webview shares the one registered wry class, so the
                // method goes on once per process: a second window's add would
                // fail (the class implements it by then) and warn about a fix
                // that is in place and working.
                static AFFORDANCE: std::sync::Once = std::sync::Once::new();
                let r = win.with_webview(|pw| unsafe {
                    AFFORDANCE.call_once(|| {
                        let view = &*(pw.inner() as *mut objc2::runtime::AnyObject);
                        // Step past the KVO subclass AppKit swizzles in (wry
                        // observes the document title): the instance's isa
                        // reverts to the original class when observation ends,
                        // and a method added on the notifying class goes out of
                        // reach with it. wry's own class is the stable place —
                        // a KVO subclass made later inherits the override.
                        let mut cls = Some(view.class());
                        while let Some(c) = cls {
                            let name = c.name().to_string_lossy().to_string();
                            if name.contains("WryWebView") && !name.contains("NSKVONotifying") {
                                break;
                            }
                            cls = c.superclass();
                        }
                        let Some(c) = cls else {
                            applog("warn", "writing-tools affordance: wry webview class not found");
                            return;
                        };
                        extern "C" fn no(
                            _this: &objc2::runtime::AnyObject,
                            _sel: objc2::runtime::Sel,
                        ) -> objc2::runtime::Bool {
                            objc2::runtime::Bool::NO
                        }
                        // BOOL encodes as `B` on arm64 and `c` on x86_64 — take
                        // it from objc2 rather than picking one.
                        let types = std::ffi::CString::new(format!(
                            "{}@:",
                            <objc2::runtime::Bool as objc2::encode::Encode>::ENCODING
                        ))
                        .expect("no interior nul in an objc type encoding");
                        let added = objc2::ffi::class_addMethod(
                            c as *const objc2::runtime::AnyClass as *mut objc2::runtime::AnyClass,
                            objc2::sel!(allowsWritingToolsAffordance),
                            std::mem::transmute::<
                                extern "C" fn(&objc2::runtime::AnyObject, objc2::runtime::Sel) -> objc2::runtime::Bool,
                                unsafe extern "C-unwind" fn(),
                            >(no),
                            types.as_ptr(),
                        );
                        if !added.as_bool() {
                            applog("warn", "writing-tools affordance: class_addMethod failed");
                        }
                    });
                });
                // A failure here leaves the app crashing on a selection with a
                // log that looks exactly like the fix landed — say so instead.
                if let Err(e) = r {
                    applog("warn", &format!("writing-tools affordance: with_webview failed: {e}"));
                }
            }
            // Live-refresh, change-gated. The tmux session set is a cheap
            // fingerprint that shifts whenever a place is opened/closed (even from a
            // bare terminal); emit only when it changes, so the UI's full re-pull (a
            // git sweep) doesn't fire every few seconds. A slow safety re-emit
            // (~30s) still catches dirty/branch drift that leaves no tmux trace.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                // Claude's real working state — read from ~/.claude/sessions/<pid>.json
                // probes (see claude_activity), keyed by worktree path. Replaces the
                // old tmux #{session_activity} signal, which tracked CLIENT attach/
                // keypress, not pane output — so the dot decayed while Claude worked.
                let mut last = worktrees_core::tmux::session_fingerprint();
                let mut last_busy: Vec<String> = Vec::new();
                let mut last_waiting: Vec<String> = Vec::new();
                let mut ticks: u32 = 0;
                // Auto-fetch scheduling. The pass runs INLINE on this thread (no
                // extra thread → passes can never stack; the AtomicU64 interval is
                // pushed from the frontend). Trade-off: while a pass runs, the 3s
                // tmux-fingerprint poll below is paused for up to ~60s per repo —
                // acceptable, since a stale tmux fingerprint only delays a refresh
                // the fetch itself will trigger via places:changed at the end.
                let mut last_fetch = std::time::Instant::now();
                // Consecutive ticks each path has been busy — the dwell guard's
                // memory, and the only state a completion edge needs.
                let mut busy_ticks: HashMap<String, u32> = HashMap::new();
                // Dock-shell cwd sampling, every 5th tick (~15s). Slow on
                // purpose: the exit hook is the accurate capture, this one only
                // has to bound how much a crash or a force-quit can lose.
                let mut cwd_ticks: u32 = 0;
                // The unsent-prompt sample, on the same 15s beat and for the
                // same reason: it costs a subprocess, and a draft is a thing a
                // human is typing — 15s is faster than anyone notices.
                let mut draft_ticks: u32 = 0;
                let mut last_drafts: Vec<Draft> = Vec::new();
                // Logged separately from the emit gate: the SET changes every
                // time someone adds a word to a prompt, and app.log rotates at
                // 1MB. The counts are the part worth keeping.
                let mut last_draft_counts = (usize::MAX, usize::MAX);
                // Only the first of a repeating tmux failure is worth a line.
                let mut last_draft_err = String::new();
                // …and only the first of a repeating viewer-refresh line.
                //
                // A SET, not the last message: the tick emits one line per
                // failing place and one per image a derive refused, so two
                // places (or one place and one skipped screenshot) alternate
                // and a one-slot dedup logs both of them every 3 s forever.
                // Cleared when a tick has nothing to say, so a fault that comes
                // back after a recovery is heard again, and capped so a place
                // producing fresh text on every tick cannot grow it without
                // bound.
                let mut said_viewer: std::collections::HashSet<String> = std::collections::HashSet::new();
                // Cold start: what happened while the app was closed. Runs before
                // the first sleep so the nav's afterglow is right on frame one.
                backfill_worked(&handle);
                // Same cold-start slot: drop remembered directories for places
                // that were removed while the app was closed.
                forget_vanished_places(&handle);
                // Same cold-start slot, same reasoning one level out: drop saved
                // scrollback for places that are gone, and anything past the age
                // horizon.
                sweep_term_history(&handle);
                loop {
                    std::thread::sleep(Duration::from_secs(3));
                    let interval = FETCH_INTERVAL_SECS.load(Ordering::Relaxed);
                    if interval > 0 && last_fetch.elapsed().as_secs() >= interval {
                        for root in read_projects(&handle) {
                            let _ = fetch_origin_root(&root);
                        }
                        last_fetch = std::time::Instant::now(); // measure gap AFTER the pass
                        let _ = handle.emit("places:changed", ()); // re-pull fresh ahead/behind once
                    }
                    // Every tick, not every fifth: a request has a 30 s life
                    // and a person is waiting on it.
                    drain_inbox(&handle);
                    // And the browser's copy of every place the server is
                    // serving. Every tick for the same reason: somebody is
                    // watching an agent write, and the page polls its `ETag`
                    // about once a second — a slower beat here would be a
                    // browser tab that is visibly behind the dock beside it,
                    // with a conditional GET saying nothing has changed.
                    //
                    // It costs a stat per document per registered place and
                    // NOTHING when none is registered, which is the normal
                    // state; `viewer::refresh` is where that is argued. It is
                    // not gated on window visibility, unlike the pane's own
                    // poll: the whole point of the browser viewer is the second
                    // monitor, so "the app is hidden" is exactly when the tab
                    // most needs to be current.
                    let notes = viewer::refresh(&handle.state::<viewer::Viewer>(), sysclock::now_epoch());
                    if notes.is_empty() {
                        said_viewer.clear();
                    }
                    for n in notes {
                        // Deduped, like the draft scan below: a place that
                        // cannot be written will fail every 3 s forever, and a
                        // log that repeats that is a log nobody reads. The
                        // LEVEL is the tick's, not this call site's: a skipped
                        // image is the same event `open_docs_viewer` logs at
                        // "warn", and saying it louder here made one event two.
                        if said_viewer.len() > 64 {
                            said_viewer.clear();
                        }
                        if said_viewer.insert(n.msg.clone()) {
                            applog(n.level, &format!("viewer refresh: {}", n.msg));
                        }
                    }
                    cwd_ticks += 1;
                    if cwd_ticks >= 5 {
                        cwd_ticks = 0;
                        save_shell_cwds(&handle, &handle.state::<Shells>());
                        save_shell_scrollback(&handle, &handle.state::<Shells>());
                    }
                    let fp = worktrees_core::tmux::session_fingerprint();
                    ticks += 1;
                    if fp != last || ticks >= 10 {
                        last = fp.clone(); // `fp` is the draft sample's session list too
                        ticks = 0;
                        let _ = handle.emit("places:changed", ());
                    }
                    draft_ticks += 1;
                    if draft_ticks >= 5 {
                        draft_ticks = 0;
                        // `fp` is the session list this tick already paid for,
                        // so the whole pass is ONE extra tmux spawn per 15s
                        // regardless of how many sessions are open.
                        match scan_drafts(&fp) {
                            Ok(drafts) => {
                                last_draft_err.clear();
                                if trace_spawns() {
                                    applog("info", "trace: draft sample = 1 tmux spawn (chained)");
                                }
                                // Emit on EVERY change, the transition to empty
                                // included: a draft the user just sent must
                                // take its ✎ with it.
                                if drafts != last_drafts {
                                    if let Some(f) = draft_trace_filter() {
                                        for d in drafts.iter().filter(|d| d.path.contains(&f)) {
                                            let head: String = d.text.chars().take(20).collect();
                                            applog("info", &format!(
                                                "draft {} queued={} {:?}", d.path, d.queued, head));
                                        }
                                    }
                                    let counts =
                                        (drafts.len(), drafts.iter().filter(|d| d.queued).count());
                                    if counts != last_draft_counts {
                                        last_draft_counts = counts;
                                        applog("info", &format!(
                                            "drafts: {} ({} queued)", counts.0, counts.1));
                                    }
                                    last_drafts = drafts.clone();
                                    let _ = handle.emit("sessions:drafts", Drafts { drafts });
                                }
                            }
                            // Keep the last known set rather than blanking every
                            // ✎ on one bad tick (a pane that died mid-chain).
                            Err(e) => {
                                if e != last_draft_err {
                                    last_draft_err = e.clone();
                                    applog("warn", &format!("draft sample: {e}"));
                                }
                            }
                        }
                    }
                    let (mut busy, mut waiting) = claude_activity();
                    busy.sort_unstable();
                    waiting.sort_unstable();
                    // Two sessions in the SAME dir each push their cwd. The
                    // frontend already de-dupes into a Set, but the dwell
                    // counter would not: a doubled path would qualify in one
                    // tick instead of two and let a blip through.
                    busy.dedup();
                    waiting.dedup();
                    // Change-gated: emit only when EITHER set shifts, so an idle
                    // machine stays silent (the frontend just re-applies the last set).
                    if busy != last_busy || waiting != last_waiting {
                        last_busy = busy.clone();
                        last_waiting = waiting.clone();
                        let _ = handle.emit(
                            "sessions:busy",
                            ClaudeActivity { busy: busy.clone(), waiting },
                        );
                    }
                    // Completion edges, computed AFTER the busy emit so the dot's
                    // hand-off (green out, ember in) arrives in that order.
                    let exits = completion_edges(&mut busy_ticks, &busy);
                    if !exits.is_empty() {
                        // read_projects only on a real edge — not every 3s tick
                        let roots = read_projects(&handle);
                        let epoch = sysclock::now_epoch();
                        for path in exits {
                            if stamp_worked(&roots, &path, epoch) {
                                let _ = handle.emit("sessions:done", TaskDone { path, epoch });
                            }
                        }
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_places,
            list_workspace,
            drop_reference,
            add_project,
            create_project,
            probe_dir,
            init_repo,
            create_initial_commit,
            remove_project,
            reorder_projects,
            set_lifecycle,
            set_pin,
            set_note,
            set_title,
            touch_place,
            mark_seen,
            new_place,
            switch_place,
            list_branches,
            remove_place,
            open_place,
            close_place,
            remote_url,
            fetch_origin,
            set_fetch_interval,
            check_update,
            update_cli,
            get_ai_config,
            mcp_status,
            mcp_install,
            mcp_uninstall,
            codex_mcp_status,
            codex_mcp_install,
            codex_mcp_uninstall,
            project_config_read,
            doctor,
            place_health,
            list_automations,
            upsert_automation,
            delete_automation,
            run_automation,
            list_runs,
            get_run,
            apply_proposal,
            ai_status_report,
            relink,
            provision,
            sync_status,
            sync_hub_list,
            sync_preview,
            sync_apply,
            init_suggest,
            init_write,
            diagnostics,
            tmux_check,
            set_zoom,
            claude_usage,
            claude_status,
            log_info,
            list_drafts,
            log_event,
            log_tail,
            get_changelog,
            open_editor,
            open_terminal,
            list_dir,
            changed_files,
            file_diff,
            read_file,
            file_readable,
            list_docs,
            place_plan,
            plan_prompt,
            open_docs_viewer,
            read_file_base64,
            write_file,
            list_shell_sessions,
            close_shell_session,
            shell_open,
            shell_write,
            shell_resize,
            shell_detach,
            settings_info,
            profiles_info,
            save_profile,
            new_profile_id,
            delete_profile,
            set_project_profile,
            set_default_profile,
            skills_list,
            skill_inspect,
            skill_install_local,
            skill_preview_git,
            skill_install_git,
            skill_remove,
            get_settings,
            set_settings,
            term_open,
            term_write,
            term_resize,
            term_close,
            set_term_history_opts,
            term_history_info,
            term_history_clear,
            ui_events_append,
            ui_usage,
            ui_events_clear
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        // Owned dock shells die with the app — they have no tmux server holding
        // them up, so without this the PTY children outlive the window as
        // orphaned logins.
        .run(|handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let shells = handle.state::<Shells>();
                // Where each tab ended up, recorded BEFORE the sweep — a killed
                // shell has no cwd left to read.
                save_shell_cwds(handle, &shells);
                // …and what each tab was SHOWING, likewise before the sweep: a
                // killed shell still has its ring, but it will not have it long.
                save_shell_scrollback(handle, &shells);
                let keys: Vec<ShellKey> = shells.0.lock().unwrap().keys().cloned().collect();
                for k in &keys {
                    kill_shell(&shells, k);
                }
                // And the documentation server, for a harder reason than the
                // shells': what it is holding is a loopback port onto the
                // user's documents. Surviving the window would mean serving
                // them to whatever finds the port, with nothing on screen to
                // say it is still running.
                viewer::kill(&handle.state::<viewer::Viewer>());
                if let Ok(d) = handle.path().app_config_dir() {
                    viewer::cleanup(&d);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    // ── Claude plan usage: what a failed fetch answers ──────────────────────
    //
    // Written after an episode on 2026-09-19 where the oauth endpoint 429'd for
    // ~4m45s and the rail tile disappeared for the whole of it, while the app
    // spent 15 real fetches inside one minute keeping the rate limit warm.

    fn reading(source: &str, fetched_at: i64, pct: f64) -> UsageInfo {
        UsageInfo {
            source: source.into(),
            fetched_at,
            limits: vec![UsageLimit {
                kind: "session".into(),
                label: "Session".into(),
                percent: pct,
                severity: "normal".into(),
                resets_at: None,
            }],
        }
    }

    /// The fix, stated: a failed poll keeps the numbers on screen. `source`
    /// changes so the frontend dims it and names what it is showing; the
    /// reading itself — including `fetched_at`, which the tooltip prints — is
    /// untouched, because inventing a fresh timestamp for old data would be the
    /// one thing worse than hiding it.
    #[test]
    fn a_failed_fetch_degrades_to_the_last_good_reading() {
        let out = usage_degraded(1_000, Some(reading("oauth", 870, 35.0)), None);
        assert_eq!(out.source, "cached");
        assert_eq!(out.fetched_at, 870, "the age is the reading's, not now's");
        assert_eq!(out.limits[0].percent, 35.0);
    }

    /// Bounded, though: the bars measure a 5h and a 7d window, so a reading
    /// from before one rolled over is not stale, it is wrong. Inclusive at the
    /// bound, like `stale_or_silence` — the sibling probe that already made
    /// this bargain, and whose boundary test this one is written to match.
    #[test]
    fn a_reading_past_the_stale_ceiling_is_dropped() {
        let at = |age: i64| Some(reading("oauth", 1_000 - age, 35.0));
        assert_eq!(usage_degraded(1_000, at(USAGE_STALE_MAX_SECS), None).source, "cached", "at the bound, inclusive");
        assert_eq!(usage_degraded(1_000, at(USAGE_STALE_MAX_SECS + 1), None).source, "unavailable");
    }

    /// The cached reading beats a statusline snapshot even when the snapshot is
    /// NEWER, which is the one place this does not prefer fresher data. The
    /// statusline rewrites its file on every Claude Code prompt, so on a machine
    /// that runs one the snapshot is almost always newer — and it is two rows
    /// with no severity and no model bucket. Preferring it would drop a bar for
    /// the length of an outage and put it back afterwards, which is the layout
    /// moving exactly when this function exists to hold it still.
    #[test]
    fn the_cached_reading_outranks_a_fresher_snapshot() {
        let out = usage_degraded(1_000, Some(reading("oauth", 800, 35.0)), Some(reading("statusline", 995, 61.0)));
        assert_eq!(out.source, "cached");
        assert_eq!(out.limits[0].percent, 35.0, "…and it is the cached NUMBERS, not just the label");
        // once it is past the ceiling the snapshot is all that is left, and then
        // it is served whatever its age
        let out = usage_degraded(1_000, Some(reading("oauth", 1_000 - USAGE_STALE_MAX_SECS - 1, 35.0)), Some(reading("statusline", 995, 61.0)));
        assert_eq!(out.source, "statusline");
        assert_eq!(out.limits[0].percent, 61.0);
    }

    /// Hiding is still the answer when there is nothing left to show — and a
    /// statusline snapshot on its own is still served, ceiling or no ceiling:
    /// it carries its own honest mtime and has always been allowed to be old.
    #[test]
    fn with_nothing_held_the_widget_still_hides() {
        assert_eq!(usage_degraded(1_000, None, None).source, "unavailable");
        assert!(usage_degraded(1_000, None, None).limits.is_empty());
        let ancient = usage_degraded(1_000, None, Some(reading("statusline", 1, 61.0)));
        assert_eq!(ancient.source, "statusline");
    }

    /// The other half: a failure now buys the same silence a success does.
    /// Before this, nothing was cached on failure, so the 120s floor never
    /// applied and every window-focus pull went straight back to the endpoint.
    #[test]
    fn a_failure_holds_off_the_next_fetch() {
        assert!(!usage_in_backoff(1_000, None), "no failure recorded, no backoff");
        assert!(usage_in_backoff(1_000, Some(1_000 - USAGE_FAIL_TTL_SECS + 1)));
        assert!(!usage_in_backoff(1_000, Some(1_000 - USAGE_FAIL_TTL_SECS)), "the floor is a floor, not a ceiling");
        // the shape that broke: a burst of pulls seconds after a 429
        assert!(usage_in_backoff(1_000, Some(999)));
    }

    // ── Claude service status ───────────────────────────────────────────────

    /// A trimmed `summary.json`: six components as the real page carries them,
    /// plus whatever incidents the caller wants.
    fn status_body(code: &str, api: &str, incidents: &str) -> String {
        format!(
            r#"{{
              "page": {{"id":"tymt9n04zgry","name":"Claude","url":"https://status.claude.com"}},
              "components": [
                {{"id":"rwppv331jlwc","name":"claude.ai","status":"major_outage"}},
                {{"id":"0qbwn08sd68x","name":"Claude Console (platform.claude.com)","status":"operational"}},
                {{"id":"k8w3r06qmzrp","name":"Claude API (api.anthropic.com)","status":"{api}"}},
                {{"id":"yyzkbfz2thpt","name":"Claude Code","status":"{code}"}},
                {{"id":"bpp5gb3hpjcl","name":"Claude Cowork","status":"operational"}},
                {{"id":"0scnb50nvy53","name":"Claude for Government","status":"operational"}}
              ],
              "incidents": [{incidents}],
              "scheduled_maintenances": [],
              "status": {{"indicator":"critical","description":"Major outage"}}
            }}"#
        )
    }

    /// The whole reason this does not read `status.indicator`: the page's own
    /// verdict covers six components and this app feels two. claude.ai is in a
    /// major outage in every body above and the page calls itself critical —
    /// and a worktree does not care.
    #[test]
    fn severity_ignores_components_this_app_does_not_use() {
        let s = status_parse(&status_body("operational", "operational", ""), 100).unwrap();
        assert_eq!(s.severity, "none");
        assert_eq!(s.components.len(), 2, "only the watched two are reported");
        assert_eq!(s.components[0].name, "Claude Code");
        assert_eq!(s.total, 6, "…out of the six the page carries");
    }

    #[test]
    fn severity_is_the_worst_watched_component() {
        let deg = status_parse(&status_body("operational", "degraded_performance", ""), 100).unwrap();
        assert_eq!(deg.severity, "degraded");
        let part = status_parse(&status_body("partial_outage", "operational", ""), 100).unwrap();
        assert_eq!(part.severity, "degraded");
        let down = status_parse(&status_body("major_outage", "degraded_performance", ""), 100).unwrap();
        assert_eq!(down.severity, "down", "down wins over degraded");
    }

    /// A state Statuspage has not shipped yet must not read as "fine".
    #[test]
    fn an_unknown_component_state_counts_as_degraded() {
        let s = status_parse(&status_body("operational", "on_fire", ""), 100).unwrap();
        assert_eq!(s.severity, "degraded");
    }

    /// Ids are the match, so a retitled component keeps being watched — and the
    /// name we SHOW is the page's current one, not our stale prefix.
    #[test]
    fn a_renamed_component_is_still_matched_by_id() {
        let body = status_body("operational", "operational", "")
            .replace("Claude Code", "Claude Code (CLI + app)");
        let s = status_parse(&body, 100).unwrap();
        assert_eq!(s.components[0].name, "Claude Code (CLI + app)");
    }

    /// And a re-ISSUED component — new id, same name — is matched by the name
    /// prefix, which is the only reason the prefix is carried at all.
    #[test]
    fn a_reissued_component_is_still_matched_by_name() {
        let body = status_body("major_outage", "operational", "").replace("yyzkbfz2thpt", "newidnewid00");
        let s = status_parse(&body, 100).unwrap();
        assert_eq!(s.severity, "down");
    }

    /// Both watched components gone is the page's shape moving under us. An
    /// all-clear would be a lie told exactly when we can no longer tell.
    #[test]
    fn losing_every_watched_component_is_an_error_not_an_all_clear() {
        let body = status_body("operational", "operational", "")
            .replace("yyzkbfz2thpt", "gone1")
            .replace("Claude Code", "Something Else")
            .replace("k8w3r06qmzrp", "gone2")
            .replace("Claude API (api.anthropic.com)", "Other API");
        assert!(status_parse(&body, 100).is_err());
    }

    const INC_API: &str = r#"{
        "name":"Elevated errors on the Messages API","status":"monitoring",
        "shortlink":"https://stspg.io/abc","updated_at":"2026-09-18T15:00:00Z",
        "components":[{"id":"k8w3r06qmzrp"}],
        "incident_updates":[
          {"body":"A fix has been applied and error rates are recovering.","updated_at":"2026-09-18T15:04:00Z"},
          {"body":"We are investigating.","updated_at":"2026-09-18T14:30:00Z"}
        ]
    }"#;
    const INC_OTHER: &str = r#"{
        "name":"Issues with Google Play subscriptions","status":"investigating",
        "components":[{"id":"rwppv331jlwc"}],
        "incident_updates":[{"body":"Unrelated.","updated_at":"2026-09-18T15:04:00Z"}]
    }"#;

    #[test]
    fn the_incident_shown_is_one_that_names_a_watched_component() {
        let body = status_body("operational", "degraded_performance", &format!("{INC_OTHER},{INC_API}"));
        let inc = status_parse(&body, 100).unwrap().incident.expect("an incident");
        assert_eq!(inc.name, "Elevated errors on the Messages API");
        // the LATEST update's body, which is the sentence worth showing
        assert!(inc.body.unwrap().starts_with("A fix has been applied"));
        assert_eq!(inc.updated_at, Some(1789743840));
        assert_eq!(inc.url.as_deref(), Some("https://stspg.io/abc"));
    }

    #[test]
    fn an_incident_on_an_unwatched_component_is_not_reported() {
        let body = status_body("operational", "operational", INC_OTHER);
        let s = status_parse(&body, 100).unwrap();
        assert!(s.incident.is_none());
        assert_eq!(s.severity, "none");
    }

    #[test]
    fn a_body_that_is_not_the_status_page_is_an_error() {
        assert!(status_parse("not json", 100).is_err());
        assert!(status_parse(r#"{"page":{}}"#, 100).is_err());
    }

    /// A pull that lands before the network is back must not turn a live
    /// incident into silence — but a cached outage may not outlive the outage
    /// either. Both halves of that bargain, at the boundary.
    #[test]
    fn a_failed_fetch_serves_the_last_answer_only_while_it_can_still_be_true() {
        let at = |t: i64| Some(ClaudeStatus {
            source: "live".into(), fetched_at: t, severity: "down".into(),
            components: vec![StatusComponent { name: "Claude Code".into(), status: "major_outage".into() }],
            total: 6, incident: None, page_url: STATUS_PAGE.into(),
        });
        let now = 1_000_000;
        // fresh-ish and expired: still the truest thing we have
        assert_eq!(stale_or_silence(at(now - 60), now).unwrap().severity, "down");
        assert!(stale_or_silence(at(now - STATUS_STALE_MAX_SECS), now).is_some(), "at the bound, inclusive");
        // past the bound the outage may well be over; silence beats a stale alarm
        assert!(stale_or_silence(at(now - STATUS_STALE_MAX_SECS - 1), now).is_none());
        // nothing cached at all
        assert!(stale_or_silence(None, now).is_none());
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

    /// A nav drag sends the order it can SEE. The file is the truth, and the
    /// window that dragged may be looking at a stale copy of it — so the merge
    /// has to be lossless in both directions: nothing invented, nothing lost.
    #[test]
    fn reordering_projects_can_never_add_or_drop_one() {
        let current = v(&["/a", "/b", "/c"]);
        assert_eq!(
            merge_project_order(&current, v(&["/c", "/a", "/b"])),
            v(&["/c", "/a", "/b"]),
            "a plain permutation applies verbatim",
        );
        assert_eq!(
            merge_project_order(&current, v(&["/c", "/gone", "/a"])),
            v(&["/c", "/a", "/b"]),
            "a root the file no longer has is dropped, not written back",
        );
        assert_eq!(
            merge_project_order(&current, v(&["/c"])),
            v(&["/c", "/a", "/b"]),
            "roots the dragger never saw keep their order, appended",
        );
        assert_eq!(
            merge_project_order(&current, v(&["/b", "/b", "/a"])),
            v(&["/b", "/a", "/c"]),
            "a repeated root is taken once, at its first position",
        );
        assert_eq!(
            merge_project_order(&current, vec![]),
            current,
            "an empty request is a no-op, not a wipe",
        );
    }

    /// A cut patch must end on a line boundary. Half a line of source is
    /// indistinguishable from a real line once it reaches the grid — it renders
    /// as content, with a line number, and nothing says it is a fragment.
    #[test]
    fn trim_patch_cuts_on_a_line_boundary() {
        let small = "@@ -1,1 +1,1 @@\n-a\n+b\n";
        let (out, cut) = trim_patch(small);
        assert_eq!(out, small, "a patch under the cap is returned untouched");
        assert!(!cut);

        // One line per 12 bytes, so the cap lands mid-line rather than on a
        // boundary by luck — a cap that happened to fall on "\n" would pass
        // whether or not the boundary logic existed.
        let big: String = std::iter::repeat(" 0123456789\n").take(DIFF_MAX / 12 + 200).collect();
        let (out, cut) = trim_patch(&big);
        assert!(cut, "past the cap it reports truncation");
        assert!(out.len() <= DIFF_MAX, "never longer than the cap: {}", out.len());
        assert!(out.ends_with('\n'), "cut on a boundary, not mid-line");
        assert!(big.starts_with(&out), "the kept part is a prefix of the original");
    }

    /// Multi-byte characters must not be split. `is_char_boundary` is the guard;
    /// without it `&patch[..DIFF_MAX]` panics instead of returning a short patch.
    #[test]
    fn trim_patch_respects_utf8_boundaries() {
        // The fixture only tests the guard if the cap lands INSIDE a character,
        // and whether it does is pure arithmetic on the line length: the first
        // version of this test used a 7-byte line, `DIFF_MAX % 7 == 2` fell on a
        // boundary, and the test passed with the guard deleted. Hence the
        // assertion below — it fails loudly if a future DIFF_MAX makes the
        // fixture stop straddling.
        let big: String = std::iter::repeat(" …abcd\n").take(DIFF_MAX / 9 + 200).collect();
        assert!(
            !big.is_char_boundary(DIFF_MAX),
            "fixture no longer straddles a character at the cap — it would pass without the guard",
        );
        let (out, cut) = trim_patch(&big);
        assert!(cut);
        assert!(out.len() <= DIFF_MAX);
        assert!(out.ends_with('\n'), "still cut on a line boundary");
        assert!(big.starts_with(&out));
    }

    /// Real repos, because the whole question is what git exits with — a fake
    /// would be asserting my own assumptions back at me. Both cases below
    /// rendered as "no changes" before the fix, over a row the tree had lit.
    #[test]
    fn a_repo_with_no_commits_reports_every_line_as_new() {
        let base = std::env::temp_dir().join(format!("wt-unborn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let cwd = base.to_string_lossy().to_string();
        assert!(git::git_ok(&cwd, &["init", "-q"]), "git init");
        let f = base.join("f.txt");
        std::fs::write(&f, "hello\n").unwrap();
        // STAGED, so `ls-files --error-unmatch` succeeds and the untracked arm
        // above does NOT catch it — that is exactly what made this reachable.
        assert!(git::git_ok(&cwd, &["add", "f.txt"]), "git add");
        assert!(!git::has_commits(&cwd), "fixture must have an unborn HEAD");
        assert!(
            git::git_ok(&cwd, &["ls-files", "--error-unmatch", "--", "f.txt"]),
            "fixture must be TRACKED, or it proves nothing",
        );

        let d = file_diff_for(&f, false).expect("must not error");
        assert!(d.untracked, "no commits means no before: every line is new");
        assert!(d.patch.is_empty(), "the frontend builds the rows from content");
        assert!(!d.binary);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A tracked file in a normal repo still diffs — the guard above must not
    /// have swallowed the ordinary path.
    #[test]
    fn a_committed_change_still_produces_a_patch() {
        let base = std::env::temp_dir().join(format!("wt-diffok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let cwd = base.to_string_lossy().to_string();
        assert!(git::git_ok(&cwd, &["init", "-q"]));
        let f = base.join("f.txt");
        std::fs::write(&f, "one\n").unwrap();
        assert!(git::git_ok(&cwd, &["add", "-A"]));
        assert!(git::git_ok(&cwd, &["-c", "user.email=s@s", "-c", "user.name=s", "commit", "-qm", "base"]));
        std::fs::write(&f, "one\ntwo\n").unwrap();

        let d = file_diff_for(&f, true).expect("must not error");
        assert!(!d.untracked, "the file is tracked and has a committed version");
        assert!(d.patch.contains("+two"), "the working-tree change must be in the patch: {:?}", d.patch);
        assert_eq!(d.against, "head");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn binary_patches_are_recognised() {
        assert!(patch_is_binary("diff --git a/x b/x\nBinary files a/x and b/x differ\n"));
        assert!(patch_is_binary("diff --git a/x b/x\nGIT binary patch\nliteral 12\n"));
        // A source line that merely mentions the phrase is not a marker: the
        // test is on the LINE, and this one is a diff body line, so it starts
        // with a marker character.
        assert!(!patch_is_binary("@@ -1 +1 @@\n+// Binary files are skipped here\n"));
    }

    /// Real symlinks on a real filesystem — the whole point of the classifier is
    /// what the syscalls answer, so a fake would test nothing. Torn down at the
    /// end; a panic leaves a directory under $TMPDIR, which is what it is for.
    #[test]
    fn symlinks_report_target_shape_and_whether_they_leave_the_workspace() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("wt-symlink-{}", std::process::id()));
        let root = base.join("project");
        let outside = base.join("elsewhere");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("sub/file.txt"), b"x").unwrap();
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        symlink("sub", root.join("inside")).unwrap(); // relative, in-workspace
        symlink(root.join("sub/file.txt"), root.join("inside-file")).unwrap();
        symlink(&outside, root.join("away")).unwrap(); // absolute, out
        symlink(root.join("nope"), root.join("broken")).unwrap();
        symlink(".git", root.join("g")).unwrap(); // the name-skip, routed around
        let roots = vec![std::fs::canonicalize(&root).unwrap()];

        let (is_dir, target, block) = classify_symlink(&root.join("inside"), &roots);
        assert!(is_dir, "a link to a directory must read as a directory");
        assert_eq!(target.as_deref(), Some("sub"), "a relative link stays relative");
        assert_eq!(block, None, "target is inside the project root — followable");

        let (is_dir, _, block) = classify_symlink(&root.join("inside-file"), &roots);
        assert!(!is_dir);
        assert_eq!(block, None);

        let (is_dir, target, block) = classify_symlink(&root.join("away"), &roots);
        assert!(is_dir);
        assert!(target.unwrap().ends_with("elsewhere"));
        assert_eq!(block, Some("outside"), "outside every root — must not be followed");

        // A dangling link resolves nowhere, so it is a file for shape purposes
        // and just as inert — but for a different reason than one pointing away,
        // and the row says which.
        let (is_dir, target, block) = classify_symlink(&root.join("broken"), &roots);
        assert!(!is_dir);
        assert!(target.is_some(), "the link still says where it MEANT to point");
        assert_eq!(block, Some("missing"));

        // `list_dir` drops `.git` by name, so a link is the way around it.
        let (is_dir, _, block) = classify_symlink(&root.join("g"), &roots);
        assert!(is_dir);
        assert_eq!(block, Some("git"), "a link must not be a way back into .git");

        // An ordinary directory is never mistaken for a link out.
        assert!(under_roots(&roots, &std::fs::canonicalize(root.join("sub")).unwrap()));
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// The dwell guard: a place must be seen busy on two consecutive ticks
    /// before leaving counts as finished work. One tick is a blip (a resume, a
    /// probe written mid-transition) and must stamp nothing.
    #[test]
    fn completion_edge_needs_two_consecutive_busy_ticks() {
        let mut t = HashMap::new();
        assert!(completion_edges(&mut t, &v(&["/a"])).is_empty()); // tick 1: seen once
        assert!(completion_edges(&mut t, &[]).is_empty()); // gone after ONE tick → blip
        assert!(t.is_empty(), "a discarded blip must not linger in the counters");

        assert!(completion_edges(&mut t, &v(&["/a"])).is_empty());
        assert!(completion_edges(&mut t, &v(&["/a"])).is_empty()); // still busy on tick 2
        assert_eq!(completion_edges(&mut t, &[]), v(&["/a"]));
        assert!(completion_edges(&mut t, &[]).is_empty()); // and only ONCE
    }

    /// Independent places must not interfere: one finishing says nothing about
    /// another still working.
    #[test]
    fn completion_edges_track_places_independently() {
        let mut t = HashMap::new();
        for _ in 0..2 {
            completion_edges(&mut t, &v(&["/a", "/b"]));
        }
        assert_eq!(completion_edges(&mut t, &v(&["/b"])), v(&["/a"]));
        assert_eq!(completion_edges(&mut t, &[]), v(&["/b"]));
    }

    /// Two sessions in the same dir report the same cwd twice. The caller
    /// de-dupes; if it ever stops, the dwell guard would silently halve.
    #[test]
    fn duplicate_paths_would_double_count_the_dwell() {
        let mut t = HashMap::new();
        completion_edges(&mut t, &v(&["/a", "/a"]));
        assert_eq!(t["/a"], 2, "documents WHY the caller must dedup before this");
    }

    /// The backfill's only judgement call: which history lines are WORK. Slash
    /// housekeeping lands in history.jsonl exactly like a prompt, so without the
    /// denylist a `/clear` would light a place where nothing happened — the one
    /// thing the afterglow must never do.
    #[test]
    fn work_prompts_exclude_housekeeping_slashes() {
        assert!(is_work_prompt("fix the flaky test"));
        assert!(is_work_prompt("  /commit -m wip  ")); // unknown slash = user's own command
        assert!(is_work_prompt("/close-out"));
        assert!(!is_work_prompt("/clear"));
        assert!(!is_work_prompt("/resume some-session")); // args must not smuggle it past
        assert!(!is_work_prompt(""));
        assert!(!is_work_prompt("   "));
    }

    /// Claude Code has written `timestamp` as both a number and a quoted string.
    /// Trusting one shape would silently zero the backfill on the other.
    #[test]
    fn history_epoch_accepts_both_shapes() {
        use serde_json::json;
        assert_eq!(hist_epoch(&json!(1_786_318_766_274i64)), Some(1_786_318_766));
        assert_eq!(hist_epoch(&json!("1786318766274")), Some(1_786_318_766));
        assert_eq!(hist_epoch(&json!(null)), None);
        assert_eq!(hist_epoch(&json!("not-a-number")), None);
    }

    /// A transcript line carries ISO-8601; history.jsonl carries epoch millis.
    /// One parser reads both, and a date string must never fall through to the
    /// number branch.
    #[test]
    fn entry_epoch_accepts_iso_and_epoch_millis() {
        use serde_json::json;
        assert_eq!(entry_epoch(&json!("2026-09-20T01:49:52.243Z")), Some(1_789_868_992));
        assert_eq!(entry_epoch(&json!("2026-09-20T01:49:52Z")), Some(1_789_868_992));
        assert_eq!(entry_epoch(&json!(1_786_318_766_274i64)), Some(1_786_318_766));
        assert_eq!(entry_epoch(&json!("nope")), None);
        assert_eq!(entry_epoch(&json!(null)), None);
    }

    /// ⚠ THE regression this function exists for. A transcript is dated by the
    /// timestamps inside it, never by its mtime: Claude Code rewrites a live
    /// session's `.jsonl` for hours after its last turn, and the backfill runs
    /// on every launch with a forward-only stamp — so an mtime read re-dated
    /// every still-open place to "just finished" at each restart, and the unread
    /// ring came back on all of them. The file here is written NOW, so its mtime
    /// is now and its content is hours old: the two answers cannot be confused.
    #[test]
    fn transcript_epoch_dates_a_session_by_its_content_not_its_mtime() {
        let p = std::env::temp_dir().join(format!("wt-tsx-{}.jsonl", std::process::id()));
        std::fs::write(
            &p,
            concat!(
                r#"{"type":"user","timestamp":"2026-09-20T01:40:00.000Z"}"#,
                "\n",
                r#"{"type":"assistant","timestamp":"2026-09-20T01:49:52.243Z"}"#,
                "\n",
                // No timestamp at all: skipped, not treated as a zero.
                r#"{"type":"system","subtype":"turn_duration","durationMs":210986}"#,
                "\n",
                // Out of order — a rewritten summary lands last carrying an
                // older stamp, and the newest entry must still win.
                r#"{"type":"summary","timestamp":"2026-09-20T01:45:00.000Z"}"#,
                "\n",
                "not json at all\n",
            ),
        )
        .unwrap();
        let got = transcript_epoch(&p).expect("a transcript with timestamps must date");
        assert_eq!(got, 1_789_868_992, "the NEWEST timestamp in the file, 01:49:52Z");
        let now = worktrees_core::sysclock::now_epoch();
        assert!(
            now - got > 3600,
            "transcript_epoch returned {got}, within an hour of now ({now}) — that is the \
             file's mtime, which is exactly the bug: a session idle since this morning would \
             be stamped as having just finished on every restart"
        );
        let _ = std::fs::remove_file(&p);
    }

    /// Everything unreadable degrades to "no answer", so the backfill falls back
    /// to the prompt time instead of inventing one.
    #[test]
    fn transcript_epoch_has_no_answer_for_a_file_it_cannot_date() {
        let miss = std::env::temp_dir().join(format!("wt-tsx-missing-{}.jsonl", std::process::id()));
        assert_eq!(transcript_epoch(&miss), None);
        let p = std::env::temp_dir().join(format!("wt-tsx-blank-{}.jsonl", std::process::id()));
        std::fs::write(&p, "{\"type\":\"user\"}\nnot json\n").unwrap();
        assert_eq!(transcript_epoch(&p), None);
        let _ = std::fs::remove_file(&p);
    }

    /// A tail read starts mid-line by construction; that fragment must be
    /// dropped rather than fed to the JSON parser as a whole record.
    #[test]
    fn tail_lines_drops_the_partial_first_line() {
        let p = std::env::temp_dir().join(format!("wt-tail-{}.jsonl", std::process::id()));
        std::fs::write(&p, "aaaa\nbbbb\ncccc\n").unwrap();
        assert_eq!(tail_lines(&p, 1024), vec!["aaaa", "bbbb", "cccc"]);
        // 9 bytes back = "b\ncccc\n" plus a fragment of the first line
        assert_eq!(tail_lines(&p, 9), vec!["cccc"]);
        let _ = std::fs::remove_file(&p);
    }

    /// The tail offset lands wherever it lands, and prompts contain emoji. A
    /// strict decode would throw away every line over a split character — and
    /// stay broken, since the boundary only moves as the file grows.
    #[test]
    fn tail_lines_survives_a_split_multibyte_char() {
        let p = std::env::temp_dir().join(format!("wt-tail-utf8-{}.jsonl", std::process::id()));
        std::fs::write(&p, "aaaa\n🎉bbb\ncccc\n").unwrap(); // 5 + 8 + 5 = 18 bytes
        // 11 bytes back = offset 7, two bytes into the 4-byte emoji at 5..9
        assert_eq!(tail_lines(&p, 11), vec!["cccc"]);
        let _ = std::fs::remove_file(&p);
    }

    // ── change set parsers ───────────────────────────────────────────────
    // Fixtures are real `-z` output: NUL-TERMINATED records, not newlines.

    use ChangeKind::*;

    fn st(out: &str) -> Vec<(String, ChangeKind)> {
        parse_status_z(out)
    }
    fn ns(out: &str) -> Vec<(String, ChangeKind)> {
        parse_name_status_z(out)
    }
    fn p(s: &str, k: ChangeKind) -> (String, ChangeKind) {
        (s.to_string(), k)
    }
    /// Records joined the way git emits them: NUL-TERMINATED, one per entry.
    /// Built from a slice rather than one `\`-continued literal — the
    /// continuation strips leading whitespace, which would silently eat the very
    /// status column (` M`, ` D`) these fixtures exist to cover.
    fn z(recs: &[&str]) -> String {
        recs.iter().map(|r| format!("{r}\0")).collect()
    }

    #[test]
    fn status_z_classifies_every_column_pair_the_tree_can_meet() {
        let out = z(&[
            " M src/App.tsx",
            "M  src/lib.rs",
            "MM Cargo.toml",
            "A  src/new.rs",
            "?? notes.md",
            " D gone.txt",
            "D  staged-gone.txt",
            " T link",
            "UU conflict.rs",
        ]);
        assert_eq!(
            st(&out),
            vec![
                p("src/App.tsx", Modified),
                p("src/lib.rs", Modified),
                p("Cargo.toml", Modified),
                p("src/new.rs", Added),
                p("notes.md", Untracked),
                p("gone.txt", Deleted),
                p("staged-gone.txt", Deleted),
                p("link", Modified),
                p("conflict.rs", Modified),
            ]
        );
    }

    /// `AD` is added to the index and then removed from the worktree. There is no
    /// row on disk to call "added", so it has to come back deleted — otherwise
    /// the tree marks a directory and shows nothing inside it.
    #[test]
    fn status_z_reports_a_staged_add_deleted_from_the_worktree_as_deleted() {
        assert_eq!(st("AD src/oops.rs\0"), vec![p("src/oops.rs", Deleted)]);
    }

    /// The original of a rename lives in the NEXT record, and it is gone from
    /// disk — the ghost row the tree draws for it depends on this pair. A COPY's
    /// source is still there, so it must not be reported.
    #[test]
    fn status_z_splits_a_rename_into_added_plus_deleted_and_leaves_a_copy_alone() {
        assert_eq!(
            st("R  new/name.rs\0old/name.rs\0"),
            vec![p("new/name.rs", Added), p("old/name.rs", Deleted)]
        );
        assert_eq!(st("C  copy.rs\0orig.rs\0"), vec![p("copy.rs", Added)]);
        // …and the extra field must be CONSUMED: a record after a rename is a
        // status again, not a path.
        assert_eq!(
            st("R  a\0b\0?? c\0"),
            vec![p("a", Added), p("b", Deleted), p("c", Untracked)]
        );
    }

    /// A filename git would C-quote in the newline format arrives raw under `-z`
    /// — including a space, which is why the path starts at byte 3 and is never
    /// split on whitespace.
    #[test]
    fn status_z_keeps_paths_with_spaces_and_non_ascii_whole() {
        assert_eq!(st("?? docs/my notes.md\0"), vec![p("docs/my notes.md", Untracked)]);
        assert_eq!(st(" M docs/éclair 🎉.md\0"), vec![p("docs/éclair 🎉.md", Modified)]);
    }

    #[test]
    fn status_z_ignores_junk_records() {
        // `!!` only appears with --ignored, but the tree must not show ignored
        // files as untracked if it ever does; a stub too short to hold a path is
        // not a record.
        assert_eq!(st("!! target/debug\0 M ok.rs\0"), vec![p("ok.rs", Modified)]);
        assert_eq!(st("?? \0"), vec![]);
        assert_eq!(st(""), vec![]);
    }

    #[test]
    fn name_status_z_pairs_each_letter_with_its_path() {
        let out = "M\0src/App.tsx\0A\0src/new.rs\0D\0old.rs\0T\0link\0";
        assert_eq!(
            ns(out),
            vec![
                p("src/App.tsx", Modified),
                p("src/new.rs", Added),
                p("old.rs", Deleted),
                p("link", Modified),
            ]
        );
    }

    /// `R100`/`C75` carry a similarity score on the token and TWO paths after it.
    /// Reading only one would pair the next record's status with a path.
    #[test]
    fn name_status_z_consumes_both_paths_of_a_rename() {
        assert_eq!(
            ns("R100\0old.rs\0new.rs\0M\0after.rs\0"),
            vec![p("new.rs", Added), p("old.rs", Deleted), p("after.rs", Modified)]
        );
        assert_eq!(ns("C75\0orig.rs\0copy.rs\0"), vec![p("copy.rs", Added)]);
    }

    #[test]
    fn name_status_z_stops_on_a_status_with_no_path() {
        assert_eq!(ns("M\0a.rs\0M"), vec![p("a.rs", Modified)]);
        assert_eq!(ns(""), vec![]);
    }

    /// The two calls' REAL output, captured verbatim from a scratch repo whose
    /// branch carries one of every shape at once: a commit that modified, added,
    /// deleted and renamed, then uncommitted work that modified, deleted, and
    /// left an untracked file inside a brand-new directory. Both parsers are
    /// covered above; this is the ONE case that asserts what the frontend
    /// actually receives, and it is real bytes rather than a guess at the format.
    #[test]
    fn the_union_matches_what_git_printed_for_a_branch_with_every_shape() {
        let diff = z(&[
            "D", "docs/old.md",
            "A", "src/added.rs",
            "M", "src/mod.rs",
            "R100", "src/moved-from.rs", "src/moved-to.rs",
        ]);
        let status = z(&[" M src/keep.rs", " D tools/gen.sh", "?? fresh/deep/untracked.txt"]);
        let mut map: BTreeMap<String, ChangeKind> = BTreeMap::new();
        for (k, v) in ns(&diff) {
            map.insert(k, v);
        }
        for (k, v) in st(&status) {
            map.insert(k, v);
        }
        let want: BTreeMap<String, ChangeKind> = [
            // the rename's source is gone, so the tree gets a ghost for it
            ("docs/old.md", Deleted),
            ("src/moved-from.rs", Deleted),
            ("tools/gen.sh", Deleted),
            ("src/added.rs", Added),
            ("src/moved-to.rs", Added),
            ("src/mod.rs", Modified),
            ("src/keep.rs", Modified),
            // `-uall`: the file, not the `fresh/` directory that holds it
            ("fresh/deep/untracked.txt", Untracked),
        ]
        .into_iter()
        .map(|(p, k)| (p.to_string(), k))
        .collect();
        assert_eq!(map, want);
    }

    /// The whole point of the two-call union: the working tree is applied second
    /// and wins, because it describes the disk the tree is about to list.
    #[test]
    fn working_tree_status_overrides_the_committed_one_for_the_same_path() {
        let mut map: BTreeMap<String, ChangeKind> = BTreeMap::new();
        for (k, v) in ns("A\0src/new.rs\0M\0src/App.tsx\0") {
            map.insert(k, v);
        }
        for (k, v) in st(" D src/new.rs\0") {
            map.insert(k, v);
        }
        assert_eq!(map.get("src/new.rs"), Some(&Deleted));
        assert_eq!(map.get("src/App.tsx"), Some(&Modified));
    }

    // ── dock shell cwd memory ────────────────────────────────────────────────

    /// The OS read itself, on the one process whose cwd the test already knows.
    #[test]
    fn proc_cwd_reads_a_live_process_directory() {
        let got = proc_cwd(std::process::id()).expect("own cwd");
        // /var vs /private/var on macOS — compare resolved paths.
        assert_eq!(
            std::fs::canonicalize(got).unwrap(),
            std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap()
        );
    }

    /// Read the master to EOF and throw it away. Production always has one of
    /// these (`shell_open` spawns a reader thread); a pty test that skips it
    /// deadlocks in a way the app never can — the shell's output fills the pty
    /// buffer, and the child then WEDGES MID-EXIT (`ps` shows state `E`, never
    /// reaped) so even SIGKILL + `wait` hangs forever.
    fn drain(master: &(dyn MasterPty + Send)) {
        let mut reader = master.try_clone_reader().unwrap();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
        });
    }

    /// SIGKILL + reap, for tests that must not depend on how a shell reacts to
    /// a signal. `Child::kill` in portable-pty sends SIGHUP (lib.rs:347 of that
    /// crate), and an interactive `/bin/sh` on a pty whose master is still open
    /// survives it. The app gets away with SIGHUP because dropping the `Shell`
    /// closes the master and the EOF finishes the job; a test holding the master
    /// open would wait forever.
    fn hard_kill(child: &mut (dyn Child + Send + Sync)) {
        if let Some(pid) = child.process_id() {
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
        let _ = child.wait();
    }

    fn cwds(entries: &[(&str, u32, &str)]) -> CwdMap {
        let mut m = CwdMap::new();
        for (place, idx, path) in entries {
            m.entry(place.to_string()).or_default().insert(*idx, path.to_string());
        }
        m
    }

    fn key(repo: &str, slug: &str, index: u32) -> ShellKey {
        (repo.to_string(), slug.to_string(), index)
    }

    /// The merge exists so a tab that has NOT been re-spawned since the restart
    /// keeps its directory. Sampling only the live tab must leave the other one
    /// exactly as it was.
    #[test]
    fn merge_cwds_leaves_unsampled_tabs_alone() {
        let mut map = cwds(&[("/r|feat", 1, "/r/.worktrees/feat/app"), ("/r|feat", 2, "/r/.worktrees/feat/docs")]);
        let changed = merge_cwds(&mut map, vec![(key("/r", "feat", 1), "/r/.worktrees/feat/crates".into())]);
        assert!(changed);
        assert_eq!(map, cwds(&[("/r|feat", 1, "/r/.worktrees/feat/crates"), ("/r|feat", 2, "/r/.worktrees/feat/docs")]));
    }

    /// An idle shell samples the same path every 15s; that must not be a write.
    #[test]
    fn merge_cwds_reports_no_change_when_nothing_moved() {
        let mut map = cwds(&[("/r|feat", 1, "/r/.worktrees/feat/app")]);
        assert!(!merge_cwds(&mut map, vec![(key("/r", "feat", 1), "/r/.worktrees/feat/app".into())]));
    }

    #[test]
    fn a_remembered_directory_wins_over_the_place_root() {
        let here = std::env::current_dir().unwrap();
        let here = here.to_str().unwrap();
        assert_eq!(pick_start_dir(Some(here), "/place/root"), here);
    }

    /// A worktree that was removed, a directory that was renamed: the memory is
    /// stale, and a shell must still open somewhere real.
    #[test]
    fn a_vanished_directory_falls_back_to_the_place_root() {
        assert_eq!(pick_start_dir(Some("/no/such/dir/anywhere"), "/place/root"), "/place/root");
        assert_eq!(pick_start_dir(None, "/place/root"), "/place/root");
        // a FILE is not somewhere a shell can start either
        let f = std::env::current_dir().unwrap().join("Cargo.toml");
        assert_eq!(pick_start_dir(Some(f.to_str().unwrap()), "/place/root"), "/place/root");
    }

    /// The mechanism this whole feature rests on: a REAL shell on a REAL pty,
    /// told to `cd`, and the directory read back out of the OS by pid. Nothing
    /// mocked — the unit tests above only ever read this process's own cwd,
    /// which would pass just as happily if `proc_cwd` could not follow a child.
    #[test]
    fn proc_cwd_follows_a_live_shell_into_a_new_directory() {
        // Unique per run: two concurrent `cargo test` invocations must not share
        // these, and the test removes them at the end.
        let base = std::env::temp_dir().join(format!("wt-cwd-shell-{}", std::process::id()));
        let start = base.join("start");
        let moved = base.join("moved");
        std::fs::create_dir_all(&start).unwrap();
        std::fs::create_dir_all(&moved).unwrap();
        let real = |p: &Path| std::fs::canonicalize(p).unwrap();

        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        // /bin/sh, not $SHELL -l: the login shell's rc files are the app's
        // concern, not this mechanism's, and they make the test environment-
        // dependent for nothing.
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.cwd(&start);
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        drain(&*pair.master);
        let pid = live_pid(&mut *child).expect("pid of a running shell");

        assert_eq!(real(&proc_cwd(pid).expect("cwd at spawn")), real(&start));

        let mut w = pair.master.take_writer().unwrap();
        write!(w, "cd {}\n", moved.display()).unwrap();
        w.flush().unwrap();

        // The shell needs a moment to read the line; poll rather than sleep a
        // guessed amount.
        // resolved BEFORE the cleanup below — canonicalize needs the directory
        // to still exist, so computing it after the rmdir only ever panics
        let want = real(&moved);
        let mut got = None;
        for _ in 0..100 {
            let now = proc_cwd(pid).map(|p| real(&p));
            if now.as_deref() == Some(want.as_path()) {
                got = now;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        hard_kill(&mut *child);
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(got, Some(want), "shell cd was not visible through proc_cwd");
    }

    /// The reaped-pid guard, which is the difference between sampling this tab
    /// and sampling whatever process the OS later hands that pid to. A shell
    /// that exits is deliberately KEPT in the registry (the tab survives and
    /// offers a restart) and `list_shell_sessions` reaps it on the next dock
    /// mount — after which `process_id()` still answers, and answers wrongly.
    #[test]
    fn a_reaped_shell_reports_no_pid_to_sample() {
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut child = pair.slave.spawn_command(CommandBuilder::new("/bin/sh")).unwrap();
        drop(pair.slave);
        drain(&*pair.master);

        let pid = live_pid(&mut *child).expect("running shell has a pid");
        assert!(proc_cwd(pid).is_some(), "a running shell must have a readable cwd");

        hard_kill(&mut *child); // the reap — try_wait in list_shell_sessions does the same
        assert!(child.process_id().is_some(), "portable-pty still hands back the dangling pid");
        assert_eq!(live_pid(&mut *child), None, "a reaped shell must never be sampled");
    }

    // ── dock shell resizes ───────────────────────────────────────────────────

    /// The decision, and the bookkeeping that goes with it. A pane re-attaching
    /// asks to be the size it already is on every tab flip, and the
    /// ResizeObserver's first fire repeats whatever the attach just said — so
    /// "already that size" is the common case, not the corner one. Forgetting to
    /// record what WAS applied is the subtler half: every later request would
    /// then be compared against the size the pty had two resizes ago, and a
    /// repeat would sail through as a change.
    #[test]
    fn a_resize_to_the_size_the_pty_already_has_is_dropped() {
        let mut cur = PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 };
        let applied: Vec<Option<(u16, u16)>> = [(120, 40), (80, 24), (80, 24), (120, 40), (120, 40)]
            .iter()
            .map(|(c, r)| next_pty_size(&mut cur, *c, *r).map(|s| (s.cols, s.rows)))
            .collect();
        assert_eq!(
            applied,
            vec![None, Some((80, 24)), None, Some((120, 40)), None],
            "only the requests that MOVE the pty may reach it"
        );
        assert_eq!((cur.cols, cur.rows), (120, 40), "the record follows what was applied");
    }

    /// …and the same decision on a real PTY, through the method `shell_resize`
    /// and `shell_open`'s re-attach both call. A shell that traps SIGWINCH says
    /// whether the ioctl actually happened.
    ///
    /// Note what this can and cannot catch. macOS (and Linux) compare the new
    /// winsize against the old one inside TIOCSWINSZ and signal only on a
    /// change, so a suppression bug does NOT show up here as a spurious W — the
    /// kernel hides it. What it does catch is the opposite and worse failure:
    /// suppression that swallows a size change, leaving the shell laying its
    /// prompt out for a terminal that is no longer that shape.
    #[test]
    fn a_real_size_change_still_reaches_the_shell() {
        let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).unwrap();
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        // `R` once the handler is installed: a resize sent before that is simply
        // lost, and the test would blame the code for the race.
        cmd.arg("trap 'printf W' WINCH; printf R; while :; do sleep 0.05; done");
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);

        // Drains the master AND is the assertion's eyes — production always has
        // exactly one such reader (see `drain`).
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mine = seen.clone();
        let mut reader = pair.master.try_clone_reader().unwrap();
        thread::spawn(move || {
            let mut buf = [0u8; 256];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => mine.lock().unwrap().extend_from_slice(&buf[..n]),
                }
            }
        });
        let writer = pair.master.take_writer().unwrap();
        let mut sh = Shell {
            master: pair.master,
            writer,
            child,
            stop: Arc::new(AtomicBool::new(false)),
            ring: Arc::new(Mutex::new(VecDeque::new())),
            sink: Arc::new(Mutex::new(None)),
            gen: 1,
            size,
            dirty: Arc::new(AtomicBool::new(false)),
        };
        // Poll rather than sleep a guessed amount; bounded so a shell that never
        // answers fails the test instead of hanging it.
        let count_of = |c: u8| seen.lock().unwrap().iter().filter(|b| **b == c).count();
        let winches = || count_of(b'W');
        let wait = |f: &dyn Fn() -> usize, n: usize| {
            for _ in 0..60 {
                if f() >= n {
                    return true;
                }
                thread::sleep(Duration::from_millis(50));
            }
            false
        };
        let wait_for = |n: usize| wait(&winches, n);

        assert!(wait(&|| count_of(b'R'), 1), "the trap shell never came up");
        assert!(sh.resize(100, 30).is_ok());
        assert!(wait_for(1), "a genuine resize must reach the shell as SIGWINCH");
        assert!(sh.resize(120, 40).is_ok());
        assert!(wait_for(2), "and so must the next one — the recorded size has to move with it");
        assert_eq!((sh.size.cols, sh.size.rows), (120, 40));

        // The repeat: nothing more may arrive. (The kernel would swallow it even
        // without the guard — this pins the assumption, it does not test it.)
        assert!(sh.resize(120, 40).is_ok());
        thread::sleep(Duration::from_millis(300));
        assert_eq!(winches(), 2, "resizing to the size it already has must be silent");

        hard_kill(&mut *sh.child);
    }

    /// Round-trip through the real file, including the two rules a plain
    /// serialize test would miss: an emptied place leaves no husk behind, and a
    /// `false` return writes nothing at all.
    #[test]
    fn the_cwd_file_round_trips_and_prunes() {
        let dir = std::env::temp_dir().join(format!("wt-cwd-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shell-cwds.json");
        let _ = std::fs::remove_file(&file);

        edit_cwds_at(&file, |m| merge_cwds(m, vec![(key("/r", "feat", 1), "/tmp".into())]));
        assert_eq!(read_cwds_at(&file), cwds(&[("/r|feat", 1, "/tmp")]));

        // a no-op edit must not even create noise
        let before = std::fs::read(&file).unwrap();
        edit_cwds_at(&file, |m| merge_cwds(m, vec![(key("/r", "feat", 1), "/tmp".into())]));
        assert_eq!(std::fs::read(&file).unwrap(), before);

        // closing the last tab of a place drops the place, not an empty husk —
        // through the SAME predicate close_shell_session uses, not a copy of it
        edit_cwds_at(&file, |m| forget_tab(m, "/r", "feat", 1));
        assert_eq!(read_cwds_at(&file), CwdMap::new());
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), "{}");

        // a corrupt file reads as "no memory", never as a failure
        std::fs::write(&file, b"{not json").unwrap();
        assert_eq!(read_cwds_at(&file), CwdMap::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The startup sweep's decision. A place removed with `worktrees rm` from a
    /// terminal leaves its entry behind — nothing tells the app — so this is
    /// what stops a recreated slug inheriting the previous life's directories.
    #[test]
    fn only_places_that_no_longer_exist_are_forgotten() {
        let here = std::env::current_dir().unwrap().to_str().unwrap().to_string();
        let keys: Vec<String> = ["/r|alive", "/r|removed", "/dead-repo|any", "no-separator"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let gone = vanished_keys(&keys, |repo, slug| match (repo, slug) {
            ("/r", "alive") => Some(here.clone()),
            ("/r", "removed") => Some("/gone/for/good".into()),
            _ => None, // the project itself is unreachable
        });
        assert_eq!(gone, vec!["/r|removed", "/dead-repo|any", "no-separator"]);
    }

    /// A repo path is absolute and may itself contain the separator; the slug
    /// never does, so the split has to come from the right.
    #[test]
    fn a_repo_path_containing_the_separator_still_splits_at_the_slug() {
        let seen = std::cell::RefCell::new(Vec::new());
        let keys = vec!["/odd|repo|feat".to_string()];
        vanished_keys(&keys, |repo, slug| {
            seen.borrow_mut().push((repo.to_string(), slug.to_string()));
            None
        });
        assert_eq!(seen.into_inner(), vec![("/odd|repo".to_string(), "feat".to_string())]);
    }

    /// `shell_open`'s actual lookup — `remembered_dir` composed with
    /// `pick_start_dir` — over a map that came off disk. The pieces were each
    /// covered; the composition, which is the only thing the feature does for
    /// the user, was not.
    #[test]
    fn a_reopened_tab_starts_in_the_directory_the_file_remembers() {
        let dir = std::env::temp_dir().join(format!("wt-cwd-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shell-cwds.json");
        let _ = std::fs::remove_file(&file);
        let here = std::env::current_dir().unwrap();
        let here = here.to_str().unwrap().to_string();
        let place = "/place/root";

        // tab 1 was left in a real directory, tab 2 in one since deleted
        edit_cwds_at(&file, |m| {
            merge_cwds(
                m,
                vec![
                    (key("/r", "feat", 1), here.clone()),
                    (key("/r", "feat", 2), "/gone/for/good".into()),
                ],
            )
        });
        let map = read_cwds_at(&file);

        let open = |index: u32| pick_start_dir(remembered_dir(&map, "/r", "feat", index).as_deref(), place);
        assert_eq!(open(1), here, "tab 1 must reopen where it was");
        assert_eq!(open(2), place, "a vanished directory falls back to the place root");
        assert_eq!(open(3), place, "a tab with no memory opens at the place root");
        // a DIFFERENT place must not see this one's memory
        assert_eq!(
            pick_start_dir(remembered_dir(&map, "/r", "other", 1).as_deref(), place),
            place
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── dock shell history: scrollback + per-tab commands ───────────────────

    /// A scratch `term-history` root, unique per run: two concurrent `cargo
    /// test` invocations must not share one.
    fn hist_root(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-hist-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Round-trip through the real files, including the two rules a plain
    /// serialize test would miss: the width travels with the bytes, and a tab
    /// that has never been saved reads as nothing rather than as an error.
    #[test]
    fn a_scrollback_round_trips_with_the_width_it_was_written_at() {
        let root = hist_root("round");
        let k = key("/r", "feat", 3);

        assert!(read_scrollback(&root, &k).is_none(), "an unsaved tab has no history");

        write_scrollback(&root, &k, b"line one\nbuild output\n", 174);
        let (bytes, meta) = read_scrollback(&root, &k).expect("saved tab reads back");
        assert_eq!(bytes, b"line one\nbuild output\n", "a ring that never rolled keeps its first line");
        assert_eq!(meta.cols, 174, "the width travels with the bytes");
        assert_eq!((meta.repo.as_str(), meta.slug.as_str(), meta.index), ("/r", "feat", 3));

        // closing the tab takes everything with it
        forget_tab_history(&root, &k);
        assert!(read_scrollback(&root, &k).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The two halves of `rolled_ring`, and the reason it is not just
    /// `trim_to_line_start`.
    ///
    /// A ring AT the cap begins wherever `drain` cut — routinely mid-escape-
    /// sequence — so its first fragment goes. A ring BELOW the cap begins where
    /// the shell began, and trimming that drops a real line. The second case is
    /// the one that bites, because it COMPOUNDS: the restore seeds from the
    /// trimmed copy, so the next save trims the next line, one per restart.
    #[test]
    fn only_a_ring_that_actually_rolled_loses_its_first_line() {
        let short = b"line one\nline two\n";
        assert_eq!(rolled_ring(short), short, "a ring below the cap is saved whole");

        // at the cap, and cut mid-line the way `drain` leaves it
        let mut full = b"tail-of-a-cut-line".to_vec();
        full.extend(std::iter::repeat(b'x').take(SHELL_RING));
        full.insert(18, b'\n');
        let kept = rolled_ring(&full);
        assert!(kept.len() < full.len(), "a rolled ring drops its leading fragment");
        assert_eq!(kept[0], b'x', "…and resumes at the start of a line");

        // the compounding case, stated directly: saving what a restore seeded
        // must be a fixed point, not another line gone
        let seeded = rolled_ring(&full).to_vec();
        assert_eq!(rolled_ring(&seeded), seeded.as_slice(), "re-saving a restored ring loses nothing");
    }

    /// The directory name is a hash, so it is only a NAME. `meta.json` is the
    /// identity — without that check a collision would hand one tab another
    /// tab's output, which is the one failure worse than opening blank.
    #[test]
    fn a_meta_that_names_another_tab_reads_as_nothing() {
        let root = hist_root("ident");
        let mine = key("/r", "feat", 1);
        write_scrollback(&root, &mine, b"\nmine\n", 80);

        // forge the collision: same directory, someone else's meta
        let tab = tab_dir(&root, &mine);
        let theirs = TabMeta {
            repo: "/other".into(),
            slug: "theirs".into(),
            index: 9,
            cols: 80,
            at: sysclock::now_epoch(),
        };
        std::fs::write(tab.join("meta.json"), serde_json::to_vec(&theirs).unwrap()).unwrap();
        assert!(read_scrollback(&root, &mine).is_none(), "another tab's meta is not mine");

        // and a corrupt one is simply no history, never an error
        std::fs::write(tab.join("meta.json"), b"{not json").unwrap();
        assert!(read_scrollback(&root, &mine).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A repo path is absolute and may contain anything; two tabs must never
    /// land in the same directory, and the same tab must always land in its own.
    #[test]
    fn every_tab_gets_its_own_stem_and_keeps_it() {
        let a = tab_stem(&key("/odd|repo", "feat", 1));
        assert_eq!(a, tab_stem(&key("/odd|repo", "feat", 1)), "a stem is stable");
        let others = [
            key("/odd|repo", "feat", 2),        // same place, next tab
            key("/odd", "repo|feat", 1),        // the separator moved
            key("/other", "feat", 1),           // another project
        ];
        for o in &others {
            assert_ne!(a, tab_stem(o), "{o:?} collided with /odd|repo|feat|1");
        }
        // a slug that is all path separators still names a file, not a path
        let s = tab_stem(&key("/r", "feat/../../etc", 1));
        assert!(!s.contains('/'), "a stem must never contain a separator: {s}");
    }

    /// The restore payload's contracts: the seam comes AFTER the content, an
    /// empty save sends nothing at all (which keeps `replay` at 0 so the pane
    /// treats the new shell's own first output as live), and the seam carries an
    /// absolute time because it is persisted with the ring.
    #[test]
    fn a_seam_follows_the_content_and_an_empty_save_sends_nothing() {
        let at = 1_700_000_000;
        assert!(restore_payload(b"", at).is_empty(), "nothing saved, nothing replayed");

        let out = restore_payload(b"the build log\n", at);
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("the build log\n"), "the content comes first: {text:?}");
        assert!(
            text.find("the build log").unwrap() < text.find("restored").unwrap(),
            "the seam marks the END of what was restored"
        );
        // absolute, not an age: this is seeded into the ring and persisted with
        // it, so "2h ago" would freeze and be wrong forever after
        assert!(text.contains(&fmt_local(at)), "the seam names when it was saved: {text:?}");
        // cancels a control sequence the ring's tail was cut inside, so the seam
        // cannot be swallowed as somebody's argument
        assert!(out.starts_with(SEAM_HEAD) || out.windows(SEAM_HEAD.len()).any(|w| w == SEAM_HEAD));
    }

    /// Three launches with nothing typed between them must not stack three
    /// seams — but a seam with a session's work under it still marks a real
    /// boundary and stays.
    #[test]
    fn a_seam_nothing_was_typed_under_is_replaced_not_followed() {
        let (t1, t2) = (1_700_000_000, 1_700_009_000);
        let once = restore_payload(b"the build log\n", t1);
        let twice = restore_payload(&once, t2);
        let text = String::from_utf8_lossy(&twice);
        assert_eq!(text.matches("── restored · ").count(), 1, "only the newest seam survives: {text:?}");
        assert!(text.contains(&fmt_local(t2)) && !text.contains(&fmt_local(t1)));

        // …but work done after a seam keeps it
        let mut worked = once.clone();
        worked.extend_from_slice(b"$ cargo test\nall good\n");
        let after = restore_payload(&worked, t2);
        let text = String::from_utf8_lossy(&after);
        assert_eq!(text.matches("── restored · ").count(), 2, "a boundary with work under it stays: {text:?}");
    }

    /// The seam is written for a person sitting in front of the terminal, so it
    /// is the machine's own clock — unlike app.log, which is UTC.
    #[test]
    fn the_seam_time_is_local_not_utc() {
        let at = 1_700_000_000;
        let local = fmt_local(at);
        assert_eq!(local.len(), fmt_utc(at).len(), "same shape, different offset");
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let t = at as libc::time_t;
        unsafe { libc::localtime_r(&t, &mut tm) };
        assert_eq!(local, fmt_utc(at + tm.tm_gmtoff as i64));
    }

    /// The cold-start sweep's decision, the way `only_places_that_no_longer_exist_are_forgotten`
    /// covers the cwd map's. Two reasons to drop a tab and one to keep it.
    #[test]
    fn a_tab_is_swept_when_its_place_is_gone_or_it_has_gone_stale() {
        let now = 2_000_000_000;
        let fresh = now - 60;
        let old = now - TERM_HIST_MAX_AGE_SECS - 1;
        let m = |repo: &str, slug: &str, at: i64| TabMeta {
            repo: repo.into(),
            slug: slug.into(),
            index: 1,
            cols: 80,
            at,
        };
        let metas = vec![
            (PathBuf::from("/h/alive"), m("/r", "alive", fresh)),
            (PathBuf::from("/h/removed"), m("/r", "removed", fresh)),
            (PathBuf::from("/h/stale"), m("/r", "alive", old)),
            (PathBuf::from("/h/dead-repo"), m("/dead", "any", fresh)),
        ];
        let here = std::env::current_dir().unwrap().to_str().unwrap().to_string();
        let gone = stale_history(&metas, now, |repo, slug| match (repo, slug) {
            ("/r", "alive") => Some(here.clone()),
            ("/r", "removed") => Some("/gone/for/good".into()),
            _ => None, // the project itself is unreachable
        });
        let gone: Vec<&str> = gone.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(gone, vec!["/h/removed", "/h/stale", "/h/dead-repo"]);
    }

    /// The generated shims, read as text. They are the whole per-tab history
    /// mechanism, and every clause below is load-bearing — see the section
    /// comment for which real configuration each one exists for.
    #[test]
    fn the_zdotdir_shims_hand_the_users_own_config_back_before_taking_the_history() {
        let root = hist_root("zdot");
        let zdot = root.join("zdotdir");
        write_zdotdir(&zdot).unwrap();

        let read = |n: &str| std::fs::read_to_string(zdot.join(n)).unwrap();
        for n in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
            assert!(read(n).starts_with("# Generated by worktrees"), "{n} is unlabelled");
        }

        // .zshenv: their environment while theirs runs, ours back afterwards —
        // otherwise a `~/.zshenv` that exports ZDOTDIR hijacks the whole chain
        let env = read(".zshenv");
        assert!(env.contains("unset ZDOTDIR"), "their .zshenv must see a normal environment");
        assert!(env.contains(r#". "$HOME/.zshenv""#), "their .zshenv is sourced, not replaced");
        assert!(
            env.find("unset ZDOTDIR").unwrap() < env.find("$HOME/.zshenv").unwrap(),
            "ZDOTDIR must be unset BEFORE theirs is sourced"
        );
        assert!(env.contains("export ZDOTDIR=$_WT_Z"), "and taken back after");

        // .zshrc: their ZDOTDIR restored before their rc (which may reference
        // it), our HISTFILE after it (so a user rc that sets HISTFILE loses)
        let rc = read(".zshrc");
        let restore = rc.find("unset ZDOTDIR").expect("restores their ZDOTDIR");
        let source = rc.find("/.zshrc\"").expect("sources their rc");
        let hist = rc.find("HISTFILE=\"$WORKTREES_HISTFILE\"").expect("sets our HISTFILE");
        assert!(restore < source, "their rc must be able to read $ZDOTDIR");
        assert!(source < hist, "our HISTFILE must be set AFTER theirs, or theirs wins");
        // without this a force-quit takes the whole session's commands with it:
        // the app SIGHUPs its shells, and zsh's default flushes only on a clean exit
        assert!(rc.contains("setopt INC_APPEND_HISTORY"));
        assert!(!rc.contains("SHARE_HISTORY"), "each tab has its own file; nothing to share");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Which lever each shell gets. zsh cannot use `HISTFILE` — macOS's
    /// `/etc/zshrc` overwrites it unconditionally — and nothing else can use
    /// ZDOTDIR.
    #[test]
    fn each_shell_gets_the_only_lever_that_works_for_it() {
        let root = hist_root("lever");
        let k = key("/r", "feat", 1);
        let zdot = tab_dir(&root, &k).join("zdotdir");

        let mut zsh = CommandBuilder::new("/bin/zsh");
        assert_eq!(apply_tab_history_env(&mut zsh, &root, &k, "/bin/zsh"), Some("zdotdir"));
        assert_eq!(zsh.get_env("ZDOTDIR"), Some(zdot.as_os_str()));
        assert_eq!(zsh.get_env("WORKTREES_HISTFILE"), Some(zdot.join(".zsh_history").as_os_str()));
        assert!(zsh.get_env("HISTFILE").is_none(), "zsh's HISTFILE is /etc/zshrc's to set");

        let mut bash = CommandBuilder::new("/bin/bash");
        assert_eq!(apply_tab_history_env(&mut bash, &root, &k, "/bin/bash"), Some("histfile"));
        assert!(bash.get_env("ZDOTDIR").is_none(), "ZDOTDIR means nothing to bash");
        assert_eq!(
            bash.get_env("HISTFILE"),
            Some(tab_dir(&root, &k).join(".bash_history").as_os_str())
        );

        let mut fish = CommandBuilder::new("/opt/homebrew/bin/fish");
        assert_eq!(apply_tab_history_env(&mut fish, &root, &k, "/opt/homebrew/bin/fish"), None);
        assert!(fish.get_env("ZDOTDIR").is_none() && fish.get_env("HISTFILE").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The one that proves the MECHANISM rather than the strings: a real login
    /// zsh, with a real `/etc/zshrc` in the way, writing its history where this
    /// feature says it will.
    ///
    /// The string assertions above would pass just as happily if zsh ignored the
    /// shims entirely. This is the sibling of
    /// `proc_cwd_follows_a_live_shell_into_a_new_directory`, and it carries the
    /// same two rules: DRAIN the master (a pty test that does not wedges the
    /// child mid-exit, unkillable) and SIGKILL rather than trusting a signal.
    #[test]
    fn a_real_login_zsh_records_into_the_generated_zdotdir() {
        if !Path::new("/bin/zsh").exists() {
            return; // nothing to prove on a box without zsh
        }
        let root = hist_root("realzsh");
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        // A user rc with something only IT can provide, so "sourced" and
        // "replaced" cannot look the same.
        std::fs::write(home.join(".zshrc"), "export WT_TEST_MARKER=from-the-users-rc\n").unwrap();

        let k = key("/r", "feat", 1);
        let zdot = tab_dir(&root, &k).join("zdotdir");
        let mut cmd = CommandBuilder::new("/bin/zsh");
        cmd.arg("-l");
        cmd.cwd(&root);
        cmd.env("HOME", &home);
        cmd.env("TERM", "xterm-256color");
        assert_eq!(apply_tab_history_env(&mut cmd, &root, &k, "/bin/zsh"), Some("zdotdir"));

        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        drain(&*pair.master);

        // Answer into a FILE rather than parsing the pty: prompts, escape
        // sequences and echo make the stream a poor witness.
        let out = root.join("answers");
        let mut w = pair.master.take_writer().unwrap();
        write!(
            w,
            ": wt-history-probe\n\
             print -r -- \"HISTFILE=$HISTFILE\" > {out}\n\
             print -r -- \"MARKER=$WT_TEST_MARKER\" >> {out}\n\
             print -r -- \"ZDOTDIR=${{ZDOTDIR-unset}}\" >> {out}\n",
            out = out.display()
        )
        .unwrap();
        w.flush().unwrap();

        let hist = zdot.join(".zsh_history");
        let mut answers = String::new();
        for _ in 0..200 {
            if let Ok(s) = std::fs::read_to_string(&out) {
                if s.lines().count() >= 3 {
                    answers = s;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
        // INC_APPEND_HISTORY writes per command, so the probe is already there —
        // this shell is never asked to exit cleanly, which is the point.
        let recorded = std::fs::read_to_string(&hist).unwrap_or_default();
        hard_kill(&mut *child);
        let _ = std::fs::remove_dir_all(&root);

        assert!(!answers.is_empty(), "the shell never answered");
        assert!(
            answers.contains(&format!("HISTFILE={}", hist.display())),
            "HISTFILE is not the tab's: {answers}"
        );
        assert!(
            answers.contains("MARKER=from-the-users-rc"),
            "the user's own .zshrc was not sourced: {answers}"
        );
        assert!(
            answers.contains("ZDOTDIR=unset"),
            "ZDOTDIR must be handed back, so child shells are unaffected: {answers}"
        );
        assert!(
            recorded.contains("wt-history-probe"),
            "a killed shell still recorded its commands (INC_APPEND_HISTORY): {recorded:?}"
        );
    }

    // ── "Ask Claude" (ai_status_report) — the two pure pieces ────────────────
    // The spawn itself cannot be tested here: there is no fake `claude` anywhere
    // in this repo (the bats harness fakes tmux and git, never the AI), which is
    // why docs/ai-profiles-manual-checks.md §11 exists. What IS testable is the
    // text we hand the model and the check that decides whether to spawn at all.

    /// The prompt is the WHOLE interface to the model, and every clause in it is
    /// load-bearing: drop the JSON and claude re-derives facts the check already
    /// measured; drop the ban on commands and a headless run — which cannot
    /// answer a permission prompt — stalls on a tool it will never be granted;
    /// drop the path and it reports on whatever directory it guessed.
    #[test]
    fn the_status_prompt_carries_the_facts_the_questions_and_the_ban() {
        let json = r#"{"schema_version":1,"verdict":"at-risk","reasons":["3 commits not on origin/main"]}"#;
        let p = status_prompt(json, "/tmp/wt/feat-x");

        // VERBATIM — not re-serialized, not summarized. The report the sheet
        // renders and the report claude reads have to be the same bytes.
        assert!(p.contains(json), "the health JSON must appear verbatim");
        assert!(p.contains("/tmp/wt/feat-x"), "the worktree path must be named");

        // The three questions, in order.
        assert!(p.contains("what was this worktree for"));
        assert!(p.contains("what state did the work end in"));
        assert!(p.contains("resume / push-then-abandon / abandon"));
        let (q1, q3) = (p.find("what was this worktree for"), p.find("push-then-abandon"));
        assert!(q1 < q3, "the questions must stay in order");

        // The reading list, the budget, and the ban.
        for planning in ["task_plan.md", "findings.md", "progress.md", "ROADMAP.md"] {
            assert!(p.contains(planning), "the prompt should point at {planning}");
        }
        assert!(p.contains("under 250 words"));
        assert!(p.contains("Do not run commands"));
    }

    /// The guard that decides whether anything is spawned at all.
    ///
    /// The case that matters most is the third: `ops::ai_launch_for` fails
    /// CLOSED by swapping in a `printf … >&2` sentinel while KEEPING
    /// `match_word: "claude"`. A guard written against `match_word` — the
    /// obvious choice, and what every other caller in the codebase reads — waves
    /// that through, and the app then caches printf's error message as claude's
    /// considered read of the worktree.
    #[test]
    fn the_guard_reads_the_composed_command_not_the_match_word() {
        // plain claude
        assert!(claude_launch_check("claude", "claude").is_ok());
        // profiled claude — claude_launch appends flags to the same first word
        assert!(claude_launch_check(
            "claude --append-system-prompt-file '/x/rules.md' --settings '/x/s.json' --strict-mcp-config",
            "claude",
        )
        .is_ok());
        // an absolute path still basenames to claude
        assert!(claude_launch_check("/opt/homebrew/bin/claude -r", "claude").is_ok());

        // fail-closed sentinel: match_word STILL says claude, the command does not
        let sentinel = "printf '%s\\n' 'profile could not be prepared' >&2";
        let e = claude_launch_check(sentinel, "claude").expect_err("the printf sentinel must be refused");
        assert!(e.contains("could not be prepared"), "say WHICH failure it was: {e}");
        assert!(e.contains("app log"), "point at where the reason is: {e}");

        // a different AI tool — a different fix, so a different message
        let e = claude_launch_check("aider --model x", "aider").expect_err("non-claude ai_cmd must be refused");
        assert!(e.contains("ai_cmd"), "{e}");
        assert!(e.contains("aider"), "name the tool it found: {e}");

        // ai_cmd = none (plain shell). ⚠ `ai_word_of("")` DEFAULTS to "claude",
        // so an empty command passes the word check and has to be caught first.
        let e = claude_launch_check("", "claude").expect_err("an empty command must be refused");
        assert!(e.contains("none"), "{e}");
        assert!(claude_launch_check("   ", "claude").is_err(), "whitespace is empty too");
    }

    /// The tail is what the user reads when a run fails, so it has to survive
    /// the two shapes a CLI's stderr actually takes: nothing at all, and pages.
    #[test]
    fn the_stderr_tail_keeps_the_end_and_drops_the_blanks() {
        assert_eq!(stderr_tail(b"", 3), "");
        assert_eq!(stderr_tail(b"\n\n  \n", 3), "", "blank lines are not a reason");
        assert_eq!(stderr_tail(b"one\ntwo\nthree", 5), "one\ntwo\nthree");
        assert_eq!(stderr_tail(b"one\ntwo\nthree\nfour", 2), "three\nfour", "the END is the reason");
    }

    // ── local usage metrics ────────────────────────────────────────────────

    fn ev(k: &str, key: &str, s: &str, t_ms: i64, ms: i64) -> UiEvent {
        UiEvent { t: t_ms, k: k.into(), key: key.into(), s: s.into(), ms }
    }

    /// The one rule this whole feature rests on: what lands in the file is a
    /// name from the source, never something a person typed. Every rejected
    /// shape here is user text wearing a key's clothes.
    #[test]
    fn a_key_is_a_written_name_not_typed_text() {
        for ok in ["nav.row.select", "chord.cmd-b", "dock.files", "main", "a", "files_row"] {
            assert!(valid_token(ok), "{ok} is a hand-written key");
        }
        for bad in [
            "",                                   // no key at all
            "/Users/davidpena/workspace/repo",    // a path
            "feat/redesign",                      // a branch
            "standup and daily work",             // a typed note
            "a\tb",                               // whitespace of any kind
            "café",                               // non-ASCII: user text, or a title
            "…",
        ] {
            assert!(!valid_token(bad), "{bad:?} must be refused");
        }
        // 64 is the cap; 65 is not
        assert!(valid_token(&"a".repeat(64)));
        assert!(!valid_token(&"a".repeat(65)));
        assert!(!valid_token(&"k".repeat(100)));
        // and the whole-event gate rides on it
        assert!(valid_event(&ev("act", "nav.row", "nav", 1, 0)));
        assert!(!valid_event(&ev("act", "nav.row", "nav/x", 1, 0)), "the surface is checked too");
        assert!(!valid_event(&ev("keystroke", "a", "main", 1, 0)), "only act and dwell exist");
        assert!(!valid_event(&ev("act", "nav.row", "nav", 0, 0)), "an event with no clock is not one");
    }

    /// The day a click belongs to is the day the USER was having, and the
    /// frontend is the only thing that knows which that is. Same instant,
    /// three timezones, three different columns.
    #[test]
    fn a_day_bucket_is_the_users_day_not_utcs() {
        // 2026-09-05T23:30:00Z
        let late = 1_788_651_000_000i64;
        assert_eq!(day_label(day_index(late, 0)), "2026-09-05");
        assert_eq!(day_label(day_index(late, 120)), "2026-09-06", "UTC+2 is already tomorrow");
        assert_eq!(day_label(day_index(late, -420)), "2026-09-05", "UTC-7 is still today");
        // 2026-09-06T02:00:00Z — the same wall clock from the other side
        let early = late + 2 * 3600 * 1000 + 30 * 60 * 1000;
        assert_eq!(day_label(day_index(early, 0)), "2026-09-06");
        assert_eq!(day_label(day_index(early, -420)), "2026-09-05", "UTC-7 has not gone to bed yet");
    }

    /// A heatmap with a hole in it is a lie about the days it skipped, so the
    /// report is a dense grid: exactly `days` columns, oldest first, zeros where
    /// nothing happened.
    #[test]
    fn the_report_is_a_dense_grid_with_the_gaps_filled() {
        let day = 86_400_000i64;
        let now = 1_788_651_000_000i64; // 2026-09-05T23:30:00Z
        let events = vec![
            ev("act", "nav.row", "nav", now, 0),
            ev("act", "nav.row", "nav", now, 0),
            ev("act", "nav.row", "nav", now - 2 * day, 0),
            ev("act", "places", "nav", now - 2 * day, 0),
            ev("dwell", "dwell", "main", now, 3000),
            ev("dwell", "dwell", "main", now - day, 1000),
            ev("dwell", "dwell", "home", now, 2000),
            ev("act", "ancient", "nav", now - 30 * day, 0), // outside the range
        ];
        let u = usage_from(&events, now, 3, 0, "/tmp/ui-events.jsonl".into());
        assert_eq!(u.days, vec!["2026-09-03", "2026-09-04", "2026-09-05"]);
        assert_eq!(u.acts.len(), 3);
        assert_eq!(u.acts[0], ActRow { key: "nav.row".into(), total: 3, per_day: vec![1, 0, 2] });
        assert_eq!(u.acts[1], ActRow { key: "places".into(), total: 1, per_day: vec![1, 0, 0] });
        // the whole point of the exercise: a control that HAS been used, but
        // not once in the range, is a row of zeros rather than an absence
        assert_eq!(u.acts[2], ActRow { key: "ancient".into(), total: 0, per_day: vec![0, 0, 0] });
        assert_eq!(u.dwell[0], DwellRow { surface: "main".into(), ms_total: 4000, per_day: vec![0, 1000, 3000] });
        assert_eq!(u.dwell[1], DwellRow { surface: "home".into(), ms_total: 2000, per_day: vec![0, 0, 2000] });
        // and the columns really are the user's days: 23:30Z is already
        // tomorrow at UTC+2, so the whole grid — the labels AND every event in
        // it — slides one day forward rather than the counts moving between
        // columns. A report bucketed in UTC would put today's clicks under
        // yesterday for anyone east of Greenwich after 22:00.
        let shifted = usage_from(&events, now, 3, 120, "/tmp/x".into());
        assert_eq!(shifted.days, vec!["2026-09-04", "2026-09-05", "2026-09-06"]);
        assert_eq!(shifted.acts[0], ActRow { key: "nav.row".into(), total: 3, per_day: vec![1, 0, 2] });
        // one hour further into the past and the oldest pair drops out of the
        // window entirely — the range is inclusive of exactly `days` days
        let narrow = usage_from(&events, now, 2, 0, "/tmp/x".into());
        assert_eq!(narrow.days, vec!["2026-09-04", "2026-09-05"]);
        assert_eq!(narrow.acts[0], ActRow { key: "nav.row".into(), total: 2, per_day: vec![0, 2] });
        assert_eq!(narrow.acts.last().map(|r| (r.key.as_str(), r.total)), Some(("places", 0)),
                   "a key with nothing left in range falls to the bottom at zero");
    }

    /// Append refuses what does not look like a key, and rotates ONCE — the
    /// file is a convenience, not an archive.
    #[test]
    fn the_events_file_refuses_text_and_rotates_once() {
        let dir = std::env::temp_dir().join(format!("wt-uievents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ui-events.jsonl");

        ui_events_append_at(&file, &[
            ev("act", "nav.row", "nav", 1_788_651_000_000, 0),
            ev("act", "/Users/me/workspace/secret-project", "nav", 1_788_651_000_000, 0),
            ev("act", "ok.two", "main", 1_788_651_000_000, 0),
        ]);
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 2, "the path-shaped key must not be stored: {text}");
        assert!(!text.contains("secret-project"), "{text}");
        // `ms` is dwell-only, so an act line does not carry one
        assert!(!text.lines().next().unwrap().contains("\"ms\""), "{text}");

        // over the cap → one generation aside, and the next append starts clean
        std::fs::write(&file, "x".repeat(UI_EVENTS_MAX as usize + 1)).unwrap();
        ui_events_append_at(&file, &[ev("act", "after.rotate", "nav", 1_788_651_000_000, 0)]);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);
        assert!(dir.join("ui-events.jsonl.1").exists(), "the old generation is kept, once");

        // …and a read spans both generations
        std::fs::write(dir.join("ui-events.jsonl.1"), "{\"t\":1788651000000,\"k\":\"act\",\"key\":\"older\",\"s\":\"nav\"}\nnot json\n").unwrap();
        let all = read_events(&file);
        assert_eq!(all.len(), 2, "one from each generation, the torn line dropped");
        assert_eq!(all[0].key, "older", "the older generation comes first");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The remote spec is whatever `git remote get-url` says; the web base is
    /// what a browser can open. Every shape git accepts for a hosted repo maps
    /// to one https URL, and everything else (a local clone, an unknown scheme)
    /// is None rather than a made-up link.
    #[test]
    fn a_remote_spec_becomes_one_https_base_or_none() {
        for (spec, want) in [
            ("git@github.com:acme/repo.git", "https://github.com/acme/repo"),
            ("git@github.com:acme/repo", "https://github.com/acme/repo"),
            ("ssh://git@github.com/acme/repo.git", "https://github.com/acme/repo"),
            ("ssh://git@gitea.local:2222/acme/repo.git", "https://gitea.local/acme/repo"),
            ("https://github.com/acme/repo.git", "https://github.com/acme/repo"),
            ("https://github.com/acme/repo", "https://github.com/acme/repo"),
            ("http://gitlab.internal/group/sub/repo.git", "http://gitlab.internal/group/sub/repo"),
            ("  git@github.com:acme/repo.git\n", "https://github.com/acme/repo"),
        ] {
            assert_eq!(normalize_remote(spec).as_deref(), Some(want), "{spec}");
        }
        for spec in ["/Users/x/repo.git", "../sibling", "file:///tmp/repo", "git://github.com/acme/repo", "git@nocolon"] {
            assert_eq!(normalize_remote(spec), None, "{spec}");
        }
    }
}
