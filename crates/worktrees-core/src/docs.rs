//! The per-place documentation index — the walk behind the app's Docs tab.
//!
//! A place is the primary axis, not the repo. valleos has eleven active places
//! and seven of them carry the *pre-restructure* `docs/` tree while four carry
//! the new one; both states are correct for their branch, and a reader that
//! does not say which place it is showing lets a 53-commits-old ADR read as
//! current. So there is no repo-wide index here and there never will be: this
//! function takes one worktree directory and returns what that worktree, at
//! this commit, calls documentation.
//!
//! **Convention, not configuration.** Four of those eleven places have no docs
//! tooling on their branch at all, so anything that depended on the project's
//! own generator would work on some places and not others — which is the
//! inconsistency the tab exists to fix. The order is fixed here:
//!
//! 1. `ROOT_FILES`, in that order, when present;
//! 2. the place's brief (`ops::BRIEF_PATH`), when present;
//! 3. any other root-level `*.md`, alphabetically;
//! 4. `docs/**/*.md`, depth-first, files before subdirectories.
//!
//! `[docs]` (proposal §3.2) changes exactly two of those four. `paths`
//! replaces step 4 — the repo says which of its directories hold documentation
//! — while 1, 2 and 3 stay, because the root files and the brief are about the
//! repo rather than about its documentation layout. `index` prepends one entry
//! ahead of everything, because "where reading starts" is a fact only the repo
//! knows. Nothing else moves, and there is no key that can name a command:
//! ADR 0001 is why `Docs` is two path fields (see `projcfg::Docs`).
//!
//! 3 sits before 4 so the ungrouped run is CONTIGUOUS — one block of
//! root-level documents, then the tree. It read the other way round first, and
//! that emitted two separate `group: ""` runs with the whole `docs/` tree
//! between them; the dock rendered the second block under no header, below
//! `docs/archive/`, and React saw two sibling groups with the same key. It is
//! also better for the case that actually dominates: seven of those eleven
//! places carry a FLAT tree, where every document is root-level markdown and
//! `docs/` does not exist at all.
//!
//! Ordering, grouping and titles are decided HERE and rendered verbatim by the
//! frontend. That is deliberate: a frontend that re-derived any of it would be
//! a mirror of a core rule, and this repo has learned twice what a silent
//! mirror costs (`dnd.ts::predictTier`, the new-worktree verdict line). There
//! is nothing for `docs-check.mjs` to guard because there is nothing mirrored.
//!
//! **What is refused, and why it is refused here rather than at read time.**
//! `SKIP_DIRS` is not a performance rule. On `(main)`, `.worktrees/` *contains
//! every other place*, so walking it would file ten places' documentation under
//! one — the exact confusion the tab exists to remove. And every entry is
//! `symlink_metadata`'d, never `metadata`'d: a committed
//! `docs/secrets.md -> ~/.ssh/id_rsa` must list as nothing at all. The caller
//! has already canonicalised `root` and proved it lies under a registered
//! project (`guard_under_projects`); this walk never leaves it, because it
//! never follows the one thing that could.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::projcfg::Docs;

/// Root-level documents, in the order a reader wants them. Present ones only.
pub const ROOT_FILES: [&str; 5] = ["README.md", "CLAUDE.md", "DESIGN.md", "ROADMAP.md", "CHANGELOG.md"];

/// The documentation tree, by convention.
pub const DOCS_DIR: &str = "docs";

/// Never descended. `.git` and `.worktrees` are correctness (see the module
/// note); the other three are noise that would blow the entry cap on any repo
/// with dependencies installed — `node_modules` alone carries thousands of
/// `README.md`s that document somebody else's code.
pub const SKIP_DIRS: [&str; 5] = [".git", ".worktrees", "node_modules", "target", "dist"];

/// How deep under `docs/` the walk goes. Past this a tree is generated, not
/// written, and the index is not a file browser — the Files tab still is.
pub const MAX_DEPTH: usize = 12;

/// Hard cap on rows. Reached, the index reports `truncated` rather than
/// pretending: an index that silently stops is the same class of lie as a
/// reader that does not say how stale it is.
pub const MAX_ENTRIES: usize = 2_000;

/// How much of a file is read to find its title. A `title:` key or an H1 past
/// 8 KiB is not a title anyone wrote to be seen first, and 424 files (valleos,
/// across every place) × a full read is not a thing to do on a poll tick.
const TITLE_SNIFF: u64 = 8 * 1024;

/// A title longer than this is a paragraph that forgot its `#`.
const TITLE_MAX: usize = 200;

/// One document, as the index renders it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DocEntry {
    /// Absolute path — what `read_file` and `revealItemInDir` are handed.
    pub path: String,
    /// Path relative to the place root. This is the row's second line, and it
    /// is what makes the brief self-describing (`.planning/brief.md`) without
    /// the walk having to invent an English label for it.
    pub rel: String,
    /// Frontmatter `title:` → first H1 → the filename stem.
    pub title: String,
    /// The containing directory, relative to the place root, or `""` for the
    /// root files and the brief. The frontend renders one header per distinct
    /// group in first-appearance order; `""` gets no header.
    ///
    /// The brief is deliberately `""` rather than `.planning`: it is about this
    /// PLACE, which is the one thing the root files are not, and a group of one
    /// under a gitignored directory name reads as an accident.
    pub group: String,
    /// Last modification, in milliseconds since the epoch; `0` when the stat
    /// failed. The recency mark in the Docs tab is this against the place's
    /// seen epoch, and it is mtime rather than git status for one reason:
    /// **the documents this signal exists for are gitignored.** `task_plan.md`,
    /// `findings.md`, `progress.md` and the whole of `.planning/` (this repo's
    /// own `.gitignore`, and `ops::BRIEF_PATH`'s docstring says every repo of
    /// ours does the same) never appear in `git status`, so a git-derived mark
    /// would light up `CHANGELOG.md` forever and the brief never — backwards
    /// from what a reader wants. It costs nothing here: the walk has already
    /// `symlink_metadata`'d this path to prove it a regular file and then
    /// OPENED it to sniff a title.
    ///
    /// Milliseconds because `lib.rs::file_mtime_ms` already speaks them, and a
    /// second's resolution is genuinely too coarse for "did this change while I
    /// was reading it".
    pub mtime_ms: u64,
}

/// The index for one place.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DocsIndex {
    pub entries: Vec<DocEntry>,
    /// `MAX_ENTRIES` was hit and the walk stopped. Said out loud in the UI.
    pub truncated: bool,
}

/// Is this a markdown file we would list? Extension only — the walk already
/// knows it is a regular, non-symlinked file.
fn is_md(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// A regular file, following NOTHING. `symlink_metadata` is the whole point:
/// `metadata` would stat through a link and happily report `~/.ssh/id_rsa` as a
/// perfectly ordinary file.
fn is_regular_file(p: &Path) -> bool {
    std::fs::symlink_metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

/// Modification time in milliseconds since the epoch, `0` when it cannot be
/// read. `symlink_metadata` for the same reason as above — every caller has
/// already proved this is a regular file, and a stat that follows links has no
/// business in this module even where it would agree.
///
/// A failure is `0`, never an error: a row whose mtime could not be read is
/// still a row. `0` reads as "older than any baseline", so the mark is simply
/// absent — which is the right way for this to fail.
fn mtime_ms(p: &Path) -> u64 {
    stat_ms_len(p).0
}

/// Modification time in milliseconds and byte length, from ONE stat.
///
/// Two callers want different halves of the same `symlink_metadata` — the index
/// wants the mtime for its recency mark, the fingerprint wants both — and a
/// second stat per file is the one cost this module is trying not to pay
/// (`fingerprint_with`'s note). `(0, 0)` when it cannot be read, for the same
/// reason `mtime_ms` answers `0`: a row whose stat failed is still a row.
fn stat_ms_len(p: &Path) -> (u64, u64) {
    let Ok(md) = std::fs::symlink_metadata(p) else { return (0, 0) };
    let ms = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    (ms, md.len())
}

/// Collapse whitespace, cap the length, and refuse an empty result.
fn clean_title(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut space = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
        if out.chars().count() >= TITLE_MAX {
            break;
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Strip one layer of matching quotes, the way every frontmatter dialect writes
/// a title with a colon in it.
fn unquote(s: &str) -> &str {
    let t = s.trim();
    for q in ['"', '\''] {
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            return &t[1..t.len() - 1];
        }
    }
    t
}

/// Split a leading YAML frontmatter block off a document: `Some((block, rest))`
/// where `block` is what sits between the fences and `rest` is everything after
/// the closing one. `None` when the file does not open with `---`, and — the
/// part that matters — when that block is never CLOSED.
///
/// ONE parser, because two readers of the same block drift. `title_from` needs
/// the block to find `title:` and the body to find the H1; `derive::body`
/// needs the body to throw the block away (a viewer renders frontmatter as
/// noise — `mo` makes an expanded `<details>` "Metadata" block on every page).
/// Those are the same question asked twice, and the second asker is the one
/// that DELETES what it decides is frontmatter.
///
/// Which is why an unterminated `---` is not frontmatter here. `title_from`
/// used to consume the rest of the file looking for a fence that never came,
/// and that cost it nothing — no `# ` line was reachable, so it returned `None`
/// and the row fell back to the filename. The same reading in the derive would
/// silently delete the whole document. A lone `---` on line 1 is a thematic
/// break; treating it as an open block is a guess, and the guess is only free
/// for the reader that cannot lose anything by it.
///
/// **And a TERMINATED one is not frontmatter either, unless the block opens a
/// YAML mapping.** The same thematic break with any later `---` under it — and
/// `---` is a spelling this repo's own prose uses — closed a block that was
/// never opened, so everything between them was metadata and the derive deleted
/// the top of the document. Jekyll and gray-matter read it the same way, which
/// is why this survived: it is ecosystem-consistent and still wrong for the one
/// reader that DELETES. So the block is asked one question, `opens_a_mapping`,
/// and prose fails it.
pub fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let open_end = text.find('\n').map(|i| i + 1).unwrap_or(text.len());
    // `str::lines` strips a trailing `\r`; this is the same test on raw bytes.
    if text[..open_end].trim_end_matches(['\n', '\r']) != "---" {
        return None;
    }
    let mut pos = open_end;
    while pos < text.len() {
        let end = text[pos..].find('\n').map(|i| pos + i + 1).unwrap_or(text.len());
        let line = text[pos..end].trim_end_matches(['\n', '\r']).trim_end();
        if line == "---" || line == "..." {
            let block = &text[open_end..pos];
            return opens_a_mapping(block).then(|| (block, &text[end..]));
        }
        pos = end;
    }
    None
}

/// Does this block read as a YAML MAPPING — the only thing frontmatter ever is?
///
/// One question, of the first non-blank line, because that is where the answer
/// is and a fuller parse would be a YAML reader living in a markdown tool. An
/// EMPTY block passes: `---\n---\n` is a legal, if pointless, frontmatter.
///
/// A `#` line is deliberately NOT skipped as a YAML comment. `# Heading` is the
/// far more likely reading of those bytes in a file this tool only ever meets
/// as markdown, and skipping it would hand a document that opens with a rule
/// and a heading straight back to the deleter — the failure this exists to
/// stop, wearing the fix's own clothes.
fn opens_a_mapping(block: &str) -> bool {
    for line in block.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        return yaml_key(t);
    }
    true
}

/// Does `t` begin `key:` — a bare key ending at the colon, or a quoted one?
///
/// The key may not contain whitespace, which is what separates a mapping from
/// most English: `See the note below: it matters` fails on the space. A key
/// with a space in it is legal YAML and vanishingly rare in frontmatter, and
/// the cost of refusing one is an unstripped block that renders as visible text
/// — not a deleted document, which is the whole asymmetry here.
///
/// What this does NOT separate, said out loud: a paragraph whose first word is
/// followed by a colon (`Note: …`) is a mapping key by every rule any generator
/// applies, and it is read as one here too. That ambiguity is in the bytes. The
/// rule removes the shape that actually bit — a rule, a blank line, and prose —
/// and does not pretend to more.
fn yaml_key(t: &str) -> bool {
    let key_end = match t.chars().next() {
        Some(q @ ('"' | '\'')) => match t[1..].find(q) {
            Some(i) => 1 + i + q.len_utf8(),
            None => return false,
        },
        Some(c) if c.is_ascii_alphanumeric() || c == '_' => match t.find(|c: char| c == ':' || c.is_whitespace()) {
            Some(i) => i,
            None => return false,
        },
        _ => return false,
    };
    t[key_end..].starts_with(':') && t[key_end + 1..].chars().next().map(char::is_whitespace).unwrap_or(true)
}

/// The title a document declares: frontmatter `title:`, else the first H1.
///
/// `title` is the one key Jekyll, VitePress, Docusaurus and MkDocs all agree
/// on; reading anything beyond it is per-generator work this tool should not
/// own. Everything else falls through to the filename, which is why a repo with
/// no frontmatter anywhere still gets a usable index.
///
/// Two refusals that look like fussiness and are not. A `#` inside a fenced
/// block is a shell comment — `README`s that open with an install snippet are
/// the common case, and `# Install with homebrew` is not the document's title.
/// And the H1 scan starts AFTER the frontmatter block, so a `# ` in a YAML
/// comment cannot win either.
pub fn title_from(text: &str) -> Option<String> {
    let (front, body) = match split_frontmatter(text) {
        Some((f, rest)) => (Some(f), rest),
        None => (None, text),
    };
    // ── frontmatter ──
    if let Some(front) = front {
        for line in front.lines() {
            // Top-level key only: an indented `title:` belongs to some nested
            // map, and guessing which one is per-generator work.
            if let Some(v) = line.strip_prefix("title:") {
                if let Some(found) = clean_title(unquote(v)) {
                    return Some(found);
                }
            }
        }
    }
    // ── first H1, outside any fence ──
    let mut fence: Option<String> = None;
    for line in body.lines() {
        let t = line.trim_start();
        if let Some(open) = &fence {
            if t.starts_with(open.as_str()) {
                fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(t.chars().take(3).collect());
            continue;
        }
        if let Some(rest) = t.strip_prefix("# ") {
            if let Some(title) = clean_title(rest.trim_end_matches(['#', ' '])) {
                return Some(title);
            }
        }
    }
    None
}

/// Read the head of `path` and ask it what it is called; the filename stem is
/// the answer when it declines.
fn title_of(path: &Path, name: &str) -> String {
    use std::io::Read;
    let stem = || Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    let Ok(f) = std::fs::File::open(path) else { return stem() };
    let mut buf = Vec::new();
    // A truncated read can split a multi-byte char; `from_utf8_lossy` turns
    // that into one replacement character at the very end, where no title is.
    if f.take(TITLE_SNIFF).read_to_end(&mut buf).is_err() {
        return stem();
    }
    title_from(&String::from_utf8_lossy(&buf)).unwrap_or_else(stem)
}

/// The filesystem's own spelling of `want` among `names`, or `None`.
///
/// **Exact first, case-insensitively only as a fallback**, and the order is the
/// whole point. `is_regular_file(root.join("README.md"))` is TRUE on APFS for a
/// file actually named `Readme.md`, so a stat can never answer this: it hands
/// back a name that is not on disk, and every later comparison — `seen`, the
/// row's `rel`, the path `write_tree` writes — is then made against a spelling
/// the volume merely tolerates. Only a listing knows what the entries are
/// called.
///
/// The fallback is not a case-insensitive lookup wearing a hat. A
/// case-SENSITIVE filesystem may legitimately carry both `README.md` and
/// `Readme.md`; there the exact match wins the named slot and the other file is
/// an ordinary root document, which is what it is. The fallback fires whenever
/// no entry is spelled the way the constant is — which is the case-insensitive
/// volume, but ALSO a case-sensitive one carrying only `Readme.md`, where it
/// promotes that file into the README slot instead of leaving it to the
/// alphabetical sweep. That is the better answer (a reader wants the readme
/// first), but it means this is a promotion rule and not merely a
/// disambiguation, so a test that pins the ORDER of a case-variant root file is
/// pinning this, not the de-duplication. Several inexact matches and no exact one is
/// a shape only a case-sensitive volume can produce, so the smallest is taken
/// for a deterministic answer and the rest fall through to the root sweep.
fn resolve_name(names: &[String], want: &str) -> Option<String> {
    let mut ci: Option<&String> = None;
    for n in names {
        if n == want {
            return Some(n.clone());
        }
        if n.eq_ignore_ascii_case(want) && ci.map(|c| n < c).unwrap_or(true) {
            ci = Some(n);
        }
    }
    ci.cloned()
}

/// `resolve_name`, component by component, for a path the project DECLARED
/// (`[docs] index = "docs/index.md"` against a file named `docs/Index.md`).
/// `None` unless every component resolves and the result is a regular file.
///
/// A listing per component rather than one stat, for the reason `resolve_name`
/// gives — and it is affordable here because there is at most one declared
/// index per place, two or three components deep.
fn resolve_rel(root: &Path, rel: &str) -> Option<String> {
    let mut dir = root.to_path_buf();
    let mut parts: Vec<String> = Vec::new();
    for want in rel.split('/').filter(|c| !c.is_empty()) {
        let names = dir_names(&dir);
        let name = resolve_name(&names, want)?;
        dir = dir.join(&name);
        parts.push(name);
    }
    if parts.is_empty() || !is_regular_file(&dir) {
        return None;
    }
    Some(parts.join("/"))
}

/// Every entry name in `dir`, or an empty list when it cannot be read — the
/// module's "a row that cannot be read simply is not one" applied to a listing.
fn dir_names(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()
}

/// Build one entry. `rel` is the caller's business — it is what the row shows —
/// and so is `group`, which is NOT simply `rel`'s parent: the brief lives in
/// `.planning/` and belongs with the root files anyway (see `DocEntry::group`).
fn entry(root: &Path, rel: &str, group: &str) -> DocEntry {
    let path = root.join(rel);
    let name = Path::new(rel).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| rel.to_string());
    DocEntry {
        title: title_of(&path, &name),
        mtime_ms: mtime_ms(&path),
        path: path.to_string_lossy().to_string(),
        rel: rel.to_string(),
        group: group.to_string(),
    }
}

/// The walk itself, with what to MAKE of each row left to the caller.
///
/// Two callers, and the one thing they may never disagree about is which files
/// a place has, in which order: `index_with` builds a `DocEntry` per row (a
/// stat and an 8 KiB read each), `fingerprint_with` stats and stops. Writing
/// the second walk beside the first would be a mirror of the rule it exists to
/// follow, and this repo has paid for a silent mirror twice
/// (`dnd.ts::predictTier`, the new-worktree verdict line) — the tell being that
/// the mirror's own tests keep passing while it drifts. So the order, the
/// skips, the dedupe and the cap live here once and the visitor decides only
/// what a row costs.
///
/// `visit(root, rel, group)` per row, in listing order; the `bool` is
/// `truncated`.
fn walk<T>(root: &Path, docs: Option<&Docs>, mut visit: impl FnMut(&Path, &str, &str) -> T) -> (Vec<T>, bool) {
    let mut entries: Vec<T> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut truncated = false;

    // ONE dedupe rule for the whole walk, and it lives here rather than at the
    // four call sites. `[docs] index = "docs/index.md"` names a file the tree
    // pass then walks into, so the row came out twice — and a per-site check
    // would have closed that one route and left the next one open. `false`
    // means the CAP was hit, which is the only thing the caller must report; a
    // duplicate is simply not pushed.
    //
    // A nested `fn` rather than a closure: it has to be generic over `T` and
    // take `visit` by `&mut`, and everything it touches is already a parameter.
    fn push<T>(
        entries: &mut Vec<T>,
        seen: &mut BTreeSet<String>,
        root: &Path,
        visit: &mut impl FnMut(&Path, &str, &str) -> T,
        rel: String,
        group: &str,
    ) -> bool {
        if seen.contains(&rel) {
            return true;
        }
        if entries.len() >= MAX_ENTRIES {
            return false;
        }
        entries.push(visit(root, &rel, group));
        seen.insert(rel);
        true
    }

    // ONE listing of the root, shared by the named-file pass and the sweep
    // below it. A stat cannot resolve a named file on a case-insensitive volume
    // (`resolve_name`), and two passes reading the same directory through
    // different eyes is how the same file came out twice.
    let root_names: Vec<String> = dir_names(root)
        .into_iter()
        .filter(|n| is_regular_file(&root.join(n)))
        .collect();

    // 0. `[docs] index` — where reading starts, ahead of everything. Grouped
    //    with the root files however deep it lives: it is the landing page for
    //    the whole place, so a `docs` header above it would file it under a
    //    tree it is introducing.
    if let Some(i) = docs.and_then(|d| d.index.as_ref()) {
        if let Some(rel) = resolve_rel(root, i.as_str()) {
            if !push(&mut entries, &mut seen, root, &mut visit, rel, "") {
                truncated = true;
            }
        }
    }
    // 1. the named root files, in the order a reader wants them — each under
    //    the name the filesystem really gives it, which is also the key `seen`
    //    then carries into step 3.
    for f in ROOT_FILES {
        if let Some(rel) = resolve_name(&root_names, f) {
            if !push(&mut entries, &mut seen, root, &mut visit, rel, "") {
                truncated = true;
            }
        }
    }
    // 2. this place's brief — the one document the tool itself writes
    if is_regular_file(&root.join(crate::ops::BRIEF_PATH))
        && !push(&mut entries, &mut seen, root, &mut visit, crate::ops::BRIEF_PATH.to_string(), "")
    {
        truncated = true;
    }
    // 3. whatever other markdown is at the root — with the named files above,
    //    not stranded below the tree (see the module note)
    let mut rest: Vec<String> = Vec::new();
    for name in &root_names {
        if is_md(name) && !seen.contains(name) {
            rest.push(name.clone());
        }
    }
    rest.sort_by_key(|r| r.to_lowercase());
    for r in rest {
        if !push(&mut entries, &mut seen, root, &mut visit, r, "") {
            truncated = true;
            break;
        }
    }
    // 4. the documentation tree(s). `[docs] paths` replaces the convention's
    //    `docs/`, in DECLARED order — the repo chose that order and sorting it
    //    would be this module second-guessing the one thing it was told.
    let declared: Vec<String> = match docs.map(|d| d.paths.as_slice()) {
        Some(p) if !p.is_empty() => p.iter().map(|r| r.as_str().to_string()).collect(),
        _ => vec![DOCS_DIR.to_string()],
    };
    for rel in declared {
        if truncated {
            break;
        }
        let target = root.join(&rel);
        let Ok(md) = std::fs::symlink_metadata(&target) else { continue }; // declared, absent: skip
        if md.is_file() {
            // A declared FILE is listed on its own, grouped by its directory
            // like any other entry.
            if is_md(&rel) {
                let group = Path::new(&rel).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                if !push(&mut entries, &mut seen, root, &mut visit, rel.clone(), &group) {
                    truncated = true;
                }
            }
            continue;
        }
        if !md.is_dir() {
            continue; // a symlink is neither, which is the treatment it gets
        }
        let mut stack: Vec<(PathBuf, String, usize)> = vec![(target, rel, 0)];
        while let Some((dir, rel, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let (mut files, mut dirs): (Vec<String>, Vec<(PathBuf, String)>) = (Vec::new(), Vec::new());
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') {
                    continue;
                }
                // lstat: a symlink is neither a file nor a directory here, which
                // is exactly the treatment it should get.
                let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
                if md.is_dir() {
                    if depth + 1 <= MAX_DEPTH {
                        dirs.push((e.path(), format!("{rel}/{name}")));
                    }
                } else if md.is_file() && is_md(&name) {
                    files.push(format!("{rel}/{name}"));
                }
            }
            files.sort_by_key(|r| r.to_lowercase());
            for r in files {
                if !push(&mut entries, &mut seen, root, &mut visit, r, &rel) {
                    truncated = true;
                    break;
                }
            }
            if truncated {
                break;
            }
            // Reverse: `stack.pop()` is LIFO, and the groups must come out in
            // alphabetical order or a reader cannot find anything twice.
            dirs.sort_by(|a, b| b.1.to_lowercase().cmp(&a.1.to_lowercase()));
            for (p, r) in dirs {
                stack.push((p, r, depth + 1));
            }
        }
    }
    (entries, truncated)
}

/// The index for the worktree at `root`, by convention alone.
pub fn index(root: &Path) -> DocsIndex {
    index_with(root, None)
}

/// The index for the worktree at `root`, which the caller has already
/// canonicalised and proved is under a registered project. `docs` is the
/// project's `[docs]` section when it has one.
///
/// Never errors: a place with no documentation is an empty index, not a
/// failure, and an unreadable subdirectory drops out rather than taking the
/// listing with it. "Fail per file, loudly" is the dock's rule
/// (`ViewErrorBoundary`); here the unit that can fail is a row, and a row that
/// cannot be read simply is not one.
///
/// **A declared path that does not exist is skipped, never an error.** Four of
/// valleos's eleven places have no `apps/docs` on their branch; a config
/// written on main must not make the index fail on a place that has not
/// rebased, which would be the inconsistency this tab exists to remove wearing
/// a different hat.
pub fn index_with(root: &Path, docs: Option<&Docs>) -> DocsIndex {
    let (entries, truncated) = walk(root, docs, entry);
    DocsIndex { entries, truncated }
}

/// FNV-1a, 64-bit, folded over the bytes the fingerprint is made of.
///
/// Not a cryptographic digest and deliberately not one: this compares the
/// tool's own stat results against the tool's own stat results a few seconds
/// earlier, so there is nobody to choose colliding inputs and nothing to forge.
/// `worktrees-core` carries serde and `toml` and nothing else — a dependency
/// here would be a new supply-chain entry to detect that a file changed.
pub(crate) fn fnv1a(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// A stat-only digest of everything `index_with` would list: the rows, in
/// order, each with its size and modification time. `fingerprint(a) !=
/// fingerprint(b)` means the place's documentation changed between them.
///
/// **This exists because `index_with` is too expensive to call on a timer.**
/// The app re-derives a registered place's browser copy on its own tick, and
/// the index reads the head of every file to find a title — `TITLE_SNIFF` is
/// 8 KiB, so a 400-document place is megabytes of reads every few seconds, per
/// registered place, forever. The fingerprint touches the same paths with
/// `symlink_metadata` and nothing else, and the full walk runs only when it has
/// moved.
///
/// It covers more than mtimes, on purpose: the ROW SET and its ORDER are in the
/// digest too, so a file added, removed, renamed, or a `[docs] paths` list read
/// in a different order all move it even where no surviving file was touched.
/// A directory's own mtime is never consulted — it is not a reliable signal for
/// a change two levels down, and the walk is already enumerating.
///
/// **What it can miss, said out loud:** a write that lands in the same
/// MILLISECOND as the last one *and* leaves the byte length identical. The cost
/// of that is one derived copy staying stale until the next change — which is
/// the failure this whole feature exists to remove, so it is worth knowing that
/// the window is a millisecond of wall clock and an exact length match, not a
/// second's mtime granularity. A content hash would close it and would cost
/// exactly what the fingerprint exists to avoid.
pub fn fingerprint(root: &Path) -> u64 {
    fingerprint_with(root, None)
}

/// `fingerprint` with the project's `[docs]` section, exactly as `index_with`
/// takes it. The caller must pass the SAME config it indexes with, or the
/// digest describes a different set of files than the one it is guarding.
pub fn fingerprint_with(root: &Path, docs: Option<&Docs>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let (_, truncated) = walk(root, docs, |root, rel, _group| {
        let (ms, len) = stat_ms_len(&root.join(rel));
        // A separator between the fields, so `("ab", 1)` and `("a", 0xb1)`
        // cannot fold to the same state. `rel` is the only variable-length part.
        h = fnv1a(h, rel.as_bytes());
        h = fnv1a(h, &[0xff]);
        h = fnv1a(h, &ms.to_le_bytes());
        h = fnv1a(h, &len.to_le_bytes());
    });
    // The cap is part of the answer: an index that stopped at 2,000 and one
    // that stopped at 2,000 for a different reason are the same digest, but a
    // place that CROSSED the cap since the last look has genuinely changed.
    fnv1a(h, &[truncated as u8])
}

/// Fold a stat of each of `rels` into an existing fingerprint.
///
/// **Why this is separate from the walk.** The digest above is stat-only over
/// the MARKDOWN the index lists, and a screenshot is not markdown — so an
/// edited image moved nothing and the browser went on serving the old one. The
/// obvious repair is to have the fingerprint read documents and find their
/// references, which is precisely the cost `fingerprint_with` exists to avoid
/// (8 KiB per file, per place, every few seconds). Instead the caller hands
/// back the images it copied on the LAST derive — paths it already knows — and
/// they are stat'ed alongside the documents.
///
/// What that catches: an image edited, replaced, truncated or deleted, and an
/// image that was missing and has now arrived (the caller lists candidates it
/// could not copy, so a missing file's `(0, 0)` moves when it appears). What it
/// cannot catch: an image referenced by a document for the FIRST time, because
/// nothing here has heard of it yet — but a first reference means the document
/// changed, and that moves the walk's half of the digest in the same tick. The
/// two halves cover each other; neither does alone.
///
/// Order matters, as it does in the walk: this is a fold, not a sum.
pub fn fold_assets(base: u64, root: &Path, rels: &[String]) -> u64 {
    let mut h = base;
    for rel in rels {
        let (ms, len) = stat_ms_len(&root.join(rel));
        h = fnv1a(h, rel.as_bytes());
        // A different separator from the walk's `0xff`, so an image and a
        // document with the same rel and the same stat cannot fold to the same
        // state — the two lists are built by different code and may legally
        // overlap.
        h = fnv1a(h, &[0xfe]);
        h = fnv1a(h, &ms.to_le_bytes());
        h = fnv1a(h, &len.to_le_bytes());
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projcfg::RelPath;
    use std::fs;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("wtdocs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }
    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    fn rels(i: &DocsIndex) -> Vec<&str> {
        i.entries.iter().map(|e| e.rel.as_str()).collect()
    }

    /// The recency mark's whole input. Two things are asserted and the second
    /// is the one that matters: the brief is GITIGNORED in every repo of ours
    /// (`ops::BRIEF_PATH`), so it can never carry a git status — if this field
    /// did not exist, the one document the tool writes itself would be the one
    /// document the Docs tab could never mark as new.
    #[test]
    fn every_entry_carries_an_mtime_including_the_gitignored_ones() {
        let t = tmp("mtime");
        let r = &t.0;
        write(r, "README.md", "# readme");
        write(r, crate::ops::BRIEF_PATH, "# the brief");
        write(r, "task_plan.md", "# plan");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let idx = index(r);
        assert_eq!(idx.entries.len(), 3, "three documents, one of them the brief");
        for e in &idx.entries {
            // Just written, so: real, and not in the future. The window is wide
            // because a slow CI box is not a bug; a ZERO is.
            assert!(e.mtime_ms > 0, "{} indexed with no mtime", e.rel);
            assert!(
                e.mtime_ms >= now_ms.saturating_sub(60_000) && e.mtime_ms <= now_ms + 60_000,
                "{} mtime {} is not within a minute of now ({now_ms})",
                e.rel,
                e.mtime_ms
            );
        }
    }

    #[test]
    fn the_root_files_come_first_in_their_fixed_order_then_the_brief() {
        let t = tmp("order");
        let r = &t.0;
        // written in the wrong order on purpose — the walk imposes its own
        for f in ["CHANGELOG.md", "README.md", "ROADMAP.md", "DESIGN.md", "CLAUDE.md"] {
            write(r, f, "# x");
        }
        write(r, crate::ops::BRIEF_PATH, "# the brief");
        write(r, "NOTES.md", "# notes");
        write(r, "docs/a.md", "# a");
        assert_eq!(
            rels(&index(r)),
            vec!["README.md", "CLAUDE.md", "DESIGN.md", "ROADMAP.md", "CHANGELOG.md", ".planning/brief.md", "NOTES.md", "docs/a.md"],
        );
    }

    /// The frontend renders one block per RUN of equal `group`, so two separate
    /// `""` runs would be two unheaded blocks with the whole tree between them
    /// — and, keyed by group name, two React siblings with the same key. That
    /// is how this was found. The contiguity is the contract.
    #[test]
    fn every_group_is_one_contiguous_run() {
        let t = tmp("runs");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "ZZZ.md", "# z");
        write(r, crate::ops::BRIEF_PATH, "# b");
        write(r, "docs/a.md", "# a");
        write(r, "docs/sub/b.md", "# b");
        let i = index(r);
        let mut runs: Vec<&str> = Vec::new();
        for e in &i.entries {
            if runs.last() != Some(&e.group.as_str()) {
                assert!(!runs.contains(&e.group.as_str()), "group {:?} resumes after another: {:?}", e.group, rels(&i));
                runs.push(&e.group);
            }
        }
        assert_eq!(runs, vec!["", "docs", "docs/sub"]);
    }

    /// APFS answers `symlink_metadata("README.md")` for a file actually named
    /// `Readme.md`, so the named-file pass used to push the CANONICAL spelling
    /// — a `rel`/`path` that does not exist on disk — and the root sweep, whose
    /// `seen` check is a case-SENSITIVE compare, then pushed the real entry a
    /// second time. Two rows for one file, and `viewer::write_tree` wrote both
    /// names, which collide back to one file on the same volume.
    #[test]
    fn a_root_file_is_listed_once_under_the_name_it_really_has() {
        let t = tmp("rootcase");
        let r = &t.0;
        write(r, "Readme.md", "# readme");
        write(r, "changelog.md", "# changes");
        let i = index(r);
        assert_eq!(rels(&i), vec!["Readme.md", "changelog.md"], "the disk's own spelling, once each");
        for e in &i.entries {
            assert!(Path::new(&e.path).exists(), "{} names a path that is not there", e.path);
        }
    }

    #[test]
    fn the_brief_is_grouped_with_the_root_files_not_under_planning() {
        let t = tmp("briefgroup");
        let r = &t.0;
        write(r, crate::ops::BRIEF_PATH, "# the brief");
        let i = index(r);
        assert_eq!(i.entries.len(), 1);
        assert_eq!(i.entries[0].group, "", "the brief is about the PLACE, like the root files");
        assert_eq!(i.entries[0].rel, ".planning/brief.md", "…and says where it lives");
    }

    #[test]
    fn the_walk_never_descends_worktrees_node_modules_target_or_a_dotdir() {
        let t = tmp("skip");
        let r = &t.0;
        write(r, "docs/real.md", "# real");
        // `.worktrees/` on (main) CONTAINS every other place: walking it would
        // file ten places' documentation under one.
        write(r, "docs/.worktrees/other/docs/ghost.md", "# ghost");
        write(r, "docs/node_modules/pkg/README.md", "# vendor");
        write(r, "docs/target/doc/gen.md", "# generated");
        write(r, "docs/dist/out.md", "# built");
        write(r, "docs/.git/hooks/README.md", "# git");
        write(r, "docs/.hidden/secret.md", "# hidden");
        assert_eq!(rels(&index(r)), vec!["docs/real.md"]);
    }

    #[test]
    fn a_symlinked_document_is_listed_as_nothing_at_all() {
        let t = tmp("link");
        let r = &t.0;
        write(r, "docs/real.md", "# real");
        write(r, "outside.md", "# the target");
        // A committed `docs/secrets.md -> ~/.ssh/id_rsa` is the shape this is
        // about; a link to a sibling is the same mechanism, testable anywhere.
        std::os::unix::fs::symlink(r.join("outside.md"), r.join("docs/link.md")).unwrap();
        std::os::unix::fs::symlink(r.join("docs"), r.join("docs-link")).unwrap();
        let i = index(r);
        assert_eq!(rels(&i), vec!["outside.md", "docs/real.md"], "the link is absent; its target listed on its own merits");
    }

    #[test]
    fn the_entry_cap_reports_truncation_rather_than_stopping_quietly() {
        let t = tmp("cap");
        let r = &t.0;
        for i in 0..(MAX_ENTRIES + 10) {
            write(r, &format!("docs/p{i:05}.md"), "# p");
        }
        let i = index(r);
        assert_eq!(i.entries.len(), MAX_ENTRIES);
        assert!(i.truncated, "an index that silently stops is the lie this tab exists to remove");
    }

    /// BOTH sides of the cap, because only one of them is a cap. A test that
    /// writes a file far past `MAX_DEPTH` and finds it absent passes for an
    /// off-by-one in either direction — and the direction that costs something
    /// is the one where the limit arrived early and a real document went
    /// missing with no truncation flag to say so.
    #[test]
    fn the_depth_cap_stops_the_walk_at_max_depth_and_not_before_it() {
        let t = tmp("depth");
        let r = &t.0;
        let dirs = |n: usize| -> String { (1..=n).map(|i| format!("d{i}/")).collect() };
        // `docs/` itself is depth 0, so `docs/d1/…/d{MAX_DEPTH}` is the last
        // directory the walk may descend into.
        write(r, &format!("docs/{}deepest.md", dirs(MAX_DEPTH)), "# deepest");
        write(r, &format!("docs/{}past.md", dirs(MAX_DEPTH + 1)), "# past");
        write(r, "docs/near.md", "# near");
        let i = index(r);
        let listed = rels(&i);
        assert!(listed.iter().any(|r| r.ends_with("/deepest.md")), "MAX_DEPTH is listed, not cut: {listed:?}");
        assert!(!listed.iter().any(|r| r.ends_with("/past.md")), "MAX_DEPTH + 1 was walked: {listed:?}");
        assert!(listed.contains(&"docs/near.md"));
    }

    #[test]
    fn groups_come_out_in_alphabetical_order_with_files_before_subdirectories() {
        let t = tmp("groups");
        let r = &t.0;
        write(r, "docs/index.md", "# index");
        write(r, "docs/zebra/z.md", "# z");
        write(r, "docs/archive/old.md", "# old");
        write(r, "docs/architecture/overview.md", "# overview");
        assert_eq!(
            rels(&index(r)),
            vec!["docs/index.md", "docs/architecture/overview.md", "docs/archive/old.md", "docs/zebra/z.md"],
        );
    }

    #[test]
    fn a_title_is_frontmatter_then_h1_then_the_filename() {
        assert_eq!(title_from("---\ntitle: From frontmatter\n---\n# An H1\n").as_deref(), Some("From frontmatter"));
        assert_eq!(title_from("---\nlayout: page\n---\n\n# An H1\n").as_deref(), Some("An H1"));
        assert_eq!(title_from("just prose, no heading\n"), None);
        // the shapes frontmatter really arrives in
        assert_eq!(title_from("---\ntitle: \"Quoted: with a colon\"\n---\n").as_deref(), Some("Quoted: with a colon"));
        assert_eq!(title_from("---\ntitle: 'single'\n---\n").as_deref(), Some("single"));
        assert_eq!(title_from("---\ntitle:\n---\n# fallback\n").as_deref(), Some("fallback"), "an empty key is no key");
        assert_eq!(title_from("---\nnav:\n  title: nested\n---\n# top\n").as_deref(), Some("top"), "an indented key is some other map's");
        // An opening `---` with no closing one is a thematic break, not a block
        // that runs to EOF. `derive::body` DELETES what it calls
        // frontmatter, so one parser decides this for both of them.
        assert_eq!(title_from("---\n\n# After a rule\n").as_deref(), Some("After a rule"));
        assert_eq!(title_from("#   Spaced   out   ##\n").as_deref(), Some("Spaced out"));
        assert_eq!(title_from("#no space is not a heading\n# yes it is\n").as_deref(), Some("yes it is"));
    }

    /// A document may OPEN with a thematic break — and `---` is the spelling
    /// this repo's own prose uses. Any later `---` then closed a block that was
    /// never opened, and everything between them was frontmatter: `title_from`
    /// merely lost a title to that, but `derive::body` DELETES what this call
    /// says is frontmatter, so the reader lost the top of the document with no
    /// sign that anything was there.
    ///
    /// The tightening is one question asked of the block's first non-blank
    /// line: does it open a YAML MAPPING (`key:`)? Every frontmatter dialect
    /// this tool meets writes one, and prose does not. A `#` line is not
    /// skipped as a YAML comment on purpose — `# Heading` is the far more
    /// likely reading of the same bytes, and skipping it would hand a heading's
    /// document back to the deleter.
    #[test]
    fn a_thematic_break_is_not_an_unclosed_frontmatter_block() {
        let doc = "---\n\nThe opening paragraph, under a rule.\n\n---\n\nMore prose.\n";
        assert_eq!(split_frontmatter(doc), None, "prose between two rules was read as metadata");
        assert_eq!(title_from("---\n\n# The Real Title\n\n---\n\nbody\n").as_deref(), Some("The Real Title"));
        // A block whose first line is a mapping key still is frontmatter, in
        // every shape a generator writes it.
        for front in [
            "---\ntitle: X\n---\nbody\n",
            "---\n\ntitle: X\n---\nbody\n",
            "---\nnav:\n  title: nested\n---\nbody\n",
            "---\ntitle:\n---\nbody\n",
            "---\n\"quoted key\": X\n---\nbody\n",
            "---\ndate: 2026-09-19 10:00:00\n---\nbody\n",
            "---\n---\nbody\n",
        ] {
            assert!(split_frontmatter(front).is_some(), "real frontmatter refused: {front:?}");
            assert_eq!(split_frontmatter(front).unwrap().1, "body\n", "{front:?}");
        }
        // A sentence whose colon is not its first word's is prose, because a
        // key may not carry whitespace.
        assert_eq!(split_frontmatter("---\nSee the note below: it matters.\n\nmore\n---\nbody\n"), None);
        // What is NOT claimed: `Note: something` is a mapping key by every
        // rule any generator applies, and this reads it as one too. The
        // ambiguity is in the bytes, not in the test — it is recorded so the
        // next reader does not take the rule for more than it is.
        assert!(split_frontmatter("---\nNote: a paragraph opening with one word and a colon.\n---\nbody\n").is_some());
    }

    #[test]
    fn a_hash_inside_a_fence_is_a_shell_comment_not_a_title() {
        // The common README: an install snippet before the heading.
        let src = "```sh\n# Install with homebrew\nbrew install x\n```\n\n# The Real Title\n";
        assert_eq!(title_from(src).as_deref(), Some("The Real Title"));
        let tilde = "~~~\n# not this\n~~~\n# this\n";
        assert_eq!(title_from(tilde).as_deref(), Some("this"));
        // …and a fence that never closes swallows the rest, which is correct:
        // there is no heading outside it.
        assert_eq!(title_from("```\n# nope\n"), None);
    }

    #[test]
    fn a_title_is_read_from_disk_and_falls_back_to_the_filename_stem() {
        let t = tmp("titles");
        let r = &t.0;
        write(r, "README.md", "---\ntitle: The Readme\n---\n# ignored\n");
        write(r, "docs/plain.md", "no heading here\n");
        write(r, "docs/heading.md", "# A Heading\n");
        let i = index(r);
        let by = |rel: &str| i.entries.iter().find(|e| e.rel == rel).unwrap().title.clone();
        assert_eq!(by("README.md"), "The Readme");
        assert_eq!(by("docs/plain.md"), "plain");
        assert_eq!(by("docs/heading.md"), "A Heading");
    }

    /// Layer B of the path boundary, stated as the invariant rather than as a
    /// list of the shapes that break it. The caller has already proved `root`
    /// is under a registered project (`guard_under_projects`); this is the
    /// promise that the walk cannot then hand back something that is not.
    ///
    /// It is asserted on the CANONICAL path deliberately. A symlinked directory
    /// escapes while every `rel` still reads as a child — `docs/out -> /etc`
    /// yields `docs/out/passwd`, which is under the root as a string and
    /// nowhere near it on disk. Resolve, then compare.
    #[test]
    fn every_entry_resolves_under_the_root_it_was_given() {
        let t = tmp("contain");
        let r = &t.0;
        write(r, "docs/real.md", "# real");
        let away = t.0.join("..").join(format!("wtdocs-away-{}", std::process::id()));
        fs::create_dir_all(&away).unwrap();
        fs::write(away.join("leak.md"), "# leak").unwrap();
        std::os::unix::fs::symlink(&away, r.join("docs/out")).unwrap();
        std::os::unix::fs::symlink(away.join("leak.md"), r.join("docs/leak.md")).unwrap();
        let canon_root = fs::canonicalize(r).unwrap();
        let i = index(r);
        for e in &i.entries {
            let c = fs::canonicalize(&e.path).unwrap();
            assert!(c.starts_with(&canon_root), "{} resolves to {} — outside {}", e.rel, c.display(), canon_root.display());
        }
        assert_eq!(rels(&i), vec!["docs/real.md"]);
        let _ = fs::remove_dir_all(&away);
    }

    // ── [docs] (proposal §3.2) ───────────────────────────────────────────────

    fn cfg(paths: &[&str], index: Option<&str>) -> Docs {
        Docs {
            paths: paths.iter().map(|p| RelPath::parse(p).unwrap()).collect(),
            index: index.map(|i| RelPath::parse(i).unwrap()),
        }
    }

    #[test]
    fn declared_paths_replace_the_tree_and_keep_the_root_files() {
        let t = tmp("cfgpaths");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "docs/convention.md", "# c");
        write(r, "handbook/a.md", "# a");
        write(r, "packages/db/README.md", "# db");
        let i = index_with(r, Some(&cfg(&["handbook", "packages/db/README.md"], None)));
        assert_eq!(
            rels(&i),
            vec!["README.md", "handbook/a.md", "packages/db/README.md"],
            "declared paths replace docs/, the ROOT files stay — they are about the repo, not its layout",
        );
    }

    #[test]
    fn a_declared_directory_is_walked_and_a_declared_file_is_listed_alone() {
        let t = tmp("cfgkinds");
        let r = &t.0;
        write(r, "handbook/a.md", "# a");
        write(r, "handbook/deep/b.md", "# b");
        write(r, "notes/one.md", "# one");
        write(r, "notes/two.md", "# two");
        let i = index_with(r, Some(&cfg(&["handbook", "notes/one.md"], None)));
        // DECLARED order, not sorted and not dirs-before-files: the repo chose
        // this order, and second-guessing it is the one thing this must not do.
        assert_eq!(rels(&i), vec!["handbook/a.md", "handbook/deep/b.md", "notes/one.md"]);
        // the single file is grouped by its own directory, like any other entry
        assert_eq!(i.entries[2].group, "notes");
    }

    #[test]
    fn a_declared_path_that_does_not_exist_is_skipped_not_an_error() {
        // Four of eleven places have no `apps/docs` on their branch. A config
        // written on main must not make the index fail on a place that has not
        // rebased — that is the inconsistency the tab exists to remove.
        let t = tmp("cfgmissing");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "handbook/a.md", "# a");
        let i = index_with(r, Some(&cfg(&["handbook", "apps/docs", "gone.md"], None)));
        assert_eq!(rels(&i), vec!["README.md", "handbook/a.md"]);
        assert!(!i.truncated);
    }

    #[test]
    fn the_declared_index_is_listed_first_above_the_root_files() {
        let t = tmp("cfgindex");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "CLAUDE.md", "# c");
        write(r, "docs/index.md", "# landing");
        let i = index_with(r, Some(&cfg(&[], Some("docs/index.md"))));
        assert_eq!(rels(&i)[0], "docs/index.md", "reading starts here: {:?}", rels(&i));
        assert_eq!(i.entries[0].group, "", "…and it sits with the root files, not under docs/");
        // …and it is not listed TWICE when the convention would have found it
        assert_eq!(rels(&i).iter().filter(|r| **r == "docs/index.md").count(), 1);
        assert_eq!(rels(&i), vec!["docs/index.md", "README.md", "CLAUDE.md"]);
    }

    /// The same family one level in: a declared `index` is a path the PROJECT
    /// spelled, and the file it names may be spelled differently on disk.
    #[test]
    fn a_declared_index_is_found_under_the_filesystems_own_spelling() {
        let t = tmp("cfgindexcase");
        let r = &t.0;
        write(r, "docs/Index.md", "# landing");
        let i = index_with(r, Some(&cfg(&[], Some("docs/index.md"))));
        assert_eq!(rels(&i), vec!["docs/Index.md"], "listed once, under the name it really has");
        assert!(Path::new(&i.entries[0].path).exists(), "{}", i.entries[0].path);
    }

    /// The fallback's boundary, stated where a filesystem cannot be used to
    /// state it: this machine is APFS, so the case-SENSITIVE half of the rule
    /// has no way to exist on disk here. An exact entry always wins its slot,
    /// and only the volume that could never have told the spellings apart falls
    /// back — otherwise a Linux checkout carrying both files would lose one.
    #[test]
    fn an_exact_entry_wins_its_slot_and_only_a_missing_one_falls_back() {
        let both = vec!["README.md".to_string(), "Readme.md".to_string()];
        assert_eq!(resolve_name(&both, "README.md").as_deref(), Some("README.md"));
        let only_odd = vec!["Readme.md".to_string()];
        assert_eq!(resolve_name(&only_odd, "README.md").as_deref(), Some("Readme.md"));
        assert_eq!(resolve_name(&only_odd, "CHANGELOG.md"), None);
        // Several inexact and no exact: deterministic, and the rest are left to
        // the root sweep rather than dropped.
        let many = vec!["readme.md".to_string(), "ReadMe.md".to_string()];
        assert_eq!(resolve_name(&many, "README.md").as_deref(), Some("ReadMe.md"), "byte order, so the answer is stable");
    }

    #[test]
    fn a_declared_index_that_is_missing_changes_nothing() {
        let t = tmp("cfgindexgone");
        let r = &t.0;
        write(r, "README.md", "# r");
        let i = index_with(r, Some(&cfg(&[], Some("docs/index.md"))));
        assert_eq!(rels(&i), vec!["README.md"]);
    }

    #[test]
    fn a_declared_path_is_still_walked_under_every_layer_b_rule() {
        // Layer A already refused `..`, `~`, `.git` and friends at parse time.
        // This is the OTHER half: a path that is legal as a string still gets
        // the walk's own refusals — no symlink following, no `.worktrees`, no
        // dotted directory, and the containment invariant.
        let t = tmp("cfglayerb");
        let r = &t.0;
        write(r, "handbook/a.md", "# a");
        write(r, "handbook/.hidden/x.md", "# x");
        write(r, "handbook/node_modules/pkg/README.md", "# vendor");
        write(r, "handbook/.worktrees/other/docs/ghost.md", "# ghost");
        let away = t.0.join("..").join(format!("wtdocs-cfgaway-{}", std::process::id()));
        fs::create_dir_all(&away).unwrap();
        fs::write(away.join("leak.md"), "# leak").unwrap();
        std::os::unix::fs::symlink(&away, r.join("handbook/out")).unwrap();
        let i = index_with(r, Some(&cfg(&["handbook"], None)));
        assert_eq!(rels(&i), vec!["handbook/a.md"]);
        let canon_root = fs::canonicalize(r).unwrap();
        for e in &i.entries {
            assert!(fs::canonicalize(&e.path).unwrap().starts_with(&canon_root), "{}", e.rel);
        }
        let _ = fs::remove_dir_all(&away);
    }

    #[test]
    fn declared_paths_keep_one_contiguous_run_per_group() {
        // The contract the frontend renders against, under config too.
        let t = tmp("cfgruns");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "b/one.md", "# 1");
        write(r, "a/two.md", "# 2");
        write(r, "a/sub/three.md", "# 3");
        let i = index_with(r, Some(&cfg(&["b", "a"], Some("README.md"))));
        let mut runs: Vec<&str> = Vec::new();
        for e in &i.entries {
            if runs.last() != Some(&e.group.as_str()) {
                assert!(!runs.contains(&e.group.as_str()), "group {:?} resumes: {:?}", e.group, rels(&i));
                runs.push(&e.group);
            }
        }
        assert_eq!(runs, vec!["", "b", "a", "a/sub"], "declared ORDER is preserved, not sorted");
    }

    #[test]
    fn a_place_with_no_documentation_is_an_empty_index_not_a_failure() {
        let t = tmp("empty");
        let i = index(&t.0);
        assert!(i.entries.is_empty());
        assert!(!i.truncated);
        // …and so is a directory that does not exist at all.
        let gone = index(&t.0.join("nope"));
        assert!(gone.entries.is_empty());
    }

    // ── the fingerprint ──────────────────────────────────────────────────────

    /// Rewrite a file and make sure the stat the fingerprint reads has actually
    /// moved. mtime is milliseconds and a test can rewrite inside one of them,
    /// which would make a change-detection test flaky in the direction that
    /// reads as a PASS for the broken code — so the helper waits for the stat to
    /// differ rather than sleeping a guessed amount.
    fn rewrite(root: &Path, rel: &str, body: &str) {
        let before = stat_ms_len(&root.join(rel));
        for _ in 0..200 {
            write(root, rel, body);
            if stat_ms_len(&root.join(rel)) != before {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("{rel}: the filesystem never reported a new mtime or length");
    }

    /// The whole point: an edit moves it, and nothing else does.
    ///
    /// The second half is what makes the first one worth anything — a digest
    /// that changed on every call would also "detect" every edit, and would
    /// re-derive every registered place on every tick forever.
    #[test]
    fn a_touched_document_moves_the_fingerprint_and_a_quiet_place_does_not() {
        let t = tmp("fp-touch");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "docs/a.md", "# a");
        let first = fingerprint(r);
        assert_eq!(first, fingerprint(r), "a place nobody touched must read the same twice");
        rewrite(r, "docs/a.md", "# a\n\nnow with prose\n");
        assert_ne!(first, fingerprint(r), "an edited document did not move the fingerprint");
    }

    /// Membership, not just mtimes — and a removal is the case that proves it,
    /// because deleting a file touches no surviving file's stat at all.
    ///
    /// The deleted document is deliberately **not** the most recently written
    /// one. The obvious cheap answer to "has this place changed" is the newest
    /// mtime in the tree, and it reads this place as untouched while its index
    /// has lost a row — so the derived copy in the browser would keep serving a
    /// document that no longer exists, which is `forget_place`'s hazard arriving
    /// one file at a time.
    #[test]
    fn a_removed_or_added_document_moves_the_fingerprint() {
        let t = tmp("fp-set");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "docs/b.md", "# b");
        write(r, "docs/a.md", "# a");
        rewrite(r, "docs/a.md", "# a, and now the newest file here");
        let with_b = fingerprint(r);
        fs::remove_file(r.join("docs/b.md")).unwrap();
        let without_b = fingerprint(r);
        assert_ne!(with_b, without_b, "a deleted document did not move the fingerprint");
        write(r, "docs/c.md", "# c");
        assert_ne!(without_b, fingerprint(r), "a new document did not move the fingerprint");
    }

    /// It walks EXACTLY what the index lists, because it is the same walk.
    ///
    /// Everything written here is something `index_with` refuses — another
    /// place's documents under `.worktrees/`, a vendored `README.md`, a dotted
    /// directory, a symlink out of the tree, a file that is not markdown. A
    /// fingerprint written as its own recursive walk would see all of them, and
    /// the failure would be silent in the expensive direction: `(main)` contains
    /// every other place, so every keystroke in every worktree would re-derive
    /// `(main)`'s browser copy.
    #[test]
    fn the_fingerprint_ignores_exactly_what_the_index_ignores() {
        let t = tmp("fp-skip");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "docs/a.md", "# a");
        let quiet = fingerprint(r);
        assert_eq!(rels(&index(r)), vec!["README.md", "docs/a.md"]);

        write(r, ".worktrees/other/docs/ghost.md", "# another place entirely");
        write(r, "docs/node_modules/pkg/README.md", "# somebody else's code");
        write(r, "docs/.hidden/x.md", "# dotted");
        write(r, "target/debug/notes.md", "# build output");
        write(r, "docs/a.txt", "not markdown");
        let away = t.0.join("..").join(format!("wtdocs-fpaway-{}", std::process::id()));
        fs::create_dir_all(&away).unwrap();
        fs::write(away.join("leak.md"), "# leak").unwrap();
        let _ = fs::remove_file(r.join("docs/out"));
        std::os::unix::fs::symlink(&away, r.join("docs/out")).unwrap();

        assert_eq!(rels(&index(r)), vec!["README.md", "docs/a.md"], "the index changed; the premise is gone");
        assert_eq!(quiet, fingerprint(r), "the fingerprint saw files the index does not list");
        let _ = fs::remove_dir_all(&away);
    }

    /// The config has to be the SAME config the index was built with, or the
    /// digest is guarding a different set of files than the one on screen — a
    /// place whose `[docs] paths` moved documentation out of `docs/` would be
    /// watched at the wrong address, and the copy in the browser would never
    /// refresh again.
    #[test]
    fn the_fingerprint_follows_the_declared_paths() {
        let t = tmp("fp-cfg");
        let r = &t.0;
        write(r, "README.md", "# r");
        write(r, "handbook/a.md", "# a");
        write(r, "docs/ignored.md", "# not declared");
        let c = cfg(&["handbook"], None);
        let declared = fingerprint_with(r, Some(&c));
        // `docs/` is not in the declared set, so nothing that happens there is
        // this place's documentation.
        rewrite(r, "docs/ignored.md", "# edited, and still not declared");
        assert_eq!(declared, fingerprint_with(r, Some(&c)), "an undeclared directory moved the fingerprint");
        rewrite(r, "handbook/a.md", "# a, edited");
        assert_ne!(declared, fingerprint_with(r, Some(&c)), "a declared document did not move it");
        // …and the convention's answer for the same tree is a different one,
        // which is why the caller may not mix them up.
        assert_ne!(fingerprint(r), fingerprint_with(r, Some(&c)));
    }

    /// Declared ORDER is a decision the repo made (`[docs] paths = ["b", "a"]`
    /// is listed b-then-a on purpose), so a change to it is a change to the
    /// index — and the digest has to move or the derived copy keeps the old
    /// order forever.
    #[test]
    fn declared_order_is_part_of_the_fingerprint() {
        let t = tmp("fp-order");
        let r = &t.0;
        write(r, "a/one.md", "# 1");
        write(r, "b/two.md", "# 2");
        let ab = fingerprint_with(r, Some(&cfg(&["a", "b"], None)));
        let ba = fingerprint_with(r, Some(&cfg(&["b", "a"], None)));
        assert_ne!(ab, ba, "the two orders are the same digest, so a re-ordered index cannot be seen");
    }

    /// The walk lists markdown, so an IMAGE is invisible to it — which is why a
    /// re-derived document kept showing the screenshot it was copied with. The
    /// caller hands back the images it copied and they are stat'ed here; this
    /// says that each of the four transitions that matter moves the digest, and
    /// that a place where nothing happened does not.
    #[test]
    fn an_image_moves_the_fingerprint_that_the_walk_cannot_see() {
        let t = tmp("fp-assets");
        let r = &t.0;
        write(r, "README.md", "# Read me\n\n![shot](shots/a.png)\n");
        fs::create_dir_all(r.join("shots")).unwrap();
        fs::write(r.join("shots/a.png"), b"first").unwrap();
        let rels = vec!["shots/a.png".to_string()];
        let docs = fingerprint(r);

        let base = fold_assets(docs, r, &rels);
        // Nothing happened.
        assert_eq!(base, fold_assets(docs, r, &rels), "a quiet place does not re-derive");
        // The walk on its own cannot tell any of the rest apart.
        fs::write(r.join("shots/a.png"), b"second and longer").unwrap();
        assert_eq!(docs, fingerprint(r), "the walk is supposed to be blind to this");
        let edited = fold_assets(docs, r, &rels);
        assert_ne!(base, edited, "an edited image moved nothing");
        // Deleted.
        fs::remove_file(r.join("shots/a.png")).unwrap();
        let gone = fold_assets(docs, r, &rels);
        assert_ne!(edited, gone, "a deleted image moved nothing");
        // Arrived: a candidate that was missing is the only way this is ever
        // noticed, which is why the caller reports what it could NOT copy too.
        fs::write(r.join("shots/a.png"), b"first").unwrap();
        assert_ne!(gone, fold_assets(docs, r, &rels), "an image that arrived moved nothing");
        // And the list itself is part of the answer: a document that stopped
        // referencing an image changes what is folded, not just the stats.
        assert_ne!(base, fold_assets(docs, r, &[]), "the reference list is not in the digest");
    }
}
