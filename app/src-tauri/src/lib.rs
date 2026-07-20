// worktrees UI — Rust core. Three jobs, no git/tmux/docker logic of its own:
//   1. command runner  — shells out to the `worktrees` CLI and returns its JSON
//   2. PTY host        — attaches to a live tmux session (never owns a shell)
//   3. (later) sole writer of the declared-state store
// See DESIGN.md. This is the P1 spike: prove the embedded tmux terminal.

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

/// `worktrees ls --json` run inside `repo`, returned verbatim as JSON. The UI
/// already knows the schema (DESIGN.md); we stay a thin pass-through here.
#[tauri::command]
fn list_places(repo: String) -> Result<serde_json::Value, String> {
    let out = std::process::Command::new(worktrees_bin())
        .args(["ls", "--json"])
        .current_dir(&repo)
        .output()
        .map_err(|e| format!("failed to run `{}`: {e}", worktrees_bin()))?;
    if !out.status.success() {
        return Err(format!(
            "`worktrees ls --json` exited {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("invalid JSON from worktrees: {e}"))
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
            term_open,
            term_write,
            term_resize,
            term_close
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
