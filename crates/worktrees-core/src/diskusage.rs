//! Disk usage of places: what each place holds alone, what it shares, and
//! what can be deleted from it (`worktrees du`, serve's `disk_*` rows).
//!
//! **Counting.** A place's files are walked with `lstat` and never followed
//! through a symlink. A file's bytes are its ALLOCATED blocks. Two things make
//! a byte count lie, and both are corrected here:
//! - **Hard links**: one inode counted once, however many names it has.
//! - **APFS clones** (`cp -c`, pnpm's store, `git worktree` checkouts that
//!   clone): a cloned file shows its full size under every name while the
//!   blocks exist once. On macOS each file's clone id and clone reference count
//!   come from `getattrlist(ATTR_CMNEXT_CLONEID | ATTR_CMNEXT_CLONE_REFCNT)`.
//!
//! A file's bytes are the place's **own** when every name of the inode and
//! every clone of the file is inside the place: removing the place frees them.
//! Otherwise they are **shared**, and removing the place frees nothing of them.
//! `ATTR_CMNEXT_PRIVATESIZE` is NOT used: Time Machine's local snapshots pin
//! blocks, so it reads about zero for any file older than the last snapshot
//! (measured 2026-11-24, nine snapshots). A clone that diverged after a partial
//! write is counted whole; that case is not measured.
//!
//! **Categories** are by folder NAME, an estimate: `node_modules`, `Pods`,
//! `.venv`, `venv`, `vendor` are dependencies; `target`, `.next`, `.turbo`,
//! `dist`, `build`, `.build`, `DerivedData*`, `out`, `.gradle`, `.cache`,
//! `.swiftpm` are build output; the place's `.git` is git; the rest is source.
//!
//! **Deleting** ([`delete_in_place`]) is narrower than the categories: only a
//! path git IGNORES, holding nothing git tracks, a real directory or file (no
//! symlink anywhere on the way), never `.git`, never `(main)`'s `.worktrees/`.
//! It is permanent: the Trash would free nothing until emptied.
//!
//! Nothing here runs in the 3 s poll. A scan is asked for, and its answer is
//! cached per project until the next ask with `refresh`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use crate::Project;

const DEPS: &[&str] = &["node_modules", "Pods", ".venv", "venv", "vendor"];
const BUILD: &[&str] = &["target", ".next", ".turbo", "dist", "build", ".build", "out", ".gradle", ".cache", ".swiftpm"];

/// What a folder's bytes are, by its name or an ancestor's.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Cat {
    Build,
    Deps,
    Source,
    Git,
}

impl Cat {
    fn idx(self) -> usize {
        self as usize
    }
    const ALL: [Cat; 4] = [Cat::Build, Cat::Deps, Cat::Source, Cat::Git];
}

/// The category a directory named `name` takes, given its parent's.
fn cat_of(parent: Cat, name: &str, depth: usize) -> Cat {
    if parent != Cat::Source {
        return parent;
    }
    if depth == 0 && name == ".git" {
        return Cat::Git;
    }
    if DEPS.contains(&name) {
        return Cat::Deps;
    }
    if BUILD.contains(&name) || name.starts_with("DerivedData") {
        return Cat::Build;
    }
    Cat::Source
}

/// Bytes per category.
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct ByCat {
    pub build: u64,
    pub deps: u64,
    pub source: u64,
    pub git: u64,
}

impl ByCat {
    fn from(a: &[u64; 4]) -> ByCat {
        ByCat { build: a[0], deps: a[1], source: a[2], git: a[3] }
    }
    pub fn total(&self) -> u64 {
        self.build + self.deps + self.source + self.git
    }
}

/// A folder a cleanup may offer: the OUTERMOST folder named as build output or
/// dependencies. Whether git ignores it is asked only at delete time.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Cleanable {
    /// Relative to the place's root, `/`-separated.
    pub path: String,
    pub cat: Cat,
    /// Bytes only this place holds: what deleting it frees.
    pub own: u64,
    pub shared: u64,
    pub files: u64,
    /// Identity at scan time, as [`Node::id`].
    pub id: String,
}

/// One place's usage.
#[derive(Serialize, Clone, Debug)]
pub struct PlaceUsage {
    pub slug: String,
    pub path: String,
    pub is_main: bool,
    /// Bytes only this place holds, by category: what removing it frees.
    pub own: ByCat,
    /// Bytes this place shares with another place or a store.
    pub shared: u64,
    /// Allocated blocks as `du` counts them: a hard-linked inode once, every
    /// clone in full (what `du -sh` says of the folder).
    pub alloc: u64,
    pub files: u64,
    /// Entries that could not be read (permissions, a race with a delete).
    pub errors: u64,
    /// Largest first; at most [`CLEANABLE_MAX`].
    pub cleanable: Vec<Cleanable>,
}

pub const CLEANABLE_MAX: usize = 12;

/// A project's usage: every place, scanned at `scanned_epoch`.
#[derive(Serialize, Clone, Debug)]
pub struct ProjectUsage {
    pub root: String,
    pub scanned_epoch: i64,
    pub scan_ms: u64,
    pub places: Vec<PlaceUsage>,
    /// Every block once across the project: hard links and clones counted
    /// once however many places hold them.
    pub unique: u64,
    pub volume: Volume,
}

/// The volume the project is on.
#[derive(Serialize, Clone, Debug, Default)]
pub struct Volume {
    pub free: u64,
    pub total: u64,
    /// Time Machine local snapshots on the data volume, macOS only: they hold
    /// deleted files' blocks until they expire, so a delete may free nothing
    /// at once. `None` where it cannot be asked.
    pub snapshots: Option<u32>,
}

/// One node of a place's tree, pruned for drawing.
#[derive(Serialize, Clone, Debug)]
pub struct Node {
    /// The name; empty for an aggregate of small entries.
    pub n: String,
    /// Relative path from the place root; absent for an aggregate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p: Option<String>,
    pub own: u64,
    pub shared: u64,
    pub files: u64,
    /// The category of a leaf or aggregate; a folder's is its own name's.
    pub c: Cat,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dir: bool,
    /// Identity at scan time, `"<dev>:<ino>"`, checked by a delete. A string:
    /// an inode number can pass 2^53, where a JSON number stops being exact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub k: Vec<Node>,
}

// ── the walk ──────────────────────────────────────────────────────────────

struct Dir {
    name: String,
    parent: usize,
    cat: Cat,
    dev: u64,
    ino: u64,
    kids: Vec<usize>,
    /// Bytes of files directly in it: own by category, shared.
    own: [u64; 4],
    shared: u64,
    files: u64,
    /// Files at least `BIG_FILE` large, kept as leaves.
    big: Vec<(String, u64, bool, u64, u64)>,
}

const BIG_FILE: u64 = 4 << 20;

/// A file that has another name or a clone somewhere: decided after the walk.
struct Multi {
    dir: usize,
    name: String,
    ino: u64,
    dev: u64,
    nlink: u64,
    clone: u64,
    refs: u32,
    alloc: u64,
}

/// The raw result of walking one place.
struct Walk {
    dirs: Vec<Dir>,
    alloc: u64,
    files: u64,
    errors: u64,
    /// `(group key, alloc)` of every file with another name or clone, once per
    /// group in this place: the project's `unique` merges these.
    groups: HashMap<u64, u64>,
    /// Bytes of files with one name and no clone.
    singles: u64,
}

#[cfg(target_os = "macos")]
fn clone_info(path: &Path) -> Option<(u64, u32)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    #[repr(C, packed)]
    struct Buf {
        len: u32,
        clone: u64,
        refs: u32,
    }
    const ATTR_CMNEXT_CLONEID: u32 = 0x100;
    const ATTR_CMNEXT_CLONE_REFCNT: u32 = 0x1000;
    const FSOPT_NOFOLLOW: u32 = 0x1;
    const FSOPT_ATTR_CMN_EXTENDED: u32 = 0x20;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
    al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    al.forkattr = ATTR_CMNEXT_CLONEID | ATTR_CMNEXT_CLONE_REFCNT;
    let mut b = Buf { len: 0, clone: 0, refs: 0 };
    let r = unsafe {
        libc::getattrlist(
            c.as_ptr(),
            &mut al as *mut _ as *mut libc::c_void,
            &mut b as *mut _ as *mut libc::c_void,
            std::mem::size_of::<Buf>(),
            FSOPT_NOFOLLOW | FSOPT_ATTR_CMN_EXTENDED,
        )
    };
    if r != 0 {
        return None;
    }
    Some((b.clone, b.refs))
}

#[cfg(not(target_os = "macos"))]
fn clone_info(_path: &Path) -> Option<(u64, u32)> {
    None
}

fn walk(root: &Path, is_main: bool) -> Walk {
    let mut w = Walk { dirs: Vec::new(), alloc: 0, files: 0, errors: 0, groups: HashMap::new(), singles: 0 };
    let Ok(md) = fs::symlink_metadata(root) else {
        w.errors += 1;
        return w;
    };
    w.dirs.push(Dir {
        name: String::new(),
        parent: usize::MAX,
        cat: Cat::Source,
        dev: md.dev(),
        ino: md.ino(),
        kids: Vec::new(),
        own: [0; 4],
        shared: 0,
        files: 0,
        big: Vec::new(),
    });
    let mut multis: Vec<Multi> = Vec::new();
    // (dir index, path, depth)
    let mut stack: Vec<(usize, PathBuf, usize)> = vec![(0, root.to_path_buf(), 0)];
    while let Some((di, path, depth)) = stack.pop() {
        let rd = match fs::read_dir(&path) {
            Ok(r) => r,
            Err(_) => {
                w.errors += 1;
                continue;
            }
        };
        for e in rd {
            let Ok(e) = e else {
                w.errors += 1;
                continue;
            };
            let name = e.file_name().to_string_lossy().into_owned();
            // (main)'s folder holds every other place; each is its own row.
            if is_main && depth == 0 && name == ".worktrees" {
                continue;
            }
            let p = e.path();
            let Ok(md) = fs::symlink_metadata(&p) else {
                w.errors += 1;
                continue;
            };
            let ft = md.file_type();
            if ft.is_dir() {
                let cat = cat_of(w.dirs[di].cat, &name, depth);
                let ni = w.dirs.len();
                w.dirs.push(Dir {
                    name,
                    parent: di,
                    cat,
                    dev: md.dev(),
                    ino: md.ino(),
                    kids: Vec::new(),
                    own: [0; 4],
                    shared: 0,
                    files: 0,
                    big: Vec::new(),
                });
                w.dirs[di].kids.push(ni);
                // Another filesystem mounted inside a place is not the place's.
                if md.dev() == w.dirs[0].dev {
                    stack.push((ni, p, depth + 1));
                }
                continue;
            }
            // Files and symlinks (a link's own blocks, never its target's).
            let alloc = md.blocks() * 512;
            w.files += 1;
            w.dirs[di].files += 1;
            let (clone, refs) = if ft.is_file() && alloc > 0 { clone_info(&p).unwrap_or((0, 0)) } else { (0, 0) };
            if md.nlink() > 1 || refs > 1 {
                multis.push(Multi { dir: di, name, ino: md.ino(), dev: md.dev(), nlink: md.nlink(), clone, refs, alloc });
            } else {
                w.alloc += alloc;
                w.singles += alloc;
                let c = w.dirs[di].cat.idx();
                w.dirs[di].own[c] += alloc;
                if alloc >= BIG_FILE {
                    w.dirs[di].big.push((name, alloc, false, md.dev(), md.ino()));
                }
            }
        }
    }
    // Decide each file with another name or a clone.
    let mut links: HashMap<(u64, u64), u64> = HashMap::new();
    let mut clones: HashMap<u64, u32> = HashMap::new();
    let mut seen_ino: HashSet<(u64, u64)> = HashSet::new();
    for m in &multis {
        *links.entry((m.dev, m.ino)).or_default() += 1;
        if m.refs > 1 && seen_ino.insert((m.dev, m.ino)) {
            *clones.entry(m.clone).or_default() += 1;
        }
    }
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut seen_clone: HashSet<u64> = HashSet::new();
    for m in multis {
        if !seen.insert((m.dev, m.ino)) {
            continue;
        }
        w.alloc += m.alloc;
        if m.refs > 1 && !seen_clone.insert(m.clone) {
            // A second clone of the same blocks inside this place costs nothing.
            continue;
        }
        let key = if m.refs > 1 { m.clone } else { (m.dev << 40) ^ m.ino ^ (1 << 63) };
        w.groups.entry(key).or_insert(m.alloc);
        let shared = links.get(&(m.dev, m.ino)).copied().unwrap_or(0) < m.nlink
            || (m.refs > 1 && clones.get(&m.clone).copied().unwrap_or(0) < m.refs);
        let d = &mut w.dirs[m.dir];
        if shared {
            d.shared += m.alloc;
        } else {
            d.own[d.cat.idx()] += m.alloc;
        }
        if m.alloc >= BIG_FILE {
            d.big.push((m.name, m.alloc, shared, m.dev, m.ino));
        }
    }
    w
}

/// Totals per directory, children included: (own by category, shared, files).
fn rollup(w: &Walk) -> Vec<([u64; 4], u64, u64)> {
    let mut t: Vec<([u64; 4], u64, u64)> = w.dirs.iter().map(|d| (d.own, d.shared, d.files)).collect();
    // Children always come after their parent in `dirs`.
    for i in (1..w.dirs.len()).rev() {
        let p = w.dirs[i].parent;
        let (o, s, f) = t[i];
        for c in 0..4 {
            t[p].0[c] += o[c];
        }
        t[p].1 += s;
        t[p].2 += f;
    }
    t
}

fn rel_path(w: &Walk, mut i: usize) -> String {
    let mut parts = Vec::new();
    while i != 0 {
        parts.push(w.dirs[i].name.as_str());
        i = w.dirs[i].parent;
    }
    parts.reverse();
    parts.join("/")
}

fn summarize(slug: &str, path: &str, is_main: bool, w: &Walk) -> PlaceUsage {
    let t = rollup(w);
    let mut cleanable: Vec<Cleanable> = Vec::new();
    for (i, d) in w.dirs.iter().enumerate().skip(1) {
        let pc = w.dirs[d.parent].cat;
        if (d.cat == Cat::Build || d.cat == Cat::Deps) && pc == Cat::Source {
            let (o, s, f) = t[i];
            let own: u64 = o.iter().sum();
            if own + s > 0 {
                cleanable.push(Cleanable { path: rel_path(w, i), cat: d.cat, own, shared: s, files: f, id: format!("{}:{}", d.dev, d.ino) });
            }
        }
    }
    cleanable.sort_by(|a, b| b.own.cmp(&a.own).then(a.path.cmp(&b.path)));
    cleanable.truncate(CLEANABLE_MAX);
    PlaceUsage {
        slug: slug.to_string(),
        path: path.to_string(),
        is_main,
        own: ByCat::from(&t[0].0),
        shared: t[0].1,
        alloc: w.alloc,
        files: w.files,
        errors: w.errors,
        cleanable,
    }
}

/// The pruned tree: a child smaller than `cut` of the place's total folds into
/// one aggregate per (category, shared) under its parent; at most `max_nodes`.
fn tree(w: &Walk, cut_frac: f64, max_nodes: usize) -> Node {
    let t = rollup(w);
    let total: u64 = t[0].0.iter().sum::<u64>() + t[0].1;
    let cut = ((total as f64) * cut_frac).max(1.0) as u64;
    let mut budget = max_nodes;
    fn size(t: &[([u64; 4], u64, u64)], i: usize) -> u64 {
        t[i].0.iter().sum::<u64>() + t[i].1
    }
    fn build(w: &Walk, t: &[([u64; 4], u64, u64)], i: usize, cut: u64, budget: &mut usize, path: String) -> Node {
        let d = &w.dirs[i];
        let mut kids: Vec<Node> = Vec::new();
        // Small entries, folded: own bytes by category, shared by category.
        let mut small_own = [0u64; 4];
        let mut small_shared = [0u64; 4];
        let mut small_files = [0u64; 4];
        let mut order: Vec<usize> = d.kids.clone();
        order.sort_by_key(|&k| std::cmp::Reverse(size(t, k)));
        for k in order {
            let s = size(t, k);
            if s >= cut && *budget > 0 {
                *budget -= 1;
                let p = if path.is_empty() { w.dirs[k].name.clone() } else { format!("{path}/{}", w.dirs[k].name) };
                kids.push(build(w, t, k, cut, budget, p));
            } else {
                for c in 0..4 {
                    small_own[c] += t[k].0[c];
                }
                small_shared[w.dirs[k].cat.idx()] += t[k].1;
                small_files[w.dirs[k].cat.idx()] += t[k].2;
            }
        }
        let mut big_bytes_own = 0u64;
        let mut big_bytes_shared = 0u64;
        for (name, alloc, shared, dev, ino) in &d.big {
            if *alloc >= cut && *budget > 0 {
                *budget -= 1;
                let p = if path.is_empty() { name.clone() } else { format!("{path}/{name}") };
                let (own, sh) = if *shared { (0, *alloc) } else { (*alloc, 0) };
                if *shared {
                    big_bytes_shared += alloc;
                } else {
                    big_bytes_own += alloc;
                }
                kids.push(Node { n: name.clone(), p: Some(p), own, shared: sh, files: 1, c: d.cat, dir: false, id: Some(format!("{dev}:{ino}")), k: Vec::new() });
            }
        }
        // This folder's own loose files.
        let c = d.cat.idx();
        let loose_own = d.own[c].saturating_sub(big_bytes_own);
        let loose_shared = d.shared.saturating_sub(big_bytes_shared);
        small_own[c] += loose_own;
        small_shared[c] += loose_shared;
        small_files[c] += d.files.saturating_sub(kids.iter().filter(|n| !n.dir).count() as u64);
        for cat in Cat::ALL {
            let ci = cat.idx();
            if small_own[ci] > 0 {
                kids.push(Node { n: String::new(), p: None, own: small_own[ci], shared: 0, files: small_files[ci], c: cat, dir: false, id: None, k: Vec::new() });
                small_files[ci] = 0;
            }
            if small_shared[ci] > 0 {
                kids.push(Node { n: String::new(), p: None, own: 0, shared: small_shared[ci], files: small_files[ci], c: cat, dir: false, id: None, k: Vec::new() });
            }
        }
        kids.sort_by_key(|n| std::cmp::Reverse(n.own + n.shared));
        let own: u64 = t[i].0.iter().sum();
        Node {
            n: d.name.clone(),
            p: if i == 0 { None } else { Some(path) },
            own,
            shared: t[i].1,
            files: t[i].2,
            c: d.cat,
            dir: true,
            id: Some(format!("{}:{}", d.dev, d.ino)),
            k: kids,
        }
    }
    build(w, &t, 0, cut, &mut budget, String::new())
}

// ── projects, the cache ───────────────────────────────────────────────────

fn volume(path: &Path) -> Volume {
    let mut v = Volume::default();
    if let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0 {
            v.free = s.f_bavail as u64 * s.f_frsize as u64;
            v.total = s.f_blocks as u64 * s.f_frsize as u64;
        }
    }
    v.snapshots = snapshots();
    v
}

#[cfg(target_os = "macos")]
fn snapshots() -> Option<u32> {
    let out = crate::childenv::command("tmutil").args(["listlocalsnapshots", "/"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains("com.apple.TimeMachine")).count() as u32)
}

#[cfg(not(target_os = "macos"))]
fn snapshots() -> Option<u32> {
    None
}

/// Scan every place of `project`, four at a time.
pub fn scan_project(project: &Project) -> ProjectUsage {
    let start = Instant::now();
    let places = project.place_index();
    let results: Mutex<Vec<(usize, PlaceUsage, HashMap<u64, u64>, u64)>> = Mutex::new(Vec::new());
    let next = Mutex::new(0usize);
    std::thread::scope(|s| {
        for _ in 0..4.min(places.len().max(1)) {
            s.spawn(|| loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    let i = *n;
                    *n += 1;
                    i
                };
                let Some(p) = places.get(i) else { break };
                let w = walk(Path::new(&p.path), p.is_main);
                let u = summarize(&p.slug, &p.path, p.is_main, &w);
                results.lock().unwrap().push((i, u, w.groups, w.singles));
            });
        }
    });
    let mut r = results.into_inner().unwrap();
    r.sort_by_key(|x| x.0);
    let mut groups: HashMap<u64, u64> = HashMap::new();
    let mut unique = 0u64;
    let mut out = Vec::new();
    for (_, u, g, singles) in r {
        unique += singles;
        for (k, v) in g {
            groups.entry(k).or_insert(v);
        }
        out.push(u);
    }
    unique += groups.values().sum::<u64>();
    ProjectUsage {
        root: project.main_root.clone(),
        scanned_epoch: crate::sysclock::now_epoch(),
        scan_ms: start.elapsed().as_millis() as u64,
        places: out,
        unique,
        volume: volume(Path::new(&project.main_root)),
    }
}

/// One place's tree for drawing, and its summary.
#[derive(Serialize, Clone, Debug)]
pub struct PlaceTree {
    pub usage: PlaceUsage,
    pub tree: Node,
    pub scanned_epoch: i64,
    pub scan_ms: u64,
    pub volume: Volume,
}

/// Fold children under 0.15 % of the place, at most 2500 nodes.
pub fn scan_place_tree(place_root: &Path, slug: &str, is_main: bool) -> PlaceTree {
    let start = Instant::now();
    let w = walk(place_root, is_main);
    let usage = summarize(slug, &place_root.to_string_lossy(), is_main, &w);
    let tree = tree(&w, 0.0015, 2500);
    PlaceTree {
        usage,
        tree,
        scanned_epoch: crate::sysclock::now_epoch(),
        scan_ms: start.elapsed().as_millis() as u64,
        volume: volume(place_root),
    }
}

type Slot = Arc<Mutex<Option<Arc<ProjectUsage>>>>;
static CACHE: Mutex<Option<HashMap<String, Slot>>> = Mutex::new(None);

fn slot(root: &str) -> Slot {
    let mut c = CACHE.lock().unwrap();
    c.get_or_insert_with(HashMap::new).entry(root.to_string()).or_default().clone()
}

/// A project's usage: the cached scan, or a new one when there is none or
/// `refresh`. Two callers asking at once share one scan (the slot's lock).
pub fn project_usage(project: &Project, refresh: bool) -> Arc<ProjectUsage> {
    let s = slot(&project.main_root);
    let mut g = s.lock().unwrap();
    if let (Some(u), false) = (g.as_ref(), refresh) {
        return u.clone();
    }
    let u = Arc::new(scan_project(project));
    *g = Some(u.clone());
    u
}

/// The cached scan only, never a new one.
pub fn cached_usage(root: &str) -> Option<Arc<ProjectUsage>> {
    let s = slot(root);
    let g = s.lock().ok()?;
    g.clone()
}

fn forget(root: &str) {
    if let Some(c) = CACHE.lock().unwrap().as_mut() {
        c.remove(root);
    }
}

// ── deleting ──────────────────────────────────────────────────────────────

/// What a delete did.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Deleted {
    pub path: String,
    /// Allocated bytes under the path before it went (every name once).
    pub bytes: u64,
    pub files: u64,
    pub dir: bool,
}

/// A [`Node::id`] back into `(dev, ino)`.
pub fn parse_id(s: &str) -> Option<(u64, u64)> {
    let (d, i) = s.split_once(':')?;
    Some((d.parse().ok()?, i.parse().ok()?))
}

fn lexically_ok(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains('\0')
        && !rel.contains('\\')
        && rel.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// Why `rel` may not be deleted from the place at `root`, or `None`.
/// Rules, in order: a plain relative path; never `.git`; never `(main)`'s
/// `.worktrees/`; no symlink on the way (every component lstat'd, the last
/// too); the scan's identity when given; nothing git tracks inside; git
/// ignores it.
pub fn delete_refusal(root: &Path, is_main: bool, rel: &str, expect: Option<(u64, u64)>) -> Option<String> {
    if !lexically_ok(rel) {
        return Some(format!("not a path inside the place: {rel:?}"));
    }
    let first = rel.split('/').next().unwrap_or("");
    if first.eq_ignore_ascii_case(".git") {
        return Some("git's own folder is never deleted here".into());
    }
    if is_main && first.eq_ignore_ascii_case(".worktrees") {
        return Some(".worktrees/ holds the other places: remove a place with Remove, not here".into());
    }
    let mut cur = root.to_path_buf();
    for c in rel.split('/') {
        cur.push(c);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return Some(format!("{rel} goes through a link; nothing is deleted through one")),
            Ok(_) => {}
            Err(_) => return Some(format!("{rel} is not there any more; rescan")),
        }
    }
    let md = fs::symlink_metadata(&cur).ok()?;
    if let Some((dev, ino)) = expect {
        if md.dev() != dev || md.ino() != ino {
            return Some(format!("{rel} changed since the scan; rescan"));
        }
    }
    // Containment by identity, for a case-insensitive volume: the canonical
    // target must sit under the canonical root, and not in (main)'s .worktrees.
    let (Ok(croot), Ok(ccur)) = (fs::canonicalize(root), fs::canonicalize(&cur)) else {
        return Some(format!("{rel} cannot be resolved"));
    };
    if !ccur.starts_with(&croot) || ccur == croot {
        return Some(format!("{rel} is outside the place"));
    }
    if is_main && ccur.starts_with(croot.join(".worktrees")) {
        return Some(".worktrees/ holds the other places: remove a place with Remove, not here".into());
    }
    let dir = root.to_string_lossy();
    let pathspec = format!(":(literal){rel}");
    match crate::git::git(&dir, &["ls-files", "-z", "--cached", "--", &pathspec]) {
        Ok(o) if o.status.success() => {
            if !o.stdout.is_empty() {
                return Some(format!("git tracks files in {rel}; change them with git, not here"));
            }
        }
        _ => return Some(format!("git could not say what it tracks in {rel}")),
    }
    match crate::git::git(&dir, &["check-ignore", "-q", "--", rel]) {
        Ok(o) if o.status.code() == Some(0) => None,
        Ok(o) if o.status.code() == Some(1) => Some(format!("git does not ignore {rel}; only ignored files are deleted here")),
        _ => Some(format!("git could not say whether it ignores {rel}")),
    }
}

fn measure(p: &Path) -> (u64, u64) {
    let Ok(md) = fs::symlink_metadata(p) else { return (0, 0) };
    if !md.is_dir() {
        return (md.blocks() * 512, 1);
    }
    let mut seen = HashSet::new();
    let (mut b, mut f) = (0u64, 0u64);
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = fs::symlink_metadata(e.path()) else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                f += 1;
                if seen.insert((m.dev(), m.ino())) {
                    b += m.blocks() * 512;
                }
            }
        }
    }
    (b, f)
}

/// Open the directory `rel`'s parent names under `root`, one component at a
/// time with `O_NOFOLLOW | O_DIRECTORY`, so no link on the way is followed
/// whatever happens to the paths after [`delete_refusal`] looked at them.
fn open_parent(root: &Path, rel: &str) -> Result<(std::os::fd::OwnedFd, std::ffi::CString), String> {
    use std::ffi::CString;
    use std::os::fd::{FromRawFd, OwnedFd, AsRawFd};
    use std::os::unix::ffi::OsStrExt;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let croot = CString::new(root.as_os_str().as_bytes()).map_err(|_| "a NUL in the place's path".to_string())?;
    let fd = unsafe { libc::open(croot.as_ptr(), flags) };
    if fd < 0 {
        return Err(format!("the place cannot be opened: {}", std::io::Error::last_os_error()));
    }
    let mut dir = unsafe { OwnedFd::from_raw_fd(fd) };
    let parts: Vec<&str> = rel.split('/').collect();
    for c in &parts[..parts.len() - 1] {
        let cc = CString::new(*c).map_err(|_| format!("a NUL in {rel:?}"))?;
        let fd = unsafe { libc::openat(dir.as_raw_fd(), cc.as_ptr(), flags) };
        if fd < 0 {
            return Err(format!("{rel} changed on the way (a folder became a link?); nothing was deleted"));
        }
        dir = unsafe { OwnedFd::from_raw_fd(fd) };
    }
    let last = CString::new(*parts.last().unwrap()).map_err(|_| format!("a NUL in {rel:?}"))?;
    Ok((dir, last))
}

fn fstatat_nofollow(dir: i32, name: &std::ffi::CStr) -> Option<libc::stat> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::fstatat(dir, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
    (r == 0).then_some(st)
}

/// Delete `rel` from the place at `root`, permanently, if [`delete_refusal`]
/// finds no reason not to. The project's cached scan is dropped.
///
/// The check reads paths; the act must not trust them, because whatever works
/// in the place (an agent, a build) can swap a folder for a link in between.
/// So the act walks to `rel`'s parent by handle (`open_parent`), checks the
/// object there is the one checked (dev, ino; never a link), and RENAMES it
/// into a fresh folder at the place's root, which is then removed with std's
/// `remove_dir_all` (it never follows a link, CVE-2022-21658). A swap after
/// the rename moves nothing outside the place.
pub fn delete_in_place(
    project_root: &str,
    root: &Path,
    is_main: bool,
    rel: &str,
    expect: Option<(u64, u64)>,
) -> Result<Deleted, String> {
    use std::os::fd::AsRawFd;
    if let Some(why) = delete_refusal(root, is_main, rel, expect) {
        return Err(why);
    }
    let checked = fs::symlink_metadata(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    let (dev, ino) = expect.unwrap_or((checked.dev(), checked.ino()));
    let (parent, name) = open_parent(root, rel)?;
    let st = fstatat_nofollow(parent.as_raw_fd(), &name).ok_or_else(|| format!("{rel} is not there any more; rescan"))?;
    let kind = st.st_mode as u32 & libc::S_IFMT as u32;
    if kind == libc::S_IFLNK as u32 || st.st_dev as u64 != dev || st.st_ino as u64 != ino {
        return Err(format!("{rel} changed since it was checked; nothing was deleted"));
    }
    let dir = kind == libc::S_IFDIR as u32;
    let (bytes, files) = measure(&root.join(rel));
    // A folder of our own at the place's root, made with mkdir (which never
    // follows or reuses anything already there) and a random name.
    let hold = root.join(format!(".worktrees-deleting-{}", crate::ulid::new()));
    fs::create_dir(&hold).map_err(|e| format!("could not make a folder to delete through: {e}"))?;
    let (hold_fd, _) = open_parent(root, &format!("{}/x", hold.file_name().unwrap().to_string_lossy()))?;
    let r = unsafe { libc::renameat(parent.as_raw_fd(), name.as_ptr(), hold_fd.as_raw_fd(), name.as_ptr()) };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        let _ = fs::remove_dir(&hold);
        return Err(format!("{rel}: {e}"));
    }
    let moved = fstatat_nofollow(hold_fd.as_raw_fd(), &name);
    if moved.map(|m| m.st_dev as u64 != dev || m.st_ino as u64 != ino).unwrap_or(true) {
        // Not the object checked: put it back and refuse.
        unsafe { libc::renameat(hold_fd.as_raw_fd(), name.as_ptr(), parent.as_raw_fd(), name.as_ptr()) };
        let _ = fs::remove_dir(&hold);
        return Err(format!("{rel} changed while it was being deleted; nothing was deleted"));
    }
    forget(project_root);
    fs::remove_dir_all(&hold).map_err(|e| format!("{rel} was moved to {} and could not be removed: {e}", hold.display()))?;
    Ok(Deleted { path: rel.to_string(), bytes, files, dir })
}

// ── the CLI ───────────────────────────────────────────────────────────────

fn human(b: u64) -> String {
    let g = b as f64 / 1e9;
    if g >= 100.0 {
        format!("{g:.0} GB")
    } else if g >= 1.0 {
        format!("{g:.1} GB")
    } else {
        format!("{:.0} MB", b as f64 / 1e6)
    }
}

/// `worktrees du [--json] [<place>]`: every place's own and shared bytes.
pub fn cmd_du(p: &Project, ui: &mut dyn crate::Ui, args: &[String]) -> i32 {
    let mut json = std::env::var("WORKTREES_JSON").ok().as_deref() == Some("1");
    let mut names: Vec<String> = Vec::new();
    let (opts, rest) = crate::argv::split_options(args);
    for a in opts {
        match a.as_str() {
            "--json" => json = true,
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => names.push(s.to_string()),
        }
    }
    names.extend(rest.iter().cloned());
    if names.len() > 1 {
        ui.error("usage: worktrees du [--json] [<place>]");
        return 1;
    }
    let u = scan_project(p);
    let places: Vec<&PlaceUsage> = match names.first() {
        None => u.places.iter().collect(),
        Some(n) => {
            let want = if n == "main" { "(main)".to_string() } else { n.clone() };
            let v: Vec<&PlaceUsage> = u.places.iter().filter(|x| x.slug == want).collect();
            if v.is_empty() {
                ui.error(&format!("No place '{n}'. See: worktrees ls"));
                return 1;
            }
            v
        }
    };
    if json {
        let mut v = serde_json::to_value(&u).unwrap_or_default();
        if names.first().is_some() {
            v["places"] = serde_json::to_value(&places).unwrap_or_default();
        }
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return 0;
    }
    let mut rows = places.clone();
    rows.sort_by_key(|x| std::cmp::Reverse(x.own.total()));
    let w = rows.iter().map(|x| x.slug.len()).max().unwrap_or(5).clamp(5, 40);
    println!("{:<w$}  {:>9}  {:>9}  {:>9}  {:>9}", "PLACE", "ONLY HERE", "SHARED", "ON DISK", "BUILD", w = w);
    for x in &rows {
        println!(
            "{:<w$}  {:>9}  {:>9}  {:>9}  {:>9}",
            x.slug,
            human(x.own.total()),
            human(x.shared),
            human(x.alloc),
            human(x.own.build),
            w = w
        );
    }
    let own: u64 = u.places.iter().map(|x| x.own.total()).sum();
    let alloc: u64 = u.places.iter().map(|x| x.alloc).sum();
    println!();
    println!(
        "{} places · counted once {} · du-style {} · scanned in {:.1} s",
        u.places.len(),
        human(u.unique),
        human(alloc),
        u.scan_ms as f64 / 1000.0
    );
    let _ = own;
    if let Some(n) = u.volume.snapshots.filter(|n| *n > 0) {
        println!("{n} Time Machine local snapshot(s): deleted files may free nothing until they expire.");
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-du-{tag}-{}", crate::ulid::new()));
        fs::create_dir_all(&d).unwrap();
        fs::canonicalize(&d).unwrap()
    }

    fn write(p: &Path, n: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![7u8; n]).unwrap();
    }

    fn git(d: &Path, args: &[&str]) {
        let s = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
        assert!(s.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&s.stderr));
    }

    fn repo(tag: &str) -> PathBuf {
        let d = tmp(tag);
        git(&d, &["init", "-q"]);
        git(&d, &["config", "user.email", "t@t"]);
        git(&d, &["config", "user.name", "t"]);
        write(&d.join(".gitignore"), 0);
        fs::write(d.join(".gitignore"), "node_modules/\ntarget\nscratch.log\n").unwrap();
        write(&d.join("src/main.rs"), 9000);
        write(&d.join("target/debug/big"), 200_000);
        write(&d.join("node_modules/a/index.js"), 50_000);
        write(&d.join("scratch.log"), 100);
        write(&d.join("notes.txt"), 100);
        git(&d, &["add", ".gitignore", "src"]);
        git(&d, &["commit", "-qm", "x"]);
        d
    }

    #[test]
    fn categories_follow_the_outermost_named_folder() {
        assert_eq!(cat_of(Cat::Source, "node_modules", 3), Cat::Deps);
        assert_eq!(cat_of(Cat::Source, "target", 1), Cat::Build);
        assert_eq!(cat_of(Cat::Deps, "src", 4), Cat::Deps);
        assert_eq!(cat_of(Cat::Source, ".git", 0), Cat::Git);
        assert_eq!(cat_of(Cat::Source, ".git", 2), Cat::Source);
        assert_eq!(cat_of(Cat::Source, "DerivedData-x", 2), Cat::Build);
    }

    #[test]
    fn a_hard_link_inside_the_place_is_own_and_counted_once() {
        let d = tmp("hl");
        write(&d.join("a"), 100_000);
        fs::hard_link(d.join("a"), d.join("b")).unwrap();
        let w = walk(&d, false);
        let u = summarize("x", "x", false, &w);
        let one = fs::metadata(d.join("a")).unwrap().blocks() * 512;
        assert_eq!(u.own.source, one, "one inode, two names: counted once");
        assert_eq!(u.shared, 0);
        assert_eq!(u.alloc, one, "du counts an inode once");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_hard_link_to_outside_the_place_is_shared() {
        let d = tmp("hlo");
        let out = tmp("hlo-out");
        write(&out.join("a"), 100_000);
        fs::hard_link(out.join("a"), d.join("b")).unwrap();
        let u = summarize("x", "x", false, &walk(&d, false));
        assert_eq!(u.own.total(), 0);
        assert!(u.shared > 0);
        fs::remove_dir_all(&d).unwrap();
        fs::remove_dir_all(&out).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_apfs_clone_outside_the_place_is_shared_and_inside_is_own() {
        let d = tmp("cl");
        let out = tmp("cl-out");
        write(&out.join("orig"), 400_000);
        let ok = Command::new("cp").args(["-c"]).arg(out.join("orig")).arg(d.join("copy")).status().unwrap().success();
        if !ok || clone_info(&d.join("copy")).map(|c| c.1).unwrap_or(0) < 2 {
            eprintln!("not an APFS volume that clones; skipped");
            return;
        }
        let u = summarize("x", "x", false, &walk(&d, false));
        assert_eq!(u.own.total(), 0, "its twin is outside: removing the place frees nothing");
        assert!(u.shared >= 400_000);
        // Both clones inside one place: own, and once.
        Command::new("cp").args(["-c"]).arg(out.join("orig")).arg(out.join("twin")).status().unwrap();
        fs::remove_file(out.join("orig")).unwrap();
        fs::rename(out.join("twin"), d.join("twin")).unwrap();
        let w = walk(&d, false);
        let u = summarize("x", "x", false, &w);
        let one = fs::metadata(d.join("copy")).unwrap().blocks() * 512;
        assert_eq!(u.own.source, one, "two clones of one file inside the place: own, counted once");
        assert_eq!(u.shared, 0);
        fs::remove_dir_all(&d).unwrap();
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn main_skips_its_worktrees_folder_and_lists_cleanable_folders() {
        let d = repo("main");
        write(&d.join(".worktrees/other/target/x"), 300_000);
        let u = summarize("(main)", "x", true, &walk(&d, true));
        assert!(u.own.build >= 200_000 && u.own.build < 300_000, "the other place's bytes are not main's: {:?}", u.own);
        assert!(u.own.deps >= 50_000);
        assert!(u.own.git > 0);
        let paths: Vec<&str> = u.cleanable.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["target", "node_modules"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_tree_folds_small_entries_and_keeps_paths_and_identity() {
        let d = repo("tree");
        let w = walk(&d, false);
        let t = tree(&w, 0.05, 100);
        let names: Vec<&str> = t.k.iter().map(|n| n.n.as_str()).collect();
        assert!(names.contains(&"target"), "{names:?}");
        let tg = t.k.iter().find(|n| n.n == "target").unwrap();
        assert_eq!(tg.p.as_deref(), Some("target"));
        assert_eq!(tg.c, Cat::Build);
        let md = fs::symlink_metadata(d.join("target")).unwrap();
        assert_eq!(tg.id, Some(format!("{}:{}", md.dev(), md.ino())));
        assert_eq!(parse_id(tg.id.as_deref().unwrap()), Some((md.dev(), md.ino())));
        let sum: u64 = t.k.iter().map(|n| n.own + n.shared).sum();
        assert_eq!(sum, t.own + t.shared, "children add up to the place");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn delete_takes_only_what_git_ignores() {
        let d = repo("del");
        let r = |rel: &str| delete_refusal(&d, false, rel, None);
        assert_eq!(r("target"), None);
        assert_eq!(r("node_modules/a"), None);
        assert_eq!(r("scratch.log"), None);
        assert!(r("src").unwrap().contains("git tracks"), "{:?}", r("src"));
        assert!(r("notes.txt").unwrap().contains("does not ignore"));
        assert!(r(".git").unwrap().contains("git's own"));
        assert!(r("../x").unwrap().contains("not a path"));
        assert!(r("/etc").unwrap().contains("not a path"));
        assert!(r("nope").unwrap().contains("not there"));
        let md = fs::symlink_metadata(d.join("target")).unwrap();
        assert!(delete_refusal(&d, false, "target", Some((md.dev(), md.ino() + 1))).unwrap().contains("changed since"));
        let done = delete_in_place("x", &d, false, "target", Some((md.dev(), md.ino()))).unwrap();
        assert!(done.dir && done.bytes >= 200_000 && done.files == 1, "{done:?}");
        assert!(!d.join("target").exists());
        assert!(d.join("src/main.rs").exists());
        let left: Vec<_> = fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().starts_with(".worktrees-deleting-")).collect();
        assert!(left.is_empty(), "the holding folder is removed: {left:?}");
        let f = delete_in_place("x", &d, false, "scratch.log", None).unwrap();
        assert!(!f.dir && !d.join("scratch.log").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    /// The Disk view keeps a scan until Rescan, so a delete it sends can name
    /// a folder that was replaced since, or a place that is gone. Both are
    /// refused by the scan's identity and by the path, with nothing deleted.
    #[test]
    fn a_folder_replaced_after_the_scan_is_refused_and_kept() {
        let d = repo("swap");
        let scan = summarize("x", &d.to_string_lossy(), false, &walk(&d, false));
        let id = scan.cleanable.iter().find(|c| c.path == "target").map(|c| parse_id(&c.id).unwrap()).unwrap();
        // A build that cleans and rebuilds: same name, a new folder.
        fs::remove_dir_all(d.join("target")).unwrap();
        write(&d.join("target/debug/fresh"), 1000);
        let e = delete_in_place("x", &d, false, "target", Some(id)).unwrap_err();
        assert!(e.contains("changed since the scan"), "{e}");
        assert!(d.join("target/debug/fresh").exists(), "the new folder is left alone");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_place_removed_after_the_scan_is_refused() {
        let d = repo("gone");
        let scan = summarize("x", &d.to_string_lossy(), false, &walk(&d, false));
        let id = scan.cleanable.iter().find(|c| c.path == "target").map(|c| parse_id(&c.id).unwrap()).unwrap();
        fs::remove_dir_all(&d).unwrap();
        let e = delete_in_place("x", &d, false, "target", Some(id)).unwrap_err();
        assert!(e.contains("not there any more"), "{e}");
        assert!(!d.exists(), "nothing was made where the place was");
    }

    #[test]
    fn delete_never_goes_through_a_link() {
        let d = repo("link");
        let out = tmp("link-out");
        write(&out.join("victim/f"), 10);
        std::os::unix::fs::symlink(out.join("victim"), d.join("node_modules/ln")).unwrap();
        let why = delete_refusal(&d, false, "node_modules/ln", None).unwrap();
        assert!(why.contains("link"), "{why}");
        std::os::unix::fs::symlink(&out, d.join("target/up")).unwrap();
        let why = delete_refusal(&d, false, "target/up/victim", None).unwrap();
        assert!(why.contains("link"), "{why}");
        // Deleting the folder that holds a link removes the link, never its target.
        delete_in_place("x", &d, false, "node_modules", None).unwrap();
        assert!(out.join("victim/f").exists());
        fs::remove_dir_all(&d).unwrap();
        fs::remove_dir_all(&out).unwrap();
    }

    /// The act walks by handle: a folder swapped for a link after the check
    /// stops the walk, so nothing past the link is reached.
    #[test]
    fn the_act_never_walks_through_a_link_that_appeared_after_the_check() {
        let d = repo("swap");
        let out = tmp("swap-out");
        write(&out.join("a/keep"), 10);
        assert!(open_parent(&d, "node_modules/a/index.js").is_ok());
        fs::rename(d.join("node_modules"), d.join("nm-real")).unwrap();
        std::os::unix::fs::symlink(&out, d.join("node_modules")).unwrap();
        let why = open_parent(&d, "node_modules/a/index.js").unwrap_err();
        assert!(why.contains("changed on the way"), "{why}");
        assert!(out.join("a/keep").exists());
        fs::remove_dir_all(&d).unwrap();
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn main_never_deletes_its_worktrees_folder() {
        let d = repo("mainwt");
        fs::write(d.join(".gitignore"), "node_modules/\ntarget\nscratch.log\n.worktrees/\n").unwrap();
        write(&d.join(".worktrees/other/f"), 10);
        assert!(delete_refusal(&d, true, ".worktrees", None).unwrap().contains("other places"));
        assert!(delete_refusal(&d, true, ".Worktrees/other", None).unwrap().contains("other places"));
        assert!(delete_refusal(&d, true, ".worktrees/other", None).unwrap().contains("other places"));
        fs::remove_dir_all(&d).unwrap();
    }
}
