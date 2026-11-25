//! A workspace's working memory: `<workspace folder>/memory/`, a git
//! repository only the engine commits (`docs/proposals/workspace-chats.md`
//! §2, §2.5, revision 3's §9).
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
//! - by `--git-dir` and `--work-tree`, never by discovery. A `memory/` whose
//!   `.git` has gone would otherwise resolve UPWARD to whatever repository
//!   holds the data dir (a dotfiles one), and `add -A` would commit into it.
//!   The same two beat a `core.worktree` in the repository's config.
//! - with every `GIT_*` variable dropped from the child's environment, and
//!   with no global and no system config.
//! - with **nothing the folder holds able to run**. An agent writes under this
//!   folder and `.git` is in it. `core.hooksPath=/dev/null` is NOT enough: git
//!   2.54 runs `hook.<name>.command` from the repository's config under it
//!   (measured), and `core.fsmonitor` and `filter.<name>.clean` run on a plain
//!   `add`. So the repository's effective config is listed first and each name
//!   in a family that runs a command is switched off by `-c`
//!   ([`neutralized`]). The repository keeps working and nothing is written to
//!   its config.
//! - with `--` before every path a client supplied, and as a literal pathspec.
//! - with no transport: `GIT_ALLOW_PROTOCOL` names none, and nothing here ever
//!   adds a remote.
//!
//! **A save is not a commit** (§9 item 2). [`write_at`] only writes; the poll
//! commits a dirty tree after a quiet minute, and so does whatever starts a
//! chat, a fold or a revert ([`commit_if_dirty_at`]).
//!
//! **What a commit leaves out** (and [`list_at`] names in `skipped`): a folder
//! holding a repository of its own (git would take it as a link, and looking
//! inside it runs ITS config), and a file over [`MAX_COMMIT_BYTES`].

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::{fsx, safepath, wsfolder};

pub const DIR: &str = "memory";
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
    /// Present on disk and not in `folders`, or in it and out of the history.
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

/// `path` as an existing regular file inside `mem`. `safe_under` keeps it in
/// the folder; what it does not know is `.git`, which a link inside the folder
/// (`x -> .git`) would reach without leaving it.
fn existing(mem: &Path, path: &str) -> Result<PathBuf, String> {
    rel_ok(path)?;
    let f = safepath::safe_under(mem, path).ok_or_else(|| match fs::symlink_metadata(mem.join(path)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => format!("{ERR_GONE}: {path}"),
        _ => format!("not a file of this memory: {path}"),
    })?;
    let canon = fs::canonicalize(mem).map_err(|e| format!("{}: {e}", mem.display()))?;
    let inside = f.strip_prefix(&canon).map_err(|_| format!("not a file of this memory: {path}"))?;
    if inside.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
        return Err(format!("not a file of this memory: {path}"));
    }
    Ok(f)
}

/// `path`'s folder inside `mem`, the missing ones made and each a real
/// directory (`fsx::make_dir`). Never `create_dir_all`, which follows a link
/// it meets on the way.
fn parent_of(mem: &Path, path: &str) -> Result<PathBuf, String> {
    rel_ok(path)?;
    let mut cur = fs::canonicalize(mem).map_err(|e| format!("{}: {e}", mem.display()))?;
    let parts: Vec<&str> = path.split('/').collect();
    for p in &parts[..parts.len() - 1] {
        cur.push(p);
        fsx::make_dir(&cur).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(cur)
}

fn mtime_ms(md: &fs::Metadata) -> u64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
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

fn sig_of(md: &fs::Metadata) -> Sig {
    use std::os::unix::fs::MetadataExt;
    Sig { mtime_ms: mtime_ms(md), size: md.len(), changed_ns: i128::from(md.ctime()) * 1_000_000_000 + i128::from(md.ctime_nsec()), ino: md.ino() }
}

#[derive(Default)]
struct Walk {
    files: Vec<Found>,
    skipped: Vec<Skipped>,
    truncated: bool,
}

/// Every regular file under `mem`: one `readdir` a folder and one `lstat` an
/// entry, no link followed, no dot name entered, no `.git` at any depth.
fn walk(mem: &Path) -> std::io::Result<Walk> {
    let mut out = Walk::default();
    let mut seen = 0usize;
    let mut todo: Vec<(String, usize)> = vec![(String::new(), 0)];
    while let Some((rel, depth)) = todo.pop() {
        let dir = if rel.is_empty() { mem.to_path_buf() } else { mem.join(&rel) };
        let entries = match fs::read_dir(&dir) {
            Ok(r) => r.flatten().collect::<Vec<_>>(),
            // The folder itself must be readable, or an unreadable one would
            // read as "everything was removed".
            Err(e) if rel.is_empty() => return Err(e),
            Err(_) => {
                out.skipped.push(Skipped { path: rel, why: "cannot be read" });
                continue;
            }
        };
        if !rel.is_empty() && entries.iter().any(|e| e.file_name().eq_ignore_ascii_case(".git")) {
            out.skipped.push(Skipped { path: rel, why: "holds a git repository of its own" });
            continue;
        }
        for e in entries {
            let name = e.file_name();
            let Some(name) = name.to_str() else {
                out.skipped.push(Skipped { path: format!("{rel}/{}", name.to_string_lossy()), why: "its name is not UTF-8" });
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            seen += 1;
            if seen > MAX_ENTRIES {
                out.truncated = true;
                return Ok(out);
            }
            let path = if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") };
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                if depth + 1 >= MAX_DEPTH {
                    out.skipped.push(Skipped { path, why: "too deep" });
                } else {
                    todo.push((path, depth + 1));
                }
            } else if md.is_file() {
                out.files.push(Found { path, sig: sig_of(&md) });
            } else {
                out.skipped.push(Skipped { path, why: "not a regular file" });
            }
        }
    }
    Ok(out)
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

/// Every file's. A missing folder is no files; a folder that cannot be read is
/// an error, never "all removed".
pub type Stamp = BTreeMap<String, Sig>;

pub fn stamp_at(root: &Path, id: &str) -> Result<Stamp, String> {
    let mem = dir_at(root, id)?;
    match walk(&mem) {
        Ok(w) => Ok(w.files.into_iter().map(|f| (f.path, f.sig)).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stamp::new()),
        Err(e) => Err(format!("{}: {e}", mem.display())),
    }
}

/// What moved between two stamps: `(changed, removed)`.
pub fn changed_since(before: &Stamp, now: &Stamp) -> (Vec<(String, Sig)>, Vec<String>) {
    let changed = now.iter().filter(|(p, s)| before.get(*p) != Some(s)).map(|(p, s)| (p.clone(), *s)).collect();
    let removed = before.keys().filter(|p| !now.contains_key(*p)).cloned().collect();
    (changed, removed)
}

/// The workspaces under `root` that have a memory: one `readdir`, and a ULID
/// name, so `.removed/` is none of them.
pub fn ids_at(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root)
        .map(|r| r.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| crate::ulid::is_valid(n)).collect())
        .unwrap_or_default();
    v.retain(|id| is_made(&root.join(id).join(DIR)));
    v.sort();
    v
}

// ── git ─────────────────────────────────────────────────────────────────────

fn real_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

/// A real folder with a real `.git` folder: a `.git` FILE or link names a
/// repository somewhere else.
fn is_made(mem: &Path) -> bool {
    real_dir(mem) && real_dir(&mem.join(".git"))
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

/// One memory's repository, opened: what its config would run is known and off.
struct Repo {
    mem: PathBuf,
    off: Vec<String>,
}

impl Repo {
    fn open(mem: &Path) -> Result<Repo, String> {
        if !is_made(mem) {
            return Err("this workspace has no memory yet".into());
        }
        // A `commondir` sends the config, the refs and the objects elsewhere.
        if fs::symlink_metadata(mem.join(".git/commondir")).is_ok() {
            return Err("the memory's .git belongs to another repository".into());
        }
        let mut r = Repo { mem: mem.to_path_buf(), off: Vec::new() };
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
        c.arg("-C").arg(&self.mem);
        c.arg(format!("--git-dir={}", self.mem.join(".git").display()));
        c.arg(format!("--work-tree={}", self.mem.display()));
        c.args(["--no-pager", "--no-optional-locks"]);
        for f in FIXED.iter().copied().chain(self.off.iter().map(String::as_str)) {
            c.arg("-c").arg(f);
        }
        c.stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Result<Output, String> {
        self.command().args(args).output().map_err(|e| format!("git: {e}"))
    }

    fn bytes(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        let o = self.run(args)?;
        if !o.status.success() {
            let cmd = args.iter().find(|a| !a.starts_with('-')).unwrap_or(&"");
            return Err(format!("git {cmd} failed: {}", String::from_utf8_lossy(&o.stderr).trim()));
        }
        Ok(o.stdout)
    }

    fn text(&self, args: &[&str]) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.bytes(args)?).into_owned())
    }

    fn has_commits(&self) -> bool {
        self.run(&["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok_and(|o| o.status.success())
    }

    /// A revert this engine began and did not finish (it was killed between
    /// the two). Under the lock nothing else can be in one.
    fn recover(&self) {
        if fs::symlink_metadata(self.mem.join(".git/REVERT_HEAD")).is_ok() {
            let _ = self.run(&["revert", "--abort"]);
        }
    }

    /// What `add` and `status` are asked about: everything but what a commit
    /// leaves out.
    fn pathspecs(&self) -> Result<Vec<String>, String> {
        let w = walk(&self.mem).map_err(|e| format!("{}: {e}", self.mem.display()))?;
        let nested = w.skipped.iter().filter(|s| s.why.contains("repository")).map(|s| s.path.clone());
        let large = w.files.iter().filter(|f| f.sig.size > MAX_COMMIT_BYTES).map(|f| f.path.clone());
        Ok(std::iter::once(":/".to_string()).chain(nested.chain(large).map(|p| format!(":(top,exclude,literal){p}"))).collect())
    }

    fn dirty(&self, specs: &[String]) -> Result<Vec<String>, String> {
        let mut args = vec!["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=all", "--no-renames", "--"];
        args.extend(specs.iter().map(String::as_str));
        Ok(self.text(&args)?.split('\0').filter_map(|l| l.get(3..)).filter(|p| !p.is_empty()).map(String::from).collect())
    }
}

/// Held by whatever changes the repository or saves a file into it: a mutex in
/// this process and an flock for another engine.
struct Held {
    _file: fs::File,
    _serial: MutexGuard<'static, ()>,
}

static SERIAL: Mutex<()> = Mutex::new(());

fn hold(folder: &Path) -> Result<Held, String> {
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let p = folder.join(".memory.lock");
    let file = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    // SAFETY: flock gets a live descriptor, and closing it releases the lock
    // on every exit path.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!("could not lock {}", p.display()));
    }
    Ok(Held { _file: file, _serial: serial })
}

/// Run `f` with workspace `id`'s memory held still: no commit, undo or save of
/// this module's is in it, or starts in it, meanwhile. What moves the folder
/// uses it, or a commit would be cut in half by the move.
pub fn while_still_at<T>(root: &Path, id: &str, f: impl FnOnce() -> T) -> Result<T, String> {
    let folder = wsfolder::folder_at(root, id)?;
    let _held = if is_made(&folder.join(DIR)) { Some(hold(&folder)?) } else { None };
    Ok(f())
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
/// exactly as it is.
pub fn init_at(root: &Path, id: &str, name: &str) -> Result<Status, String> {
    wsfolder::refuse_real(root)?;
    let folder = wsfolder::folder_at(root, id)?;
    fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    fsx::make_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let held = hold(&folder)?;
    let mem = folder.join(DIR);
    fsx::make_dir(&mem).map_err(|e| format!("{}: {e}", mem.display()))?;
    let git = mem.join(".git");
    let fresh = match fs::symlink_metadata(&git) {
        Ok(m) if m.is_dir() => false,
        Ok(_) => return Err("the memory's .git is not a folder".into()),
        Err(_) => true,
    };
    let repo = Repo { mem: mem.clone(), off: Vec::new() };
    if fresh {
        // No template: a template dir is the person's, and may hold hooks.
        repo.bytes(&["init", "--quiet", "--template=", "--initial-branch=main"])?;
    }
    let repo = Repo::open(&mem)?;
    if !repo.has_commits() {
        // `.git` is the engine's: what a commit must never hold.
        let info = git.join("info");
        fsx::make_dir(&info).map_err(|e| format!("{}: {e}", info.display()))?;
        fsx::write_atomic(&info.join("exclude"), b".DS_Store\n.*.tmp\n", None).map_err(|e| e.to_string())?;
        for d in ["notes", "topics", FILES] {
            fsx::make_dir(&mem.join(d)).map_err(|e| format!("{d}: {e}"))?;
        }
        let index = mem.join(INDEX);
        if fs::symlink_metadata(&index).is_err() {
            fsx::write_atomic(&index, template(name).as_bytes(), None).map_err(|e| format!("{INDEX}: {e}"))?;
        }
        commit_held(&repo, Some(MADE))?;
    }
    drop(held);
    status_at(root, id)
}

pub fn status_at(root: &Path, id: &str) -> Result<Status, String> {
    let mem = dir_at(root, id)?;
    if !is_made(&mem) {
        return Ok(Status { made: false, dirty: Vec::new() });
    }
    let repo = Repo::open(&mem)?;
    Ok(Status { made: true, dirty: repo.dirty(&repo.pathspecs()?)? })
}

// ── read ────────────────────────────────────────────────────────────────────

/// A markdown file's first heading, from its first 4 KB. A temp that vanished
/// is no title.
fn peek_title(mem: &Path, path: &str) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut buf = Vec::new();
    let f = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(mem.join(path)).ok()?;
    f.take(4096).read_to_end(&mut buf).ok()?;
    crate::ideas::title_of(&String::from_utf8_lossy(&buf))
}

/// Every file, folded by top folder: `memory/`'s own first (the index before
/// the rest), then `notes`, `topics`, `files`, then any other by name. Notes
/// come newest first, by their dated names.
pub fn list_at(root: &Path, id: &str) -> Result<Listing, String> {
    let mem = dir_at(root, id)?;
    let w = match walk(&mem) {
        Ok(w) => w,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Listing::default()),
        Err(e) => return Err(format!("{}: {e}", mem.display())),
    };
    let mut out = Listing { count: w.files.len(), truncated: w.truncated, skipped: w.skipped, ..Listing::default() };
    let mut by: BTreeMap<String, Vec<File>> = BTreeMap::new();
    for f in w.files {
        out.bytes += f.sig.size;
        if f.sig.size > MAX_COMMIT_BYTES {
            out.skipped.push(Skipped { path: f.path.clone(), why: "too large for the history" });
        }
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

/// `f` opened without following a link, its first `cap` bytes, and what was
/// opened. The walk from check to open is closed by the handle's own identity.
fn open_read(f: &Path, path: &str, cap: u64) -> Result<(fs::Metadata, Vec<u8>), String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let want = fs::symlink_metadata(f).map_err(|_| format!("{ERR_GONE}: {path}"))?;
    let h = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(f)
        .map_err(|e| format!("{path}: {e}"))?;
    let md = h.metadata().map_err(|e| format!("{path}: {e}"))?;
    if !md.is_file() || (md.dev(), md.ino()) != (want.dev(), want.ino()) {
        return Err(format!("not a file of this memory: {path}"));
    }
    let mut buf = Vec::new();
    h.take(cap).read_to_end(&mut buf).map_err(|e| format!("{path}: {e}"))?;
    Ok((md, buf))
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
    let mem = dir_at(root, id)?;
    let f = existing(&mem, path)?;
    let (md, bytes) = open_read(&f, path, MAX_TEXT_BYTES + 1)?;
    let content = as_text(path, md.len(), bytes)?;
    Ok(Doc { path: path.into(), file: name_of(path), title: title_for(path, &content), mtime_ms: mtime_ms(&md), rev: rev_of(content.as_bytes()), size: md.len(), content })
}

/// What is at `path` now, for whoever is told it moved: its mtime, its size,
/// and [`Doc::rev`] for a file small enough to open here.
pub fn seen_at(root: &Path, id: &str, path: &str) -> Result<(u64, u64, Option<String>), String> {
    let f = existing(&dir_at(root, id)?, path)?;
    let (md, bytes) = open_read(&f, path, MAX_TEXT_BYTES + 1)?;
    let rev = (bytes.len() as u64 <= MAX_TEXT_BYTES).then(|| rev_of(&bytes));
    Ok((mtime_ms(&md), md.len(), rev))
}

/// `path` as it was in `commit`: what a restore puts back, and what an older
/// version reads as. `mtime_ms` is 0: it is not the file's.
pub fn read_commit_at(root: &Path, id: &str, path: &str, commit: &str) -> Result<Doc, String> {
    rel_ok(path)?;
    let repo = Repo::open(&dir_at(root, id)?)?;
    let commit = known(&repo, commit)?;
    let (size, bytes) = blob(&repo, &commit, path, MAX_TEXT_BYTES)?.ok_or_else(|| format!("{ERR_GONE}: {path} in that change"))?;
    let content = as_text(path, size, bytes)?;
    Ok(Doc { path: path.into(), file: name_of(path), title: title_for(path, &content), mtime_ms: 0, rev: rev_of(content.as_bytes()), size, content })
}

/// An image of this memory, under the chat images' rules
/// (`safepath::open_place_image`). `true`: `etag` still names it, no bytes.
pub fn image_at(root: &Path, id: &str, path: &str, etag: Option<&str>) -> Result<(safepath::ImageRead, bool), String> {
    let mem = dir_at(root, id)?;
    existing(&mem, path)?;
    safepath::open_place_image(&mem, false, path, MAX_FILE_BYTES as u64, etag, &|_| Ok(())).map_err(|e| e.replace("this place", "this memory"))
}

// ── write ───────────────────────────────────────────────────────────────────

/// The person's save, over the file `base` says they read: refused when that
/// is no longer it. `keep_history`: commit what is on disk at
/// `path` first, for a text written knowingly over one that came from
/// elsewhere. That one file and no other, so it takes nothing more of a fold
/// that is being written than what is about to be overwritten.
pub fn write_at(root: &Path, id: &str, path: &str, content: &str, base: &Base, keep_history: bool) -> Result<Saved, String> {
    wsfolder::refuse_real(root)?;
    rel_ok(path)?;
    if content.len() as u64 > MAX_TEXT_BYTES {
        return Err(format!("{path} is too large to save here"));
    }
    if kind_of(path) != Kind::Text {
        return Err(format!("{path} is not a text file"));
    }
    let folder = wsfolder::folder_at(root, id)?;
    let mem = folder.join(DIR);
    if !is_made(&mem) {
        return Err("this workspace has no memory yet".into());
    }
    let _held = hold(&folder)?;
    let dst = match base {
        Base::Rev(_) | Base::Mtime(_) => {
            let f = existing(&mem, path)?;
            let (md, bytes) = open_read(&f, path, MAX_TEXT_BYTES + 1)?;
            // Equal to what is there: nothing is written, and nothing is lost.
            if bytes == content.as_bytes() {
                return Ok(saved(path, content, &md));
            }
            let same = match base {
                Base::Rev(r) => *r == rev_of(&bytes),
                Base::Mtime(m) => *m == mtime_ms(&md),
                Base::None => false,
            };
            if !same {
                return Err(format!("{ERR_STALE}: {path}"));
            }
            f
        }
        Base::None => {
            let dir = parent_of(&mem, path)?;
            let f = dir.join(name_of(path));
            if fs::symlink_metadata(&f).is_ok() {
                return Err(format!("there is already a file at {path}"));
            }
            f
        }
    };
    if keep_history {
        let repo = Repo::open(&mem)?;
        repo.recover();
        commit_specs(&repo, &[format!(":(top,literal){path}")], Some(&format!("Before keeping your text: {path}")), &[])?;
    }
    let mode = fs::symlink_metadata(&dst).ok().map(|m| m.permissions());
    fsx::write_atomic(&dst, content.as_bytes(), mode).map_err(|e| format!("{path}: {e}"))?;
    let md = fs::symlink_metadata(&dst).map_err(|e| format!("{path}: {e}"))?;
    Ok(saved(path, content, &md))
}

fn saved(path: &str, content: &str, md: &fs::Metadata) -> Saved {
    Saved { path: path.into(), file: name_of(path), title: title_for(path, content), mtime_ms: mtime_ms(md), rev: rev_of(content.as_bytes()), size: md.len() }
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
    let folder = wsfolder::folder_at(root, id)?;
    let mem = folder.join(DIR);
    if !is_made(&mem) {
        return Err("this workspace has no memory yet".into());
    }
    let _held = hold(&folder)?;
    let dir = parent_of(&mem, &format!("{FILES}/{name}"))?;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (name, String::new()),
    };
    let free = std::iter::once(name.to_string())
        .chain((2..1000).map(|n| format!("{stem}-{n}{ext}")))
        .find(|n| fs::symlink_metadata(dir.join(n)).is_err())
        .ok_or_else(|| format!("too many files named {name}"))?;
    let dst = dir.join(&free);
    fsx::write_atomic(&dst, bytes, None).map_err(|e| format!("{free}: {e}"))?;
    let md = fs::symlink_metadata(&dst).map_err(|e| format!("{free}: {e}"))?;
    Ok(Saved { path: format!("{FILES}/{free}"), file: free.clone(), title: free, mtime_ms: mtime_ms(&md), rev: rev_of(bytes), size: md.len() })
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
    commit_specs(repo, &repo.pathspecs()?, message, &[])
}

fn commit_specs(repo: &Repo, specs: &[String], message: Option<&str>, live: &[String]) -> Result<Option<Change>, String> {
    let dirty = repo.dirty(specs)?;
    if dirty.is_empty() {
        return Ok(None);
    }
    let mut add = vec!["add", "--all", "--"];
    add.extend(specs.iter().map(String::as_str));
    repo.bytes(&add)?;
    // A file removed, or put back as it was, between the status and the add.
    if repo.has_commits() && repo.run(&["diff", "--cached", "--quiet", "--ignore-submodules=all"])?.status.success() {
        return Ok(None);
    }
    let msg = message.map(subject).filter(|m| !m.is_empty()).unwrap_or_else(|| subject(&edited(&dirty)));
    let msg = format!("--message={msg}");
    let named: Vec<String> = live.iter().filter(|c| crate::ulid::is_valid(c)).take(LIVE_MAX).map(|c| format!("{LIVE}{c}")).collect();
    let trailer = format!("--message={}", named.join("\n"));
    let mut args = vec!["commit", "--quiet", "--no-verify", "--cleanup=whitespace", &msg];
    if !named.is_empty() {
        args.push(&trailer);
    }
    repo.bytes(&args)?;
    Ok(head(repo)?)
}

fn head(repo: &Repo) -> Result<Option<Change>, String> {
    Ok(log_in(repo, None, None, 1)?.changes.pop())
}

/// Commit a dirty tree: what a fold, a chat's start and the quiet minute do.
/// `message` `None` names the files. `None` back: there was nothing to commit.
pub fn commit_if_dirty_at(root: &Path, id: &str, message: Option<&str>) -> Result<Option<Change>, String> {
    wsfolder::refuse_real(root)?;
    let folder = wsfolder::folder_at(root, id)?;
    let mem = folder.join(DIR);
    if !is_made(&mem) {
        return Ok(None);
    }
    let _held = hold(&folder)?;
    let repo = Repo::open(&mem)?;
    repo.recover();
    commit_held(&repo, message)
}

/// The quiet minute's commit: [`commit_if_dirty_at`] with no message, and the
/// chats that were live in the body. `live` is asked with the time of the
/// commit before it (seconds since the epoch, 0 for none), under the lock, so
/// it can leave out what that commit already holds.
pub fn commit_quiet_at(root: &Path, id: &str, live: impl FnOnce(i64) -> Vec<String>) -> Result<Option<Change>, String> {
    wsfolder::refuse_real(root)?;
    let folder = wsfolder::folder_at(root, id)?;
    let mem = folder.join(DIR);
    if !is_made(&mem) {
        return Ok(None);
    }
    let _held = hold(&folder)?;
    let repo = Repo::open(&mem)?;
    repo.recover();
    let last = head(&repo)?.map_or(0, |c| c.at);
    commit_specs(&repo, &repo.pathspecs()?, None, &live(last))
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
/// nothing has reverted IT.
fn undone(repo: &Repo) -> Result<HashMap<String, String>, String> {
    let out = repo.text(&["log", "-z", "--first-parent", "--fixed-strings", "--grep", REVERTS, "--format=tformat:%H%x02%b"])?;
    let mut map: HashMap<String, String> = HashMap::new();
    for rec in out.split('\0') {
        let Some((sha, body)) = rec.trim_start_matches('\n').split_once('\x02') else { continue };
        // Newest first, so whether this one stands is already known.
        if let Some(target) = reverts_in(body).filter(|_| !map.contains_key(sha)) {
            map.entry(target).or_insert_with(|| sha.to_string());
        }
    }
    Ok(map)
}

/// The commits of workspace `id`'s memory that are undone now, whatever undid
/// them: one git call, for whoever keeps a commit and shows whether it stands.
pub fn undone_at(root: &Path, id: &str) -> Result<std::collections::HashSet<String>, String> {
    let mem = dir_at(root, id)?;
    if !is_made(&mem) {
        return Ok(Default::default());
    }
    let repo = Repo::open(&mem)?;
    if !repo.has_commits() {
        return Ok(Default::default());
    }
    Ok(undone(&repo)?.into_keys().collect())
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
/// one path when given. `before` is the last commit of the page before. Two
/// git calls a page, whatever it holds.
pub fn log_at(root: &Path, id: &str, path: Option<&str>, before: Option<&str>, n: Option<usize>) -> Result<Log, String> {
    if let Some(p) = path {
        rel_ok(p)?;
    }
    let mem = dir_at(root, id)?;
    if !is_made(&mem) {
        return Ok(Log::default());
    }
    log_in(&Repo::open(&mem)?, path, before, n.unwrap_or(LOG_PAGE).clamp(1, LOG_MAX))
}

/// One commit, as [`log_at`] lists it.
fn change_of(repo: &Repo, commit: &str) -> Result<Change, String> {
    let commit = known(repo, commit)?;
    let out = repo.text(&["log", "-z", "--first-parent", "-m", "--no-renames", "--no-color", "--no-ext-diff", "--no-textconv", "--raw", "--abbrev=40", "--numstat", FORMAT, "--max-count=1", &commit, "--"])?;
    let mut c = parse_log(&out).pop().ok_or("no such change in this memory")?;
    c.reverted_by = undone(repo)?.remove(&c.commit);
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
    let repo = Repo::open(&dir_at(root, id)?)?;
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
    let folder = wsfolder::folder_at(root, id)?;
    let mem = folder.join(DIR);
    let repo = Repo::open(&mem)?;
    let _held = hold(&folder)?;
    repo.recover();
    let change = change_of(&repo, commit)?;
    if change.kind == ChangeKind::Made || !repo.run(&["rev-parse", "--verify", "--quiet", &format!("{commit}^")])?.status.success() {
        return Err("the change that made this memory cannot be undone".into());
    }
    if change.reverted_by.is_some() {
        return Err("that change is already undone".into());
    }
    commit_held(&repo, None)?;
    let o = repo.run(&["revert", "--no-edit", commit])?;
    if !o.status.success() {
        let conflicts: Vec<String> =
            repo.text(&["diff", "--name-only", "-z", "--diff-filter=U"])?.split('\0').filter(|p| !p.is_empty()).map(String::from).collect();
        let began = fs::symlink_metadata(mem.join(".git/REVERT_HEAD")).is_ok();
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
        /// The test's own git, for planting: never the module's.
        fn git(&self, args: &[&str]) {
            assert!(!self.mem().as_os_str().is_empty() && self.mem().starts_with(&self.base));
            let o = Repo { mem: self.mem(), off: Vec::new() }.run(args).unwrap();
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
        let raw = Repo { mem: s.mem(), off: Vec::new() }.text(&["log", "-1", "--format=%B"]).unwrap();
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
        let repo = Repo::open(&s.mem()).unwrap();
        assert_eq!(repo.text(&["log", "--format=%an <%ae> %cn <%ce>"]).unwrap().trim(), "worktrees <memory@localhost> worktrees <memory@localhost>");
        assert_eq!(repo.text(&["remote"]).unwrap(), "", "no remote");
        assert!(!s.mem().join(".git/hooks").exists(), "no template");
        assert!(!fs::read_to_string(s.mem().join(".git/config")).unwrap().contains("worktree ="), "it can be moved");
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
        let above = Repo { mem: s.base.clone(), off: Vec::new() };
        above.bytes(&["init", "--quiet", "--template="]).unwrap();
        fs::remove_dir_all(s.mem().join(".git")).unwrap();
        s.put("topics/a.md", "a\n");
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, Some("x")).unwrap(), None);
        assert!(log_at(&s.root, &s.id, None, None, None).unwrap().changes.is_empty());
        assert!(!above.has_commits());
        // A `.git` FILE naming that repository is refused, not followed.
        fs::write(s.mem().join(".git"), format!("gitdir: {}\n", s.base.join(".git").display())).unwrap();
        assert!(init_at(&s.root, &s.id, "x").unwrap_err().contains("not a folder"));
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, Some("x")).unwrap(), None);
        fs::remove_file(s.mem().join(".git")).unwrap();
        symlink(s.base.join(".git"), s.mem().join(".git")).unwrap();
        assert_eq!(commit_if_dirty_at(&s.root, &s.id, Some("x")).unwrap(), None);
        assert!(!above.has_commits());
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
        let git = s.mem().join(".git");
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
        let bare = Repo { mem: s.mem(), off: Vec::new() };
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
        let c = Repo::open(&s.mem()).unwrap().command();
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
        assert_eq!(stamp_at(&s.root, &s.id).unwrap().keys().collect::<Vec<_>>(), vec![INDEX, "topics/t.md"]);
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
        let c4 = s.commit("link");
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
        assert!(!s.mem().join(".git/REVERT_HEAD").exists(), "aborted");
        // Undone by hand: there is nothing left to undo, and nothing is left behind.
        s.put("topics/u.md", "u\n");
        let u = s.commit("u");
        fs::remove_file(s.mem().join("topics/u.md")).unwrap();
        s.commit("by hand");
        assert!(s.revert(&u).is_err_and(|e| e.contains("nothing left")));
        assert!(!s.mem().join(".git/REVERT_HEAD").exists());
        // A revert the engine was killed in is put back before the next act.
        s.put(INDEX, "three\n");
        s.commit("three");
        assert!(!Repo::open(&s.mem()).unwrap().run(&["revert", "--no-edit", &c.commit]).unwrap().status.success());
        assert!(s.mem().join(".git/REVERT_HEAD").exists(), "the fixture is mid-revert");
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
        assert_eq!(changed_since(&a, &b).0.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec![INDEX], "same mtime, new size");
        s.put("topics/t.md", "t\n");
        fs::remove_file(&p).unwrap();
        let c = at();
        let (ch, rm) = changed_since(&b, &c);
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
            assert_eq!(changed_since(&before, &now).0.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec![INDEX], "{other:?}");
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
            fs::create_dir_all(mem.join(".git")).unwrap();
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
