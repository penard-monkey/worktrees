// worktrees UI — Tauri backend. Uses worktrees-core as a LIBRARY (in-process; no
// subprocess, no WORKTREES_BIN). Two jobs of its own:
//   1. state    — core computes derived `ls`; core::store owns the declared sidecar;
//                 the app merges them + reconciles lifecycle_effective for the UI.
//   2. PTY host — attaches to a live tmux session for the place's canonical
//                 shell, and OWNS the dock's scratch shells outright (no tmux).
// See DESIGN.md / MIGRATION.md.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
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
            // Propagate launch's rc: a failed new-session must reach the UI
            // banner / app.log, not silently report success. Single-pane
            // (spare_shell=false): Claude gets full width; the scratch shell
            // lives in the dock's Terminal tab.
            ops::launch(p, ui, &p.main_root, &session, "", &ai_cmd, false, false)
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
    let cmd = worktrees_core::config::resolve_ai_cmd(None);
    let word = cmd.split_whitespace().next().unwrap_or("");
    let base = word.rsplit('/').next().unwrap_or(word);
    base == "claude"
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
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return (busy, waiting);
    }
    let dir = Path::new(&home).join(".claude/sessions");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return (busy, waiting), // dir missing/unreadable → no dots
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
    (busy, waiting)
}

/// Payload for `sessions:busy` — PATHS (worktree dirs), keyed to a place's `path`.
#[derive(Serialize, Clone)]
struct ClaudeActivity {
    busy: Vec<String>,
    waiting: Vec<String>,
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
}

#[derive(Serialize)]
struct FileContent {
    content: String,
    truncated: bool,
    binary: bool,
    /// mtime (ms since epoch) — the dock echoes it back on save as a
    /// compare-and-swap token so a stale buffer can't clobber a newer edit.
    mtime: u64,
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
    for r in read_projects(app) {
        if let Ok(rc) = std::fs::canonicalize(&r) {
            if canon == rc || canon.starts_with(&rc) {
                return Ok(canon);
            }
        }
    }
    Err(format!("path outside workspace: {path}"))
}

/// Immediate children of `path` (one level; the tree lazy-expands). Dirs first,
/// then case-insensitive by name. `.git` and gitignored entries are dropped.
#[tauri::command]
async fn list_dir(app: AppHandle, path: String) -> Result<Vec<FsEntry>, String> {
    let dir = guard_under_projects(&app, &path)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let mut entries: Vec<FsEntry> = Vec::new();
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(FsEntry { name, path: e.path().to_string_lossy().to_string(), is_dir });
    }
    // Drop gitignored entries in one `git check-ignore --stdin` batch (NUL-safe).
    let ignored = git_check_ignore(&dir, entries.iter().map(|e| e.path.as_str()));
    entries.retain(|e| !ignored.contains(&e.path));
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

/// File contents for the viewer. Capped (default 1 MiB) and binary-guarded
/// (a NUL byte in the read slice → `binary: true`, empty content). The frontend
/// shows a "binary / open in editor" placeholder instead of garbage.
#[tauri::command]
async fn read_file(app: AppHandle, path: String, max_bytes: Option<u64>) -> Result<FileContent, String> {
    let f = guard_under_projects(&app, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    let cap = max_bytes.unwrap_or(1_000_000);
    // Bounded read: `take(cap+1)` never allocates more than the cap even for a
    // multi-GB file the user clicks by accident (video, core dump, tarball).
    let file = std::fs::File::open(&f).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > cap;
    let slice = &bytes[..bytes.len().min(cap as usize)];
    let mtime = file_mtime_ms(&f);
    if slice.contains(&0) {
        return Ok(FileContent { content: String::new(), truncated, binary: true, mtime });
    }
    Ok(FileContent { content: String::from_utf8_lossy(slice).to_string(), truncated, binary: false, mtime })
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

/// The 1-based indices of a place's live dock shells. The dock restores its
/// Terminal tabs from this; shells don't outlive the app (see `Shells`), so
/// after a restart it's empty and the dock opens a fresh one.
#[tauri::command]
async fn list_shell_sessions(repo: String, slug: String, shells: State<'_, Shells>) -> Result<Vec<u32>, String> {
    let map = shells.0.lock().unwrap();
    let mut ids: Vec<u32> = map
        .keys()
        .filter(|(r, s, _)| r == &repo && s == &slug)
        .map(|(_, _, i)| *i)
        .collect();
    ids.sort_unstable();
    Ok(ids)
}

/// End one shell tab — kills the process, unlike `shell_detach`.
#[tauri::command]
async fn close_shell_session(repo: String, slug: String, index: u32, shells: State<'_, Shells>) -> Result<(), String> {
    kill_shell(&shells, &(repo, slug, index));
    Ok(())
}

/// Remove a place (`rm <slug> -y` [+ --branch/--force]); the UI confirms first.
#[tauri::command]
async fn remove_place(
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
/// `on_bytes`. Idempotent: a second call for a live shell just swaps the sink
/// and replays — which is exactly what a tab flip or a dock re-open does.
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
) -> Result<(), String> {
    let key: ShellKey = (repo.clone(), slug.clone(), index);
    {
        let map = shells.0.lock().unwrap();
        if let Some(sh) = map.get(&key) {
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
            return Ok(());
        }
    }

    let (session, cwd) = place_session_cwd(&repo, &slug)?;
    // One-time cleanup for anyone upgrading: their `<session>~term*` sidecars
    // are orphans now — nothing will ever attach them again. Cheap and
    // idempotent, and only reached when this place has no shell yet.
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
    cmd.cwd(&cwd);
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

    shells
        .0
        .lock()
        .unwrap()
        .insert(key, Shell { master: pair.master, writer, child, stop, ring, sink });
    Ok(())
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
/// flip or ⌘J must not kill what you left building.
#[tauri::command]
async fn shell_detach(repo: String, slug: String, index: u32, shells: State<'_, Shells>) -> Result<(), String> {
    let map = shells.0.lock().unwrap();
    if let Some(sh) = map.get(&(repo, slug, index)) {
        *sh.sink.lock().unwrap() = None;
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
                    // Change-gated: emit only when EITHER set shifts, so an idle
                    // machine stays silent (the frontend just re-applies the last set).
                    if busy != last_busy || waiting != last_waiting {
                        last_busy = busy.clone();
                        last_waiting = waiting.clone();
                        let _ = handle.emit("sessions:busy", ClaudeActivity { busy, waiting });
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
            log_info,
            log_event,
            log_tail,
            get_changelog,
            open_editor,
            open_terminal,
            list_dir,
            read_file,
            write_file,
            list_shell_sessions,
            close_shell_session,
            shell_open,
            shell_write,
            shell_resize,
            shell_detach,
            settings_info,
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
                let keys: Vec<ShellKey> = shells.0.lock().unwrap().keys().cloned().collect();
                for k in &keys {
                    kill_shell(&shells, k);
                }
            }
        });
}
