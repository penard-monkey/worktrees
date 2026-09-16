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
    let mut lines = text.lines().peekable();
    // ── frontmatter ──
    if matches!(lines.peek(), Some(&"---")) {
        lines.next();
        let mut found: Option<String> = None;
        for line in lines.by_ref() {
            let t = line.trim_end();
            if t == "---" || t == "..." {
                break;
            }
            // Top-level key only: an indented `title:` belongs to some nested
            // map, and guessing which one is per-generator work.
            if found.is_none() {
                if let Some(v) = line.strip_prefix("title:") {
                    found = clean_title(unquote(v));
                }
            }
        }
        if found.is_some() {
            return found;
        }
    }
    // ── first H1, outside any fence ──
    let mut fence: Option<String> = None;
    for line in lines {
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

/// Build one entry. `rel` is the caller's business — it is what the row shows —
/// and so is `group`, which is NOT simply `rel`'s parent: the brief lives in
/// `.planning/` and belongs with the root files anyway (see `DocEntry::group`).
fn entry(root: &Path, rel: &str, group: &str) -> DocEntry {
    let path = root.join(rel);
    let name = Path::new(rel).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| rel.to_string());
    DocEntry {
        title: title_of(&path, &name),
        path: path.to_string_lossy().to_string(),
        rel: rel.to_string(),
        group: group.to_string(),
    }
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
    let mut entries: Vec<DocEntry> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut truncated = false;

    // ONE dedupe rule for the whole walk, and it lives here rather than at the
    // four call sites. `[docs] index = "docs/index.md"` names a file the tree
    // pass then walks into, so the row came out twice — and a per-site check
    // would have closed that one route and left the next one open. `false`
    // means the CAP was hit, which is the only thing the caller must report; a
    // duplicate is simply not pushed.
    let push = |entries: &mut Vec<DocEntry>, seen: &mut BTreeSet<String>, rel: String, group: &str| -> bool {
        if seen.contains(&rel) {
            return true;
        }
        if entries.len() >= MAX_ENTRIES {
            return false;
        }
        entries.push(entry(root, &rel, group));
        seen.insert(rel);
        true
    };

    // 0. `[docs] index` — where reading starts, ahead of everything. Grouped
    //    with the root files however deep it lives: it is the landing page for
    //    the whole place, so a `docs` header above it would file it under a
    //    tree it is introducing.
    if let Some(i) = docs.and_then(|d| d.index.as_ref()) {
        if is_regular_file(&root.join(i.as_str())) && !push(&mut entries, &mut seen, i.as_str().to_string(), "") {
            truncated = true;
        }
    }
    // 1. the named root files, in the order a reader wants them
    for f in ROOT_FILES {
        if is_regular_file(&root.join(f)) && !push(&mut entries, &mut seen, f.to_string(), "") {
            truncated = true;
        }
    }
    // 2. this place's brief — the one document the tool itself writes
    if is_regular_file(&root.join(crate::ops::BRIEF_PATH))
        && !push(&mut entries, &mut seen, crate::ops::BRIEF_PATH.to_string(), "")
    {
        truncated = true;
    }
    // 3. whatever other markdown is at the root — with the named files above,
    //    not stranded below the tree (see the module note)
    let mut rest: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !is_md(&name) || seen.contains(&name) {
                continue;
            }
            if std::fs::symlink_metadata(e.path()).map(|m| m.is_file()).unwrap_or(false) {
                rest.push(name);
            }
        }
    }
    rest.sort_by_key(|r| r.to_lowercase());
    for r in rest {
        if !push(&mut entries, &mut seen, r, "") {
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
                if !push(&mut entries, &mut seen, rel.clone(), &group) {
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
                if !push(&mut entries, &mut seen, r, &rel) {
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
    DocsIndex { entries, truncated }
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

    #[test]
    fn the_depth_cap_stops_the_walk() {
        let t = tmp("depth");
        let r = &t.0;
        let deep: String = (1..=MAX_DEPTH + 2).map(|i| format!("d{i}/")).collect();
        write(r, &format!("docs/{deep}far.md"), "# far");
        write(r, "docs/near.md", "# near");
        let i = index(r);
        assert!(i.entries.iter().all(|e| !e.rel.ends_with("far.md")), "past MAX_DEPTH: {:?}", rels(&i));
        assert!(i.entries.iter().any(|e| e.rel == "docs/near.md"));
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
        assert_eq!(title_from("#   Spaced   out   ##\n").as_deref(), Some("Spaced out"));
        assert_eq!(title_from("#no space is not a heading\n# yes it is\n").as_deref(), Some("yes it is"));
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
}
