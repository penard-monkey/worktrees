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
    /// A human-readable name for this place, shown instead of the slug.
    ///
    /// The slug is NOT a name the tool stores — it is `basename(worktree_dir)`,
    /// re-derived from disk on every read. Renaming the *place* would therefore
    /// mean renaming the directory, which is a rename of six things at once:
    /// the git worktree registration, this store's key, the tmux session
    /// (`{prefix}-{slug}`) and every `~term` sidecar hanging off it, the
    /// recorded `COMPOSE_PROJECT_NAME`, and — worst — the Claude history
    /// directory, which is keyed on the ABSOLUTE worktree path, so a rename
    /// silently orphans the conversation and breaks auto-resume.
    ///
    /// So this is a LABEL, and identity stays where it is. AI profiles already
    /// draw the same line (`profile.rs`: an immutable `id`, a freely renameable
    /// `name`); places simply never had it. The UI keeps the slug visible
    /// alongside a title precisely because the session, the directory and
    /// "Copy path" are all still slug-derived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_epoch: Option<i64>,
    /// When Claude last FINISHED a task here. Deliberately NOT the same fact as
    /// `last_opened_epoch`: opening (or resuming) a session leaves its probe at
    /// `status: "idle"` and never stamps this — only a session that actually
    /// went `busy` and came back out of it does. That distinction is the whole
    /// feature: the nav's decaying "afterglow" dot must mean *work happened*,
    /// not *you looked in here*.
    ///
    /// Monotonic: every writer takes `max` with what is already stored, so a
    /// startup backfill from an older log can never walk a live observation back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_worked_epoch: Option<i64>,
    /// The AI profile this place's session was last STARTED with, and that
    /// profile's `updated_epoch` at the time.
    ///
    /// Recorded so the UI can say "this session is running an older version of
    /// the profile — restart to pick up your edits". Without a stamp there is
    /// nothing to compare a profile's current `updated_epoch` against, and the
    /// badge would either never appear or always appear.
    ///
    /// NOT written when a launch merely attaches to a session that was already
    /// up: that session is still running whatever it started with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_epoch: Option<i64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let t = std::env::temp_dir().join(format!(
            "wtstore-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        Tmp(t)
    }

    /// The afterglow stamp's migration story, which is the whole reason it could
    /// be added without a version bump: a store written by an OLDER binary has
    /// no `last_worked_epoch` (reads as None), and a store carrying keys this
    /// binary has never heard of must survive a write untouched — otherwise a
    /// newer app's fields would be silently deleted by an older one.
    #[test]
    fn worked_stamp_round_trips_beside_unknown_keys() {
        let t = tmp("worked");
        let repo = t.0.to_string_lossy().to_string();
        fs::write(
            t.0.join(STORE_FILE),
            r#"{"version":1,"places":{"alpha":{"pinned":true,"from_the_future":42}}}"#,
        )
        .unwrap();

        assert_eq!(read_lenient(&repo).places["alpha"].last_worked_epoch, None);

        edit(&repo, "alpha", |d| d.last_worked_epoch = Some(1_700_000_000)).unwrap();

        let back = read_lenient(&repo);
        let alpha = &back.places["alpha"];
        assert_eq!(alpha.last_worked_epoch, Some(1_700_000_000));
        assert_eq!(alpha.pinned, Some(true));
        assert_eq!(alpha.extra.get("from_the_future").and_then(|v| v.as_i64()), Some(42));
    }

    /// `title` rides the same no-version-bump story as the afterglow stamp: an
    /// older binary's store has no `title` (reads as `None`), and adding one
    /// must leave every other field — including keys this binary has never
    /// heard of — exactly as it found them. Writing a title must also not
    /// disturb `note`, which is the field it sits next to and is most likely to
    /// be confused with.
    #[test]
    fn title_round_trips_beside_note_and_unknown_keys() {
        let t = tmp("title");
        let repo = t.0.to_string_lossy().to_string();
        fs::write(
            t.0.join(STORE_FILE),
            r#"{"version":1,"places":{"alpha":{"note":"auth refactor","pinned":true,"from_the_future":42}}}"#,
        )
        .unwrap();

        assert_eq!(read_lenient(&repo).places["alpha"].title, None);

        edit(&repo, "alpha", |d| d.title = Some("Auth refactor".into())).unwrap();

        let back = read_lenient(&repo);
        let alpha = &back.places["alpha"];
        assert_eq!(alpha.title.as_deref(), Some("Auth refactor"));
        assert_eq!(alpha.note.as_deref(), Some("auth refactor"));
        assert_eq!(alpha.pinned, Some(true));
        assert_eq!(alpha.extra.get("from_the_future").and_then(|v| v.as_i64()), Some(42));

        // clearing is how the UI removes a title (empty string -> None), and it
        // must not leave a `"title": null` behind for an older binary to read
        edit(&repo, "alpha", |d| d.title = None).unwrap();
        let raw = fs::read_to_string(t.0.join(STORE_FILE)).unwrap();
        assert!(!raw.contains("title"), "cleared title must not be serialized: {raw}");
    }

    /// `last_worked_epoch` must NOT be confused with `last_opened_epoch`: only
    /// the latter feeds lifecycle reconciliation, so work alone can never make a
    /// place that was never opened read as `idle`.
    #[test]
    fn worked_stamp_does_not_move_lifecycle() {
        let now = 1_700_000_000;
        let d = Declared { last_worked_epoch: Some(now - 60), ..Default::default() };
        assert_eq!(reconcile(Some(&d), false, now), "closed");
    }
}
