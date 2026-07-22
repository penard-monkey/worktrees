//! The per-repo DECLARED-state sidecar (`.worktrees.places.json`) + lifecycle
//! reconciliation. Core owns read + write; the CLI's `ls --json` stays live-only
//! (it never touches this), and the app overlays declared state + reconciles.
//! Moved here from the app in Increment 4 so both consumers share one store.

use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::sysclock::now_epoch;

pub const IDLE_WINDOW_SECS: i64 = 7 * 24 * 3600;
const STORE_FILE: &str = ".worktrees.places.json";

// Serialize all in-process writes (Tauri multi-window = same process).
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// One place's DECLARED facts. Unknown keys round-trip via `extra` so a hand-edit
/// or a newer app version isn't clobbered.
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

/// `<repo>/.worktrees.places.json`, with `repo` canonicalized so the app (and any
/// second window) resolve the SAME path/lock regardless of how the caller spelled it.
fn store_path(repo: &str) -> PathBuf {
    let base = fs::canonicalize(repo).unwrap_or_else(|_| PathBuf::from(repo));
    base.join(STORE_FILE)
}

/// Reconcile DECLARED (sticky) state with LIVE state → the effective label.
/// `active`/`idle` are never persisted; they're derived here.
pub fn reconcile(d: Option<&Declared>, tmux_up: bool, now: i64) -> String {
    match d.and_then(|d| d.lifecycle.as_deref()) {
        Some("archived") => return "archived".into(),
        Some("abandoned") => return "abandoned".into(),
        Some("saved") => return "saved".into(), // sticky; UI still shows the live dot
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

/// For writes: a parse error must NOT be clobbered (a hand-edit typo has to stay
/// human-repairable), so surface it instead of overwriting.
fn read_strict(path: &Path) -> Result<Store, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{STORE_FILE} is not valid JSON ({e}) — not overwriting")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(e.to_string()),
    }
}

// Best-effort cross-process lock via atomic mkdir. In-process writers are
// serialized by WRITE_LOCK; cross-process contention is rare and short.
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

/// Read-under-lock → field-merge one place → atomic write. Preserves unknown keys
/// (per place and top-level) and every other place untouched.
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
