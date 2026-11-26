//! A workspace's working memory: the files in `<workspace folder>/memory/`, and
//! their history in a git repository only the engine commits
//! (`docs/proposals/workspace-chats.md` §2, §2.5, revision 3's §9).
//!
//! **The repository is not in the folder.** It is the engine's, in
//! `.engine/<workspace id>/memory.git` (`wsfolder::engine_dir_at`), so nothing
//! in the tree a chat writes is git's config, hooks or metadata. A memory of
//! the earlier layout, whose repository was `memory/.git`, is moved on first
//! touch ([`migrate`]): its commits and nothing else of it.
//!
//! **Every act opens the folder once** ([`Place`]) and reaches everything from
//! that handle, git's own working directory included: what is put in the
//! folder's place meanwhile is never read, written or committed, and no link
//! is followed at any component. Whatever changes something holds the
//! workspace's one lock (`wslock`), which other engines hold too.
//!
//! `MEMORY.md` is the index, `notes/`, `topics/` and `files/` are a convention
//! the templates teach, and nothing here parses any of it. Agents and the
//! person write the files with whatever they like; this module lists, reads
//! and saves them, and keeps their history.
//!
//! **Every function takes the ROOT the workspaces' folders sit in and the
//! workspace's id** (`wsfolder`). Tests hand it a temp dir.
//!
//! **How git is called here, always** ([`Repo`]):
//! - by `--git-dir` and `--work-tree`, never by discovery: a memory with no
//!   repository would otherwise resolve UPWARD to whatever holds the data dir
//!   (a dotfiles one), and a commit would go there. The same two beat a
//!   `core.worktree` in the repository's config.
//! - with every `GIT_*` variable dropped from the child's environment, and
//!   with no global and no system config.
//! - with **nothing able to run**. The repository's config is the engine's own
//!   file; should someone have written in it, its effective config is listed
//!   first and each name in a family that runs a command is switched off by
//!   `-c` ([`neutralized`]): `core.hooksPath=/dev/null` is NOT enough, git 2.54
//!   runs `hook.<name>.command` from the config under it (measured), and
//!   `core.fsmonitor` and `filter.<name>.clean` run on a plain `add`.
//! - with `--` before every path a client supplied, and as a literal pathspec.
//! - with no transport: `GIT_ALLOW_PROTOCOL` names none, and nothing here ever
//!   adds a remote.
//!
//! **A save is not a commit** (§9 item 2). [`write_at`] only writes; the poll
//! commits a dirty tree after a quiet minute, and so does whatever starts a
//! chat, a fold or a revert ([`commit_if_dirty_at`]).
//!
//! **One set of files** ([`walk`]): what the list shows, the watch compares and
//! a commit holds. A regular file whose name a client can send ([`rel_ok`]), at
//! most [`MAX_DEPTH`] deep, no larger than [`MAX_COMMIT_BYTES`], in no folder
//! that holds a repository of its own. Anything else is named in `skipped` (a
//! dot name silently), and what the history already holds of it stays as it
//! was: it is neither changed nor removed. No `.gitignore` is asked: what the
//! app shows is in the history. A walk that stopped at [`MAX_ENTRIES`] says
//! nothing about what it did not reach, and a tracked path is removed only
//! when looking for that very path finds nothing ([`Presence`]).
//!
//! **Byte for byte**: no attribute of the folder's changes what is stored
//! ([`ATTRIBUTES`]).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::pindir::{Dir, Stat};
use crate::{fsx, safepath, wsfolder, wslock};

pub const DIR: &str = "memory";
/// The repository's folder, in the engine's half.
const GIT: &str = "memory.git";
pub const INDEX: &str = "MEMORY.md";
/// Where [`put_file_at`] puts a file.
pub const FILES: &str = "files";
/// The index's budget. Said, never enforced by cutting (§2.3).
pub const INDEX_BUDGET: u64 = 16 * 1024;
/// A file put into `files/`, and an image shown from here.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// A text file larger than this is neither read nor saved by the app.
pub const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
/// A file larger than this stays out of the history.
pub const MAX_COMMIT_BYTES: u64 = 16 * 1024 * 1024;
/// One file's patch is cut here, at a line.
pub const MAX_PATCH_BYTES: usize = 512 * 1024;
/// What one walk of the folder reads before it stops.
pub const MAX_ENTRIES: usize = 5000;
const MAX_DEPTH: usize = 8;
pub const LOG_PAGE: usize = 50;
const LOG_MAX: usize = 200;

const MADE: &str = "Memory made";
/// git's own words, which [`revert_at`] keeps in the commits it makes.
const REVERTS: &str = "This reverts commit ";
/// The trailer that names a chat, one a line, in a quiet-minute commit's body.
const LIVE: &str = "Live-chat: ";
/// The trailer that names the fold's attempt a commit is of.
const ATTEMPT: &str = "Fold-attempt: ";
/// The most chats one commit names.
const LIVE_MAX: usize = 16;

/// The refusals a client tells apart (`app/src/doc/memory.ts` mirrors them,
/// `memory-docs-check.mjs` holds the two equal).
pub const ERR_STALE: &str = "the file changed on disk since you opened it";
pub const ERR_GONE: &str = "no memory file";

// ── shapes ──────────────────────────────────────────────────────────────────

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Image,
    Other,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Relative to `memory/`, `/` between components.
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime_ms: u64,
    pub kind: Kind,
    /// A markdown file's first heading, from its first 4 KB.
    pub title: Option<String>,
}

/// The files under one top folder; `""` is `memory/` itself.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    pub files: Vec<File>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub why: &'static str,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    pub folders: Vec<Folder>,
    pub count: usize,
    pub bytes: u64,
    /// Present on disk and in neither the list nor the history, by name.
    pub skipped: Vec<Skipped>,
    /// The walk stopped at [`MAX_ENTRIES`].
    pub truncated: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub path: String,
    pub content: String,
    pub mtime_ms: u64,
    /// [`Doc::rev`].
    pub rev: String,
    pub size: u64,
    pub budget: u64,
    pub over_budget: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// `memory/` is there, with its repository.
    pub made: bool,
    /// What the next commit would hold.
    pub dirty: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub path: String,
    pub file: String,
    pub title: String,
    pub content: String,
    pub mtime_ms: u64,
    /// Which bytes these are: opaque, equal for equal bytes and for no others.
    /// What a save names its base by.
    pub rev: String,
    pub size: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: String,
    pub file: String,
    pub title: String,
    pub mtime_ms: u64,
    /// [`Doc::rev`], of what was written.
    pub rev: String,
    pub size: u64,
}

/// What a save says the file was when it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// There was none: the save MAKES the file.
    None,
    /// These bytes ([`Doc::rev`]).
    Rev(String),
    /// A file of this mtime. Two writes can share one: a client that knows
    /// `rev` sends that.
    Mtime(u64),
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// git's letter: A, M, D, T.
    pub status: String,
    /// `None` for a binary file.
    pub added: Option<u64>,
    pub removed: Option<u64>,
    pub binary: bool,
}

/// What made a commit, by its subject: the engine's own words are fixed, and
/// a fold's are `fold: <title>`.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Made,
    Fold,
    Edit,
    Undo,
    Redo,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub commit: String,
    /// Seconds since the epoch.
    pub at: i64,
    pub subject: String,
    pub kind: ChangeKind,
    pub files: Vec<FileChange>,
    pub added: u64,
    pub removed: u64,
    /// The commit this one undoes or redoes.
    pub reverts: Option<String>,
    /// The commit that undid this one, while that still stands.
    pub reverted_by: Option<String>,
    /// The chats that were live while what a quiet-minute commit holds was
    /// changed ([`commit_quiet_at`]). Live, not its author: nobody is told
    /// which of them, or which person, wrote it.
    pub live: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct Log {
    pub changes: Vec<Change>,
    /// There are older ones: ask again with `before` the last one's commit.
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub mime: &'static str,
    pub size: u64,
    pub bytes: Vec<u8>,
    /// Why it is not shown; `bytes` is empty then.
    pub refused: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDiff {
    /// A unified diff. `truncated`: cut at [`MAX_PATCH_BYTES`].
    Text { patch: String, truncated: bool },
    /// An image: each side there is, as it can be drawn.
    Image { before: Option<Blob>, after: Option<Blob> },
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub change: Change,
    /// The one file asked for.
    pub file: Option<(FileChange, FileDiff)>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Reverted {
    /// The new commit; `None` when nothing was changed.
    pub change: Option<Change>,
    /// The files that changed since, which the revert could not be applied to.
    pub conflicts: Vec<String>,
}

// ── paths ───────────────────────────────────────────────────────────────────

/// Workspace `id`'s memory folder.
pub fn dir_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    Ok(wsfolder::folder_at(root, id)?.join(DIR))
}

/// Where workspace `id`'s repository is.
pub fn git_dir_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    Ok(wsfolder::engine_dir_at(root, id)?.join(GIT))
}

/// One memory, open: its folder by a handle, its repository by the path the
/// engine alone writes under.
struct Place {
    mem: Dir,
    git: PathBuf,
}

/// Workspace `id`'s memory, its folder and no link, one of the earlier layout
/// moved first. `None`: it has none.
fn place(root: &Path, id: &str) -> Result<Option<Place>, String> {
    let Some(mem) = folder(root, id)? else { return Ok(None) };
    let git = git_dir_at(&wsfolder::resolved(root), id)?;
    if !engine_git(root, id) {
        if !earlier(root, id, &mem) {
            return Ok(None);
        }
        migrate(root, id, &mem)?;
    }
    Ok(Some(Place { mem, git }))
}

fn made(root: &Path, id: &str) -> Result<Place, String> {
    place(root, id)?.ok_or_else(|| "this workspace has no memory yet".into())
}

/// The engine's repository of workspace `id` is there, with no link on the way.
fn engine_git(root: &Path, id: &str) -> bool {
    half(root, id).and_then(|d| d.sub(GIT)).is_ok()
}

/// The string half of a path a client sent: relative, plain components, and
/// none that starts with a dot. That last is how `.git` is out of reach, with
/// `.gitattributes` and `.gitignore`, and the temps of `fsx::write_atomic`.
fn rel_ok(path: &str) -> Result<(), String> {
    let bad = path.is_empty()
        || path.len() > 1024
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(|c| c.is_control() || crate::harness::invisible(c))
        || path.split('/').any(|p| p.is_empty() || p.starts_with('.'));
    if bad {
        return Err(format!("not a memory path: {path:?}"));
    }
    Ok(())
}

/// The folder `path` is in, and its name there. Nothing is made.
fn locate(mem: &Dir, path: &str) -> Result<(Dir, String), String> {
    rel_ok(path)?;
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    Ok((mem.under(dir).map_err(|e| absent(path, &e))?, name.to_string()))
}

fn absent(path: &str, e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound { format!("{ERR_GONE}: {path}") } else { format!("not a file of this memory: {path}") }
}

/// The folder `path` is in, the missing ones made.
fn parent_of(mem: &Dir, path: &str) -> Result<(Dir, String), String> {
    rel_ok(path)?;
    let mut parts: Vec<&str> = path.split('/').collect();
    let name = parts.pop().unwrap_or(path).to_string();
    let at = mem.under("").and_then(|m| parts.iter().try_fold(m, |d, p| d.make(p)));
    Ok((at.map_err(|e| format!("{path}: {e}"))?, name))
}

/// [`Doc::rev`]: 128 bits of the bytes' SHA-256.
pub fn rev_of(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn ext_of(path: &str) -> String {
    name_of(path).rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

fn mime_of(path: &str) -> Option<&'static str> {
    let ext = ext_of(path);
    safepath::IMAGE_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m)
}

fn kind_of(path: &str) -> Kind {
    if mime_of(path).is_some() {
        return Kind::Image;
    }
    match ext_of(path).as_str() {
        "md" | "markdown" | "txt" | "csv" | "json" | "yaml" | "yml" | "toml" => Kind::Text,
        _ => Kind::Other,
    }
}

fn title_for(path: &str, text: &str) -> String {
    crate::ideas::title_of(text).unwrap_or_else(|| name_of(path))
}

// ── the walk ────────────────────────────────────────────────────────────────

struct Found {
    path: String,
    sig: Sig,
}

fn sig_of(m: &Stat) -> Sig {
    Sig { mtime_ms: m.mtime_ms, size: m.size, changed_ns: m.changed_ns, ino: m.ino }
}

#[derive(Default)]
struct Walk {
    files: Vec<Found>,
    skipped: Vec<Skipped>,
    truncated: bool,
}

/// Whether the memory keeps a file of this name, by the name alone.
fn name_kept(path: &str) -> bool {
    rel_ok(path).is_ok() && path.split('/').count() <= MAX_DEPTH
}

/// Every file the memory keeps: one `readdir` a folder and one `lstat` an
/// entry, no link followed, no dot name entered, no `.git` at any depth. By
/// name, a folder's files before what is under it, so that a walk that stops
/// at the limit stops at the same place each time.
fn walk(mem: &Dir) -> std::io::Result<Walk> {
    let mut out = Walk::default();
    let mut seen = 0usize;
    let mut todo = VecDeque::from([(String::new(), 0usize)]);
    while let Some((rel, depth)) = todo.pop_front() {
        let (dir, entries) = match mem.under(&rel).and_then(|d| Ok((d.names()?, d))) {
            Ok((names, d)) => (d, names),
            // The folder itself must be readable, or an unreadable one would
            // read as "everything was removed".
            Err(e) if rel.is_empty() => return Err(e),
            Err(_) => {
                out.skipped.push(Skipped { path: rel, why: "cannot be read" });
                continue;
            }
        };
        if !rel.is_empty() && entries.iter().any(|n| n.eq_ignore_ascii_case(".git")) {
            out.skipped.push(Skipped { path: rel, why: "holds a git repository of its own" });
            continue;
        }
        for name in entries {
            if name.as_bytes().starts_with(b".") {
                continue;
            }
            seen += 1;
            if seen > MAX_ENTRIES {
                out.truncated = true;
                return Ok(out);
            }
            let shown = name.to_string_lossy();
            let path = if rel.is_empty() { shown.to_string() } else { format!("{rel}/{shown}") };
            let Some(name) = name.to_str() else {
                out.skipped.push(Skipped { path, why: "its name is not UTF-8" });
                continue;
            };
            if rel_ok(&path).is_err() {
                out.skipped.push(Skipped { path, why: "its name cannot be used here" });
                continue;
            }
            let Ok(md) = dir.stat(name) else { continue };
            if md.is_dir() {
                if depth + 1 >= MAX_DEPTH {
                    out.skipped.push(Skipped { path, why: "too deep" });
                } else {
                    todo.push_back((path, depth + 1));
                }
            } else if !md.is_file() {
                out.skipped.push(Skipped { path, why: "not a regular file" });
            } else if md.size > MAX_COMMIT_BYTES {
                out.skipped.push(Skipped { path, why: "too large for the history" });
            } else {
                out.files.push(Found { path, sig: sig_of(&md) });
            }
        }
    }
    Ok(out)
}

/// What looking for one path finds.
#[derive(Debug, PartialEq, Eq)]
enum Presence {
    /// Nothing there by that name: it is no file, and no folder on the way to
    /// it is either.
    Gone,
    /// A regular file.
    File { ino: u64, links: u64 },
    /// Something else, or it cannot be told.
    Other,
}

fn presence(mem: &Dir, path: &str) -> Presence {
    let said = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => Presence::Gone,
        _ => Presence::Other,
    };
    let Ok(mut cur) = mem.under("") else { return Presence::Other };
    let mut parts = path.split('/').peekable();
    while let Some(p) = parts.next() {
        let m = match cur.stat(p) {
            Ok(m) => m,
            Err(e) => return said(e),
        };
        match (parts.peek().is_none(), m.is_dir(), m.is_file()) {
            (false, true, _) => match cur.sub(p) {
                Ok(d) => cur = d,
                Err(e) => return said(e),
            },
            // A folder where the file was, or a file where its folder was.
            (true, true, _) | (false, _, true) => return Presence::Gone,
            (true, _, true) => return Presence::File { ino: m.ino, links: m.links },
            _ => return Presence::Other,
        }
    }
    Presence::Other
}

/// What a tick compares of one file. The mtime alone would miss a second write
/// in its millisecond, and a tool that puts the old one back; the size with it,
/// one that leaves the size too. Nothing but a write moves the change time,
/// and a file put in another's place is another file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sig {
    pub mtime_ms: u64,
    pub size: u64,
    pub changed_ns: i128,
    pub ino: u64,
}

/// What one walk saw. A missing folder is no files; a folder that cannot be
/// read is an error, never "all removed".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stamp {
    files: BTreeMap<String, Sig>,
    /// Passed over: what is at or under one of these is not known.
    skipped: Vec<String>,
    /// The walk stopped at the limit: nothing it did not reach is known.
    truncated: bool,
}

impl Stamp {
    pub fn of(files: impl IntoIterator<Item = (String, Sig)>) -> Stamp {
        Stamp { files: files.into_iter().collect(), ..Stamp::default() }
    }
    pub fn len(&self) -> usize {
        self.files.len()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
    fn knows(&self, path: &str) -> bool {
        let under = |s: &str| path.strip_prefix(s).is_some_and(|r| r.is_empty() || r.starts_with('/'));
        !self.truncated && !self.skipped.iter().any(|s| under(s))
    }
}

pub fn stamp_at(root: &Path, id: &str) -> Result<Stamp, String> {
    let Some(at) = folder(root, id)? else { return Ok(Stamp::default()) };
    let w = walk(&at).map_err(|e| format!("{}: {e}", dir_at(root, id).map(|d| d.display().to_string()).unwrap_or_default()))?;
    Ok(Stamp {
        files: w.files.into_iter().map(|f| (f.path, f.sig)).collect(),
        skipped: w.skipped.into_iter().map(|s| s.path).collect(),
        truncated: w.truncated,
    })
}

/// The files' folder alone, for what asks nothing of the history. `None`: it
/// is not there.
fn folder(root: &Path, id: &str) -> Result<Option<Dir>, String> {
    let at = dir_at(root, id)?;
    match Dir::open(&wsfolder::folder_at(root, id)?).and_then(|f| f.sub(DIR)) {
        Ok(d) => Ok(Some(d)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", at.display())),
    }
}

/// What moved since `before`: `(what to compare with next, changed, removed)`.
/// A file this walk could not answer for is kept as it was, and neither.
pub fn advance(before: &Stamp, mut now: Stamp) -> (Stamp, Vec<(String, Sig)>, Vec<String>) {
    let changed = now.files.iter().filter(|(p, s)| before.files.get(*p) != Some(s)).map(|(p, s)| (p.clone(), *s)).collect();
    let mut removed = Vec::new();
    for (p, s) in &before.files {
        if now.files.contains_key(p) {
            continue;
        }
        if now.knows(p) {
            removed.push(p.clone());
        } else {
            now.files.insert(p.clone(), *s);
        }
    }
    (now, changed, removed)
}

/// The workspaces under `root` that have a memory: one `readdir`, and a ULID
/// name, so `.removed/` is none of them.
pub fn ids_at(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root)
        .map(|r| r.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| crate::ulid::is_valid(n)).collect())
        .unwrap_or_default();
    v.retain(|id| is_made(root, id));
    v.sort();
    v
}

// ── git ─────────────────────────────────────────────────────────────────────

/// Workspace `id` has a memory: a real folder, and the engine's repository or
/// one of the earlier layout's still to move. A `.git` FILE or link there names
/// a repository somewhere else, and is none.
fn is_made(root: &Path, id: &str) -> bool {
    let Ok(Some(mem)) = folder(root, id) else { return false };
    engine_git(root, id) || earlier(root, id, &mem)
}

/// Its repository is the earlier layout's, or on its way from there: a move
/// that was cut off left it between the two.
fn earlier(root: &Path, id: &str, mem: &Dir) -> bool {
    mem.stat(".git").is_ok_and(|m| m.is_dir()) || half(root, id).is_ok_and(|d| d.stat(MOVING).is_ok_and(|m| m.is_dir()))
}

/// Workspace `id`'s engine half, through no link.
fn half(root: &Path, id: &str) -> std::io::Result<Dir> {
    Dir::open(&root.join(wsfolder::ENGINE_DIR))?.sub(id)
}

/// The `-c`s every call carries.
const FIXED: &[&str] = &[
    "core.hooksPath=/dev/null",
    "commit.gpgsign=false",
    "tag.gpgsign=false",
    "user.name=worktrees",
    "user.email=memory@localhost",
    "core.fsmonitor=false",
    "core.attributesFile=/dev/null",
    "core.excludesFile=/dev/null",
    "core.autocrlf=false",
    // A name as the folder spells it, and no mode but a plain file's.
    "core.precomposeUnicode=false",
    "core.fileMode=false",
    // Whatever git tidies, it tidies before it answers.
    "gc.autoDetach=false",
    "maintenance.autoDetach=false",
    "core.quotePath=false",
    "core.bare=false",
    "log.showSignature=false",
    "log.showRoot=true",
    "diff.ignoreSubmodules=all",
    "diff.noprefix=false",
    "diff.mnemonicPrefix=false",
    "submodule.recurse=false",
    "merge.conflictStyle=merge",
    "protocol.allow=never",
];

/// `info/attributes`, which outranks every `.gitattributes` of the folder: no
/// conversion, no filter, no driver, no encoding. What is stored is the bytes.
const ATTRIBUTES: &str = "* -text -filter -ident !eol !working-tree-encoding !merge !diff\n";

/// The edits a memory's git gets over the environment it would inherit: every
/// `GIT_*` variable goes (`GIT_DIR`, `GIT_INDEX_FILE`, `GIT_AUTHOR_NAME`,
/// `GIT_CONFIG_COUNT`… all of them redirect or reconfigure git), then ours.
/// `GIT_EXEC_PATH` stays: it is where this git's own programs are.
fn env_edits(inherited: impl Iterator<Item = String>) -> Vec<(String, Option<&'static str>)> {
    let mut v: Vec<(String, Option<&'static str>)> =
        inherited.filter(|k| k.starts_with("GIT_") && k != "GIT_EXEC_PATH").map(|k| (k, None)).collect();
    for k in ["GIT_DIR", "GIT_WORK_TREE"] {
        v.push((k.into(), None));
    }
    for (k, val) in [
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_ATTR_NOSYSTEM", "1"),
        ("GIT_ALLOW_PROTOCOL", "none"),
        ("GIT_NO_LAZY_FETCH", "1"),
        ("GIT_NO_REPLACE_OBJECTS", "1"),
        ("GIT_LITERAL_PATHSPECS", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_EDITOR", "true"),
        ("LC_ALL", "C"),
    ] {
        v.push((k.into(), Some(val)));
    }
    v
}

/// The `-c`s that switch off what a repository's config would run. `keys` is
/// its effective config (includes followed), by name. A subsection holding
/// `=` cannot be named in a `-c`, so it is refused.
fn neutralized(keys: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for k in keys {
        let Some((section, rest)) = k.split_once('.') else { continue };
        let Some((sub, var)) = rest.rsplit_once('.') else { continue };
        let set: &[&str] = match section.to_ascii_lowercase().as_str() {
            "hook" => &["enabled=false"],
            "filter" => &["clean=", "smudge=", "process=", "required=false"],
            "merge" if var.eq_ignore_ascii_case("driver") => &["driver="],
            "diff" if ["command", "textconv"].contains(&var.to_ascii_lowercase().as_str()) => &["command=", "textconv="],
            _ => continue,
        };
        if sub.contains('=') || sub.chars().any(char::is_control) {
            return Err(format!("the memory's .git/config names {section} {sub:?}, which worktrees will not run git with: remove it"));
        }
        for s in set {
            let c = format!("{section}.{sub}.{s}");
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    Ok(out)
}

fn answered(args: &[&str], o: Output) -> Result<Vec<u8>, String> {
    if !o.status.success() {
        let cmd = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"");
        return Err(format!("git {cmd} failed: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(o.stdout)
}

/// One memory's repository, opened: what its config would run is known and off.
struct Repo {
    place: Place,
    off: Vec<String>,
}

impl Repo {
    fn open(place: Place) -> Result<Repo, String> {
        let mut r = Repo { place, off: Vec::new() };
        let keys = r.text(&["config", "--list", "--name-only", "-z"])?;
        r.off = neutralized(&keys.split('\0').map(String::from).collect::<Vec<_>>())?;
        Ok(r)
    }

    fn command(&self) -> std::process::Command {
        let mut c = crate::childenv::command("git");
        for (k, v) in env_edits(std::env::vars_os().filter_map(|(k, _)| k.into_string().ok())) {
            match v {
                Some(v) => c.env(k, v),
                None => c.env_remove(k),
            };
        }
        let mem = self.place.mem.fd();
        // Into the folder that was opened, whatever its path names by now.
        // SAFETY: fchdir is async-signal-safe, and the handle outlives the
        // child's start: `self` holds it.
        unsafe {
            c.pre_exec(move || if libc::fchdir(mem) == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) });
        }
        c.arg(format!("--git-dir={}", self.place.git.display()));
        c.args(["--work-tree=.", "--no-pager", "--no-optional-locks"]);
        for f in FIXED.iter().copied().chain(self.off.iter().map(String::as_str)) {
            c.arg("-c").arg(f);
        }
        c.stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Result<Output, String> {
        self.command().args(args).output().map_err(|e| format!("git: {e}"))
    }

    /// With `input` for its stdin and, when given, an index of its own.
    fn run_in(&self, args: &[&str], input: &[u8], index: Option<&Path>) -> Result<Output, String> {
        let mut c = self.command();
        if let Some(i) = index {
            c.env("GIT_INDEX_FILE", i);
        }
        let mut child = c.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("git: {e}"))?;
        let mut pipe = child.stdin.take().expect("piped");
        // Beside the read: a git that answers before it has read it all would
        // otherwise wait for us while we wait for it.
        let wrote = std::thread::scope(|s| {
            let w = s.spawn(move || pipe.write_all(input));
            let o = child.wait_with_output();
            let _ = w.join();
            o
        });
        wrote.map_err(|e| format!("git: {e}"))
    }

    fn bytes(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        answered(args, self.run(args)?)
    }

    fn feed(&self, args: &[&str], input: &[u8], index: Option<&Path>) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&answered(args, self.run_in(args, input, index)?)?).into_owned())
    }

    fn text(&self, args: &[&str]) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.bytes(args)?).into_owned())
    }

    fn has_commits(&self) -> bool {
        head_id(&self.place.git).is_some()
    }

    /// A revert this engine began and did not finish (it was killed between
    /// the two). Under the lock nothing else can be in one.
    fn recover(&self) {
        if fs::symlink_metadata(self.place.git.join("REVERT_HEAD")).is_ok() {
            let _ = self.run(&["revert", "--abort"]);
        }
    }

    /// The tree of HEAD, or the empty one before the first commit.
    fn base(&self) -> Result<String, String> {
        if self.has_commits() {
            return Ok("HEAD".into());
        }
        Ok(self.feed(&["hash-object", "-t", "tree", "--stdin"], b"", None)?.trim().to_string())
    }

    /// Make the index HEAD plus what the memory keeps of the folder now, and
    /// answer the paths that differ. The lock is held. Whatever else the index
    /// held, and whatever was slipped in between the walk and the add (a link
    /// where a file was), is put back as HEAD has it.
    fn stage(&self) -> Result<Vec<String>, String> {
        let w = walk(&self.place.mem).map_err(|e| format!("the memory's folder: {e}"))?;
        let on: HashSet<&str> = w.files.iter().map(|f| f.path.as_str()).collect();
        let inos: HashSet<u64> = w.files.iter().map(|f| f.sig.ino).collect();
        // One more spelling of a file that is there under another: a disk that
        // folds case or accents finds it by the old name too.
        let gone = |p: &str| match presence(&self.place.mem, p) {
            Presence::Gone => true,
            Presence::File { ino, links } => links == 1 && inos.contains(&ino),
            Presence::Other => false,
        };
        let tracked = self.text(&["ls-files", "-z", "--cached"])?;
        let mut out = Vec::new();
        for p in tracked.split('\0').filter(|p| !p.is_empty() && name_kept(p) && !on.contains(p) && gone(p)) {
            out.extend_from_slice(p.as_bytes());
            out.push(0);
        }
        self.feed(&["update-index", "--force-remove", "-z", "--stdin"], &out, None)?;
        let add: Vec<u8> = w.files.iter().flat_map(|f| f.path.bytes().chain([0])).collect();
        self.feed(&["update-index", "--add", "--remove", "-z", "--stdin"], &add, None)?;
        let raw = self.text(&["diff-index", "--cached", "--raw", "-z", "--no-renames", "--abbrev=40", "--ignore-submodules=none", &self.base()?])?;
        let (mut dirty, mut back) = (Vec::new(), Vec::new());
        let mut fields = raw.split('\0');
        while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
            let m: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
            let [old_mode, new_mode, old, _, status] = m[..] else { continue };
            let ours = if status == "D" { name_kept(path) && gone(path) } else { new_mode.starts_with("100") && on.contains(path) };
            if ours {
                dirty.push(path.to_string());
            } else {
                back.extend_from_slice(format!("{old_mode} {old} 0\t{path}").as_bytes());
                back.push(0);
            }
        }
        if !back.is_empty() {
            self.feed(&["update-index", "--replace", "-z", "--index-info"], &back, None)?;
        }
        Ok(dirty)
    }

    /// What every act that writes needs in place.
    fn prepare(&self) -> Result<(), String> {
        let info = self.place.git.join("info");
        fsx::make_dir(&info).map_err(|e| format!("{}: {e}", info.display()))?;
        let at = info.join("attributes");
        if fs::read(&at).ok().as_deref() != Some(ATTRIBUTES.as_bytes()) {
            fsx::write_atomic(&at, ATTRIBUTES.as_bytes(), None).map_err(|e| format!("{}: {e}", at.display()))?;
        }
        self.recover();
        Ok(())
    }
}

/// Run `f` with workspace `id`'s memory held still: no commit, undo or save of
/// this module's is in it, or starts in it, meanwhile. What moves the folder
/// uses it, or a commit would be cut in half by the move.
pub fn while_still_at<T>(root: &Path, id: &str, f: impl FnOnce() -> T) -> Result<T, String> {
    let _held = if is_made(root, id) { Some(wslock::take(root, id)?) } else { None };
    Ok(f())
}

// ── the earlier layout ──────────────────────────────────────────────────────

/// What a repository of the engine's is configured with, whole.
const CONFIG: &str = "[core]\n\trepositoryformatversion = 0\n\tfilemode = false\n\tbare = true\n";

/// Move a memory of the earlier layout, whose repository is `memory/.git`, to
/// the engine's half: its commits, and nothing else of it. Its config, hooks,
/// attributes, alternates and index (whose flags hide a change from git) were
/// in a tree a chat writes and are left behind. Nothing is refused that has not
/// been put back: either the old
/// folder is where it was, or the new one is whole.
fn migrate(root: &Path, id: &str, mem: &Dir) -> Result<(), String> {
    wsfolder::refuse_real(root)?;
    let _held = wslock::take(root, id)?;
    if engine_git(root, id) {
        return Ok(());
    }
    let at = wsfolder::make_engine_at(root, id)?;
    let half = Dir::open(&at).map_err(|e| format!("{}: {e}", at.display()))?;
    let moving = at.join(MOVING);
    match (half.stat(MOVING).is_ok(), mem.stat(".git").is_ok()) {
        (true, true) => {
            return Err(format!("this memory has two histories, {} and memory/.git, and neither is touched: remove the one that is not its", moving.display()));
        }
        // A move that was cut off.
        (true, false) => {}
        (false, _) => mem.rename(".git", &half, MOVING).map_err(|e| format!("the memory's history could not be moved: {e}"))?,
    }
    finish_migration(root, id, mem, &half).map_err(|e| match half.rename(MOVING, mem, ".git") {
        Ok(()) => e,
        Err(back) => format!("{e}; and it could not be put back: {back}"),
    })
}

/// The earlier repository, between the two layouts.
const MOVING: &str = "memory.git.moving";

fn finish_migration(root: &Path, id: &str, mem: &Dir, half: &Dir) -> Result<(), String> {
    let at = wsfolder::engine_dir_at(&wsfolder::resolved(root), id)?.join(MOVING);
    adopt(&at)?;
    let place = Place { mem: mem.under("").map_err(|e| e.to_string())?, git: at.clone() };
    let repo = Repo { place, off: Vec::new() };
    if repo.text(&["fsck", "--connectivity-only", "--no-dangling"]).is_err() {
        return Err("the memory's history is not whole, and is left where it is".into());
    }
    // A HEAD that names no commit, over a history that has some.
    if !repo.has_commits() && !repo.text(&["for-each-ref", "--count=1"])?.is_empty() {
        return Err("the memory's history is on no branch, and is left where it is".into());
    }
    // An undo it was killed in: its half is taken back before its index goes.
    if fs::symlink_metadata(at.join("REVERT_HEAD")).is_ok() {
        repo.bytes(&["revert", "--abort"])?;
    }
    for gone in ["index", "REVERT_HEAD", "ORIG_HEAD", "MERGE_MSG", "sequencer"] {
        remove(&at.join(gone))?;
    }
    half.rename(MOVING, half, GIT).map_err(|e| format!("the memory's history could not be moved: {e}"))?;
    // Its lock, in the folder then.
    let _ = Dir::open(&wsfolder::folder_at(root, id)?).and_then(|d| d.unlink(".memory.lock"));
    Ok(())
}

fn remove(p: &Path) -> Result<(), String> {
    let done = match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(p),
        Ok(_) => fs::remove_file(p),
        Err(_) => Ok(()),
    };
    done.map_err(|e| format!("{}: {e}", p.display()))
}

/// Make `git`, a repository that was in a folder a chat writes, the engine's:
/// the objects, the refs and HEAD are kept, when they
/// are nothing but files and folders; an undo in progress and its index are,
/// for the caller to end; everything else is removed and the config written.
fn adopt(git: &Path) -> Result<(), String> {
    let refuse = |why: &str| Err(format!("the memory's history {why}, and is left where it is"));
    for part in ["objects", "refs", "HEAD", "packed-refs", "index", "REVERT_HEAD", "sequencer"] {
        if !only_files(&git.join(part)).map_err(|e| e.to_string())? {
            return refuse("holds something that is neither a file nor a folder");
        }
    }
    let old = fs::read_to_string(git.join("config")).unwrap_or_default();
    if old.to_ascii_lowercase().contains("[extensions") {
        return refuse("is of a format worktrees does not make");
    }
    let keep = ["objects", "refs", "HEAD", "packed-refs", "index", "REVERT_HEAD", "ORIG_HEAD", "MERGE_MSG", "sequencer"];
    for e in fs::read_dir(git).map_err(|e| e.to_string())?.flatten() {
        if !keep.iter().any(|k| e.file_name() == *k) {
            remove(&e.path())?;
        }
    }
    // Alternates, and a commit graph that would be believed over the commits.
    remove(&git.join("objects/info"))?;
    fsx::write_atomic(&git.join("config"), CONFIG.as_bytes(), None).map_err(|e| e.to_string())
}

fn only_files(p: &Path) -> std::io::Result<bool> {
    let m = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    if !m.is_dir() {
        return Ok(m.is_file());
    }
    for e in fs::read_dir(p)? {
        if !only_files(&e?.path())? {
            return Ok(false);
        }
    }
    Ok(true)
}

// ── make ────────────────────────────────────────────────────────────────────

fn template(name: &str) -> String {
    let name: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    format!(
        "# Memory: {}\n\n## What this workspace is for\n\n## Projects\n\n## Decisions\n\n## How we work here\n\n## Open\n",
        name.trim()
    )
}

/// Make workspace `id`'s memory, or finish making it. One already made is left
/// exactly as it is; one whose folder is gone is put back as its history has
/// it.
pub fn init_at(root: &Path, id: &str, name: &str) -> Result<Status, String> {
    wsfolder::refuse_real(root)?;
    let held = wslock::take(root, id)?;
    let top = wsfolder::make_at(root, id)?;
    let top = Dir::open(&top).map_err(|e| format!("{}: {e}", top.display()))?;
    let absent = top.stat(DIR).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    let mem = top.make(DIR).map_err(|e| format!("{DIR}: {e}"))?;
    let place = match place(root, id)? {
        Some(p) => p,
        None => {
            if mem.stat(".git").is_ok() {
                return Err("the memory's .git is not a folder".into());
            }
            found(&wsfolder::make_engine_at(root, id)?.join(GIT))?;
            made(root, id)?
        }
    };
    let repo = Repo::open(place)?;
    let first = !repo.has_commits();
    if first || absent {
        repo.prepare()?;
        for d in ["notes", "topics", FILES] {
            repo.place.mem.make(d).map_err(|e| format!("{d}: {e}"))?;
        }
    }
    if first {
        let mem = &repo.place.mem;
        if mem.stat(INDEX).is_err() {
            fsx::write_atomic_in(mem, INDEX, template(name).as_bytes(), None).map_err(|e| format!("{INDEX}: {e}"))?;
        }
        commit_held(&repo, Some(MADE))?;
    } else if absent {
        // The folder went and its history did not.
        repo.bytes(&["read-tree", "HEAD"])?;
        repo.bytes(&["checkout-index", "--all", "--quiet"])?;
    }
    drop(held);
    status_at(root, id)
}

/// An empty repository at `git`, as `git init --bare` with no template would
/// make it and with no one's defaults in it.
fn found(git: &Path) -> Result<(), String> {
    let said = |e: std::io::Error| format!("{}: {e}", git.display());
    for d in ["", "objects", "refs", "refs/heads"] {
        fsx::make_dir(&git.join(d)).map_err(said)?;
    }
    fsx::write_atomic(&git.join("config"), CONFIG.as_bytes(), None).map_err(said)?;
    fsx::write_atomic(&git.join("HEAD"), b"ref: refs/heads/main\n", None).map_err(said)
}

pub fn status_at(root: &Path, id: &str) -> Result<Status, String> {
    if !is_made(root, id) {
        dir_at(root, id)?;
        return Ok(Status { made: false, dirty: Vec::new() });
    }
    let _held = wslock::take(root, id)?;
    let repo = Repo::open(made(root, id)?)?;
    repo.prepare()?;
    Ok(Status { made: true, dirty: repo.stage()? })
}

// ── read ────────────────────────────────────────────────────────────────────

/// A markdown file's first heading, from its first 4 KB. A temp that vanished
/// is no title.
fn peek_title(mem: &Dir, path: &str) -> Option<String> {
    use std::io::Read;
    let (dir, name) = locate(mem, path).ok()?;
    let mut buf = Vec::new();
    dir.read(&name).ok()?.take(4096).read_to_end(&mut buf).ok()?;
    crate::ideas::title_of(&String::from_utf8_lossy(&buf))
}

/// Every file, folded by top folder: `memory/`'s own first (the index before
/// the rest), then `notes`, `topics`, `files`, then any other by name. Notes
/// come newest first, by their dated names.
pub fn list_at(root: &Path, id: &str) -> Result<Listing, String> {
    let Some(mem) = folder(root, id)? else { return Ok(Listing::default()) };
    let w = walk(&mem).map_err(|e| format!("{}: {e}", dir_at(root, id).map(|d| d.display().to_string()).unwrap_or_default()))?;
    let mut out = Listing { count: w.files.len(), truncated: w.truncated, skipped: w.skipped, ..Listing::default() };
    let mut by: BTreeMap<String, Vec<File>> = BTreeMap::new();
    for f in w.files {
        out.bytes += f.sig.size;
        let top = f.path.split_once('/').map(|(t, _)| t.to_string()).unwrap_or_default();
        let title = matches!(ext_of(&f.path).as_str(), "md" | "markdown").then(|| peek_title(&mem, &f.path)).flatten();
        by.entry(top).or_default().push(File { name: name_of(&f.path), kind: kind_of(&f.path), size: f.sig.size, mtime_ms: f.sig.mtime_ms, title, path: f.path });
    }
    let rank = |n: &str| ["", "notes", "topics", FILES].iter().position(|x| *x == n).unwrap_or(9);
    for (name, mut files) in by {
        files.sort_by(|a, b| (b.path == INDEX).cmp(&(a.path == INDEX)).then_with(|| a.path.cmp(&b.path)));
        if name == "notes" {
            files.reverse();
        }
        out.folders.push(Folder { name, files });
    }
    // This memory's repository is the engine's: one here is somebody's own, and
    // no part of it.
    if mem.stat(".git").is_ok() && engine_git(root, id) {
        out.skipped.push(Skipped { path: ".git".into(), why: "a git repository of its own" });
    }
    out.folders.sort_by(|a, b| rank(&a.name).cmp(&rank(&b.name)).then_with(|| a.name.cmp(&b.name)));
    out.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// `MEMORY.md`, or `None` when there is none (nobody is made to keep one).
pub fn index_at(root: &Path, id: &str) -> Result<Option<Index>, String> {
    match read_at(root, id, INDEX) {
        Ok(d) => Ok(Some(Index {
            path: d.path,
            content: d.content,
            mtime_ms: d.mtime_ms,
            rev: d.rev,
            size: d.size,
            budget: INDEX_BUDGET,
            over_budget: d.size > INDEX_BUDGET,
        })),
        Err(e) if e.starts_with(ERR_GONE) => Ok(None),
        Err(e) => Err(e),
    }
}

/// One file of the memory, read: how it was when it was opened, and its first
/// `cap` bytes. A regular file, reached through no link.
fn open_read(mem: &Dir, path: &str, cap: u64) -> Result<(Stat, Vec<u8>), String> {
    let (dir, name) = locate(mem, path)?;
    read_in(&dir, &name, path, cap)
}

fn read_in(dir: &Dir, name: &str, path: &str, cap: u64) -> Result<(Stat, Vec<u8>), String> {
    use std::io::Read;
    // Asked before it is opened: opening a device or a pipe is an act.
    if !dir.stat(name).map_err(|e| absent(path, &e))?.is_file() {
        return Err(format!("not a file of this memory: {path}"));
    }
    let h = dir.read(name).map_err(|e| absent(path, &e))?;
    let md = crate::pindir::stat_file(&h).map_err(|e| format!("{path}: {e}"))?;
    if !md.is_file() {
        return Err(format!("not a file of this memory: {path}"));
    }
    let mut buf = Vec::new();
    h.take(cap).read_to_end(&mut buf).map_err(|e| format!("{path}: {e}"))?;
    Ok((md, buf))
}

/// The files' folder, for a read: one that is not there is a file that is not.
fn opened(root: &Path, id: &str, path: &str) -> Result<Dir, String> {
    rel_ok(path)?;
    folder(root, id)?.ok_or_else(|| format!("{ERR_GONE}: {path}"))
}

fn as_text(path: &str, size: u64, bytes: Vec<u8>) -> Result<String, String> {
    if size > MAX_TEXT_BYTES {
        return Err(format!("{path} is too large to open here ({} KB)", size / 1024));
    }
    if bytes.contains(&0) {
        return Err(format!("{path} is not text"));
    }
    String::from_utf8(bytes).map_err(|_| format!("{path} is not UTF-8 text"))
}

pub fn read_at(root: &Path, id: &str, path: &str) -> Result<Doc, String> {
    let (md, bytes) = open_read(&opened(root, id, path)?, path, MAX_TEXT_BYTES + 1)?;
    let content = as_text(path, md.size, bytes)?;
    Ok(Doc { path: path.into(), file: name_of(path), title: title_for(path, &content), mtime_ms: md.mtime_ms, rev: rev_of(content.as_bytes()), size: md.size, content })
}

/// What is at `path` now, for whoever is told it moved: its mtime, its size,
/// and [`Doc::rev`] for a file small enough to open here.
pub fn seen_at(root: &Path, id: &str, path: &str) -> Result<(u64, u64, Option<String>), String> {
    let (md, bytes) = open_read(&opened(root, id, path)?, path, MAX_TEXT_BYTES + 1)?;
    let rev = (bytes.len() as u64 <= MAX_TEXT_BYTES).then(|| rev_of(&bytes));
    Ok((md.mtime_ms, md.size, rev))
}

/// `path` as it was in `commit`: what a restore puts back, and what an older
/// version reads as. `mtime_ms` is 0: it is not the file's.
pub fn read_commit_at(root: &Path, id: &str, path: &str, commit: &str) -> Result<Doc, String> {
    rel_ok(path)?;
    let repo = Repo::open(made(root, id)?)?;
    let commit = known(&repo, commit)?;
    let (size, bytes) = blob(&repo, &commit, path, MAX_TEXT_BYTES)?.ok_or_else(|| format!("{ERR_GONE}: {path} in that change"))?;
    let content = as_text(path, size, bytes)?;
    Ok(Doc { path: path.into(), file: name_of(path), title: title_for(path, &content), mtime_ms: 0, rev: rev_of(content.as_bytes()), size, content })
}

/// An image of this memory, under the chat images' rules
/// (`safepath::open_place_image`). `true`: `etag` still names it, no bytes.
pub fn image_at(root: &Path, id: &str, path: &str, etag: Option<&str>) -> Result<(safepath::ImageRead, bool), String> {
    let mem = opened(root, id, path)?;
    let mime = mime_of(path).ok_or_else(|| format!("not an image: {path}"))?;
    let cap = MAX_FILE_BYTES as u64;
    let (md, mut bytes) = open_read(&mem, path, cap + 1)?;
    let at = dir_at(root, id)?.join(path);
    let read = |bytes, truncated| safepath::ImageRead { path: at.clone(), bytes, truncated, on_disk: md.size, mtime_ms: md.mtime_ms, mime };
    let truncated = bytes.len() as u64 > cap;
    if etag == Some(format!("{}-{}", md.mtime_ms, md.size).as_str()) {
        return Ok((read(Vec::new(), truncated), true));
    }
    bytes.truncate(cap as usize);
    if truncated && mime == "image/svg+xml" {
        // Its tail was never looked at, and half an SVG is no picture.
        return Err(format!("not shown: an SVG over {cap} bytes is refused: {path}"));
    }
    safepath::check_image(&bytes, mime, path)?;
    Ok((read(bytes, truncated), false))
}

// ── write ───────────────────────────────────────────────────────────────────

/// The person's save, over the file `base` says they read: refused when that
/// is no longer it. `keep_history`: commit what is on disk at
/// `path` first, for a text written knowingly over one that came from
/// elsewhere. That one file and no other, so it takes nothing more of a fold
/// that is being written than what is about to be overwritten.
pub fn write_at(root: &Path, id: &str, path: &str, content: &str, base: &Base, keep_history: bool) -> Result<Saved, String> {
    write_with(root, id, path, content, base, keep_history, &|| ())
}

/// [`write_at`], with a test's say in what happens between the check and the
/// write.
fn write_with(root: &Path, id: &str, path: &str, content: &str, base: &Base, keep_history: bool, between: &dyn Fn()) -> Result<Saved, String> {
    wsfolder::refuse_real(root)?;
    rel_ok(path)?;
    if content.len() as u64 > MAX_TEXT_BYTES {
        return Err(format!("{path} is too large to save here"));
    }
    if kind_of(path) != Kind::Text {
        return Err(format!("{path} is not a text file"));
    }
    if !is_made(root, id) {
        return Err("this workspace has no memory yet".into());
    }
    let _held = wslock::take(root, id)?;
    let place = made(root, id)?;
    let mut was = None;
    let (dir, name) = match base {
        Base::Rev(_) | Base::Mtime(_) => {
            let (dir, name) = locate(&place.mem, path)?;
            let (md, bytes) = read_in(&dir, &name, path, MAX_TEXT_BYTES + 1)?;
            // Equal to what is there: nothing is written, and nothing is lost.
            if bytes == content.as_bytes() {
                return Ok(saved(path, content, &md));
            }
            let same = match base {
                Base::Rev(r) => *r == rev_of(&bytes),
                Base::Mtime(m) => *m == md.mtime_ms,
                Base::None => false,
            };
            if !same {
                return Err(format!("{ERR_STALE}: {path}"));
            }
            was = Some(bytes);
            (dir, name)
        }
        Base::None => {
            let (dir, name) = parent_of(&place.mem, path)?;
            if dir.stat(&name).is_ok() {
                return Err(format!("there is already a file at {path}"));
            }
            (dir, name)
        }
    };
    if let Some(bytes) = was {
        if keep_history {
            let repo = Repo::open(place)?;
            repo.prepare()?;
            preserve(&repo, path, &bytes, &format!("Before keeping your text: {path}"))?;
        }
        between();
        // What was read, and preserved, is what is about to go.
        if read_in(&dir, &name, path, MAX_TEXT_BYTES + 1)?.1 != bytes {
            return Err(format!("{ERR_STALE}: {path}"));
        }
    }
    let mode = dir.stat(&name).ok().map(|m| m.perm());
    fsx::write_atomic_in(&dir, &name, content.as_bytes(), mode).map_err(|e| format!("{path}: {e}"))?;
    let md = dir.stat(&name).map_err(|e| format!("{path}: {e}"))?;
    Ok(saved(path, content, &md))
}

/// Put `bytes` in the history as the file at `path`, and answer only when the
/// history gives them back. A commit of that path alone, made beside the
/// index: no ignore rule, no conversion and nothing staged has a part in it.
fn preserve(repo: &Repo, path: &str, bytes: &[u8], message: &str) -> Result<(), String> {
    let lost = || format!("the text on disk could not be put in the history, so nothing was replaced: {path}");
    if bytes.len() as u64 > MAX_TEXT_BYTES {
        return Err(lost());
    }
    let holds = |rev: &str| blob(repo, rev, path, MAX_TEXT_BYTES).map(|b| b.is_some_and(|(_, b)| b == bytes));
    if holds("HEAD")? {
        return Ok(());
    }
    let sha = repo.feed(&["hash-object", "-w", "--stdin"], bytes, None)?;
    let entry = format!("100644 {} 0\t{path}\0", sha.trim());
    let index = repo.place.git.join(format!("keep.{:016x}", fsx::random_u64()));
    let tree = (|| {
        repo.feed(&["read-tree", "HEAD"], b"", Some(&index))?;
        repo.feed(&["update-index", "--add", "--replace", "-z", "--index-info"], entry.as_bytes(), Some(&index))?;
        repo.feed(&["write-tree"], b"", Some(&index))
    })();
    let _ = fs::remove_file(&index);
    let head = repo.text(&["rev-parse", "--verify", "HEAD"])?;
    let commit = repo.feed(&["commit-tree", tree?.trim(), "-p", head.trim()], subject(message).as_bytes(), None)?;
    repo.bytes(&["update-ref", "HEAD", commit.trim(), head.trim()])?;
    repo.feed(&["update-index", "--add", "--replace", "-z", "--index-info"], entry.as_bytes(), None)?;
    if holds("HEAD")? { Ok(()) } else { Err(lost()) }
}

fn saved(path: &str, content: &str, md: &Stat) -> Saved {
    Saved { path: path.into(), file: name_of(path), title: title_for(path, content), mtime_ms: md.mtime_ms, rev: rev_of(content.as_bytes()), size: md.size }
}

/// Put a file under `files/`, under its own name or the next free one
/// (`rack-2.png`). `name` is a plain file name.
pub fn put_file_at(root: &Path, id: &str, name: &str, bytes: &[u8]) -> Result<Saved, String> {
    wsfolder::refuse_real(root)?;
    if name.contains('/') {
        return Err(format!("not a file name: {name:?}"));
    }
    rel_ok(name).map_err(|_| format!("not a file name: {name:?}"))?;
    if name.len() > 200 {
        return Err("the file's name is too long".into());
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("{name} is {} KB, over the {} MB a memory takes", bytes.len() / 1024, MAX_FILE_BYTES / 1024 / 1024));
    }
    if let Some(mime) = mime_of(name) {
        safepath::check_image(bytes, mime, name)?;
    }
    if !is_made(root, id) {
        return Err("this workspace has no memory yet".into());
    }
    let _held = wslock::take(root, id)?;
    let (dir, _) = parent_of(&made(root, id)?.mem, &format!("{FILES}/{name}"))?;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (name, String::new()),
    };
    let free = std::iter::once(name.to_string())
        .chain((2..1000).map(|n| format!("{stem}-{n}{ext}")))
        .find(|n| dir.stat(n).is_err())
        .ok_or_else(|| format!("too many files named {name}"))?;
    fsx::write_atomic_in(&dir, &free, bytes, None).map_err(|e| format!("{free}: {e}"))?;
    let md = dir.stat(&free).map_err(|e| format!("{free}: {e}"))?;
    Ok(Saved { path: format!("{FILES}/{free}"), file: free.clone(), title: free, mtime_ms: md.mtime_ms, rev: rev_of(bytes), size: md.size })
}

// ── commit ──────────────────────────────────────────────────────────────────

/// A commit's subject: one line, no control character, bounded.
fn subject(message: &str) -> String {
    let s: String = message.chars().map(|c| if c.is_control() { ' ' } else { c }).take(200).collect();
    s.trim().to_string()
}

fn edited(paths: &[String]) -> String {
    let shown: Vec<&str> = paths.iter().take(3).map(String::as_str).collect();
    let more = paths.len().saturating_sub(3);
    format!("Edited: {}{}", shown.join(", "), if more > 0 { format!(" and {more} more") } else { String::new() })
}

/// Commit whatever is dirty, or answer `None`. The lock is held.
fn commit_held(repo: &Repo, message: Option<&str>) -> Result<Option<Change>, String> {
    commit_with(repo, message, &[])
}

/// With a trailer a line, in the body.
fn commit_with(repo: &Repo, message: Option<&str>, trailers: &[String]) -> Result<Option<Change>, String> {
    let dirty = repo.stage()?;
    if dirty.is_empty() {
        return Ok(None);
    }
    let msg = message.map(subject).filter(|m| !m.is_empty()).unwrap_or_else(|| subject(&edited(&dirty)));
    let msg = format!("--message={msg}");
    let body = format!("--message={}", trailers.join("\n"));
    let mut args = vec!["commit", "--quiet", "--no-verify", "--cleanup=whitespace", &msg];
    if !trailers.is_empty() {
        args.push(&body);
    }
    repo.bytes(&args)?;
    head(repo)
}

fn head(repo: &Repo) -> Result<Option<Change>, String> {
    Ok(log_in(repo, None, None, 1)?.changes.pop())
}

/// The commit HEAD is, read from the repository's files: no process, for a
/// poll. `None` before the first commit, or with no memory.
pub fn head_at(root: &Path, id: &str) -> Option<String> {
    head_id(&git_dir_at(root, id).ok()?)
}

fn head_id(git: &Path) -> Option<String> {
    let text = |p: PathBuf| fs::read_to_string(p).ok();
    let head = text(git.join("HEAD"))?;
    let Some(name) = head.trim().strip_prefix("ref: ") else {
        return Some(head.trim().to_string()).filter(|h| is_sha(h));
    };
    if name.split('/').any(|p| p.is_empty() || p.starts_with('.')) {
        return None;
    }
    let loose = text(git.join(name)).map(|t| t.trim().to_string()).filter(|h| is_sha(h));
    loose.or_else(|| text(git.join("packed-refs"))?.lines().find_map(|l| l.split_once(' ').filter(|(_, n)| *n == name).map(|(h, _)| h.to_string())).filter(|h| is_sha(h)))
}

/// Workspace `id`'s repository under its lock and ready to write, when it has
/// a memory.
fn to_commit(root: &Path, id: &str) -> Result<Option<(wslock::Held, Repo)>, String> {
    wsfolder::folder_at(root, id)?;
    if !is_made(root, id) {
        return Ok(None);
    }
    let held = wslock::take(root, id)?;
    let Some(p) = place(root, id)? else { return Ok(None) };
    let repo = Repo::open(p)?;
    repo.prepare()?;
    Ok(Some((held, repo)))
}

/// Commit a dirty tree: what a fold, a chat's start and the quiet minute do.
/// `message` `None` names the files. `None` back: there was nothing to commit.
pub fn commit_if_dirty_at(root: &Path, id: &str, message: Option<&str>) -> Result<Option<Change>, String> {
    wsfolder::refuse_real(root)?;
    let Some((_held, repo)) = to_commit(root, id)? else { return Ok(None) };
    commit_held(&repo, message)
}

/// The quiet minute's commit: [`commit_if_dirty_at`] with no message, and the
/// chats that were live in the body. `live` is asked with the time of the
/// commit before it (seconds since the epoch, 0 for none), under the lock, so
/// it can leave out what that commit already holds.
pub fn commit_quiet_at(root: &Path, id: &str, live: impl FnOnce(i64) -> Vec<String>) -> Result<Option<Change>, String> {
    wsfolder::refuse_real(root)?;
    let Some((_held, repo)) = to_commit(root, id)? else { return Ok(None) };
    let last = head(&repo)?.map_or(0, |c| c.at);
    let named: Vec<String> = live(last).iter().filter(|c| crate::ulid::is_valid(c)).take(LIVE_MAX).map(|c| format!("{LIVE}{c}")).collect();
    commit_with(&repo, None, &named)
}

/// A fold's commit: [`commit_if_dirty_at`] with the attempt it is of in the
/// body, so that the commit of an attempt can be found again.
pub fn commit_fold_at(root: &Path, id: &str, subject: &str, attempt: &str) -> Result<Option<Change>, String> {
    wsfolder::refuse_real(root)?;
    if !crate::ulid::is_valid(attempt) {
        return Err(format!("not a fold's attempt: {attempt:?}"));
    }
    let Some((_held, repo)) = to_commit(root, id)? else { return Ok(None) };
    commit_with(&repo, Some(subject), &[format!("{ATTEMPT}{attempt}")])
}

/// The commit HEAD reaches that says it is `attempt`'s, by a line of its body
/// and not by a match anywhere in it. The oldest, should a later one repeat it.
pub fn fold_commit_at(root: &Path, id: &str, attempt: &str) -> Result<Option<Change>, String> {
    if !crate::ulid::is_valid(attempt) {
        return Err(format!("not a fold's attempt: {attempt:?}"));
    }
    let Some(p) = place(root, id)? else { return Ok(None) };
    let repo = Repo::open(p)?;
    if !repo.has_commits() {
        return Ok(None);
    }
    let line = format!("{ATTEMPT}{attempt}");
    let out = repo.text(&["log", "-z", "--first-parent", "--fixed-strings", "--grep", &line, "--format=tformat:%H%x02%b"])?;
    let of = |rec: &str| rec.trim_start_matches('\n').split_once('\x02').filter(|(_, b)| b.lines().any(|l| l == line)).map(|(sha, _)| sha.to_string());
    match out.split('\0').filter_map(of).last() {
        Some(sha) => change_of(&repo, &sha).map(Some),
        None => Ok(None),
    }
}

// ── history ─────────────────────────────────────────────────────────────────

pub fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// `commit` as a full id that HEAD reaches. A client names a commit by the id
/// a log answered with, never by an expression git would evaluate.
fn known(repo: &Repo, commit: &str) -> Result<String, String> {
    if !is_sha(commit) || !repo.run(&["merge-base", "--is-ancestor", commit, "HEAD"])?.status.success() {
        return Err("no such change in this memory".into());
    }
    Ok(commit.to_string())
}

fn kind_of_subject(s: &str) -> ChangeKind {
    if s == MADE {
        ChangeKind::Made
    } else if s.starts_with("fold:") {
        ChangeKind::Fold
    } else if s.starts_with("Undo: ") || s.starts_with("Revert \"") {
        ChangeKind::Undo
    } else if s.starts_with("Redo: ") || s.starts_with("Reapply \"") {
        ChangeKind::Redo
    } else {
        ChangeKind::Edit
    }
}

fn reverts_in(body: &str) -> Option<String> {
    body.lines().find_map(|l| l.strip_prefix(REVERTS)).and_then(|r| r.get(..40)).filter(|s| is_sha(s)).map(String::from)
}

/// The chats a commit's body names: ids and nothing else.
fn live_in(body: &str) -> Vec<String> {
    body.lines().filter_map(|l| l.strip_prefix(LIVE)).map(str::trim).filter(|c| crate::ulid::is_valid(c)).take(LIVE_MAX).map(String::from).collect()
}

const FORMAT: &str = "--format=tformat:%x01%H%x02%at%x02%s%x02%b%x02";

/// `git log -z --raw --numstat` with [`FORMAT`]. A raw record is followed by
/// its path as the next field; a numstat carries its own.
fn parse_log(out: &str) -> Vec<Change> {
    let mut v: Vec<Change> = Vec::new();
    let mut fields = out.split('\0').map(|f| f.trim_start_matches('\n'));
    while let Some(f) = fields.next() {
        if let Some(h) = f.strip_prefix('\x01') {
            let p: Vec<&str> = h.split('\x02').collect();
            if p.len() < 4 {
                continue;
            }
            v.push(Change {
                commit: p[0].into(),
                at: p[1].parse().unwrap_or(0),
                subject: p[2].into(),
                kind: kind_of_subject(p[2]),
                files: Vec::new(),
                added: 0,
                removed: 0,
                reverts: reverts_in(p[3]),
                reverted_by: None,
                live: live_in(p[3]),
            });
            continue;
        }
        let Some(c) = v.last_mut() else { continue };
        if f.starts_with(':') {
            let status = f.rsplit(' ').next().unwrap_or("").to_string();
            if let Some(path) = fields.next() {
                c.files.push(FileChange { path: path.into(), status, added: None, removed: None, binary: false });
            }
            continue;
        }
        let mut t = f.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (t.next(), t.next(), t.next()) else { continue };
        if let Some(fc) = c.files.iter_mut().find(|x| x.path == path) {
            fc.added = a.parse().ok();
            fc.removed = r.parse().ok();
            fc.binary = a == "-";
            c.added += fc.added.unwrap_or(0);
            c.removed += fc.removed.unwrap_or(0);
        }
    }
    v
}

/// Which commits are undone now, and by which: a revert counts only while
/// nothing has reverted IT. The same for one HEAD always, so it is read once
/// for each: the whole history's messages are behind it.
fn undone(repo: &Repo) -> Result<Arc<Undone>, String> {
    let Some(head) = head_id(&repo.place.git) else { return Ok(Arc::default()) };
    UNDONE.of(&head, || {
        let out = repo.text(&["log", "-z", "--first-parent", "--fixed-strings", "--grep", REVERTS, "--format=tformat:%H%x02%b", &head])?;
        let mut map = Undone::new();
        for rec in out.split('\0') {
            let Some((sha, body)) = rec.trim_start_matches('\n').split_once('\x02') else { continue };
            // Newest first, so whether this one stands is already known.
            if let Some(target) = reverts_in(body).filter(|_| !map.contains_key(sha)) {
                map.entry(target).or_insert_with(|| sha.to_string());
            }
        }
        Ok(map)
    })
}

/// A commit, and the one that undid it.
type Undone = HashMap<String, String>;

static UNDONE: ByHead = ByHead(Mutex::new(VecDeque::new()));

/// The last [`HEADS`] heads' answers. Two memories share no head: a commit's
/// id is of everything behind it.
struct ByHead(Mutex<VecDeque<(String, Arc<Undone>)>>);
const HEADS: usize = 64;

impl ByHead {
    fn of(&self, head: &str, read: impl FnOnce() -> Result<Undone, String>) -> Result<Arc<Undone>, String> {
        let known = |q: &VecDeque<(String, Arc<Undone>)>| q.iter().find(|(h, _)| h == head).map(|(_, u)| u.clone());
        if let Some(u) = known(&self.0.lock().unwrap_or_else(|e| e.into_inner())) {
            return Ok(u);
        }
        // Not under the lock: another memory's history is not waited for.
        let u = Arc::new(read()?);
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if known(&q).is_none() {
            q.push_back((head.to_string(), u.clone()));
            if q.len() > HEADS {
                q.pop_front();
            }
        }
        Ok(u)
    }
}

/// The commits of workspace `id`'s memory that are undone now, whatever undid
/// them: one git call, for whoever keeps a commit and shows whether it stands.
pub fn undone_at(root: &Path, id: &str) -> Result<std::collections::HashSet<String>, String> {
    let Some(p) = place(root, id)? else { return Ok(Default::default()) };
    let repo = Repo::open(p)?;
    if !repo.has_commits() {
        return Ok(Default::default());
    }
    Ok(undone(&repo)?.keys().cloned().collect())
}

fn log_in(repo: &Repo, path: Option<&str>, before: Option<&str>, n: usize) -> Result<Log, String> {
    if !repo.has_commits() {
        return Ok(Log::default());
    }
    let count = format!("--max-count={}", n + 1);
    let mut args = vec!["--literal-pathspecs", "log", "-z", "--first-parent", "-m", "--no-renames", "--no-color", "--no-ext-diff", "--no-textconv", "--raw", "--abbrev=40", "--numstat", FORMAT, &count];
    let from = match before {
        Some(b) => {
            args.push("--skip=1");
            known(repo, b)?
        }
        None => "HEAD".to_string(),
    };
    args.push(&from);
    args.push("--");
    if let Some(p) = path {
        args.push(p);
    }
    let mut changes = parse_log(&repo.text(&args)?);
    let more = changes.len() > n;
    changes.truncate(n);
    let by = undone(repo)?;
    for c in &mut changes {
        c.reverted_by = by.get(&c.commit).cloned();
    }
    Ok(Log { changes, more })
}

/// The folder's changes, newest first, `n` at a time (50, at most 200), for
/// one path when given. `before` is the last commit of the page before. Three
/// git calls for a page (the config's names, `before` itself, the log), and one
/// over every message the first time a HEAD is asked about.
pub fn log_at(root: &Path, id: &str, path: Option<&str>, before: Option<&str>, n: Option<usize>) -> Result<Log, String> {
    if let Some(p) = path {
        rel_ok(p)?;
    }
    let Some(p) = place(root, id)? else { return Ok(Log::default()) };
    log_in(&Repo::open(p)?, path, before, n.unwrap_or(LOG_PAGE).clamp(1, LOG_MAX))
}

/// One commit, as [`log_at`] lists it.
fn change_of(repo: &Repo, commit: &str) -> Result<Change, String> {
    let commit = known(repo, commit)?;
    let out = repo.text(&["log", "-z", "--first-parent", "-m", "--no-renames", "--no-color", "--no-ext-diff", "--no-textconv", "--raw", "--abbrev=40", "--numstat", FORMAT, "--max-count=1", &commit, "--"])?;
    let mut c = parse_log(&out).pop().ok_or("no such change in this memory")?;
    c.reverted_by = undone(repo)?.get(&c.commit).cloned();
    Ok(c)
}

/// `path`'s bytes in `commit`, unless it has none or they are over `cap`
/// (then no bytes, and the size says so).
fn blob(repo: &Repo, commit: &str, path: &str, cap: u64) -> Result<Option<(u64, Vec<u8>)>, String> {
    // The tree entry, not `<rev>:<path>`: git reads a leading `./` of the
    // latter against the cwd. A path the tree does not hold is no line.
    let out = repo.text(&["--literal-pathspecs", "ls-tree", "-z", "--long", "--full-tree", commit, "--", path])?;
    let Some((meta, _)) = out.split('\0').find(|r| !r.is_empty()).and_then(|r| r.split_once('\t')) else { return Ok(None) };
    let p: Vec<&str> = meta.split_whitespace().collect();
    // A link (120000) and a nested repository (160000) are no file to show.
    let [mode, "blob", sha, size] = p[..] else { return Ok(None) };
    if !mode.starts_with("100") {
        return Ok(None);
    }
    let size: u64 = size.parse().unwrap_or(u64::MAX);
    if size > cap {
        return Ok(Some((size, Vec::new())));
    }
    Ok(Some((size, repo.bytes(&["cat-file", "blob", sha])?)))
}

fn image_side(repo: &Repo, rev: &str, path: &str, mime: &'static str) -> Result<Option<Blob>, String> {
    let Some((size, bytes)) = blob(repo, rev, path, MAX_FILE_BYTES as u64)? else { return Ok(None) };
    let refused = if size > MAX_FILE_BYTES as u64 {
        Some(format!("not shown: {} KB, over the {} MB limit", size / 1024, MAX_FILE_BYTES / 1024 / 1024))
    } else {
        safepath::check_image(&bytes, mime, path).err()
    };
    let bytes = if refused.is_some() { Vec::new() } else { bytes };
    Ok(Some(Blob { mime, size, bytes, refused }))
}

/// `commit`'s files, and with `path` that one file's difference in it.
pub fn show_at(root: &Path, id: &str, commit: &str, path: Option<&str>) -> Result<Shown, String> {
    let repo = Repo::open(made(root, id)?)?;
    let change = change_of(&repo, commit)?;
    let Some(path) = path else { return Ok(Shown { change, file: None }) };
    // A path of history need not be on disk, so it is held to the commit's own
    // list instead, exactly.
    rel_ok(path)?;
    let fc = change.files.iter().find(|f| f.path == path).cloned().ok_or_else(|| format!("that change does not touch {path}"))?;
    let diff = if let Some(mime) = mime_of(path) {
        let parent = format!("{commit}^");
        let before = if fc.status == "A" { None } else { image_side(&repo, &parent, path, mime)? };
        let after = if fc.status == "D" { None } else { image_side(&repo, commit, path, mime)? };
        FileDiff::Image { before, after }
    } else if fc.binary {
        FileDiff::Binary
    } else {
        let out = repo.bytes(&[
            "--literal-pathspecs", "show", "--format=", "--first-parent", "-m", "--no-renames", "--no-color", "--no-ext-diff", "--no-textconv", "--unified=3", commit, "--", path,
        ])?;
        let truncated = out.len() > MAX_PATCH_BYTES;
        let end = if truncated { out[..MAX_PATCH_BYTES].iter().rposition(|b| *b == b'\n').map_or(MAX_PATCH_BYTES, |i| i + 1) } else { out.len() };
        FileDiff::Text { patch: String::from_utf8_lossy(&out[..end]).into_owned(), truncated }
    };
    Ok(Shown { change, file: Some((fc, diff)) })
}

/// Undo `commit`, or redo it when `commit` is itself an undo: a NEW commit.
/// Nothing is ever reset or dropped. A dirty tree is committed first, because
/// git refuses a revert over uncommitted edits. On a conflict the revert is
/// aborted and the answer is the files.
pub fn revert_at(root: &Path, id: &str, commit: &str) -> Result<Reverted, String> {
    wsfolder::refuse_real(root)?;
    if !is_made(root, id) {
        wsfolder::folder_at(root, id)?;
        return Err("this workspace has no memory yet".into());
    }
    let _held = wslock::take(root, id)?;
    let repo = Repo::open(made(root, id)?)?;
    repo.prepare()?;
    let change = change_of(&repo, commit)?;
    if change.kind == ChangeKind::Made || !repo.run(&["rev-parse", "--verify", "--quiet", &format!("{commit}^")])?.status.success() {
        return Err("the change that made this memory cannot be undone".into());
    }
    if change.reverted_by.is_some() {
        return Err("that change is already undone".into());
    }
    commit_held(&repo, None)?;
    // A file the memory does not keep where the undo would put one: git takes
    // an ignored one for its own to overwrite.
    let tracked = repo.text(&["ls-tree", "-r", "-z", "--name-only", "HEAD"])?;
    let tracked: HashSet<&str> = tracked.split('\0').collect();
    if let Some(f) = change.files.iter().find(|f| !tracked.contains(f.path.as_str()) && presence(&repo.place.mem, &f.path) != Presence::Gone) {
        return Err(format!("{} is in the way: it is not a file this memory keeps", f.path));
    }
    let o = repo.run(&["revert", "--no-edit", commit])?;
    if !o.status.success() {
        let conflicts: Vec<String> =
            repo.text(&["diff", "--name-only", "-z", "--diff-filter=U"])?.split('\0').filter(|p| !p.is_empty()).map(String::from).collect();
        let began = fs::symlink_metadata(repo.place.git.join("REVERT_HEAD")).is_ok();
        if began {
            repo.bytes(&["revert", "--abort"])?;
        }
        if !conflicts.is_empty() {
            return Ok(Reverted { change: None, conflicts });
        }
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        return Err(if err.contains("would be overwritten") {
            "a file changed while this was being undone: try again".into()
        } else if began || String::from_utf8_lossy(&o.stdout).contains("nothing to commit") {
            "there is nothing left of that change to undo".into()
        } else {
            format!("git revert failed: {}", err.trim())
        });
    }
    // Our own words over git's, keeping its sentence: the one [`undone`] reads,
    // so a crash between the two still reads as an undo.
    let what = change.subject.strip_prefix("Undo: ").or_else(|| change.subject.strip_prefix("Redo: ")).unwrap_or(&change.subject);
    let verb = if change.kind == ChangeKind::Undo { "Redo" } else { "Undo" };
    let msg = format!("{}\n\n{REVERTS}{commit}.", subject(&format!("{verb}: {what}")));
    repo.bytes(&["commit", "--quiet", "--amend", "--no-verify", "--allow-empty", "--cleanup=whitespace", &format!("--message={msg}")])?;
    Ok(Reverted { change: head(&repo)?, conflicts: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Scratch {
        base: PathBuf,
        root: PathBuf,
        id: String,
    }

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let base = std::env::temp_dir().join(format!("wt-memory-{tag}-{}-{:x}", std::process::id(), fsx::random_u64()));
            fs::create_dir_all(&base).unwrap();
            Scratch { root: base.join("workspaces"), id: crate::ulid::new(), base }
        }
        fn made(tag: &str) -> Scratch {
            let s = Scratch::new(tag);
            assert!(init_at(&s.root, &s.id, "Personal").unwrap().made);
            s
        }
        fn mem(&self) -> PathBuf {
            dir_at(&self.root, &self.id).unwrap()
        }
        fn put(&self, path: &str, text: &str) {
            let p = self.mem().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        fn text(&self, path: &str) -> String {
            fs::read_to_string(self.mem().join(path)).unwrap()
        }
        fn commit(&self, msg: &str) -> Change {
            commit_if_dirty_at(&self.root, &self.id, Some(msg)).unwrap().expect("there was something to commit")
        }
        fn log(&self) -> Vec<Change> {
            log_at(&self.root, &self.id, None, None, None).unwrap().changes
        }
        fn read(&self, path: &str) -> Result<Doc, String> {
            read_at(&self.root, &self.id, path)
        }
        fn write(&self, path: &str, text: &str, m: Option<u64>) -> Result<Saved, String> {
            write_at(&self.root, &self.id, path, text, &m.map_or(Base::None, Base::Mtime), false)
        }
        fn revert(&self, c: &Change) -> Result<Reverted, String> {
            revert_at(&self.root, &self.id, &c.commit)
        }
        /// The repository with nothing switched off, and where it is.
        fn repo(&self) -> Repo {
            Repo { place: made(&self.root, &self.id).unwrap(), off: Vec::new() }
        }
        fn git_dir(&self) -> PathBuf {
            git_dir_at(&self.root, &self.id).unwrap()
        }
        /// The test's own git, for planting: never the module's.
        fn git(&self, args: &[&str]) {
            assert!(!self.mem().as_os_str().is_empty() && self.mem().starts_with(&self.base));
            let o = self.repo().run(args).unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    /// The smallest PNG header `image_dims` reads, `n` wide.
    fn png(n: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(n.to_be_bytes());
        b.extend(1u32.to_be_bytes());
        b.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    /// The chats live while it was changed, as a trailer: ids and nothing else,
    /// never in the subject, and only on the quiet minute's own commit.
    #[test]
    fn a_quiet_commit_names_the_chats_that_were_live_and_nothing_else_does() {
        let s = Scratch::made("live");
        let (a, b) = (crate::ulid::new(), crate::ulid::new());
        s.put("topics/t.md", "one\n");
        let mut told = None;
        let c = commit_quiet_at(&s.root, &s.id, |at| {
            told = Some(at);
            vec![a.clone(), "x\nSubject: forged".into(), b.clone()]
        })
        .unwrap()
        .unwrap();
        assert_eq!(told, Some(s.log()[1].at), "the commit before it");
        assert_eq!((c.subject.as_str(), c.kind, c.live.clone()), ("Edited: topics/t.md", ChangeKind::Edit, vec![a, b]));
        assert_eq!(s.log()[0].live, c.live, "as History reads it");
        let raw = s.repo().text(&["log", "-1", "--format=%B"]).unwrap();
        assert!(!raw.contains("forged"), "what is not an id is not written either: {raw}");
        // Nobody live is no body; and nothing else carries one.
        s.put("topics/t.md", "two\n");
        assert!(commit_quiet_at(&s.root, &s.id, |_| vec![]).unwrap().unwrap().live.is_empty());
        s.put("topics/t.md", "three\n");
        assert!(s.commit("fold: X").live.is_empty());
        let undone = s.revert(&c).map(|r| r.conflicts);
        assert_eq!(undone.unwrap(), vec!["topics/t.md"]);
        assert_eq!(commit_quiet_at(&s.root, &s.id, |_| vec![crate::ulid::new()]).unwrap(), None, "clean is no commit");
    }

    #[test]
    fn making_is_one_commit_by_a_fixed_author_and_making_again_changes_nothing() {
        let s = Scratch::new("make");
        assert_eq!(status_at(&s.root, &s.id).unwrap(), Status { made: false, dirty: vec![] });
        assert!(!s.root.exists(), "asking makes nothing");
        assert_eq!(list_at(&s.root, &s.id).unwrap(), Listing::default());
        assert_eq!(log_at(&s.root, &s.id, None, None, None).unwrap(), Log::default());
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, None).unwrap(), None);
        assert_eq!(init_at(&s.root, &s.id, "Per\nsonal").unwrap(), Status { made: true, dirty: vec![] });
        assert!(s.text(INDEX).starts_with("# Memory: Per sonal\n\n## What this workspace is for\n"));
        let l = s.log();
        assert_eq!((l.len(), l[0].subject.as_str(), l[0].kind), (1, MADE, ChangeKind::Made));
        assert_eq!(l[0].files[0].path, INDEX);
        let repo = Repo::open(made(&s.root, &s.id).unwrap()).unwrap();
        assert_eq!(repo.text(&["log", "--format=%an <%ae> %cn <%ce>"]).unwrap().trim(), "worktrees <memory@localhost> worktrees <memory@localhost>");
        assert_eq!(repo.text(&["remote"]).unwrap(), "", "no remote");
        assert!(!s.git_dir().join("hooks").exists() && !s.mem().join(".git").exists(), "no template");
        assert!(!fs::read_to_string(s.git_dir().join("config")).unwrap().contains("worktree"), "it can be moved");
        fs::remove_file(s.mem().join(INDEX)).unwrap();
        init_at(&s.root, &s.id, "Personal").unwrap();
        assert!(!s.mem().join(INDEX).exists(), "a memory already made is left as it is");
        assert_eq!(index_at(&s.root, &s.id).unwrap(), None);
    }

    /// A folder that holds files and no repository is finished, its files kept.
    #[test]
    fn a_folder_with_files_and_no_repository_is_made_around_them() {
        let s = Scratch::new("adopt");
        s.put(INDEX, "# Mine\n");
        assert!(!status_at(&s.root, &s.id).unwrap().made);
        init_at(&s.root, &s.id, "Personal").unwrap();
        assert_eq!(s.text(INDEX), "# Mine\n");
    }

    /// The repository is never looked for UPWARD: a memory whose `.git` is gone
    /// under a folder that is itself a repository commits nothing there.
    #[test]
    fn a_memory_without_its_repository_never_commits_into_the_one_above_it() {
        let s = Scratch::made("upward");
        let above = |args: &[&str]| crate::childenv::command("git").arg("-C").arg(&s.base).args(args).output().unwrap().status.success();
        assert!(above(&["init", "--quiet", "--template="]));
        let untouched = || assert!(!above(&["rev-parse", "--verify", "--quiet", "HEAD"]) && above(&["diff", "--cached", "--quiet"]));
        fs::remove_dir_all(s.git_dir()).unwrap();
        s.put("topics/a.md", "a\n");
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, Some("x")).unwrap(), None);
        assert!(log_at(&s.root, &s.id, None, None, None).unwrap().changes.is_empty());
        untouched();
        // A `.git` FILE naming that repository is refused, not followed.
        fs::write(s.mem().join(".git"), format!("gitdir: {}\n", s.base.join(".git").display())).unwrap();
        assert!(init_at(&s.root, &s.id, "x").unwrap_err().contains("not a folder"));
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, Some("x")).unwrap(), None);
        fs::remove_file(s.mem().join(".git")).unwrap();
        symlink(s.base.join(".git"), s.mem().join(".git")).unwrap();
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, Some("x")).unwrap(), None);
        untouched();
        assert!(!status_at(&s.root, &s.id).unwrap().made);
    }

    /// NOTHING in the folder runs: the hooks in `.git/hooks`, a hooks path in
    /// the repository's config, git 2.54's hooks IN config (which a hooks path
    /// of /dev/null does not stop), a file system monitor, a filter, a merge
    /// and a diff driver from `.gitattributes`, and one in an INCLUDED file.
    /// Every act that calls git is run over it.
    #[test]
    fn nothing_the_folder_holds_runs_when_the_engine_calls_git() {
        let s = Scratch::made("hostile");
        let ran = s.base.join("ran");
        fs::create_dir_all(&ran).unwrap();
        let mark = |n: &str| format!("touch {}/{n}; cat", ran.display());
        let git = s.git_dir();
        for dir in [git.join("hooks"), s.base.join("hooks")] {
            fs::create_dir_all(&dir).unwrap();
            for h in ["pre-commit", "post-commit", "commit-msg", "prepare-commit-msg", "post-index-change", "reference-transaction", "post-checkout", "pre-auto-gc"] {
                fsx::exec_fixture(&dir.join(h), &format!("#!/bin/sh\ntouch {}/{}-{h}\n", ran.display(), name_of(&dir.to_string_lossy())));
            }
        }
        let mut cfg = fs::read_to_string(git.join("config")).unwrap();
        cfg += &format!("[core]\n\thooksPath = {}\n\tfsmonitor = \"{}\"\n\tworktree = {}\n", s.base.join("hooks").display(), mark("fsmonitor"), s.base.display());
        cfg += &format!("[hook \"a b\"]\n\tevent = pre-commit\n\tevent = post-commit\n\tcommand = \"{}\"\n", mark("config-hook"));
        cfg += &format!("[filter \"x.y\"]\n\tclean = \"{}\"\n\tsmudge = \"{}\"\n\tprocess = \"{}\"\n\trequired = true\n", mark("clean"), mark("smudge"), mark("process"));
        cfg += &format!("[merge \"m\"]\n\tdriver = \"{}\"\n[diff \"d\"]\n\tcommand = \"{}\"\n\ttextconv = \"{}\"\n[diff]\n\texternal = \"{}\"\n", mark("merge"), mark("difft"), mark("textconv"), mark("external"));
        cfg += &format!("[gpg]\n\tprogram = {}/nope\n[commit]\n\tgpgsign = true\n[log]\n\tshowSignature = true\n[core]\n\teditor = \"{}\"\n\tpager = \"{}\"\n", s.base.display(), mark("editor"), mark("pager"));
        cfg += &format!("[include]\n\tpath = {}\n", s.base.join("more").display());
        fs::write(git.join("config"), &cfg).unwrap();
        fs::write(s.base.join("more"), format!("[hook \"inc\"]\n\tevent = pre-commit\n\tcommand = \"{}\"\n[filter \"z\"]\n\tclean = \"{}\"\n", mark("inc-hook"), mark("inc-clean"))).unwrap();
        fs::write(s.mem().join(".gitattributes"), "topics/** filter=x.y merge=m diff=d\nnotes/** filter=z\n").unwrap();

        s.put("topics/a.md", "one\n");
        s.put("notes/n.md", "n\n");
        let one = s.commit("one");
        s.put("topics/a.md", "two\n");
        let two = s.commit("two");
        s.put("topics/a.md", "three\n");
        s.commit("three");
        assert_eq!(s.revert(&two).unwrap().conflicts, vec!["topics/a.md"], "a conflict: the merge driver's turn");
        assert!(s.log().len() >= 4);
        show_at(&s.root, &s.id, &one.commit, Some("topics/a.md")).unwrap();
        read_commit_at(&s.root, &s.id, "topics/a.md", &one.commit).unwrap();
        s.put("topics/b.md", "b\n");
        let b = s.commit("b");
        s.revert(&b).unwrap();
        status_at(&s.root, &s.id).unwrap();
        let left: Vec<_> = fs::read_dir(&ran).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(left.is_empty(), "these ran: {left:?}");
        // And it committed the memory, not the folder `core.worktree` named.
        assert!(one.files.iter().all(|f| !f.path.starts_with("workspaces/")), "{one:?}");
        assert!(one.files.iter().any(|f| f.path == "topics/a.md"));
        // The plant is live: with nothing switched off by name, git runs it.
        s.put("notes/o.md", "o\n");
        let bare = s.repo();
        fs::remove_file(git.join("info/attributes")).unwrap();
        s.put("topics/o.md", "o\n");
        for d in ["notes", "topics"] {
            let _ = bare.run(&["add", "--", d]);
        }
        // The event alone: a commit re-reads a racily clean entry through the
        // filter first, and dies there before any hook.
        let hook = bare.run(&["hook", "run", "pre-commit"]);
        for n in ["config-hook", "inc-hook", "process", "inc-clean"] {
            // git before 2.54 has no hooks in config to run.
            if !n.ends_with("hook") || has_config_hooks() {
                assert!(ran.join(n).exists(), "the fixture's {n} runs when nothing stops it: {hook:?}");
            }
        }
    }

    fn has_config_hooks() -> bool {
        let v = String::from_utf8_lossy(&crate::childenv::command("git").arg("--version").output().unwrap().stdout).into_owned();
        let n: Vec<u32> = v.split_whitespace().nth(2).unwrap_or("").split('.').filter_map(|x| x.parse().ok()).collect();
        n >= vec![2, 54]
    }

    #[test]
    fn a_config_name_that_cannot_be_switched_off_is_refused() {
        let k = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(neutralized(&k(&["core.bare", "user.name", "remote.o.url", "merge.m.name", "diff.d.xfuncname", "filter"])).unwrap(), Vec::<String>::new());
        assert_eq!(neutralized(&k(&["hook.a.b.command", "hook.a.b.event"])).unwrap(), k(&["hook.a.b.enabled=false"]));
        assert!(neutralized(&k(&["filter.a=b.clean"])).unwrap_err().contains("remove it"));
    }

    /// The child's environment: whatever git is told by the process it is in
    /// is dropped, and what it must not do is said.
    #[test]
    fn the_childs_environment_carries_no_git_variable_but_ours() {
        let e = env_edits(["GIT_DIR", "GIT_INDEX_FILE", "GIT_AUTHOR_NAME", "GIT_CONFIG_COUNT", "GIT_EXEC_PATH", "PATH", "HOME"].into_iter().map(String::from));
        let of = |k: &str| e.iter().rev().find(|(n, _)| n == k).map(|(_, v)| *v);
        for k in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_AUTHOR_NAME", "GIT_CONFIG_COUNT"] {
            assert_eq!(of(k), Some(None), "{k} is removed");
        }
        for k in ["GIT_EXEC_PATH", "PATH", "HOME"] {
            assert_eq!(of(k), None, "{k} stays as it is");
        }
        assert_eq!(of("GIT_CONFIG_GLOBAL"), Some(Some("/dev/null")));
        assert_eq!(of("GIT_ALLOW_PROTOCOL"), Some(Some("none")));
        let s = Scratch::made("env");
        let c = Repo::open(made(&s.root, &s.id).unwrap()).unwrap().command();
        let envs: HashMap<_, _> = c.get_envs().collect();
        assert_eq!(envs.get(std::ffi::OsStr::new("GIT_DIR")), Some(&None));
        assert_eq!(envs.get(std::ffi::OsStr::new("GIT_WORK_TREE")), Some(&None));
        let argv: Vec<_> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        for f in ["core.hooksPath=/dev/null", "commit.gpgsign=false", "user.name=worktrees", "user.email=memory@localhost"] {
            assert!(argv.windows(2).any(|w| w[0] == "-c" && w[1] == f), "{f}: {argv:?}");
        }
    }

    /// Every way a path can name something that is not a file of this memory,
    /// over every function that takes one. Nothing outside is read, made or
    /// changed.
    #[test]
    fn no_path_reaches_outside_the_folder_or_into_its_repository() {
        let s = Scratch::made("paths");
        let out = s.base.join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("secret.md"), "secret").unwrap();
        fs::write(out.join("secret.png"), png(1)).unwrap();
        symlink(out.join("secret.md"), s.mem().join("link.md")).unwrap();
        symlink(out.join("secret.png"), s.mem().join("link.png")).unwrap();
        symlink(out.join("nope.md"), s.mem().join("dangling.md")).unwrap();
        symlink(&out, s.mem().join("away")).unwrap();
        fs::create_dir_all(s.mem().join(".git")).unwrap();
        fs::write(s.mem().join(".git/config"), "theirs").unwrap();
        symlink(s.mem().join(".git"), s.mem().join("g")).unwrap();
        s.put("topics/t.md", "t\n");
        symlink(s.mem().join("topics/t.md"), s.mem().join("inside.md")).unwrap();
        let cfg = fs::read(s.mem().join(".git/config")).unwrap();
        for p in [
            "", "/etc/hosts", "../out/secret.md", "topics/../../../out/secret.md", "a//b.md", "./MEMORY.md", ".git/config", ".GIT/config",
            "topics/.git/config", "g/config", "g/x.md", "link.md", "dangling.md", "inside.md", "away/secret.md", "away/new.md", "topics\\x.md",
            "a\0.md", "a\n.md", ".gitattributes", "-x/../y.md", "topics/\u{202e}x.md",
        ] {
            assert!(s.read(p).is_err(), "read {p:?}");
            assert!(s.write(p, "MINE", Some(0)).is_err(), "save {p:?}");
            assert!(s.write(p, "MINE", None).is_err(), "make {p:?}");
        }
        for p in ["link.png", "away/secret.png", "../out/secret.png", ".git/x.png"] {
            assert!(image_at(&s.root, &s.id, p, None).is_err(), "image {p:?}");
        }
        for n in ["", "a/b.png", "../x.png", ".hidden", "..", "/x", "a\\b", "a\nb", &"x".repeat(201)] {
            assert!(put_file_at(&s.root, &s.id, n, &png(1)).is_err(), "put {n:?}");
        }
        assert_eq!(fs::read_to_string(out.join("secret.md")).unwrap(), "secret");
        assert_eq!(fs::read_dir(&out).unwrap().count(), 2, "nothing was made outside");
        assert_eq!(fs::read(s.mem().join(".git/config")).unwrap(), cfg);
        // `files` being a link is refused too.
        fs::remove_dir(s.mem().join(FILES)).unwrap();
        symlink(&out, s.mem().join(FILES)).unwrap();
        assert!(put_file_at(&s.root, &s.id, "x.png", &png(1)).is_err());
        assert_eq!(fs::read_dir(&out).unwrap().count(), 2);
        // Nor listed, nor watched, and the list says why.
        let l = list_at(&s.root, &s.id).unwrap();
        let listed: Vec<_> = l.folders.iter().flat_map(|f| f.files.iter().map(|x| x.path.as_str())).collect();
        assert_eq!(listed, vec![INDEX, "topics/t.md"]);
        assert!(l.skipped.iter().any(|x| x.path == "link.md" && x.why == "not a regular file"), "{:?}", l.skipped);
        assert_eq!(stamp_at(&s.root, &s.id).unwrap().files.keys().collect::<Vec<_>>(), vec![INDEX, "topics/t.md"]);
        // A path with a leading dash is only a path.
        assert!(s.write("--all.md", "x\n", None).is_ok());
        assert_eq!(s.commit("dash").files.iter().filter(|f| f.path == "--all.md").count(), 1);
        assert_eq!(log_at(&s.root, &s.id, Some("--all.md"), None, None).unwrap().changes.len(), 1);
        // A pathspec's magic is only a name.
        assert!(log_at(&s.root, &s.id, Some(":(glob)**"), None, None).unwrap().changes.is_empty());
        assert!(dir_at(&s.root, "../x").is_err());
    }

    #[test]
    fn a_save_is_not_a_commit_and_is_refused_over_a_change_made_elsewhere() {
        let s = Scratch::made("save");
        let d = s.read(INDEX).unwrap();
        assert_eq!((d.title.as_str(), d.file.as_str()), ("Memory: Personal", INDEX));
        let w = s.write(INDEX, "# Ours\n", Some(d.mtime_ms)).unwrap();
        assert_eq!((w.title.as_str(), w.size), ("Ours", 7));
        assert_eq!(s.read(INDEX).unwrap().mtime_ms, w.mtime_ms, "the mtime a save answers with is the file's");
        assert_eq!(s.log().len(), 1, "no commit");
        assert_eq!(status_at(&s.root, &s.id).unwrap().dirty, vec![INDEX]);
        let e = s.write(INDEX, "# Late\n", Some(w.mtime_ms + 1)).unwrap_err();
        assert!(e.starts_with(ERR_STALE), "{e}");
        assert_eq!(s.text(INDEX), "# Ours\n");
        assert_eq!(s.write(INDEX, "# Ours\n", Some(w.mtime_ms)).unwrap().mtime_ms, w.mtime_ms, "the same text writes nothing");
        // Making: only where there is nothing, with the folders on the way.
        assert!(s.write(INDEX, "x", None).unwrap_err().contains("already a file"));
        assert_eq!(s.write("topics/deep/new.md", "# New\n", None).unwrap().title, "New");
        assert!(s.write("topics/gone.md", "x", Some(1)).unwrap_err().starts_with(ERR_GONE));
        assert!(s.read("topics/gone.md").unwrap_err().starts_with(ERR_GONE));
        assert!(s.write("files/x.png", "x", None).unwrap_err().contains("not a text file"));
        // Keep mine: what is on disk is in the history before it goes.
        fs::write(s.mem().join(INDEX), "# Theirs\n").unwrap();
        let m = s.read(INDEX).unwrap().mtime_ms;
        write_at(&s.root, &s.id, INDEX, "# Mine\n", &Base::Mtime(m), true).unwrap();
        let top = &s.log()[0];
        assert_eq!(top.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec![INDEX], "that file and no other");
        assert!(status_at(&s.root, &s.id).unwrap().dirty.contains(&"topics/deep/new.md".to_string()));
        assert_eq!(top.subject, "Before keeping your text: MEMORY.md");
        assert_eq!(read_commit_at(&s.root, &s.id, INDEX, &top.commit).unwrap().content, "# Theirs\n");
        assert_eq!(s.text(INDEX), "# Mine\n");
    }

    #[test]
    fn text_that_is_not_text_is_refused_in_words() {
        let s = Scratch::made("text");
        fs::write(s.mem().join("a.md"), [0xff, 0xfe]).unwrap();
        fs::write(s.mem().join("b.md"), b"a\0b").unwrap();
        fs::write(s.mem().join("c.md"), vec![b'x'; MAX_TEXT_BYTES as usize + 1]).unwrap();
        assert!(s.read("a.md").unwrap_err().contains("not UTF-8"));
        assert!(s.read("b.md").unwrap_err().contains("not text"));
        assert!(s.read("c.md").unwrap_err().contains("too large"));
    }

    #[test]
    fn the_list_folds_by_folder_and_the_index_says_when_it_is_over_budget() {
        let s = Scratch::made("list");
        for p in ["notes/2026-11-01-a.md", "notes/2026-11-24-b.md", "topics/nas.md", "z/extra.md", "readme.txt"] {
            s.put(p, "x\n");
        }
        s.put("notes/2026-11-24-b.md", "# Release order\n");
        put_file_at(&s.root, &s.id, "rack.png", &png(3)).unwrap();
        let l = list_at(&s.root, &s.id).unwrap();
        assert_eq!(l.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["", "notes", "topics", "files", "z"]);
        let of = |n: &str| l.folders.iter().find(|f| f.name == n).unwrap().files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>();
        assert_eq!(of(""), vec![INDEX, "readme.txt"]);
        assert_eq!(of("notes"), vec!["notes/2026-11-24-b.md", "notes/2026-11-01-a.md"], "newest first");
        assert_eq!(l.folders[1].files[0].title.as_deref(), Some("Release order"));
        assert_eq!(l.folders[3].files[0].kind, Kind::Image);
        assert_eq!(l.count, 7);
        assert!(!index_at(&s.root, &s.id).unwrap().unwrap().over_budget);
        s.put(INDEX, &"x".repeat(INDEX_BUDGET as usize + 1));
        let i = index_at(&s.root, &s.id).unwrap().unwrap();
        assert!(i.over_budget && i.content.len() as u64 == i.size, "said, never cut");
        // A save in flight and Finder's file are never in a commit.
        s.put(".MEMORY.md.0123456789abcdef.tmp", "x");
        s.put("topics/.DS_Store", "x");
        assert!(status_at(&s.root, &s.id).unwrap().dirty.iter().all(|p| !p.contains("/.") && !p.starts_with('.')));
        // A clone inside, and a huge file: named, and left out of the history.
        fs::create_dir_all(s.mem().join("topics/clone/.git")).unwrap();
        s.put("topics/clone/in.md", "x\n");
        fs::File::create(s.mem().join("files/big.bin")).unwrap().set_len(MAX_COMMIT_BYTES + 1).unwrap();
        let l = list_at(&s.root, &s.id).unwrap();
        assert!(l.skipped.iter().any(|x| x.path == "topics/clone"), "{:?}", l.skipped);
        assert!(l.skipped.iter().any(|x| x.path == "files/big.bin"), "{:?}", l.skipped);
        let c = s.commit("all");
        assert!(c.files.iter().all(|f| !f.path.starts_with("topics/clone") && f.path != "files/big.bin"), "{c:?}");
        assert_eq!(status_at(&s.root, &s.id).unwrap().dirty, Vec::<String>::new(), "and they do not stay dirty for good");
    }

    #[test]
    fn a_put_keeps_every_file_and_refuses_what_is_too_big() {
        let s = Scratch::made("put");
        let a = put_file_at(&s.root, &s.id, "rack.png", &png(1)).unwrap();
        let b = put_file_at(&s.root, &s.id, "rack.png", &png(2)).unwrap();
        assert_eq!((a.path.as_str(), b.path.as_str()), ("files/rack.png", "files/rack-2.png"));
        assert_eq!(fs::read(s.mem().join("files/rack.png")).unwrap(), png(1));
        assert!(put_file_at(&s.root, &s.id, "x.bin", &vec![0; MAX_FILE_BYTES + 1]).unwrap_err().contains("4 MB"));
        assert!(put_file_at(&s.root, &s.id, "x.png", b"not a png").unwrap_err().contains("header"));
        let (img, same) = image_at(&s.root, &s.id, "files/rack.png", None).unwrap();
        assert_eq!((img.bytes, img.mime, same), (png(1), "image/png", false));
        let etag = format!("{}-{}", img.mtime_ms, img.on_disk);
        assert!(image_at(&s.root, &s.id, "files/rack.png", Some(&etag)).unwrap().1);
        s.put("topics/t.md", "x");
        assert!(image_at(&s.root, &s.id, "topics/t.md", None).unwrap_err().contains("not an image"));
    }

    #[test]
    fn a_commit_names_its_files_and_counts_and_the_log_pages() {
        let s = Scratch::made("log");
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, None).unwrap(), None, "clean");
        s.put("topics/a.md", "1\n2\n");
        s.put("notes/ünï.md", "n\n");
        put_file_at(&s.root, &s.id, "rack.png", &[png(1), vec![0; 64]].concat()).unwrap();
        let c = s.commit("fold: Release\norder");
        assert_eq!((c.subject.as_str(), c.kind), ("fold: Release order", ChangeKind::Fold));
        assert_eq!(c.files.iter().map(|f| (f.path.as_str(), f.status.as_str(), f.added, f.binary)).collect::<Vec<_>>(), vec![
            ("files/rack.png", "A", None, true),
            ("notes/ünï.md", "A", Some(1), false),
            ("topics/a.md", "A", Some(2), false),
        ]);
        assert_eq!((c.added, c.removed), (3, 0));
        s.put("topics/a.md", "2\n");
        fs::remove_file(s.mem().join("notes/ünï.md")).unwrap();
        let e = commit_if_dirty_at(&s.root, &s.id, None).unwrap().unwrap();
        assert_eq!((e.subject.as_str(), e.kind), ("Edited: notes/ünï.md, topics/a.md", ChangeKind::Edit));
        assert_eq!((e.files[0].status.as_str(), e.added, e.removed), ("D", 0, 2));
        assert_eq!(log_at(&s.root, &s.id, Some("topics/a.md"), None, None).unwrap().changes.len(), 2);
        for i in 0..4 {
            s.put("topics/b.md", &format!("{i}\n"));
            s.commit(&format!("b{i}"));
        }
        let p1 = log_at(&s.root, &s.id, Some("topics/b.md"), None, Some(3)).unwrap();
        assert_eq!((p1.changes.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), p1.more), (vec!["b3", "b2", "b1"], true));
        // A commit lands between the pages: the second is where it was.
        s.put("topics/b.md", "late\n");
        s.commit("late");
        let p2 = log_at(&s.root, &s.id, Some("topics/b.md"), Some(&p1.changes[2].commit), Some(3)).unwrap();
        assert_eq!((p2.changes.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), p2.more), (vec!["b0"], false));
        for bad in ["HEAD", "--all", "deadbeef", &"0".repeat(40), "main"] {
            assert!(show_at(&s.root, &s.id, bad, None).is_err(), "{bad}");
            assert!(revert_at(&s.root, &s.id, bad).is_err(), "{bad}");
            assert!(log_at(&s.root, &s.id, None, Some(bad), None).is_err(), "{bad}");
        }
        // The last file removed: an empty folder is a commit like any other.
        for f in list_at(&s.root, &s.id).unwrap().folders.iter().flat_map(|f| &f.files) {
            fs::remove_file(s.mem().join(&f.path)).unwrap();
        }
        assert!(s.commit("emptied").files.iter().all(|f| f.status == "D"));
    }

    #[test]
    fn one_files_diff_at_a_time_and_a_changed_image_answers_both_sides() {
        let s = Scratch::made("show");
        s.put("topics/a.md", "one\ntwo\n");
        s.put("files/r.png", "");
        fs::write(s.mem().join("files/r.png"), png(1)).unwrap();
        fs::write(s.mem().join("files/x.bin"), b"a\0b").unwrap();
        let c1 = s.commit("first");
        s.put("topics/a.md", "one\n2\n");
        fs::write(s.mem().join("files/r.png"), png(2)).unwrap();
        fs::write(s.mem().join("files/x.bin"), b"a\0c").unwrap();
        let c2 = s.commit("second");
        let show = |c: &Change, p: Option<&str>| show_at(&s.root, &s.id, &c.commit, p).unwrap();
        assert_eq!(show(&c2, None).change.files.len(), 3);
        let Some((_, FileDiff::Text { patch, truncated })) = show(&c2, Some("topics/a.md")).file else { panic!() };
        assert!(patch.contains("-two\n+2\n") && !patch.contains("r.png") && !truncated, "{patch}");
        let Some((_, FileDiff::Image { before, after })) = show(&c2, Some("files/r.png")).file else { panic!() };
        assert_eq!((before.unwrap().bytes, after.unwrap().bytes), (png(1), png(2)));
        let Some((_, FileDiff::Image { before, after })) = show(&c1, Some("files/r.png")).file else { panic!() };
        assert!(before.is_none() && after.unwrap().refused.is_none());
        assert_eq!(show(&c2, Some("files/x.bin")).file.unwrap().1, FileDiff::Binary);
        assert!(show_at(&s.root, &s.id, &c2.commit, Some(INDEX)).unwrap_err().contains("does not touch"));
        // An image too large to draw says so, without its bytes.
        fs::write(s.mem().join("files/r.png"), [png(3), vec![0; MAX_FILE_BYTES]].concat()).unwrap();
        let c3 = s.commit("big");
        let Some((_, FileDiff::Image { after: Some(a), .. })) = show(&c3, Some("files/r.png")).file else { panic!() };
        assert!(a.bytes.is_empty() && a.refused.unwrap().contains("4 MB"));
        // Nor the bytes of one whose header cannot be read.
        fs::write(s.mem().join("files/r.png"), "no header").unwrap();
        let cx = s.commit("bad");
        let Some((_, FileDiff::Image { after: Some(a), .. })) = show(&cx, Some("files/r.png")).file else { panic!() };
        assert!(a.bytes.is_empty() && a.refused.unwrap().contains("header"));
        // A link is in the history as a link and shown as no image.
        fs::remove_file(s.mem().join("files/r.png")).unwrap();
        symlink("/etc/hosts", s.mem().join("files/r.png")).unwrap();
        s.git(&["update-index", "--add", "files/r.png"]);
        s.git(&["commit", "--quiet", "--message=link"]);
        let c4 = s.log().remove(0);
        let Some((_, FileDiff::Image { after, .. })) = show(&c4, Some("files/r.png")).file else { panic!() };
        assert!(after.is_none());
        // A long patch is cut at a line and says so.
        s.put("topics/long.md", &"0123456789abcdef\n".repeat(40_000));
        let c5 = s.commit("long");
        let Some((_, FileDiff::Text { patch, truncated })) = show(&c5, Some("topics/long.md")).file else { panic!() };
        assert!(truncated && patch.len() <= MAX_PATCH_BYTES && patch.ends_with('\n'));
    }

    #[test]
    fn an_undo_is_a_commit_a_redo_undoes_it_and_the_log_says_which_stands() {
        let s = Scratch::made("undo");
        s.put("notes/n.md", "note\n");
        s.put(INDEX, "# M\n- see notes/n.md\n");
        let fold = s.commit("fold: Storage");
        let before = s.log().len();
        // Uncommitted edits to the same file, elsewhere in it, are committed first.
        s.put("topics/live.md", "a live chat's\n");
        let u = s.revert(&fold).unwrap();
        let undo = u.change.unwrap();
        assert_eq!((undo.subject.as_str(), undo.kind, undo.reverts.as_ref()), ("Undo: fold: Storage", ChangeKind::Undo, Some(&fold.commit)));
        assert!(!s.mem().join("notes/n.md").exists());
        assert_eq!(s.text("topics/live.md"), "a live chat's\n");
        let l = s.log();
        assert_eq!((l.len(), l[1].subject.as_str()), (before + 2, "Edited: topics/live.md"));
        let state = |c: &Change| s.log().into_iter().find(|x| x.commit == c.commit).unwrap().reverted_by;
        assert_eq!(state(&fold), Some(undo.commit.clone()));
        assert!(s.revert(&fold).unwrap_err().contains("already undone"));
        let redo = s.revert(&undo).unwrap().change.unwrap();
        assert_eq!((redo.subject.as_str(), redo.kind), ("Redo: fold: Storage", ChangeKind::Redo));
        assert_eq!(s.text("notes/n.md"), "note\n");
        assert_eq!((state(&fold), state(&undo)), (None, Some(redo.commit.clone())));
        let again = s.revert(&redo).unwrap().change.unwrap();
        assert_eq!(again.subject, "Undo: fold: Storage");
        assert_eq!((state(&redo), state(&undo), state(&fold)), (Some(again.commit), None, Some(undo.commit)));
        let made = s.log().pop().unwrap();
        assert!(s.revert(&made).unwrap_err().contains("cannot be undone"));
    }

    #[test]
    fn a_conflict_changes_nothing_and_names_the_files() {
        let s = Scratch::made("conflict");
        s.put(INDEX, "one\n");
        let c = s.commit("fold: A");
        s.put(INDEX, "two\n");
        s.put("topics/t.md", "t\n");
        s.commit("later");
        let head = s.log()[0].commit.clone();
        let r = s.revert(&c).unwrap();
        assert_eq!((r.change, r.conflicts), (None, vec![INDEX.to_string()]));
        assert_eq!((s.text(INDEX).as_str(), s.log()[0].commit.as_str()), ("two\n", head.as_str()));
        assert_eq!(status_at(&s.root, &s.id).unwrap().dirty, Vec::<String>::new());
        assert!(!s.git_dir().join("REVERT_HEAD").exists(), "aborted");
        // Undone by hand: there is nothing left to undo, and nothing is left behind.
        s.put("topics/u.md", "u\n");
        let u = s.commit("u");
        fs::remove_file(s.mem().join("topics/u.md")).unwrap();
        s.commit("by hand");
        assert!(s.revert(&u).is_err_and(|e| e.contains("nothing left")));
        assert!(!s.git_dir().join("REVERT_HEAD").exists());
        // A revert the engine was killed in is put back before the next act.
        s.put(INDEX, "three\n");
        s.commit("three");
        assert!(!Repo::open(made(&s.root, &s.id).unwrap()).unwrap().run(&["revert", "--no-edit", &c.commit]).unwrap().status.success());
        assert!(s.git_dir().join("REVERT_HEAD").exists(), "the fixture is mid-revert");
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, None).unwrap(), None, "no conflict markers are committed");
        assert_eq!(s.text(INDEX), "three\n");
    }

    #[test]
    fn a_stamp_moves_with_a_files_mtime_or_size_and_never_with_a_dot_name() {
        let s = Scratch::made("stamp");
        let at = || stamp_at(&s.root, &s.id).unwrap();
        let a = at();
        s.commit_nothing();
        fs::write(s.mem().join(".x.0123456789abcdef.tmp"), "x").unwrap();
        assert_eq!(at(), a, "a commit and a temp move nothing");
        let p = s.mem().join(INDEX);
        let t = fs::metadata(&p).unwrap().modified().unwrap();
        fs::write(&p, "longer than it was, and by a lot more than one byte or two").unwrap();
        fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        let b = at();
        assert_eq!(advance(&a, b.clone()).1.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec![INDEX], "same mtime, new size");
        s.put("topics/t.md", "t\n");
        fs::remove_file(&p).unwrap();
        let c = at();
        let (kept, ch, rm) = advance(&b, c.clone());
        assert_eq!(kept, c);
        assert_eq!((ch.len(), ch[0].0.as_str(), rm), (1, "topics/t.md", vec![INDEX.to_string()]));
        assert_eq!(ch[0].1.mtime_ms, s.read("topics/t.md").unwrap().mtime_ms, "the mtime a read answers with");
        assert_eq!(ids_at(&s.root), vec![s.id.clone()]);
        fs::create_dir_all(s.root.join(".removed").join(crate::ulid::new()).join(DIR).join(".git")).unwrap();
        fs::create_dir_all(s.root.join("by-hand").join(DIR).join(".git")).unwrap();
        fs::create_dir_all(s.root.join(crate::ulid::new())).unwrap();
        assert_eq!(ids_at(&s.root), vec![s.id.clone()], "a removed workspace's and one with no memory are none");
    }

    impl Scratch {
        fn commit_nothing(&self) {
            self.git(&["commit", "--quiet", "--allow-empty", "--message=x"]);
        }
    }

    /// The file at `path` with `text` in it and the mtime it already had.
    fn same_mtime(s: &Scratch, path: &str, text: &str) {
        let p = s.mem().join(path);
        let t = fs::metadata(&p).unwrap().modified().unwrap();
        fs::write(&p, text).unwrap();
        fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), t, "the fixture keeps the mtime");
    }

    /// Two writers inside one mtime: a save over a base named by its bytes is
    /// refused, whatever the clock and the size say, and the watch sees both.
    #[test]
    fn an_equal_mtime_is_not_an_equal_text() {
        let s = Scratch::made("rev");
        for other in ["# Thxirs\n", "# Theirs, and longer\n"] {
            s.put(INDEX, "# Theirs\n");
            let d = s.read(INDEX).unwrap();
            let (m, size, rev) = seen_at(&s.root, &s.id, INDEX).unwrap();
            assert_eq!((m, size, rev.as_ref()), (d.mtime_ms, d.size, Some(&d.rev)));
            let before = stamp_at(&s.root, &s.id).unwrap();
            same_mtime(&s, INDEX, other);
            let now = stamp_at(&s.root, &s.id).unwrap();
            assert_eq!(advance(&before, now).1.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec![INDEX], "{other:?}");
            assert_ne!(seen_at(&s.root, &s.id, INDEX).unwrap().2, Some(d.rev.clone()));
            let e = write_at(&s.root, &s.id, INDEX, "# Mine\n", &Base::Rev(d.rev), false).unwrap_err();
            assert!(e.starts_with(ERR_STALE), "{e}");
            assert_eq!(s.text(INDEX), other);
        }
        // The other way round: the bytes are the ones read, so a touch is no
        // conflict, and a save answers with the rev the next one names.
        let d = s.read(INDEX).unwrap();
        assert_eq!(index_at(&s.root, &s.id).unwrap().unwrap().rev, d.rev);
        fs::File::options().write(true).open(s.mem().join(INDEX)).unwrap().set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(9)).unwrap();
        let w = write_at(&s.root, &s.id, INDEX, "# Mine\n", &Base::Rev(d.rev.clone()), false).unwrap();
        assert_eq!(w.rev, s.read(INDEX).unwrap().rev);
        assert_eq!(write_at(&s.root, &s.id, INDEX, "# Mine\n", &Base::Rev(d.rev), false).unwrap().rev, w.rev, "what is already there is nothing lost");
        assert_eq!(write_at(&s.root, &s.id, INDEX, "# 2\n", &Base::Rev(w.rev), false).unwrap().size, 4);
        assert!(write_at(&s.root, &s.id, "topics/gone.md", "x", &Base::Rev("0".into()), false).unwrap_err().starts_with(ERR_GONE));
        // Too large to open is too large to name.
        s.put("big.md", &"x".repeat(MAX_TEXT_BYTES as usize + 1));
        assert_eq!(seen_at(&s.root, &s.id, "big.md").unwrap().2, None);
        assert!(write_at(&s.root, &s.id, "big.md", "x", &Base::Rev(rev_of(b"x")), false).is_err());
    }

    fn committed(s: &Scratch) -> Vec<String> {
        s.commit("all").files.iter().map(|f| f.path.clone()).collect()
    }
    fn listed(s: &Scratch) -> Vec<String> {
        list_at(&s.root, &s.id).unwrap().folders.into_iter().flat_map(|f| f.files).map(|f| f.path).collect()
    }
    fn watched(s: &Scratch) -> Vec<String> {
        stamp_at(&s.root, &s.id).unwrap().files.into_keys().collect()
    }
    fn skipped(s: &Scratch) -> Vec<String> {
        list_at(&s.root, &s.id).unwrap().skipped.into_iter().map(|x| x.path).collect()
    }

    /// Keep mine: the replaced bytes are in the history exactly, whatever the
    /// folder's ignore rules, its attributes or the index say, and alone.
    #[test]
    fn keep_mine_puts_the_very_bytes_it_replaces_in_the_history_and_nothing_else() {
        let s = Scratch::made("keep");
        s.put(".gitignore", "topics/draft.md\n");
        s.put(".gitattributes", "*.md text eol=lf ident working-tree-encoding=UTF-16\n");
        s.put("topics/other.md", "staged\n");
        s.git(&["-c", "core.attributesFile=/dev/null", "update-index", "--add", "--", "topics/other.md"]);
        let keep = |path: &str, text: &str| {
            let d = s.read(path).unwrap();
            write_at(&s.root, &s.id, path, text, &Base::Rev(d.rev), true)
        };
        for (path, theirs) in [("topics/draft.md", "theirs\n"), (INDEX, "a\r\nb $Id$\r\n"), ("notes/n.md", "first\n"), ("notes/n.md", "")] {
            s.put(path, theirs);
            let n = s.log().len();
            keep(path, "mine\n").unwrap();
            let top = s.log().remove(0);
            assert_eq!(s.log().len(), n + 1, "{path}");
            assert_eq!(top.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec![path], "that file and no other");
            assert_eq!(read_commit_at(&s.root, &s.id, path, &top.commit).unwrap().content, theirs);
            assert_eq!(s.text(path), "mine\n");
        }
        // What the history already holds is not committed again.
        assert!(committed(&s).contains(&"topics/other.md".to_string()));
        let n = s.log().len();
        keep("topics/draft.md", "mine again\n").unwrap();
        assert_eq!(s.log().len(), n);
        // A folder where the history has a file.
        s.put("topics/d", "a file\n");
        s.put("topics/d.md", "x\n");
        s.commit("d");
        fs::remove_file(s.mem().join("topics/d")).unwrap();
        s.put("topics/d/in.md", "theirs\n");
        keep("topics/d/in.md", "mine\n").unwrap();
        assert_eq!(read_commit_at(&s.root, &s.id, "topics/d/in.md", &s.log()[0].commit).unwrap().content, "theirs\n");
        // Nothing can be put in the history: nothing is replaced.
        s.put(INDEX, "theirs\n");
        let lock = s.git_dir().join("refs/heads/main.lock");
        fs::write(&lock, "").unwrap();
        assert!(keep(INDEX, "mine\n").is_err());
        assert_eq!(s.text(INDEX), "theirs\n");
        fs::remove_file(&lock).unwrap();
        assert!(fs::read_dir(s.git_dir()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with("keep.")));
    }

    #[test]
    fn a_text_written_while_a_save_is_made_is_not_replaced() {
        let s = Scratch::made("between");
        for keep in [false, true] {
            s.put(INDEX, "theirs\n");
            let d = s.read(INDEX).unwrap();
            let e = write_with(&s.root, &s.id, INDEX, "mine\n", &Base::Rev(d.rev), keep, &|| s.put(INDEX, "later\n")).unwrap_err();
            assert!(e.starts_with(ERR_STALE), "{e}");
            assert_eq!(s.text(INDEX), "later\n");
        }
    }

    /// The list, the watch and the history hold the same files, whatever they
    /// are called and whatever the folder says to ignore.
    #[test]
    fn the_list_the_watch_and_the_history_hold_the_same_files() {
        let s = Scratch::made("one-set");
        s.put(".secret.md", "x");
        s.put("topics/.hidden/x.md", "x");
        s.put(".gitignore", "*.txt\n");
        s.put("topics/ignored.txt", "x");
        symlink("/etc/hosts", s.mem().join("topics/l.md")).unwrap();
        symlink(s.base.clone(), s.mem().join("away")).unwrap();
        s.put("topics/a\nb.md", "x");
        s.put("topics/c\\d.md", "x");
        s.put("topics/e\u{301}.md", "x");
        s.put("--all", "x");
        s.put(":(glob)*.md", "x");
        s.put("topics/clone/in.md", "x");
        fs::create_dir_all(s.mem().join("topics/clone/.git")).unwrap();
        s.put("a/b/c/d/e/f/g/seven.md", "x");
        s.put("a/b/c/d/e/f/g/h/deep.md", "x");
        fs::File::create(s.mem().join("files/big.bin")).unwrap().set_len(MAX_COMMIT_BYTES + 1).unwrap();
        // Staged by hand: in none of them either.
        s.git(&["update-index", "--add", ".secret.md", "topics/l.md"]);
        let mut want = vec!["--all", ":(glob)*.md", "a/b/c/d/e/f/g/seven.md", "topics/e\u{301}.md", "topics/ignored.txt"];
        want.sort();
        assert_eq!(committed(&s), want);
        want.push(INDEX);
        want.sort();
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        assert_eq!((sorted(listed(&s)), sorted(watched(&s))), (want.iter().map(|x| x.to_string()).collect(), want.iter().map(|x| x.to_string()).collect()));
        assert_eq!(skipped(&s), vec!["a/b/c/d/e/f/g/h", "away", "files/big.bin", "topics/a\nb.md", "topics/c\\d.md", "topics/clone", "topics/l.md"]);
        assert_eq!(status_at(&s.root, &s.id).unwrap().dirty, Vec::<String>::new(), "and nothing stays dirty for good");
        assert_eq!(log_at(&s.root, &s.id, Some(":(glob)*.md"), None, None).unwrap().changes.len(), 1);
        s.git(&["config", "core.precomposeunicode", "true"]);
        for p in &want {
            s.read(p).unwrap();
            assert_eq!(log_at(&s.root, &s.id, Some(p), None, None).unwrap().changes.len(), 1, "{p}");
        }
    }

    /// What the history holds and the memory no longer keeps is not removed
    /// from it; what is gone is, by any spelling.
    #[test]
    fn only_a_file_that_is_gone_is_removed_from_the_history() {
        let s = Scratch::made("removals");
        for p in ["topics/nested/t.md", "notes/grew.md", "notes/linked.md", "notes/went.md", "notes/dir", "Case.md", "x/y/z.md"] {
            s.put(p, p);
        }
        s.commit("all");
        let before = stamp_at(&s.root, &s.id).unwrap();
        fs::create_dir_all(s.mem().join("topics/nested/.git")).unwrap();
        fs::File::create(s.mem().join("notes/grew.md")).unwrap().set_len(MAX_COMMIT_BYTES + 1).unwrap();
        fs::remove_file(s.mem().join("notes/linked.md")).unwrap();
        symlink("/etc/hosts", s.mem().join("notes/linked.md")).unwrap();
        fs::remove_file(s.mem().join("notes/went.md")).unwrap();
        // A file became a folder, a folder a file, and a name changed its case.
        fs::remove_file(s.mem().join("notes/dir")).unwrap();
        s.put("notes/dir/in.md", "x");
        fs::remove_dir_all(s.mem().join("x")).unwrap();
        s.put("x", "x");
        fs::rename(s.mem().join("Case.md"), s.mem().join("case.md")).unwrap();
        let (now, _, removed) = advance(&before, stamp_at(&s.root, &s.id).unwrap());
        assert_eq!(removed, vec!["Case.md", "notes/dir", "notes/went.md", "x/y/z.md"]);
        let mut c: Vec<(String, String)> = s.commit("moved").files.into_iter().map(|f| (f.status, f.path)).collect();
        c.sort();
        let want = [("A", "case.md"), ("A", "notes/dir/in.md"), ("A", "x"), ("D", "Case.md"), ("D", "notes/dir"), ("D", "notes/went.md"), ("D", "x/y/z.md")];
        assert_eq!(c, want.map(|(a, b)| (a.to_string(), b.to_string())));
        assert_eq!(status_at(&s.root, &s.id).unwrap().dirty, Vec::<String>::new());
        // Really gone now, they are removed: they were kept until it was known.
        fs::remove_dir_all(s.mem().join("topics/nested")).unwrap();
        fs::remove_file(s.mem().join("notes/grew.md")).unwrap();
        fs::remove_file(s.mem().join("notes/linked.md")).unwrap();
        assert_eq!(advance(&now, stamp_at(&s.root, &s.id).unwrap()).2, vec!["notes/grew.md", "notes/linked.md", "topics/nested/t.md"]);
        assert_eq!(s.commit("gone").files.iter().map(|f| f.status.as_str()).collect::<Vec<_>>(), vec!["D", "D", "D"]);
    }

    /// At the limit: what the walk did not reach is in none of the three, is
    /// not taken for removed, and nothing over the cap slips in behind it.
    #[test]
    fn a_walk_cut_at_the_limit_is_incomplete_and_never_removes() {
        let s = Scratch::made("limit");
        for d in ["a", "b", "c"] {
            for i in 0..MAX_ENTRIES / 2 {
                s.put(&format!("{d}/{i}.md"), "x");
            }
            fs::File::create(s.mem().join(d).join("zbig.bin")).unwrap().set_len(MAX_COMMIT_BYTES + 1).unwrap();
        }
        assert!(list_at(&s.root, &s.id).unwrap().truncated);
        let before = stamp_at(&s.root, &s.id).unwrap();
        for i in 0..50 {
            s.put(&format!("a/0new{i}.md"), "x");
        }
        let (kept, changed, removed) = advance(&before, stamp_at(&s.root, &s.id).unwrap());
        assert_eq!((changed.len(), removed), (50, vec![]), "a file past the limit is not a removed one");
        let c = committed(&s);
        assert!(!c.iter().any(|p| p.contains("zbig")), "a file over the cap was committed");
        let (mut l, w) = (listed(&s), watched(&s));
        l.sort();
        assert_eq!(l, w);
        l.retain(|p| p != INDEX);
        assert_eq!(c, l);
        // More arrive before them, and what is pushed past the limit stays in
        // the history; one that is gone past it goes from it.
        for i in 0..50 {
            s.put(&format!("a/1new{i}.md"), "x");
        }
        let last = c.iter().filter(|p| p.starts_with("b/")).next_back().unwrap().clone();
        fs::remove_file(s.mem().join(&last)).unwrap();
        let d = s.commit("more");
        assert_eq!(d.files.iter().filter(|f| f.status == "D").map(|f| f.path.as_str()).collect::<Vec<_>>(), vec![last.as_str()]);
        // Room again: the walk is whole, and what it does not see is gone.
        for d in ["a", "c"] {
            fs::remove_dir_all(s.mem().join(d)).unwrap();
        }
        let whole = stamp_at(&s.root, &s.id).unwrap();
        assert!(!whole.truncated);
        let removed = advance(&kept, whole).2;
        assert!(removed.iter().all(|p| p.starts_with("a/") || *p == last) && removed.contains(&last), "{}", removed.len());
    }

    #[test]
    fn an_undo_never_writes_over_a_file_the_memory_does_not_keep() {
        let s = Scratch::made("in-the-way");
        s.put("notes/n.md", "note\n");
        s.commit("n");
        fs::remove_file(s.mem().join("notes/n.md")).unwrap();
        let gone = s.commit("gone");
        s.put(".gitignore", "notes/\n");
        fs::File::create(s.mem().join("notes/n.md")).unwrap().set_len(MAX_COMMIT_BYTES + 1).unwrap();
        assert!(s.revert(&gone).unwrap_err().contains("in the way"));
        assert_eq!(fs::metadata(s.mem().join("notes/n.md")).unwrap().len(), MAX_COMMIT_BYTES + 1);
    }

    /// A fold's commit is found by its attempt, and by nothing that only
    /// mentions it.
    #[test]
    fn a_folds_commit_is_found_by_its_attempt() {
        let s = Scratch::made("attempt");
        let (a, b) = (crate::ulid::new(), crate::ulid::new());
        let find = |x: &str| fold_commit_at(&s.root, &s.id, x).unwrap();
        s.put("topics/t.md", "0\n");
        s.commit(&format!("quotes Fold-attempt: {a}"));
        assert_eq!(find(&a), None);
        assert_eq!(commit_fold_at(&s.root, &s.id, "fold: A", &a).unwrap(), None, "clean");
        s.put("topics/t.md", "1\n");
        let c = commit_fold_at(&s.root, &s.id, "fold: A", &a).unwrap().unwrap();
        assert_eq!((c.subject.as_str(), c.kind, find(&a).map(|x| x.commit)), ("fold: A", ChangeKind::Fold, Some(c.commit.clone())));
        s.put("topics/t.md", "2\n");
        s.commit("later");
        s.revert(&s.log()[0].clone()).unwrap();
        assert_eq!(find(&a).unwrap().commit, c.commit);
        assert_eq!(find(&b), None);
        assert!(fold_commit_at(&s.root, &s.id, ".*").is_err() && commit_fold_at(&s.root, &s.id, "x", "a\nb").is_err());
    }

    #[test]
    fn the_head_read_from_its_files_is_gits_own() {
        let s = Scratch::new("head");
        assert_eq!(head_at(&s.root, &s.id), None);
        init_at(&s.root, &s.id, "x").unwrap();
        assert_eq!(head_at(&s.root, &s.id), Some(s.log()[0].commit.clone()));
        s.git(&["pack-refs", "--all"]);
        assert_eq!(head_at(&s.root, &s.id), Some(s.log()[0].commit.clone()), "packed");
        s.put("topics/t.md", "x");
        let c = s.commit("later");
        assert_eq!(head_at(&s.root, &s.id), Some(c.commit.clone()), "a loose one over a packed one");
        s.git(&["checkout", "--quiet", "--detach"]);
        assert_eq!(head_at(&s.root, &s.id), Some(c.commit));
        s.git(&["symbolic-ref", "HEAD", "refs/heads/none"]);
        assert_eq!(head_at(&s.root, &s.id), None);
    }

    // ── the earlier layout ─────────────────────────────────────────────────

    /// A memory as the earlier layout made it: its repository in `memory/.git`.
    fn earlier_layout(tag: &str) -> Scratch {
        let s = Scratch::new(tag);
        fs::create_dir_all(s.mem()).unwrap();
        s.put(INDEX, "# Memory\n");
        s.put("topics/t.md", "one\n");
        assert!(old_git(&s, &["init", "--quiet", "--template=", "--initial-branch=main"]));
        assert!(old_git(&s, &["add", "--all"]) && old_git(&s, &["commit", "--quiet", "--message=Memory made"]));
        s.put("topics/t.md", "two\n");
        assert!(old_git(&s, &["commit", "--quiet", "--all", "--message=fold: A"]));
        fs::write(s.root.join(&s.id).join(".memory.lock"), "").unwrap();
        s
    }
    fn old_git(s: &Scratch, args: &[&str]) -> bool {
        assert!(s.mem().starts_with(&s.base));
        let mut c = crate::childenv::command("git");
        for (k, v) in env_edits(std::env::vars_os().filter_map(|(k, _)| k.into_string().ok())) {
            match v {
                Some(v) => c.env(k, v),
                None => c.env_remove(k),
            };
        }
        c.env_remove("GIT_LITERAL_PATHSPECS").arg("-C").arg(s.mem()).arg(format!("--git-dir={}", s.mem().join(".git").display()));
        c.arg(format!("--work-tree={}", s.mem().display())).args(["-c", "user.name=worktrees", "-c", "user.email=memory@localhost"]).args(args).output().unwrap().status.success()
    }
    fn subjects(s: &Scratch) -> Vec<String> {
        s.log().into_iter().map(|c| c.subject).collect()
    }
    fn engine_holds(s: &Scratch) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(s.git_dir()).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
        v.sort();
        v
    }

    /// Whatever is asked first moves it: the commits come, and nothing else
    /// that was in the folder a chat writes.
    #[test]
    fn an_earlier_memory_is_moved_on_first_touch_and_only_its_commits_come() {
        type Act = fn(&Scratch);
        let acts: [(&str, Act); 6] = [
            ("log", |s| assert_eq!(subjects(s).len(), 2)),
            ("init", |s| assert!(init_at(&s.root, &s.id, "x").unwrap().made)),
            ("status", |s| assert!(status_at(&s.root, &s.id).unwrap().made)),
            ("commit", |s| assert_eq!(commit_if_dirty_at(&s.root, &s.id, None).unwrap(), None)),
            ("undo", |s| assert!(s.revert(&s.log()[0].clone()).is_ok())),
            ("keep", |s| {
                s.put(INDEX, "theirs\n");
                let d = s.read(INDEX).unwrap();
                write_at(&s.root, &s.id, INDEX, "mine\n", &Base::Rev(d.rev), true).unwrap();
            }),
        ];
        for (tag, act) in acts {
            let s = earlier_layout(tag);
            assert!(old_git(&s, &["update-index", "--assume-unchanged", "topics/t.md"]));
            let old = s.mem().join(".git");
            let ran = s.base.join("ran");
            let cmd = |n: &str| format!("touch {}-{n}; cat", ran.display());
            fs::create_dir_all(old.join("hooks")).unwrap();
            fs::create_dir_all(old.join("info")).unwrap();
            for h in ["pre-commit", "post-checkout", "reference-transaction", "post-index-change"] {
                fsx::exec_fixture(&old.join("hooks").join(h), &format!("#!/bin/sh\ntouch {}-{h}\n", ran.display()));
            }
            let mut cfg = fs::read_to_string(old.join("config")).unwrap();
            cfg += &format!("[hook \"x\"]\n\tevent = pre-commit\n\tcommand = \"{}\"\n[filter \"f\"]\n\tclean = \"{}\"\n\tsmudge = \"{}\"\n[core]\n\tfsmonitor = \"{}\"\n\tworktree = {}\n", cmd("hook"), cmd("clean"), cmd("smudge"), cmd("fs"), s.base.display());
            fs::write(old.join("config"), cfg).unwrap();
            fs::write(old.join("info/attributes"), "* filter=f\n").unwrap();
            fs::create_dir_all(old.join("objects/info")).unwrap();
            fs::write(old.join("objects/info/alternates"), format!("{}\n", s.base.join("elsewhere").display())).unwrap();
            fs::write(old.join("shallow"), "").unwrap();
            assert_eq!(ids_at(&s.root), vec![s.id.clone()], "the poll knows it before");
            act(&s);
            assert!(!old.exists(), "{tag}");
            let held = engine_holds(&s);
            assert!(held.iter().all(|n| ["COMMIT_EDITMSG", "HEAD", "ORIG_HEAD", "config", "index", "info", "logs", "objects", "packed-refs", "refs"].contains(&n.as_str())), "{tag}: {held:?}");
            assert_eq!(fs::read_to_string(s.git_dir().join("config")).unwrap(), CONFIG);
            assert!(!s.git_dir().join("objects/info/alternates").exists());
            let l = subjects(&s);
            assert_eq!(l[l.len() - 2..], ["fold: A".to_string(), "Memory made".into()]);
            assert!(!s.root.join(&s.id).join(".memory.lock").exists());
            let left: Vec<_> = fs::read_dir(&s.base).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().starts_with("ran")).collect();
            assert!(left.is_empty(), "{tag}: these ran: {left:?}");
            s.put("topics/new.md", "x\n");
            s.put("topics/t.md", "a change the old index was told to hide\n");
            let c = committed(&s);
            assert!(["topics/new.md", "topics/t.md"].iter().all(|p| c.iter().any(|x| x == p)), "{tag}: {c:?}");
        }
        // Two at once: one moves it, the other finds it moved.
        let s = earlier_layout("at-once");
        let go = std::sync::Barrier::new(6);
        std::thread::scope(|t| {
            let all: Vec<_> = (0..6).map(|_| t.spawn(|| { go.wait(); subjects(&s).len() })).collect();
            assert!(all.into_iter().all(|h| h.join().unwrap() == 2));
        });
    }

    /// What cannot be moved is left where it was, and said; the files are
    /// still there to read.
    #[test]
    fn a_history_that_cannot_be_moved_is_left_where_it_is() {
        let at = |s: &Scratch, p: &str| s.mem().join(".git").join(p);
        type Spoil = fn(&Scratch);
        let spoils: [(&str, Spoil); 7] = [
            ("link", |s| symlink("/etc/hosts", s.mem().join(".git/objects/zz")).unwrap()),
            ("refs", |s| {
                fs::rename(s.mem().join(".git/refs"), s.base.join("refs")).unwrap();
                symlink(s.base.join("refs"), s.mem().join(".git/refs")).unwrap();
            }),
            ("head", |s| fs::write(s.mem().join(".git/HEAD"), "ref: refs/heads/../../x\n").unwrap()),
            ("format", |s| fs::write(s.mem().join(".git/config"), "[core]\n\trepositoryformatversion = 1\n[Extensions]\n\tobjectformat = sha256\n").unwrap()),
            ("empty", |s| fs::remove_dir_all(s.mem().join(".git/objects")).unwrap()),
            ("unborn", |s| fs::write(s.mem().join(".git/HEAD"), "ref: refs/heads/none\n").unwrap()),
            ("broken", |s| {
                let d = s.mem().join(".git/objects");
                let first = fs::read_dir(&d).unwrap().flatten().find(|e| e.file_name().len() == 2).unwrap().path();
                fs::remove_dir_all(first).unwrap();
            }),
        ];
        for (tag, spoil) in spoils {
            let s = earlier_layout(tag);
            spoil(&s);
            for _ in 0..2 {
                let e = commit_if_dirty_at(&s.root, &s.id, None).unwrap_err();
                assert!(e.contains("left where it is"), "{tag}: {e}");
            }
            assert!(at(&s, "HEAD").exists() && !s.git_dir().exists(), "{tag}");
            assert!(fs::symlink_metadata(s.git_dir().with_file_name(MOVING)).is_err(), "{tag}");
            assert_eq!(s.read("topics/t.md").unwrap().content, "two\n");
            assert_eq!(list_at(&s.root, &s.id).unwrap().count, 2);
        }
    }

    #[test]
    fn a_move_that_was_cut_off_is_finished_and_two_histories_are_not_guessed_between() {
        let s = earlier_layout("cut");
        let moving = wsfolder::make_engine_at(&s.root, &s.id).unwrap().join(MOVING);
        fs::rename(s.mem().join(".git"), &moving).unwrap();
        assert_eq!(ids_at(&s.root), vec![s.id.clone()], "it is not a workspace with no memory");
        assert_eq!(subjects(&s).len(), 2);
        assert!(!moving.exists() && s.git_dir().is_dir());

        let s = earlier_layout("two");
        let moving = wsfolder::make_engine_at(&s.root, &s.id).unwrap().join(MOVING);
        fs::create_dir_all(&moving).unwrap();
        fs::write(moving.join("mine"), "").unwrap();
        assert!(commit_if_dirty_at(&s.root, &s.id, None).unwrap_err().contains("two histories"));
        assert!(s.mem().join(".git/HEAD").exists() && moving.join("mine").exists() && !s.git_dir().exists());

        // A `.git` that appears in a memory already moved is nobody's history
        // of it: the engine's stands, the other is left alone and named.
        let s = Scratch::made("later");
        fs::create_dir_all(s.mem().join(".git/objects")).unwrap();
        fs::write(s.mem().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        s.put("topics/t.md", "x\n");
        assert_eq!(committed(&s), vec!["topics/t.md"]);
        assert_eq!(skipped(&s), vec![".git"]);
        assert!(s.mem().join(".git/HEAD").exists());
    }

    /// An undo the earlier engine was killed in is taken back as it is moved:
    /// no conflict marker is ever committed.
    #[test]
    fn an_earlier_memory_killed_in_an_undo_is_put_back() {
        let s = earlier_layout("mid-undo");
        s.put("topics/t.md", "three\n");
        assert!(old_git(&s, &["commit", "--quiet", "--all", "--message=later"]));
        assert!(!old_git(&s, &["revert", "--no-edit", "HEAD~1"]));
        assert!(s.mem().join(".git/REVERT_HEAD").exists() && s.text("topics/t.md").contains("<<<<"));
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, None).unwrap(), None);
        assert_eq!((s.text("topics/t.md").as_str(), subjects(&s).len()), ("three\n", 3));
        assert!(!s.git_dir().join("REVERT_HEAD").exists());
    }

    /// The folder, or the memory, replaced by a link: nothing is reached
    /// through it. Replaced between an act's steps: the act is on the folder
    /// it opened.
    #[test]
    fn the_folder_is_opened_once_and_never_through_a_link() {
        let s = Scratch::made("pin");
        let out = s.base.join("out");
        fs::create_dir_all(out.join("topics")).unwrap();
        fs::write(out.join("secret.md"), "secret").unwrap();
        fs::write(out.join("topics/t.md"), "secret").unwrap();
        s.put("topics/t.md", "ours\n");
        let repo = Repo::open(made(&s.root, &s.id).unwrap()).unwrap();
        fs::rename(s.mem(), s.base.join("was")).unwrap();
        symlink(&out, s.mem()).unwrap();
        let c = commit_held(&repo, Some("x")).unwrap().unwrap();
        assert_eq!(c.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["topics/t.md"]);
        assert_eq!(repo.text(&["show", "HEAD:topics/t.md"]).unwrap(), "ours\n");
        drop(repo);
        // Nothing begins there.
        assert!(s.read("secret.md").is_err() && list_at(&s.root, &s.id).is_err() && stamp_at(&s.root, &s.id).is_err());
        assert!(s.write("n.md", "x", None).is_err() && put_file_at(&s.root, &s.id, "n.png", &png(1)).is_err());
        assert!(ids_at(&s.root).is_empty() && init_at(&s.root, &s.id, "x").is_err());
        assert!(commit_if_dirty_at(&s.root, &s.id, None).unwrap_or(None).is_none());
        assert_eq!(fs::read_dir(&out).unwrap().count(), 2, "nothing was made there");

        // A workspace's folder that is a link is none.
        let s = Scratch::made("pin-folder");
        let id = crate::ulid::new();
        let other = s.base.join("other");
        fs::rename(s.root.join(&s.id), &other).unwrap();
        symlink(&other, s.root.join(&id)).unwrap();
        fs::rename(s.root.join(".engine").join(&s.id), s.root.join(".engine").join(&id)).unwrap();
        assert!(read_at(&s.root, &id, INDEX).is_err() && list_at(&s.root, &id).is_err() && ids_at(&s.root).is_empty());
        assert!(write_at(&s.root, &id, "topics/n.md", "x", &Base::None, false).is_err() && init_at(&s.root, &id, "x").is_err());
        assert!(!other.join("memory/topics/n.md").exists());

        // Nor is an engine's half that is one.
        let s = Scratch::made("pin-engine");
        let away = s.base.join("away");
        fs::rename(s.root.join(".engine").join(&s.id), &away).unwrap();
        symlink(&away, s.root.join(".engine").join(&s.id)).unwrap();
        s.put("topics/t.md", "x");
        assert!(ids_at(&s.root).is_empty());
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, None).unwrap(), None);
    }

    /// The folder went and its history did not: the files are put back as the
    /// history has them, and nothing is committed as removed. One that is only
    /// emptied is as somebody left it.
    #[test]
    fn a_memory_whose_folder_went_is_put_back_from_its_history() {
        let s = Scratch::made("went");
        s.put("topics/t.md", "a\r\n");
        s.put(".gitattributes", "*.md text eol=lf\n");
        s.commit("t");
        let n = s.log().len();
        for gone in [s.mem(), s.root.join(&s.id)] {
            fs::remove_dir_all(&gone).unwrap();
            assert!(!status_at(&s.root, &s.id).unwrap().made && commit_if_dirty_at(&s.root, &s.id, None).unwrap().is_none());
            assert_eq!(init_at(&s.root, &s.id, "Another").unwrap(), Status { made: true, dirty: vec![] });
            assert_eq!((s.text("topics/t.md").as_str(), s.text(INDEX).starts_with("# Memory: Personal"), s.log().len()), ("a\r\n", true, n));
            assert!(s.mem().join("notes").is_dir() && s.mem().join(FILES).is_dir());
        }
        for f in [INDEX, "topics/t.md"] {
            fs::remove_file(s.mem().join(f)).unwrap();
        }
        assert_eq!(init_at(&s.root, &s.id, "Another").unwrap().dirty, vec![INDEX, "topics/t.md"]);
    }

    /// What is undone is read once for a head, and again for another: a commit,
    /// an undo and a redo each make one. A failure is kept for none.
    #[test]
    fn what_is_undone_is_read_once_a_head() {
        let c = ByHead(Mutex::new(VecDeque::new()));
        let reads = std::cell::Cell::new(0);
        let of = |head: &str, ok: bool| {
            c.of(head, || {
                reads.set(reads.get() + 1);
                if ok { Ok(Undone::from([(head.to_string(), "by".to_string())])) } else { Err("no".into()) }
            })
        };
        assert!(of("a", false).is_err() && of("a", true).unwrap().contains_key("a"));
        assert!(of("a", true).unwrap().contains_key("a") && of("b", true).unwrap().contains_key("b"));
        assert_eq!(reads.get(), 3);
        for i in 0..HEADS {
            of(&format!("h{i}"), true).unwrap();
        }
        assert_eq!(c.0.lock().unwrap().len(), HEADS, "and no more are kept");
        of("a", true).unwrap();
        assert_eq!(reads.get(), 3 + HEADS + 1, "the oldest went");
    }

    /// What a page of History costs over a long one, measured:
    /// `cargo test -p worktrees-core --release memory_history_cost -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn memory_history_cost() {
        let s = Scratch::made("history-cost");
        let repo = s.repo();
        // Every tenth an undo of the one before, as git's own stream takes them.
        let mut stream = String::new();
        let head = repo.text(&["rev-parse", "HEAD"]).unwrap();
        stream += &format!("reset refs/heads/main\nfrom {}\n", head.trim());
        for i in 0..5000 {
            let msg = if i % 10 == 9 { format!("Undo: e{i}\n\n{REVERTS}{}.", "0".repeat(39) + "1") } else { format!("Edited: topics/t{}.md", i % 40) };
            let text = format!("{i}\n");
            stream += &format!("commit refs/heads/main\ncommitter w <m@l> {} +0000\ndata {}\n{msg}\nM 100644 inline topics/t{}.md\ndata {}\n{text}\n", 1_700_000_000 + i, msg.len(), i % 40, text.len());
        }
        repo.feed(&["fast-import", "--quiet", "--force"], stream.as_bytes(), None).unwrap();
        repo.bytes(&["read-tree", "HEAD"]).unwrap();
        let time = |what: &str, f: &dyn Fn()| {
            let t = std::time::Instant::now();
            f();
            let first = t.elapsed();
            let t = std::time::Instant::now();
            for _ in 0..10 {
                f();
            }
            println!("{what}: {first:?}, then {:?}", t.elapsed() / 10);
        };
        let log = |path: Option<&str>, before: Option<&str>, n: usize| log_at(&s.root, &s.id, path, before, Some(n)).unwrap();
        let deep = repo.text(&["rev-list", "--max-count=1", "--skip=4500", "HEAD"]).unwrap().trim().to_string();
        time("the newest change (workspace_memory)", &|| assert_eq!(log(None, None, 1).changes.len(), 1));
        time("a first page of 50", &|| assert_eq!(log(None, None, 50).changes.len(), 50));
        time("a page of 50, 4500 commits down", &|| assert_eq!(log(None, Some(&deep), 50).changes.len(), 50));
        time("a page of one file's", &|| assert_eq!(log(Some("topics/t7.md"), None, 50).changes.len(), 50));
        time("one change (a diff's head)", &|| assert!(show_at(&s.root, &s.id, &deep, None).is_ok()));
        time("the undone (a chat list's)", &|| assert!(undone_at(&s.root, &s.id).is_ok()));
    }

    /// The poll's cost, measured: `cargo test -p worktrees-core --release
    /// memory_poll_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn memory_poll_cost() {
        let s = Scratch::new("cost");
        let mut ids = Vec::new();
        for _ in 0..20 {
            let id = crate::ulid::new();
            let mem = dir_at(&s.root, &id).unwrap();
            fs::create_dir_all(&mem).unwrap();
            found(&wsfolder::make_engine_at(&s.root, &id).unwrap().join(GIT)).unwrap();
            for i in 0..200 {
                let d = mem.join(["notes", "topics", "files", ""][i % 4]);
                fs::create_dir_all(&d).unwrap();
                fs::write(d.join(format!("f{i}.md")), "x").unwrap();
            }
            ids.push(id);
        }
        let t = std::time::Instant::now();
        for _ in 0..50 {
            for id in ids_at(&s.root) {
                assert_eq!(stamp_at(&s.root, &id).unwrap().len(), 200);
            }
        }
        println!("20 workspaces x 200 files: {:?} a tick", t.elapsed() / 50);
    }
}
