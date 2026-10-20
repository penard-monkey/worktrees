//! The right dock's Files tab: the tree, diffs, reads and writes.
//!
//! Every path that reaches disk goes through `projects::guard_under_projects`
//! — canonicalise, then require the result under a registered project root.
//! AGENTS.md records why that is not optional and why a partial guard looks
//! like a working one: `symlink_metadata` refuses to follow only the LAST
//! component, so a check built from a constant path reads straight through an
//! intermediate link while the walker beside it refuses the same row.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};
use worktrees_core::{git, harness, ops, provider, sysclock, tmux, Project};

use crate::log::log as applog;
use crate::projects::{guard_under_projects, project_roots, under_roots};
use crate::term::{live_pid, proc_cwd, Shells};
use crate::{viewer, Ctx};


#[derive(Serialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// Only ever true when the caller asked for ignored entries — the filtered
    /// listing has nothing to mark, since everything ignored is already gone.
    pub ignored: bool,
    /// Symlink target exactly as written — a relative link stays relative,
    /// because that is what is on disk and what the user would `ls -l`. None
    /// for every ordinary entry.
    pub link: Option<String>,
    /// Why the tree must not follow this link, or None when it may. A reason
    /// code rather than a bool: all three render inert, but "outside the
    /// workspace" is a false statement about a link that merely dangles, and
    /// the row's tooltip is the only place a user learns why nothing happens.
    pub link_block: Option<&'static str>,
}

#[derive(Serialize)]
pub struct FileContent {
    pub content: String,
    pub truncated: bool,
    pub binary: bool,
    /// mtime (ms since epoch) — the dock echoes it back on save as a
    /// compare-and-swap token so a stale buffer can't clobber a newer edit.
    pub mtime: u64,
    /// Full size on disk (NOT the length of `content`, which is capped) — the
    /// viewer header shows it, and it's the only honest number for a file that
    /// came back truncated or binary.
    pub size: u64,
}

/// An image (or any small blob) as base64, for the viewer's `data:` URI. Kept
/// separate from `read_file` so the text path never pays for the encode.
#[derive(Serialize)]
pub struct FileBlob {
    pub b64: String,
    /// Bytes actually encoded (== `size` unless the cap truncated the read).
    pub size: u64,
    pub truncated: bool,
    pub mtime: u64,
}

/// How one path differs from the branch's base, coarsely. Four classes is what
/// a tinted NAME can carry on its own; a fifth would need a second channel.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Modified,
    Added,
    Untracked,
    Deleted,
}

#[derive(Serialize, PartialEq, Eq, Debug)]
pub struct FileChange {
    /// Absolute, canonical — the same shape `list_dir` hands the tree, so the
    /// frontend can look a row up by `entry.path` with no normalizing.
    pub path: String,
    pub status: ChangeKind,
}

/// Base-branch candidates, most authoritative first — the remote's view before
/// the local one, since `origin/main` is what moves on fetch.
///
/// Shared by `changed_files` and `file_diff` ON PURPOSE, so the two can never
/// resolve different BASES. Verified across eight repo shapes (committed-only,
/// uncommitted-only, base moved on, no origin, no main/master, unrelated
/// histories, rename): the loops always settle on the same candidate, even
/// though one breaks on the first `diff <cand>...HEAD` that succeeds and the
/// other on the first `merge-base` that does — they fail under identical
/// conditions.
///
/// A shared base is NOT a promise the two always agree on a given file, and one
/// case makes that visible: a file committed on the branch and then changed BACK
/// in the working tree. `changed_files` marks it (the worktree differs from
/// HEAD) while `file_diff` vs base correctly finds nothing, so a lit row sits
/// above "no changes". Both are true — the branch touched it, and it now equals
/// the base — and the HEAD toggle shows the revert. The set semantics genuinely
/// differ: commit-range ∪ working-tree status here, merge-base vs worktree there.
pub const BASE_CANDS: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

/// Everything in one worktree that differs from its branch's base.
#[derive(Serialize)]
pub struct ChangeSet {
    /// The repo top-level these paths hang off, canonicalized. Returned rather
    /// than assumed: the tree walks ancestors to cascade a marker upward, and it
    /// needs to know where to stop without trusting the `root` it passed in
    /// (a place path can be a symlink; every path here is resolved).
    pub root: String,
    pub files: Vec<FileChange>,
}

/// File mtime in ms since the epoch (0 if unavailable) — a cheap change token.
pub fn file_mtime_ms(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}


/// One symlink as the tree needs it: `(is_dir, target, why_not_to_follow)`.
///
/// `is_dir` follows the link, because the shape a user means by `_tmp/` is the
/// target's. `target` is what the link literally says — a relative link stays
/// relative, matching `ls -l`.
///
/// The block reason is the guard's own question asked ahead of time, plus the
/// one thing the guard does not ask: `.git`. The listing drops it by NAME, so
/// without this a repo shipping `ln -s .git g` would hand the tree a browsable
/// caret straight into it — the whole point of not following links is that a
/// repo's own contents do not get to choose what the app opens.
pub fn classify_symlink(p: &Path, roots: &[PathBuf]) -> (bool, Option<String>, Option<&'static str>) {
    let is_dir = std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false);
    let target = std::fs::read_link(p).map(|t| t.to_string_lossy().to_string()).ok();
    let block = match std::fs::canonicalize(p) {
        // Dangling. Every command behind the row canonicalizes too, so it is
        // just as inert as one pointing away — for a different reason.
        Err(_) => Some("missing"),
        Ok(c) if !under_roots(roots, &c) => Some("outside"),
        Ok(c) if c.components().any(|s| s.as_os_str() == ".git") => Some("git"),
        Ok(_) => None,
    };
    (is_dir, target, block)
}

/// Immediate children of `path` (one level; the tree lazy-expands). Dirs first,
/// then case-insensitive by name.
///
/// `.git` is always dropped. Gitignored entries are dropped too UNLESS
/// `show_ignored` asks for them, in which case they are returned flagged rather
/// than filtered — the tree dims them. The toggle exists because the files a
/// session actually produces (build output, and this repo's own gitignored
/// working notes) are exactly the ones the filtered listing hides.
///
/// Symlinks are stat'd THROUGH for their shape (so a link to a directory reads
/// as one) and carry their target plus `link_block`, the reason the tree must
/// not follow them. Following is left to the tree, which does not: the guard
/// canonicalizes, so a link out of the workspace is unlistable by construction,
/// and honouring one would let a repo's own contents choose what the app reads.
pub fn list_dir(ctx: &Ctx, path: String, show_ignored: Option<bool>) -> Result<Vec<FsEntry>, String> {
    let dir = guard_under_projects(ctx, &path)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let mut entries: Vec<FsEntry> = Vec::new();
    // Built on the first symlink seen, not up front: most directories have none
    // and would pay a projects-file read plus a canonicalize per root for it.
    let mut roots: Option<Vec<PathBuf>> = None;
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        let ft = e.file_type().ok();
        let p = e.path();
        // read_dir's file_type is an lstat: it answers about the LINK, so a
        // symlink to a directory came back `is_dir: false` — a file glyph, no
        // caret, and a click that tried to open it as a file. Stat the target
        // for the shape; a broken link has none and stays a file.
        let (is_dir, link, link_block) = match ft {
            Some(t) if t.is_symlink() => {
                classify_symlink(&p, roots.get_or_insert_with(|| project_roots(ctx)))
            }
            other => (other.map(|t| t.is_dir()).unwrap_or(false), None, None),
        };
        let path = p.to_string_lossy().to_string();
        entries.push(FsEntry { name, path, is_dir, ignored: false, link, link_block });
    }
    // One `git check-ignore --stdin` batch for the whole directory (NUL-safe).
    let ignored = git_check_ignore(&dir, entries.iter().map(|e| e.path.as_str()));
    if show_ignored.unwrap_or(false) {
        for e in &mut entries {
            e.ignored = ignored.contains(&e.path);
        }
    } else {
        entries.retain(|e| !ignored.contains(&e.path));
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Paths git would ignore (best-effort). `-z` in/out keeps it robust to spaces
/// and newlines in filenames. Empty set on any git failure → nothing filtered.
pub fn git_check_ignore<'a>(dir: &Path, paths: impl Iterator<Item = &'a str>) -> std::collections::HashSet<String> {
    use std::process::Stdio;
    let mut stdin_buf = Vec::new();
    for p in paths {
        stdin_buf.extend_from_slice(p.as_bytes());
        stdin_buf.push(0);
    }
    if stdin_buf.is_empty() {
        return Default::default();
    }
    let child = worktrees_core::childenv::command("git")
        .args(["-C", &dir.to_string_lossy(), "check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(_) => return Default::default(),
    };
    // Write stdin on its own thread while the parent drains stdout below — a big
    // directory's path list can exceed the pipe buffer, and writing all of it
    // before reading would deadlock (git blocks on stdout, we block on stdin).
    if let Some(mut si) = child.stdin.take() {
        thread::spawn(move || {
            let _ = si.write_all(&stdin_buf);
            // drop closes the pipe → git sees EOF
        });
    }
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return Default::default(),
    };
    out.stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect()
}

/// Every path under `root` that differs from the branch's BASE — committed on
/// this branch *and* uncommitted (staged, unstaged, untracked). The Files tab
/// tints those rows and cascades the mark up through their directories.
///
/// THREE spawns per call, and the call is per REFRESH, not per directory: the
/// frontend derives a set once and answers every row from it. Doing this the way
/// `list_dir` does `check-ignore` — a batch per listing — would be one git
/// process per open node per poll tick, and the tree re-lists every open node.
///
/// A path that is not a repo (or has no git) gets an empty set, not an error:
/// nothing differs from a base that does not exist, and the tree must still list.
pub fn changed_files(ctx: &Ctx, root: String) -> Result<ChangeSet, String> {
    let dir = guard_under_projects(ctx, &root)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    let cwd = dir.to_string_lossy().to_string();
    // Porcelain paths are relative to the repo TOP-LEVEL, never to `-C`. For a
    // place that IS the worktree root they agree; resolving it anyway is what
    // keeps a root one level down from marking the wrong rows.
    let top = match git::git_out(&cwd, &["rev-parse", "--show-toplevel"]) {
        Some(t) if !t.is_empty() => std::fs::canonicalize(&t).unwrap_or_else(|_| PathBuf::from(t)),
        _ => return Ok(ChangeSet { root: cwd, files: Vec::new() }),
    };
    let mut map: BTreeMap<String, ChangeKind> = BTreeMap::new();
    // Committed on this branch. THREE dots — the diff is against the merge base,
    // so a base branch that has moved on since doesn't light up every file those
    // other commits touched. Candidates follow core's `base_ref()` precedence
    // (the remote's view first, since `origin/main` is what moves on fetch); the
    // first range git can resolve wins, and a repo with none of them — an unborn
    // HEAD, a repo with no main/master — just gets the uncommitted half.
    for cand in BASE_CANDS {
        if let Some(out) = git::git_out(&cwd, &["diff", "--name-status", "-z", &format!("{cand}...HEAD")]) {
            for (p, k) in parse_name_status_z(&out) {
                map.insert(p, k);
            }
            break;
        }
    }
    // Uncommitted. `--untracked-files=all` is not optional: the default
    // `-unormal` collapses a whole new directory into one `dir/` record, which
    // would mark the directory and leave every file inside it unmarked.
    //
    // Applied SECOND so it wins on collision — it describes the disk as it is
    // now. A file added in a commit and then deleted has to read `deleted`
    // (there is no row to tint, only a ghost to draw), not `added`.
    if let Some(out) = git::git_out(&cwd, &["status", "--porcelain", "-z", "--untracked-files=all"]) {
        for (p, k) in parse_status_z(&out) {
            map.insert(p, k);
        }
    }
    let files = map
        .into_iter()
        .map(|(rel, status)| FileChange { path: top.join(&rel).to_string_lossy().to_string(), status })
        .collect();
    Ok(ChangeSet { root: top.to_string_lossy().to_string(), files })
}

/// `git status --porcelain -z --untracked-files=all` → (repo-relative, kind).
///
/// `-z` records are NUL-terminated `XY <path>`, and a rename/copy puts the
/// ORIGINAL path in the NEXT record instead of after a ` -> ` — which is the
/// whole reason for `-z`: no C-quoting, and no ambiguity with a filename that
/// contains an arrow. A rename's original is GONE from disk, so it is reported
/// deleted; that is what gives the tree a ghost row to draw for it.
pub fn parse_status_z(out: &str) -> Vec<(String, ChangeKind)> {
    let mut it = out.split('\0').filter(|r| !r.is_empty());
    let mut v = Vec::new();
    while let Some(rec) = it.next() {
        // "XY path": two status columns, a space, then at least one path byte.
        if rec.len() < 4 {
            continue;
        }
        let (x, y) = (rec.as_bytes()[0], rec.as_bytes()[1]);
        // Byte 3 is a char boundary — the two columns and the space are ASCII.
        let path = rec[3..].to_string();
        match (x, y) {
            (b'?', _) => v.push((path, ChangeKind::Untracked)),
            (b'!', _) => {} // ignored — only ever emitted with --ignored, never asked for
            (b'R', _) | (b'C', _) => {
                let orig = it.next().unwrap_or("").to_string();
                v.push((path, ChangeKind::Added));
                // A copy leaves its source where it was; a rename does not.
                if x == b'R' && !orig.is_empty() {
                    v.push((orig, ChangeKind::Deleted));
                }
            }
            // Before the `A` arm on purpose: `AD` is staged-added then removed
            // from the worktree, and the row that would carry `added` is gone.
            (b'D', _) | (_, b'D') => v.push((path, ChangeKind::Deleted)),
            (b'A', _) => v.push((path, ChangeKind::Added)),
            // M, T, U and anything a future git adds: it differs, which is all
            // four classes are for.
            _ => v.push((path, ChangeKind::Modified)),
        }
    }
    v
}

/// `git diff --name-status -z <range>` → (repo-relative, kind). Fields are
/// NUL-separated, a status token then its path — except `R…`/`C…`, which are
/// followed by TWO paths (old, then new).
pub fn parse_name_status_z(out: &str) -> Vec<(String, ChangeKind)> {
    let mut it = out.split('\0').filter(|r| !r.is_empty());
    let mut v = Vec::new();
    while let Some(tok) = it.next() {
        let letter = tok.as_bytes()[0];
        match letter {
            b'R' | b'C' => {
                let old = it.next().unwrap_or("").to_string();
                let new = it.next().unwrap_or("").to_string();
                if !new.is_empty() {
                    v.push((new, ChangeKind::Added));
                }
                if letter == b'R' && !old.is_empty() {
                    v.push((old, ChangeKind::Deleted));
                }
            }
            _ => {
                // A trailing status with no path means truncated output; stop
                // rather than pairing it with the next record's status token.
                let Some(path) = it.next() else { break };
                let kind = match letter {
                    b'A' => ChangeKind::Added,
                    b'D' => ChangeKind::Deleted,
                    _ => ChangeKind::Modified,
                };
                v.push((path.to_string(), kind));
            }
        }
    }
    v
}

/// One file's diff, as the viewer's two-column renderer consumes it.
#[derive(Serialize)]
pub struct FileDiff {
    /// Echo of what was asked for ("base" | "head").
    ///
    /// Informational only — the toggle's race is handled on the frontend by the
    /// fetch effect's `alive` flag, which cleanup trips when `diffBase` changes,
    /// so a stale response is dropped before it can be stored. This field is NOT
    /// what does that; it is here so a logged or inspected response says which
    /// question it answered.
    pub against: String,
    /// The ref the base resolved to, for the header. "vs base" without saying
    /// WHICH base is a claim the reader has to take on faith, and the answer
    /// differs per repo (`origin/main`, `master`, or nothing at all).
    pub base_label: String,
    /// git's unified diff at full context. EMPTY means the file is identical to
    /// the base — deliberately distinct from a patch that parses to no rows.
    pub patch: String,
    /// There is no BEFORE for this path, so every line of it is new: either git
    /// has never seen it, or the repo has no commits yet (an unborn HEAD, where
    /// a staged file is tracked but has no committed version to diff against).
    /// No git output at all in that case — the frontend builds all-added rows
    /// from the content it already read. Spawning `git diff --no-index
    /// /dev/null` would be a second process for something we derive for free.
    pub untracked: bool,
    pub binary: bool,
    /// The patch hit the byte cap below and is cut short at a line boundary.
    pub truncated: bool,
}

/// Byte cap on a patch. Full context means the payload is roughly BOTH copies of
/// the file, so this is the pair-wise twin of `read_file`'s 1 MiB — and it
/// crosses the IPC bridge as one string.
pub const DIFF_MAX: usize = 2_000_000;

/// Cut `patch` to at most `DIFF_MAX` bytes, at a LINE boundary. Mid-line would
/// hand the parser half a line of source and it would render as real content.
pub fn trim_patch(patch: &str) -> (String, bool) {
    if patch.len() <= DIFF_MAX {
        return (patch.to_string(), false);
    }
    let mut cut = DIFF_MAX;
    while cut > 0 && !patch.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &patch[..cut];
    // No newline in the first 2 MB means one enormous line; there is no boundary
    // to fall back to, so the char boundary is the best available cut.
    let end = head.rfind('\n').map(|i| i + 1).unwrap_or(cut);
    (patch[..end].to_string(), true)
}

/// Does this patch say "binary" rather than showing lines?
pub fn patch_is_binary(patch: &str) -> bool {
    patch.lines().any(|l| l.starts_with("Binary files ") || l == "GIT binary patch")
}

/// One file's diff against its branch's base (or against HEAD).
///
/// `--unified=1000000` is the whole design: full context means ONE payload
/// carries both complete versions of the file *and* their alignment, so the
/// frontend needs no diff algorithm and can syntax-highlight each side as a
/// whole file. Hunk-sized context would mis-colour every line whose block
/// comment or string opened above the hunk.
///
/// A path outside a repo, or with no git, returns an empty diff rather than an
/// error: the tree listed the file perfectly well, and a banner over it would
/// be reporting the absence of a feature as a failure.
pub fn file_diff(ctx: &Ctx, path: String, against: Option<String>) -> Result<FileDiff, String> {
    let f = guard_under_projects(ctx, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    file_diff_for(&f, against.as_deref() == Some("head"))
}

/// The git half of `file_diff`, split out from the command so it can be tested
/// against real repos: the command itself needs an `AppHandle` for the path
/// guard, which a unit test has no way to build. Both of the states worth
/// pinning here — an unborn HEAD, and a git invocation that genuinely fails —
/// used to render as "no changes", so they get tests rather than trust.
pub fn file_diff_for(f: &Path, want_head: bool) -> Result<FileDiff, String> {
    let against = if want_head { "head" } else { "base" }.to_string();
    let none = |label: &str, untracked: bool| FileDiff {
        against: against.clone(),
        base_label: label.to_string(),
        patch: String::new(),
        untracked,
        binary: false,
        truncated: false,
    };
    let dir = f.parent().unwrap_or(f).to_string_lossy().to_string();
    let top = match git::git_out(&dir, &["rev-parse", "--show-toplevel"]) {
        Some(t) if !t.is_empty() => std::fs::canonicalize(&t).unwrap_or_else(|_| PathBuf::from(t)),
        _ => return Ok(none("", false)),
    };
    let cwd = top.to_string_lossy().to_string();
    // Relative to the repo TOP, because that is where `-C` points below. A place
    // one level down from the worktree root would otherwise ask git about a path
    // that does not exist and get an empty diff for a file that clearly differs.
    let rel = f.strip_prefix(&top).unwrap_or(f).to_string_lossy().to_string();

    if !git::git_ok(&cwd, &["ls-files", "--error-unmatch", "--", &rel]) {
        // No label: there is no ref to name, and the viewer says "new file"
        // rather than trying to phrase the absence of one.
        return Ok(none("", true));
    }
    // An unborn HEAD is the other way to have no BEFORE, and it does not look
    // like one: a `git add`ed file in a fresh repo IS tracked, so the check
    // above passes, every `merge-base` then fails, the fallback lands on `HEAD`
    // — and `git diff HEAD` exits 128 on `fatal: bad revision 'HEAD'`. That used
    // to arrive as an empty patch and render as "no changes vs HEAD" for a file
    // the tree had just tinted `added`. Every line of it is new; say so.
    if !git::has_commits(&cwd) {
        return Ok(none("", true));
    }

    // `merge-base`, not the `cand...HEAD` range syntax `changed_files` uses:
    // that range compares two COMMITS, and what the viewer needs is the base
    // against the WORKING TREE, so the uncommitted half shows up too. Same
    // candidates, same precedence, so the two always agree on which base.
    let (base_rev, base_label) = if want_head {
        ("HEAD".to_string(), "HEAD".to_string())
    } else {
        BASE_CANDS
            .iter()
            .find_map(|cand| {
                git::git_out(&cwd, &["merge-base", cand, "HEAD"])
                    .filter(|s| !s.is_empty())
                    .map(|sha| (sha, (*cand).to_string()))
            })
            // No main/master anywhere: HEAD is the only base there is, and
            // saying so beats silently diffing against something else.
            .unwrap_or_else(|| ("HEAD".to_string(), "HEAD".to_string()))
    };

    // `None` is a NON-ZERO EXIT, and it must not become an empty patch: empty is
    // documented and rendered as "identical to the base", so collapsing the two
    // turns any git failure into a confident, wrong "no changes". `git diff`
    // without `--exit-code` returns 0 whether or not there are differences, so a
    // non-zero exit here is always a real failure and belongs in a banner.
    let patch = git::git_out(
        &cwd,
        &["diff", "--no-ext-diff", "--no-color", "--unified=1000000", &base_rev, "--", &rel],
    )
    .ok_or_else(|| format!("could not diff {rel} against {base_label}"))?;
    if patch_is_binary(&patch) {
        return Ok(FileDiff { against, base_label, patch: String::new(), untracked: false, binary: true, truncated: false });
    }
    let (patch, truncated) = trim_patch(&patch);
    Ok(FileDiff { against, base_label, patch, untracked: false, binary: false, truncated })
}

/// The Docs tab's index for one place, plus the ref its staleness is measured
/// against.
///
/// `base` is NOT `Place::upstream`, and the difference is the whole reason this
/// field exists. Core measures `behind` against the project's BASE ref
/// (`project.rs`: `rev-list --left-right --count <base_ref>...HEAD`, where
/// `base_ref()` is `origin/main` when a fetch has brought one, else the local
/// base) — deliberately, because "how far from main" is the question, and
/// upstream told a different story on any branch that had merged main in. The
/// `upstream` field is informational and on a feature branch it names
/// `origin/<that branch>`. So a header reading "25 behind {upstream}" would
/// print the wrong ref on every pushed branch — a silently-wrong staleness
/// signal, in the one surface built to stop exactly that.
#[derive(Serialize)]
pub struct DocsIndex {
    /// e.g. `origin/main`. Named in the header beside `behind`, because a bare
    /// "25 behind" is the same silent error one level up.
    pub base: String,
    /// `.worktrees.toml` in THIS PLACE did not parse. The index still lists, by
    /// convention, and the pane says so — a broken config must never be a blank
    /// Docs tab, which is the "never a dead server for the whole place" rule
    /// (`ViewErrorBoundary`) applied one level up.
    pub config_error: Option<String>,
    pub entries: Vec<worktrees_core::docs::DocEntry>,
    pub truncated: bool,
}

/// Walk one place for its documentation (`worktrees_core::docs`).
///
/// `async fn` like every other handler — a sync one runs on the main thread,
/// and this reads up to 2,000 files' heads plus two `show-ref`s.
///
/// The walk itself refuses symlinks and `.worktrees`/`.git`/`node_modules`/
/// `target`/`dist`; this is the other half of that boundary, the same
/// `guard_under_projects` contract `list_dir` and `read_file` are held to. A
/// path outside every registered project is refused before anything is read.
///
/// A project that cannot be discovered is not a failure: the index still
/// lists, with an empty `base`, and the header simply says "N behind" without
/// naming a ref. Losing the ref name is survivable; losing the index is not.
///
/// ⚠ **`[docs]` is read from THIS PLACE, not from the main worktree** — the one
/// consumer of `projcfg` that does, and the divergence is deliberate.
/// `project_prefix`, materialize, `[ports]` and `[compose]` all read
/// `main_root` because they describe the PROJECT: one prefix, one port stride,
/// one compose file list, whatever branch you are standing on. `[docs]`
/// describes *content that exists on a branch*, and the Docs tab's whole
/// premise is that content differs per place.
///
/// Read main's instead and the feature defeats itself. A restructure that lands
/// `[docs] paths = ["handbook"]` on main would, on the seven of eleven places
/// that have not rebased, declare a directory their branch does not have — and
/// every one of them would show an EMPTY index while their `docs/` sat there
/// unlisted. A place whose branch predates the key gets the convention, which
/// is what its tree actually looks like. Both states correct for their branch,
/// which is the sentence this whole feature exists to make true.
///
/// It grants a branch no power it did not have: `Docs` is two `RelPath` fields
/// (Layer A already applied), the walk re-checks containment (Layer B), and the
/// place directory itself was guarded above. The worst a hostile branch can do
/// is point the listing at a different directory inside its own worktree.
pub fn list_docs(ctx: &Ctx, repo: String, root: String) -> Result<DocsIndex, String> {
    let dir = guard_under_projects(ctx, &root)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    let base = Project::discover(Path::new(&repo)).map(|p| p.base_ref()).unwrap_or_default();
    // A config that does not parse falls back to the convention and SAYS so.
    // `load` already honours WORKTREES_NO_PROJECT_CONFIG, so the audit switch
    // turns `[docs]` off with the rest of the project rung — no new code path.
    let (cfg, config_error) = match worktrees_core::projcfg::load(&dir) {
        Ok((c, _findings)) => (c, None),
        Err(e) => {
            applog("warn", &format!("list_docs root={root}: {e}"));
            (None, Some(e.to_string()))
        }
    };
    let idx = worktrees_core::docs::index_with(&dir, cfg.as_ref().and_then(|c| c.docs.as_ref()));
    Ok(DocsIndex { base, config_error, entries: idx.entries, truncated: idx.truncated })
}

/// The Plan dock tab: the place's planning-with-files summary (goal, phases,
/// progress, errors) plus its brief, resolved by the project's effective
/// planning level. `worktrees_core::plan::summarize_place` never
/// fails — an absent or unreadable plan is `source: "none"` — so the only
/// error here is the guard. Read-only: the session owns these files.
pub fn place_plan(ctx: &Ctx, root: String) -> Result<worktrees_core::plan::PlanSummary, String> {
    let dir = guard_under_projects(ctx, &root)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    Ok(worktrees_core::plan::summarize_place(&dir))
}

/// The Plan tab's "Generate plan": paste `ops::PLAN_PROMPT` into the place's
/// Claude session and leave it there for the user to send.
///
/// The app still writes no plan — the SESSION does, when the user presses
/// Enter on a prompt they can read first. That is why this is a paste and not
/// a `send-keys … Enter`, and why `paste_to_ai` has no trailing newline. The
/// text is the fixed constant; nothing from the frontend reaches the pane
/// except which session to put it in.
///
/// `session` rather than a slug for the same reason as `drop_reference`: the
/// place may be on an ADOPTED session whose name is not the canonical one, and
/// the frontend already holds the real name from `ls`.
pub fn plan_prompt(session: String, provider: Option<String>) -> Result<(), String> {
    // Addressed by the AI's pane, not by an index (`tmux::ai_pane`), and an
    // honest error when no Claude is there rather than a paste onto a shell.
    let ai_word = provider.as_deref().unwrap_or(harness::default_adapter().provider().id);
    let Some(adapter) = harness::by_id(ai_word) else { return Err("unknown agent provider".into()) };
    // pi's only modal is project trust, whose highlighted answer is Trust: a
    // paste is input, and the next Enter there trusts the repo. Look first; a
    // capture that fails is not a clear screen either. (pi folds the paste to
    // `[paste #1 N chars]` — harmless, the user still submits it.)
    if adapter.provider().id == provider::PI.id {
        match worktrees_core::pi::screen_of(&session) {
            Some(worktrees_core::pi::PiScreen::TrustModal) => {
                return Err("pi is asking whether to trust this project — answer that in pi first, then generate the plan.".into())
            }
            Some(worktrees_core::pi::PiScreen::Gone) => return Err(format!("pi is not running in {session}")),
            None => return Err(format!("could not read pi's screen in {session}")),
            _ => {}
        }
    }
    // kimi: every modal replaces its composer, and a paste into a half-typed
    // composer would glue onto the user's draft — the composer must be there
    // and empty (`kimi_paste_refusal`).
    if adapter.provider().id == provider::KIMI.id {
        let target = format!("={session}:");
        let screen = worktrees_core::kimi::capture_target(&target);
        // The opener helper types into the same composer (`send` refuses too).
        let pending = tmux::tmux(&["display-message", "-p", "-t", &target, "#{pane_current_path}"])
            .ok()
            .filter(|o| o.status.success())
            .is_some_and(|o| worktrees_core::kimi::opener_pending(String::from_utf8_lossy(&o.stdout).trim()));
        if let Some(why) = kimi_paste_refusal(screen.as_ref().map(|(c, s)| (c.as_str(), s.as_str())), pending, "this place") {
            return Err(why);
        }
    }
    tmux::paste_to_ai(&session, adapter.provider().match_word, ops::PLAN_PROMPT)?;
    applog("info", &format!("plan_prompt: pasted into {session}"));
    Ok(())
}

/// The Plan tab's "Ask (main)": paste `ops::plan_update_prompt(<slug>)` into
/// `(main)`'s agent, asking the orchestrator to update what it steers for this
/// place, and leave it there for the user to send.
///
/// The same contract as `plan_prompt` — fixed text, a paste with no Enter, an
/// honest error when no agent is there — with two differences. The SLUG is
/// ours: it is derived from `root` (guarded, and required to be a direct child
/// of the project's `.worktrees/`), so `(main)` itself has nothing to ask and
/// nothing the frontend typed reaches the prompt. And the composer being
/// pasted into is one the user is NOT looking at, so it is read first: unsent
/// text there would have the prompt glued onto it, and a modal (`waiting`)
/// would take the paste as its answer. Both refuse rather than guess.
///
/// `session` and `provider` are (main)'s, from `ls` like `plan_prompt`'s:
/// (main) may sit on an adopted session whose name is not the canonical one.
pub fn plan_ask_main(ctx: &Ctx, root: String, session: String, provider: Option<String>) -> Result<(), String> {
    let dir = guard_under_projects(ctx, &root)?;
    let project = Project::discover(&dir).map_err(|e| e.to_string())?;
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let place = canon(&dir).ok_or_else(|| format!("not a directory: {root}"))?;
    if place.parent().map(Path::to_path_buf) != canon(Path::new(&project.wt_root)) {
        return Err("only a place under .worktrees/ can ask (main) about its plan".into());
    }
    let slug = place.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let prompt = ops::plan_update_prompt(&slug)
        .ok_or_else(|| format!("refusing to put the place name {slug:?} in a prompt — it is not a plain directory name"))?;
    let ai_word = provider.as_deref().unwrap_or(harness::default_adapter().provider().id);
    let Some(adapter) = harness::by_id(ai_word) else { return Err("unknown agent provider".into()) };
    let prov = adapter.provider();
    let pane = tmux::ai_pane(&session, prov.match_word)
        .ok_or_else(|| format!("no {} running in (main)'s session {session}", prov.match_word))?;
    let plain = tmux::capture(&pane).ok_or_else(|| format!("could not read (main)'s screen in {session}"))?;
    let styled = if prov.id == provider::CLAUDE.id {
        let o = tmux::tmux(&["capture-pane", "-e", "-p", "-t", pane.as_str()]).map_err(|e| e.to_string())?;
        if !o.status.success() {
            return Err(format!("could not read (main)'s screen in {session}"));
        }
        String::from_utf8_lossy(&o.stdout).into_owned()
    } else {
        String::new()
    };
    // Keyed on the PANE being pasted into (the probe's `tmux` carries its
    // `%id`), or on the peer name pane-0 claude is launched with — never on
    // the repo-root cwd, which a claude in some other terminal shares.
    let claude_waiting = prov.id == provider::CLAUDE.id
        && worktrees_core::agent::live_probes().iter().any(|p| {
            let here = p.tmux.as_deref().and_then(worktrees_core::agent::pane_id) == Some(pane.as_str())
                || p.name.as_deref() == Some(session.as_str());
            here && worktrees_core::agent::effective_state(p) == "waiting"
        });
    let pi_screen = (prov.id == provider::PI.id).then(|| worktrees_core::pi::screen_of(&session));
    if prov.id == provider::KIMI.id && worktrees_core::kimi::opener_pending(&project.main_root) {
        return Err(KIMI_OPENER_PENDING.replace("{place}", "(main)"));
    }
    if let Some(why) = ask_main_refusal(prov.id, &plain, &styled, claude_waiting, pi_screen) {
        return Err(why);
    }
    // The pane that was read, not a second lookup that could find another.
    tmux::paste_into(&pane, &prompt)?;
    applog("info", &format!("plan_ask_main: pasted into {session} for {slug}"));
    Ok(())
}

/// Why (main)'s composer cannot take a paste right now, or `None`. Pure, so the
/// rule is testable on captured screens: `plain` is `capture-pane -p`, `styled`
/// is `-e` (claude only — its own suggestion is drawn DIM, and only the styled
/// capture tells it from typing), `pi` is pi's screen read when (main) runs pi.
pub fn ask_main_refusal(
    provider_id: &str,
    plain: &str,
    styled: &str,
    claude_waiting: bool,
    pi: Option<Option<worktrees_core::pi::PiScreen>>,
) -> Option<String> {
    const DRAFT: &str = "(main) has unsent text at its prompt — send or clear it there first, then ask again.";
    const MODAL: &str = "(main) is asking you something — answer it there first, then ask again.";
    // An input box this reader does not recognise is NOT an empty one: a
    // selector, a dialog or a new layout would take the paste as input.
    const UNREAD: &str = "couldn't read (main)'s prompt — is a menu or dialog open there?";
    if provider_id == provider::CLAUDE.id {
        if claude_waiting {
            return Some(MODAL.into());
        }
        if !worktrees_core::agent::prompt_box_on_screen(styled) {
            return Some(UNREAD.into());
        }
        if worktrees_core::agent::draft_from_screen(styled).is_some() {
            return Some(DRAFT.into());
        }
    } else if provider_id == provider::CODEX.id {
        use worktrees_core::codex::Composer;
        if worktrees_core::codex::waiting_on_screen(plain) {
            return Some(MODAL.into());
        }
        match worktrees_core::codex::composer_on_screen(plain) {
            None => return Some(UNREAD.into()),
            Some(Composer::Text(_)) => return Some(DRAFT.into()),
            Some(Composer::Empty) => {}
        }
    } else if provider_id == provider::KIMI.id {
        // `plain` is the capture of the pane being pasted into, and it is
        // running kimi (the pane was found by its command).
        return kimi_paste_refusal(Some(("kimi", plain)), false, "(main)");
    } else if provider_id == provider::PI.id {
        use worktrees_core::pi::PiScreen;
        match pi.flatten() {
            Some(PiScreen::TrustModal) => {
                return Some("pi in (main) is asking whether to trust this project — answer that there first.".into())
            }
            Some(PiScreen::Gone) => return Some("pi is not running in (main)".into()),
            None => return Some("could not read pi's screen in (main)".into()),
            _ => {}
        }
    }
    None
}

/// Why a paste into a kimi pane must not happen now, from one capture of it
/// (its pane command and screen), or `None`.
const KIMI_OPENER_PENDING: &str = "kimi in {place} is still being handed its brief — wait until it starts working, then try again.";

fn kimi_paste_refusal(capture: Option<(&str, &str)>, opener_pending: bool, place: &str) -> Option<String> {
    use worktrees_core::kimi::{read_composer, screen_from, KimiScreen};
    if opener_pending {
        return Some(KIMI_OPENER_PENDING.replace("{place}", place));
    }
    let Some((cmd, screen)) = capture else { return Some(format!("could not read kimi's screen in {place}")) };
    match screen_from(cmd, screen) {
        KimiScreen::Gone => Some(format!("kimi is not running in {place}")),
        KimiScreen::Trust => Some(format!("kimi in {place} is asking whether to trust this folder — answer that there first.")),
        KimiScreen::Modal => Some(format!("kimi in {place} is asking you something — answer it there first, then try again.")),
        KimiScreen::Other => Some(format!("couldn't read kimi's prompt in {place} — is it still starting?")),
        KimiScreen::Composer => match read_composer(screen) {
            Some(c) if !c.input.is_empty() => {
                Some(format!("kimi in {place} has unsent text at its prompt — send or clear it there first."))
            }
            _ => None,
        },
    }
}

/// The staleness facts, as the frontend already holds them in `Place`.
///
/// Passed in rather than recomputed. Every field is in the `Place` the dock is
/// rendering from at the moment the button is pressed, and re-deriving them here
/// would mean a second git fan-out per click — for numbers that would then be
/// allowed to DISAGREE with the ones on screen two inches away. `base` is the
/// exception and stays ours (`Project::base_ref`), because it is the one fact
/// `Place` does not carry and the one the header gets wrong by guessing
/// (§11.4: it is the base ref, never `upstream`).
#[derive(Deserialize)]
pub struct ViewerPlace {
    pub branch: Option<String>,
    pub behind: Option<i64>,
    pub dirty: Option<bool>,
    pub dirty_files: Option<u32>,
    pub last_commit_subject: Option<String>,
    pub last_commit_epoch: Option<i64>,
}

/// Open one place's documentation in the user's real browser.
///
/// Returns a URL for the frontend to hand to `openUrl` — the same plugin path
/// `FilesPane` already uses for an `https:` link in a document. Nothing is
/// opened from here: the app stays a control surface, and the one thing this
/// command owns is deciding whether there is a URL worth opening at all.
///
/// `async fn` like every other handler, and for a harder reason than most: it
/// binds a port, walks the place, and writes a few hundred files. On the main
/// thread that is a frozen window. It is also what puts this call inside a
/// tokio runtime, which is what `docserver::start` needs in order to spawn its
/// accept loop.
///
/// **Nothing about this runs at launch.** A missing browser bundle surfaces
/// here, on the click, as one failed invoke — the Docs tab lists and reads
/// every document with no server at all, which is what phases 1 and 2 shipped.
/// The frontend's optimism (`tmuxOk`'s shape) is the other half of that rule.
///
/// The tree it serves is DERIVED, never the repo's own files: a viewer that
/// edits what it is viewing is not a viewer, and the `click` directives that
/// make a diagram navigable have to bake in a port and a token that do not
/// exist until this function has run (§5.2's answer, "A′").
pub fn open_docs_viewer(
    ctx: &Ctx,
    resource_dir: Option<String>,
    v: &viewer::Viewer,
    repo: String,
    root: String,
    slug: String,
    path: Option<String>,
    place: ViewerPlace,
) -> Result<String, String> {
    let dir = guard_under_projects(ctx, &root)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    // The document, if one was named, is guarded on its own terms rather than
    // trusted because the place was: `path` is frontend-supplied, and the index
    // lookup below happens against a walk that has not run yet.
    let want = match path.as_deref() {
        None => None,
        Some(p) => Some(guard_under_projects(ctx, p)?.to_string_lossy().into_owned()),
    };
    let base = Project::discover(Path::new(&repo)).map(|p| p.base_ref()).unwrap_or_default();
    let (cfg, config_error) = match worktrees_core::projcfg::load(&dir) {
        Ok((c, _findings)) => (c, None),
        Err(e) => (None, Some(e.to_string())),
    };
    if let Some(e) = &config_error {
        applog("warn", &format!("open_docs_viewer root={root}: {e}"));
    }
    let docs_cfg = cfg.as_ref().and_then(|c| c.docs.as_ref()).cloned();
    // BEFORE the index walk, deliberately: the tick compares this digest against
    // a later one, and a document written between the two walks has to read as a
    // change rather than as something already derived. `viewer::Request::
    // fingerprint` has the long version.
    let fingerprint = worktrees_core::docs::fingerprint_with(&dir, docs_cfg.as_ref());
    let idx = worktrees_core::docs::index_with(&dir, docs_cfg.as_ref());

    let config_dir = ctx.config_dir.clone();
    // `resource_dir` is absent in `tauri dev` (there is no bundle), which is
    // what `WORKTREES_VIEWER_BIN` exists for.
    // Where a bundled resource lives is a host fact; the caller resolves it
    // and hands it in, exactly as `diagnostics` does.
    let resource_dir = resource_dir.map(PathBuf::from);
    let req = viewer::Request {
        root: &dir,
        slug: &slug,
        path: want.as_deref(),
        stale: worktrees_core::derive::Staleness {
            place: slug.clone(),
            branch: place.branch,
            behind: place.behind,
            base,
            dirty: place.dirty,
            dirty_files: place.dirty_files,
            last_commit_subject: place.last_commit_subject,
            last_commit_epoch: place.last_commit_epoch,
            // The transform may not read the clock; this is not the transform.
            //
            // The same instant twice, and they mean different things: the facts
            // above were captured NOW (the frontend read them off the `Place` it
            // is rendering), and this copy is being written NOW. The tick moves
            // the second one alone, which is what makes the two separable at
            // all — see `Staleness::derived_epoch`.
            now_epoch: sysclock::now_epoch(),
            derived_epoch: sysclock::now_epoch(),
        },
        docs: docs_cfg,
        fingerprint,
    };
    match viewer::open(&v, &config_dir, resource_dir.as_deref(), &idx, &req) {
        Ok(opened) => {
            applog("info", &format!("open_docs_viewer slug={slug} entries={}", idx.entries.len()));
            // An image that was referenced and deliberately not copied is a
            // broken image in the reader's browser with no other explanation
            // anywhere — the open succeeded, so there is no error to carry it.
            for n in &opened.notes {
                applog("warn", &format!("open_docs_viewer slug={slug}: {n}"));
            }
            Ok(opened.url)
        }
        Err(e) => {
            // Never swallowed: the viewer is the one part of this feature the
            // user cannot see failing anywhere else.
            applog("error", &format!("open_docs_viewer slug={slug} root={root}: {e}"));
            Err(e)
        }
    }
}

/// File contents for the viewer. Capped (default 1 MiB) and binary-guarded
/// (a NUL byte in the read slice → `binary: true`, empty content). The frontend
/// shows a "binary / open in editor" placeholder instead of garbage.
pub fn read_file(ctx: &Ctx, path: String, max_bytes: Option<u64>) -> Result<FileContent, String> {
    let f = guard_under_projects(ctx, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    // `max_bytes` is frontend-controlled: clamp it. An unclamped `cap + 1`
    // overflows on u64::MAX — panicking in a debug build, and in release
    // wrapping to 0, which would report a non-empty file as empty and NOT
    // truncated. Saturating keeps the failure mode "reads everything".
    let cap = max_bytes.unwrap_or(1_000_000).min(u64::MAX - 1);
    // Bounded read: `take(cap+1)` never allocates more than the cap even for a
    // multi-GB file the user clicks by accident (video, core dump, tarball).
    let file = std::fs::File::open(&f).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > cap;
    let slice = &bytes[..bytes.len().min(cap as usize)];
    let mtime = file_mtime_ms(&f);
    let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(bytes.len() as u64);
    if slice.contains(&0) {
        return Ok(FileContent { content: String::new(), truncated, binary: true, mtime, size });
    }
    Ok(FileContent {
        content: String::from_utf8_lossy(slice).to_string(),
        truncated,
        binary: false,
        mtime,
        size,
    })
}

/// Is `path` still a readable file inside the workspace? Answers the question
/// `files_open` restore has to ask before it reopens a remembered path.
///
/// **Returns `Ok(false)` where every other FS command returns `Err`** — that is
/// the whole point of it existing rather than the frontend calling `read_file`
/// and catching. A remembered file can be deleted, renamed, gitignored or left
/// behind by a branch switch between visits, and `FileView` routes a failed
/// read to `onError`, i.e. the app's error banner. Restoring through a command
/// that *errors* would therefore greet you with a banner for the entirely
/// ordinary act of deleting a file you once had open — which is exactly why
/// `PlacePanels` refused to remember the open file at all until this existed.
///
/// So: outside the workspace, missing, or not-a-file all collapse to `false`.
/// The `Err` arm is left in the signature for an IPC-level failure only; the
/// guard's own rejection is deliberately swallowed.
pub fn file_readable(ctx: &Ctx, path: String) -> Result<bool, String> {
    Ok(guard_under_projects(ctx, &path).map(|f| f.is_file()).unwrap_or(false))
}

/// The dock shell a terminal link came from, so its LIVE cwd can be read.
#[derive(Deserialize)]
pub struct ShellRef {
    pub repo: String,
    pub slug: String,
    pub index: u32,
}

/// The most candidates one `resolve_term_paths` will look at. A row of output
/// holds a handful; this only bounds a pathological one.
pub const TERM_PATHS_MAX: usize = 64;

/// One path as printed in a terminal → the absolute file it names, or `None`.
///
/// A link exists only for what the viewer could actually open: a REGULAR FILE,
/// after canonicalising (so `..` and symlinks are judged by where they land),
/// inside a registered project — `guard_under_projects`' own question, asked of
/// a path that is not a command argument. Everything else is no link at all,
/// not a link to Finder: the viewer cannot read it, and terminal text is the
/// one input here that no user chose, so it does not get to name arbitrary
/// files for the app to act on.
///
/// Relative paths try each base in order (the dock shell's live cwd, then the
/// place root); `~/` is the home dir; and `a/x`/`b/x` — git diff's prefixes —
/// fall back to `x` when the literal path is not a file.
pub fn resolve_term_path(raw: &str, bases: &[PathBuf], home: Option<&Path>, roots: &[PathBuf]) -> Option<String> {
    if raw.is_empty() || raw.len() > 4096 || raw.contains('\0') {
        return None;
    }
    let hit = |p: PathBuf| -> Option<String> {
        let c = std::fs::canonicalize(&p).ok()?;
        (c.is_file() && under_roots(roots, &c)).then(|| c.to_string_lossy().into_owned())
    };
    let tries = |rel: &str| -> Option<String> {
        if let Some(rest) = rel.strip_prefix("~/") {
            return hit(home?.join(rest));
        }
        if Path::new(rel).is_absolute() {
            return hit(PathBuf::from(rel));
        }
        bases.iter().find_map(|b| hit(b.join(rel)))
    };
    tries(raw).or_else(|| {
        let rest = raw.strip_prefix("a/").or_else(|| raw.strip_prefix("b/"))?;
        tries(rest)
    })
}

/// The working directory of a tmux session's active pane, or `None`.
///
/// `=name:` is tmux's EXACT session match (a bare `-t name` prefix-matches, so
/// `cdv-app` would answer for `cdv-app-x`). An unknown session prints an EMPTY
/// line and still exits 0 — measured on a private `-L` server — so empty is
/// read as "no answer" rather than trusted as a path.
pub fn tmux_pane_cwd(session: &str) -> Option<PathBuf> {
    if session.is_empty() || session.contains(['\0', '\n']) {
        return None;
    }
    let target = format!("={session}:");
    let out = worktrees_core::tmux::tmux(&["display-message", "-p", "-t", &target, "#{pane_current_path}"]).ok()?;
    if !out.status.success() {
        return None;
    }
    let cwd = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let p = PathBuf::from(cwd);
    (p.is_absolute() && p.is_dir()).then_some(p)
}

/// Which of the paths a terminal printed are files the viewer can open — the
/// answer behind ⌘-click links (`termLinkProvider` in TerminalPane.tsx). The
/// frontend batches a row's candidates into one call and caches the answers,
/// so this is a stat per path plus, for a tmux pane, ONE `display-message` per
/// call — never a spawn per mousemove.
///
/// Relative paths resolve against where the program that printed them IS: a
/// dock shell's live cwd, or the tmux pane's (`cd app && cargo test` prints
/// `src/lib.rs`, which means `app/src/lib.rs` — against the place root it is
/// either no link or, worse, the root's same-named file). The place root is
/// the fallback after either.
pub fn resolve_term_paths(
    ctx: &Ctx,
    shells: &Shells,
    root: String,
    shell: Option<ShellRef>,
    session: Option<String>,
    paths: Vec<String>,
) -> Result<Vec<Option<String>>, String> {
    let mut bases = Vec::new();
    if let Some(cwd) = session.as_deref().and_then(tmux_pane_cwd) {
        bases.push(cwd);
    }
    if let Some(sh) = shell {
        // Read live, not from `shell-cwds.json`: that file is a 15 s sample and
        // the user may have `cd`'d a moment ago. The pid is checked for life
        // first (see `live_pid` — a reaped pid can belong to a stranger).
        let pid = {
            let mut map = shells.0.lock().unwrap();
            map.get_mut(&(sh.repo, sh.slug, sh.index)).and_then(|s| live_pid(&mut *s.child))
        };
        if let Some(cwd) = pid.and_then(proc_cwd) {
            bases.push(cwd);
        }
    }
    bases.push(PathBuf::from(&root));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let roots = project_roots(ctx);
    Ok(paths
        .iter()
        .take(TERM_PATHS_MAX)
        .map(|p| resolve_term_path(p, &bases, home.as_deref(), &roots))
        .collect())
}

/// Each place's file names → its repo-relative paths, from ONE
/// `git ls-files --cached --others --exclude-standard` (an index read plus
/// the untracked walk git already does for `status`; no traversal of ours).
/// Measured at 20–35 ms on the largest registered project (3.6k files).
/// Rebuilt only when the frontend's `generation` moves — the same
/// `places:changed`/Refresh token that re-lists the Files tree — never per
/// render. Untracked files are in it on purpose: a plan names the files a
/// branch is ADDING.
#[derive(Default)]
pub struct DocNameIndex(Mutex<HashMap<String, (u64, Arc<HashMap<String, Vec<String>>>)>>);

/// `ls-files -z` output → basename → paths. Pure, for the tests.
pub fn basename_index(listing: &[u8]) -> HashMap<String, Vec<String>> {
    let mut idx: HashMap<String, Vec<String>> = HashMap::new();
    for rel in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rel = String::from_utf8_lossy(rel).into_owned();
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        idx.entry(name).or_default().push(rel);
    }
    idx
}

/// How many same-named files a bare name may offer. Past this the dropdown is
/// a search result, not a disambiguation.
pub const DOC_NAME_CHOICES_MAX: usize = 12;

/// A path found in rendered markdown, as the viewer needs it: the absolute
/// file to open, and how to name it in a tooltip or a menu.
#[derive(Serialize, Debug, PartialEq)]
pub struct DocPathHit {
    pub path: String,
    /// Relative to the place root; the absolute path when it is elsewhere.
    pub rel: String,
}

/// One candidate from a markdown document → every file it may name.
/// Empty = no link, one = a link, several = the click asks which.
///
/// A PATH is decided by `resolve_term_path` — the terminal's rules, not a
/// second resolver: a regular file, canonicalised, inside a registered
/// project, `~/`, git's `a/`/`b/`. A BARE NAME (no `/`) that is not a file
/// next to the doc or at the root is looked up in the place's name index, and
/// every candidate the index offers goes through the same resolver again, so
/// a file deleted since the index was built, or a symlink leading out, is
/// still refused.
pub fn resolve_doc_path(
    raw: &str,
    bases: &[PathBuf],
    home: Option<&Path>,
    roots: &[PathBuf],
    root: &Path,
    index: &mut dyn FnMut() -> Option<Arc<HashMap<String, Vec<String>>>>,
) -> Vec<DocPathHit> {
    let root_c = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let hit = |abs: String| {
        let rel = Path::new(&abs).strip_prefix(&root_c).map(|r| r.to_string_lossy().into_owned()).unwrap_or_else(|_| abs.clone());
        DocPathHit { path: abs, rel }
    };
    if let Some(abs) = resolve_term_path(raw, bases, home, roots) {
        return vec![hit(abs)];
    }
    if raw.contains('/') || raw.starts_with('~') || !raw.contains('.') {
        return Vec::new();
    }
    let Some(idx) = index() else { return Vec::new() };
    let only_root = [root.to_path_buf()];
    let mut out: Vec<DocPathHit> = Vec::new();
    for rel in idx.get(raw).into_iter().flatten() {
        if let Some(abs) = resolve_term_path(rel, &only_root, home, roots) {
            if !out.iter().any(|h| h.path == abs) {
                out.push(hit(abs));
            }
        }
        if out.len() >= DOC_NAME_CHOICES_MAX {
            break;
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// Which tokens in a rendered markdown document name files — the answer
/// behind its path links (`mdpaths.ts`). The terminal's `resolve_term_paths`
/// with the bases a DOCUMENT has: its own directory first (a plan that says
/// `findings.md` means the one beside it), then the place root. The frontend
/// batches a document's candidates `TERM_PATHS_MAX` at a time and caches the
/// answers per document.
pub fn resolve_doc_paths(
    ctx: &Ctx,
    names: &DocNameIndex,
    root: String,
    doc: Option<String>,
    generation: u64,
    paths: Vec<String>,
) -> Result<Vec<Vec<DocPathHit>>, String> {
    let mut bases = Vec::new();
    if let Some(dir) = doc.as_deref().and_then(|d| Path::new(d).parent()) {
        bases.push(dir.to_path_buf());
    }
    bases.push(PathBuf::from(&root));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let roots = project_roots(ctx);
    let root_p = PathBuf::from(&root);
    // Built at most once per call, and only when a bare name missed its stat.
    let mut got: Option<Option<Arc<HashMap<String, Vec<String>>>>> = None;
    let mut index = || {
        got.get_or_insert_with(|| {
            if let Some((g, idx)) = names.0.lock().unwrap().get(&root) {
                if *g == generation {
                    return Some(idx.clone());
                }
            }
            let out = git::git(&root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"]).ok()?;
            if !out.status.success() {
                return None;
            }
            let idx = Arc::new(basename_index(&out.stdout));
            names.0.lock().unwrap().insert(root.clone(), (generation, idx.clone()));
            Some(idx)
        })
        .clone()
    };
    Ok(paths
        .iter()
        .take(TERM_PATHS_MAX)
        .map(|p| resolve_doc_path(p, &bases, home.as_deref(), &roots, &root_p, &mut index))
        .collect())
}

/// Raw bytes as base64 — the viewer builds a `data:` URI from it to show an
/// image inline. Same path guard as every other FS command. The cap is smaller
/// than `read_file`'s (base64 inflates 4/3, and this crosses the IPC bridge as
/// one string): a bigger image reports `truncated` and the viewer refuses to
/// render a half-decoded file rather than showing a corrupt one.
pub fn read_file_base64(ctx: &Ctx, path: String, max_bytes: Option<u64>) -> Result<FileBlob, String> {
    let f = guard_under_projects(ctx, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    // 4 MiB: base64 inflates 4/3 and the result crosses the IPC bridge as one
    // string, so a markdown doc full of images can hold several of these at
    // once. Clamped for the same overflow reason as `read_file`.
    let cap = max_bytes.unwrap_or(4_000_000).min(u64::MAX - 1);
    let file = std::fs::File::open(&f).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > cap;
    if truncated {
        bytes.truncate(cap as usize);
    }
    Ok(FileBlob {
        b64: b64_encode(&bytes),
        size: bytes.len() as u64,
        truncated,
        mtime: file_mtime_ms(&f),
    })
}

/// Standard base64 (RFC 4648, padded). Hand-rolled: the app pulls in no base64
/// crate for what is one table and a three-byte loop.
pub fn b64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Save an edit. The file must already exist (guard canonicalizes) — the dock
/// edits files it surfaced, it doesn't create new ones. `expected_mtime` is a
/// compare-and-swap guard: if the file changed on disk since the dock read it
/// (Claude edited it in another pane), the save is refused rather than silently
/// clobbering. The write is atomic (temp file + rename) so a crash mid-save
/// can't leave a half-written file, and preserves the file's mode bits.
///
/// Returns the SAVED file's mtime, which is the `expected_mtime` for the next
/// save in the same sitting. Without it the editor would still hold the mtime
/// it read before this write, and its second save would be refused as a
/// conflict with its own first one — the guard firing on the one writer it is
/// not there to stop.
pub fn write_file(ctx: &Ctx, path: String, content: String, expected_mtime: Option<u64>) -> Result<u64, String> {
    let f = guard_under_projects(ctx, &path)?;
    if !f.is_file() {
        return Err(format!("not a file: {path}"));
    }
    if let Some(exp) = expected_mtime {
        if file_mtime_ms(&f) != exp {
            return Err("file changed on disk since you opened it — reload to see the latest".into());
        }
    }
    let dir = f.parent().ok_or("no parent directory")?;
    let base = f.file_name().and_then(|n| n.to_str()).unwrap_or("edit");
    let tmp = dir.join(format!(".{base}.wt-tmp"));
    std::fs::write(&tmp, content.as_bytes()).map_err(|e| e.to_string())?;
    if let Ok(meta) = std::fs::metadata(&f) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions()); // keep the exec bit etc.
    }
    std::fs::rename(&tmp, &f).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(file_mtime_ms(&f))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rules behind a terminal link: a FILE, inside a registered project,
    /// found from the first base that has it; `~/` and git's `a/`/`b/`.
    #[test]
    fn a_terminal_path_is_a_link_only_to_a_file_the_viewer_can_open() {
        use super::resolve_term_path as r;
        let base = std::env::temp_dir().join(format!("wt-termpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let proj = base.join("proj");
        let sub = proj.join("app");
        std::fs::create_dir_all(sub.join("src")).unwrap();
        std::fs::write(proj.join("README.md"), "x").unwrap();
        std::fs::write(sub.join("src/a.ts"), "x").unwrap();
        std::fs::write(proj.join("only-root.md"), "x").unwrap();
        std::fs::write(base.join("outside.md"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("outside.md"), proj.join("sneaky.md")).unwrap();
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap().to_string_lossy().into_owned();
        let roots = vec![std::fs::canonicalize(&proj).unwrap()];
        let bases = vec![sub.clone(), proj.clone()];
        let home = Some(proj.as_path());

        assert_eq!(r("src/a.ts", &bases, home, &roots), Some(canon(&sub.join("src/a.ts"))), "first base (a shell's cwd)");
        assert_eq!(r("only-root.md", &bases, home, &roots), Some(canon(&proj.join("only-root.md"))), "falls back to the place root");
        assert_eq!(r("../README.md", &bases, home, &roots), Some(canon(&proj.join("README.md"))));
        assert_eq!(r(&proj.join("README.md").to_string_lossy(), &bases, home, &roots), Some(canon(&proj.join("README.md"))));
        assert_eq!(r("~/README.md", &bases, home, &roots), Some(canon(&proj.join("README.md"))), "~/ is HOME");
        assert_eq!(r("b/src/a.ts", &bases, home, &roots), Some(canon(&sub.join("src/a.ts"))), "git diff's b/ prefix");
        assert_eq!(r("src", &bases, home, &roots), None, "a directory is not a link");
        assert_eq!(r("and/or", &bases, home, &roots), None, "nothing there");
        assert_eq!(r("../outside.md", &bases, home, &roots), None, "outside every project");
        assert_eq!(r(&base.join("outside.md").to_string_lossy(), &bases, home, &roots), None);
        #[cfg(unix)]
        assert_eq!(r("sneaky.md", &bases, home, &roots), None, "a symlink is judged by where it LANDS");
        assert_eq!(r("", &bases, home, &roots), None);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A markdown document's paths: the doc's own directory before the root,
    /// the terminal's rules for everything that has a `/`, and a bare name
    /// looked up in the name index — whose answers are re-checked by the same
    /// resolver, so a stale index entry or an escaping symlink is no link.
    #[test]
    fn a_markdown_path_resolves_like_a_terminal_one_and_a_bare_name_by_index() {
        use super::{basename_index, resolve_doc_path as r, DocPathHit};
        let base = std::env::temp_dir().join(format!("wt-docpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let proj = base.join("proj");
        let plan = proj.join(".planning");
        std::fs::create_dir_all(proj.join("app/src")).unwrap();
        std::fs::create_dir_all(proj.join("crates/core/src")).unwrap();
        std::fs::create_dir_all(&plan).unwrap();
        std::fs::write(proj.join("findings.md"), "root").unwrap();
        std::fs::write(plan.join("findings.md"), "beside the doc").unwrap();
        std::fs::write(proj.join("app/src/App.tsx"), "x").unwrap();
        std::fs::write(proj.join("app/src/lib.rs"), "x").unwrap();
        std::fs::write(proj.join("crates/core/src/lib.rs"), "x").unwrap();
        std::fs::write(base.join("outside.rs"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("outside.rs"), proj.join("app/src/sneaky.rs")).unwrap();
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap().to_string_lossy().into_owned();
        let roots = vec![std::fs::canonicalize(&proj).unwrap()];
        let bases = vec![plan.clone(), proj.clone()];
        let listing = b"findings.md\0.planning/findings.md\0app/src/App.tsx\0app/src/lib.rs\0crates/core/src/lib.rs\0app/src/sneaky.rs\0app/src/gone.ts\0";
        let idx = Arc::new(basename_index(listing));
        assert_eq!(idx["lib.rs"], ["app/src/lib.rs", "crates/core/src/lib.rs"]);
        let built = std::cell::Cell::new(0);
        let mut index = || { built.set(built.get() + 1); Some(idx.clone()) };
        let one = |abs: String, rel: &str| vec![DocPathHit { path: abs, rel: rel.into() }];

        assert_eq!(r("findings.md", &bases, None, &roots, &proj, &mut index), one(canon(&plan.join("findings.md")), ".planning/findings.md"), "the doc's own directory first");
        assert_eq!(r("app/src/App.tsx", &bases, None, &roots, &proj, &mut index), one(canon(&proj.join("app/src/App.tsx")), "app/src/App.tsx"), "then the place root");
        assert_eq!(r("b/app/src/App.tsx", &bases, None, &roots, &proj, &mut index).len(), 1, "git's b/ prefix, as in a terminal");
        assert_eq!(built.get(), 0, "a path that stats never touches the index");
        assert_eq!(r("App.tsx", &bases, None, &roots, &proj, &mut index), one(canon(&proj.join("app/src/App.tsx")), "app/src/App.tsx"), "a bare name, by index");
        let two = r("lib.rs", &bases, None, &roots, &proj, &mut index);
        assert_eq!(two.iter().map(|h| h.rel.as_str()).collect::<Vec<_>>(), ["app/src/lib.rs", "crates/core/src/lib.rs"], "several = the click asks");
        assert!(r("gone.ts", &bases, None, &roots, &proj, &mut index).is_empty(), "an index entry that is no longer a file");
        #[cfg(unix)]
        assert!(r("sneaky.rs", &bases, None, &roots, &proj, &mut index).is_empty(), "an index entry that leads outside");
        let before = built.get();
        assert!(r("src/nope.rs", &bases, None, &roots, &proj, &mut index).is_empty(), "a missing PATH is not a name lookup");
        assert!(r("Makefile", &bases, None, &roots, &proj, &mut index).is_empty(), "no extension, no lookup");
        assert_eq!(built.get(), before);
        assert!(r("../outside.rs", &bases, None, &roots, &proj, &mut index).is_empty(), "outside every project");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Real symlinks on a real filesystem — the whole point of the classifier is
    /// what the syscalls answer, so a fake would test nothing. Torn down at the
    /// end; a panic leaves a directory under $TMPDIR, which is what it is for.
    #[test]
    fn symlinks_report_target_shape_and_whether_they_leave_the_workspace() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("wt-symlink-{}", std::process::id()));
        let root = base.join("project");
        let outside = base.join("elsewhere");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("sub/file.txt"), b"x").unwrap();
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        symlink("sub", root.join("inside")).unwrap(); // relative, in-workspace
        symlink(root.join("sub/file.txt"), root.join("inside-file")).unwrap();
        symlink(&outside, root.join("away")).unwrap(); // absolute, out
        symlink(root.join("nope"), root.join("broken")).unwrap();
        symlink(".git", root.join("g")).unwrap(); // the name-skip, routed around
        let roots = vec![std::fs::canonicalize(&root).unwrap()];

        let (is_dir, target, block) = classify_symlink(&root.join("inside"), &roots);
        assert!(is_dir, "a link to a directory must read as a directory");
        assert_eq!(target.as_deref(), Some("sub"), "a relative link stays relative");
        assert_eq!(block, None, "target is inside the project root — followable");

        let (is_dir, _, block) = classify_symlink(&root.join("inside-file"), &roots);
        assert!(!is_dir);
        assert_eq!(block, None);

        let (is_dir, target, block) = classify_symlink(&root.join("away"), &roots);
        assert!(is_dir);
        assert!(target.unwrap().ends_with("elsewhere"));
        assert_eq!(block, Some("outside"), "outside every root — must not be followed");

        // A dangling link resolves nowhere, so it is a file for shape purposes
        // and just as inert — but for a different reason than one pointing away,
        // and the row says which.
        let (is_dir, target, block) = classify_symlink(&root.join("broken"), &roots);
        assert!(!is_dir);
        assert!(target.is_some(), "the link still says where it MEANT to point");
        assert_eq!(block, Some("missing"));

        // `list_dir` drops `.git` by name, so a link is the way around it.
        let (is_dir, _, block) = classify_symlink(&root.join("g"), &roots);
        assert!(is_dir);
        assert_eq!(block, Some("git"), "a link must not be a way back into .git");

        // An ordinary directory is never mistaken for a link out.
        assert!(under_roots(&roots, &std::fs::canonicalize(root.join("sub")).unwrap()));
        std::fs::remove_dir_all(&base).unwrap();
    }

    fn st(out: &str) -> Vec<(String, ChangeKind)> {
        parse_status_z(out)
    }

    fn ns(out: &str) -> Vec<(String, ChangeKind)> {
        parse_name_status_z(out)
    }

    fn p(s: &str, k: ChangeKind) -> (String, ChangeKind) {
        (s.to_string(), k)
    }

    /// The two calls' REAL output, captured verbatim from a scratch repo whose
    /// branch carries one of every shape at once: a commit that modified, added,
    /// deleted and renamed, then uncommitted work that modified, deleted, and
    /// left an untracked file inside a brand-new directory. Both parsers are
    /// covered above; this is the ONE case that asserts what the frontend
    /// actually receives, and it is real bytes rather than a guess at the format.
    #[test]
    fn the_union_matches_what_git_printed_for_a_branch_with_every_shape() {
        let diff = z(&[
            "D", "docs/old.md",
            "A", "src/added.rs",
            "M", "src/mod.rs",
            "R100", "src/moved-from.rs", "src/moved-to.rs",
        ]);
        let status = z(&[" M src/keep.rs", " D tools/gen.sh", "?? fresh/deep/untracked.txt"]);
        let mut map: BTreeMap<String, ChangeKind> = BTreeMap::new();
        for (k, v) in ns(&diff) {
            map.insert(k, v);
        }
        for (k, v) in st(&status) {
            map.insert(k, v);
        }
        let want: BTreeMap<String, ChangeKind> = [
            // the rename's source is gone, so the tree gets a ghost for it
            ("docs/old.md", Deleted),
            ("src/moved-from.rs", Deleted),
            ("tools/gen.sh", Deleted),
            ("src/added.rs", Added),
            ("src/moved-to.rs", Added),
            ("src/mod.rs", Modified),
            ("src/keep.rs", Modified),
            // `-uall`: the file, not the `fresh/` directory that holds it
            ("fresh/deep/untracked.txt", Untracked),
        ]
        .into_iter()
        .map(|(p, k)| (p.to_string(), k))
        .collect();
        assert_eq!(map, want);
    }

    /// The whole point of the two-call union: the working tree is applied second
    /// and wins, because it describes the disk the tree is about to list.
    #[test]
    fn working_tree_status_overrides_the_committed_one_for_the_same_path() {
        let mut map: BTreeMap<String, ChangeKind> = BTreeMap::new();
        for (k, v) in ns("A\0src/new.rs\0M\0src/App.tsx\0") {
            map.insert(k, v);
        }
        for (k, v) in st(" D src/new.rs\0") {
            map.insert(k, v);
        }
        assert_eq!(map.get("src/new.rs"), Some(&Deleted));
        assert_eq!(map.get("src/App.tsx"), Some(&Modified));
    }

    // ── change set parsers ───────────────────────────────────────────────
    // Fixtures are real `-z` output: NUL-TERMINATED records, not newlines.

    use ChangeKind::*;

    /// Records joined the way git emits them: NUL-TERMINATED, one per entry.
    /// Built from a slice rather than one `\`-continued literal — the
    /// continuation strips leading whitespace, which would silently eat the very
    /// status column (` M`, ` D`) these fixtures exist to cover.
    fn z(recs: &[&str]) -> String {
        recs.iter().map(|r| format!("{r}\0")).collect()
    }

    #[test]
    fn status_z_classifies_every_column_pair_the_tree_can_meet() {
        let out = z(&[
            " M src/App.tsx",
            "M  src/lib.rs",
            "MM Cargo.toml",
            "A  src/new.rs",
            "?? notes.md",
            " D gone.txt",
            "D  staged-gone.txt",
            " T link",
            "UU conflict.rs",
        ]);
        assert_eq!(
            st(&out),
            vec![
                p("src/App.tsx", Modified),
                p("src/lib.rs", Modified),
                p("Cargo.toml", Modified),
                p("src/new.rs", Added),
                p("notes.md", Untracked),
                p("gone.txt", Deleted),
                p("staged-gone.txt", Deleted),
                p("link", Modified),
                p("conflict.rs", Modified),
            ]
        );
    }

    /// `AD` is added to the index and then removed from the worktree. There is no
    /// row on disk to call "added", so it has to come back deleted — otherwise
    /// the tree marks a directory and shows nothing inside it.
    #[test]
    fn status_z_reports_a_staged_add_deleted_from_the_worktree_as_deleted() {
        assert_eq!(st("AD src/oops.rs\0"), vec![p("src/oops.rs", Deleted)]);
    }

    /// The original of a rename lives in the NEXT record, and it is gone from
    /// disk — the ghost row the tree draws for it depends on this pair. A COPY's
    /// source is still there, so it must not be reported.
    #[test]
    fn status_z_splits_a_rename_into_added_plus_deleted_and_leaves_a_copy_alone() {
        assert_eq!(
            st("R  new/name.rs\0old/name.rs\0"),
            vec![p("new/name.rs", Added), p("old/name.rs", Deleted)]
        );
        assert_eq!(st("C  copy.rs\0orig.rs\0"), vec![p("copy.rs", Added)]);
        // …and the extra field must be CONSUMED: a record after a rename is a
        // status again, not a path.
        assert_eq!(
            st("R  a\0b\0?? c\0"),
            vec![p("a", Added), p("b", Deleted), p("c", Untracked)]
        );
    }

    /// A filename git would C-quote in the newline format arrives raw under `-z`
    /// — including a space, which is why the path starts at byte 3 and is never
    /// split on whitespace.
    #[test]
    fn status_z_keeps_paths_with_spaces_and_non_ascii_whole() {
        assert_eq!(st("?? docs/my notes.md\0"), vec![p("docs/my notes.md", Untracked)]);
        assert_eq!(st(" M docs/éclair 🎉.md\0"), vec![p("docs/éclair 🎉.md", Modified)]);
    }

    #[test]
    fn status_z_ignores_junk_records() {
        // `!!` only appears with --ignored, but the tree must not show ignored
        // files as untracked if it ever does; a stub too short to hold a path is
        // not a record.
        assert_eq!(st("!! target/debug\0 M ok.rs\0"), vec![p("ok.rs", Modified)]);
        assert_eq!(st("?? \0"), vec![]);
        assert_eq!(st(""), vec![]);
    }

    #[test]
    fn name_status_z_pairs_each_letter_with_its_path() {
        let out = "M\0src/App.tsx\0A\0src/new.rs\0D\0old.rs\0T\0link\0";
        assert_eq!(
            ns(out),
            vec![
                p("src/App.tsx", Modified),
                p("src/new.rs", Added),
                p("old.rs", Deleted),
                p("link", Modified),
            ]
        );
    }

    /// `R100`/`C75` carry a similarity score on the token and TWO paths after it.
    /// Reading only one would pair the next record's status with a path.
    #[test]
    fn name_status_z_consumes_both_paths_of_a_rename() {
        assert_eq!(
            ns("R100\0old.rs\0new.rs\0M\0after.rs\0"),
            vec![p("new.rs", Added), p("old.rs", Deleted), p("after.rs", Modified)]
        );
        assert_eq!(ns("C75\0orig.rs\0copy.rs\0"), vec![p("copy.rs", Added)]);
    }

    #[test]
    fn name_status_z_stops_on_a_status_with_no_path() {
        assert_eq!(ns("M\0a.rs\0M"), vec![p("a.rs", Modified)]);
        assert_eq!(ns(""), vec![]);
    }

    /// "Ask (main)" pastes into a composer the user is not looking at, so the
    /// paste is refused where it would glue onto typing or answer a modal —
    /// and goes through on an empty prompt, for each provider that has a
    /// reader for its composer.
    #[test]
    fn ask_main_refuses_a_draft_or_a_modal_and_takes_an_empty_prompt() {
        use worktrees_core::pi::PiScreen;
        let empty = "─────────\n❯ \n─────────\n  ⏵⏵ auto mode on\n";
        let typed = "─────────\n❯ half a thought\n─────────\n  ⏵⏵ auto mode on\n";
        assert_eq!(ask_main_refusal("claude", empty, empty, false, None), None);
        assert!(ask_main_refusal("claude", typed, typed, false, None).unwrap().contains("unsent text"));
        assert!(ask_main_refusal("claude", empty, empty, true, None).unwrap().contains("asking you"));
        // Claude's own DIM suggestion is not typing — only the `-e` capture
        // can tell, so plain and styled differ here on purpose.
        let sugg_plain = "─────\n❯\u{a0}suggested\n─────\n";
        let sugg_styled = "─────\n\u{1b}[2m❯\u{a0}suggested\u{1b}[0m\n─────\n";
        assert_eq!(ask_main_refusal("claude", sugg_plain, sugg_styled, false, None), None);
        // No input box at all (a selector, the trust prompt) is not "empty".
        let selector = "Do you trust this folder?\n\n❯ No, exit\n  Yes, I trust this folder\n";
        assert!(ask_main_refusal("claude", selector, selector, false, None).unwrap().contains("couldn't read"));

        let c_empty = include_str!("../../../crates/worktrees-core/tests/fixtures/codex-send/empty.txt");
        let c_typed = include_str!("../../../crates/worktrees-core/tests/fixtures/codex-send/typed.txt");
        assert_eq!(ask_main_refusal("codex", c_empty, "", false, None), None);
        assert!(ask_main_refusal("codex", c_typed, "", false, None).unwrap().contains("unsent text"));
        assert!(ask_main_refusal("codex", "not a codex screen", "", false, None).unwrap().contains("couldn't read"));
        let c_modal = include_str!("../../../crates/worktrees-core/tests/fixtures/codex-send/probe-approval.txt");
        assert!(ask_main_refusal("codex", c_modal, "", false, None).unwrap().contains("asking you"));

        assert_eq!(ask_main_refusal("pi", "", "", false, Some(Some(PiScreen::Other))), None);
        assert!(ask_main_refusal("pi", "", "", false, Some(Some(PiScreen::TrustModal))).is_some());
        assert!(ask_main_refusal("pi", "", "", false, Some(None)).is_some());
    }

    const KIMI_IDLE: &str = include_str!("../../../crates/worktrees-core/tests/fixtures/kimi-screen/2.2.0/idle.txt");
    const KIMI_TYPED: &str = include_str!("../../../crates/worktrees-core/tests/fixtures/kimi-screen/2.2.0/composer-typed.txt");
    const KIMI_TRUST: &str = include_str!("../../../crates/worktrees-core/tests/fixtures/kimi-screen/2.2.0/trust.txt");
    const KIMI_APPROVAL: &str = include_str!("../../../crates/worktrees-core/tests/fixtures/kimi-screen/2.2.0/approval.txt");

    #[test]
    fn a_kimi_paste_needs_an_empty_composer_and_no_prompt_open() {
        assert_eq!(kimi_paste_refusal(Some(("kimi", KIMI_IDLE)), false, "x"), None);
        assert!(kimi_paste_refusal(Some(("kimi", KIMI_IDLE)), true, "x").unwrap().contains("handed its brief"), "two writers into one composer");
        assert!(kimi_paste_refusal(Some(("kimi", KIMI_TYPED)), false, "x").unwrap().contains("unsent text"));
        assert!(kimi_paste_refusal(Some(("kimi", KIMI_TRUST)), false, "x").unwrap().contains("trust"));
        assert!(kimi_paste_refusal(Some(("kimi", KIMI_APPROVAL)), false, "x").unwrap().contains("asking you"));
        assert!(kimi_paste_refusal(Some(("zsh", KIMI_IDLE)), false, "x").unwrap().contains("not running"));
        assert!(kimi_paste_refusal(None, false, "x").is_some());
        assert!(ask_main_refusal("kimi", KIMI_TYPED, "", false, None).unwrap().contains("unsent text"));
        assert_eq!(ask_main_refusal("kimi", KIMI_IDLE, "", false, None), None);
    }

    /// A cut patch must end on a line boundary. Half a line of source is
    /// indistinguishable from a real line once it reaches the grid — it renders
    /// as content, with a line number, and nothing says it is a fragment.
    #[test]
    fn trim_patch_cuts_on_a_line_boundary() {
        let small = "@@ -1,1 +1,1 @@\n-a\n+b\n";
        let (out, cut) = trim_patch(small);
        assert_eq!(out, small, "a patch under the cap is returned untouched");
        assert!(!cut);

        // One line per 12 bytes, so the cap lands mid-line rather than on a
        // boundary by luck — a cap that happened to fall on "\n" would pass
        // whether or not the boundary logic existed.
        let big: String = std::iter::repeat(" 0123456789\n").take(DIFF_MAX / 12 + 200).collect();
        let (out, cut) = trim_patch(&big);
        assert!(cut, "past the cap it reports truncation");
        assert!(out.len() <= DIFF_MAX, "never longer than the cap: {}", out.len());
        assert!(out.ends_with('\n'), "cut on a boundary, not mid-line");
        assert!(big.starts_with(&out), "the kept part is a prefix of the original");
    }

    /// Multi-byte characters must not be split. `is_char_boundary` is the guard;
    /// without it `&patch[..DIFF_MAX]` panics instead of returning a short patch.
    #[test]
    fn trim_patch_respects_utf8_boundaries() {
        // The fixture only tests the guard if the cap lands INSIDE a character,
        // and whether it does is pure arithmetic on the line length: the first
        // version of this test used a 7-byte line, `DIFF_MAX % 7 == 2` fell on a
        // boundary, and the test passed with the guard deleted. Hence the
        // assertion below — it fails loudly if a future DIFF_MAX makes the
        // fixture stop straddling.
        let big: String = std::iter::repeat(" …abcd\n").take(DIFF_MAX / 9 + 200).collect();
        assert!(
            !big.is_char_boundary(DIFF_MAX),
            "fixture no longer straddles a character at the cap — it would pass without the guard",
        );
        let (out, cut) = trim_patch(&big);
        assert!(cut);
        assert!(out.len() <= DIFF_MAX);
        assert!(out.ends_with('\n'), "still cut on a line boundary");
        assert!(big.starts_with(&out));
    }

    /// Real repos, because the whole question is what git exits with — a fake
    /// would be asserting my own assumptions back at me. Both cases below
    /// rendered as "no changes" before the fix, over a row the tree had lit.
    #[test]
    fn a_repo_with_no_commits_reports_every_line_as_new() {
        let base = std::env::temp_dir().join(format!("wt-unborn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let cwd = base.to_string_lossy().to_string();
        assert!(git::git_ok(&cwd, &["init", "-q"]), "git init");
        let f = base.join("f.txt");
        std::fs::write(&f, "hello\n").unwrap();
        // STAGED, so `ls-files --error-unmatch` succeeds and the untracked arm
        // above does NOT catch it — that is exactly what made this reachable.
        assert!(git::git_ok(&cwd, &["add", "f.txt"]), "git add");
        assert!(!git::has_commits(&cwd), "fixture must have an unborn HEAD");
        assert!(
            git::git_ok(&cwd, &["ls-files", "--error-unmatch", "--", "f.txt"]),
            "fixture must be TRACKED, or it proves nothing",
        );

        let d = file_diff_for(&f, false).expect("must not error");
        assert!(d.untracked, "no commits means no before: every line is new");
        assert!(d.patch.is_empty(), "the frontend builds the rows from content");
        assert!(!d.binary);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A tracked file in a normal repo still diffs — the guard above must not
    /// have swallowed the ordinary path.
    #[test]
    fn a_committed_change_still_produces_a_patch() {
        let base = std::env::temp_dir().join(format!("wt-diffok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let cwd = base.to_string_lossy().to_string();
        assert!(git::git_ok(&cwd, &["init", "-q"]));
        let f = base.join("f.txt");
        std::fs::write(&f, "one\n").unwrap();
        assert!(git::git_ok(&cwd, &["add", "-A"]));
        assert!(git::git_ok(&cwd, &["-c", "user.email=s@s", "-c", "user.name=s", "commit", "-qm", "base"]));
        std::fs::write(&f, "one\ntwo\n").unwrap();

        let d = file_diff_for(&f, true).expect("must not error");
        assert!(!d.untracked, "the file is tracked and has a committed version");
        assert!(d.patch.contains("+two"), "the working-tree change must be in the patch: {:?}", d.patch);
        assert_eq!(d.against, "head");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn binary_patches_are_recognised() {
        assert!(patch_is_binary("diff --git a/x b/x\nBinary files a/x and b/x differ\n"));
        assert!(patch_is_binary("diff --git a/x b/x\nGIT binary patch\nliteral 12\n"));
        // A source line that merely mentions the phrase is not a marker: the
        // test is on the LINE, and this one is a diff body line, so it starts
        // with a marker character.
        assert!(!patch_is_binary("@@ -1 +1 @@\n+// Binary files are skipped here\n"));
    }
}
