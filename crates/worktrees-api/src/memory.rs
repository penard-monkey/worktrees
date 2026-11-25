//! A workspace's memory: the console rows over `worktrees_core::memory`, and
//! what the poll does for it (`docs/proposals/workspace-chats.md` §5, §9).
//!
//! Every row takes the workspace's id, which serve's resolver has found in
//! `workspaces.json` (`workspace(id)`), and at most a path in that workspace's
//! memory. The folder is the engine's (`Ctx::workspaces_dir`). Console only.
//!
//! **`workspaces:memory`** is `{id, changed: [{path, mtime_ms, size, rev}],
//! removed: [path]}`, one event a workspace, from the 3 s poll: the shape a
//! document's session takes for ideas (`app/src/doc/ideasave.ts`, `Changed`,
//! with a path for the id). It is sent for the app's OWN saves too, a tick
//! later, and a client tells them apart by `rev`, which names the bytes: a
//! read and a save answer with theirs, and a `changed` entry carrying it is
//! that text. `rev` is `null` for a file too large to open here. Its first
//! sight of a memory is a baseline and sends nothing.
//!
//! **What a tick costs:** one `readdir` a folder and one `lstat` an entry, for
//! every workspace that has a memory, and no git. 20 workspaces of 200 files
//! each measured 6.8 ms (core's `memory_poll_cost`, release, an Apple silicon
//! Mac, warm cache): 0.2% of one core.
//!
//! **The quiet minute** (§9 item 2). A memory that moved is committed once it
//! has been still for [`QUIET`]; a memory first seen counts as moved then, so
//! what was edited while the engine was down is committed too. A fold in
//! progress HOLDS its workspace ([`Memory::hold`]): its commit must be the
//! whole of it and under its own message, and an agent that thinks for a
//! minute would otherwise lose half of it to "Edited: …".

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use worktrees_core::memory::{self as store, Base, Change, Doc, FileChange, Index, Listing, Log, Reverted, Saved, Sig, Stamp};
use worktrees_core::{attach, registry, workspacechats, workspaces};

use crate::files::{b64_encode, PlaceImage};
use crate::Ctx;

/// How long a memory is still before the poll commits it.
pub const QUIET: Duration = Duration::from_secs(60);

/// This engine's holds: workspaces whose dirty tree is someone's to commit.
/// In memory only: whoever folds across a restart takes its hold again.
#[derive(Default)]
pub struct Memory {
    held: Mutex<HashSet<String>>,
}

impl Memory {
    /// Keep the poll from committing `id`'s memory, and refuse an undo there.
    pub fn hold(&self, id: &str) {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
    }
    pub fn release(&self, id: &str) {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }
    pub fn is_held(&self, id: &str) -> bool {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).contains(id)
    }
}

fn root(ctx: &Ctx) -> &Path {
    ctx.workspaces_dir()
}

// ── the rows ────────────────────────────────────────────────────────────────

/// `workspace_memory`'s answer. With no memory, `made` is false and the rest
/// is empty; a memory that cannot be read is an error, never this.
#[derive(Serialize, Debug)]
pub struct MemoryView {
    pub made: bool,
    /// The folder, for Show in Finder (a reveal on the host, `onhost.ts`).
    pub dir: String,
    /// `MEMORY.md`; `None` when there is none.
    pub index: Option<Index>,
    #[serde(flatten)]
    pub listing: Listing,
    /// The newest commit, with its files and counts.
    pub last: Option<Change>,
}

pub fn workspace_memory(ctx: &Ctx, id: String) -> Result<MemoryView, String> {
    let r = root(ctx);
    let dir = store::dir_at(r, &id)?.to_string_lossy().into_owned();
    if !store::ids_at(r).contains(&id) {
        return Ok(MemoryView { made: false, dir, index: None, listing: Listing::default(), last: None });
    }
    Ok(MemoryView {
        made: true,
        dir,
        index: store::index_at(r, &id)?,
        listing: store::list_at(r, &id)?,
        last: store::log_at(r, &id, None, None, Some(1))?.changes.pop(),
    })
}

/// Make the memory (§2.2). One already made is left as it is.
pub fn workspace_memory_make(ctx: &Ctx, id: String) -> Result<MemoryView, String> {
    // The removal holds this too: never a folder for a workspace just removed.
    let lock = crate::workspacechats::life_lock(ctx, &id);
    let _life = lock.lock().unwrap_or_else(|e| e.into_inner());
    let w = workspaces::read_at(ctx.workspaces_file())?.workspaces.into_iter().find(|w| w.id == id).ok_or_else(|| format!("no workspace {id}"))?;
    make(ctx, &w)?;
    workspace_memory(ctx, id)
}

/// Not under a registered project (or over one): a repository made there would
/// be taken for part of it.
fn make(ctx: &Ctx, w: &workspaces::Workspace) -> Result<(), String> {
    let r = root(ctx);
    workspacechats::check_root(r, &registry::read_lenient().roots())?;
    store::init_at(r, &w.id, &w.name).map(|_| ())
}

/// What a chat's start and resume do first. No memory yet: it is made, and one
/// that cannot be made refuses the launch. A dirty one is committed, as the
/// quiet minute would, so what the chat changes is told from what was there —
/// but not while a fold holds it: that dirty tree is the fold's. A commit that
/// fails is logged and keeps nobody from their chat: the poll tries again.
/// The caller holds `workspacechats::life_lock`.
pub fn before_launch(ctx: &Ctx, w: &workspaces::Workspace) -> Result<(), String> {
    if !store::ids_at(root(ctx)).contains(&w.id) {
        return make(ctx, w);
    }
    if !ctx.memory.is_held(&w.id) {
        if let Err(e) = memory_commit_if_dirty(ctx, &w.id, None) {
            crate::log::warn(&format!("memory: {}: could not commit before a chat: {e}", w.id));
        }
    }
    Ok(())
}

/// One text file; with `commit`, as it was in that change.
pub fn workspace_memory_file(ctx: &Ctx, id: String, path: String, commit: Option<String>) -> Result<Doc, String> {
    match commit {
        Some(c) => store::read_commit_at(root(ctx), &id, &path, &c),
        None => store::read_at(root(ctx), &id, &path),
    }
}

/// One image, as `read_place_image` answers a chat's: a client that holds it
/// sends its `etag` and is told `unchanged`.
pub fn workspace_memory_image(ctx: &Ctx, id: String, path: String, etag: Option<String>) -> Result<PlaceImage, String> {
    let (read, unchanged) = store::image_at(root(ctx), &id, &path, etag.as_deref())?;
    let etag = format!("{}-{}", read.mtime_ms, read.on_disk);
    let size = if unchanged { read.on_disk } else { read.bytes.len() as u64 };
    Ok(PlaceImage { b64: b64_encode(&read.bytes), size, truncated: read.truncated, mtime: read.mtime_ms, mime: read.mime, etag, unchanged })
}

/// The person's save. `expected_rev` is the `rev` the file was read with, or
/// `expected_mtime` from a client that knows none (both: the rev decides).
/// Neither makes the file, and is refused when there is one; `keep_history` commits what is on disk at `path` first,
/// a fold in progress or not: that text is nowhere else.
pub fn workspace_memory_write(ctx: &Ctx, id: String, path: String, content: String, expected_mtime: Option<u64>, expected_rev: Option<String>, keep_history: Option<bool>) -> Result<Saved, String> {
    let base = match (expected_rev, expected_mtime) {
        (Some(r), _) => Base::Rev(r),
        (None, Some(m)) => Base::Mtime(m),
        (None, None) => Base::None,
    };
    store::write_at(root(ctx), &id, &path, &content, &base, keep_history == Some(true))
}

/// Put a file under `files/`: `name` is a plain file name, `data` its bytes as
/// base64. A name that is taken becomes the next free one.
pub fn workspace_memory_put(ctx: &Ctx, id: String, name: String, data: String) -> Result<Saved, String> {
    if attach::decoded_len(&data).is_some_and(|n| n > store::MAX_FILE_BYTES) {
        return Err(format!("{name} is over the {} MB a memory takes", store::MAX_FILE_BYTES / 1024 / 1024));
    }
    let bytes = attach::b64_decode(&data).ok_or_else(|| format!("{name}: not base64"))?;
    store::put_file_at(root(ctx), &id, &name, &bytes)
}

/// The changes, newest first: 50 a page (`n`, at most 200), of one file with
/// `path`, older than the commit `before`.
pub fn workspace_memory_log(ctx: &Ctx, id: String, path: Option<String>, before: Option<String>, n: Option<usize>) -> Result<Log, String> {
    store::log_at(root(ctx), &id, path.as_deref(), before.as_deref(), n)
}

/// One side of a changed image, as a `data:` URI takes it.
#[derive(Serialize, Debug)]
pub struct ImageSide {
    pub mime: &'static str,
    pub size: u64,
    /// Empty when `refused`.
    pub b64: String,
    pub refused: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Diff {
    Text { patch: String, truncated: bool },
    Image { before: Option<ImageSide>, after: Option<ImageSide> },
    Binary,
}

#[derive(Serialize, Debug)]
pub struct FileDiff {
    #[serde(flatten)]
    pub change: FileChange,
    pub diff: Diff,
}

#[derive(Serialize, Debug)]
pub struct Shown {
    pub change: Change,
    pub file: Option<FileDiff>,
}

/// One change, and with `path` that one file's difference in it.
pub fn workspace_memory_diff(ctx: &Ctx, id: String, commit: String, path: Option<String>) -> Result<Shown, String> {
    let s = store::show_at(root(ctx), &id, &commit, path.as_deref())?;
    let side = |b: store::Blob| ImageSide { mime: b.mime, size: b.size, b64: b64_encode(&b.bytes), refused: b.refused };
    Ok(Shown {
        change: s.change,
        file: s.file.map(|(change, d)| FileDiff {
            change,
            diff: match d {
                store::FileDiff::Text { patch, truncated } => Diff::Text { patch, truncated },
                store::FileDiff::Image { before, after } => Diff::Image { before: before.map(side), after: after.map(side) },
                store::FileDiff::Binary => Diff::Binary,
            },
        }),
    })
}

/// Undo a change, or redo an undo: a new commit. `conflicts` names the files
/// that changed since, and nothing was done.
pub fn workspace_memory_revert(ctx: &Ctx, id: String, commit: String) -> Result<Reverted, String> {
    if ctx.memory.is_held(&id) {
        return Err("a chat is folding into this memory: wait for it, or stop it".into());
    }
    let done = store::revert_at(root(ctx), &id, &commit)?;
    // An undone fold says so in its chat's record, and a redone one no longer.
    if done.change.is_some() {
        crate::workspacefolds::mark_reverted(ctx, &id);
    }
    Ok(done)
}

/// Commit what is dirty, for whoever starts a chat or a fold.
pub fn memory_commit_if_dirty(ctx: &Ctx, id: &str, message: Option<&str>) -> Result<Option<Change>, String> {
    store::commit_if_dirty_at(root(ctx), id, message)
}

// ── the poll ────────────────────────────────────────────────────────────────

struct Seen {
    stamp: Stamp,
    /// When it last moved; `None` once that has been committed.
    moved: Option<Instant>,
    /// The chats that were live through a tick it moved in, each with when its
    /// last such tick began (seconds since the epoch).
    live: BTreeMap<String, i64>,
}

impl Seen {
    /// The chats to name on a commit made after the one at `last`: live through
    /// a tick that began after it. A tick that began before it, or in its
    /// second, may have moved what that commit already holds.
    fn live_since(&self, last: i64) -> Vec<String> {
        self.live.iter().filter(|(_, began)| **began > last).map(|(c, _)| c.clone()).collect()
    }
}

/// What the poll last saw of each workspace's memory.
#[derive(Default)]
pub struct MemoryWatch {
    seen: HashMap<String, Seen>,
    /// The last thing said about each, so a failure that repeats every tick is
    /// one line.
    said: HashMap<String, String>,
    /// The tick before this one: when it was, and which chats were live in
    /// each workspace. `None` before the first.
    before: Option<(i64, Live)>,
    /// This one's.
    here: (i64, Live),
}

/// The live chats of each workspace.
pub type Live = HashMap<String, HashSet<String>>;

impl MemoryWatch {
    /// The chats live at both ends of the tick that ends now, and when it
    /// began: whoever was there for whatever moved in it. One that started or
    /// ended in it is left out, because it may not have been; and a first tick
    /// knows nobody.
    fn through(&self, id: &str) -> Vec<(String, i64)> {
        let Some((began, was)) = &self.before else { return Vec::new() };
        let (Some(was), Some(is)) = (was.get(id), self.here.1.get(id)) else { return Vec::new() };
        was.intersection(is).map(|c| (c.clone(), *began)).collect()
    }
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Moved {
    pub path: String,
    pub mtime_ms: u64,
    pub size: u64,
    /// The bytes' own name (`Doc::rev`); `None` for a file too large to open.
    pub rev: Option<String>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct MemoryChanged {
    pub id: String,
    pub changed: Vec<Moved>,
    pub removed: Vec<String>,
}

/// One tick: the events due, and which memories have been quiet long enough to
/// commit. `now` is the caller's clock and `stamp` its reading of one memory.
/// A memory that could not be read sends nothing and keeps what was known: read
/// as empty it would list every file as removed.
fn tick_in(
    ids: &[String],
    w: &mut MemoryWatch,
    now: Instant,
    held: &dyn Fn(&str) -> bool,
    stamp: &dyn Fn(&str) -> Result<Stamp, String>,
    rev: &dyn Fn(&str, &str, Sig) -> Moved,
) -> (Vec<MemoryChanged>, Vec<String>) {
    let (mut events, mut quiet) = (Vec::new(), Vec::new());
    w.seen.retain(|id, _| ids.contains(id));
    w.said.retain(|id, _| ids.contains(id));
    for id in ids {
        let through = w.through(id);
        let at = match stamp(id) {
            Ok(s) => s,
            Err(e) => {
                if w.said.get(id) != Some(&e) {
                    crate::log::warn(&format!("memory: {id}: {e}"));
                    w.said.insert(id.clone(), e);
                }
                continue;
            }
        };
        match w.seen.get_mut(id) {
            None => {
                w.seen.insert(id.clone(), Seen { stamp: at, moved: Some(now), live: BTreeMap::new() });
                continue;
            }
            Some(s) => {
                let (at, changed, removed) = store::advance(&s.stamp, at);
                s.stamp = at;
                let moved = !changed.is_empty() || !removed.is_empty();
                let changed = changed.into_iter().map(|(path, sig)| rev(id, &path, sig)).collect();
                if moved {
                    events.push(MemoryChanged { id: id.clone(), changed, removed });
                    s.moved = Some(now);
                    s.live.extend(through);
                } else if held(id) {
                    // The whole minute is still owed once it is let go.
                    s.moved = Some(now);
                } else if s.moved.is_some_and(|t| now.saturating_duration_since(t) >= QUIET) {
                    s.moved = None;
                    quiet.push(id.clone());
                }
            }
        }
    }
    (events, quiet)
}

/// A file that moved, as it is now: what was stamped may have moved again
/// since, and what is told is the bytes a read would answer with.
fn moved(root: &Path, id: &str, path: &str, sig: Sig) -> Moved {
    let (mtime_ms, size, rev) = store::seen_at(root, id, path).unwrap_or((sig.mtime_ms, sig.size, None));
    Moved { path: path.into(), mtime_ms, size, rev }
}

/// The poll's beat.
///
/// `live` is which chats are live in this same beat: a quiet-minute commit
/// names, in its body, the ones that were live while it was changed. It costs
/// no call (the poll has them) and claims no author: History says "while
/// “<chat>” was live", which is true of a hand edit too.
pub fn tick(ctx: &Ctx, w: &mut MemoryWatch, live: Live) {
    let r = root(ctx);
    let wall = worktrees_core::sysclock::now_epoch();
    let was = std::mem::replace(&mut w.here, (wall, live));
    // The default stands for "no tick yet", and no clock reads 0.
    w.before = (was.0 != 0).then_some(was);
    let ids = store::ids_at(r);
    let (events, quiet) = tick_in(&ids, w, Instant::now(), &|id| ctx.memory.is_held(id), &|id| store::stamp_at(r, id), &|id, path, sig| moved(r, id, path, sig));
    for ev in events {
        ctx.emit("workspaces:memory", serde_json::to_value(ev).unwrap_or(serde_json::Value::Null));
    }
    for id in quiet {
        let done = store::commit_quiet_at(r, &id, |last| w.seen.get(&id).map(|s| s.live_since(last)).unwrap_or_default());
        if let (Some(s), Ok(_)) = (w.seen.get_mut(&id), &done) {
            s.live.clear();
        }
        match done {
            Ok(Some(c)) => {
                w.said.remove(&id);
                crate::log::info(&format!("memory: {id}: committed {} ({} file(s))", &c.commit[..7], c.files.len()));
            }
            Ok(None) => {}
            Err(e) => {
                // Tried again after another quiet minute, and said once.
                if let Some(s) = w.seen.get_mut(&id) {
                    s.moved = Some(Instant::now());
                }
                if w.said.get(&id) != Some(&e) {
                    crate::log::warn(&format!("memory: {id}: could not commit: {e}"));
                    w.said.insert(id, e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use worktrees_core::{fsx, ulid};

    struct Scratch {
        base: PathBuf,
        ctx: Ctx,
        rec: Arc<crate::tests::Recorder>,
        id: String,
    }

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let base = std::env::temp_dir().join(format!("wt-api-memory-{tag}-{}-{:x}", std::process::id(), fsx::random_u64()));
            fs::create_dir_all(&base).unwrap();
            let rec = Arc::new(crate::tests::Recorder::default());
            let ctx = Ctx::new(base.join("cfg"), rec.clone()).with_workspaces_dir(base.join("ws"));
            let id = ulid::new();
            store::init_at(&base.join("ws"), &id, "Personal").unwrap();
            Scratch { base, ctx, rec, id }
        }
        fn put(&self, path: &str, text: &str) {
            let p = store::dir_at(self.ctx.workspaces_dir(), &self.id).unwrap().join(path);
            fs::write(p, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn plain(_: &str, path: &str, s: Sig) -> Moved {
        Moved { path: path.into(), mtime_ms: s.mtime_ms, size: s.size, rev: None }
    }

    fn stamp(files: &[(&str, u64)]) -> Stamp {
        Stamp::of(files.iter().map(|(p, m)| (p.to_string(), Sig { mtime_ms: *m, size: 1, ..Sig::default() })))
    }

    /// The rows over one memory, as a client reads them.
    #[test]
    fn the_rows_answer_the_shapes_a_client_reads() {
        let s = Scratch::new("rows");
        let c = &s.ctx;
        let id = || s.id.clone();
        let none = workspace_memory(c, ulid::new()).unwrap();
        assert!(!none.made && none.index.is_none() && none.last.is_none());
        let v = serde_json::to_value(workspace_memory(c, id()).unwrap()).unwrap();
        assert_eq!((v["made"].as_bool(), v["count"].as_u64(), v["folders"][0]["files"][0]["path"].as_str()), (Some(true), Some(1), Some("MEMORY.md")), "{v}");
        assert_eq!((v["index"]["over_budget"].as_bool(), v["index"]["budget"].as_u64(), v["last"]["kind"].as_str()), (Some(false), Some(16384), Some("made")));
        let png = [b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01".to_vec(), vec![8, 2, 0, 0, 0]].concat();
        let put = workspace_memory_put(c, id(), "rack.png".into(), b64_encode(&png)).unwrap();
        assert_eq!(put.path, "files/rack.png");
        let img = workspace_memory_image(c, id(), put.path.clone(), None).unwrap();
        assert_eq!((img.b64.as_str(), img.mime, img.truncated), (b64_encode(&png).as_str(), "image/png", false));
        let again = workspace_memory_image(c, id(), put.path.clone(), Some(img.etag)).unwrap();
        assert!(again.unchanged && again.b64.is_empty());
        assert!(workspace_memory_put(c, id(), "x.bin".into(), "!".into()).unwrap_err().contains("base64"));
        assert!(workspace_memory_put(c, id(), "x.bin".into(), "!!!!".repeat(2_000_000)).unwrap_err().contains("4 MB"), "its size, before it is decoded");
        let w = workspace_memory_write(c, id(), "topics/t.md".into(), "# T\n".into(), None, None, None).unwrap();
        let d = workspace_memory_file(c, id(), "topics/t.md".into(), None).unwrap();
        assert_eq!((d.content.as_str(), d.mtime_ms, d.title.as_str()), ("# T\n", w.mtime_ms, "T"));
        // A base named by its bytes decides, whatever the mtime beside it says.
        let write = |m: u64, r: &str| workspace_memory_write(c, id(), "topics/t.md".into(), "# U\n".into(), Some(m), Some(r.into()), None);
        assert!(write(d.mtime_ms, "0").unwrap_err().starts_with(store::ERR_STALE));
        assert_eq!((d.rev.as_str(), write(1, &d.rev).unwrap().size), (w.rev.as_str(), 4));
        let back = workspace_memory_write(c, id(), "topics/t.md".into(), "# T\n".into(), None, Some(store::rev_of(b"# U\n")), None).unwrap();
        assert_eq!(back.rev, d.rev);
        let done = memory_commit_if_dirty(c, &s.id, Some("fold: X")).unwrap().unwrap();
        assert_eq!(memory_commit_if_dirty(c, &s.id, Some("fold: X")).unwrap(), None);
        let v = serde_json::to_value(workspace_memory_diff(c, id(), done.commit.clone(), Some("files/rack.png".into())).unwrap()).unwrap();
        assert_eq!((v["file"]["path"].as_str(), v["file"]["diff"]["kind"].as_str()), (Some("files/rack.png"), Some("image")), "{v}");
        assert_eq!((v["file"]["diff"]["before"].is_null(), v["file"]["diff"]["after"]["b64"].as_str()), (true, Some(b64_encode(&png).as_str())));
        let v = serde_json::to_value(workspace_memory_diff(c, id(), done.commit.clone(), Some("topics/t.md".into())).unwrap()).unwrap();
        assert!(v["file"]["diff"]["kind"] == "text" && v["file"]["diff"]["patch"].as_str().unwrap().contains("+# T"), "{v}");
        // A fold in progress owns the dirty tree: no undo. A text kept over
        // what it wrote commits that one file, and nothing else of its.
        c.memory.hold(&s.id);
        assert!(workspace_memory_revert(c, id(), done.commit.clone()).unwrap_err().contains("folding"));
        s.put("topics/t.md", "# Theirs\n");
        s.put("notes/half.md", "half\n");
        let m = workspace_memory_file(c, id(), "topics/t.md".into(), None).unwrap().mtime_ms;
        workspace_memory_write(c, id(), "topics/t.md".into(), "# Mine\n".into(), Some(m), None, Some(true)).unwrap();
        let top = workspace_memory_log(c, id(), None, None, Some(1)).unwrap().changes.remove(0);
        assert_eq!(top.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["topics/t.md"]);
        assert_eq!(workspace_memory_file(c, id(), "topics/t.md".into(), Some(top.commit)).unwrap().content, "# Theirs\n");
        c.memory.release(&s.id);
        let r = workspace_memory_revert(c, id(), done.commit).unwrap();
        assert_eq!((r.change, r.conflicts), (None, vec!["topics/t.md".to_string()]));
    }

    /// A chat's start and resume: no memory is made, a dirty one is committed
    /// as the quiet minute would, and a fold's dirty tree is left to the fold.
    #[test]
    fn a_launch_makes_the_memory_and_commits_a_dirty_one_unless_a_fold_holds_it() {
        let s = Scratch::new("launch");
        let w = workspaces::Workspace { id: ulid::new(), name: "Homelab".into(), ..Default::default() };
        before_launch(&s.ctx, &w).unwrap();
        let v = workspace_memory(&s.ctx, w.id.clone()).unwrap();
        assert!(v.made && v.index.unwrap().content.starts_with("# Memory: Homelab\n"));
        let log = |id: &str| workspace_memory_log(&s.ctx, id.into(), None, None, None).unwrap().changes;
        before_launch(&s.ctx, &w).unwrap();
        assert_eq!(log(&w.id).len(), 1, "nothing to commit");
        let mine = workspaces::Workspace { id: s.id.clone(), name: "Personal".into(), ..Default::default() };
        s.put("topics/t.md", "by hand\n");
        s.ctx.memory.hold(&s.id);
        before_launch(&s.ctx, &mine).unwrap();
        assert_eq!(log(&s.id).len(), 1, "a fold's dirty tree is the fold's");
        s.ctx.memory.release(&s.id);
        before_launch(&s.ctx, &mine).unwrap();
        assert_eq!(log(&s.id)[0].subject, "Edited: topics/t.md");
        // Broken, it keeps nobody from a chat; one that cannot be MADE does.
        fs::write(store::dir_at(s.ctx.workspaces_dir(), &s.id).unwrap().join(".git/config"), "[").unwrap();
        before_launch(&s.ctx, &mine).unwrap();
        let bad = workspaces::Workspace { id: ulid::new(), name: "B".into(), ..Default::default() };
        fs::write(s.ctx.workspaces_dir().join(&bad.id), "a file where its folder goes").unwrap();
        assert!(before_launch(&s.ctx, &bad).is_err());
    }

    /// A removed workspace's memory goes with its folder, history and all, its
    /// hold is let go, and the move waits for whoever is in it.
    #[test]
    fn a_removed_workspaces_memory_moves_with_its_folder_and_its_hold_is_released() {
        use std::os::fd::AsRawFd;
        let s = Scratch::new("removed");
        let file = s.base.join("cfg").join("workspaces.json");
        let ctx = s.ctx.clone().with_workspaces_file(file.clone());
        let id = workspaces::create_at(&file, "Home").unwrap().id;
        workspace_memory_make(&ctx, id.clone()).unwrap();
        ctx.memory.hold(&id);
        // Someone is in it: the removal waits, however long.
        let at = ctx.workspaces_dir().join(&id);
        let lock = fs::File::options().write(true).open(at.join(".memory.lock")).unwrap();
        // SAFETY: a live descriptor, closed below.
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let (c2, i2) = (ctx.clone(), id.clone());
        let t = std::thread::spawn(move || tx.send(crate::workspaces::workspace_remove(&c2, i2).map(|r| r.moved_to)).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err(), "it did not wait");
        assert!(at.join("memory/MEMORY.md").exists());
        drop(lock);
        let to = rx.recv_timeout(Duration::from_secs(20)).expect("the removal never ended").unwrap().unwrap();
        t.join().unwrap();
        assert_eq!(to, ctx.workspaces_dir().join(".removed").join(&id).to_string_lossy());
        assert!(Path::new(&to).join("memory/MEMORY.md").is_file() && Path::new(&to).join("memory/.git").is_dir());
        assert!(!ctx.memory.is_held(&id), "its hold is released");
        assert!(!at.exists() && !store::ids_at(ctx.workspaces_dir()).contains(&id));
        assert!(workspace_memory_make(&ctx, id).unwrap_err().contains("no workspace"), "and none is made for it again");
        assert!(!at.exists());
    }

    /// The event names each path and the mtime a client compares with its own
    /// save; the first sight of a memory, and a memory that went, send nothing.
    #[test]
    fn the_event_names_each_path_and_the_mtime_a_save_answered_with() {
        let s = Scratch::new("event");
        let mut w = MemoryWatch::default();
        tick(&s.ctx, &mut w, Live::new());
        assert!(s.rec.events.lock().unwrap().is_empty(), "the baseline");
        let saved = workspace_memory_write(&s.ctx, s.id.clone(), "topics/t.md".into(), "# T\n".into(), None, None, None).unwrap();
        tick(&s.ctx, &mut w, Live::new());
        tick(&s.ctx, &mut w, Live::new());
        let ev = std::mem::take(&mut *s.rec.events.lock().unwrap());
        assert_eq!(ev, vec![(
            "workspaces:memory".to_string(),
            serde_json::json!({"id": s.id, "changed": [{"path": "topics/t.md", "mtime_ms": saved.mtime_ms, "size": 4, "rev": saved.rev}], "removed": []}),
        )]);
        memory_commit_if_dirty(&s.ctx, &s.id, None).unwrap().unwrap();
        tick(&s.ctx, &mut w, Live::new());
        assert!(s.rec.events.lock().unwrap().is_empty(), "a commit is no change to a file");
        fs::remove_file(store::dir_at(s.ctx.workspaces_dir(), &s.id).unwrap().join("topics/t.md")).unwrap();
        tick(&s.ctx, &mut w, Live::new());
        assert_eq!(s.rec.events.lock().unwrap()[0].1["removed"], serde_json::json!(["topics/t.md"]));
        s.rec.events.lock().unwrap().clear();
        fs::remove_dir_all(s.ctx.workspaces_dir()).unwrap();
        tick(&s.ctx, &mut w, Live::new());
        assert!(s.rec.events.lock().unwrap().is_empty(), "a workspace that went is not its files removed one by one");
    }

    /// The quiet minute, on a scripted clock: committed once, a minute after
    /// the LAST move, and a memory first seen counts as moved then.
    #[test]
    fn a_memory_is_committed_a_quiet_minute_after_it_last_moved_and_once() {
        let ids = vec!["A".to_string()];
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut w = MemoryWatch::default();
        let mut go = |sec: u64, st: &Stamp| tick_in(&ids, &mut w, at(sec), &|_| false, &|_| Ok(st.clone()), &plain);
        let (a, b) = (stamp(&[("x.md", 1)]), stamp(&[("x.md", 2)]));
        assert_eq!(go(0, &a), (vec![], vec![]));
        assert_eq!(go(59, &a).1, Vec::<String>::new());
        assert_eq!(go(60, &a).1, ids, "what was edited while the engine was down");
        assert_eq!(go(120, &a).1, Vec::<String>::new(), "once");
        assert_eq!(go(123, &b).0.len(), 1);
        assert_eq!(go(150, &a).0.len(), 1, "moved again");
        assert_eq!(go(183, &a).1, Vec::<String>::new(), "33 s after the last move");
        assert_eq!(go(210, &a).1, ids);
    }

    #[test]
    fn a_held_memory_waits_and_then_owes_the_whole_minute() {
        let ids = vec!["A".to_string()];
        let t0 = Instant::now();
        let mut w = MemoryWatch::default();
        let mut go = |sec: u64, held: bool| tick_in(&ids, &mut w, t0 + Duration::from_secs(sec), &|_| held, &|_| Ok(stamp(&[("x.md", 1)])), &plain).1;
        go(0, false);
        assert!(go(70, true).is_empty(), "a fold is thinking");
        assert!(go(300, true).is_empty());
        assert!(go(310, false).is_empty(), "let go: the minute starts");
        assert_eq!(go(360, false), ids);
    }

    /// A memory that went and came back (a workspace removed and put back by
    /// hand) is seen for the first time again, not compared with what it was.
    #[test]
    fn a_memory_that_comes_back_is_a_baseline_again() {
        let ids = vec!["A".to_string()];
        let t = Instant::now();
        let mut w = MemoryWatch::default();
        tick_in(&ids, &mut w, t, &|_| false, &|_| Ok(stamp(&[("x.md", 1)])), &plain);
        tick_in(&[], &mut w, t, &|_| false, &|_| Ok(Stamp::default()), &plain);
        assert_eq!(tick_in(&ids, &mut w, t, &|_| false, &|_| Ok(stamp(&[("x.md", 9)])), &plain), (vec![], vec![]));
    }

    #[test]
    fn a_memory_that_cannot_be_read_sends_nothing_and_keeps_what_was_known() {
        let ids = vec!["A".to_string()];
        let t = Instant::now();
        let mut w = MemoryWatch::default();
        let a = stamp(&[("x.md", 1), ("y.md", 1)]);
        tick_in(&ids, &mut w, t, &|_| false, &|_| Ok(a.clone()), &plain);
        assert_eq!(tick_in(&ids, &mut w, t, &|_| false, &|_| Err("denied".into()), &plain), (vec![], vec![]));
        let b = stamp(&[("x.md", 1), ("y.md", 2)]);
        let (ev, _) = tick_in(&ids, &mut w, t, &|_| false, &|_| Ok(b.clone()), &plain);
        assert_eq!(ev, vec![MemoryChanged { id: "A".into(), changed: vec![plain("A", "y.md", Sig { mtime_ms: 2, size: 1, ..Sig::default() })], removed: vec![] }]);
    }

    /// Who a quiet commit names: live at both ends of the tick it moved in,
    /// and only for what no commit holds yet.
    #[test]
    fn a_quiet_commit_names_only_chats_that_were_live_throughout() {
        let ids = vec!["A".to_string()];
        let t = Instant::now();
        let mut w = MemoryWatch::default();
        let live = |chats: &[&str]| -> Live { HashMap::from([("A".to_string(), chats.iter().map(|c| c.to_string()).collect())]) };
        // One beat: the clock, who is live, and the memory's stamp.
        let mut beat = |wall: i64, chats: &[&str], m: u64| {
            let was = std::mem::replace(&mut w.here, (wall, live(chats)));
            w.before = (was.0 != 0).then_some(was);
            tick_in(&ids, &mut w, t, &|_| false, &|_| Ok(stamp(&[("x.md", m)])), &plain);
            w.seen["A"].live.iter().map(|(c, at)| (c.clone(), *at)).collect::<Vec<_>>()
        };
        assert!(beat(100, &["one"], 1).is_empty(), "a first sight knows nobody");
        assert!(beat(103, &["one"], 1).is_empty(), "nothing moved");
        assert_eq!(beat(106, &["one", "two"], 2), vec![("one".to_string(), 103)], "two started in it");
        assert_eq!(beat(109, &["two"], 3), vec![("one".to_string(), 103), ("two".to_string(), 106)], "one ended in it");
        assert_eq!(beat(112, &[], 4).len(), 2, "nobody was there throughout");
        let s = &w.seen["A"];
        assert_eq!(s.live_since(0), vec!["one", "two"]);
        // A commit made at 104 may hold what moved between 103 and 106.
        assert_eq!(s.live_since(104), vec!["two"]);
        assert_eq!(s.live_since(106), Vec::<String>::new(), "in its very second");
    }

    /// The poll's own commit, end to end: the chat that was live is named, and
    /// the next quiet minute starts again with nobody.
    #[test]
    fn the_polls_quiet_commit_carries_the_live_chats() {
        let s = Scratch::new("quiet-live");
        let chat = ulid::new();
        let mut w = MemoryWatch::default();
        let live = || -> Live { HashMap::from([(s.id.clone(), HashSet::from([chat.clone()]))]) };
        // The scratch's first commit is seconds old: the clock is moved past it.
        let beat = |w: &mut MemoryWatch| {
            tick(&s.ctx, w, live());
            w.here.0 += 60;
        };
        beat(&mut w);
        s.put("topics/t.md", "one\n");
        beat(&mut w);
        w.seen.get_mut(&s.id).unwrap().moved = Instant::now().checked_sub(QUIET);
        beat(&mut w);
        let top = || store::log_at(s.ctx.workspaces_dir(), &s.id, None, None, Some(1)).unwrap().changes.remove(0);
        assert_eq!((top().subject.as_str(), top().live), ("Edited: topics/t.md", vec![chat.clone()]));
        assert!(w.seen[&s.id].live.is_empty(), "a commit clears who was named");
    }

    #[test]
    fn an_engine_under_test_cannot_write_the_real_folder() {
        worktrees_core::registry::forbid_real_writes();
        let real = worktrees_core::wsfolder::default_root();
        let existed = real.exists();
        let id = ulid::new();
        for e in [
            store::init_at(&real, &id, "x").unwrap_err(),
            store::write_at(&real, &id, "a.md", "x", &Base::None, false).unwrap_err(),
            store::put_file_at(&real, &id, "a.png", b"x").unwrap_err(),
            store::commit_if_dirty_at(&real, &id, None).unwrap_err(),
            store::revert_at(&real, &id, &"0".repeat(40)).unwrap_err(),
        ] {
            assert!(e.contains("real workspaces folder"), "{e}");
        }
        assert_eq!(real.exists(), existed, "not even the folder");
    }
}
