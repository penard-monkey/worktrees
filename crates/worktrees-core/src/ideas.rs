//! Ideas: one markdown document per idea, in a folder of plain files
//! (`docs/proposals/idea-board-phases.md` §2.1, §5, phase 1).
//!
//! The folder is `$XDG_DATA_HOME/worktrees/ideas/` (default
//! `~/.local/share/worktrees/ideas/`), or whatever `WORKTREES_IDEAS_DIR`
//! names ([`OVERRIDE_ENV`]). That variable moves THIS folder only: a test or a
//! sandbox must never move `XDG_DATA_HOME` to isolate it, because a scratch
//! Claude given its own `XDG_DATA_HOME` reinstalls itself there and relinks
//! the real `~/.local/bin/claude` (AGENTS.md). The folder is created on the
//! first write and never on a read.
//!
//! **The file is canonical and is never re-printed.** A write stores exactly
//! the bytes it was given; the app's own edits (adopt, rename) change the
//! lines they are about and copy every other byte through. So frontmatter keys
//! this module does not know (phase 3's `related`, `absorbed`, `from`; phase
//! 4's `plan_of`), HTML comments (later phases' anchors), tables and fences all
//! survive. Nothing here parses a document into a model and prints it back.
//!
//! A document:
//!
//! ```markdown
//! ---
//! id: 01JC8Z4K3M9QX2T7VBN5R6WYHD
//! created: 2026-11-09
//! ---
//!
//! # Jellyfin on the home server
//! ```
//!
//! - `id` is a ULID (`crate::ulid`) written once and never changed. Every
//!   lookup is by id, through ONE [`Index`] rebuilt from the folder.
//! - The title is the first `# heading` outside the frontmatter and outside a
//!   fence; it is not stored in the frontmatter, so the two cannot disagree.
//! - The file name is [`slug`] of the title. [`rename_at`] changes the heading
//!   and the name together; typing a new heading in an editor changes neither
//!   the name nor anything else (typing is not an operation).
//!
//! Every change the app makes is ONE named operation with plain parameters —
//! [`create_at`], [`write_at`], [`rename_at`], [`remove_at`], [`adopt_at`] —
//! because those are what an agent will later propose (§7).
//!
//! **History:** before an operation replaces or deletes a document's bytes
//! (rename, remove, adopt), the current file is copied to
//! `.history/<id>/<UTC time>.md`. A write (typing) is not copied. Nothing here
//! prunes history.
//!
//! **Hand-made files.** A `.md` someone put in the folder with no `id` is
//! LISTED (`id: null`, `problem: "no-id"`, its title from its heading or its
//! file name) and readable by nothing else, since every other row takes an id.
//! It gets an id only through [`adopt_at`], an explicit act that names the
//! file by its bare name — a read or a listing never writes. A file whose `id`
//! is not a ULID (`problem: "bad-id"`), or two files carrying one id
//! (`problem: "duplicate-id"`, both listed), are listed and refused by every
//! lookup, with the reason: the fix is a person's (the app cannot know which
//! copy is the idea).
//!
//! **Symlinks.** The folder itself may be a link (a person pointing it into a
//! synced folder is a legitimate choice, and is theirs). INSIDE it, a link is
//! not a document: the scan `lstat`s each entry and lists only regular files,
//! so a link is never read, never written through, never removed, and is
//! reported in [`Index::skipped`]. `.history` and `.history/<id>` must be real
//! directories (`fsx::make_dir`). Only top-level entries are read, so no path
//! is ever built through an intermediate component the scan did not lstat.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The variable that names the ideas folder, and nothing else.
pub const OVERRIDE_ENV: &str = "WORKTREES_IDEAS_DIR";

/// Where a document's copies go before an operation changes it.
pub const HISTORY_DIR: &str = ".history";

/// A document larger than this is listed with `problem: "too-large"` and is
/// not read. An idea is prose; this bounds a pathological file, nothing else.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// A file name's stem is cut to this many bytes, at a character boundary.
pub const SLUG_MAX: usize = 64;

/// The ideas folder: [`OVERRIDE_ENV`] when it is set and not empty, else
/// [`default_dir`].
pub fn dir() -> PathBuf {
    match std::env::var_os(OVERRIDE_ENV).filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => default_dir(),
    }
}

/// `$XDG_DATA_HOME/worktrees/ideas`, or `~/.local/share/worktrees/ideas`:
/// the user's real folder, which the test guard protects whatever the
/// override says.
pub fn default_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.local/share", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(base).join("worktrees").join("ideas")
}

/// `Err` for a write under `root` when it is the user's real folder and this
/// process runs an engine for a test (`registry::forbid_real_writes`, the
/// same one-way switch: one guard for every user-owned store).
pub(crate) fn refuse_real(root: &Path) -> Result<(), String> {
    refuse_real_in(root, &default_dir(), crate::registry::real_writes_forbidden())
}

fn refuse_real_in(root: &Path, real: &Path, forbidden: bool) -> Result<(), String> {
    if !forbidden {
        return Ok(());
    }
    let same = root == real || matches!((fs::canonicalize(root), fs::canonicalize(real)), (Ok(a), Ok(b)) if a == b);
    if same {
        return Err(format!(
            "refused: a test reached the user's real ideas folder ({}). Give the engine under test its own \
             folder ({OVERRIDE_ENV} in a child's environment, never set_var, or `Ctx::with_ideas_dir`), or use \
             the `*_at` forms with a temp dir.",
            real.display()
        ));
    }
    Ok(())
}

// ── one document's text ─────────────────────────────────────────────────────

/// The frontmatter's byte span: `(inner_start, inner_end, body_start)`. It
/// opens with `---` as the file's very first line and closes at the next line
/// that is `---` or `...`. `None` without one.
fn frontmatter(text: &str) -> Option<(usize, usize, usize)> {
    let first_end = text.find('\n')?;
    if text[..first_end].trim_end_matches('\r') != "---" {
        return None;
    }
    let inner_start = first_end + 1;
    let mut at = inner_start;
    while at < text.len() {
        let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        let line = text[at..end].trim_end_matches('\r');
        if line == "---" || line == "..." {
            return Some((inner_start, at, (end + 1).min(text.len())));
        }
        at = end + 1;
    }
    None
}

/// The value of a top-level `key:` in the frontmatter, unquoted.
fn front_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let (s, e, _) = frontmatter(text)?;
    text[s..e].lines().find_map(|l| {
        let v = l.strip_prefix(key)?.strip_prefix(':')?;
        Some(v.trim().trim_matches(|c| c == '"' || c == '\''))
    })
}

/// The `id` a document's text carries, as written (it may not be a ULID).
pub fn id_of(text: &str) -> Option<&str> {
    front_value(text, "id").filter(|v| !v.is_empty())
}

/// The first `# heading` outside the frontmatter and any fence: its line's
/// byte span (without the line ending) and its text.
fn heading(text: &str) -> Option<(usize, usize, String)> {
    let mut at = frontmatter(text).map_or(0, |(_, _, b)| b);
    let mut fence: Option<(u8, usize)> = None;
    while at < text.len() {
        let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        let raw = &text[at..end];
        let line = raw.trim_end_matches('\r');
        let indent = line.len() - line.trim_start_matches(' ').len();
        let t = &line[indent..];
        if indent <= 3 {
            let run = |c: u8| t.bytes().take_while(|b| *b == c).count();
            match fence {
                Some((c, n)) => {
                    if run(c) >= n && t[run(c)..].trim().is_empty() {
                        fence = None;
                    }
                }
                None => {
                    for c in [b'`', b'~'] {
                        if run(c) >= 3 {
                            fence = Some((c, run(c)));
                        }
                    }
                    if fence.is_none() && (t == "#" || t.starts_with("# ") || t.starts_with("#\t")) {
                        let title = t[1..].trim().trim_end_matches('#').trim_end().to_string();
                        return Some((at, at + line.len(), title));
                    }
                }
            }
        }
        at = end + 1;
    }
    None
}

/// The title a document's text gives it: its first `# heading`, or `None`.
pub fn title_of(text: &str) -> Option<String> {
    heading(text).map(|(_, _, t)| t).filter(|t| !t.is_empty())
}

/// A title as the app writes it into a heading: one line, trimmed.
fn clean_title(title: &str) -> String {
    let t: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = t.trim();
    if t.is_empty() { "Untitled".to_string() } else { t.to_string() }
}

/// `text` with its title set to `title`: the first heading's LINE replaced,
/// or, with none, a heading inserted where the body starts. Every other byte
/// is copied through.
fn with_title(text: &str, title: &str) -> String {
    let h = format!("# {}", clean_title(title));
    match heading(text) {
        Some((s, e, _)) => format!("{}{h}{}", &text[..s], &text[e..]),
        None => {
            let mut b = frontmatter(text).map_or(0, |(_, _, b)| b);
            // Keep the conventional blank line after the frontmatter above it.
            if b > 0 && text[b..].starts_with('\n') {
                b += 1;
            }
            format!("{}{h}\n\n{}", &text[..b], &text[b..])
        }
    }
}

/// `text` given an `id` (and a `created`, when it has none): two lines added
/// to its frontmatter right after the opening `---`, or a new frontmatter
/// block in front of it. Every other byte is copied through.
fn with_id(text: &str, id: &str, created: &str) -> String {
    match frontmatter(text) {
        Some((s, _, _)) => {
            let mut add = format!("id: {id}\n");
            if front_value(text, "created").is_none() {
                add.push_str(&format!("created: {created}\n"));
            }
            format!("{}{add}{}", &text[..s], &text[s..])
        }
        None => format!("---\nid: {id}\ncreated: {created}\n---\n\n{text}"),
    }
}

/// The file-name stem for a title: lower case, letters and digits kept (any
/// script), every run of anything else one `-`, no `-` at either end, cut to
/// [`SLUG_MAX`] bytes. So no `/`, no leading dot, no empty name: `untitled`.
/// The ONLY way a title becomes a file name.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if out.len() + c.len_utf8() > SLUG_MAX {
                break;
            }
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            if out.len() + 1 > SLUG_MAX {
                break;
            }
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() { "untitled".to_string() } else { out.to_string() }
}

/// A free `<stem>.md` in `root`, else `<stem>-2.md`, `-3`, … `own` is the
/// document's current name, which is never a collision with itself. "Taken"
/// is ANY entry at the name, a link or a hand-made file included.
///
/// A name that reaches `own`'s own file (`Groceries.md` asked for as
/// `groceries.md` on a case-insensitive volume) is `own`'s too.
fn free_name(root: &Path, stem: &str, own: Option<&str>) -> String {
    let mine = own.and_then(|o| fs::symlink_metadata(root.join(o)).ok());
    (1..)
        .map(|n| if n == 1 { format!("{stem}.md") } else { format!("{stem}-{n}.md") })
        .find(|name| {
            Some(name.as_str()) == own
                || match fs::symlink_metadata(root.join(name)) {
                    Err(_) => true,
                    Ok(md) => mine.as_ref().is_some_and(|m| same_file(m, &md)),
                }
        })
        .expect("an unbounded range finds a free name")
}

fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

/// A bare document name a client may hand back (`adopt_at`): one component,
/// not hidden, `.md`.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\0') && !name.starts_with('.') && name.ends_with(".md")
}

// ── the index ───────────────────────────────────────────────────────────────

/// Why a listed document cannot be addressed by id.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Problem {
    /// No `id` in its frontmatter (hand-made): `adopt_at` gives it one.
    NoId,
    /// An `id` that is not a ULID: a person's fix.
    BadId,
    /// Another file carries the same id (a copy made in Finder): a person's fix.
    DuplicateId,
    /// Over [`MAX_BYTES`]; not read.
    TooLarge,
    /// Not UTF-8; listed by its name, not read.
    NotText,
    /// The engine could not stat or read it (a permission, an I/O error):
    /// listed by its name so the folder still lists, and addressed by nothing.
    Unreadable,
}

/// One document as the list shows it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Idea {
    /// Its ULID, or `None` when it has none that can be used (`problem`).
    pub id: Option<String>,
    /// Its first heading, else its file name without `.md`.
    pub title: String,
    /// Its bare file name in the folder (no path).
    pub file: String,
    /// The `created:` value, as written.
    pub created: Option<String>,
    /// Its modification time in ms: `write_at`'s `expected_mtime`, and what
    /// `ideas:changed` carries.
    pub mtime_ms: u64,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<Problem>,
}

/// Every document in the folder, and the one lookup by id.
#[derive(Serialize, Default, Debug)]
pub struct Index {
    /// Newest first.
    pub ideas: Vec<Idea>,
    /// Entries that are not documents and were not followed: a symlink, a
    /// directory, anything that is not a regular file. Bare names.
    pub skipped: Vec<String>,
    /// id → position in `ideas`, for the ids exactly one file carries.
    #[serde(skip)]
    by_id: HashMap<String, usize>,
    // Phase 3 adds `absorbed: HashMap<String, String>` (a merged-away id →
    // the id that absorbed it), filled from each document's `absorbed:` line
    // in `scan_at`; `resolve` follows it after `by_id`. Every lookup already
    // goes through `resolve`, so nothing else moves.
}

impl Index {
    /// The document `id` names, or why none can be addressed by it.
    pub fn resolve(&self, id: &str) -> Result<&Idea, String> {
        if !crate::ulid::is_valid(id) {
            return Err(format!("not an idea id: {id:?}"));
        }
        if let Some(i) = self.by_id.get(id) {
            return Ok(&self.ideas[*i]);
        }
        let dups: Vec<&str> = self
            .ideas
            .iter()
            .filter(|d| d.problem == Some(Problem::DuplicateId) && d.id.as_deref() == Some(id))
            .map(|d| d.file.as_str())
            .collect();
        if !dups.is_empty() {
            return Err(format!("two files carry the id {id} ({}): remove or re-id one of them by hand", dups.join(", ")));
        }
        Err(format!("no idea has the id {id}"))
    }
}

/// A file's mtime in ms (0 when unreadable), from `lstat`.
pub fn mtime_ms(md: &fs::Metadata) -> u64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// Read the folder at `root` into an [`Index`]. A missing folder is an empty
/// index; nothing is created. Only top-level entries, only regular `.md` files
/// not starting with `.` (which leaves out `.history`, a write's temp and
/// Finder's `.DS_Store`), each `lstat`ed before it is read.
///
/// Only a failure to read the FOLDER fails the scan. An error on one entry
/// is about that entry: an editor that saves by rename (vim, most sync
/// clients) removes and recreates its file, so a name `readdir` gave can be
/// gone by the `lstat` or the read — that entry is skipped — and any other
/// error lists the file as [`Problem::Unreadable`]. Every row starts with a
/// scan, so one bad file must never fail the rows for every other idea.
pub fn scan_at(root: &Path) -> io::Result<Index> {
    let rd = match fs::read_dir(root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Index::default()),
        Err(e) => return Err(e),
    };
    // An entry the iterator itself fails on has no name to list or skip by.
    let names = rd.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok());
    Ok(scan_with(root, names, &|p| fs::read(p)))
}

/// [`scan_at`] past the `readdir`: the entry names, and how a file's bytes
/// are read. The seam a test scripts a vanished name or an unreadable file
/// through, rather than racing a real one (or relying on `chmod 000`, which
/// a root test process reads through).
fn scan_with(root: &Path, names: impl Iterator<Item = String>, read: &dyn Fn(&Path) -> io::Result<Vec<u8>>) -> Index {
    let mut idx = Index::default();
    for name in names {
        if name.starts_with('.') || !name.ends_with(".md") {
            continue;
        }
        let p = root.join(&name);
        let stem = name.trim_end_matches(".md").to_string();
        let unreadable = |name: String| Idea { id: None, title: stem.clone(), file: name, created: None, mtime_ms: 0, size: 0, problem: Some(Problem::Unreadable) };
        let md = match fs::symlink_metadata(&p) {
            Ok(md) => md,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                idx.ideas.push(unreadable(name));
                continue;
            }
        };
        if !md.file_type().is_file() {
            idx.skipped.push(name);
            continue;
        }
        let mut doc = Idea { id: None, title: stem.clone(), file: name, created: None, mtime_ms: mtime_ms(&md), size: md.len(), problem: None };
        if md.len() > MAX_BYTES {
            doc.problem = Some(Problem::TooLarge);
        } else {
            let bytes = match read(&p) {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => {
                    doc.problem = Some(Problem::Unreadable);
                    idx.ideas.push(doc);
                    continue;
                }
            };
            match String::from_utf8(bytes) {
                Err(_) => doc.problem = Some(Problem::NotText),
                Ok(text) => {
                    if let Some(t) = title_of(&text) {
                        doc.title = t;
                    }
                    doc.created = front_value(&text, "created").map(str::to_string);
                    match id_of(&text) {
                        None => doc.problem = Some(Problem::NoId),
                        Some(id) if !crate::ulid::is_valid(id) => doc.problem = Some(Problem::BadId),
                        Some(id) => doc.id = Some(id.to_string()),
                    }
                }
            }
        }
        idx.ideas.push(doc);
    }
    // Newest first; the name breaks a tie so the order is stable.
    idx.ideas.sort_by(|x, y| y.mtime_ms.cmp(&x.mtime_ms).then_with(|| x.file.cmp(&y.file)));
    let mut ids: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, d) in idx.ideas.iter().enumerate() {
        if let (Some(id), None) = (&d.id, d.problem) {
            ids.entry(id.clone()).or_default().push(i);
        }
    }
    for (id, at) in ids {
        if let [one] = at[..] {
            idx.by_id.insert(id, one);
        } else {
            for i in at {
                idx.ideas[i].problem = Some(Problem::DuplicateId);
            }
        }
    }
    idx.skipped.sort();
    idx
}

/// What the poll compares between ticks: every top-level entry's name, kind,
/// size and mtime, sorted. One `readdir` and one `lstat` per entry, and no
/// file is opened; a missing folder is an empty stamp.
pub fn stamp_at(root: &Path) -> Vec<(String, bool, u64, u64)> {
    let Ok(rd) = fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<_> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if name.starts_with('.') || !name.ends_with(".md") {
                return None;
            }
            let md = fs::symlink_metadata(e.path()).ok()?;
            Some((name, md.file_type().is_file(), md.len(), mtime_ms(&md)))
        })
        .collect();
    out.sort();
    out
}

// ── reads ───────────────────────────────────────────────────────────────────

/// One document and its exact text.
#[derive(Serialize, Debug)]
pub struct Doc {
    #[serde(flatten)]
    pub idea: Idea,
    pub content: String,
}

pub fn list_at(root: &Path) -> Result<Index, String> {
    scan_at(root).map_err(|e| format!("{}: {e}", root.display()))
}

pub fn read_at(root: &Path, id: &str) -> Result<Doc, String> {
    let idx = list_at(root)?;
    let idea = idx.resolve(id)?.clone();
    let (content, md) = load(root, &idea)?;
    Ok(Doc { idea: Idea { mtime_ms: mtime_ms(&md), size: md.len(), ..idea }, content })
}

/// The current text of a listed document, from a path re-checked to be a
/// regular file (the scan's verdict could be a moment old).
fn load(root: &Path, idea: &Idea) -> Result<(String, fs::Metadata), String> {
    let p = root.join(&idea.file);
    let md = fs::symlink_metadata(&p).map_err(|e| format!("{}: {e}", idea.file))?;
    if !md.file_type().is_file() {
        return Err(format!("{} is no longer a regular file", idea.file));
    }
    let text = String::from_utf8(fs::read(&p).map_err(|e| format!("{}: {e}", idea.file))?)
        .map_err(|_| format!("{} is not UTF-8", idea.file))?;
    Ok((text, md))
}

fn check_mtime(md: &fs::Metadata, expected: Option<u64>) -> Result<(), String> {
    match expected {
        Some(exp) if mtime_ms(md) != exp => {
            Err("the idea changed on disk since you opened it — reload to see the latest".into())
        }
        _ => Ok(()),
    }
}

// ── the operations ──────────────────────────────────────────────────────────

/// Today, UTC, as `YYYY-MM-DD`.
fn today() -> String {
    date_of(crate::sysclock::now_epoch())
}

pub(crate) fn date_of(epoch: i64) -> String {
    let (y, m, d) = crate::attach::civil_from_days(epoch.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `20261108T153012.345Z`: when a history copy was taken, sortable.
pub(crate) fn history_stamp() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let secs = ms.div_euclid(1000);
    let (y, m, d) = crate::attach::civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}.{:03}Z", s / 3600, s / 60 % 60, s % 60, ms.rem_euclid(1000))
}

/// Make the folder for a write: the guard, then `create_dir_all` (the
/// folder is under the user's home, not a checkout, and may itself be a link).
fn ensure_root(root: &Path) -> Result<(), String> {
    refuse_real(root)?;
    fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))
}

/// Create `path`, which must not exist (`O_EXCL`, `O_NOFOLLOW`), with `bytes`.
/// For a NEW file only — a replace goes through `fsx::write_atomic`.
pub(crate) fn create_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(0o644)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Copy `bytes` (a document's current text) to `.history/<id>/<time>.md`.
pub fn history_copy_at(root: &Path, id: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if !crate::ulid::is_valid(id) {
        return Err(format!("not an idea id: {id:?}"));
    }
    refuse_real(root)?;
    let h = root.join(HISTORY_DIR);
    let d = h.join(id);
    for dir in [&h, &d] {
        crate::fsx::make_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let stamp = history_stamp();
    for n in 1..100 {
        let name = if n == 1 { format!("{stamp}.md") } else { format!("{stamp}-{n}.md") };
        let p = d.join(name);
        match create_new(&p, bytes) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", p.display())),
        }
    }
    Err(format!("{}: no free history name", d.display()))
}

/// A new, blank idea titled `title`.
pub fn create_at(root: &Path, title: &str) -> Result<Doc, String> {
    ensure_root(root)?;
    let title = clean_title(title);
    let id = crate::ulid::new();
    let content = format!("---\nid: {id}\ncreated: {}\n---\n\n# {title}\n", today());
    let stem = slug(&title);
    for _ in 0..16 {
        let file = free_name(root, &stem, None);
        match create_new(&root.join(&file), content.as_bytes()) {
            Ok(()) => return read_at(root, &id),
            // Someone took the name between the look and the create: look again.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{file}: {e}")),
        }
    }
    Err(format!("no free file name for {stem:?}"))
}

/// Replace the text of the idea `id` with exactly `content`, unless the file
/// changed since `expected_mtime`. `content` must carry the same `id`, so a
/// client cannot save one document over another. Bytes equal to what is on
/// disk write nothing, and the mtime does not move. Returns the idea as it
/// is after the save: its `mtime_ms` is the next save's `expected_mtime`,
/// and its `file` the name it now has.
///
/// **The first heading names the file** (as [`rename_at`] would): after the
/// bytes are written, a file whose name is not [`slug`] of the title (no
/// heading: `untitled`) is moved to it, suffixed when taken ([`name_for`]).
/// The move keeps the inode, so the mtime returned is the write's.
///
/// A save (typing) copies nothing to history; [`write_kept_at`] does.
pub fn write_at(root: &Path, id: &str, content: &str, expected_mtime: Option<u64>) -> Result<Idea, String> {
    write_with_at(root, id, content, expected_mtime, false)
}

/// [`write_at`] that first copies the file's CURRENT bytes to history when it
/// would change them: the overwrite a person chooses over text that came from
/// elsewhere ("Keep mine" in a conflict), and what an agent's proposal that
/// rewrites a document will use. That text exists nowhere else; an ordinary
/// save never takes this path.
pub fn write_kept_at(root: &Path, id: &str, content: &str, expected_mtime: Option<u64>) -> Result<Idea, String> {
    write_with_at(root, id, content, expected_mtime, true)
}

fn write_with_at(root: &Path, id: &str, content: &str, expected_mtime: Option<u64>, keep_history: bool) -> Result<Idea, String> {
    refuse_real(root)?;
    let idx = list_at(root)?;
    let idea = idx.resolve(id)?.clone();
    match id_of(content) {
        Some(c) if c == id => {}
        Some(c) => return Err(format!("the text carries the id {c}, not {id}: refusing to save one idea over another")),
        None => return Err(format!("the text has no id: keep the frontmatter's `id: {id}` line")),
    }
    let (now, md) = load(root, &idea)?;
    check_mtime(&md, expected_mtime)?;
    if now != content {
        if keep_history {
            history_copy_at(root, id, now.as_bytes())?;
        }
        let p = root.join(&idea.file);
        crate::fsx::write_atomic(&p, content.as_bytes(), Some(md.permissions())).map_err(|e| format!("{}: {e}", idea.file))?;
    }
    // Checked on an unchanged save too: a rename that failed after its write
    // is tried again by the next save, whatever it carries.
    let file = rename_to_title(root, &idea.file, content)?;
    let md = fs::symlink_metadata(root.join(&file)).map_err(|e| format!("{file}: {e}"))?;
    Ok(Idea {
        title: title_of(content).unwrap_or_else(|| file.trim_end_matches(".md").to_string()),
        created: front_value(content, "created").map(str::to_string),
        mtime_ms: mtime_ms(&md),
        size: md.len(),
        file,
        ..idea
    })
}

/// The name a document whose text is `text`, now at `own`, should have: `own`
/// itself when its stem is already [`slug`] of the title (no heading:
/// `untitled`), or that slug with the `-<n>` [`free_name`] gives while the
/// plain name is taken by another entry — so a second "Jellyfin Sharing"
/// stays `jellyfin-sharing-2.md` and does not bounce. Otherwise the first
/// free name for the slug.
fn name_for(root: &Path, own: &str, text: &str) -> String {
    let want = slug(&clean_title(&title_of(text).unwrap_or_default()));
    let stem = own.strip_suffix(".md").unwrap_or(own);
    if stem == want {
        return own.to_string();
    }
    let suffixed = stem
        .strip_prefix(want.as_str())
        .and_then(|r| r.strip_prefix('-'))
        .is_some_and(|n| !n.starts_with('0') && n.parse::<u32>().is_ok_and(|n| n >= 2));
    if suffixed && fs::symlink_metadata(root.join(format!("{want}.md"))).is_ok() {
        return own.to_string();
    }
    free_name(root, &want, Some(own))
}

/// Move the document at `own` to [`name_for`] its text, and say where it is.
/// A name taken between the look and the move is looked for again.
fn rename_to_title(root: &Path, own: &str, text: &str) -> Result<String, String> {
    for _ in 0..16 {
        let to = name_for(root, own, text);
        if to == own {
            return Ok(to);
        }
        match move_excl(&root.join(own), &root.join(&to)) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("saved, but could not rename {own} → {to}: {e}")),
        }
    }
    Err(format!("saved, but found no free name to rename {own} to"))
}

/// Retitle the idea `id`: its first heading's line, and its file name, which
/// becomes [`slug`] of `title` (suffixed when taken). The `id` does not move,
/// and every other byte is copied through. The current file goes to history
/// first.
pub fn rename_at(root: &Path, id: &str, title: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    refuse_real(root)?;
    let idx = list_at(root)?;
    let idea = idx.resolve(id)?.clone();
    let (text, md) = load(root, &idea)?;
    check_mtime(&md, expected_mtime)?;
    history_copy_at(root, id, text.as_bytes())?;
    let new_text = with_title(&text, title);
    let new_file = free_name(root, &slug(&clean_title(title)), Some(&idea.file));
    let mut at = root.join(&idea.file);
    if new_file != idea.file {
        let to = root.join(&new_file);
        move_excl(&at, &to).map_err(|e| format!("{} → {new_file}: {e}", idea.file))?;
        at = to;
    }
    if new_text != text {
        crate::fsx::write_atomic(&at, new_text.as_bytes(), Some(md.permissions())).map_err(|e| format!("{new_file}: {e}"))?;
    }
    read_at(root, id)
}

/// Move `from` to `to`, which must not exist: a hard link (which refuses an
/// existing name) and an unlink, so a name taken in the meantime is never
/// clobbered. A filesystem with no hard links falls back to a checked rename.
///
/// A `to` that is `from`'s own file (a change of case on a case-insensitive
/// volume) is a plain rename: nothing else can be clobbered.
fn move_excl(from: &Path, to: &Path) -> io::Result<()> {
    if let (Ok(a), Ok(b)) = (fs::symlink_metadata(from), fs::symlink_metadata(to)) {
        if same_file(&a, &b) {
            return fs::rename(from, to);
        }
    }
    match fs::hard_link(from, to) {
        Ok(()) => fs::remove_file(from),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => {
            if fs::symlink_metadata(to).is_ok() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "the new name is taken"));
            }
            fs::rename(from, to)
        }
    }
}

/// Delete the idea `id`, after copying it to history.
pub fn remove_at(root: &Path, id: &str, expected_mtime: Option<u64>) -> Result<(), String> {
    refuse_real(root)?;
    let idx = list_at(root)?;
    let idea = idx.resolve(id)?.clone();
    let (text, md) = load(root, &idea)?;
    check_mtime(&md, expected_mtime)?;
    history_copy_at(root, id, text.as_bytes())?;
    fs::remove_file(root.join(&idea.file)).map_err(|e| format!("{}: {e}", idea.file))
}

/// Give the hand-made document `file` (a bare name the list showed with
/// `problem: "no-id"`) an id: an `id:` line, and a `created:` from the file's
/// own birth time when it has none, added to its frontmatter, or a
/// frontmatter in front of it. Every other byte is copied through. Its file
/// name does not change.
pub fn adopt_at(root: &Path, file: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    refuse_real(root)?;
    if !plain_name(file) {
        return Err(format!("not a document name: {file:?}"));
    }
    let idx = list_at(root)?;
    let Some(idea) = idx.ideas.iter().find(|d| d.file == file).cloned() else {
        return Err(format!("no document named {file:?}"));
    };
    match idea.problem {
        Some(Problem::NoId) => {}
        None => return Err(format!("{file} already has the id {}", idea.id.unwrap_or_default())),
        Some(p) => return Err(format!("{file} cannot be adopted ({p:?}): fix it by hand")),
    }
    let (text, md) = load(root, &idea)?;
    check_mtime(&md, expected_mtime)?;
    let born = md.created().or_else(|_| md.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(today, |d| date_of(d.as_secs() as i64));
    let id = crate::ulid::new();
    history_copy_at(root, &id, text.as_bytes())?;
    let new_text = with_id(&text, &id, &born);
    crate::fsx::write_atomic(&root.join(file), new_text.as_bytes(), Some(md.permissions())).map_err(|e| format!("{file}: {e}"))?;
    read_at(root, &id)
}

// ── the same, in the folder `dir()` names ───────────────────────────────────

pub fn list() -> Result<Index, String> {
    list_at(&dir())
}
pub fn read(id: &str) -> Result<Doc, String> {
    read_at(&dir(), id)
}
pub fn create(title: &str) -> Result<Doc, String> {
    create_at(&dir(), title)
}
pub fn write(id: &str, content: &str, expected_mtime: Option<u64>) -> Result<Idea, String> {
    write_at(&dir(), id, content, expected_mtime)
}
pub fn rename(id: &str, title: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    rename_at(&dir(), id, title, expected_mtime)
}
pub fn remove(id: &str, expected_mtime: Option<u64>) -> Result<(), String> {
    remove_at(&dir(), id, expected_mtime)
}
pub fn adopt(file: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    adopt_at(&dir(), file, expected_mtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!("wt-ideas-{tag}-{}-{:x}", std::process::id(), crate::fsx::random_u64()));
            fs::create_dir_all(&d).unwrap();
            Tmp(fs::canonicalize(&d).unwrap())
        }
        /// The ideas folder, NOT yet created.
        fn root(&self) -> PathBuf {
            self.0.join("ideas")
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Frontmatter keys this module does not know, an HTML comment (a later
    /// phase's anchor), a table and a mermaid fence — whose `# ` line is not
    /// the title.
    fn rich(id: &str) -> String {
        format!(
            "---\nid: {id}\ncreated: 2026-11-09\nrelated: [01JC8Z9P0000000000000000AA]\nplan_of: []\nx-mine: \"keep  me\"\n---\n\n\
             # Jellyfin on the home server\n\nFree writing. <!-- ¶ 7k2q -->\n\n| a | b |\n|---|:-:|\n| 1 | 2 |\n\n\
             ```mermaid\n# not a title\ngraph TD; A-->B\n```\n\nTrailing   spaces   \r\nand a CRLF line.\r\n"
        )
    }

    fn raw_doc(root: &Path, name: &str, text: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join(name), text).unwrap();
    }

    #[test]
    fn a_read_or_a_listing_never_creates_the_folder() {
        let t = Tmp::new("noread");
        let root = t.root();
        assert!(list_at(&root).unwrap().ideas.is_empty());
        assert!(read_at(&root, &crate::ulid::new()).is_err());
        assert!(stamp_at(&root).is_empty());
        assert!(!root.exists(), "nothing created by a read");
        create_at(&root, "First").unwrap();
        assert!(root.is_dir(), "the first write creates it");
    }

    #[test]
    fn create_writes_a_blank_document_with_an_id_and_its_title_named_by_slug() {
        let t = Tmp::new("create");
        let root = t.root();
        let d = create_at(&root, "Jellyfin on the home server").unwrap();
        let id = d.idea.id.clone().unwrap();
        assert!(crate::ulid::is_valid(&id));
        assert_eq!(d.idea.file, "jellyfin-on-the-home-server.md");
        assert_eq!(d.idea.title, "Jellyfin on the home server");
        assert_eq!(d.content, format!("---\nid: {id}\ncreated: {}\n---\n\n# Jellyfin on the home server\n", today()));
        let again = create_at(&root, "Jellyfin on the home server").unwrap();
        assert_eq!(again.idea.file, "jellyfin-on-the-home-server-2.md", "a suffix on collision");
        assert_ne!(again.idea.id, d.idea.id);
        assert_eq!(list_at(&root).unwrap().ideas.len(), 2);
    }

    #[test]
    fn a_write_is_byte_exact_and_a_save_with_no_change_moves_neither_bytes_nor_mtime() {
        let t = Tmp::new("write");
        let root = t.root();
        let d = create_at(&root, "x").unwrap();
        let id = d.idea.id.unwrap();
        let text = rich(&id);
        let w1 = write_at(&root, &id, &text, Some(d.idea.mtime_ms)).unwrap();
        let m1 = w1.mtime_ms;
        let p = root.join(&w1.file);
        assert_eq!(fs::read(&p).unwrap(), text.as_bytes(), "exactly the bytes given");
        let before = fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let w2 = write_at(&root, &id, &text, Some(m1)).unwrap();
        assert_eq!(w2.mtime_ms, m1, "the same mtime back");
        assert_eq!(w2.file, w1.file, "and the same name");
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), before, "and on disk");
        assert_eq!(fs::read(&p).unwrap(), text.as_bytes());
        // The title is read from the heading, not the fence's `# ` line.
        assert_eq!(read_at(&root, &id).unwrap().idea.title, "Jellyfin on the home server");
        assert!(!root.join(HISTORY_DIR).exists(), "a save (typing) copies nothing to history");
    }

    /// Keep mine: the text it overwrites came from another editor and exists
    /// nowhere else, so it is copied to history first — and only when the
    /// write changes the bytes.
    #[test]
    fn a_kept_write_copies_the_text_it_replaces_to_history_first() {
        let t = Tmp::new("kept");
        let root = t.root();
        let d = create_at(&root, "x").unwrap();
        let id = d.idea.id.unwrap();
        let p = root.join(&d.idea.file);
        let theirs = format!("{}theirs, from another editor\n", d.content);
        fs::write(&p, &theirs).unwrap();
        let m = mtime_ms(&fs::metadata(&p).unwrap());
        let mine = format!("{}mine\n", d.content);
        let m2 = write_kept_at(&root, &id, &mine, Some(m)).unwrap().mtime_ms;
        assert_eq!(fs::read_to_string(&p).unwrap(), mine);
        let hist: Vec<_> = fs::read_dir(root.join(HISTORY_DIR).join(&id)).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(hist.len(), 1, "one copy");
        assert_eq!(fs::read_to_string(&hist[0]).unwrap(), theirs, "of the text it replaced");
        write_kept_at(&root, &id, &mine, Some(m2)).unwrap();
        assert_eq!(fs::read_dir(root.join(HISTORY_DIR).join(&id)).unwrap().count(), 1, "no change, no copy");
        let e = write_kept_at(&root, &id, &theirs, Some(1)).unwrap_err();
        assert!(e.contains("changed on disk"), "{e}");
        assert_eq!(fs::read_dir(root.join(HISTORY_DIR).join(&id)).unwrap().count(), 1, "a refused write copies nothing");
    }

    #[test]
    fn a_stale_expected_mtime_is_refused_and_writes_nothing() {
        let t = Tmp::new("stale");
        let root = t.root();
        let d = create_at(&root, "x").unwrap();
        let id = d.idea.id.unwrap();
        let e = write_at(&root, &id, &rich(&id), Some(d.idea.mtime_ms.wrapping_sub(1))).unwrap_err();
        assert!(e.contains("changed on disk"), "{e}");
        assert_eq!(fs::read_to_string(root.join(&d.idea.file)).unwrap(), d.content);
        for e in [
            rename_at(&root, &id, "y", Some(1)).unwrap_err(),
            remove_at(&root, &id, Some(1)).map(|_| unreachable!()).unwrap_err(),
        ] {
            assert!(e.contains("changed on disk"), "{e}");
        }
        assert!(!root.join(HISTORY_DIR).exists(), "a refused act copies nothing to history");
    }

    #[test]
    fn a_write_must_carry_the_documents_own_id() {
        let t = Tmp::new("idcheck");
        let root = t.root();
        let a = create_at(&root, "a").unwrap();
        let b = create_at(&root, "b").unwrap();
        let (ida, idb) = (a.idea.id.unwrap(), b.idea.id.unwrap());
        let e = write_at(&root, &ida, &rich(&idb), None).unwrap_err();
        assert!(e.contains("not") && e.contains(&idb), "{e}");
        let e = write_at(&root, &ida, "# no frontmatter\n", None).unwrap_err();
        assert!(e.contains("no id"), "{e}");
        assert_eq!(fs::read_to_string(root.join(&a.idea.file)).unwrap(), a.content, "untouched");
    }

    #[test]
    fn a_rename_moves_the_heading_and_the_name_and_every_other_byte_survives() {
        let t = Tmp::new("rename");
        let root = t.root();
        let d = create_at(&root, "Old").unwrap();
        let id = d.idea.id.unwrap();
        let text = rich(&id);
        write_at(&root, &id, &text, None).unwrap();
        let r = rename_at(&root, &id, "Jellyfin, at home!", None).unwrap();
        assert_eq!(r.idea.id.as_deref(), Some(id.as_str()), "the id does not move");
        assert_eq!(r.idea.file, "jellyfin-at-home.md");
        assert!(!root.join("old.md").exists());
        let want = text.replace("# Jellyfin on the home server\n", "# Jellyfin, at home!\n");
        assert_ne!(want, text);
        assert_eq!(fs::read_to_string(root.join(&r.idea.file)).unwrap(), want, "byte for byte outside the heading");
        // The pre-rename text is in history.
        let hist: Vec<_> = fs::read_dir(root.join(HISTORY_DIR).join(&id)).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(hist.len(), 1);
        assert_eq!(fs::read_to_string(&hist[0]).unwrap(), text);
        // A rename to the same slug keeps the name.
        let r2 = rename_at(&root, &id, "JELLYFIN at home", None).unwrap();
        assert_eq!(r2.idea.file, "jellyfin-at-home.md");
    }

    /// What the window sends: the text with its first heading edited.
    fn titled(id: &str, heading: Option<&str>) -> String {
        let h = heading.map_or(String::new(), |h| format!("# {h}\n\n"));
        format!("---\nid: {id}\ncreated: 2026-11-10\n---\n\n{h}body, kept as typed  \r\n")
    }

    fn files(root: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(root).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| !n.starts_with('.')).collect();
        v.sort();
        v
    }

    /// David's `untitled.md` whose first line became `# Jellyfin Sharing`.
    #[test]
    fn a_save_whose_heading_changed_renames_the_file_to_its_slug() {
        let t = Tmp::new("hname");
        let root = t.root();
        let d = create_at(&root, "").unwrap();
        assert_eq!(d.idea.file, "untitled.md");
        let id = d.idea.id.unwrap();
        let text = titled(&id, Some("Jellyfin Sharing"));
        let w = write_at(&root, &id, &text, Some(d.idea.mtime_ms)).unwrap();
        assert_eq!(w.file, "jellyfin-sharing.md");
        assert_eq!(w.title, "Jellyfin Sharing");
        assert_eq!(w.id.as_deref(), Some(id.as_str()), "the id does not move");
        assert_eq!(files(&root), ["jellyfin-sharing.md"], "untitled.md is gone");
        assert_eq!(fs::read_to_string(root.join(&w.file)).unwrap(), text, "byte for byte");
        let md = fs::symlink_metadata(root.join(&w.file)).unwrap();
        assert_eq!((w.mtime_ms, w.size), (mtime_ms(&md), md.len()), "the answer is the file's");
        assert!(!root.join(HISTORY_DIR).exists(), "a save's rename copies nothing to history");
        let r = read_at(&root, &id).unwrap();
        assert_eq!((r.idea.file.as_str(), r.idea.mtime_ms), ("jellyfin-sharing.md", w.mtime_ms), "the next save's expected mtime holds");
        // Another heading moves it again.
        let w2 = write_at(&root, &id, &titled(&id, Some("Jellyfin, shared!")), Some(w.mtime_ms)).unwrap();
        assert_eq!(w2.file, "jellyfin-shared.md");
        assert_eq!(files(&root), ["jellyfin-shared.md"]);
    }

    #[test]
    fn a_suffixed_name_for_the_same_title_does_not_bounce() {
        let t = Tmp::new("hbounce");
        let root = t.root();
        let a = create_at(&root, "Jellyfin Sharing").unwrap();
        let b = create_at(&root, "").unwrap();
        let idb = b.idea.id.unwrap();
        let w = write_at(&root, &idb, &titled(&idb, Some("Jellyfin Sharing")), None).unwrap();
        assert_eq!(w.file, "jellyfin-sharing-2.md", "a taken name gets a suffix");
        for n in 0..3 {
            let w = write_at(&root, &idb, &format!("{}more {n}\n", titled(&idb, Some("Jellyfin Sharing"))), None).unwrap();
            assert_eq!(w.file, "jellyfin-sharing-2.md", "save {n} keeps the suffix");
        }
        assert_eq!(files(&root), ["jellyfin-sharing-2.md", "jellyfin-sharing.md"]);
        assert_eq!(read_at(&root, a.idea.id.as_ref().unwrap()).unwrap().content, a.content, "the other idea is untouched");
        // A suffix that is part of the title is not one: "Roadmap 2026" → "Roadmap".
        let c = create_at(&root, "Roadmap 2026").unwrap();
        let idc = c.idea.id.unwrap();
        assert_eq!(write_at(&root, &idc, &titled(&idc, Some("Roadmap")), None).unwrap().file, "roadmap.md");
    }

    #[test]
    fn a_save_with_no_heading_names_the_file_untitled() {
        let t = Tmp::new("huntitled");
        let root = t.root();
        create_at(&root, "").unwrap();
        let d = create_at(&root, "Plan").unwrap();
        let id = d.idea.id.unwrap();
        let w = write_at(&root, &id, &titled(&id, None), None).unwrap();
        assert_eq!(w.file, "untitled-2.md", "untitled.md is taken");
        assert_eq!(w.title, "untitled-2", "no heading: the list shows the file name");
        let w = write_at(&root, &id, &titled(&id, Some("#")).replace("# #", "#"), None).unwrap();
        assert_eq!(w.file, "untitled-2.md", "an empty heading is no title, and the suffix holds");
    }

    #[test]
    fn a_hand_made_file_takes_its_headings_name_on_its_first_save_after_adoption() {
        let t = Tmp::new("hadopt");
        let root = t.root();
        raw_doc(&root, "Groceries.md", "# Groceries\n\n- milk\n");
        let a = adopt_at(&root, "Groceries.md", None).unwrap();
        assert_eq!(a.idea.file, "Groceries.md", "adoption keeps the name");
        let id = a.idea.id.unwrap();
        let text = format!("{}- eggs\n", a.content);
        let w = write_at(&root, &id, &text, Some(a.idea.mtime_ms)).unwrap();
        assert_eq!(w.file, "groceries.md", "a change of case only, on any volume");
        assert_eq!(files(&root), ["groceries.md"]);
        assert_eq!(fs::read_to_string(root.join("groceries.md")).unwrap(), text);
    }

    #[test]
    fn a_rename_of_a_document_with_no_heading_adds_one_after_the_frontmatter() {
        let id = crate::ulid::new();
        let t = format!("---\nid: {id}\n---\n\nbody\n");
        assert_eq!(with_title(&t, "T"), format!("---\nid: {id}\n---\n\n# T\n\nbody\n"));
        assert_eq!(with_title("body\n", " T\n2 "), "# T 2\n\nbody\n");
        assert_eq!(with_title("x\n# Old ##\r\ny", "New"), "x\n# New\r\ny", "the line ending stays");
    }

    #[test]
    fn remove_copies_to_history_then_deletes() {
        let t = Tmp::new("remove");
        let root = t.root();
        let d = create_at(&root, "Gone").unwrap();
        let id = d.idea.id.unwrap();
        remove_at(&root, &id, Some(d.idea.mtime_ms)).unwrap();
        assert!(!root.join("gone.md").exists());
        assert!(list_at(&root).unwrap().ideas.is_empty());
        let hist: Vec<_> = fs::read_dir(root.join(HISTORY_DIR).join(&id)).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(fs::read_to_string(&hist[0]).unwrap(), d.content);
        assert!(read_at(&root, &id).unwrap_err().contains("no idea"));
    }

    #[test]
    fn a_hand_made_file_is_listed_without_an_id_and_adopted_only_on_request() {
        let t = Tmp::new("adopt");
        let root = t.root();
        raw_doc(&root, "Groceries.md", "# Groceries\n\n- milk\n");
        raw_doc(&root, "fm.md", "---\ntags: [x]\n---\nno heading\n");
        raw_doc(&root, "notes.txt", "not markdown");
        let before = stamp_at(&root);
        let idx = list_at(&root).unwrap();
        assert_eq!(stamp_at(&root), before, "a listing writes nothing");
        let g = idx.ideas.iter().find(|d| d.file == "Groceries.md").unwrap();
        assert_eq!((g.id.as_deref(), g.problem, g.title.as_str()), (None, Some(Problem::NoId), "Groceries"));
        let f = idx.ideas.iter().find(|d| d.file == "fm.md").unwrap();
        assert_eq!(f.title, "fm", "no heading: the file name");
        assert_eq!(idx.ideas.len(), 2, "only .md");

        let a = adopt_at(&root, "Groceries.md", Some(g.mtime_ms)).unwrap();
        let id = a.idea.id.clone().unwrap();
        assert!(a.content.starts_with(&format!("---\nid: {id}\ncreated: ")));
        assert!(a.content.ends_with("---\n\n# Groceries\n\n- milk\n"), "{}", a.content);
        assert_eq!(a.idea.file, "Groceries.md", "the name stays");
        let a2 = adopt_at(&root, "fm.md", None).unwrap();
        let id2 = a2.idea.id.unwrap();
        assert!(a2.content.starts_with(&format!("---\nid: {id2}\ncreated: ")) && a2.content.ends_with("tags: [x]\n---\nno heading\n"), "{}", a2.content);
        assert!(adopt_at(&root, "Groceries.md", None).unwrap_err().contains("already has"));
        for bad in ["../x.md", ".history", "a/b.md", "notes.txt", "missing.md"] {
            assert!(adopt_at(&root, bad, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_bad_or_duplicated_id_is_listed_and_refused_by_every_lookup() {
        let t = Tmp::new("dups");
        let root = t.root();
        let d = create_at(&root, "One").unwrap();
        let id = d.idea.id.unwrap();
        fs::copy(root.join("one.md"), root.join("one copy.md")).unwrap();
        raw_doc(&root, "bad.md", "---\nid: ../../etc\n---\n# Bad\n");
        let idx = list_at(&root).unwrap();
        assert_eq!(idx.ideas.iter().filter(|d| d.problem == Some(Problem::DuplicateId)).count(), 2);
        assert_eq!(idx.ideas.iter().find(|d| d.file == "bad.md").unwrap().problem, Some(Problem::BadId));
        let e = read_at(&root, &id).unwrap_err();
        assert!(e.contains("two files carry") && e.contains("one copy.md"), "{e}");
        assert!(write_at(&root, &id, &d.content, None).is_err());
        assert!(read_at(&root, "../../etc").unwrap_err().contains("not an idea id"));
    }

    #[test]
    fn a_symlink_in_the_folder_is_not_a_document_and_is_never_followed() {
        let t = Tmp::new("link");
        let root = t.root();
        let d = create_at(&root, "Real").unwrap();
        let id = d.idea.id.clone().unwrap();
        // A link to a real idea of another folder, and one to a directory.
        let outside = t.0.join("outside.md");
        fs::write(&outside, format!("---\nid: {}\n---\n# Outside\n", crate::ulid::new())).unwrap();
        symlink(&outside, root.join("outside.md")).unwrap();
        symlink(&t.0, root.join("dir.md")).unwrap();
        let idx = list_at(&root).unwrap();
        assert_eq!(idx.ideas.len(), 1, "only the regular file");
        assert_eq!(idx.skipped, vec!["dir.md".to_string(), "outside.md".to_string()]);
        // A rename onto a name a link holds takes a suffix; the link stays.
        let r = rename_at(&root, &id, "Outside", None).unwrap();
        assert_eq!(r.idea.file, "outside-2.md");
        assert!(fs::symlink_metadata(root.join("outside.md")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&outside).unwrap().lines().nth(3), Some("# Outside"), "untouched");
        // A `.history` that is a link is refused, not written through.
        let t2 = Tmp::new("histlink");
        let r2 = t2.root();
        let d2 = create_at(&r2, "H").unwrap();
        fs::create_dir_all(t2.0.join("elsewhere")).unwrap();
        symlink(t2.0.join("elsewhere"), r2.join(HISTORY_DIR)).unwrap();
        let e = remove_at(&r2, d2.idea.id.as_deref().unwrap(), None).unwrap_err();
        assert!(e.contains("not a real directory"), "{e}");
        assert!(r2.join("h.md").exists(), "no history copy, no delete");
        assert_eq!(fs::read_dir(t2.0.join("elsewhere")).unwrap().count(), 0);
    }

    /// One bad entry is about that entry: a name gone by the `lstat` (an
    /// editor saving by rename) is skipped, a file that cannot be read is
    /// listed as unreadable and addressed by nothing, and every OTHER idea
    /// stays listed and readable. Scripted through the seam, never raced.
    #[test]
    fn one_vanished_or_unreadable_entry_does_not_fail_the_folder() {
        let t = Tmp::new("badentry");
        let root = t.root();
        let good = create_at(&root, "Good").unwrap();
        let locked = create_at(&root, "Locked").unwrap();
        let racy = create_at(&root, "Racy").unwrap();
        let names = ["good.md", "gone.md", "locked.md", "racy.md"].map(String::from);
        let read = |p: &Path| -> io::Result<Vec<u8>> {
            match p.file_name().and_then(|n| n.to_str()) {
                Some("locked.md") => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                // Gone between its `lstat` and its read.
                Some("racy.md") => Err(io::Error::from(io::ErrorKind::NotFound)),
                _ => fs::read(p),
            }
        };
        let idx = scan_with(&root, names.into_iter(), &read);
        let files: Vec<&str> = idx.ideas.iter().map(|d| d.file.as_str()).collect();
        assert!(!files.contains(&"gone.md") && !files.contains(&"racy.md"), "a vanished entry is skipped: {files:?}");
        let l = idx.ideas.iter().find(|d| d.file == "locked.md").unwrap();
        assert_eq!((l.problem, l.id.as_deref(), l.title.as_str()), (Some(Problem::Unreadable), None, "locked"));
        assert!(idx.resolve(locked.idea.id.as_deref().unwrap()).is_err(), "and addressed by nothing");
        assert!(idx.resolve(racy.idea.id.as_deref().unwrap()).is_err());
        assert_eq!(idx.resolve(good.idea.id.as_deref().unwrap()).unwrap().file, "good.md", "the others still resolve");
        // The real scan of the same folder, with nothing wrong, lists all three.
        assert_eq!(scan_at(&root).unwrap().ideas.len(), 3);
    }

    #[test]
    fn the_folder_itself_may_be_a_link() {
        let t = Tmp::new("rootlink");
        let real = t.0.join("synced");
        fs::create_dir_all(&real).unwrap();
        let root = t.0.join("ideas");
        symlink(&real, &root).unwrap();
        let d = create_at(&root, "Via link").unwrap();
        assert!(real.join("via-link.md").is_file());
        assert_eq!(list_at(&root).unwrap().ideas[0].id, d.idea.id);
    }

    #[test]
    fn a_slug_has_no_slash_no_leading_dot_and_a_length_cap() {
        assert_eq!(slug("Jellyfin on the home server"), "jellyfin-on-the-home-server");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug(".hidden"), "hidden");
        assert_eq!(slug("   "), "untitled");
        assert_eq!(slug("Café — ñandú 🦩"), "café-ñandú");
        let long = slug(&"á".repeat(100));
        assert!(long.len() <= SLUG_MAX && long.chars().all(|c| c == 'á'), "{long}");
        let long = slug(&"ab ".repeat(40));
        assert!(long.len() <= SLUG_MAX && !long.ends_with('-'), "{long}");
    }

    #[test]
    fn the_title_is_the_first_heading_outside_frontmatter_and_fences() {
        assert_eq!(title_of("---\nid: x\n# no\n---\n# Yes\n").as_deref(), Some("Yes"));
        assert_eq!(title_of("~~~\n# no\n~~~\n## sub\n# Yes #\n").as_deref(), Some("Yes"));
        assert_eq!(title_of("````\n```\n# no\n````\n#nospace\n   # Yes").as_deref(), Some("Yes"));
        assert_eq!(title_of("no heading"), None);
        assert_eq!(id_of("---\nid: \"01JC8Z4K3M9QX2T7VBN5R6WYHD\"\n---\n"), Some("01JC8Z4K3M9QX2T7VBN5R6WYHD"));
        assert_eq!(id_of("id: x\n"), None, "only in a frontmatter");
    }

    #[test]
    fn the_stamp_moves_on_an_outside_edit_and_not_on_a_listing() {
        let t = Tmp::new("stamp");
        let root = t.root();
        let d = create_at(&root, "S").unwrap();
        let s1 = stamp_at(&root);
        list_at(&root).unwrap();
        read_at(&root, d.idea.id.as_deref().unwrap()).unwrap();
        assert_eq!(stamp_at(&root), s1);
        fs::write(root.join("s.md"), format!("{}more\n", d.content)).unwrap();
        assert_ne!(stamp_at(&root), s1);
        // History and temps are not documents.
        fs::create_dir_all(root.join(HISTORY_DIR)).unwrap();
        let s2 = stamp_at(&root);
        fs::write(root.join(".s.md.0123456789abcdef.tmp"), "").unwrap();
        assert_eq!(stamp_at(&root), s2);
    }

    /// The guard's rule on its inputs. The switch itself is process-wide and
    /// set by the tests that start an engine, never here.
    #[test]
    fn a_forbidden_process_cannot_write_the_real_ideas_folder() {
        let real = Path::new("/home/u/.local/share/worktrees/ideas");
        let e = refuse_real_in(real, real, true).unwrap_err();
        assert!(e.contains("real ideas folder") && e.contains(OVERRIDE_ENV), "{e}");
        assert!(refuse_real_in(real, real, false).is_ok());
        assert!(refuse_real_in(Path::new("/tmp/t/ideas"), real, true).is_ok());
    }
}
