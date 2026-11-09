//! The ideas rows (`worktrees_core::ideas`, idea board phase 1) and the
//! change event the poll sends for them.
//!
//! Each row is ONE operation with plain parameters, addressed by the idea's
//! `id` — never by a path from the client. The folder is the engine's
//! (`Ctx::ideas_dir`), so a test moves it with `Ctx::with_ideas_dir` and a
//! spawned engine with `WORKTREES_IDEAS_DIR`, never with `XDG_DATA_HOME`.
//! Desktop only: serve declares every row console-only.
//!
//! **`ideas:changed`** is sent by the 3 s poll when the folder's stamp moves
//! (one `readdir` and one `lstat` per entry a tick; the folder is read only
//! on a change). Its payload is `{changed: [{id, mtime_ms}], removed: [id]}`.
//! It fires for the app's OWN saves too, a tick later, and a client tells
//! the two apart by mtime: `idea_write`, `idea_rename` and `idea_adopt`
//! answer with the file's mtime after the act, and a `changed` entry for the
//! open idea whose `mtime_ms` equals the last one the client was given is
//! its own write — anything else is an edit made elsewhere. A client
//! re-pulls `ideas_list` on every event, whatever it carries: a hand-made
//! file has no id to name and is in neither list.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use worktrees_core::ideas::{self, Doc, Index};

use crate::Ctx;

fn root(ctx: &Ctx) -> &Path {
    ctx.ideas_dir()
}

/// `ideas_list`'s answer: the index, and the folder it was read from — the
/// one path this module hands out, for the window's Show in Finder (a reveal
/// on the HOST, through `onhost.ts`; every row still takes an id, never it).
#[derive(Serialize)]
pub struct Listing {
    #[serde(flatten)]
    pub index: Index,
    pub dir: String,
}

pub fn ideas_list(ctx: &Ctx) -> Result<Listing, String> {
    Ok(Listing { index: ideas::list_at(root(ctx))?, dir: root(ctx).to_string_lossy().into_owned() })
}

pub fn idea_read(ctx: &Ctx, id: String) -> Result<Doc, String> {
    ideas::read_at(root(ctx), &id)
}

/// A new blank idea; no title is `Untitled`.
pub fn idea_create(ctx: &Ctx, title: Option<String>) -> Result<Doc, String> {
    ideas::create_at(root(ctx), title.as_deref().unwrap_or(""))
}

/// Save `content` byte for byte; the saved mtime comes back. `keep_history`
/// copies the file's current bytes to `.history/` first (a person keeping
/// their text over an edit made elsewhere); an ordinary save never sets it.
pub fn idea_write(ctx: &Ctx, id: String, content: String, expected_mtime: Option<u64>, keep_history: Option<bool>) -> Result<u64, String> {
    if keep_history == Some(true) {
        ideas::write_kept_at(root(ctx), &id, &content, expected_mtime)
    } else {
        ideas::write_at(root(ctx), &id, &content, expected_mtime)
    }
}

pub fn idea_rename(ctx: &Ctx, id: String, title: String, expected_mtime: Option<u64>) -> Result<Doc, String> {
    ideas::rename_at(root(ctx), &id, &title, expected_mtime)
}

pub fn idea_remove(ctx: &Ctx, id: String, expected_mtime: Option<u64>) -> Result<(), String> {
    ideas::remove_at(root(ctx), &id, expected_mtime)
}

/// Give a hand-made document (`problem: "no-id"` in the list) an id. The one
/// row that names a document by its FILE, because it has no id yet: a bare
/// name in the folder, refused with any `/` or a leading dot.
pub fn idea_adopt(ctx: &Ctx, file: String, expected_mtime: Option<u64>) -> Result<Doc, String> {
    ideas::adopt_at(root(ctx), &file, expected_mtime)
}

// ── the poll ────────────────────────────────────────────────────────────────

/// What the poll last saw of the folder.
#[derive(Default, Clone)]
pub struct IdeasWatch {
    stamp: Option<Vec<(String, bool, u64, u64)>>,
    /// id → (mtime ms, size): the size too, because two writes inside one
    /// millisecond (a create and its first save, over one socket) leave the
    /// mtime where it was.
    seen: HashMap<String, (u64, u64)>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Changed {
    pub id: String,
    pub mtime_ms: u64,
}

#[derive(Serialize, Debug, PartialEq, Eq, Default)]
pub struct IdeasChanged {
    pub changed: Vec<Changed>,
    pub removed: Vec<String>,
}

fn seen(idx: &Index) -> HashMap<String, (u64, u64)> {
    idx.ideas.iter().filter(|d| d.problem.is_none()).filter_map(|d| Some((d.id.clone()?, (d.mtime_ms, d.size)))).collect()
}

/// One tick: `Some` with the event's payload when the folder changed since
/// the last tick. The first tick only takes the baseline.
pub fn tick_at(root: &Path, w: &mut IdeasWatch) -> Option<IdeasChanged> {
    tick_with(root, w, &ideas::scan_at)
}

/// [`tick_at`] with the scan given: the seam a test scripts a failed scan
/// through. A scan that fails sends NOTHING and keeps the last stamp and the
/// last ids, so the next tick tries again: read as an empty folder, it
/// would list every idea in `removed`, and a client that trusts the event
/// closes the open editor.
fn tick_with(root: &Path, w: &mut IdeasWatch, scan: &dyn Fn(&Path) -> std::io::Result<Index>) -> Option<IdeasChanged> {
    let stamp = ideas::stamp_at(root);
    if w.stamp.as_ref() == Some(&stamp) {
        return None;
    }
    let now = match scan(root) {
        Ok(i) => seen(&i),
        Err(e) => {
            crate::log::warn(&format!("ideas: could not read {}: {e}", root.display()));
            return None;
        }
    };
    let first = w.stamp.is_none();
    w.stamp = Some(stamp);
    let before = std::mem::replace(&mut w.seen, now);
    if first {
        return None;
    }
    let mut ev = IdeasChanged::default();
    for (id, m) in &w.seen {
        if before.get(id) != Some(m) {
            ev.changed.push(Changed { id: id.clone(), mtime_ms: m.0 });
        }
    }
    ev.removed = before.keys().filter(|id| !w.seen.contains_key(*id)).cloned().collect();
    ev.changed.sort_by(|a, b| a.id.cmp(&b.id));
    ev.removed.sort();
    Some(ev)
}

/// The poll's beat: send `ideas:changed` when the folder moved.
pub fn tick(ctx: &Ctx, w: &mut IdeasWatch) {
    if let Some(ev) = tick_at(root(ctx), w) {
        ctx.emit("ideas:changed", serde_json::to_value(ev).unwrap_or(serde_json::Value::Null));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wt-api-ideas-{tag}-{}-{:x}", std::process::id(), worktrees_core::fsx::random_u64()));
        fs::create_dir_all(&d).unwrap();
        d.join("ideas")
    }

    /// The event says which idea moved and to what mtime, so a client whose
    /// own save returned that mtime knows the event is its own; an edit made
    /// elsewhere carries an mtime it was never given.
    #[test]
    fn the_change_event_names_the_idea_and_the_mtime_a_client_compares_with_its_own_save() {
        let root = tmp("tick");
        let mut w = IdeasWatch::default();
        assert_eq!(tick_at(&root, &mut w), None, "the baseline, a missing folder");
        assert!(!root.exists(), "the poll creates nothing");
        let d = ideas::create_at(&root, "One").unwrap();
        let id = d.idea.id.clone().unwrap();
        let ev = tick_at(&root, &mut w).unwrap();
        assert_eq!(ev.changed, vec![Changed { id: id.clone(), mtime_ms: d.idea.mtime_ms }]);
        assert_eq!(tick_at(&root, &mut w), None, "nothing moved");

        // The app's own save: the event's mtime is the one the save returned.
        std::thread::sleep(std::time::Duration::from_millis(15));
        let saved = ideas::write_at(&root, &id, &format!("{}\nmore\n", d.content), None).unwrap();
        assert_eq!(tick_at(&root, &mut w).unwrap().changed, vec![Changed { id: id.clone(), mtime_ms: saved }]);
        // A save with no change sends nothing.
        ideas::write_at(&root, &id, &format!("{}\nmore\n", d.content), Some(saved)).unwrap();
        assert_eq!(tick_at(&root, &mut w), None);

        // An edit elsewhere: an mtime no row ever answered with.
        std::thread::sleep(std::time::Duration::from_millis(15));
        fs::write(root.join(&d.idea.file), format!("{}\nfrom vim\n", d.content)).unwrap();
        let ev = tick_at(&root, &mut w).unwrap();
        assert_eq!(ev.changed.len(), 1);
        assert_ne!(ev.changed[0].mtime_ms, saved);

        // A write inside the same millisecond (the mtime put back) is still
        // a change: its size moved.
        let p = root.join(&d.idea.file);
        let t = fs::metadata(&p).unwrap().modified().unwrap();
        fs::write(&p, format!("{}\nfrom vim, longer\n", d.content)).unwrap();
        fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        assert_eq!(tick_at(&root, &mut w).unwrap().changed.len(), 1, "same mtime, new size");

        // A hand-made file moves the folder but names no id.
        fs::write(root.join("hand.md"), "# Hand\n").unwrap();
        assert_eq!(tick_at(&root, &mut w), Some(IdeasChanged::default()));

        ideas::remove_at(&root, &id, None).unwrap();
        assert_eq!(tick_at(&root, &mut w).unwrap().removed, vec![id]);
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    /// A scan that fails after a good baseline is not "every idea removed":
    /// no event, the baseline kept, and the next good tick compares with it.
    #[test]
    fn a_failed_scan_sends_nothing_and_keeps_the_baseline() {
        let root = tmp("tickerr");
        let mut w = IdeasWatch::default();
        let d = ideas::create_at(&root, "Kept").unwrap();
        let id = d.idea.id.clone().unwrap();
        assert_eq!(tick_at(&root, &mut w), None, "the baseline");
        let (stamp, seen) = (w.stamp.clone(), w.seen.clone());
        std::thread::sleep(std::time::Duration::from_millis(15));
        fs::write(root.join(&d.idea.file), format!("{}\nmore\n", d.content)).unwrap();
        let fail = |_: &Path| -> std::io::Result<Index> { Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)) };
        assert_eq!(tick_with(&root, &mut w, &fail), None, "no event on a failed scan");
        assert_eq!((&w.stamp, &w.seen), (&stamp, &seen), "the baseline unchanged");
        let ev = tick_at(&root, &mut w).expect("the next good tick sees the edit");
        assert_eq!((ev.changed.len(), ev.removed.len()), (1, 0));
        assert_eq!(ev.changed[0].id, id);
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }
}
