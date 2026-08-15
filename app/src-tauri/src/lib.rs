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
use serde::Serialize;
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager, State};
use worktrees_core::ui::CaptureUi;
use worktrees_core::{git, ops, store, sysclock, Project, Ui};

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
    if let Some(places) = v.get_mut("places").and_then(|p| p.as_array_mut()) {
        for place in places.iter_mut() {
            let slug = place.get("slug").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let tmux_up = place.pointer("/tmux_session/up").and_then(|b| b.as_bool()).unwrap_or(false);
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
                    if tmux_up {
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
                } else if tmux_up && effective.is_some() {
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

/// `git init` + an EMPTY first commit, then add the repo to the workspace. The
/// commit is not optional politeness: without it HEAD is unborn and the very
/// next thing the user does (new worktree) fails on an invalid object name.
/// `--allow-empty` keeps it a pure bootstrap — no file is added or touched.
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

/// The empty bootstrap commit. Git refuses to commit without an identity, and
/// its stderr says exactly which `git config` line is missing — pass it through
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
) -> Result<CmdResult, String> {
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
async fn open_place(repo: String, slug: String, fresh: Option<bool>) -> Result<CmdResult, String> {
    run_op(&format!("open {slug} fresh={}", fresh.unwrap_or(false)), &repo, move |p, ui| {
        // Auto-resume: if this place already has a Claude Code conversation on
        // disk, launch the AI pane with the resume arg (-r) instead of cold.
        // `fresh` (right-click "Open fresh") skips it. Gated on the configured
        // AI actually being Claude — appending -r to an arbitrary ai_cmd breaks it.
        let resume = !fresh.unwrap_or(false)
            && ai_is_claude()
            && p.claude_session_present(&p.place_dir(&slug));
        if slug == "(main)" {
            if !worktrees_core::tmux::have_tmux() {
                ui.error("tmux not found");
                return 1;
            }
            let session = p.session_name("(main)");
            let mut ai_cmd = worktrees_core::config::resolve_ai_cmd(None);
            if resume && !ai_cmd.is_empty() {
                ai_cmd = format!("{ai_cmd} {}", worktrees_core::config::resolve_ai_resume_arg());
            }
            // Propagate launch's rc: a failed new-session must reach the UI
            // banner / app.log, not silently report success. Single-pane
            // (spare_shell=false): Claude gets full width; the scratch shell
            // lives in the dock's Terminal tab.
            let ai = ops::ai_launch_for(p, ui, &p.main_root, &ai_cmd);
            ops::launch(p, ui, &p.main_root, &session, "", &ai, false, false)
        } else {
            let mut args = vec![slug, "--no-attach".into(), "--no-spare".into()];
            if resume {
                args.push("-r".into());
            }
            ops::cmd_open(p, ui, &args)
        }
    })
}

/// Auto-resume only applies when the AI pane actually runs Claude Code (same
/// first-word/basename derivation as ops::launch).
fn ai_is_claude() -> bool {
    // Same single derivation core uses for tmux adoption — see
    // `profile::ai_word_of`. Re-deriving it here is how the three copies drifted.
    worktrees_core::profile::ai_word_of(&worktrees_core::config::resolve_ai_cmd(None)) == "claude"
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
    shells: State<'_, Shells>,
) -> Result<CmdResult, String> {
    // core cmd_close sweeps this place's tmux-era sidecars; the owned dock
    // shells are app state, so they're swept here — same rule as before, the
    // dock's scratch shells die with the place.
    let slug_log = slug.clone();
    let mut args = vec![slug.clone()];
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
    } else if r.ok {
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

/// Run `cmd` with piped output and a hard deadline; kill past it. Every
/// update-path subprocess goes through this so a hung login profile or a
/// stalled network can never wedge an invoke forever. Reader threads drain the
/// pipes (a >64KB burst would otherwise deadlock try_wait).
fn run_deadline(mut cmd: std::process::Command, secs: u64) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    let mut child = cmd.spawn()?;
    let so = child.stdout.take();
    let se = child.stderr.take();
    let drain = |s: Option<std::process::ChildStdout>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(mut s) = s {
                let _ = s.read_to_end(&mut b);
            }
            b
        })
    };
    let ho = drain(so);
    let he = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(mut s) = se {
            let _ = s.read_to_end(&mut b);
        }
        b
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("timed out after {secs}s")));
        }
        std::thread::sleep(Duration::from_millis(120));
    };
    Ok(std::process::Output {
        status,
        stdout: ho.join().unwrap_or_default(),
        stderr: he.join().unwrap_or_default(),
    })
}

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
// Claude Code writes ONE probe file per live session at
// `~/.claude/sessions/<pid>.json` — { pid, cwd, status, waitingFor, updatedAt, … }.
// `status` ∈ { busy, idle, waiting, shell, (missing) }; `cwd` is the session's
// working dir = the worktree directory (the app launches pane-0 claude in the
// place path), so it maps 1:1 to a place's `path`. The file is rewritten on
// status TRANSITIONS only — `updatedAt` can be minutes old while genuinely still
// busy — so we DO NOT expire by age. Liveness guard is PID-alive (stale files for
// dead pids are never cleaned on crash). Everything degrades to "no dot":
// missing/unreadable dir → empty; unparseable/incomplete file → skipped;
// dead pid → skipped.

#[derive(serde::Deserialize)]
struct ClaudeProbe {
    pid: i32,
    cwd: String,
    status: String,
}

/// True when `pid` is a live process. `kill(pid, 0)` sends no signal but runs the
/// permission/existence checks: 0 → alive; -1 with errno ESRCH → dead. EPERM
/// (exists but not ours) still means alive. Never mutates the target.
fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: kill with signal 0 performs no action beyond error checking.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    // rc == -1: alive iff the failure is EPERM (exists, not permitted), not ESRCH.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Scan `~/.claude/sessions/*.json`, keeping only LIVE-pid probes, and bucket
/// their `cwd`s by status. Returns (busy_cwds, waiting_cwds). Paths are pushed
/// as-is (the probe cwd and a place's `path` both derive from the same worktree
/// dir, so the frontend matches raw). Any I/O or parse failure degrades to empty.
fn claude_activity() -> (Vec<String>, Vec<String>) {
    let mut busy = Vec::new();
    let mut waiting = Vec::new();
    // Scan EVERY config root, not just `~/.claude`: a profiled session writes its
    // probes under the profile's own dir, so a $HOME-only scan would leave the
    // busy/waiting dots permanently dark for profiled places — a silent
    // degradation with nothing in the log to explain it.
    //
    // Union is safe because each probe carries its own `cwd` and `pid`, so it is
    // self-describing: we never need to know which profile a place is bound to,
    // and a place whose profile changed mid-session still lights up.
    for root in worktrees_core::profile::claude_config_dirs_all() {
        let entries = match std::fs::read_dir(root.join("sessions")) {
            Ok(e) => e,
            Err(_) => continue, // dir missing/unreadable → no dots from this root
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let probe: ClaudeProbe = match serde_json::from_slice(&bytes) {
                Ok(p) => p,          // missing pid/cwd/status → parse fails → skip
                Err(_) => continue,
            };
            if probe.cwd.is_empty() || !pid_alive(probe.pid) {
                continue; // dead pid (or crashed-session stale file) → no dot
            }
            match probe.status.as_str() {
                "busy" => busy.push(probe.cwd),
                "waiting" => waiting.push(probe.cwd),
                _ => {} // idle / shell / anything else → no dot
            }
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
        applog("warn", &format!("history tail seek failed: {}", path.display()));
        return Vec::new();
    }
    let mut buf = Vec::new();
    if let Err(e) = f.read_to_end(&mut buf) {
        applog("warn", &format!("history tail read failed ({e}): {}", path.display()));
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
/// Prompt time is when work STARTED, so where a transcript exists its newest
/// `.jsonl` mtime is taken as the better completion time — bounded by now, and
/// only for places a qualifying prompt already vouched for. mtime alone would
/// re-light every place merely opened, which is exactly what must not happen.
fn backfill_worked(handle: &AppHandle) {
    let roots = read_projects(handle);
    if roots.is_empty() {
        return;
    }
    let now = sysclock::now_epoch();
    let cutoff = now - BACKFILL_WINDOW_SECS;
    let mut newest: HashMap<String, i64> = HashMap::new();
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
                if let Some(m) = mtime_epoch(&jsonl) {
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

/// mtime in epoch seconds, or `None` for anything unreadable.
fn mtime_epoch(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
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

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Claude Code's own UA. Load-bearing: a foreign UA lands in a much more
/// aggressive rate-limit bucket on this endpoint.
const USAGE_UA: &str = "claude-code/2.1.220";
/// Minimum gap between real fetches. The frontend polls at 180s; this is the
/// floor that also covers window-focus pulls and a re-mount storm.
const USAGE_TTL_SECS: i64 = 120;

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
    source: String, // oauth | statusline | unavailable
    fetched_at: i64,
    limits: Vec<UsageLimit>,
}

/// Last SUCCESSFUL oauth answer, with the epoch it was fetched at (see TTL).
static USAGE_CACHE: Mutex<Option<UsageInfo>> = Mutex::new(None);

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

/// Plan-usage bars for the nav footer. Never Errs on "no data" — the widget is
/// ambient, and a banner for a background poll would be worse than a blank
/// corner; the reason goes to app.log instead.
#[tauri::command]
async fn claude_usage() -> Result<UsageInfo, String> {
    let now = sysclock::now_epoch();
    {
        let cache = USAGE_CACHE.lock().unwrap();
        if let Some(c) = cache.as_ref() {
            if now - c.fetched_at < USAGE_TTL_SECS {
                return Ok(c.clone());
            }
        }
    }
    match usage_from_oauth(now) {
        Ok(info) => {
            *USAGE_CACHE.lock().unwrap() = Some(info.clone());
            Ok(info)
        }
        Err(why) => {
            applog("warn", &format!("claude_usage: oauth unavailable: {why}"));
            match usage_from_statusline() {
                Some(info) => Ok(info),
                None => {
                    applog("warn", "claude_usage: no statusline snapshot either — widget hidden");
                    Ok(UsageInfo { source: "unavailable".into(), fetched_at: now, limits: Vec::new() })
                }
            }
        }
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
async fn diagnostics() -> Result<String, String> {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let (cli_path, cli_version) = match cli_binary() {
        Some((p, v)) => (p, v),
        None => ("(not found)".to_string(), "(unknown)".to_string()),
    };
    let path = std::env::var("PATH").unwrap_or_default();
    let (git_path, git_version) = tool_report("git");
    let (tmux_path, tmux_version) = tool_report("tmux");

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

/// Web URL for a place's branch on its origin remote: a /tree/<branch> link on
/// github.com, the repo home for other hosts, None with no origin. The UI opens
/// it via the opener plugin.
#[tauri::command]
async fn github_url(repo: String, slug: String) -> Result<Option<String>, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| e.msg)?;
    let Some(remote) = worktrees_core::git::git_out(&p.main_root, &["remote", "get-url", "origin"])
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    let Some(base) = normalize_remote(&remote) else {
        return Ok(None);
    };
    let branch = p.wt_branch(&p.place_dir(&slug));
    if branch.is_empty() || branch == "(detached)" || !base.starts_with("https://github.com/") {
        return Ok(Some(base));
    }
    Ok(Some(format!("{base}/tree/{branch}")))
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
    for cand in ["origin/main", "origin/master", "main", "master"] {
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
#[tauri::command]
async fn write_file(app: AppHandle, path: String, content: String, expected_mtime: Option<u64>) -> Result<(), String> {
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
    })
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
    kill_shell(&shells, &(repo.clone(), slug.clone(), index));
    // A tab the user CLOSED forgets where it was, exactly as it drops its name
    // (App.tsx `closeTab`) — otherwise the next tab to take this index would
    // open in a directory it never visited. A tab being RESTARTED goes through
    // this same command to reap the corpse and asks to KEEP its directory: it
    // is the same tab, and restarting a shell that died in a subdirectory only
    // to land at the place root is the exact papercut this feature removes.
    if !keep_cwd.unwrap_or(false) {
        edit_cwds(&app, |map| forget_tab(map, &repo, &slug, index));
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
    // cmd_rm sweeps this place's dock shell sidecars itself (core, only once the
    // removal proceeds past its dirty/confirm guards — a refused rm keeps them).
    let r = run_op(&format!("rm {slug_log}"), &repo, move |p, ui| ops::cmd_rm(p, ui, &args))?;
    if r.ok {
        kill_place_shells(&shells, &repo, &slug_sweep);
        // The place is gone for good, so its remembered directories are too.
        // (A `close` deliberately does NOT do this: closing keeps the tab names,
        // so it has to keep what those tabs point at.)
        edit_cwds(&app, |map| map.remove(&cwd_key(&repo, &slug_sweep)).is_some());
    }
    Ok(r)
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

/// Start (or re-attach to) the dock shell for `index` and stream it to
/// `on_bytes`; returns the attach generation `shell_detach` must present.
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
) -> Result<u64, String> {
    let key: ShellKey = (repo.clone(), slug.clone(), index);
    let mut map = shells.0.lock().unwrap();
    if let Some(sh) = map.get_mut(&key) {
        // ring THEN sink, the same order the reader takes them — otherwise a
        // write landing mid-replay is either sent twice or dropped
        let ring = sh.ring.lock().unwrap();
        let mut sink = sh.sink.lock().unwrap();
        let snapshot: Vec<u8> = ring.iter().copied().collect();
        if !snapshot.is_empty() {
            let _ = on_bytes.send(InvokeResponseBody::Raw(snapshot));
        }
        *sink = Some(on_bytes);
        drop(sink);
        drop(ring);
        let _ = sh.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        sh.gen += 1;
        return Ok(sh.gen);
    }

    let (session, cwd) = place_session_cwd(&repo, &slug)?;
    // One-time cleanup for anyone upgrading: their `<session>~term*` sidecars
    // are orphans now — nothing will ever attach them again. Cheap and
    // idempotent, and only reached when this place has no shell yet. (Runs
    // under the registry lock — a tmux round-trip, but only on first open.)
    if worktrees_core::tmux::have_tmux() {
        worktrees_core::tmux::kill_shell_sidecars(&session);
    }

    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;

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

    let child = pair.slave.spawn_command(cmd).map_err(|e| {
        applog("error", &format!("shell_open {slug}#{index}: spawn {shell_bin} failed: {e}"));
        e.to_string()
    })?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let ring = Arc::new(Mutex::new(VecDeque::<u8>::with_capacity(8192)));
    let sink = Arc::new(Mutex::new(Some(on_bytes)));
    let stop = Arc::new(AtomicBool::new(false));

    let (r_ring, r_sink, r_stop) = (ring.clone(), sink.clone(), stop.clone());
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

    map.insert(key, Shell { master: pair.master, writer, child, stop, ring, sink, gen: 1 });
    Ok(1)
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
    let map = shells.0.lock().unwrap();
    let sh = map.get(&(repo, slug, index)).ok_or("no such shell")?;
    sh.master
        .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(Terminals::default())
        .manage(Shells::default())
        .setup(|app| {
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
                // Cold start: what happened while the app was closed. Runs before
                // the first sleep so the nav's afterglow is right on frame one.
                backfill_worked(&handle);
                // Same cold-start slot: drop remembered directories for places
                // that were removed while the app was closed.
                forget_vanished_places(&handle);
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
                    cwd_ticks += 1;
                    if cwd_ticks >= 5 {
                        cwd_ticks = 0;
                        save_shell_cwds(&handle, &handle.state::<Shells>());
                    }
                    let fp = worktrees_core::tmux::session_fingerprint();
                    ticks += 1;
                    if fp != last || ticks >= 10 {
                        last = fp;
                        ticks = 0;
                        let _ = handle.emit("places:changed", ());
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
            add_project,
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
            new_place,
            switch_place,
            list_branches,
            remove_place,
            open_place,
            close_place,
            github_url,
            fetch_origin,
            set_fetch_interval,
            check_update,
            update_cli,
            get_ai_config,
            project_config_read,
            doctor,
            relink,
            provision,
            init_suggest,
            init_write,
            diagnostics,
            tmux_check,
            claude_usage,
            log_info,
            log_event,
            log_tail,
            get_changelog,
            open_editor,
            open_terminal,
            list_dir,
            changed_files,
            read_file,
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
            term_close
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
                let keys: Vec<ShellKey> = shells.0.lock().unwrap().keys().cloned().collect();
                for k in &keys {
                    kill_shell(&shells, k);
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
}
