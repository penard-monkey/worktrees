// worktrees UI — Tauri backend. Uses worktrees-core as a LIBRARY (in-process; no
// subprocess, no WORKTREES_BIN). Two jobs of its own:
//   1. state    — core computes derived `ls`; core::store owns the declared sidecar;
//                 the app merges them + reconciles lifecycle_effective for the UI.
//   2. PTY host — attaches to a live tmux session for the place's canonical
//                 shell, and OWNS the dock's scratch shells outright (no tmux).
// See DESIGN.md / MIGRATION.md.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager};
use worktrees_core::{provider, sync, sysclock};

// The command layer. Every handler below is a wrapper over one of these; what
// is left in this file is the three things a window has that a socket does
// not — the updater, page zoom, and window state — plus the poll that drives
// the nav dots and the `Ctx` the wrappers resolve from.
use worktrees_api::files::ShellRef;
use worktrees_api::ops::{
    draft_trace_filter, fetch_origin_root, scan_drafts, trace_spawns, CmdResult, Draft, Drafts,
    UpdateInfo, FETCH_INTERVAL_SECS, REPO_SLUG,
};
use worktrees_api::prefs::UiEvent;
use worktrees_api::projects::read_projects;
use worktrees_api::status::{list_published_releases, valid_tag};
use worktrees_api::util::fmt_utc;
use worktrees_api::viewer;

/// Is tmux reachable right now? The body re-resolves the GUI `PATH` when
/// asked, which is why it cannot be sync: the login-shell probe is
/// deadline-guarded at 5s and would freeze the main thread for all of it.
#[tauri::command]
async fn tmux_check(refresh: bool) -> Result<bool, String> {
    worktrees_api::hostenv::tmux_check(refresh)
}
use worktrees_core::claude_usage::UsageInfo;
// The nav-dot poll below drives these; the derivations themselves are the
// command layer's, so the app and `worktrees serve` light the same dots.
use worktrees_api::watch::{
    backfill_worked, claude_activity, completion_edges, drain_inbox, harness_feed, merge_activity,
    new_dones, stamp_worked, Activity, ClaudeActivity, Feed, LaneTick, TaskDone,
};

// The documentation viewer: one supervised child, N places, and a port. Its own
// file because it is the only part of this backend that can hand a document to
// something outside the app, and every rule in it is a refusal — the security
// surface is worth reading in one piece.
mod winstate;
// Moved to worktrees-core so the LAUNCH path can read it too: a harness that
// wants to weigh remaining quota before starting a session runs in the CLI and
// in `worktrees mcp`, neither of which can reach anything under app/src-tauri.
use worktrees_core::codex_usage;

// ── app log ──────────────────────────────────────────────────────────────────
// The sink behind `worktrees_api::log`: a plain append-only file at the
// platform's log location (macOS: ~/Library/Logs/<identifier>/app.log —
// Console.app finds it). Deliberately AppHandle-free so the panic hook can use
// it. Every op failure, frontend error, and panic lands here; Settings → Logs
// opens the folder / tails it.

/// The identifier to log under when nothing has said otherwise — what a
/// `tauri build` bundle carries.
const APP_IDENT: &str = "net.casadelvalle.worktrees";

/// The identifier this PROCESS is actually running as, which is not always
/// `APP_IDENT`: `sandbox.sh --app` passes
/// `--config '{"identifier":"net.casadelvalle.worktrees.sbx"}'`, and while the
/// log read a `const` instead, every sandbox wrote into the REAL app's log —
/// the gap AGENTS.md records under `sandbox.sh`. Resolved once at startup from
/// the live config. A `OnceLock` and not a parameter because the panic hook
/// has no handle to pass one.
static IDENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn ident() -> &'static str {
    IDENT.get().map(String::as_str).unwrap_or(APP_IDENT)
}

fn log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Logs").join(ident())
    } else {
        // XDG-ish fallback (linux dev)
        PathBuf::from(home).join(".local/share").join(ident()).join("logs")
    }
}

fn log_file() -> PathBuf {
    log_dir().join("app.log")
}

/// The app's log sink. `worktrees_api::log::install` takes one of these at
/// startup, and from then on every `crate::log::log` in the command layer —
/// and `applog` below — lands in this file.
struct AppLog;

impl worktrees_api::log::LogSink for AppLog {
    fn log(&self, level: &str, msg: &str) {
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

    fn location(&self) -> Option<(PathBuf, PathBuf)> {
        Some((log_dir(), log_file()))
    }
}

/// Unchanged for its ~150 call sites, and now one line: the log moved to
/// `worktrees_api::log` so that handlers extracted out of this file keep
/// logging without carrying a handle for it.
fn applog(level: &str, msg: &str) {
    worktrees_api::log::log(level, msg);
}

// ── the command layer's Ctx ──────────────────────────────────────────────────
// `worktrees-api` holds the handler bodies; `Ctx` is what it takes in place of
// `AppHandle` + `State<_>`. The app builds ONE at startup and `.manage()`s it,
// so every wrapper below resolves the same registries — the nav-dot watches in
// particular are per-process by nature and must not be rebuilt per call.

/// The app's `Host`: events go out over Tauri's event bus.
struct AppHost(AppHandle);

impl worktrees_api::Host for AppHost {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if let Err(e) = self.0.emit(event, payload) {
            applog("warn", &format!("emit {event}: {e}"));
        }
    }
}

/// The managed `Ctx`. Cheap to clone (every field is an `Arc` or a `PathBuf`),
/// so a wrapper takes a clone rather than holding the `State` borrow across an
/// await.
fn ctx(app: &AppHandle) -> worktrees_api::Ctx {
    app.state::<worktrees_api::Ctx>().inner().clone()
}

#[tauri::command]
async fn log_info() -> Result<worktrees_api::meta::LogInfo, String> {
    Ok(worktrees_api::meta::log_info())
}

#[tauri::command]
async fn log_event(level: String, msg: String) -> Result<(), String> {
    worktrees_api::meta::log_event(&level, msg);
    Ok(())
}

#[tauri::command]
async fn get_changelog() -> Result<worktrees_api::meta::ChangelogInfo, String> {
    Ok(worktrees_api::meta::get_changelog())
}

#[tauri::command]
async fn log_tail(lines: Option<usize>) -> Result<String, String> {
    Ok(worktrees_api::meta::log_tail(lines))
}

// ── AI profiles + the skill store ────────────────────────────────────────────
// Bodies in `worktrees_api::profiles`.

#[tauri::command]
async fn profiles_info(repo: String) -> Result<serde_json::Value, String> {
    worktrees_api::profiles::profiles_info(repo)
}

#[tauri::command]
async fn save_profile(profile: serde_json::Value) -> Result<String, String> {
    worktrees_api::profiles::save_profile(profile)
}

#[tauri::command]
async fn new_profile_id(name: String) -> Result<String, String> {
    worktrees_api::profiles::new_profile_id(name)
}

#[tauri::command]
async fn delete_profile(id: String) -> Result<worktrees_api::profiles::ProfileRemoval, String> {
    worktrees_api::profiles::delete_profile(id)
}

#[tauri::command]
async fn set_project_profile(repo: String, id: Option<String>) -> Result<(), String> {
    worktrees_api::profiles::set_project_profile(repo, id)
}

#[tauri::command]
async fn set_default_profile(id: Option<String>) -> Result<(), String> {
    worktrees_api::profiles::set_default_profile(id)
}

#[tauri::command]
async fn skills_list() -> Result<serde_json::Value, String> {
    worktrees_api::profiles::skills_list()
}

#[tauri::command]
async fn skill_inspect(path: String) -> Result<serde_json::Value, String> {
    worktrees_api::profiles::skill_inspect(path)
}

#[tauri::command]
async fn skill_install_local(path: String) -> Result<serde_json::Value, String> {
    worktrees_api::profiles::skill_install_local(path)
}

#[tauri::command]
async fn skill_preview_git(url: String, rev: Option<String>) -> Result<serde_json::Value, String> {
    worktrees_api::profiles::skill_preview_git(url, rev)
}

#[tauri::command]
async fn skill_install_git(
    url: String,
    rev: Option<String>,
    sha: String,
    name: String,
) -> Result<serde_json::Value, String> {
    worktrees_api::profiles::skill_install_git(url, rev, sha, name)
}

#[tauri::command]
async fn skill_remove(name: String) -> Result<Vec<String>, String> {
    worktrees_api::profiles::skill_remove(name)
}

// ── projects and their snapshots ─────────────────────────────────────────────
// Bodies in `worktrees_api::workspace`.

#[tauri::command]
async fn list_places(app: AppHandle, repo: String) -> Result<serde_json::Value, String> {
    worktrees_api::workspace::list_places(&ctx(&app), repo)
}

#[tauri::command]
async fn list_workspace(app: AppHandle) -> Result<worktrees_api::workspace::Workspace, String> {
    worktrees_api::workspace::list_workspace(&ctx(&app))
}

#[tauri::command]
async fn add_project(app: AppHandle, dir: String) -> Result<worktrees_api::workspace::Workspace, String> {
    worktrees_api::workspace::add_project(&ctx(&app), dir)
}

#[tauri::command]
async fn probe_dir(dir: String) -> Result<worktrees_api::workspace::DirProbe, String> {
    worktrees_api::workspace::probe_dir(dir)
}

#[tauri::command]
async fn create_project(
    app: AppHandle,
    location: String,
    name: String,
) -> Result<worktrees_api::workspace::Workspace, String> {
    worktrees_api::workspace::create_project(&ctx(&app), location, name)
}

/// The clone's progress stream. `Channel` is Tauri's; the command layer takes
/// a plain callback, so this wrapper owns the one thing a transport knows and
/// the engine does not — that the client has gone away. Logged once, then
/// quiet, and the clone runs on regardless.
#[tauri::command]
async fn clone_project(
    app: AppHandle,
    id: String,
    url: String,
    parent: String,
    name: Option<String>,
    on_progress: Channel<worktrees_core::clone::CloneProgress>,
) -> Result<worktrees_api::workspace::CloneDone, worktrees_core::clone::CloneError> {
    let c = ctx(&app);
    // The WHOLE body is blocking now (it ends in a git fan-out over every
    // project), so the whole call goes to the blocking pool rather than just
    // the clone — commands must not run their work on the main thread.
    tauri::async_runtime::spawn_blocking(move || {
        let mut dead = false;
        let mut sink = |p: worktrees_core::clone::CloneProgress| {
            if dead {
                return;
            }
            if let Err(e) = on_progress.send(p) {
                dead = true;
                applog("warn", &format!("clone progress channel closed: {e}"));
            }
        };
        worktrees_api::workspace::clone_project(&c, id, url, parent, name, &mut sink)
    })
    .await
    .map_err(|e| worktrees_core::clone::CloneError {
        kind: worktrees_core::clone::CloneErrorKind::Other,
        message: e.to_string(),
    })?
}

#[tauri::command]
async fn clone_cancel(id: String) -> Result<bool, String> {
    worktrees_api::workspace::clone_cancel(id)
}

#[tauri::command]
async fn init_repo(app: AppHandle, dir: String) -> Result<worktrees_api::workspace::Workspace, String> {
    worktrees_api::workspace::init_repo(&ctx(&app), dir)
}

#[tauri::command]
async fn create_initial_commit(
    app: AppHandle,
    repo: String,
) -> Result<worktrees_api::workspace::Workspace, String> {
    worktrees_api::workspace::create_initial_commit(&ctx(&app), repo)
}

#[tauri::command]
async fn remove_project(app: AppHandle, root: String) -> Result<worktrees_api::workspace::Workspace, String> {
    worktrees_api::workspace::remove_project(&ctx(&app), root)
}

#[tauri::command]
async fn reorder_projects(
    app: AppHandle,
    roots: Vec<String>,
) -> Result<worktrees_api::workspace::Workspace, String> {
    worktrees_api::workspace::reorder_projects(&ctx(&app), roots)
}

// ── settings, planning, and the mutating verbs ───────────────────────────────
// Bodies in `worktrees_api::ops`.

#[tauri::command]
async fn settings_info(app: AppHandle) -> Result<worktrees_api::ops::SettingsInfo, String> {
    worktrees_api::ops::settings_info(&ctx(&app))
}

#[tauri::command]
async fn get_settings(app: AppHandle) -> Result<Option<serde_json::Value>, String> {
    worktrees_api::ops::get_settings(&ctx(&app))
}

#[tauri::command]
async fn set_settings(app: AppHandle, settings: serde_json::Value) -> Result<(), String> {
    worktrees_api::ops::set_settings(&ctx(&app), settings)
}

#[tauri::command]
async fn set_lifecycle(repo: String, slug: String, label: String) -> Result<(), String> {
    worktrees_api::ops::set_lifecycle(repo, slug, label)
}

#[tauri::command]
async fn set_pin(repo: String, slug: String, on: bool) -> Result<(), String> {
    worktrees_api::ops::set_pin(repo, slug, on)
}

#[tauri::command]
async fn set_note(repo: String, slug: String, note: String) -> Result<(), String> {
    worktrees_api::ops::set_note(repo, slug, note)
}

#[tauri::command]
async fn drop_reference(
    repo: String,
    slug: String,
    into_repo: String,
    into_slug: String,
    into_session: String,
    provider: Option<String>,
) -> Result<String, String> {
    worktrees_api::ops::drop_reference(repo, slug, into_repo, into_slug, into_session, provider)
}

#[tauri::command]
async fn cross_project_status() -> Result<worktrees_api::ops::CrossProjectStatus, String> {
    worktrees_api::ops::cross_project_status()
}

#[tauri::command]
async fn set_cross_project(
    level: String,
) -> Result<worktrees_api::ops::CrossProjectStatus, String> {
    worktrees_api::ops::set_cross_project(level)
}

#[tauri::command]
async fn planning_status() -> Result<worktrees_api::ops::PlanningStatus, String> {
    worktrees_api::ops::planning_status()
}

#[tauri::command]
async fn set_planning_default(
    default: String,
) -> Result<worktrees_api::ops::PlanningStatus, String> {
    worktrees_api::ops::set_planning_default(default)
}

#[tauri::command]
async fn set_project_planning(
    root: String,
    level: Option<String>,
    plan_path: Option<String>,
    scope: Option<String>,
) -> Result<worktrees_api::ops::PlanningStatus, String> {
    worktrees_api::ops::set_project_planning(root, level, plan_path, scope)
}

#[tauri::command]
async fn planning_suggested(root: String) -> Result<bool, String> {
    worktrees_api::ops::planning_suggested(root)
}

#[tauri::command]
async fn quota_settings() -> Result<worktrees_api::ops::QuotaSettings, String> {
    worktrees_api::ops::quota_settings()
}

#[tauri::command]
async fn set_quota_settings(
    gate: bool,
    weekly_warn_pct: Option<u8>,
) -> Result<worktrees_api::ops::QuotaSettings, String> {
    worktrees_api::ops::set_quota_settings(gate, weekly_warn_pct)
}

#[tauri::command]
async fn set_project_private(
    root: String,
    private: bool,
) -> Result<worktrees_api::ops::CrossProjectStatus, String> {
    worktrees_api::ops::set_project_private(root, private)
}

#[tauri::command]
async fn copy_text(text: String) -> Result<(), String> {
    worktrees_api::ops::copy_text(text)
}

#[tauri::command]
async fn set_title(repo: String, slug: String, title: String) -> Result<(), String> {
    worktrees_api::ops::set_title(repo, slug, title)
}

#[tauri::command]
async fn touch_place(repo: String, slug: String) -> Result<(), String> {
    worktrees_api::ops::touch_place(repo, slug)
}

#[tauri::command]
async fn mark_seen(repo: String, slug: String, epoch: i64) -> Result<(), String> {
    worktrees_api::ops::mark_seen(repo, slug, epoch)
}

#[tauri::command]
async fn new_place(
    repo: String,
    branch: String,
    base: Option<String>,
    name: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    force: Option<bool>,
) -> Result<worktrees_api::ops::CmdResult, String> {
    worktrees_api::ops::new_place(repo, branch, base, name, provider, model, force)
}

#[tauri::command]
async fn switch_place(
    repo: String,
    slug: String,
    branch: String,
    base: Option<String>,
) -> Result<worktrees_api::ops::CmdResult, String> {
    worktrees_api::ops::switch_place(repo, slug, branch, base)
}

#[tauri::command]
async fn list_branches(
    repo: String,
    slug: String,
) -> Result<worktrees_api::ops::BranchList, String> {
    worktrees_api::ops::list_branches(repo, slug)
}

#[tauri::command]
async fn open_place(
    repo: String,
    slug: String,
    fresh: Option<bool>,
    provider: Option<String>,
    model: Option<String>,
    force: Option<bool>,
) -> Result<worktrees_api::ops::CmdResult, String> {
    worktrees_api::ops::open_place(repo, slug, fresh, provider, model, force)
}

#[tauri::command]
async fn set_fetch_interval(mins: u64) -> Result<(), String> {
    worktrees_api::ops::set_fetch_interval(mins)
}

#[tauri::command]
async fn fetch_origin(root: String) -> Result<(), String> {
    worktrees_api::ops::fetch_origin(root)
}

#[tauri::command]
async fn list_drafts() -> Result<Vec<worktrees_api::ops::Draft>, String> {
    worktrees_api::ops::list_drafts()
}


#[tauri::command]
async fn close_place(
    app: AppHandle,
    repo: String,
    slug: String,
    yes: bool,
    session: Option<String>,
    provider: Option<String>,
) -> Result<CmdResult, String> {
    let c = ctx(&app);
    worktrees_api::ops::close_place(repo, slug, yes, session, provider, &c.shells)
}

// ── usage, service status, releases ──────────────────────────────────────────
// Bodies in `worktrees_api::status`.

#[tauri::command]
async fn claude_usage() -> Result<UsageInfo, String> {
    worktrees_api::status::claude_usage()
}

#[tauri::command]
async fn codex_usage() -> Result<codex_usage::Info, String> {
    worktrees_api::status::codex_usage()
}

#[tauri::command]
async fn claude_status() -> Result<worktrees_api::status::ClaudeStatus, String> {
    worktrees_api::status::claude_status()
}

#[tauri::command]
async fn check_update() -> Result<UpdateInfo, String> {
    worktrees_api::status::check_update()
}

#[tauri::command]
async fn update_cli(tag: String) -> Result<CmdResult, String> {
    worktrees_api::status::update_cli(tag)
}

#[tauri::command]
async fn list_releases() -> Result<Vec<worktrees_api::status::ReleaseRow>, String> {
    worktrees_api::status::list_releases()
}

// Stays here: it drives `tauri_plugin_updater` against one release's own
// manifest — the app replacing its own bundle, which is client-local and
// means nothing to a server whose clients are browsers.

/// Install the signed app bundle of exactly `tag` — up or down. The updater is
/// pointed at that release's own manifest and its comparator accepts only that
/// version, so the signature check is the stock one and nothing else can land.
/// The caller relaunches.
#[tauri::command]
async fn install_app_version(app: AppHandle, tag: String) -> Result<String, String> {
    use tauri_plugin_updater::UpdaterExt;
    if !valid_tag(&tag) {
        return Err(format!("suspicious tag '{tag}'"));
    }
    let want = tag[1..].to_string();
    if want == env!("CARGO_PKG_VERSION") {
        return Err(format!("already running {tag}"));
    }
    if !list_published_releases()?.iter().any(|r| r.tag == tag) {
        return Err(format!("{tag} is not a published release"));
    }
    let url = tauri::Url::parse(&format!("https://github.com/{REPO_SLUG}/releases/download/{tag}/latest.json"))
        .map_err(|e| e.to_string())?;
    let cmp = want.clone();
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .version_comparator(move |_current, remote| remote.version.to_string() == cmp)
        .build()
        .map_err(|e| e.to_string())?;
    let up = updater
        .check()
        .await
        .map_err(|e| format!("{tag}: {e}"))?
        .ok_or_else(|| format!("{tag}'s manifest does not describe version {want}"))?;
    applog("info", &format!("install_app_version: {} → {tag}", env!("CARGO_PKG_VERSION")));
    up.download_and_install(|_, _| {}, || {}).await.map_err(|e| {
        applog("error", &format!("install_app_version {tag}: {e}"));
        format!("{tag}: {e}")
    })?;
    Ok(up.version)
}

// ── config surfaces, MCP setup, Ask Claude ───────────────────────────────────
// Bodies in `worktrees_api::config`.

#[tauri::command]
async fn get_ai_config() -> Result<worktrees_api::config::AiConfig, String> {
    worktrees_api::config::get_ai_config()
}

#[tauri::command]
async fn mcp_status(
    repo: Option<String>,
) -> Result<worktrees_core::mcpsetup::Status, String> {
    worktrees_api::config::mcp_status(repo)
}

#[tauri::command]
async fn mcp_install(
    repo: Option<String>,
    mutations: bool,
) -> Result<worktrees_core::mcpsetup::Outcome, String> {
    worktrees_api::config::mcp_install(repo, mutations)
}

#[tauri::command]
async fn mcp_uninstall(
    repo: Option<String>,
) -> Result<worktrees_core::mcpsetup::Outcome, String> {
    worktrees_api::config::mcp_uninstall(repo)
}

#[tauri::command]
async fn codex_mcp_status() -> Result<worktrees_core::codexmcp::Status, String> {
    worktrees_api::config::codex_mcp_status()
}

#[tauri::command]
async fn codex_mcp_install(
    mutations: bool,
) -> Result<worktrees_core::codexmcp::Outcome, String> {
    worktrees_api::config::codex_mcp_install(mutations)
}

#[tauri::command]
async fn codex_mcp_uninstall() -> Result<worktrees_core::codexmcp::Outcome, String> {
    worktrees_api::config::codex_mcp_uninstall()
}

#[tauri::command]
async fn pi_mcp_status() -> Result<worktrees_core::pimcp::Status, String> {
    worktrees_api::config::pi_mcp_status()
}

#[tauri::command]
async fn pi_mcp_install(mutations: bool) -> Result<worktrees_core::pimcp::Outcome, String> {
    worktrees_api::config::pi_mcp_install(mutations)
}

#[tauri::command]
async fn pi_mcp_uninstall() -> Result<worktrees_core::pimcp::Outcome, String> {
    worktrees_api::config::pi_mcp_uninstall()
}

#[tauri::command]
async fn codex_mcp_migration_plan() -> Result<Vec<worktrees_core::mcpmigrate::Row>, String> {
    worktrees_api::config::codex_mcp_migration_plan()
}

#[tauri::command]
async fn codex_mcp_migrate(
    names: Vec<String>,
) -> Result<Vec<worktrees_core::mcpmigrate::Outcome>, String> {
    worktrees_api::config::codex_mcp_migrate(names)
}

#[tauri::command]
async fn project_config_read(
    repo: String,
) -> Result<worktrees_api::config::ProjectConfigView, String> {
    worktrees_api::config::project_config_read(repo)
}

#[tauri::command]
async fn doctor(
    repo: String,
    slug: Option<String>,
) -> Result<worktrees_api::config::DoctorReport, String> {
    worktrees_api::config::doctor(repo, slug)
}

#[tauri::command]
async fn place_health(
    repo: String,
    slug: String,
) -> Result<worktrees_api::config::HealthReport, String> {
    worktrees_api::config::place_health(repo, slug)
}

#[tauri::command]
async fn ai_status_report(
    repo: String,
    slug: String,
) -> Result<worktrees_api::config::AiStatusReport, String> {
    worktrees_api::config::ai_status_report(repo, slug)
}

#[tauri::command]
async fn relink(
    repo: String,
    slug: Option<String>,
    force: bool,
) -> Result<CmdResult, String> {
    worktrees_api::config::relink(repo, slug, force)
}

#[tauri::command]
async fn provision(repo: String, slug: Option<String>) -> Result<CmdResult, String> {
    worktrees_api::config::provision(repo, slug)
}

// ── sync, agent setup, diagnostics, pull requests ───────────────────────────
// Bodies in `worktrees_api::project`.

#[tauri::command]
async fn sync_status(repo: String) -> Result<worktrees_api::project::SyncStatusView, String> {
    worktrees_api::project::sync_status(repo)
}

#[tauri::command]
async fn sync_hub_list() -> Result<sync::StatusJson, String> {
    worktrees_api::project::sync_hub_list()
}

#[tauri::command]
async fn sync_preview(
    repo: Option<String>,
    name: Option<String>,
    direction: String,
    with_sessions: bool,
) -> Result<sync::SyncPreviewOut, String> {
    worktrees_api::project::sync_preview(repo, name, direction, with_sessions)
}

#[tauri::command]
async fn init_suggest(
    repo: String,
) -> Result<worktrees_api::project::InitSuggestion, String> {
    worktrees_api::project::init_suggest(repo)
}

#[tauri::command]
async fn init_write(repo: String) -> Result<CmdResult, String> {
    worktrees_api::project::init_write(repo)
}

#[tauri::command]
async fn agent_setup_status(
    repo: String,
) -> Result<worktrees_api::project::AgentSetupStatus, String> {
    worktrees_api::project::agent_setup_status(repo)
}

#[tauri::command]
async fn agent_setup_fix(
    app: AppHandle,
    repo: String,
    push: Option<bool>,
) -> Result<worktrees_core::agentfiles::FixOutcome, String> {
    worktrees_api::project::agent_setup_fix(&ctx(&app), repo, push)
}

#[tauri::command]
async fn rules_apply(
    app: AppHandle,
    repo: String,
    push: Option<bool>,
    force: Option<bool>,
) -> Result<worktrees_core::agentfiles::FixOutcome, String> {
    worktrees_api::project::rules_apply(&ctx(&app), repo, push, force)
}

#[tauri::command]
async fn rules_review(
    repo: String,
    provider: Option<String>,
    model: Option<String>,
) -> Result<CmdResult, String> {
    worktrees_api::project::rules_review(repo, provider, model)
}

#[tauri::command]
async fn rules_lane_remove(app: AppHandle, repo: String) -> Result<CmdResult, String> {
    worktrees_api::project::rules_lane_remove(&ctx(&app), repo)
}

#[tauri::command]
async fn agent_guidance_status() -> Result<worktrees_core::guidance::Status, String> {
    worktrees_api::project::agent_guidance_status()
}

#[tauri::command]
async fn set_agent_guidance(
    enabled: bool,
    guard: bool,
) -> Result<worktrees_core::guidance::Status, String> {
    worktrees_api::project::set_agent_guidance(enabled, guard)
}

#[tauri::command]
async fn set_agent_guidance_skill(
    text: Option<String>,
) -> Result<worktrees_core::guidance::Status, String> {
    worktrees_api::project::set_agent_guidance_skill(text)
}

#[tauri::command]
async fn agent_guidance_diff(old: String, new: String) -> Result<String, String> {
    worktrees_api::project::agent_guidance_diff(old, new)
}

#[tauri::command]
async fn agent_guidance_merge() -> Result<worktrees_core::guidance::Merged, String> {
    worktrees_api::project::agent_guidance_merge()
}

#[tauri::command]
async fn agent_user_skills() -> Result<Vec<worktrees_core::agentfiles::UserSkill>, String> {
    worktrees_api::project::agent_user_skills()
}

#[tauri::command]
async fn agent_link_skills() -> Result<Vec<String>, String> {
    worktrees_api::project::agent_link_skills()
}

#[tauri::command]
async fn set_codex_permissions(mode: String) -> Result<(), String> {
    worktrees_api::project::set_codex_permissions(mode)
}

#[tauri::command]
async fn agent_models(
    harness: String,
) -> Result<Vec<worktrees_core::choice::ModelOption>, String> {
    worktrees_api::project::agent_models(harness)
}

#[tauri::command]
async fn pi_status(repo: Option<String>) -> Result<worktrees_api::project::PiStatus, String> {
    worktrees_api::project::pi_status(repo)
}

#[tauri::command]
async fn set_pi_trust(mode: String) -> Result<(), String> {
    worktrees_api::project::set_pi_trust(mode)
}

#[tauri::command]
async fn set_pi_allowed(repo: String, allow: bool) -> Result<bool, String> {
    worktrees_api::project::set_pi_allowed(repo, allow)
}

#[tauri::command]
async fn remote_url(repo: String) -> Result<Option<String>, String> {
    worktrees_api::project::remote_url(repo)
}

#[tauri::command]
async fn project_prs(
    repo: String,
    max_age_secs: u64,
    places: Vec<worktrees_core::github::PlaceBranch>,
) -> Result<worktrees_api::project::PrsReply, String> {
    worktrees_api::project::project_prs(repo, max_age_secs, places)
}

#[tauri::command]
async fn open_editor(path: String, cmd: String) -> Result<(), String> {
    worktrees_api::project::open_editor(path, cmd)
}

#[tauri::command]
async fn detect_editors() -> Result<Vec<worktrees_api::editors::Editor>, String> {
    worktrees_api::project::detect_editors()
}

#[tauri::command]
async fn open_terminal(cmd: String, session: String) -> Result<(), String> {
    worktrees_api::project::open_terminal(cmd, session)
}

/// Sync's progress stream, same split as `clone_project`: the command layer
/// takes a callback, and this shim owns the one thing only a transport knows —
/// that the window went away. Logged once, then quiet; the transfer runs on.
#[tauri::command]
async fn sync_apply(
    app: AppHandle,
    repo: Option<String>,
    name: Option<String>,
    direction: String,
    with_sessions: bool,
    confirmed: bool,
    install: bool,
    on_progress: Channel<worktrees_core::sync::SyncProgress>,
) -> Result<CmdResult, String> {
    let c = ctx(&app);
    let label = format!("sync {direction}");
    let repo_log = repo.clone().or_else(|| name.clone()).unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        let mut dead = false;
        let mut sink = |p: worktrees_core::sync::SyncProgress| {
            if dead {
                return;
            }
            if let Err(e) = on_progress.send(p) {
                dead = true;
                applog("warn", &format!("{label} progress channel closed repo={repo_log}: {e}"));
            }
        };
        worktrees_api::project::sync_apply(
            &c, repo, name, direction, with_sessions, confirmed, install, &mut sink,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `viewer_path` is resolved here because where a bundled resource lives is a
/// Tauri fact (`resource_dir`); the report itself is the command layer's.
#[tauri::command]
async fn diagnostics(app: AppHandle, gh: Option<bool>) -> Result<String, String> {
    let viewer_path = match viewer::bundle_path(app.path().resource_dir().ok().as_deref()) {
        Some(p) => p.to_string_lossy().into_owned(),
        None => "(no browser bundle — the Docs tab lists and reads without it)".to_string(),
    };
    worktrees_api::project::diagnostics(&ctx(&app), gh, viewer_path)
}

// ── the Files tab: tree, diffs, reads and writes ────────────────────────────
// Bodies in `worktrees_api::files`.

#[tauri::command]
async fn list_dir(
    app: AppHandle,
    path: String,
    show_ignored: Option<bool>,
) -> Result<Vec<worktrees_api::files::FsEntry>, String> {
    worktrees_api::files::list_dir(&ctx(&app), path, show_ignored)
}

#[tauri::command]
async fn changed_files(
    app: AppHandle,
    root: String,
) -> Result<worktrees_api::files::ChangeSet, String> {
    worktrees_api::files::changed_files(&ctx(&app), root)
}

#[tauri::command]
async fn file_diff(
    app: AppHandle,
    path: String,
    against: Option<String>,
) -> Result<worktrees_api::files::FileDiff, String> {
    worktrees_api::files::file_diff(&ctx(&app), path, against)
}

#[tauri::command]
async fn list_docs(
    app: AppHandle,
    repo: String,
    root: String,
) -> Result<worktrees_api::files::DocsIndex, String> {
    worktrees_api::files::list_docs(&ctx(&app), repo, root)
}

#[tauri::command]
async fn place_plan(
    app: AppHandle,
    root: String,
) -> Result<worktrees_core::plan::PlanSummary, String> {
    worktrees_api::files::place_plan(&ctx(&app), root)
}

#[tauri::command]
async fn plan_prompt(session: String, provider: Option<String>) -> Result<(), String> {
    worktrees_api::files::plan_prompt(session, provider)
}

#[tauri::command]
async fn plan_ask_main(
    app: AppHandle,
    root: String,
    session: String,
    provider: Option<String>,
) -> Result<(), String> {
    worktrees_api::files::plan_ask_main(&ctx(&app), root, session, provider)
}

#[tauri::command]
async fn open_docs_viewer(
    app: AppHandle,
    repo: String,
    root: String,
    slug: String,
    path: Option<String>,
    place: worktrees_api::files::ViewerPlace,
) -> Result<String, String> {
    let c = ctx(&app);
    // `resource_dir` is Tauri's; the viewer bundle it points at is the command
    // layer's to serve.
    let res = app.path().resource_dir().ok().map(|p| p.to_string_lossy().into_owned());
    worktrees_api::files::open_docs_viewer(&c, res, &c.viewer, repo, root, slug, path, place)
}

#[tauri::command]
async fn read_file(
    app: AppHandle,
    path: String,
    max_bytes: Option<u64>,
) -> Result<worktrees_api::files::FileContent, String> {
    worktrees_api::files::read_file(&ctx(&app), path, max_bytes)
}

#[tauri::command]
async fn file_readable(app: AppHandle, path: String) -> Result<bool, String> {
    worktrees_api::files::file_readable(&ctx(&app), path)
}

#[tauri::command]
async fn resolve_term_paths(
    app: AppHandle,
    root: String,
    shell: Option<ShellRef>,
    session: Option<String>,
    paths: Vec<String>,
) -> Result<Vec<Option<String>>, String> {
    let c = ctx(&app);
    worktrees_api::files::resolve_term_paths(&c, &c.shells, root, shell, session, paths)
}

#[tauri::command]
async fn resolve_doc_paths(
    app: AppHandle,
    root: String,
    doc: Option<String>,
    generation: u64,
    paths: Vec<String>,
) -> Result<Vec<Vec<worktrees_api::files::DocPathHit>>, String> {
    let c = ctx(&app);
    worktrees_api::files::resolve_doc_paths(&c, &c.docnames, root, doc, generation, paths)
}

#[tauri::command]
async fn read_file_base64(
    app: AppHandle,
    path: String,
    max_bytes: Option<u64>,
) -> Result<worktrees_api::files::FileBlob, String> {
    worktrees_api::files::read_file_base64(&ctx(&app), path, max_bytes)
}

#[tauri::command]
async fn write_file(
    app: AppHandle,
    path: String,
    content: String,
    expected_mtime: Option<u64>,
) -> Result<u64, String> {
    worktrees_api::files::write_file(&ctx(&app), path, content, expected_mtime)
}

// ── backend preferences, the Data pane, local usage metrics ─────────────────
// Bodies in `worktrees_api::prefs`.

#[tauri::command]
async fn set_term_history_opts(
    persist_scrollback: bool,
    per_tab_history: bool,
) -> Result<(), String> {
    worktrees_api::prefs::set_term_history_opts(persist_scrollback, per_tab_history)
}

#[tauri::command]
async fn term_history_info(
    app: AppHandle,
) -> Result<worktrees_api::prefs::TermHistoryInfo, String> {
    worktrees_api::prefs::term_history_info(&ctx(&app))
}

#[tauri::command]
async fn term_history_clear(app: AppHandle) -> Result<(), String> {
    let c = ctx(&app);
    worktrees_api::prefs::term_history_clear(&c, &c.shells)
}

#[tauri::command]
async fn remove_place(
    app: AppHandle,
    repo: String,
    slug: String,
    del_branch: bool,
    force: bool,
) -> Result<CmdResult, String> {
    let c = ctx(&app);
    worktrees_api::prefs::remove_place(&c, repo, slug, del_branch, force, &c.shells)
}

#[tauri::command]
async fn ui_events_append(app: AppHandle, events: Vec<UiEvent>) -> Result<(), String> {
    worktrees_api::prefs::ui_events_append(&ctx(&app), events)
}

#[tauri::command]
async fn ui_usage(
    app: AppHandle,
    days: u32,
    tz_offset_min: i32,
) -> Result<worktrees_api::prefs::UiUsage, String> {
    worktrees_api::prefs::ui_usage(&ctx(&app), days, tz_offset_min)
}

#[tauri::command]
async fn ui_events_clear(app: AppHandle) -> Result<(), String> {
    worktrees_api::prefs::ui_events_clear(&ctx(&app))
}

// ── the dock: shells, scrollback, and the tmux attach ───────────────────────
// Bodies in `worktrees_api::term`.

/// Tauri's byte channel as a `ByteSink`. The command layer streams a pty
/// through the trait; this is the desktop's implementation of it, and the
/// `Channel`'s own ordering is what makes the replay mute in `TerminalPane`
/// safe (AGENTS.md: only the channel's order is reliable).
struct TauriBytes(Channel<InvokeResponseBody>);

impl worktrees_api::ByteSink for TauriBytes {
    fn send(&self, bytes: &[u8]) -> Result<(), String> {
        self.0.send(InvokeResponseBody::Raw(bytes.to_vec())).map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn list_shell_sessions(
    app: AppHandle,
    repo: String,
    slug: String,
) -> Result<Vec<worktrees_api::term::ShellTab>, String> {
    worktrees_api::term::list_shell_sessions(repo, slug, &ctx(&app).shells)
}

#[tauri::command]
async fn close_shell_session(
    app: AppHandle,
    repo: String,
    slug: String,
    index: u32,
    keep_cwd: Option<bool>,
) -> Result<(), String> {
    let c = ctx(&app);
    worktrees_api::term::close_shell_session(&c, repo, slug, index, keep_cwd, &c.shells)
}

#[tauri::command]
async fn term_open(
    app: AppHandle,
    session: String,
    cols: u16,
    rows: u16,
    on_bytes: Channel<InvokeResponseBody>,
) -> Result<u32, String> {
    worktrees_api::term::term_open(session, cols, rows, std::sync::Arc::new(TauriBytes(on_bytes)), &ctx(&app).terminals)
}

#[tauri::command]
async fn term_write(app: AppHandle, id: u32, data: Vec<u8>) -> Result<(), String> {
    worktrees_api::term::term_write(id, data, &ctx(&app).terminals)
}

#[tauri::command]
async fn term_wheel(app: AppHandle, id: u32, lines: i32) -> Result<(), String> {
    worktrees_api::term::term_wheel(id, lines, &ctx(&app).terminals)
}

#[tauri::command]
async fn term_resize(app: AppHandle, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    worktrees_api::term::term_resize(id, cols, rows, &ctx(&app).terminals)
}

#[tauri::command]
async fn term_close(app: AppHandle, id: u32) -> Result<(), String> {
    worktrees_api::term::term_close(id, &ctx(&app).terminals)
}

#[tauri::command]
async fn shell_open(
    app: AppHandle,
    repo: String,
    slug: String,
    index: u32,
    cols: u16,
    rows: u16,
    on_bytes: Channel<InvokeResponseBody>,
) -> Result<worktrees_api::term::ShellAttach, String> {
    let c = ctx(&app);
    worktrees_api::term::shell_open(&c, repo, slug, index, cols, rows, std::sync::Arc::new(TauriBytes(on_bytes)), &c.shells)
}

#[tauri::command]
async fn shell_write(
    app: AppHandle,
    repo: String,
    slug: String,
    index: u32,
    data: Vec<u8>,
) -> Result<(), String> {
    worktrees_api::term::shell_write(repo, slug, index, data, &ctx(&app).shells)
}

#[tauri::command]
async fn shell_resize(
    app: AppHandle,
    repo: String,
    slug: String,
    index: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    worktrees_api::term::shell_resize(repo, slug, index, cols, rows, &ctx(&app).shells)
}

#[tauri::command]
async fn shell_detach(
    app: AppHandle,
    repo: String,
    slug: String,
    index: u32,
    gen: u64,
) -> Result<(), String> {
    worktrees_api::term::shell_detach(repo, slug, index, gen, &ctx(&app).shells)
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


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // FIRST: the app's log sink, before anything can want to log. `applog`,
    // the panic hook below, and every handler now extracted into
    // `worktrees-api` all route through it.
    worktrees_api::log::install(std::sync::Arc::new(AppLog));
    // panics land in the log too (chained: the default stderr hook still runs)
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        applog("panic", &info.to_string());
        prev_hook(info);
    }));
    worktrees_api::hostenv::fixup_gui_path();
    worktrees_api::hostenv::fixup_gui_locale();
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
        .setup(|_app| {
            // Both usage readers moved into core so the CLI and MCP can read
            // them too, and they lost their direct `applog` in the move. Hand
            // it back here, ONCE for the whole engine: the app has a log, the
            // CLI does not, and core treats an unset sink as "do not log"
            // rather than as a reason to fail. A per-module sink would make
            // this a list to keep in step with every reader core gains.
            worktrees_core::logsink::install(|level, msg| applog(level, msg));
            // The identifier this process is really running as. `sandbox.sh
            // --app` overrides it via `--config`, and `log_dir()` reads this
            // rather than the `APP_IDENT` const so a sandbox stops writing
            // into the real app's log. Lines emitted ABOVE this point (the
            // startup line) still land under the default identifier — this is
            // the earliest an app handle exists to ask.
            let _ = IDENT.set(_app.config().identifier.clone());
            // ONE `Ctx` for the process. The config dir is resolved here
            // because it is the earliest point an app handle exists; a failure
            // is logged and falls back to the current directory rather than
            // aborting startup, which is what every `app.path()` call site it
            // replaces did by returning an error the caller logged.
            let config_dir = _app.path().app_config_dir().unwrap_or_else(|e| {
                applog("error", &format!("app_config_dir: {e}"));
                PathBuf::from(".")
            });
            _app.manage(worktrees_api::Ctx::new(
                config_dir,
                std::sync::Arc::new(AppHost(_app.handle().clone())),
            ));
            Ok(())
        })
        // The four registries that used to be `.manage()`d separately are
        // fields on the `Ctx` built in `.setup()` above — one object for the
        // process, so a wrapper resolves them all from the same place. Only
        // the window tracker stays here: it IS the window.
        .manage(winstate::Tracker::default())
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
                // Settings → Behavior → Restore window. Read straight from
                // ui-state.json because this runs before the frontend exists;
                // see winstate.rs for why that file is only ever READ here.
                if let Some(w) = app.get_webview_window("main") {
                    let ui = std::fs::read(dir.join("ui-state.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
                    winstate::attach(&w, &dir, winstate::enabled(ui.as_ref()));
                }
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
                let mut last_waiting_by: std::collections::BTreeMap<&'static str, Vec<String>> = Default::default();
                let mut last_models: Option<Vec<(i32, Option<String>)>> = None;
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
                // The lane completion last stamped per place, per harness (see
                // `new_dones`).
                let mut done_seen: HashMap<&'static str, HashMap<String, i64>> = HashMap::new();
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
                // One `Ctx` for the thread's whole life, and the registries
                // out of it: the dot poll is the hottest reader of both and
                // must not re-resolve them 20 times a minute.
                let pollctx = ctx(&handle);
                let watch = pollctx.watch.clone();
                let watch = watch.as_ref();
                // Cold start: what happened while the app was closed. Runs before
                // the first sleep so the nav's afterglow is right on frame one.
                backfill_worked(&pollctx);
                // Same cold-start slot: drop remembered directories for places
                // that were removed while the app was closed.
                worktrees_api::term::forget_vanished_places(&pollctx);
                // Same cold-start slot, same reasoning one level out: drop saved
                // scrollback for places that are gone, and anything past the age
                // horizon.
                worktrees_api::term::sweep_term_history(&pollctx);
                loop {
                    std::thread::sleep(Duration::from_secs(3));
                    let interval = FETCH_INTERVAL_SECS.load(Ordering::Relaxed);
                    if interval > 0 && last_fetch.elapsed().as_secs() >= interval {
                        for root in read_projects(&pollctx) {
                            let _ = fetch_origin_root(&root);
                        }
                        last_fetch = std::time::Instant::now(); // measure gap AFTER the pass
                        let _ = handle.emit("places:changed", ()); // re-pull fresh ahead/behind once
                    }
                    // Every tick, not every fifth: a request has a 30 s life
                    // and a person is waiting on it. The command layer emits
                    // the `app:open-doc`; raising the window is the half that
                    // cannot move, because a phone has no window of ours.
                    if drain_inbox(&pollctx) > 0 {
                        if let Some(w) = handle.get_webview_window("main") {
                            let _ = w.unminimize();
                            let _ = w.show();
                            if let Err(e) = w.set_focus() {
                                applog("warn", &format!("inbox: could not focus the window: {e}"));
                            }
                        }
                    }
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
                        worktrees_api::term::save_shell_cwds(&pollctx, &pollctx.shells);
                        worktrees_api::term::save_shell_scrollback(&pollctx, &pollctx.shells);
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
                    let (busy, waiting, models) = claude_activity(&watch);
                    // Every harness without a probe file, read once each
                    // (`harness_feed`: codex's rollouts, pi's session files).
                    let lanes: Vec<(&'static str, LaneTick)> = provider::PROVIDERS
                        .iter()
                        .filter_map(|p| match harness_feed(&watch, p.id, &fp) {
                            Feed::Lane(t) => Some((p.id, t)),
                            Feed::Probes | Feed::Unpolled => None,
                        })
                        .collect();
                    // A live agent now names a different model (a first reply,
                    // a `/model` switch, codex's or pi's next turn) → re-list,
                    // so the agent label follows within a tick rather than on
                    // the 30s safety tick. `None` first: the launch already listed.
                    if lanes.iter().any(|(_, t)| t.models_moved) || last_models.as_ref().is_some_and(|m| *m != models) {
                        let _ = handle.emit("places:changed", ());
                    }
                    last_models = Some(models);
                    let waiting_by: std::collections::BTreeMap<&'static str, Vec<String>> =
                        std::iter::once((worktrees_core::provider::CLAUDE.id, waiting.clone()))
                            .chain(lanes.iter().map(|(id, t)| (*id, t.waiting.clone())))
                            .map(|(id, mut v)| {
                                v.sort_unstable();
                                v.dedup();
                                (id, v)
                            })
                            .collect();
                    let Activity { busy, waiting, edges } =
                        merge_activity(busy, waiting, lanes.iter().map(|(_, t)| t));
                    // Change-gated: emit only when EITHER set shifts, so an idle
                    // machine stays silent (the frontend just re-applies the last set).
                    if busy != last_busy || waiting != last_waiting || waiting_by != last_waiting_by {
                        last_busy = busy.clone();
                        last_waiting = waiting.clone();
                        last_waiting_by = waiting_by.clone();
                        let _ = handle.emit(
                            "sessions:busy",
                            ClaudeActivity { busy: busy.clone(), waiting, waiting_by },
                        );
                    }
                    // Completion edges, computed AFTER the busy emit so the dot's
                    // hand-off (green out, ember in) arrives in that order.
                    let exits = completion_edges(&mut busy_ticks, &edges);
                    if !exits.is_empty() {
                        // read_projects only on a real edge — not every 3s tick
                        let roots = read_projects(&pollctx);
                        let epoch = sysclock::now_epoch();
                        for path in exits {
                            if stamp_worked(&roots, &path, epoch) {
                                let _ = handle.emit("sessions:done", TaskDone { path, epoch });
                            }
                        }
                    }
                    let fresh: Vec<(String, i64)> = lanes
                        .iter()
                        .flat_map(|(id, t)| new_dones(done_seen.entry(id).or_default(), &t.done))
                        .collect();
                    if !fresh.is_empty() {
                        let roots = read_projects(&pollctx);
                        let now = sysclock::now_epoch();
                        for (path, at) in fresh {
                            // Bounded by now: the stamp is the agent's clock.
                            let epoch = at.min(now);
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
            cross_project_status,
            set_cross_project,
            planning_status,
            set_planning_default,
            set_project_planning,
            planning_suggested,
            quota_settings,
            set_quota_settings,
            set_project_private,
            copy_text,
            add_project,
            create_project,
            clone_project,
            clone_cancel,
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
            project_prs,
            fetch_origin,
            set_fetch_interval,
            check_update,
            update_cli,
            list_releases,
            install_app_version,
            get_ai_config,
            mcp_status,
            mcp_install,
            mcp_uninstall,
            codex_mcp_status,
            codex_mcp_install,
            codex_mcp_uninstall,
            codex_mcp_migration_plan,
            pi_mcp_status,
            pi_mcp_install,
            pi_mcp_uninstall,
            codex_mcp_migrate,
            project_config_read,
            doctor,
            place_health,
            ai_status_report,
            relink,
            provision,
            sync_status,
            sync_hub_list,
            sync_preview,
            sync_apply,
            init_suggest,
            init_write,
            agent_setup_status,
            agent_setup_fix,
            rules_apply,
            rules_review,
            rules_lane_remove,
            agent_link_skills,
            agent_user_skills,
            set_codex_permissions,
            agent_guidance_status,
            set_agent_guidance,
            set_agent_guidance_skill,
            agent_guidance_diff,
            agent_guidance_merge,
            agent_models,
            pi_status,
            set_pi_trust,
            set_pi_allowed,
            diagnostics,
            tmux_check,
            set_zoom,
            claude_usage,
            codex_usage,
            claude_status,
            log_info,
            list_drafts,
            log_event,
            log_tail,
            get_changelog,
            open_editor,
            detect_editors,
            open_terminal,
            list_dir,
            changed_files,
            file_diff,
            read_file,
            file_readable,
            resolve_term_paths,
            resolve_doc_paths,
            list_docs,
            place_plan,
            plan_prompt,
            plan_ask_main,
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
            term_wheel,
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
            if matches!(event, tauri::RunEvent::Ready) {
                winstate::ready(handle);
            }
            if matches!(event, tauri::RunEvent::Exit) {
                winstate::stop(handle);
                let c = ctx(handle);
                // Where each tab ended up, recorded BEFORE the sweep — a killed
                // shell has no cwd left to read.
                worktrees_api::term::save_shell_cwds(&c, &c.shells);
                // …and what each tab was SHOWING, likewise before the sweep: a
                // killed shell still has its ring, but it will not have it long.
                worktrees_api::term::save_shell_scrollback(&c, &c.shells);
                let keys: Vec<worktrees_api::term::ShellKey> =
                    c.shells.0.lock().unwrap().keys().cloned().collect();
                for k in &keys {
                    worktrees_api::term::kill_shell(&c.shells, k);
                }
                // And the documentation server, for a harder reason than the
                // shells': what it is holding is a loopback port onto the
                // user's documents. Surviving the window would mean serving
                // them to whatever finds the port, with nothing on screen to
                // say it is still running.
                viewer::kill(&c.viewer);
                // In-flight clones: cancel, and give core a moment to kill
                // git's group and remove the partial folder (the guard leaves
                // the map when that is done). Bounded — exit must not hang.
                let clones = worktrees_api::workspace::CLONE_JOBS.cancel_all();
                if !clones.is_empty() {
                    applog("info", &format!("exit: cancelling {} clone(s): {}", clones.len(), clones.join(", ")));
                    let deadline = std::time::Instant::now() + Duration::from_secs(3);
                    while !worktrees_api::workspace::CLONE_JOBS.is_idle() && std::time::Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(50));
                    }
                    if !worktrees_api::workspace::CLONE_JOBS.is_idle() {
                        applog("warn", "exit: a cancelled clone had not finished cleaning up");
                    }
                }
                if let Ok(d) = handle.path().app_config_dir() {
                    viewer::cleanup(&d);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn clone_errors_serialize_with_a_kind_the_dialog_can_branch_on() {
        let e = worktrees_core::clone::CloneError {
            kind: worktrees_core::clone::CloneErrorKind::NotFound,
            message: "No repository".into(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "not_found");
        assert_eq!(v["message"], "No repository");
    }

    // ── rollback: which releases are offered ────────────────────────────────

    // ── Claude service status ───────────────────────────────────────────────

    // ── dock shell cwd memory ────────────────────────────────────────────────

    // ── dock shell resizes ───────────────────────────────────────────────────

    // ── dock shell history: scrollback + per-tab commands ───────────────────

    // ── "Ask Claude" (ai_status_report) — the two pure pieces ────────────────
    // The spawn itself cannot be tested here: there is no fake `claude` anywhere
    // in this repo (the bats harness fakes tmux and git, never the AI), which is
    // why docs/ai-profiles-manual-checks.md §11 exists. What IS testable is the
    // text we hand the model and the check that decides whether to spawn at all.

    // ── local usage metrics ────────────────────────────────────────────────

}
