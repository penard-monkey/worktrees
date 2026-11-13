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

/// The frontmatter key that makes a document a plan and names its ideas.
pub const PLAN_OF: &str = "plan_of";

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

/// The line ending a document uses: `\r\n` when its first line ends so.
fn sep_of(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// The byte span of a top-level `key:` in the frontmatter: its line, and the
/// `- item` lines of a block list under it, each with its line ending.
fn front_key_span(text: &str, key: &str) -> Option<(usize, usize)> {
    let (s, e, _) = frontmatter(text)?;
    let mut at = s;
    while at < e {
        let end = text[at..e].find('\n').map_or(e, |i| at + i + 1);
        let line = text[at..end].trim_end_matches(['\n', '\r']);
        if line.strip_prefix(key).is_some_and(|r| r.starts_with(':')) {
            let mut stop = end;
            if line[key.len() + 1..].trim().is_empty() {
                while stop < e {
                    let next = text[stop..e].find('\n').map_or(e, |i| stop + i + 1);
                    if !text[stop..next].trim_start().starts_with("- ") && text[stop..next].trim() != "-" {
                        break;
                    }
                    stop = next;
                }
            }
            return Some((at, stop));
        }
        at = end;
    }
    None
}

/// The items of a frontmatter list, `key: [a, b]` (what the app writes),
/// `key: a`, or a block of `- a` lines under `key:`. Quotes are dropped.
pub fn front_list(text: &str, key: &str) -> Vec<String> {
    let Some((a, b)) = front_key_span(text, key) else { return Vec::new() };
    let chunk = &text[a..b];
    let first = chunk.lines().next().unwrap_or("");
    let v = first[key.len() + 1..].trim();
    let unq = |x: &str| x.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
    let items: Vec<String> = if let Some(inner) = v.strip_prefix('[') {
        inner.trim_end_matches(']').split(',').map(unq).collect()
    } else if !v.is_empty() {
        vec![unq(v)]
    } else {
        chunk.lines().skip(1).map(|l| unq(l.trim_start().trim_start_matches('-'))).collect()
    };
    let mut out: Vec<String> = Vec::new();
    for i in items {
        if !i.is_empty() && !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

/// `text` with the frontmatter's `key` set to the list `items`, written
/// `key: [a, b]`: the key's line (and a block list's lines) replaced, or a
/// line added before the closing `---`. An empty list removes the key.
/// Every other byte is copied through. `None` without a frontmatter.
fn with_front_list(text: &str, key: &str, items: &[String]) -> Option<String> {
    let (_, e, _) = frontmatter(text)?;
    let sep = sep_of(text);
    let line = if items.is_empty() { String::new() } else { format!("{key}: [{}]{sep}", items.join(", ")) };
    Some(match front_key_span(text, key) {
        Some((a, b)) => format!("{}{line}{}", &text[..a], &text[b..]),
        None => format!("{}{line}{}", &text[..e], &text[e..]),
    })
}

/// Every idea a body links to as `idea:<ULID>` (`[title](idea:<id>)`), in
/// order, once each.
pub fn links_in(text: &str) -> Vec<String> {
    let from = frontmatter(text).map_or(0, |(_, _, b)| b);
    let body = &text[from..];
    let mut out: Vec<String> = Vec::new();
    for (i, _) in body.match_indices("idea:") {
        if let Some(id) = body.get(i + 5..i + 31) {
            let ends = body[i + 31..].chars().next().is_none_or(|c| !c.is_ascii_alphanumeric());
            if ends && crate::ulid::is_valid(id) && !out.iter().any(|o| o == id) {
                out.push(id.to_string());
            }
        }
    }
    out
}

/// A document's body with its first heading's line taken out, and blank
/// lines trimmed at both ends: what a merge appends under its own heading.
fn body_without_title(text: &str) -> String {
    let b = frontmatter(text).map_or(0, |(_, _, b)| b);
    let rest = match heading(text) {
        Some((s, e, _)) => {
            let e = if text[e..].starts_with("\r\n") { e + 2 } else if text[e..].starts_with('\n') { e + 1 } else { e };
            format!("{}{}", &text[b..s], &text[e..])
        }
        None => text[b..].to_string(),
    };
    rest.trim_start_matches(['\n', '\r']).trim_end().to_string()
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
    /// `idea`, or `plan` when its frontmatter carries a `plan_of:` key
    /// (phase 4, §2.2): a plan is a document in the same folder.
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<Problem>,
}

/// What a document is: an idea, or a plan made from ideas (§2.2). Decided by
/// one fact, a `plan_of:` key in the frontmatter (an empty list included),
/// so the list, the index and every row read it the same way.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Idea,
    Plan,
}

impl Kind {
    pub fn of(text: &str) -> Kind {
        if front_key_span(text, PLAN_OF).is_some() { Kind::Plan } else { Kind::Idea }
    }
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
    /// A merged-away id → the live id whose `absorbed:` names it (phase 3,
    /// §3), or `None` when two live documents both claim it. An id that is
    /// live itself is never here: a restored idea is its own again.
    #[serde(skip)]
    absorbed: HashMap<String, Option<String>>,
    /// Each live id's relations as its own file writes them.
    #[serde(skip)]
    rels: HashMap<String, Rels>,
}

/// What one document's text says about other ideas (§3's table): its
/// frontmatter's `related`, `absorbed` and `from`, and the `idea:<id>` links
/// in its body. Read, never printed back.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Rels {
    pub related: Vec<String>,
    pub absorbed: Vec<String>,
    pub from: Option<String>,
    pub links: Vec<String>,
    /// A plan's ideas (phase 4, §2.2), as written: an absorbed id stays as
    /// it is and resolves through the index.
    pub plan_of: Vec<String>,
}

impl Rels {
    fn of(text: &str) -> Rels {
        Rels {
            related: front_list(text, "related"),
            absorbed: front_list(text, "absorbed"),
            from: front_value(text, "from").filter(|v| !v.is_empty()).map(str::to_string),
            links: links_in(text),
            plan_of: front_list(text, PLAN_OF),
        }
    }
}

impl Index {
    /// The document `id` names, or why none can be addressed by it. An id
    /// that was merged away resolves to the idea that absorbed it (§3): a
    /// link, a plan or a workspace that names it keeps working. A READ goes
    /// through here; an act that changes a file uses [`Index::resolve_exact`].
    pub fn resolve(&self, id: &str) -> Result<&Idea, String> {
        self.lookup(id).map(|(d, _)| d)
    }

    /// [`Index::resolve`], and whether it went through an `absorbed` id.
    pub fn lookup(&self, id: &str) -> Result<(&Idea, bool), String> {
        match self.exact(id) {
            Ok(d) => Ok((d, false)),
            Err(e) => match self.absorbed.get(id) {
                Some(Some(by)) => Ok((&self.ideas[self.by_id[by]], true)),
                Some(None) => Err(format!("the id {id} was merged into more than one idea: remove it from one `absorbed:` list by hand")),
                None => Err(e),
            },
        }
    }

    /// The document whose OWN id is `id`. A merged-away id is refused here
    /// with the idea that absorbed it, so an act never changes A when it was
    /// asked to change B.
    pub fn resolve_exact(&self, id: &str) -> Result<&Idea, String> {
        self.exact(id).map_err(|e| match self.absorbed.get(id) {
            Some(Some(by)) => format!("that idea was merged into “{}”: act on that one", self.ideas[self.by_id[by]].title),
            _ => e,
        })
    }

    /// The live id that absorbed `id`, when it was merged away.
    pub fn absorbed_by(&self, id: &str) -> Option<&str> {
        self.absorbed.get(id).and_then(|o| o.as_deref())
    }

    /// A live idea's relations as its own file writes them.
    pub fn rels(&self, id: &str) -> Option<&Rels> {
        self.rels.get(id)
    }

    fn exact(&self, id: &str) -> Result<&Idea, String> {
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
    let mut rels_by_file: HashMap<String, Rels> = HashMap::new();
    for name in names {
        if name.starts_with('.') || !name.ends_with(".md") {
            continue;
        }
        let p = root.join(&name);
        let stem = name.trim_end_matches(".md").to_string();
        let unreadable = |name: String| Idea { id: None, title: stem.clone(), file: name, created: None, mtime_ms: 0, size: 0, kind: Kind::Idea, problem: Some(Problem::Unreadable) };
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
        let mut doc = Idea { id: None, title: stem.clone(), file: name, created: None, mtime_ms: mtime_ms(&md), size: md.len(), kind: Kind::Idea, problem: None };
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
                    doc.kind = Kind::of(&text);
                    match id_of(&text) {
                        None => doc.problem = Some(Problem::NoId),
                        Some(id) if !crate::ulid::is_valid(id) => doc.problem = Some(Problem::BadId),
                        Some(id) => {
                            doc.id = Some(id.to_string());
                            rels_by_file.insert(doc.file.clone(), Rels::of(&text));
                        }
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
    // Relations and absorbed ids, from the live documents only.
    let mut live: Vec<(&String, &usize)> = idx.by_id.iter().collect();
    live.sort();
    for (id, i) in live {
        let Some(r) = rels_by_file.remove(&idx.ideas[*i].file) else { continue };
        for a in &r.absorbed {
            if a == id || idx.by_id.contains_key(a) || !crate::ulid::is_valid(a) {
                continue;
            }
            match idx.absorbed.get(a) {
                Some(Some(by)) if by != id => {
                    idx.absorbed.insert(a.clone(), None);
                }
                Some(_) => {}
                None => {
                    idx.absorbed.insert(a.clone(), Some(id.clone()));
                }
            }
        }
        idx.rels.insert(id.clone(), r);
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
    /// Set when the id asked for was merged into this idea (§3): "B was
    /// merged into A on <date>".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged: Option<Merged>,
}

/// The id a read asked for, which was merged into the idea it answers with:
/// its last title and the day it was absorbed, both from its newest history
/// copy (the merge took one), when there is one.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub id: String,
    pub title: Option<String>,
    pub date: Option<String>,
}

pub fn list_at(root: &Path) -> Result<Index, String> {
    scan_at(root).map_err(|e| format!("{}: {e}", root.display()))
}

pub fn read_at(root: &Path, id: &str) -> Result<Doc, String> {
    let idx = list_at(root)?;
    let (idea, via) = idx.lookup(id)?;
    let idea = idea.clone();
    let (content, md) = load(root, &idea)?;
    let merged = via.then(|| {
        let last = versions_at(root, id).ok().and_then(|v| v.into_iter().next());
        Merged { id: id.to_string(), title: last.as_ref().and_then(|v| v.title.clone()), date: last.map(|v| v.at[..10].to_string()) }
    });
    Ok(Doc { idea: Idea { mtime_ms: mtime_ms(&md), size: md.len(), ..idea }, content, merged })
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

pub fn date_of(epoch: i64) -> String {
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
    let idea = idx.resolve_exact(id)?.clone();
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
        kind: Kind::of(content),
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
    let idea = idx.resolve_exact(id)?.clone();
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
    let idea = idx.resolve_exact(id)?.clone();
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

// ── phase 3: ideas relate (§3) ──────────────────────────────────────────────
//
// The four acts are two bodies (C11): a MERGE (into an existing idea, or into
// a new one: combine) and a SPLIT (moving the text, or copying it: extract).
// Each takes its whole input as parameters, so an agent's proposal later is
// the same call (§7.1), and each copies every file it is about to change to
// `.history/<id>/` FIRST. The other side of a relation is computed from the
// index, never written.

/// `ids` once each, in order, leaving out `skip`.
fn union(lists: &[&[String]], skip: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in lists {
        for i in *l {
            if !skip.contains(&i.as_str()) && !out.contains(i) {
                out.push(i.clone());
            }
        }
    }
    out
}

/// A body handed in for a merge, combine or rewrite: the text AFTER the
/// frontmatter, which stays the core's. One that carries its own frontmatter
/// is refused, so a client can never write ids or `absorbed` by hand.
fn check_body(text: &str) -> Result<(), String> {
    if frontmatter(text).is_some() {
        return Err("the text must be the idea's body, without its frontmatter (the `---` block)".into());
    }
    Ok(())
}

/// Relate `id` to `other` (§3): `other` goes into `id`'s `related:` list.
/// `other` may be a merged-away id: the relation is to the idea that holds it
/// now. Already related, either way round, writes nothing.
pub fn relate_at(root: &Path, id: &str, other: &str) -> Result<Doc, String> {
    refuse_real(root)?;
    let idx = list_at(root)?;
    let a = idx.resolve_exact(id)?.clone();
    let b = idx.resolve(other)?.clone();
    let b_id = b.id.clone().unwrap_or_default();
    if b_id == id {
        return Err("an idea cannot be related to itself".into());
    }
    let mine = idx.rels(id).cloned().unwrap_or_default();
    let theirs = idx.rels(&b_id).cloned().unwrap_or_default();
    if mine.related.contains(&b_id) || theirs.related.iter().any(|r| r == id) {
        return read_at(root, id);
    }
    let (text, md) = load(root, &a)?;
    let mut related = mine.related.clone();
    related.push(b_id);
    let new_text = with_front_list(&text, "related", &related).ok_or("the idea has no frontmatter")?;
    history_copy_at(root, id, text.as_bytes())?;
    crate::fsx::write_atomic(&root.join(&a.file), new_text.as_bytes(), Some(md.permissions())).map_err(|e| format!("{}: {e}", a.file))?;
    read_at(root, id)
}

/// The section a merge appends for one absorbed idea.
fn from_section(title: &str, date: &str, text: &str, sep: &str) -> String {
    let body = body_without_title(text);
    let mut s = format!("## From: {title} ({date}){sep}");
    if !body.is_empty() {
        s.push_str(sep);
        s.push_str(&body.replace("\r\n", "\n").replace('\n', sep));
        s.push_str(sep);
    }
    s
}

/// Merge `from` into `into` (§3): `into` keeps its id and gains `from`'s
/// text, by default appended under `## From: <from's title> (date)`, or
/// replaced by `text` (the whole BODY, after the frontmatter: an agent's
/// rewrite is the same call). `from`'s id, and every id it had absorbed, go
/// into `into`'s `absorbed:`; its `related:` joins `into`'s. Both files are
/// copied to history first; then `into` is written and `from`'s file removed.
pub fn merge_at(root: &Path, into: &str, from: &str, text: Option<&str>, into_mtime: Option<u64>, from_mtime: Option<u64>) -> Result<Doc, String> {
    refuse_real(root)?;
    if let Some(t) = text {
        check_body(t)?;
    }
    let idx = list_at(root)?;
    let a = idx.resolve_exact(into)?.clone();
    let b = idx.resolve_exact(from)?.clone();
    if into == from {
        return Err("an idea cannot be merged into itself".into());
    }
    let (a_text, a_md) = load(root, &a)?;
    let (b_text, b_md) = load(root, &b)?;
    check_mtime(&a_md, into_mtime)?;
    check_mtime(&b_md, from_mtime)?;
    let ra = idx.rels(into).cloned().unwrap_or_default();
    let rb = idx.rels(from).cloned().unwrap_or_default();
    let absorbed = union(&[&ra.absorbed, std::slice::from_ref(&from.to_string()), &rb.absorbed], &[into]);
    let skip: Vec<&str> = absorbed.iter().map(String::as_str).chain([into, from]).collect();
    let related = union(&[&ra.related, &rb.related], &skip);
    let sep = sep_of(&a_text);
    let fronted = with_front_list(&a_text, "absorbed", &absorbed).ok_or("the idea has no frontmatter")?;
    let fronted = with_front_list(&fronted, "related", &related).ok_or("the idea has no frontmatter")?;
    let body_at = frontmatter(&fronted).map_or(0, |(_, _, b)| b);
    let new_text = match text {
        Some(t) => format!("{}{t}", &fronted[..body_at]),
        None => {
            let kept = fronted.trim_end();
            format!("{kept}{sep}{sep}{}", from_section(&b.title, &today(), &b_text, sep))
        }
    };
    history_copy_at(root, into, a_text.as_bytes())?;
    history_copy_at(root, from, b_text.as_bytes())?;
    crate::fsx::write_atomic(&root.join(&a.file), new_text.as_bytes(), Some(a_md.permissions())).map_err(|e| format!("{}: {e}", a.file))?;
    rename_to_title(root, &a.file, &new_text)?;
    fs::remove_file(root.join(&b.file)).map_err(|e| {
        format!("merged into “{}”, but could not remove {}: {e}. Both are listed until it is removed by hand", a.title, b.file)
    })?;
    read_at(root, into)
}

/// Combine `ids` (two or more) into a NEW idea titled `title` (§3, C11: a
/// merge whose target is new). Its body is `text` when given (the body,
/// after the frontmatter), else `# title` and each idea's text under its own
/// `## From:` heading, in the order given. Every source id, and every id
/// each had absorbed, go into the new idea's `absorbed:`. Each source is
/// copied to history first; then the new file is made and the sources are
/// removed.
pub fn combine_at(root: &Path, ids: &[String], title: &str, text: Option<&str>) -> Result<Doc, String> {
    refuse_real(root)?;
    if let Some(t) = text {
        check_body(t)?;
    }
    if ids.len() < 2 {
        return Err("combining needs two ideas or more".into());
    }
    let idx = list_at(root)?;
    let mut srcs: Vec<(Idea, String)> = Vec::new();
    for id in ids {
        if srcs.iter().any(|(d, _)| d.id.as_deref() == Some(id)) {
            return Err(format!("the idea {id} is named twice"));
        }
        let d = idx.resolve_exact(id)?.clone();
        let (t, _) = load(root, &d)?;
        srcs.push((d, t));
    }
    let title = clean_title(title);
    let rels: Vec<Rels> = ids.iter().map(|i| idx.rels(i).cloned().unwrap_or_default()).collect();
    let ab: Vec<&[String]> = ids.iter().zip(&rels).flat_map(|(i, r)| [std::slice::from_ref(i), &r.absorbed[..]]).collect();
    let absorbed = union(&ab, &[]);
    let skip: Vec<&str> = absorbed.iter().map(String::as_str).collect();
    let rl: Vec<&[String]> = rels.iter().map(|r| &r.related[..]).collect();
    let related = union(&rl, &skip);
    let id = crate::ulid::new();
    let date = today();
    let mut front = format!("---\nid: {id}\ncreated: {date}\nabsorbed: [{}]\n", absorbed.join(", "));
    if !related.is_empty() {
        front.push_str(&format!("related: [{}]\n", related.join(", ")));
    }
    front.push_str("---\n\n");
    let body = match text {
        Some(t) => t.to_string(),
        None => {
            let mut b = format!("# {title}\n");
            for (d, t) in &srcs {
                b.push('\n');
                b.push_str(&from_section(&d.title, &date, t, "\n"));
            }
            b
        }
    };
    let content = format!("{front}{body}");
    for (d, t) in &srcs {
        history_copy_at(root, d.id.as_deref().unwrap_or_default(), t.as_bytes())?;
    }
    let stem = slug(&clean_title(&title_of(&content).unwrap_or(title)));
    create_named(root, &stem, &content)?;
    let mut left = Vec::new();
    for (d, _) in &srcs {
        if let Err(e) = fs::remove_file(root.join(&d.file)) {
            left.push(format!("{}: {e}", d.file));
        }
    }
    if !left.is_empty() {
        return Err(format!("combined, but could not remove {}: both copies are listed until removed by hand", left.join("; ")));
    }
    read_at(root, &id)
}

/// Make `<stem>.md` (or the first free `-<n>`) holding `content`.
fn create_named(root: &Path, stem: &str, content: &str) -> Result<String, String> {
    ensure_root(root)?;
    for _ in 0..16 {
        let file = free_name(root, stem, None);
        match create_new(&root.join(&file), content.as_bytes()) {
            Ok(()) => return Ok(file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{file}: {e}")),
        }
    }
    Err(format!("no free file name for {stem:?}"))
}

/// A UTF-16 offset (what a JavaScript string counts) as a byte offset in
/// `text`; `None` past the end or inside a character.
pub fn utf16_to_byte(text: &str, n: usize) -> Option<usize> {
    let mut acc = 0;
    for (b, c) in text.char_indices() {
        if acc == n {
            return Some(b);
        }
        if acc > n {
            return None;
        }
        acc += c.len_utf16();
    }
    (acc == n).then_some(text.len())
}

/// The title a split takes from its text when it is given none: the first
/// line with words, its markdown markers dropped, cut to 80 characters.
fn title_from(sel: &str) -> String {
    let line = sel.lines().map(|l| l.trim_start_matches(|c: char| c.is_whitespace() || "#>-*+".contains(c)).trim()).find(|l| !l.is_empty());
    clean_title(&line.unwrap_or("").chars().take(80).collect::<String>())
}

/// What a split made: the new idea, and the source as it is after it.
#[derive(Serialize, Debug)]
pub struct Split {
    pub new: Doc,
    pub source: Doc,
}

/// Split `start..end` of the idea `id`'s CURRENT text (UTF-16 offsets, what
/// the editor's string counts; the text is the one `expected_mtime` names)
/// into a new idea that carries `from: <id>` (§3). Its title is `title`, or
/// the selection's first line. With `keep` (extract) the source is not
/// touched; otherwise the selection is replaced in it by a link,
/// `[<title>](idea:<new id>)`, keeping the selection's leading and trailing
/// blank space so the paragraphs around it stay where they were, after the
/// source is copied to history.
pub fn split_at(root: &Path, id: &str, start: usize, end: usize, keep: bool, title: Option<&str>, expected_mtime: Option<u64>) -> Result<Split, String> {
    refuse_real(root)?;
    let idx = list_at(root)?;
    let a = idx.resolve_exact(id)?.clone();
    let (text, md) = load(root, &a)?;
    check_mtime(&md, expected_mtime)?;
    let (Some(s), Some(e)) = (utf16_to_byte(&text, start), utf16_to_byte(&text, end)) else {
        return Err("the selection is not inside the idea's text: reload it and select again".into());
    };
    if s >= e {
        return Err("nothing is selected".into());
    }
    if s < frontmatter(&text).map_or(0, |(_, _, b)| b) {
        return Err("the selection reaches into the frontmatter (the `---` block): select text below it".into());
    }
    let sel = &text[s..e];
    let core = sel.trim();
    if core.is_empty() {
        return Err("the selection has no text".into());
    }
    let title = match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => clean_title(t),
        None => title_from(core),
    };
    let sep = sep_of(&text);
    let new_id = crate::ulid::new();
    let picked = sel.trim_start_matches(['\n', '\r']).trim_end();
    let content = format!("---{sep}id: {new_id}{sep}created: {}{sep}from: {id}{sep}---{sep}{sep}# {title}{sep}{sep}{picked}{sep}", today());
    let lead = &sel[..sel.len() - sel.trim_start().len()];
    let trail = &sel[sel.trim_end().len()..];
    let label = title.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]");
    let new_src = format!("{}{lead}[{label}](idea:{new_id}){trail}{}", &text[..s], &text[e..]);
    if !keep {
        history_copy_at(root, id, text.as_bytes())?;
    }
    create_named(root, &slug(&title), &content)?;
    if !keep {
        crate::fsx::write_atomic(&root.join(&a.file), new_src.as_bytes(), Some(md.permissions()))
            .map_err(|err| format!("made “{title}”, but could not replace the text in {}: {err}", a.file))?;
    }
    Ok(Split { new: read_at(root, &new_id)?, source: read_at(root, id)? })
}

// ── history: versions and restore ───────────────────────────────────────────

/// One copy in `.history/<id>/`.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Its bare file name: what `restore_at` takes.
    pub version: String,
    /// When it was taken, `2026-11-08T15:30:12.345Z`.
    pub at: String,
    pub size: u64,
    /// The title it had then.
    pub title: Option<String>,
}

/// `20261108T153012.345Z[-n].md` → `2026-11-08T15:30:12.345Z`.
fn stamp_iso(name: &str) -> Option<String> {
    let b = name.as_bytes();
    if b.len() < 23 || b[8] != b'T' || b[19] != b'Z' || b[15] != b'.' || !name.ends_with(".md") {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| name.get(r.clone()).filter(|x| x.bytes().all(|c| c.is_ascii_digit()));
    Some(format!(
        "{}-{}-{}T{}:{}:{}.{}Z",
        digits(0..4)?, digits(4..6)?, digits(6..8)?, digits(9..11)?, digits(11..13)?, digits(13..15)?, digits(16..19)?
    ))
}

/// The history folder of `id`, when it is a real directory (never a link).
fn history_dir(root: &Path, id: &str) -> Result<Option<PathBuf>, String> {
    if !crate::ulid::is_valid(id) {
        return Err(format!("not an idea id: {id:?}"));
    }
    let h = root.join(HISTORY_DIR);
    let d = h.join(id);
    for p in [&h, &d] {
        match fs::symlink_metadata(p) {
            Ok(md) if md.file_type().is_dir() => {}
            Ok(_) => return Err(format!("{} is not a folder", p.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", p.display())),
        }
    }
    Ok(Some(d))
}

/// Every copy history holds of the idea `id`, newest first. The id is taken
/// as it is (a removed or merged-away idea has versions too); none is an
/// empty list. Only regular files with a history stamp for a name.
pub fn versions_at(root: &Path, id: &str) -> Result<Vec<Version>, String> {
    let Some(d) = history_dir(root, id)? else { return Ok(Vec::new()) };
    let rd = fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    let mut out = Vec::new();
    for e in rd.filter_map(Result::ok) {
        let Ok(name) = e.file_name().into_string() else { continue };
        let Some(at) = stamp_iso(&name) else { continue };
        let Ok(md) = fs::symlink_metadata(e.path()) else { continue };
        if !md.file_type().is_file() {
            continue;
        }
        let title = (md.len() <= MAX_BYTES)
            .then(|| fs::read(e.path()).ok())
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|t| title_of(&t));
        out.push(Version { version: name, at, size: md.len(), title });
    }
    out.sort_by(|x, y| y.version.cmp(&x.version));
    Ok(out)
}

/// The exact text of one history copy of `id`.
pub fn version_text_at(root: &Path, id: &str, version: &str) -> Result<String, String> {
    if stamp_iso(version).is_none() || version.contains('/') {
        return Err(format!("not a version name: {version:?}"));
    }
    let Some(d) = history_dir(root, id)? else { return Err(format!("the idea {id} has no history")) };
    let p = d.join(version);
    let md = fs::symlink_metadata(&p).map_err(|e| format!("{version}: {e}"))?;
    if !md.file_type().is_file() || md.len() > MAX_BYTES {
        return Err(format!("{version} is not a readable copy"));
    }
    String::from_utf8(fs::read(&p).map_err(|e| format!("{version}: {e}"))?).map_err(|_| format!("{version} is not UTF-8"))
}

/// Put the history copy `version` back as the idea `id` (§3: a merge, split
/// or remove that was a mistake is undone this way). A live idea's current
/// file is copied to history FIRST, then replaced (and renamed after its
/// heading). An id with no live file (removed, or merged away) is made
/// again under its own id, as a new file; a merge that absorbed it keeps its
/// text and its `absorbed:` line, and the live file answers for the id. A
/// copy whose own `id` is not `id` (the text an adoption replaced) is refused.
pub fn restore_at(root: &Path, id: &str, version: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    refuse_real(root)?;
    let text = version_text_at(root, id, version)?;
    if id_of(&text) != Some(id) {
        return Err("that copy was taken before the idea had its id, so it cannot be restored as this idea: open it from .history by hand".into());
    }
    let idx = list_at(root)?;
    match idx.exact(id) {
        Ok(idea) => {
            let idea = idea.clone();
            let (now, md) = load(root, &idea)?;
            check_mtime(&md, expected_mtime)?;
            if now != text {
                history_copy_at(root, id, now.as_bytes())?;
                crate::fsx::write_atomic(&root.join(&idea.file), text.as_bytes(), Some(md.permissions())).map_err(|e| format!("{}: {e}", idea.file))?;
            }
            rename_to_title(root, &idea.file, &text)?;
        }
        Err(_) if idx.ideas.iter().any(|d| d.id.as_deref() == Some(id)) => {
            return Err(idx.exact(id).err().unwrap_or_default());
        }
        Err(_) => {
            create_named(root, &slug(&clean_title(&title_of(&text).unwrap_or_default())), &text)?;
        }
    }
    read_at(root, id)
}

// ── related ideas ───────────────────────────────────────────────────────────

/// One idea related to another, and how.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub id: String,
    pub title: String,
    /// `related` (this idea's `related:` names it), `related-by` (its
    /// `related:` names this one), `links-to` / `linked-from` (an `idea:`
    /// link in the text), `split-from` (this idea's `from:`), `split-into`
    /// (its `from:` is this one), `plan-of` (this plan's `plan_of:` names
    /// it) or `planned-in` (it is a plan whose `plan_of:` names this idea,
    /// phase 4). A plan's own links to its ideas are said once, as
    /// `plan-of` / `planned-in`, not again as links.
    pub how: &'static str,
    /// The merged-away id the relation was written with, when it resolved
    /// through one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

/// Every idea related to `id`, written and computed (§3): its own
/// `related:`, `from:` and links, and every other live idea whose
/// `related:`, `from:` or links name it or an id it absorbed. An id that
/// resolves nowhere is left out; the idea itself never appears.
pub fn related_at(root: &Path, id: &str) -> Result<Vec<Relation>, String> {
    let idx = list_at(root)?;
    let me = idx.resolve(id)?.id.clone().unwrap_or_default();
    let mine = idx.rels(&me).cloned().unwrap_or_default();
    let names_me = |x: &str| x == me || idx.absorbed_by(x) == Some(me.as_str());
    let mut out: Vec<Relation> = Vec::new();
    let mut push = |other: &str, how: &'static str| {
        let Ok((d, via)) = idx.lookup(other) else { return };
        let Some(oid) = d.id.clone() else { return };
        if oid == me || out.iter().any(|r| r.id == oid && r.how == how) {
            return;
        }
        out.push(Relation { id: oid, title: d.title.clone(), how, via: via.then(|| other.to_string()) });
    };
    for r in &mine.related {
        push(r, "related");
    }
    if let Some(f) = &mine.from {
        push(f, "split-from");
    }
    for p in &mine.plan_of {
        push(p, "plan-of");
    }
    let live = |x: &str| idx.resolve(x).ok().and_then(|d| d.id.clone());
    let planned: Vec<String> = mine.plan_of.iter().filter_map(|p| live(p)).collect();
    for l in &mine.links {
        if !live(l).is_some_and(|i| planned.contains(&i)) {
            push(l, "links-to");
        }
    }
    let mut others: Vec<(&String, &Rels)> = idx.rels.iter().filter(|(k, _)| **k != me).collect();
    others.sort_by_key(|(k, _)| *k);
    for (other, r) in others {
        if r.related.iter().any(|x| names_me(x)) {
            push(other, "related-by");
        }
        if r.from.as_deref().is_some_and(names_me) {
            push(other, "split-into");
        }
        let plans_me = r.plan_of.iter().any(|x| names_me(x));
        if plans_me {
            push(other, "planned-in");
        }
        if !plans_me && r.links.iter().any(|x| names_me(x)) {
            push(other, "linked-from");
        }
    }
    Ok(out)
}

// ── phase 4: plans (§2.2) ───────────────────────────────────────────────────
//
// A plan is a document in the same folder whose frontmatter carries
// `plan_of: [ids]` ([`Kind::Plan`]). Making one is ONE operation with its
// whole input as parameters, the draft included (§7.1: an agent's draft is
// the same call); each later change the app makes to a plan (its ideas, a
// project line) copies the file to history first and checks
// `expected_mtime`. The ideas themselves are never written: "planned in" is
// computed ([`related_at`]). A plan naming an id that was later merged away
// keeps resolving through the index, and is never rewritten for it.

/// A title as the text of a markdown link: `\`, `[` and `]` escaped.
fn link_label(title: &str) -> String {
    title.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]")
}

/// The heading a plan is given: `Plan: <title>`, unless the title says so.
fn plan_title(title: &str) -> String {
    let t = clean_title(title);
    if t.to_lowercase().starts_with("plan:") { t } else { format!("Plan: {t}") }
}

/// The body [`make_plan_at`] writes when it is given no draft: the heading,
/// a line linking every idea by title, and the three sections the template
/// of §2.2 has, empty for David to fill.
pub fn plan_template(title: &str, ideas: &[(String, String)]) -> String {
    let links: Vec<String> = ideas.iter().map(|(id, t)| format!("[{}](idea:{id})", link_label(t))).collect();
    let from = match links.len() {
        0 => String::new(),
        1 => links[0].clone(),
        n => format!("{} and {}", links[..n - 1].join(", "), links[n - 1]),
    };
    format!("# {}\n\nFrom {from}.\n\n## Goal\n\n## Projects\n\n## Work\n", plan_title(title))
}

/// `text` with `plan_of:` set to `items`; an empty list is written
/// `plan_of: []`, never dropped, because the key is what makes it a plan.
fn with_plan_of(text: &str, items: &[String]) -> Option<String> {
    if !items.is_empty() {
        return with_front_list(text, PLAN_OF, items);
    }
    let sep = sep_of(text);
    let (a, b) = front_key_span(text, PLAN_OF)?;
    Some(format!("{}{PLAN_OF}: []{sep}{}", &text[..a], &text[b..]))
}

/// The live idea each of `ids` names (an absorbed id resolves to the idea
/// that took it), once each, refusing a plan and `not`.
fn live_ideas(idx: &Index, ids: &[String], not: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    for id in ids {
        let d = idx.resolve(id)?;
        let live = d.id.clone().unwrap_or_default();
        if d.kind == Kind::Plan {
            return Err(format!("“{}” is a plan: a plan is made from ideas", d.title));
        }
        if Some(live.as_str()) == not {
            return Err("a plan cannot name itself".into());
        }
        if !out.iter().any(|(i, _)| *i == live) {
            out.push((live, d.title.clone()));
        }
    }
    Ok(out)
}

/// Make a plan from `ideas` (one or more; §2.2): a new document whose
/// frontmatter is `id`, `plan_of` (each idea's live id, once) and `created`,
/// and whose body is `draft` when given (the body after the frontmatter, as
/// [`merge_at`]'s `text`: one that carries its own frontmatter is refused),
/// else [`plan_template`] titled `title`, or the first idea's title. The
/// file is named, as every document is, by its first heading: a plan titled
/// `Plan: Jellyfin` is `plan-jellyfin.md`. No idea is written.
pub fn make_plan_at(root: &Path, ideas: &[String], title: Option<&str>, draft: Option<&str>) -> Result<Doc, String> {
    refuse_real(root)?;
    if let Some(d) = draft {
        check_body(d)?;
    }
    if ideas.is_empty() {
        return Err("a plan is made from one idea or more".into());
    }
    let idx = list_at(root)?;
    let live = live_ideas(&idx, ideas, None)?;
    let title = title.map(str::trim).filter(|t| !t.is_empty()).map_or_else(|| live[0].1.clone(), str::to_string);
    let id = crate::ulid::new();
    let ids: Vec<String> = live.iter().map(|(i, _)| i.clone()).collect();
    let body = match draft {
        Some(d) => d.to_string(),
        None => plan_template(&title, &live),
    };
    let content = format!("---\nid: {id}\n{PLAN_OF}: [{}]\ncreated: {}\n---\n\n{body}", ids.join(", "), today());
    let stem = slug(&title_of(&content).unwrap_or_else(|| plan_title(&title)));
    create_named(root, &stem, &content)?;
    read_at(root, &id)
}

/// The plan `plan`, refused when the document is not one.
fn plan_doc<'a>(idx: &'a Index, plan: &str) -> Result<&'a Idea, String> {
    let p = idx.resolve_exact(plan)?;
    if p.kind != Kind::Plan {
        return Err(format!("“{}” is not a plan", p.title));
    }
    Ok(p)
}

/// Write a plan's new text: its current file to history first.
fn rewrite_plan(root: &Path, p: &Idea, text: &str, md: &fs::Metadata, new_text: &str) -> Result<(), String> {
    if new_text == text {
        return Ok(());
    }
    history_copy_at(root, p.id.as_deref().unwrap_or_default(), text.as_bytes())?;
    crate::fsx::write_atomic(&root.join(&p.file), new_text.as_bytes(), Some(md.permissions())).map_err(|e| format!("{}: {e}", p.file))
}

/// Add `ideas` to the plan `plan`'s `plan_of:` (§6.1: an idea dropped on a
/// plan). Each is recorded by its live id; one the plan already names
/// (directly or through an absorbed id) is skipped. Nothing to add writes
/// nothing. The plan goes to history first.
pub fn plan_add_ideas_at(root: &Path, plan: &str, ideas: &[String], expected_mtime: Option<u64>) -> Result<Doc, String> {
    refuse_real(root)?;
    let idx = list_at(root)?;
    let p = plan_doc(&idx, plan)?.clone();
    let (text, md) = load(root, &p)?;
    check_mtime(&md, expected_mtime)?;
    let add = live_ideas(&idx, ideas, Some(plan))?;
    let mut list = front_list(&text, PLAN_OF);
    let named: Vec<String> = list.iter().filter_map(|x| idx.resolve(x).ok().and_then(|d| d.id.clone())).collect();
    for (id, _) in add {
        if !named.contains(&id) && !list.contains(&id) {
            list.push(id);
        }
    }
    let new_text = with_plan_of(&text, &list).ok_or("the plan has no frontmatter")?;
    rewrite_plan(root, &p, &text, &md, &new_text)?;
    read_at(root, plan)
}

/// Take `idea` out of the plan `plan`'s `plan_of:`: every entry that is
/// `idea` or resolves to the same live idea. The last one may go; the key
/// stays (`plan_of: []`), so the document is still a plan. The plan goes to
/// history first. An id the plan does not name writes nothing.
pub fn plan_remove_idea_at(root: &Path, plan: &str, idea: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    refuse_real(root)?;
    let idx = list_at(root)?;
    let p = plan_doc(&idx, plan)?.clone();
    let (text, md) = load(root, &p)?;
    check_mtime(&md, expected_mtime)?;
    let live = |x: &str| idx.resolve(x).ok().and_then(|d| d.id.clone());
    let gone = live(idea);
    let mut list = front_list(&text, PLAN_OF);
    list.retain(|x| x != idea && (gone.is_none() || live(x) != gone));
    let new_text = with_plan_of(&text, &list).ok_or("the plan has no frontmatter")?;
    rewrite_plan(root, &p, &text, &md, &new_text)?;
    read_at(root, plan)
}

/// Add `- <name> (registered)` to the plan's `## Projects` (§6.1: a project
/// dropped on a plan): after the section's last line, or as a new section
/// before `## Work` (else at the end) when the plan has none. A name the
/// section already lists writes nothing. The plan goes to history first.
pub fn plan_add_project_at(root: &Path, plan: &str, name: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    refuse_real(root)?;
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) || name.starts_with('(') {
        return Err(format!("not a project name: {name:?}"));
    }
    let idx = list_at(root)?;
    let p = plan_doc(&idx, plan)?.clone();
    let (text, md) = load(root, &p)?;
    check_mtime(&md, expected_mtime)?;
    let read = crate::plans::read(&text);
    if read.projects.as_ref().is_some_and(|ps| ps.iter().any(|x| x.name == name)) {
        return read_at(root, plan);
    }
    let sep = sep_of(&text);
    let line = format!("- {name} (registered){sep}");
    let new_text = match crate::plans::section_span(&text, "projects") {
        Some((_, body, end)) => {
            // After the last line with words: the blank lines before the
            // next heading stay below the new line.
            let kept = text[body..end].trim_end_matches(['\n', '\r']);
            let at = if kept.trim().is_empty() { body } else { body + kept.len() + sep.len() };
            let at = at.min(text.len());
            let lead = if at == text.len() && !text.ends_with('\n') { sep } else { "" };
            format!("{}{lead}{line}{}", &text[..at], &text[at..])
        }
        None => {
            let block = format!("## Projects{sep}{sep}{line}{sep}");
            match crate::plans::section_span(&text, "work") {
                Some((h, _, _)) => format!("{}{block}{}", &text[..h], &text[h..]),
                None => {
                    let kept = text.trim_end();
                    format!("{kept}{sep}{sep}{block}")
                }
            }
        }
    };
    rewrite_plan(root, &p, &text, &md, &new_text)?;
    read_at(root, plan)
}

/// One idea a plan names, as the plan strip shows it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct PlanIdea {
    /// The id as the plan writes it.
    pub id: String,
    /// The live idea it resolves to, or `None` when it resolves nowhere (a
    /// removed idea): listed, never dropped for the user.
    pub live: Option<String>,
    pub title: Option<String>,
    /// It resolved through an absorbed id (merged into `live`).
    pub merged: bool,
}

/// A plan read whole: the document, its ideas resolved, and its two
/// sections as [`crate::plans::read`] answers them.
#[derive(Serialize, Debug)]
pub struct Plan {
    pub doc: Doc,
    pub ideas: Vec<PlanIdea>,
    #[serde(flatten)]
    pub sections: crate::plans::Sections,
}

pub fn plan_read_at(root: &Path, plan: &str) -> Result<Plan, String> {
    let idx = list_at(root)?;
    plan_doc(&idx, idx.resolve(plan)?.id.as_deref().unwrap_or_default())?;
    let doc = read_at(root, plan)?;
    let ideas = front_list(&doc.content, PLAN_OF)
        .into_iter()
        .map(|id| match idx.lookup(&id) {
            Ok((d, via)) => PlanIdea { live: d.id.clone(), title: Some(d.title.clone()), merged: via, id },
            Err(_) => PlanIdea { live: None, title: None, merged: false, id },
        })
        .collect();
    let sections = crate::plans::read(&doc.content);
    Ok(Plan { doc, ideas, sections })
}

/// The plans whose `plan_of:` names `id` (or an id it absorbed), newest first.
pub fn plans_of_at(root: &Path, id: &str) -> Result<Vec<Idea>, String> {
    let idx = list_at(root)?;
    let me = idx.resolve(id)?.id.clone().unwrap_or_default();
    Ok(idx
        .ideas
        .iter()
        .filter(|d| d.kind == Kind::Plan)
        .filter(|d| {
            d.id.as_deref().and_then(|p| idx.rels(p)).is_some_and(|r| {
                r.plan_of.iter().any(|x| idx.resolve(x).ok().and_then(|d| d.id.as_deref()) == Some(me.as_str()))
            })
        })
        .cloned()
        .collect())
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

pub fn relate(id: &str, other: &str) -> Result<Doc, String> {
    relate_at(&dir(), id, other)
}
pub fn related(id: &str) -> Result<Vec<Relation>, String> {
    related_at(&dir(), id)
}
pub fn versions(id: &str) -> Result<Vec<Version>, String> {
    versions_at(&dir(), id)
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
    // ── phase 3: ideas relate ───────────────────────────────────────────────

    /// A new idea whose text is exactly `body` under its heading.
    fn made(root: &Path, title: &str, body: &str) -> Doc {
        let d = create_at(root, title).unwrap();
        let id = d.idea.id.clone().unwrap();
        let text = format!("{}\n{body}", d.content);
        write_at(root, &id, &text, None).unwrap();
        read_at(root, &id).unwrap()
    }

    /// Every history copy of `id`, oldest first, as text.
    fn copies(root: &Path, id: &str) -> Vec<String> {
        let mut v = versions_at(root, id).unwrap();
        v.reverse();
        v.iter().map(|x| version_text_at(root, id, &x.version).unwrap()).collect()
    }

    fn id(d: &Doc) -> String {
        d.idea.id.clone().unwrap()
    }

    #[test]
    fn a_frontmatter_list_reads_every_form_and_a_write_touches_only_its_key() {
        let t = "---\nid: X\nrelated: [A, \"B\" ,A]\nabsorbed:\n  - C\n  - 'D'\nx-mine: 1\n---\n\n# T\n";
        assert_eq!(front_list(t, "related"), vec!["A", "B"]);
        assert_eq!(front_list(t, "absorbed"), vec!["C", "D"]);
        assert_eq!(front_list(t, "from"), Vec::<String>::new());
        let w = with_front_list(t, "absorbed", &["C".into(), "D".into(), "E".into()]).unwrap();
        assert_eq!(w, "---\nid: X\nrelated: [A, \"B\" ,A]\nabsorbed: [C, D, E]\nx-mine: 1\n---\n\n# T\n");
        let w = with_front_list(&w, "plan_of", &["P".into()]).unwrap();
        assert!(w.ends_with("x-mine: 1\nplan_of: [P]\n---\n\n# T\n"), "{w}");
        let crlf = "---\r\nid: X\r\n---\r\n\r\n# T\r\n";
        assert_eq!(with_front_list(crlf, "related", &["A".into()]).unwrap(), "---\r\nid: X\r\nrelated: [A]\r\n---\r\n\r\n# T\r\n");
        assert_eq!(utf16_to_byte("a😀b", 3), Some(5), "an astral character is two UTF-16 units");
        assert_eq!(utf16_to_byte("a😀b", 2), None, "inside a surrogate pair");
        assert_eq!(utf16_to_byte("a😀b", 4), Some(6));
    }

    #[test]
    fn relate_writes_one_side_only_and_the_other_side_is_computed() {
        let t = Tmp::new("relate");
        let root = t.root();
        let a = made(&root, "Alpha", "a text\n");
        let b = made(&root, "Beta", "b text\n");
        let (ai, bi) = (id(&a), id(&b));
        let r = relate_at(&root, &ai, &bi).unwrap();
        assert_eq!(front_list(&r.content, "related"), vec![bi.clone()]);
        assert_eq!(r.content.replace(&format!("related: [{bi}]\n"), ""), a.content, "only the line was added");
        assert_eq!(read_at(&root, &bi).unwrap().content, b.content, "the other side is never written");
        assert_eq!(copies(&root, &ai), vec![a.content.clone()], "the history copy is the original, byte for byte");
        let rel = related_at(&root, &bi).unwrap();
        assert_eq!((rel[0].id.as_str(), rel[0].how), (ai.as_str(), "related-by"), "{rel:?}");
        assert_eq!(related_at(&root, &ai).unwrap()[0].how, "related");
        // Already related, either way round: nothing written, no copy.
        relate_at(&root, &ai, &bi).unwrap();
        relate_at(&root, &bi, &ai).unwrap();
        assert_eq!(copies(&root, &ai).len(), 1);
        assert!(copies(&root, &bi).is_empty());
        assert!(relate_at(&root, &ai, &ai).unwrap_err().contains("itself"));
    }

    #[test]
    fn a_merge_appends_under_a_heading_absorbs_the_id_and_the_old_id_resolves_to_the_absorber() {
        let t = Tmp::new("merge");
        let root = t.root();
        let ai = crate::ulid::new();
        raw_doc(&root, "jellyfin.md", &rich(&ai));
        let a = read_at(&root, &ai).unwrap();
        let b = made(&root, "Beta plan", "first para\n\nsecond para\n");
        let c = made(&root, "Gamma", "g\n");
        let (bi, ci) = (id(&b), id(&c));
        relate_at(&root, &bi, &ci).unwrap();
        let b = read_at(&root, &bi).unwrap();
        let m = merge_at(&root, &ai, &bi, None, Some(a.idea.mtime_ms), Some(b.idea.mtime_ms)).unwrap();
        // History first: both originals, byte for byte.
        assert_eq!(copies(&root, &ai), vec![a.content.clone()]);
        assert_eq!(copies(&root, &bi).last(), Some(&b.content), "(the relate copied B before)");
        // §3's table: B's id in A's `absorbed`, B's related joins A's.
        assert_eq!(front_list(&m.content, "absorbed"), vec![bi.clone()]);
        assert_eq!(front_list(&m.content, "related"), vec!["01JC8Z9P0000000000000000AA".to_string(), ci.clone()]);
        let section = format!("## From: Beta plan ({})\n\nfirst para\n\nsecond para\n", today());
        assert!(m.content.ends_with(&section), "{}", m.content);
        assert!(m.content.contains("x-mine: \"keep  me\"") && m.content.contains("<!-- ¶ 7k2q -->"), "every other byte survives");
        assert!(!root.join(&b.idea.file).exists(), "B's file leaves the list");
        // B keeps resolving, to A, with the note.
        let via = read_at(&root, &bi).unwrap();
        assert_eq!(via.idea.id.as_deref(), Some(ai.as_str()));
        assert_eq!(via.merged, Some(Merged { id: bi.clone(), title: Some("Beta plan".into()), date: Some(today()) }));
        assert!(read_at(&root, &ai).unwrap().merged.is_none());
        // An act never reaches A through B's id.
        for e in [
            write_at(&root, &bi, &m.content, None).unwrap_err(),
            remove_at(&root, &bi, None).unwrap_err(),
            rename_at(&root, &bi, "x", None).unwrap_err(),
        ] {
            assert!(e.contains("merged into “Jellyfin on the home server”"), "{e}");
        }
        assert!(root.join(&m.idea.file).exists(), "A is untouched by those");
        // A merge of A into D carries B along: B now resolves to D.
        let d = made(&root, "Delta", "d\n");
        let di = id(&d);
        let m2 = merge_at(&root, &di, &ai, Some("\n# Delta, rewritten\n\nOne text.\n"), None, None).unwrap();
        assert_eq!(front_list(&m2.content, "absorbed"), vec![ai.clone(), bi.clone()]);
        assert!(m2.content.ends_with("---\n\n# Delta, rewritten\n\nOne text.\n"), "the given body, the core's frontmatter: {}", m2.content);
        assert_eq!(m2.idea.file, "delta-rewritten.md", "the first heading names the file");
        assert_eq!(read_at(&root, &bi).unwrap().idea.id.as_deref(), Some(di.as_str()));
        assert!(merge_at(&root, &di, &ci, Some("---\nid: x\n---\n"), None, None).unwrap_err().contains("without its frontmatter"));
        assert!(merge_at(&root, &di, &di, None, None, None).unwrap_err().contains("itself"));
        assert!(merge_at(&root, &di, &ci, None, Some(1), None).unwrap_err().contains("changed on disk"));
    }

    #[test]
    fn combine_is_a_merge_into_a_new_idea() {
        let t = Tmp::new("combine");
        let root = t.root();
        let a = made(&root, "Alpha", "aa\n");
        let b = made(&root, "Beta", "bb\n");
        let (ai, bi) = (id(&a), id(&b));
        assert!(combine_at(&root, std::slice::from_ref(&ai), "x", None).unwrap_err().contains("two ideas"));
        assert!(combine_at(&root, &[ai.clone(), ai.clone()], "x", None).unwrap_err().contains("twice"));
        let c = combine_at(&root, &[ai.clone(), bi.clone()], "Both", None).unwrap();
        let ci = id(&c);
        assert_eq!(front_list(&c.content, "absorbed"), vec![ai.clone(), bi.clone()]);
        let date = today();
        assert!(c.content.ends_with(&format!("# Both\n\n## From: Alpha ({date})\n\naa\n\n## From: Beta ({date})\n\nbb\n")), "{}", c.content);
        assert_eq!(c.idea.file, "both.md");
        assert_eq!(copies(&root, &ai), vec![a.content.clone()]);
        assert_eq!(copies(&root, &bi), vec![b.content.clone()]);
        assert_eq!(list_at(&root).unwrap().ideas.len(), 1, "both sources leave the list");
        for x in [&ai, &bi] {
            let r = read_at(&root, x).unwrap();
            assert_eq!((r.idea.id.as_deref(), r.merged.as_ref().map(|m| m.id.as_str())), (Some(ci.as_str()), Some(x.as_str())));
        }
    }

    #[test]
    fn a_split_moves_the_text_to_a_new_idea_and_leaves_a_link_that_round_trips() {
        let t = Tmp::new("split");
        let root = t.root();
        let a = made(&root, "Home 😀 server", "Intro para.\n\nJellyfin sharing\nwith friends.\n\nOutro.\n");
        let ai = id(&a);
        let s = a.content.find("Jellyfin").unwrap();
        let e = a.content.find("Outro").unwrap();
        let u16 = |b: usize| a.content[..b].encode_utf16().count();
        // A selection reaching into the frontmatter, and a stale one, are refused.
        assert!(split_at(&root, &ai, 0, u16(s), false, None, None).unwrap_err().contains("frontmatter"));
        assert!(split_at(&root, &ai, u16(s), u16(e), false, None, Some(1)).unwrap_err().contains("changed on disk"));
        let out = split_at(&root, &ai, u16(s), u16(e), false, None, Some(a.idea.mtime_ms)).unwrap();
        let ci = id(&out.new);
        assert_eq!(out.new.idea.title, "Jellyfin sharing");
        assert_eq!(front_value(&out.new.content, "from"), Some(ai.as_str()));
        assert!(out.new.content.ends_with("---\n\n# Jellyfin sharing\n\nJellyfin sharing\nwith friends.\n"), "{}", out.new.content);
        let want = a.content.replace("Jellyfin sharing\nwith friends.\n\n", &format!("[Jellyfin sharing](idea:{ci})\n\n"));
        assert_eq!(out.source.content, want, "the selection is replaced by a link, blank lines kept");
        assert_eq!(links_in(&out.source.content), vec![ci.clone()]);
        assert_eq!(copies(&root, &ai), vec![a.content.clone()], "A's previous version is in history");
        let hows = |x: &str| related_at(&root, x).unwrap().into_iter().map(|r| r.how).collect::<Vec<_>>();
        assert_eq!(hows(&ai), vec!["links-to", "split-into"]);
        assert_eq!(hows(&ci), vec!["split-from", "linked-from"]);
        // And back, by restoring A's copy: the original bytes, and the version
        // it replaced kept in history in turn.
        let v = versions_at(&root, &ai).unwrap();
        let back = restore_at(&root, &ai, &v[0].version, Some(out.source.idea.mtime_ms)).unwrap();
        assert_eq!(back.content, a.content);
        assert_eq!(copies(&root, &ai), vec![a.content.clone(), out.source.content.clone()]);
        // Extract keeps the source as it is and copies nothing.
        let ex = split_at(&root, &ai, u16(s), u16(e), true, Some("Copy"), Some(back.idea.mtime_ms)).unwrap();
        assert_eq!(ex.source.content, a.content);
        assert_eq!(copies(&root, &ai).len(), 2);
        assert_eq!(ex.new.idea.title, "Copy");
    }

    #[test]
    fn restore_puts_a_copy_back_and_makes_a_removed_or_merged_idea_live_again() {
        let t = Tmp::new("restore");
        let root = t.root();
        let a = made(&root, "Alpha", "one\n");
        let b = made(&root, "Beta", "two\n");
        let (ai, bi) = (id(&a), id(&b));
        merge_at(&root, &ai, &bi, None, None, None).unwrap();
        let vb = versions_at(&root, &bi).unwrap();
        assert_eq!(vb.len(), 1);
        assert_eq!(vb[0].title.as_deref(), Some("Beta"));
        assert_eq!(&vb[0].at[..10], today());
        // B comes back under its own id, and answers for it again.
        let r = restore_at(&root, &bi, &vb[0].version, None).unwrap();
        assert_eq!((r.content.as_str(), r.merged.is_none()), (b.content.as_str(), true));
        assert!(root.join("beta.md").exists());
        assert!(restore_at(&root, &bi, "../x.md", None).unwrap_err().contains("not a version"));
        // A copy taken before an adoption has no id: refused.
        raw_doc(&root, "hand.md", "# Hand\n");
        let h = adopt_at(&root, "hand.md", None).unwrap();
        let vh = versions_at(&root, &id(&h)).unwrap();
        assert!(restore_at(&root, &id(&h), &vh[0].version, None).unwrap_err().contains("before the idea had its id"));
        assert!(versions_at(&root, &crate::ulid::new()).unwrap().is_empty());
    }

    #[test]
    fn make_plan_fills_the_template_with_the_ideas_titles_and_links() {
        let t = Tmp::new("plan-make");
        let root = t.root();
        let a = made(&root, "Jellyfin [sharing]", "a\n");
        let b = made(&root, "Home server GPU", "b\n");
        let (ai, bi) = (id(&a), id(&b));
        let (a_before, b_before) = (a.content.clone(), b.content.clone());
        let p = make_plan_at(&root, &[ai.clone(), bi.clone(), ai.clone()], Some("Jellyfin at home"), None).unwrap();
        let pi = id(&p);
        assert_eq!(p.idea.kind, Kind::Plan);
        assert_eq!(p.idea.file, "plan-jellyfin-at-home.md", "named by its heading, as every document");
        assert_eq!(front_list(&p.content, PLAN_OF), vec![ai.clone(), bi.clone()], "each idea once, in order");
        let body = format!(
            "# Plan: Jellyfin at home\n\nFrom [Jellyfin \\[sharing\\]](idea:{ai}) and [Home server GPU](idea:{bi}).\n\n## Goal\n\n## Projects\n\n## Work\n"
        );
        assert_eq!(p.content, format!("---\nid: {pi}\nplan_of: [{ai}, {bi}]\ncreated: {}\n---\n\n{body}", today()));
        assert_eq!(read_at(&root, &ai).unwrap().content, a_before, "no idea is written");
        assert_eq!(read_at(&root, &bi).unwrap().content, b_before);
        // The list says which is which, and an idea's related list says "planned in".
        let l = list_at(&root).unwrap();
        assert_eq!(l.ideas.iter().filter(|d| d.kind == Kind::Plan).count(), 1);
        let rel = related_at(&root, &ai).unwrap();
        assert_eq!(rel.iter().map(|r| (r.id.as_str(), r.how)).collect::<Vec<_>>(), vec![(pi.as_str(), "planned-in")], "said once, not again as a link");
        assert_eq!(related_at(&root, &pi).unwrap().iter().map(|r| r.how).collect::<Vec<_>>(), vec!["plan-of", "plan-of"]);
        assert_eq!(plans_of_at(&root, &bi).unwrap().iter().map(|d| d.id.clone().unwrap()).collect::<Vec<_>>(), vec![pi.clone()]);
        // One idea, no title: the idea's own.
        let solo = make_plan_at(&root, std::slice::from_ref(&bi), None, None).unwrap();
        assert!(solo.content.contains("\n# Plan: Home server GPU\n\nFrom [Home server GPU](idea:"), "{}", solo.content);
        // Refused: no idea, a plan as an idea.
        assert!(make_plan_at(&root, &[], None, None).unwrap_err().contains("one idea or more"));
        assert!(make_plan_at(&root, &[pi.clone()], None, None).unwrap_err().contains("is a plan"));
    }

    #[test]
    fn a_draft_is_the_body_and_one_with_its_own_frontmatter_is_refused() {
        let t = Tmp::new("plan-draft");
        let root = t.root();
        let a = id(&made(&root, "Alpha", "a\n"));
        let e = make_plan_at(&root, std::slice::from_ref(&a), None, Some("---\nid: 01JC90A1000000000000000000\nplan_of: []\n---\n# Plan: x\n")).unwrap_err();
        assert!(e.contains("without its frontmatter"), "{e}");
        assert_eq!(list_at(&root).unwrap().ideas.len(), 1, "nothing was made");
        let d = make_plan_at(&root, std::slice::from_ref(&a), Some("ignored for the body"), Some("# Plan: Drafted\n\n## Work\n### One\nproject: p\nbranch: b\n")).unwrap();
        assert!(d.content.ends_with("---\n\n# Plan: Drafted\n\n## Work\n### One\nproject: p\nbranch: b\n"), "{}", d.content);
        assert_eq!(d.idea.file, "plan-drafted.md");
        assert_eq!(d.idea.kind, Kind::Plan);
    }

    #[test]
    fn plan_of_edits_copy_the_plan_to_history_first_and_check_the_mtime() {
        let t = Tmp::new("plan-edit");
        let root = t.root();
        let (a, b, c) = (id(&made(&root, "A", "a\n")), id(&made(&root, "B", "b\n")), id(&made(&root, "C", "c\n")));
        let p0 = make_plan_at(&root, std::slice::from_ref(&a), Some("P"), None).unwrap();
        let pi = id(&p0);
        assert!(plan_add_ideas_at(&root, &pi, std::slice::from_ref(&b), Some(1)).unwrap_err().contains("changed on disk"));
        assert!(copies(&root, &pi).is_empty(), "a refused edit copies nothing");
        let p1 = plan_add_ideas_at(&root, &pi, &[b.clone(), a.clone()], Some(p0.idea.mtime_ms)).unwrap();
        assert_eq!(front_list(&p1.content, PLAN_OF), vec![a.clone(), b.clone()]);
        assert_eq!(copies(&root, &pi), vec![p0.content.clone()], "the plan as it was, first");
        assert_eq!(p1.content.replace(&format!("plan_of: [{a}, {b}]"), &format!("plan_of: [{a}]")), p0.content, "only the key moved");
        // Adding what it holds writes nothing and copies nothing.
        plan_add_ideas_at(&root, &pi, std::slice::from_ref(&a), None).unwrap();
        assert_eq!(copies(&root, &pi).len(), 1);
        assert!(plan_add_ideas_at(&root, &pi, std::slice::from_ref(&pi), None).unwrap_err().contains("is a plan"));
        assert!(plan_add_ideas_at(&root, &a, std::slice::from_ref(&c), None).unwrap_err().contains("is not a plan"));
        // Remove each; the last leaves `plan_of: []`, still a plan.
        let p2 = plan_remove_idea_at(&root, &pi, &a, Some(p1.idea.mtime_ms)).unwrap();
        assert_eq!(front_list(&p2.content, PLAN_OF), vec![b.clone()]);
        let p3 = plan_remove_idea_at(&root, &pi, &b, None).unwrap();
        assert!(p3.content.contains("\nplan_of: []\n"), "{}", p3.content);
        assert_eq!(p3.idea.kind, Kind::Plan);
        assert_eq!(copies(&root, &pi), vec![p0.content.clone(), p1.content.clone(), p2.content.clone()]);
        plan_remove_idea_at(&root, &pi, &c, None).unwrap();
        assert_eq!(copies(&root, &pi).len(), 3, "an id it does not name writes nothing");
        // A project, as registered, in its section; twice is once.
        let p4 = plan_add_project_at(&root, &pi, "worktrees", None).unwrap();
        assert!(p4.content.contains("## Projects\n- worktrees (registered)\n\n## Work\n"), "{}", p4.content);
        let p5 = plan_add_project_at(&root, &pi, "homelab", None).unwrap();
        assert!(p5.content.contains("## Projects\n- worktrees (registered)\n- homelab (registered)\n\n## Work\n"), "{}", p5.content);
        plan_add_project_at(&root, &pi, "homelab", None).unwrap();
        assert_eq!(copies(&root, &pi).len(), 5);
        assert_eq!(crate::plans::read(&p5.content).problems, vec![]);
    }

    #[test]
    fn a_plan_naming_a_merged_or_renamed_idea_keeps_resolving() {
        let t = Tmp::new("plan-absorbed");
        let root = t.root();
        let (a, b) = (id(&made(&root, "Jellyfin", "a\n")), id(&made(&root, "Sharing", "b\n")));
        let p = make_plan_at(&root, &[a.clone(), b.clone()], None, None).unwrap();
        let pi = id(&p);
        rename_at(&root, &a, "Jellyfin at home", None).unwrap();
        merge_at(&root, &a, &b, None, None, None).unwrap();
        assert_eq!(read_at(&root, &pi).unwrap().content, p.content, "the plan is never rewritten for it");
        let r = plan_read_at(&root, &pi).unwrap();
        assert_eq!(
            r.ideas,
            vec![
                PlanIdea { id: a.clone(), live: Some(a.clone()), title: Some("Jellyfin at home".into()), merged: false },
                PlanIdea { id: b.clone(), live: Some(a.clone()), title: Some("Jellyfin at home".into()), merged: true },
            ]
        );
        assert_eq!(plans_of_at(&root, &b).unwrap().len(), 1, "B's id finds the plan through A");
        // Adding A again is a no-op: B's entry already names it.
        plan_add_ideas_at(&root, &pi, std::slice::from_ref(&a), None).unwrap();
        assert!(copies(&root, &pi).is_empty());
        // Removing by B's id takes both entries (they are one idea now).
        let gone = plan_remove_idea_at(&root, &pi, &b, None).unwrap();
        assert!(gone.content.contains("\nplan_of: []\n"), "{}", gone.content);
    }

    #[test]
    fn a_forbidden_process_cannot_write_the_real_ideas_folder() {
        let real = Path::new("/home/u/.local/share/worktrees/ideas");
        let e = refuse_real_in(real, real, true).unwrap_err();
        assert!(e.contains("real ideas folder") && e.contains(OVERRIDE_ENV), "{e}");
        assert!(refuse_real_in(real, real, false).is_ok());
        assert!(refuse_real_in(Path::new("/tmp/t/ideas"), real, true).is_ok());
    }
}
