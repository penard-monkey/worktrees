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
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, State};
use worktrees_core::ui::CaptureUi;
use worktrees_core::{ops, store, sysclock, Project};

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
fn list_places(repo: String) -> Result<serde_json::Value, String> {
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
fn list_workspace(app: AppHandle) -> Result<Workspace, String> {
    let projects = read_projects(&app)
        .into_iter()
        .map(|root| match snapshot(&root) {
            Ok(s) => ProjectView { root, ok: true, error: None, snapshot: Some(s) },
            Err(e) => ProjectView { root, ok: false, error: Some(e), snapshot: None },
        })
        .collect();
    Ok(Workspace { projects })
}

/// Add a git repo to the workspace (stored by its canonical main root, so a
/// subdir resolves to the repo and dedupes).
#[tauri::command]
fn add_project(app: AppHandle, dir: String) -> Result<Workspace, String> {
    let project = Project::discover(Path::new(&dir)).map_err(|e| e.msg)?;
    let root = project.main_root.clone();
    let mut roots = read_projects(&app);
    if !roots.contains(&root) {
        roots.push(root);
        write_projects(&app, &roots)?;
    }
    list_workspace(app)
}

#[tauri::command]
fn remove_project(app: AppHandle, root: String) -> Result<Workspace, String> {
    let mut roots = read_projects(&app);
    roots.retain(|r| r != &root);
    write_projects(&app, &roots)?;
    list_workspace(app)
}

const LIFECYCLE_LABELS: [&str; 4] = ["closed", "saved", "archived", "abandoned"];

#[tauri::command]
fn set_lifecycle(repo: String, slug: String, label: String) -> Result<(), String> {
    if !LIFECYCLE_LABELS.contains(&label.as_str()) {
        return Err(format!("invalid lifecycle label: {label}"));
    }
    store::edit(&repo, &slug, |d| d.lifecycle = Some(label))
}

#[tauri::command]
fn set_pin(repo: String, slug: String, on: bool) -> Result<(), String> {
    store::edit(&repo, &slug, |d| d.pinned = Some(on))
}

#[tauri::command]
fn set_note(repo: String, slug: String, note: String) -> Result<(), String> {
    store::edit(&repo, &slug, |d| {
        d.note = if note.trim().is_empty() { None } else { Some(note) }
    })
}

/// Stamp last-opened (drives the `idle` window). Called when a place is opened.
#[tauri::command]
fn touch_place(repo: String, slug: String) -> Result<(), String> {
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

fn run_op<F: FnOnce(&Project, &mut CaptureUi) -> i32>(repo: &str, f: F) -> Result<CmdResult, String> {
    let project = Project::discover(Path::new(repo)).map_err(|e| e.msg)?;
    let mut ui = CaptureUi::default();
    let code = f(&project, &mut ui);
    Ok(CmdResult { ok: code == 0, code, output: ui.lines.join("\n") })
}

/// Create a worktree (`new`). `--no-attach`: the session is created (pane 0 AI,
/// pane 1 shell) but the app embeds it via its own PTY rather than attaching.
#[tauri::command]
fn new_place(
    repo: String,
    branch: String,
    base: Option<String>,
    name: Option<String>,
) -> Result<CmdResult, String> {
    let mut args: Vec<String> = vec![branch];
    if let Some(b) = base.filter(|s| !s.is_empty()) {
        args.push(b);
    }
    if let Some(n) = name.filter(|s| !s.is_empty()) {
        args.push("--name".into());
        args.push(n);
    }
    args.push("--no-attach".into());
    run_op(&repo, |p, ui| ops::cmd_new(p, ui, &args))
}

/// Move a place to another branch (`switch <slug> <branch> [base]`). `-y` skips
/// the inside-a-worktree ambiguity prompt (the UI targets a place explicitly).
#[tauri::command]
fn switch_place(
    repo: String,
    slug: String,
    branch: String,
    base: Option<String>,
) -> Result<CmdResult, String> {
    let mut args: Vec<String> = vec![slug, branch];
    if let Some(b) = base.filter(|s| !s.is_empty()) {
        args.push(b);
    }
    args.push("-y".into());
    run_op(&repo, |p, ui| ops::cmd_switch(p, ui, &args))
}

/// Remove a place (`rm <slug> -y` [+ --branch/--force]); the UI confirms first.
#[tauri::command]
fn remove_place(
    repo: String,
    slug: String,
    del_branch: bool,
    force: bool,
) -> Result<CmdResult, String> {
    let mut args: Vec<String> = vec![slug, "-y".into()];
    if del_branch {
        args.push("--branch".into());
    }
    if force {
        args.push("--force".into());
    }
    run_op(&repo, |p, ui| ops::cmd_rm(p, ui, &args))
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
fn term_open(
    session: String,
    cols: u16,
    rows: u16,
    on_bytes: Channel<InvokeResponseBody>,
    terms: State<'_, Terminals>,
) -> Result<u32, String> {
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;

    // NOTE (DESIGN P1 follow-up): plain attach = correct size while this is the
    // only client; if a second client attaches, tmux clamps to the smallest.
    let mut cmd = CommandBuilder::new("tmux");
    cmd.args(["attach-session", "-t", &session]);
    cmd.env("TERM", "xterm-256color");

    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
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
fn term_write(id: u32, data: Vec<u8>, terms: State<'_, Terminals>) -> Result<(), String> {
    let mut map = terms.0.lock().unwrap();
    let term = map.get_mut(&id).ok_or("no such terminal")?;
    term.writer.write_all(&data).map_err(|e| e.to_string())?;
    term.writer.flush().map_err(|e| e.to_string())
}

#[tauri::command]
fn term_resize(id: u32, cols: u16, rows: u16, terms: State<'_, Terminals>) -> Result<(), String> {
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
fn term_close(id: u32, terms: State<'_, Terminals>) -> Result<(), String> {
    if let Some(mut term) = terms.0.lock().unwrap().remove(&id) {
        term.stop.store(true, Ordering::Relaxed);
        let _ = term.child.kill(); // kills the CLIENT = detach, not the session
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Terminals::default())
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
            term_open,
            term_write,
            term_resize,
            term_close
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
