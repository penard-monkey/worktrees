//! `[[file]]` materialization — the plan→apply split (proposal §4 Layer B, §7, §8).
//!
//! Three functions, one code path, three commands:
//!
//! ```text
//! probe   the ONLY filesystem READ   — impure, answers "what is out there?"
//! plan    PURE                       — every rule in §4 Layer B and §7 lives here
//! apply   the ONLY filesystem WRITE  — dumb; it executes a decided plan
//! ```
//!
//! `doctor` = probe + plan + [`Plan::report`]; `relink` and `new` = probe, plan,
//! then [`apply`]. That is why `FsFacts` carries *facts* and never *verdicts*:
//! every decision is a pure function over a struct that a unit test can build
//! from literals — the same convention `tmux.rs` uses, where `PaneList::fetch`
//! is the single impure function and `session_in` is pure and fully tested.
//!
//! ⚠ Layer A (`projcfg::RelPath`) has already run on every path here — it is
//! `RelPath`'s only constructor. Layer B (this module) is the actual containment
//! boundary, and it is judged on RESOLVED paths: a config that passes every
//! string rule can still name a committed symlink to `~/.ssh/id_rsa` (§4 B3).
//!
//! Two deliberate divergences from the bash tool this ports (§11):
//!
//! - `ln -sfn` silently DESTROYS a real file at the destination. Here a regular
//!   file where a link belongs is `shadowed`: reported, never overwritten,
//!   non-zero exit, `--force` required. There is a live file on this machine
//!   that a faithful port would have deleted.
//! - `cp` was unconditional on every relink, clobbering whatever the runtime
//!   script last wrote. Here a copy is a SEED, not a mirror (§7's drift table).
//!
//! Zero recursive deletes anywhere in this module.

use std::fs;
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::diag::{Code, Finding, Report, Severity};
use crate::git;
use crate::projcfg::{Mode, ProjectConfig};
use crate::ui::Ui;

/// §4 B5. A copy source larger than this is refused rather than read: the point
/// of `[[file]]` is credentials, and an 8 MiB credential is a mistake.
pub const MAX_COPY_BYTES: u64 = 8 * 1024 * 1024;

// ── facts ────────────────────────────────────────────────────────────────────

/// Everything `plan` is allowed to know. Built by `probe` from the filesystem —
/// or, in tests, from literals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FsFacts {
    /// `canonicalize(main_root)`. Containment is judged against THIS, never
    /// against the string the caller passed (§4 B1); `None` means the main
    /// checkout itself did not resolve, and then nothing is inside it.
    pub main_real: Option<PathBuf>,
    /// One per declared entry, in config order.
    pub entries: Vec<EntryFacts>,
}

impl FsFacts {
    fn find(&self, rel: &str) -> Option<&EntryFacts> {
        self.entries.iter().find(|e| e.rel == rel)
    }
}

/// What the filesystem said about one declared entry. Facts only — "is this
/// allowed?" is `plan`'s job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryFacts {
    pub rel: String,
    /// `<main_root>/<rel>` — the path a symlink will point AT. Absolute on
    /// purpose: the cdv `sync-macs.sh` workflow depends on both machines using
    /// identical absolute targets, so these are never "improved" to relative.
    pub src: PathBuf,
    /// `<wt>/<rel>`.
    pub dst: PathBuf,
    pub src_state: SrcFacts,
    pub dst_state: DstState,
    pub dirs: DirState,
    /// `git -C <wt> check-ignore -q <rel>` (§4 B8). Not ignored ⇒ `git add -A`
    /// in that worktree would commit the credential.
    pub gitignored: bool,
}

/// The source, as seen by one `lstat`, one `canonicalize`, and one follow-stat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SrcFacts {
    /// `symlink_metadata` succeeded — something is there (§4 B2, does not follow).
    pub exists: bool,
    /// …and that something is itself a symlink. The committed-symlink vector.
    pub is_symlink: bool,
    /// `canonicalize(src.parent())` — the symlinked-ancestor catch (§4 B1).
    pub parent_real: Option<PathBuf>,
    /// `canonicalize(src)` — where the source ACTUALLY lives (§4 B3).
    pub real: Option<PathBuf>,
    /// Follow-stat says regular file (§4 B4). A FIFO source hangs `copy` forever,
    /// so nothing in `probe` OPENS a source that is not already known regular.
    pub is_regular: bool,
    pub size: u64,
    /// mtime in nanoseconds since the epoch; `None` when unreadable or pre-epoch.
    pub mtime: Option<u128>,
    /// Content hash — copies only, and only for regular files under the cap.
    pub hash: Option<u64>,
}

/// The destination, by `symlink_metadata` (never followed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DstState {
    #[default]
    Absent,
    /// `readlink` target verbatim, plus whether it currently resolves.
    Symlink { target: PathBuf, resolves: bool },
    File { mtime: Option<u128>, hash: Option<u64> },
    Dir,
    /// FIFO, socket, device — never touched.
    Other,
}

/// The destination's ancestors, walked component by component from the worktree
/// root down. §4 B6: never blind `create_dir_all`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DirState {
    /// Every ancestor already exists as a real directory.
    #[default]
    Ready,
    /// These must be created, outermost first.
    Create(Vec<PathBuf>),
    /// An ancestor exists and is not a real directory — hard error, because a
    /// symlinked ancestor we did not create is how a write escapes the worktree.
    Blocked { at: PathBuf, symlink: bool },
}

// ── the plan ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// Work `apply` will do.
    pub actions: Vec<Action>,
    /// Problems `apply` CANNOT fix. Everything here is reported verbatim by
    /// both `relink` and `doctor`.
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub rel: String,
    pub place: String,
    pub kind: ActionKind,
    pub src: PathBuf,
    pub dst: PathBuf,
    /// Ancestors to `create_dir` (never `create_dir_all`), outermost first.
    pub mkdirs: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// Nothing at the destination.
    LinkCreate,
    /// A symlink pointing somewhere else — replace it, and say so (§4 B7).
    LinkRepoint { old: PathBuf },
    /// `--force`: a real file sits where a link belongs. Moved aside, never lost.
    LinkForce { backup: PathBuf },
    /// Seed a copy: nothing at the destination.
    CopyCreate,
    /// A symlink where a copy belongs — the mode changed under an existing
    /// worktree. Replacing a link loses nothing; the content lives in main.
    CopyReplaceLink,
    /// `--force`: re-seed a drifted copy, after backing the local content up —
    /// it may be the only copy of a locally-tuned value (§7).
    CopyForce { backup: PathBuf },
}

impl Action {
    /// `doctor`'s view of pending work. An unmaterialized declared file IS the
    /// finding — it is exactly the silent failure §1.2 describes (an Android
    /// build with no FCM sender id, discovered days later on a device).
    fn finding(&self) -> Finding {
        let (code, msg) = match &self.kind {
            ActionKind::LinkCreate | ActionKind::CopyCreate => (
                Code::NotLinked,
                format!("`{}` is declared but not materialized here — run: worktrees relink {}", self.rel, self.place),
            ),
            ActionKind::LinkRepoint { old } => (
                Code::WrongMode,
                format!("`{}` links to {} instead of {}", self.rel, old.display(), self.src.display()),
            ),
            ActionKind::CopyReplaceLink => (
                Code::WrongMode,
                format!("`{}` is declared `copy` but is a symlink here", self.rel),
            ),
            ActionKind::LinkForce { .. } => (
                Code::Shadowed,
                format!("`{}` is a real file where a link belongs", self.rel),
            ),
            ActionKind::CopyForce { .. } => (
                Code::CopyStale,
                format!("`{}` differs from the source in main", self.rel),
            ),
        };
        Finding::error(code, msg).at_place(self.place.clone()).at_path(self.rel.clone())
    }
}

impl Plan {
    /// The same plan, read as a report — `doctor` never applies anything, so
    /// pending work has to become findings or the command reports "clean" on a
    /// worktree that is missing every declared file.
    pub fn report(&self) -> Report {
        let mut findings = self.findings.clone();
        findings.extend(self.actions.iter().map(Action::finding));
        Report::new(findings)
    }
}

// ── probe — the ONLY filesystem read ─────────────────────────────────────────

/// Stat everything `plan` needs, once. Nothing here decides anything; nothing
/// here writes. The one subtlety: a source is never OPENED unless the follow-stat
/// already said "regular file", because opening a FIFO blocks forever (§4 B4).
pub fn probe(cfg: &ProjectConfig, main_root: &Path, wt: &Path) -> FsFacts {
    let main_real = fs::canonicalize(main_root).ok();
    let wt_s = wt.to_string_lossy().into_owned();
    let entries = cfg
        .files
        .iter()
        .map(|e| {
            let rel = e.path.as_str();
            let src = main_root.join(rel);
            let dst = wt.join(rel);
            EntryFacts {
                rel: rel.to_string(),
                src_state: probe_src(&src, e.mode),
                dst_state: probe_dst(&dst, e.mode),
                dirs: probe_dirs(wt, rel),
                // check-ignore consults the index too, so a TRACKED path reads as
                // "not ignored" here — which is precisely the accident B8 is for.
                gitignored: git::git_ok(&wt_s, &["check-ignore", "-q", "--", rel]),
                src,
                dst,
            }
        })
        .collect();
    FsFacts { main_real, entries }
}

fn probe_src(src: &Path, mode: Mode) -> SrcFacts {
    let Ok(lst) = fs::symlink_metadata(src) else {
        return SrcFacts::default();
    };
    let mut f = SrcFacts {
        exists: true,
        is_symlink: lst.file_type().is_symlink(),
        parent_real: src.parent().and_then(|p| fs::canonicalize(p).ok()),
        real: fs::canonicalize(src).ok(),
        ..SrcFacts::default()
    };
    if let Ok(md) = fs::metadata(src) {
        f.is_regular = md.is_file();
        f.size = md.len();
        f.mtime = mtime_ns(&md);
    }
    if mode == Mode::Copy && f.is_regular && f.size <= MAX_COPY_BYTES {
        f.hash = hash_file(src);
    }
    f
}

/// `mode` only decides whether the destination's CONTENT is read: a file that
/// shadows a *link* is never compared, only refused, so there is no reason to
/// read it — and it may be arbitrarily large.
fn probe_dst(dst: &Path, mode: Mode) -> DstState {
    let Ok(lst) = fs::symlink_metadata(dst) else {
        return DstState::Absent;
    };
    let ft = lst.file_type();
    if ft.is_symlink() {
        return DstState::Symlink {
            target: fs::read_link(dst).unwrap_or_default(),
            // `exists()` follows — that is the question here: does it dangle?
            resolves: dst.exists(),
        };
    }
    if ft.is_dir() {
        return DstState::Dir;
    }
    if ft.is_file() {
        let hash = if mode == Mode::Copy { hash_file(dst) } else { None };
        return DstState::File { mtime: mtime_ns(&lst), hash };
    }
    let _ = ft.is_fifo(); // documents why `Other` exists at all
    DstState::Other
}

/// Walk `<wt>/<rel>`'s ancestors from the worktree root down. The first missing
/// component makes every deeper one a create; a component that exists but is not
/// a real directory blocks the whole entry (§4 B6).
fn probe_dirs(wt: &Path, rel: &str) -> DirState {
    let comps: Vec<&str> = rel.split('/').collect();
    if comps.len() < 2 {
        return DirState::Ready;
    }
    let mut cur = wt.to_path_buf();
    let mut create = Vec::new();
    for c in &comps[..comps.len() - 1] {
        cur = cur.join(c);
        if !create.is_empty() {
            create.push(cur.clone());
            continue;
        }
        match fs::symlink_metadata(&cur) {
            Err(_) => create.push(cur.clone()),
            Ok(md) if md.file_type().is_symlink() => {
                return DirState::Blocked { at: cur, symlink: true }
            }
            Ok(md) if md.is_dir() => {}
            Ok(_) => return DirState::Blocked { at: cur, symlink: false },
        }
    }
    if create.is_empty() {
        DirState::Ready
    } else {
        DirState::Create(create)
    }
}

fn mtime_ns(md: &fs::Metadata) -> Option<u128> {
    md.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_nanos())
}

/// FNV-1a over the file's bytes. Not cryptographic and does not need to be: the
/// question is "did this copy drift from its seed?", and both files are ours.
fn hash_file(p: &Path) -> Option<u64> {
    use io::Read;
    let mut f = fs::File::open(p).ok()?;
    // Belt for the destination side: `probe_dst` already saw a regular file, but
    // open-then-check-the-fd is the pattern §4 mandates for every read.
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_COPY_BYTES {
            return None; // over the cap: "differs" is the safe answer
        }
        for b in &buf[..n] {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Some(h)
}

// ── plan — PURE ──────────────────────────────────────────────────────────────

/// Decide, from facts alone. No filesystem, no clock, no env — the whole §4
/// Layer B / §7 rule set is in here so it can be tested from literals.
///
/// `force` is a parameter of the DECISION, not of the write: `--force` changes
/// what we intend to do, never what is on disk, and `apply` must stay dumb.
pub fn plan(cfg: &ProjectConfig, main_root: &Path, wt: &Path, f: &FsFacts, force: bool) -> Plan {
    let place = wt.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| wt.to_string_lossy().into_owned());
    let mut out = Plan::default();
    for entry in &cfg.files {
        let rel = entry.path.as_str();
        let Some(facts) = f.find(rel) else { continue };
        plan_one(&mut out, &place, main_root, rel, entry.mode, facts, f.main_real.as_deref(), force);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn plan_one(
    out: &mut Plan,
    place: &str,
    main_root: &Path,
    rel: &str,
    mode: Mode,
    e: &EntryFacts,
    main_real: Option<&Path>,
    force: bool,
) {
    let finding = |sev: Severity, code: Code, msg: String| {
        Finding::new(sev, code, msg).at_place(place.to_string()).at_path(rel.to_string())
    };

    // B8 first: a path that is not gitignored must not be materialized at all,
    // because `git add -A` in this worktree would then commit the credential.
    if !e.gitignored {
        out.findings.push(finding(
            Severity::Error,
            Code::NotGitignored,
            format!("declared file `{rel}` is not gitignored — `git add -A` in this worktree would commit it"),
        ));
        return;
    }

    // §5/§11: absent from main is a WARNING, never a silent skip. The bash
    // `[[ -f ]] || continue` is the silent-failure bug this whole feature fixes.
    if !e.src_state.exists {
        out.findings.push(finding(
            Severity::Warn,
            Code::MissingSource,
            format!("declared file `{rel}` is absent from the main checkout ({}) — nothing to link", main_root.display()),
        ));
        // A link left over from when the source DID exist now dangles, and that
        // is an error: the build reads a broken path, not a missing one.
        if let DstState::Symlink { target, resolves } = &e.dst_state {
            if !resolves {
                out.findings.push(finding(
                    Severity::Error,
                    Code::DanglingLink,
                    format!("`{rel}` here links to {} — which no longer exists", target.display()),
                ));
            }
        }
        return;
    }

    // B1 — the source's PARENT must resolve inside main. This is the only check
    // that sees a symlinked ancestor (`apps` → /etc): the path string is clean.
    match (&e.src_state.parent_real, main_real) {
        (Some(p), Some(m)) if inside(m, p) => {}
        _ => {
            out.findings.push(finding(
                Severity::Error,
                Code::UnsafePath,
                format!("declared file `{rel}` has a parent directory that resolves outside the main checkout (symlinked ancestor?) — refusing"),
            ));
            return;
        }
    }

    // B3 — the real attack. git can commit a symlink, so a hostile repo ships a
    // credential path as a symlink to ~/.ssh/id_rsa and every Layer A rule
    // passes. Cost: a legitimate `main/.env -> ~/secrets/prod.env` stops working.
    // Accepted for v1; the safe-by-default direction is unambiguous here.
    if e.src_state.is_symlink {
        match (&e.src_state.real, main_real) {
            (Some(r), Some(m)) if inside(m, r) => {}
            _ => {
                out.findings.push(finding(
                    Severity::Error,
                    Code::UnsafePath,
                    format!("declared file `{rel}` is a symlink in main that resolves outside the repo — refusing (link the real file, or point the config at it)"),
                ));
                return;
            }
        }
    }

    // B4 — regular file only. A FIFO source hangs `copy` forever; a device node
    // is worse.
    if !e.src_state.is_regular {
        out.findings.push(finding(
            Severity::Error,
            Code::UnsafePath,
            format!("declared file `{rel}` in main is not a regular file — refusing"),
        ));
        return;
    }

    // B5 — copies are read into memory, so they are capped.
    if mode == Mode::Copy && e.src_state.size > MAX_COPY_BYTES {
        out.findings.push(finding(
            Severity::Error,
            Code::UnsafePath,
            format!(
                "declared copy `{rel}` is {} bytes, over the {MAX_COPY_BYTES}-byte cap — refusing",
                e.src_state.size
            ),
        ));
        return;
    }

    // B6 — a symlinked ancestor at the DESTINATION is how a write escapes the
    // worktree. We create ancestors one component at a time or not at all.
    let mkdirs = match &e.dirs {
        DirState::Ready => Vec::new(),
        DirState::Create(v) => v.clone(),
        DirState::Blocked { at, symlink } => {
            let what = if *symlink { "a symlink" } else { "not a directory" };
            out.findings.push(finding(
                Severity::Error,
                Code::UnsafePath,
                format!("cannot place `{rel}`: {} is {what} — refusing to write through it", at.display()),
            ));
            return;
        }
    };

    let act = |kind: ActionKind| Action {
        rel: rel.to_string(),
        place: place.to_string(),
        kind,
        src: e.src.clone(),
        dst: e.dst.clone(),
        mkdirs: mkdirs.clone(),
    };

    match mode {
        // B7's table, verbatim.
        Mode::Link => match &e.dst_state {
            DstState::Absent => out.actions.push(act(ActionKind::LinkCreate)),
            DstState::Symlink { target, resolves } => {
                if target == &e.src {
                    // THE idempotence line: a correct link is a no-op, which is
                    // what makes `relink` safe to run in a loop.
                    if !resolves {
                        out.findings.push(finding(
                            Severity::Error,
                            Code::DanglingLink,
                            format!("`{rel}` links to {} — which does not resolve", target.display()),
                        ));
                    }
                } else {
                    out.actions.push(act(ActionKind::LinkRepoint { old: target.clone() }));
                }
            }
            DstState::File { .. } => {
                if force {
                    out.actions.push(act(ActionKind::LinkForce { backup: backup_path(&e.dst) }));
                } else {
                    // The §11 divergence. `ln -sfn` deletes this file, exit 0, no
                    // output, no backup — verified empirically on a real file.
                    out.findings.push(finding(
                        Severity::Error,
                        Code::Shadowed,
                        format!(
                            "a real file at {} shadows declared link `{rel}` — NOT overwriting it (pass --force to move it aside as {})",
                            e.dst.display(),
                            file_name(&backup_path(&e.dst))
                        ),
                    ));
                }
            }
            DstState::Dir => out.findings.push(finding(
                Severity::Error,
                Code::Shadowed,
                format!("{} is a directory where declared link `{rel}` belongs — refusing (ln -sfn would link INSIDE it)", e.dst.display()),
            )),
            DstState::Other => out.findings.push(finding(
                Severity::Error,
                Code::Shadowed,
                format!("{} is neither a regular file nor a symlink — refusing to touch it", e.dst.display()),
            )),
        },

        // §7's four-state drift table. A copy is a SEED, not a mirror.
        Mode::Copy => match &e.dst_state {
            DstState::Absent => out.actions.push(act(ActionKind::CopyCreate)),
            DstState::Symlink { .. } => out.actions.push(act(ActionKind::CopyReplaceLink)),
            DstState::File { mtime, hash } => {
                // HASH FIRST — a `git checkout` that only touches mtime must not
                // be able to produce a false `stale`.
                let pristine = matches!((hash, e.src_state.hash), (Some(a), Some(b)) if *a == b);
                if pristine {
                    return;
                }
                if force {
                    out.actions.push(act(ActionKind::CopyForce { backup: backup_path(&e.dst) }));
                    return;
                }
                let src_newer = matches!((e.src_state.mtime, mtime), (Some(s), Some(d)) if s > *d);
                if src_newer {
                    out.findings.push(finding(
                        Severity::Warn,
                        Code::CopyStale,
                        format!("copy `{rel}` differs from main and main is NEWER — re-seed with: worktrees relink {place} --force"),
                    ));
                } else {
                    // The expected steady state after a runtime script rewrote it.
                    // Info, and Info never affects an exit code.
                    out.findings.push(finding(
                        Severity::Info,
                        Code::CopyStale,
                        format!("copy `{rel}` differs from main (locally modified) — left alone"),
                    ));
                }
            }
            DstState::Dir | DstState::Other => out.findings.push(finding(
                Severity::Error,
                Code::Shadowed,
                format!("{} is not a regular file — refusing to write declared copy `{rel}` over it", e.dst.display()),
            )),
        },
    }
}

/// Containment: `p` is `main` itself or lives under it. Both sides are already
/// canonical, so this is a pure path comparison — no `..` can survive it.
fn inside(main: &Path, p: &Path) -> bool {
    p == main || p.starts_with(main)
}

fn backup_path(dst: &Path) -> PathBuf {
    let name = file_name(dst);
    dst.with_file_name(format!("{name}.bak"))
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

// ── apply — the ONLY filesystem write ────────────────────────────────────────

/// Execute a decided plan. No string manipulation, no re-derivation: every path
/// acted on is the one `plan` validated (§4's TOCTOU rule — "validate and act on
/// the same resolved `PathBuf`" is 90% of the mitigation).
///
/// Exit: `0` clean · `1` a write failed · `2` findings the plan refused to fix.
pub fn apply(plan: &Plan, ui: &mut dyn Ui) -> i32 {
    let mut had_error = false;
    let mut had_io_failure = false;
    for f in &plan.findings {
        match f.severity {
            Severity::Error => {
                ui.error(&f.message);
                had_error = true;
            }
            Severity::Warn => ui.warn(&f.message),
            Severity::Info => ui.info(&f.message),
        }
    }
    for a in &plan.actions {
        match run_action(a) {
            Ok(msg) => ui.info(&msg),
            Err(e) => {
                ui.error(&format!("failed to materialize `{}`: {e}", a.rel));
                had_io_failure = true;
            }
        }
    }
    if had_io_failure {
        1
    } else if had_error {
        crate::diag::EXIT_FINDINGS
    } else {
        0
    }
}

fn run_action(a: &Action) -> io::Result<String> {
    for d in &a.mkdirs {
        make_dir(d)?;
    }
    match &a.kind {
        ActionKind::LinkCreate => {
            link_atomic(&a.src, &a.dst)?;
            Ok(format!("linked {} -> {}", a.rel, a.src.display()))
        }
        ActionKind::LinkRepoint { old } => {
            link_atomic(&a.src, &a.dst)?;
            Ok(format!("relinked {} (was -> {})", a.rel, old.display()))
        }
        ActionKind::LinkForce { backup } => {
            // rename, never remove: the shadowing file may be the only copy.
            fs::rename(&a.dst, backup)?;
            link_atomic(&a.src, &a.dst)?;
            Ok(format!("linked {} (shadowing file kept as {})", a.rel, file_name(backup)))
        }
        ActionKind::CopyCreate => {
            copy_atomic(&a.src, &a.dst)?;
            Ok(format!("copied {}", a.rel))
        }
        ActionKind::CopyReplaceLink => {
            // rename(2) over a symlink replaces the LINK, never its target.
            copy_atomic(&a.src, &a.dst)?;
            Ok(format!("copied {} (replacing a symlink — mode is `copy`)", a.rel))
        }
        ActionKind::CopyForce { backup } => {
            fs::rename(&a.dst, backup)?;
            copy_atomic(&a.src, &a.dst)?;
            Ok(format!("re-copied {} (previous content kept as {})", a.rel, file_name(backup)))
        }
    }
}

/// One component, and only if it is genuinely missing. A racing `mkdir` is fine;
/// a symlink appearing where we expected to create a directory is not (§4 B6).
fn make_dir(d: &Path) -> io::Result<()> {
    match fs::create_dir(d) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let md = fs::symlink_metadata(d)?;
            if md.file_type().is_symlink() || !md.is_dir() {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not a real directory", d.display()),
                ))
            } else {
                Ok(())
            }
        }
        Err(e) => Err(e),
    }
}

/// `symlink(src, tmp)` + `rename(tmp, dst)`: atomic, and `rename` never follows
/// a symlink at the destination. Same pattern as `store.rs::write_atomic`, and
/// never `ln` — `test/helpers/common.bash` builds a PATH whitelist that a new
/// binary dependency would break.
fn link_atomic(src: &Path, dst: &Path) -> io::Result<()> {
    let tmp = tmp_beside(dst)?;
    let _ = fs::remove_file(&tmp);
    std::os::unix::fs::symlink(src, &tmp)?;
    match fs::rename(&tmp, dst) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// `File::open` FIRST, then regular-file-ness on the OPEN fd's metadata — not a
/// second lstat (§4 TOCTOU). Then tmp-in-the-same-dir + rename, as for links.
fn copy_atomic(src: &Path, dst: &Path) -> io::Result<()> {
    use io::Read;
    let mut f = fs::File::open(src)?;
    let md = f.metadata()?;
    if !md.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "source is not a regular file"));
    }
    if md.len() > MAX_COPY_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "source is over the 8 MiB copy cap"));
    }
    let mut buf = Vec::with_capacity(md.len() as usize);
    f.read_to_end(&mut buf)?;
    let tmp = tmp_beside(dst)?;
    let _ = fs::remove_file(&tmp);
    fs::write(&tmp, &buf)?;
    match fs::rename(&tmp, dst) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// A temp name in the destination's OWN directory, so `rename` stays atomic
/// (same filesystem) — `store.rs:126-133`'s rule.
fn tmp_beside(dst: &Path) -> io::Result<PathBuf> {
    let dir = dst
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent dir"))?;
    Ok(dir.join(format!(".{}.wt-tmp-{}", file_name(dst), std::process::id())))
}

// ── the CI mode (§7) ─────────────────────────────────────────────────────────

/// `doctor --config-only`: the two things checkable on a bare clone, where the
/// declared sources are gitignored and therefore ABSENT. Pure git, no worktree,
/// no filesystem state — which is the whole point.
///
/// The one place `probe`/`plan` are bypassed, deliberately: there are no facts to
/// gather about files that cannot exist in CI.
pub fn config_only(cfg: &ProjectConfig, main_root: &Path) -> Vec<Finding> {
    let root = main_root.to_string_lossy().into_owned();
    let mut out = Vec::new();
    for e in &cfg.files {
        let rel = e.path.as_str();
        if git::git_ok(&root, &["ls-files", "--error-unmatch", "--", rel]) {
            // Catches someone having committed the secret. Checked first: a
            // tracked file also reads as "not ignored", and this is the better
            // message for it.
            out.push(
                Finding::error(
                    Code::NotGitignored,
                    format!("declared file `{rel}` is TRACKED in git — the credential is committed"),
                )
                .at_path(rel.to_string()),
            );
            continue;
        }
        if !git::git_ok(&root, &["check-ignore", "-q", "--", rel]) {
            out.push(
                Finding::error(
                    Code::NotGitignored,
                    format!("declared file `{rel}` is not gitignored — `git add -A` would commit it"),
                )
                .at_path(rel.to_string()),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projcfg::{FileEntry, RelPath};

    // ── synthetic facts (no filesystem at all) ───────────────────────────────

    const MAIN: &str = "/repo";
    const WT: &str = "/repo/.worktrees/feat";

    fn cfg(entries: &[(&str, Mode)]) -> ProjectConfig {
        ProjectConfig {
            files: entries
                .iter()
                .map(|(p, m)| FileEntry { path: RelPath::parse(p).unwrap(), mode: *m })
                .collect(),
            ports: None,
            compose: None,
        }
    }

    /// A source that passes every Layer B rule: a real regular file directly
    /// inside main. Tests mutate one field at a time from here.
    fn good_src(rel: &str) -> SrcFacts {
        let src = PathBuf::from(MAIN).join(rel);
        SrcFacts {
            exists: true,
            is_symlink: false,
            parent_real: src.parent().map(Path::to_path_buf),
            real: Some(src),
            is_regular: true,
            size: 10,
            mtime: Some(1_000),
            hash: Some(7),
        }
    }

    fn facts(rel: &str, src_state: SrcFacts, dst_state: DstState) -> FsFacts {
        FsFacts {
            main_real: Some(PathBuf::from(MAIN)),
            entries: vec![EntryFacts {
                rel: rel.to_string(),
                src: PathBuf::from(MAIN).join(rel),
                dst: PathBuf::from(WT).join(rel),
                src_state,
                dst_state,
                dirs: DirState::Ready,
                gitignored: true,
            }],
        }
    }

    fn run(entries: &[(&str, Mode)], f: &FsFacts, force: bool) -> Plan {
        plan(&cfg(entries), Path::new(MAIN), Path::new(WT), f, force)
    }

    fn one(rel: &str, mode: Mode, src: SrcFacts, dst: DstState) -> Plan {
        run(&[(rel, mode)], &facts(rel, src, dst), false)
    }

    fn codes(p: &Plan) -> Vec<(Severity, Code)> {
        p.findings.iter().map(|f| (f.severity, f.code)).collect()
    }

    // ── the happy paths ──────────────────────────────────────────────────────

    #[test]
    fn an_absent_destination_is_planned_as_a_create() {
        let p = one(".env", Mode::Link, good_src(".env"), DstState::Absent);
        assert!(p.findings.is_empty(), "{:?}", p.findings);
        assert_eq!(p.actions.len(), 1);
        assert_eq!(p.actions[0].kind, ActionKind::LinkCreate);
        assert_eq!(p.actions[0].src, PathBuf::from("/repo/.env"));
        assert_eq!(p.actions[0].dst, PathBuf::from("/repo/.worktrees/feat/.env"));
    }

    #[test]
    fn a_correct_link_is_a_no_op_which_is_what_makes_relink_idempotent() {
        let dst = DstState::Symlink { target: PathBuf::from("/repo/.env"), resolves: true };
        let p = one(".env", Mode::Link, good_src(".env"), dst);
        assert!(p.actions.is_empty() && p.findings.is_empty(), "{p:?}");
    }

    #[test]
    fn a_link_pointing_elsewhere_is_repointed_and_logged() {
        let old = PathBuf::from("/somewhere/else/.env");
        let dst = DstState::Symlink { target: old.clone(), resolves: true };
        let p = one(".env", Mode::Link, good_src(".env"), dst);
        assert!(p.findings.is_empty());
        assert_eq!(p.actions[0].kind, ActionKind::LinkRepoint { old });
        // and doctor reports it rather than staying silent
        assert_eq!(p.report().exit_code(), 2);
    }

    #[test]
    fn a_link_to_the_right_place_that_no_longer_resolves_is_a_dangling_link() {
        let dst = DstState::Symlink { target: PathBuf::from("/repo/.env"), resolves: false };
        let p = one(".env", Mode::Link, good_src(".env"), dst);
        assert_eq!(codes(&p), vec![(Severity::Error, Code::DanglingLink)]);
        assert!(p.actions.is_empty());
    }

    // ── B7: the destination table ────────────────────────────────────────────

    #[test]
    fn a_regular_file_where_a_link_belongs_is_shadowed_never_overwritten() {
        // THE case. `ln -sfn` deletes this file silently; we refuse and exit 2.
        let dst = DstState::File { mtime: Some(1), hash: Some(1) };
        let p = one(".env", Mode::Link, good_src(".env"), dst.clone());
        assert_eq!(codes(&p), vec![(Severity::Error, Code::Shadowed)]);
        assert!(p.actions.is_empty(), "must not plan a write");
        assert!(p.findings[0].message.contains("NOT overwriting"), "{}", p.findings[0].message);
        assert_eq!(p.report().exit_code(), 2);

        // --force moves it aside; it is renamed, never deleted.
        let f = facts(".env", good_src(".env"), dst);
        let p = run(&[(".env", Mode::Link)], &f, true);
        assert!(p.findings.is_empty());
        assert_eq!(
            p.actions[0].kind,
            ActionKind::LinkForce { backup: PathBuf::from("/repo/.worktrees/feat/.env.bak") }
        );
    }

    #[test]
    fn a_directory_or_special_file_at_the_destination_is_a_hard_error() {
        for dst in [DstState::Dir, DstState::Other] {
            let p = one(".env", Mode::Link, good_src(".env"), dst);
            assert_eq!(codes(&p), vec![(Severity::Error, Code::Shadowed)]);
            assert!(p.actions.is_empty());
        }
        // …and --force does NOT unlock them: only the regular-file case is
        // recoverable by moving something aside.
        let f = facts(".env", good_src(".env"), DstState::Dir);
        assert!(run(&[(".env", Mode::Link)], &f, true).actions.is_empty());
    }

    // ── Layer B, from literals ───────────────────────────────────────────────

    #[test]
    fn a_source_absent_from_main_warns_rather_than_skipping_silently() {
        let p = one(".env", Mode::Link, SrcFacts::default(), DstState::Absent);
        assert_eq!(codes(&p), vec![(Severity::Warn, Code::MissingSource)]);
        assert!(p.actions.is_empty());
        // a warning alone never fails the run
        assert_eq!(p.report().exit_code(), 0);
    }

    #[test]
    fn an_absent_source_under_a_stale_link_is_also_a_dangling_link() {
        let dst = DstState::Symlink { target: PathBuf::from("/repo/.env"), resolves: false };
        let p = one(".env", Mode::Link, SrcFacts::default(), dst);
        assert_eq!(
            codes(&p),
            vec![(Severity::Warn, Code::MissingSource), (Severity::Error, Code::DanglingLink)]
        );
        assert_eq!(p.report().exit_code(), 2);
    }

    #[test]
    fn b1_a_symlinked_ancestor_in_main_is_refused() {
        // `apps` -> /etc: the STRING is clean, only the resolved parent shows it.
        let mut src = good_src("apps/.env");
        src.parent_real = Some(PathBuf::from("/etc"));
        src.real = Some(PathBuf::from("/etc/.env"));
        let p = one("apps/.env", Mode::Link, src, DstState::Absent);
        assert_eq!(codes(&p), vec![(Severity::Error, Code::UnsafePath)]);
        assert!(p.findings[0].message.contains("symlinked ancestor"));
        assert!(p.actions.is_empty());
    }

    #[test]
    fn b3_a_committed_symlink_escaping_the_repo_is_refused() {
        // The real attack: git can commit a symlink, so a hostile repo ships
        // `apps/mobile/google-services.json` as a link to ~/.ssh/id_rsa.
        let mut src = good_src("apps/mobile/google-services.json");
        src.is_symlink = true;
        src.real = Some(PathBuf::from("/home/dev/.ssh/id_rsa"));
        let p = one("apps/mobile/google-services.json", Mode::Link, src, DstState::Absent);
        assert_eq!(codes(&p), vec![(Severity::Error, Code::UnsafePath)]);
        assert!(p.actions.is_empty());

        // a symlink that stays INSIDE main is fine — that is an ordinary repo
        let mut src = good_src(".env");
        src.is_symlink = true;
        src.real = Some(PathBuf::from("/repo/config/dev.env"));
        let p = one(".env", Mode::Link, src, DstState::Absent);
        assert_eq!(p.actions.len(), 1, "{:?}", p.findings);
    }

    #[test]
    fn b4_a_non_regular_source_is_refused() {
        let mut src = good_src(".env");
        src.is_regular = false; // FIFO, dir, device…
        let p = one(".env", Mode::Link, src, DstState::Absent);
        assert_eq!(codes(&p), vec![(Severity::Error, Code::UnsafePath)]);
    }

    #[test]
    fn b5_caps_the_copy_source_size_and_only_for_copies() {
        let mut src = good_src("big.env");
        src.size = MAX_COPY_BYTES + 1;
        let p = one("big.env", Mode::Copy, src.clone(), DstState::Absent);
        assert_eq!(codes(&p), vec![(Severity::Error, Code::UnsafePath)]);
        // a LINK is never read, so its size is irrelevant
        let p = one("big.env", Mode::Link, src, DstState::Absent);
        assert!(p.findings.is_empty() && p.actions.len() == 1);
    }

    #[test]
    fn b6_missing_ancestors_become_creates_and_a_symlinked_one_is_refused() {
        let mut f = facts("apps/mobile/x.json", good_src("apps/mobile/x.json"), DstState::Absent);
        f.entries[0].dirs = DirState::Create(vec![
            PathBuf::from("/repo/.worktrees/feat/apps"),
            PathBuf::from("/repo/.worktrees/feat/apps/mobile"),
        ]);
        let p = run(&[("apps/mobile/x.json", Mode::Link)], &f, false);
        assert_eq!(p.actions[0].mkdirs.len(), 2, "outermost first, one at a time");

        f.entries[0].dirs =
            DirState::Blocked { at: PathBuf::from("/repo/.worktrees/feat/apps"), symlink: true };
        let p = run(&[("apps/mobile/x.json", Mode::Link)], &f, false);
        assert_eq!(codes(&p), vec![(Severity::Error, Code::UnsafePath)]);
        assert!(p.actions.is_empty());
    }

    #[test]
    fn b8_a_path_that_is_not_gitignored_is_an_error_and_nothing_is_written() {
        let mut f = facts(".env", good_src(".env"), DstState::Absent);
        f.entries[0].gitignored = false;
        let p = run(&[(".env", Mode::Link)], &f, false);
        assert_eq!(codes(&p), vec![(Severity::Error, Code::NotGitignored)]);
        assert!(p.actions.is_empty(), "linking it would let `git add -A` commit it");
        // …and --force does not buy past it either
        assert!(run(&[(".env", Mode::Link)], &f, true).actions.is_empty());
    }

    // ── §7: the four copy states ─────────────────────────────────────────────

    fn copy_dst(hash: u64, mtime: u128) -> DstState {
        DstState::File { mtime: Some(mtime), hash: Some(hash) }
    }

    #[test]
    fn copy_missing_is_planned_as_a_seed() {
        let p = one("x.env", Mode::Copy, good_src("x.env"), DstState::Absent);
        assert_eq!(p.actions[0].kind, ActionKind::CopyCreate);
        // doctor calls it an error: `missing` in §7's table
        assert_eq!(p.report().exit_code(), 2);
    }

    #[test]
    fn copy_dest_that_is_a_symlink_is_the_wrong_mode_and_gets_replaced() {
        let dst = DstState::Symlink { target: PathBuf::from("/repo/x.env"), resolves: true };
        let p = one("x.env", Mode::Copy, good_src("x.env"), dst);
        assert_eq!(p.actions[0].kind, ActionKind::CopyReplaceLink);
        assert_eq!(p.report().findings[0].code, Code::WrongMode);
    }

    #[test]
    fn copy_pristine_is_silent() {
        let src = good_src("x.env"); // hash 7, mtime 1000
        let p = one("x.env", Mode::Copy, src, copy_dst(7, 500));
        assert!(p.actions.is_empty() && p.findings.is_empty(), "{p:?}");
    }

    #[test]
    fn copy_drifted_is_info_and_never_affects_the_exit_code() {
        // differs, and the DEST is at least as new: the expected steady state
        // after deploy-local.sh rewrote it.
        let p = one("x.env", Mode::Copy, good_src("x.env"), copy_dst(9, 1_000));
        assert_eq!(codes(&p), vec![(Severity::Info, Code::CopyStale)]);
        assert!(p.actions.is_empty(), "relink must NOT touch it without --force");
        assert_eq!(p.report().exit_code(), 0);
    }

    #[test]
    fn copy_stale_warns_because_the_source_moved_on() {
        let p = one("x.env", Mode::Copy, good_src("x.env"), copy_dst(9, 999));
        assert_eq!(codes(&p), vec![(Severity::Warn, Code::CopyStale)]);
        assert!(p.actions.is_empty());
        assert_eq!(p.report().exit_code(), 0, "warnings are non-fatal without --strict");
    }

    #[test]
    fn copy_hash_is_compared_before_mtime_so_a_checkout_cannot_fake_staleness() {
        // Identical content, but main's mtime is far newer — exactly what
        // `git checkout` does. mtime-first would call this `stale`.
        let mut src = good_src("x.env");
        src.mtime = Some(9_999_999);
        let p = one("x.env", Mode::Copy, src, copy_dst(7, 1));
        assert!(p.findings.is_empty() && p.actions.is_empty(), "{p:?}");
    }

    #[test]
    fn copy_force_backs_the_local_content_up_before_re_seeding() {
        let f = facts("x.env", good_src("x.env"), copy_dst(9, 1_000));
        let p = run(&[("x.env", Mode::Copy)], &f, true);
        assert_eq!(
            p.actions[0].kind,
            ActionKind::CopyForce { backup: PathBuf::from("/repo/.worktrees/feat/x.env.bak") }
        );
        assert!(p.findings.is_empty());
        // a PRISTINE copy is still untouched under --force — nothing to fix
        let f = facts("x.env", good_src("x.env"), copy_dst(7, 1));
        assert!(run(&[("x.env", Mode::Copy)], &f, true).actions.is_empty());
    }

    #[test]
    fn an_unreadable_hash_on_either_side_counts_as_differing() {
        let mut src = good_src("x.env");
        src.hash = None;
        let p = one("x.env", Mode::Copy, src, copy_dst(7, 5_000));
        assert_eq!(codes(&p), vec![(Severity::Info, Code::CopyStale)]);
    }

    // ── whole-plan shape ─────────────────────────────────────────────────────

    #[test]
    fn a_plan_covers_every_declared_entry_in_config_order() {
        let entries = [(".env", Mode::Link), ("apps/x.json", Mode::Link), ("y.env", Mode::Copy)];
        let mut f = FsFacts { main_real: Some(PathBuf::from(MAIN)), entries: Vec::new() };
        for (rel, mode) in entries {
            let mut one = facts(rel, good_src(rel), DstState::Absent);
            if mode == Mode::Copy {
                one.entries[0].src_state.hash = Some(7);
            }
            f.entries.push(one.entries.remove(0));
        }
        let p = run(&entries, &f, false);
        assert_eq!(p.actions.len(), 3);
        assert_eq!(p.actions.iter().map(|a| a.rel.as_str()).collect::<Vec<_>>(), vec![".env", "apps/x.json", "y.env"]);
        assert_eq!(p.actions[0].place, "feat");
    }

    #[test]
    fn an_empty_config_plans_nothing() {
        let p = plan(&ProjectConfig::default(), Path::new(MAIN), Path::new(WT), &FsFacts::default(), false);
        assert!(p.actions.is_empty() && p.findings.is_empty());
        assert_eq!(p.report().exit_code(), 0);
    }

    // ── real filesystem: probe + apply (Layer B's resolution rules) ───────────

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let d = std::env::temp_dir().join(format!(
                "wtmat-{tag}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(d.join("main")).unwrap();
            fs::create_dir_all(d.join("wt")).unwrap();
            Tmp(d)
        }
        fn main(&self) -> PathBuf {
            self.0.join("main")
        }
        fn wt(&self) -> PathBuf {
            self.0.join("wt")
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// `probe`'s git call is the only thing a bare tmpdir cannot answer, so the
    /// filesystem tests probe and then set `gitignored` by hand — B8 is covered
    /// by its own pure test above and by the bats suite against real git.
    fn probe_ignored(cfg: &ProjectConfig, main: &Path, wt: &Path) -> FsFacts {
        let mut f = probe(cfg, main, wt);
        for e in &mut f.entries {
            e.gitignored = true;
        }
        f
    }

    fn apply_here(cfg: &ProjectConfig, main: &Path, wt: &Path, force: bool) -> (i32, Vec<String>) {
        let f = probe_ignored(cfg, main, wt);
        let p = plan(cfg, main, wt, &f, force);
        let mut ui = crate::ui::CaptureUi::default();
        let rc = apply(&p, &mut ui);
        (rc, ui.lines)
    }

    #[test]
    fn fs_link_is_created_absolute_and_relink_is_idempotent() {
        let t = Tmp::new("link");
        fs::write(t.main().join(".env"), "SECRET=1").unwrap();
        let c = cfg(&[(".env", Mode::Link)]);

        let (rc, _) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, 0);
        let dst = t.wt().join(".env");
        assert!(fs::symlink_metadata(&dst).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&dst).unwrap(), t.main().join(".env"), "absolute target, by design");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "SECRET=1");

        // second run: nothing planned at all
        let f = probe_ignored(&c, &t.main(), &t.wt());
        let p = plan(&c, &t.main(), &t.wt(), &f, false);
        assert!(p.actions.is_empty() && p.findings.is_empty(), "{p:?}");
    }

    #[test]
    fn fs_a_shadowing_regular_file_survives_and_only_force_moves_it_aside() {
        let t = Tmp::new("shadow");
        fs::write(t.main().join(".env"), "MAIN").unwrap();
        fs::write(t.wt().join(".env"), "LOCAL — the only copy").unwrap();
        let c = cfg(&[(".env", Mode::Link)]);

        let (rc, lines) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, crate::diag::EXIT_FINDINGS);
        assert_eq!(fs::read_to_string(t.wt().join(".env")).unwrap(), "LOCAL — the only copy");
        assert!(lines.iter().any(|l| l.contains("shadows declared link")), "{lines:?}");

        let (rc, _) = apply_here(&c, &t.main(), &t.wt(), true);
        assert_eq!(rc, 0);
        assert_eq!(fs::read_to_string(t.wt().join(".env.bak")).unwrap(), "LOCAL — the only copy");
        assert!(fs::symlink_metadata(t.wt().join(".env")).unwrap().file_type().is_symlink());
    }

    #[test]
    fn fs_a_symlinked_ancestor_in_main_is_refused() {
        let t = Tmp::new("ancestor");
        let outside = t.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join(".env"), "STOLEN").unwrap();
        std::os::unix::fs::symlink(&outside, t.main().join("apps")).unwrap();

        let c = cfg(&[("apps/.env", Mode::Link)]);
        let (rc, lines) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, crate::diag::EXIT_FINDINGS);
        assert!(!t.wt().join("apps").exists(), "nothing may be created");
        assert!(lines.iter().any(|l| l.contains("resolves outside the main checkout")), "{lines:?}");
    }

    #[test]
    fn fs_a_committed_symlink_escaping_the_repo_is_refused() {
        let t = Tmp::new("escape");
        let secret = t.0.join("id_rsa");
        fs::write(&secret, "PRIVATE KEY").unwrap();
        std::os::unix::fs::symlink(&secret, t.main().join(".env")).unwrap();

        let c = cfg(&[(".env", Mode::Link)]);
        let (rc, lines) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, crate::diag::EXIT_FINDINGS);
        assert!(!t.wt().join(".env").exists());
        assert!(lines.iter().any(|l| l.contains("resolves outside the repo")), "{lines:?}");
    }

    #[test]
    fn fs_a_symlinked_ancestor_in_the_worktree_is_refused() {
        let t = Tmp::new("wtancestor");
        fs::create_dir_all(t.main().join("apps")).unwrap();
        fs::write(t.main().join("apps/.env"), "SECRET").unwrap();
        let elsewhere = t.0.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, t.wt().join("apps")).unwrap();

        let c = cfg(&[("apps/.env", Mode::Link)]);
        let (rc, lines) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, crate::diag::EXIT_FINDINGS);
        assert!(!elsewhere.join(".env").exists(), "must not write THROUGH the link");
        assert!(lines.iter().any(|l| l.contains("refusing to write through it")), "{lines:?}");
    }

    #[test]
    fn fs_a_fifo_source_is_refused_and_never_opened() {
        let t = Tmp::new("fifo");
        let fifo = t.main().join(".env");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !made {
            return; // no mkfifo on this box — the pure B4 test still covers the rule
        }
        // If probe or apply ever opened this, the test would hang instead of fail.
        let c = cfg(&[(".env", Mode::Copy)]);
        let (rc, lines) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, crate::diag::EXIT_FINDINGS);
        assert!(lines.iter().any(|l| l.contains("not a regular file")), "{lines:?}");
    }

    #[test]
    fn fs_nested_dirs_are_created_one_component_at_a_time() {
        let t = Tmp::new("mkdirs");
        fs::create_dir_all(t.main().join("apps/mobile")).unwrap();
        fs::write(t.main().join("apps/mobile/google-services.json"), "{}").unwrap();

        let c = cfg(&[("apps/mobile/google-services.json", Mode::Link)]);
        let (rc, _) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, 0);
        assert!(t.wt().join("apps/mobile").is_dir());
        assert!(fs::symlink_metadata(t.wt().join("apps/mobile/google-services.json"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn fs_the_copy_drift_states_over_real_files() {
        let t = Tmp::new("copy");
        fs::write(t.main().join("x.env"), "SEED").unwrap();
        let c = cfg(&[("x.env", Mode::Copy)]);

        // missing → seeded (a real file, NOT a link)
        let (rc, _) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, 0);
        let dst = t.wt().join("x.env");
        assert!(fs::symlink_metadata(&dst).unwrap().is_file());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "SEED");

        // pristine → nothing planned
        let f = probe_ignored(&c, &t.main(), &t.wt());
        assert!(plan(&c, &t.main(), &t.wt(), &f, false).actions.is_empty());

        // drifted → left alone, and relink does not clobber it
        fs::write(&dst, "REWRITTEN BY deploy-local.sh").unwrap();
        let (rc, lines) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, 0, "drift is Info — it must never fail a run");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "REWRITTEN BY deploy-local.sh");
        assert!(lines.iter().any(|l| l.contains("locally modified")), "{lines:?}");

        // --force → re-seeded, with the local content kept alongside
        let (rc, _) = apply_here(&c, &t.main(), &t.wt(), true);
        assert_eq!(rc, 0);
        assert_eq!(fs::read_to_string(&dst).unwrap(), "SEED");
        assert_eq!(fs::read_to_string(t.wt().join("x.env.bak")).unwrap(), "REWRITTEN BY deploy-local.sh");
    }

    #[test]
    fn fs_a_source_absent_from_main_warns_and_writes_nothing() {
        let t = Tmp::new("absent");
        let c = cfg(&[(".env", Mode::Link)]);
        let (rc, lines) = apply_here(&c, &t.main(), &t.wt(), false);
        assert_eq!(rc, 0, "a warning does not fail the run");
        assert!(!t.wt().join(".env").exists());
        assert!(lines.iter().any(|l| l.contains("absent from the main checkout")), "{lines:?}");
    }
}
