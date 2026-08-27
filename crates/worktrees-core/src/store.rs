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

/// The repo path every write derives from, canonicalized so the app (and any
/// second window) resolve the SAME path/lock regardless of how the caller
/// spelled it. `store_path` and `exclude_app_state` share it on purpose: two
/// spellings of a symlinked repo must not write the store in one place and
/// `.git/info/exclude` in another.
fn store_base(repo: &str) -> PathBuf {
    fs::canonicalize(repo).unwrap_or_else(|_| PathBuf::from(repo))
}

/// `<repo>/.worktrees.places.json`.
fn store_path(repo: &str) -> PathBuf {
    store_base(repo).join(STORE_FILE)
}

/// The first time this repo gets a store, teach the repo to ignore it.
///
/// `.worktrees.places.json`, `.worktrees/` and `.worktrees-sync/` (the backups a
/// pull leaves behind) are per-MACHINE state — `sync` ferries them between
/// machines out-of-band, and excludes its own directory from the transfer — so a
/// repo that just acquired one should not answer `git status` with an untracked
/// file the tool itself wrote.
/// `.git/info/exclude` rather than `.gitignore` because the choice is this
/// checkout's, not the project's: it is never committed, and it never hides a
/// TRACKED file, so a user who deliberately committed the store is unaffected.
///
/// Best-effort in the strongest sense — every failure is swallowed, and a repo
/// whose `.git` is not a directory (a bats fake fixture, a plain dir) is skipped
/// silently. Pure `std::fs`: this repo counts subprocesses, and a `git` call on
/// the write path would show up in `spawn-count.sh`.
fn exclude_app_state(base: &Path) {
    let info = base.join(".git");
    if !info.is_dir() {
        return;
    }
    let info = info.join("info");
    if fs::create_dir_all(&info).is_err() {
        return;
    }
    let path = info.join("exclude");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let missing: Vec<&str> = ["/.worktrees.places.json", "/.worktrees/", "/.worktrees-sync/"]
        .into_iter()
        .filter(|want| !existing.lines().any(|l| l == *want))
        .collect();
    if missing.is_empty() {
        return;
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("# worktrees — per-machine app state (moved by `worktrees sync`, not git)\n");
    for line in missing {
        out.push_str(line);
        out.push('\n');
    }
    let _ = fs::write(&path, out);
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
    let base = store_base(repo);
    let path = base.join(STORE_FILE);
    let _flock = DirLock::acquire(&path)?;
    let mut store = read_strict(&path)?;
    let entry = store.places.entry(slug.to_string()).or_default();
    f(entry);
    store.version = 1;
    store.updated_epoch = Some(now_epoch());
    // Asked under the locks, BEFORE the write that would make it false: this is
    // the one moment we know the store is new to this repo.
    let creating = !path.exists();
    write_atomic(&path, &store)?;
    if creating {
        exclude_app_state(&base);
    }
    Ok(())
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

    /// The moment a repo acquires a store is the only moment we get to say
    /// "don't report this file". It has to happen exactly once — a second
    /// `edit()` writing the lines again would grow the file forever — and it
    /// must not be the app's `.gitignore`: this is one checkout's opinion.
    #[test]
    fn a_repos_first_store_write_excludes_the_app_state() {
        let t = tmp("exclude");
        let repo = t.0.to_string_lossy().to_string();
        fs::create_dir_all(t.0.join(".git/info")).unwrap();
        let exclude = t.0.join(".git/info/exclude");

        edit(&repo, "alpha", |d| d.pinned = Some(true)).unwrap();
        let after_first = fs::read_to_string(&exclude).unwrap();
        assert!(
            after_first.lines().any(|l| l == "/.worktrees.places.json"),
            "the store itself must be excluded: {after_first}"
        );
        assert!(
            after_first.lines().any(|l| l == "/.worktrees/"),
            "the worktrees dir must be excluded: {after_first}"
        );
        // sync's own backup dir: the tool writes it into the tree on every pull,
        // and it showed as `??` forever.
        assert!(
            after_first.lines().any(|l| l == "/.worktrees-sync/"),
            "sync's backup dir must be excluded: {after_first}"
        );
        assert!(
            !after_first.contains("/task_plan.md"),
            "planning docs are a personal workflow, not app state: {after_first}"
        );

        edit(&repo, "beta", |d| d.pinned = Some(true)).unwrap();
        assert_eq!(
            fs::read_to_string(&exclude).unwrap(),
            after_first,
            "later writes must not touch the file again",
        );
    }

    /// `.git/info/exclude` is a file git ships with commentary in it, and a user
    /// may have added their own lines. Appending is the only safe verb.
    #[test]
    fn an_existing_exclude_file_is_appended_to_not_replaced() {
        let t = tmp("exclude-keep");
        let repo = t.0.to_string_lossy().to_string();
        fs::create_dir_all(t.0.join(".git/info")).unwrap();
        let exclude = t.0.join(".git/info/exclude");
        fs::write(&exclude, "# git ls-files --others --exclude-from=…\n*.swp\n").unwrap();

        edit(&repo, "alpha", |d| d.pinned = Some(true)).unwrap();

        let body = fs::read_to_string(&exclude).unwrap();
        assert!(body.starts_with("# git ls-files"), "existing content stays first: {body}");
        assert!(body.lines().any(|l| l == "*.swp"), "the user's own rule survives: {body}");
        assert!(body.lines().any(|l| l == "/.worktrees.places.json"), "{body}");
        assert!(body.lines().any(|l| l == "/.worktrees/"), "{body}");
        assert!(body.lines().any(|l| l == "/.worktrees-sync/"), "{body}");
    }

    /// The append is line-exact and idempotent: a file that already names some of
    /// the entries gains only the ones it lacks, and never a duplicate. This is
    /// the shape a repo whose store predates a new entry will meet.
    #[test]
    fn an_exclude_file_that_has_some_entries_gains_only_the_missing_ones() {
        let t = tmp("exclude-partial");
        let repo = t.0.to_string_lossy().to_string();
        fs::create_dir_all(t.0.join(".git/info")).unwrap();
        let exclude = t.0.join(".git/info/exclude");
        fs::write(&exclude, "/.worktrees.places.json\n/.worktrees/\n").unwrap();

        edit(&repo, "alpha", |d| d.pinned = Some(true)).unwrap();

        let body = fs::read_to_string(&exclude).unwrap();
        let count = |want: &str| body.lines().filter(|l| *l == want).count();
        assert_eq!(count("/.worktrees.places.json"), 1, "no duplicate: {body}");
        assert_eq!(count("/.worktrees/"), 1, "no duplicate: {body}");
        assert_eq!(count("/.worktrees-sync/"), 1, "the one it lacked, once: {body}");
    }

    /// The store is written for things that are not repos at all — bats' fake
    /// fixtures, a directory someone points the CLI at. Excluding is a courtesy;
    /// failing to, or conjuring a `.git` to do it in, is not.
    #[test]
    fn a_dir_that_is_not_a_repo_gets_no_exclude_file() {
        let t = tmp("exclude-norepo");
        let repo = t.0.to_string_lossy().to_string();

        edit(&repo, "alpha", |d| d.pinned = Some(true)).expect("the store write still succeeds");

        assert_eq!(read_lenient(&repo).places["alpha"].pinned, Some(true));
        assert!(!t.0.join(".git").exists(), "no .git may be created to hold an exclude file");
    }
}
