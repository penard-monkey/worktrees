// worktrees UI — Tauri backend. Uses worktrees-core as a LIBRARY (in-process; no
// subprocess, no WORKTREES_BIN). Two jobs of its own:
//   1. state    — core computes derived `ls`; core::store owns the declared sidecar;
//                 the app merges them + reconciles lifecycle_effective for the UI.
//   2. PTY host — attaches to a live tmux session (never owns a shell).
// See DESIGN.md / MIGRATION.md.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager, State};
use worktrees_core::ui::CaptureUi;
use worktrees_core::{ops, store, sysclock, Project, Ui};

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

// ── state: core-derived places + declared overlay + reconciled lifecycle ─────

/// One repo's merged snapshot: core's live `ls` + DECLARED store overlay +
/// reconciled `lifecycle_effective` per place.
fn snapshot(repo: &str) -> Result<serde_json::Value, String> {
    let project = Project::discover(Path::new(repo)).map_err(|e| e.msg)?;
    let mut v = serde_json::to_value(project.ls()).map_err(|e| e.to_string())?;
    let store = store::read_lenient(repo);
    let now = sysclock::now_epoch();
    if let Some(places) = v.get_mut("places").and_then(|p| p.as_array_mut()) {
        for place in places.iter_mut() {
            let slug = place.get("slug").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let tmux_up = place.pointer("/tmux_session/up").and_then(|b| b.as_bool()).unwrap_or(false);
            let decl = store.places.get(&slug);
            place["declared"] = decl
                .map(|d| serde_json::to_value(d).unwrap_or(serde_json::Value::Null))
                .unwrap_or(serde_json::Value::Null);
            place["lifecycle_effective"] = serde_json::Value::String(store::reconcile(decl, tmux_up, now));
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

#[tauri::command]
async fn remove_project(app: AppHandle, root: String) -> Result<Workspace, String> {
    let mut roots = read_projects(&app);
    roots.retain(|r| r != &root);
    write_projects(&app, &roots)?;
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

#[tauri::command]
async fn set_lifecycle(repo: String, slug: String, label: String) -> Result<(), String> {
    if !LIFECYCLE_LABELS.contains(&label.as_str()) {
        return Err(format!("invalid lifecycle label: {label}"));
    }
    store::edit(&repo, &slug, |d| d.lifecycle = Some(label))
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
}

fn run_op<F: FnOnce(&Project, &mut CaptureUi) -> i32>(op: &str, repo: &str, f: F) -> Result<CmdResult, String> {
    let project = Project::discover(Path::new(repo)).map_err(|e| {
        applog("error", &format!("{op} repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let mut ui = CaptureUi::default();
    let code = f(&project, &mut ui);
    if code == 0 {
        applog("info", &format!("{op} ok repo={repo}"));
    } else {
        applog("warn", &format!("{op} rc={code} repo={repo}: {}", ui.lines.join(" | ")));
    }
    Ok(CmdResult { ok: code == 0, code, output: ui.lines.join("\n") })
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
    let mut args: Vec<String> = vec![branch];
    if let Some(b) = base.filter(|s| !s.is_empty()) {
        args.push(b);
    }
    if let Some(n) = name.filter(|s| !s.is_empty()) {
        args.push("--name".into());
        args.push(n);
    }
    args.push("--no-attach".into());
    run_op(&format!("new {branch_log}"), &repo, |p, ui| ops::cmd_new(p, ui, &args))
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
            && worktrees_core::project::claude_session_present(&p.place_dir(&slug));
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
            ops::launch(p, ui, &p.main_root, &session, "", &ai_cmd, false);
            0
        } else {
            let mut args = vec![slug, "--no-attach".into()];
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
    let cmd = worktrees_core::config::resolve_ai_cmd(None);
    let word = cmd.split_whitespace().next().unwrap_or("");
    let base = word.rsplit('/').next().unwrap_or(word);
    base == "claude"
}

/// End a place's tmux session — the worktree stays (right-click "Close session").
#[tauri::command]
async fn close_place(repo: String, slug: String) -> Result<CmdResult, String> {
    run_op(&format!("close {slug}"), &repo, move |p, ui| ops::cmd_close(p, ui, &[slug]))
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
    })
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
#[tauri::command]
async fn open_editor(path: String, cmd: String) -> Result<(), String> {
    let mut it = cmd.split_whitespace();
    let Some(prog) = it.next() else {
        return Err("no editor configured (Settings → Editor command)".into());
    };
    let mut child = std::process::Command::new(prog)
        .args(it)
        .arg(&path)
        .spawn()
        .map_err(|e| format!("couldn't launch '{cmd}': {e}"))?;
    // reap in the background — a dropped Child is never waited on (zombie per click)
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Remove a place (`rm <slug> -y` [+ --branch/--force]); the UI confirms first.
#[tauri::command]
async fn remove_place(
    repo: String,
    slug: String,
    del_branch: bool,
    force: bool,
) -> Result<CmdResult, String> {
    let slug_log = slug.clone();
    let mut args: Vec<String> = vec![slug, "-y".into()];
    if del_branch {
        args.push("--branch".into());
    }
    if force {
        args.push("--force".into());
    }
    run_op(&format!("rm {slug_log}"), &repo, |p, ui| ops::cmd_rm(p, ui, &args))
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

/// GUI-launched apps inherit launchd's bare PATH (/usr/bin:/bin:…) — no
/// homebrew, no ~/.local/bin — so the engine's tmux/git shell-outs fail even
/// though they work in every terminal (tmux is homebrew-installed: every place
/// looks dead and Enter errors). Resolve the user's real PATH from their login
/// shell once at startup (marker-wrapped so chatty profiles can't corrupt it;
/// deadline-guarded so a hung profile can't block launch), falling back to
/// appending the usual install dirs.
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
    let path = match from_shell {
        Some(p) => format!("{p}:{current}"), // dups harmless; current kept as safety net
        None => {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{home}/.local/bin:{home}/bin:/opt/homebrew/bin:/usr/local/bin:{current}")
        }
    };
    std::env::set_var("PATH", path);
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
        .setup(|app| {
            // Live-refresh, change-gated. The tmux session set is a cheap
            // fingerprint that shifts whenever a place is opened/closed (even from a
            // bare terminal); emit only when it changes, so the UI's full re-pull (a
            // git sweep) doesn't fire every few seconds. A slow safety re-emit
            // (~30s) still catches dirty/branch drift that leaves no tmux trace.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                // "churning" = tmux saw output in the session this recently.
                // 10s absorbs the 3s poll jitter yet decays fast once a session
                // goes quiet, so the busy dot means "working right now".
                const BUSY_WINDOW_SECS: i64 = 10;
                let mut last = worktrees_core::tmux::session_fingerprint();
                let mut last_busy: Vec<String> = Vec::new();
                let mut ticks: u32 = 0;
                loop {
                    std::thread::sleep(Duration::from_secs(3));
                    let fp = worktrees_core::tmux::session_fingerprint();
                    ticks += 1;
                    if fp != last || ticks >= 10 {
                        last = fp;
                        ticks = 0;
                        let _ = handle.emit("places:changed", ());
                    }
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    let mut busy: Vec<String> = worktrees_core::tmux::session_activity()
                        .into_iter()
                        .filter(|(_, t)| now - t <= BUSY_WINDOW_SECS)
                        .map(|(n, _)| n)
                        .collect();
                    busy.sort_unstable();
                    if busy != last_busy {
                        last_busy = busy.clone();
                        let _ = handle.emit("sessions:busy", busy);
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_places,
            list_workspace,
            add_project,
            remove_project,
            set_lifecycle,
            set_pin,
            set_note,
            touch_place,
            new_place,
            switch_place,
            remove_place,
            open_place,
            close_place,
            github_url,
            check_update,
            update_cli,
            log_info,
            log_event,
            log_tail,
            get_changelog,
            open_editor,
            settings_info,
            get_settings,
            set_settings,
            term_open,
            term_write,
            term_resize,
            term_close
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
