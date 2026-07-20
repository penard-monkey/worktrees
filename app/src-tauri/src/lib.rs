// worktrees UI — Rust core. Jobs, with NO git/tmux/docker logic of its own:
//   1. command runner — shells out to the `worktrees` CLI and returns its JSON
//   2. PTY host        — attaches to a live tmux session (never owns a shell)
//   3. declared store  — sole reader+writer of the per-repo lifecycle sidecar,
//                        merged into ls --json + reconciled into lifecycle_effective
// See DESIGN.md.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

// ── command runner: shell out to the `worktrees` CLI ────────────────────────

/// The CLI to drive. `WORKTREES_BIN` lets the dev run against the repo copy
/// (e.g. WORKTREES_BIN=$PWD/bin/worktrees) without installing it.
fn worktrees_bin() -> String {
    std::env::var("WORKTREES_BIN").unwrap_or_else(|_| "worktrees".to_string())
}

fn cli_ls_json(repo: &str) -> Result<serde_json::Value, String> {
    let out = std::process::Command::new(worktrees_bin())
        .args(["ls", "--json"])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("failed to run `{}`: {e}", worktrees_bin()))?;
    if !out.status.success() {
        return Err(format!(
            "`worktrees ls --json` exited {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("invalid JSON from worktrees: {e}"))
}

/// Derived state from the CLI, with the DECLARED store merged in and
/// `lifecycle_effective` reconciled. This is the one payload the UI renders.
#[tauri::command]
fn list_places(repo: String) -> Result<serde_json::Value, String> {
    let mut v = cli_ls_json(&repo)?;
    let store = store::read_lenient(&repo);
    let now = store::now_epoch();
    if let Some(places) = v.get_mut("places").and_then(|p| p.as_array_mut()) {
        for place in places.iter_mut() {
            let slug = place
                .get("slug")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let tmux_up = place
                .pointer("/tmux_session/up")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let decl = store.places.get(&slug);
            place["declared"] = decl
                .map(|d| serde_json::to_value(d).unwrap_or(serde_json::Value::Null))
                .unwrap_or(serde_json::Value::Null);
            place["lifecycle_effective"] =
                serde_json::Value::String(store::reconcile(decl, tmux_up, now));
        }
    }
    Ok(v)
}

// ── declared store: the per-repo lifecycle sidecar (Rust owns read + write) ──
mod store {
    use serde::{Deserialize, Serialize};
    use serde_json::Map;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub const IDLE_WINDOW_SECS: i64 = 7 * 24 * 3600;
    const STORE_FILE: &str = ".worktrees.places.json";

    // Serialize all in-process writes (Tauri multi-window = same process).
    static WRITE_LOCK: Mutex<()> = Mutex::new(());

    /// One place's DECLARED facts. Unknown keys round-trip via `extra` so a
    /// hand-edit or a newer app version isn't clobbered.
    #[derive(Serialize, Deserialize, Clone, Default)]
    pub struct Declared {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub lifecycle: Option<String>, // closed | saved | archived | abandoned
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pinned: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub note: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub last_opened_epoch: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub up_cmd: Option<String>,
        #[serde(flatten)]
        pub extra: Map<String, serde_json::Value>,
    }

    #[derive(Serialize, Deserialize, Default)]
    pub struct Store {
        #[serde(default)]
        pub version: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub updated_epoch: Option<i64>,
        #[serde(default)]
        pub places: BTreeMap<String, Declared>,
        #[serde(flatten)]
        pub extra: Map<String, serde_json::Value>,
    }

    pub fn now_epoch() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn store_path(repo: &str) -> PathBuf {
        Path::new(repo).join(STORE_FILE)
    }

    /// Reconcile DECLARED (sticky) state with LIVE state → the effective label.
    /// `active`/`idle` are never persisted; they're derived here.
    pub fn reconcile(d: Option<&Declared>, tmux_up: bool, now: i64) -> String {
        match d.and_then(|d| d.lifecycle.as_deref()) {
            Some("archived") => return "archived".into(),
            Some("abandoned") => return "abandoned".into(),
            Some("saved") => return "saved".into(), // sticky; UI still shows live dot
            _ => {}
        }
        if tmux_up {
            return "active".into();
        }
        if let Some(t) = d.and_then(|d| d.last_opened_epoch) {
            if now - t < IDLE_WINDOW_SECS {
                return "idle".into();
            }
        }
        "closed".into()
    }

    /// For display: missing file or parse error → empty store (never fatal).
    pub fn read_lenient(repo: &str) -> Store {
        match fs::read(store_path(repo)) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Store::default(),
        }
    }

    /// For writes: a parse error must NOT be clobbered (a hand-edit typo has to
    /// stay human-repairable), so surface it instead of overwriting.
    fn read_strict(path: &Path) -> Result<Store, String> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("{STORE_FILE} is not valid JSON ({e}) — not overwriting")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    // Best-effort cross-process lock via atomic mkdir. (PID-liveness stale-break
    // is a follow-up; in practice the only in-process writers are serialized by
    // WRITE_LOCK, and cross-process contention is rare and short.)
    struct DirLock(PathBuf);
    impl DirLock {
        fn acquire(target: &Path) -> Result<Self, String> {
            let lock = target.with_extension("json.lock");
            for _ in 0..100 {
                match fs::create_dir(&lock) {
                    Ok(_) => return Ok(DirLock(lock)),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let stale = fs::metadata(&lock)
                            .and_then(|m| m.modified())
                            .map(|t| t.elapsed().map(|e| e.as_secs() > 15).unwrap_or(false))
                            .unwrap_or(false);
                        if stale {
                            let _ = fs::remove_dir_all(&lock);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(15));
                    }
                    Err(e) => return Err(format!("lock error: {e}")),
                }
            }
            Err("could not acquire places-file lock".into())
        }
    }
    impl Drop for DirLock {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_atomic(path: &Path, store: &Store) -> Result<(), String> {
        let dir = path.parent().ok_or("no parent dir for store")?;
        let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
        // temp in the SAME dir so rename(2) is atomic (same filesystem)
        let tmp = dir.join(".worktrees.places.json.tmp");
        fs::write(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    /// Read-under-lock → field-merge one place → atomic write. Preserves unknown
    /// keys (per place and top-level) and every other place untouched.
    pub fn edit<F: FnOnce(&mut Declared)>(repo: &str, slug: &str, f: F) -> Result<(), String> {
        if slug.is_empty() {
            return Err("empty slug".into());
        }
        let _serial = WRITE_LOCK.lock().map_err(|_| "store lock poisoned")?;
        let path = store_path(repo);
        let _flock = DirLock::acquire(&path)?;
        let mut store = read_strict(&path)?;
        let entry = store.places.entry(slug.to_string()).or_default();
        f(entry);
        store.version = 1;
        store.updated_epoch = Some(now_epoch());
        write_atomic(&path, &store)
    }
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
    store::edit(&repo, &slug, |d| d.last_opened_epoch = Some(store::now_epoch()))
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
    // A grouped session (`new-session -t`) or `-f ignore-size` (tmux >= 3.2)
    // removes the clamp — deferred until multi-client is a real case.
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
                    if on_bytes
                        .send(InvokeResponseBody::Raw(buf[..n].to_vec()))
                        .is_err()
                    {
                        break; // frontend gone
                    }
                }
                Err(_) => break,
            }
        }
    });

    terms
        .0
        .lock()
        .unwrap()
        .insert(id, Term { master: pair.master, writer, child, stop });
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
        .manage(Terminals::default())
        .invoke_handler(tauri::generate_handler![
            list_places,
            set_lifecycle,
            set_pin,
            set_note,
            touch_place,
            term_open,
            term_write,
            term_resize,
            term_close
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
