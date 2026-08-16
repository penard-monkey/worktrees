//! `worktrees sync` — courier sync of a project between machines through a
//! mounted "hub" (an SSD, or any mounted path). Port of `~/bin/sync-macs`.
//!
//! rsync is SHELLED OUT, never a Rust rsync crate — the same rationale as
//! git.rs: the bats fake-PATH shim has to intercept the compiled binary, and a
//! suite that reaches the real rsync is a suite that can `--delete` a
//! developer's files.
//!
//! ADR 0001 extends to media. The hub manifest arrives on removable storage, so
//! it is DATA: `deny_unknown_fields` turns any command-shaped key into a hard
//! parse error, and the one free string it carries (`hint`) is printed, never
//! executed. What DOES run on `pull --install` comes from the user's own
//! `~/.config/worktrees/config.toml`, the same tier as `ai_cmd`/`post_create`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::{self, SyncCfg};
use crate::diag::{Code, Finding};
use crate::git;
use crate::project::Project;
use crate::sysclock;
use crate::ui::{CaptureUi, Ui};

const SYNC_USAGE: &str = "\
usage: worktrees sync push   [--hub PATH] [--dry-run|-n] [--yes|-y] [--with-sessions] [--json]
       worktrees sync pull   [name] [--hub PATH] [--dry-run|-n] [--yes|-y] [--with-sessions] [--install] [--json]
       worktrees sync status [--hub PATH] [--json]";

/// The hub-side manifest a push leaves behind, so the other machine can adopt
/// the project without registering it first.
pub const MANIFEST: &str = ".worktrees-sync.toml";
/// Backups + anything else sync owns inside a synced tree. Self-excluded.
const SYNC_DIR: &str = ".worktrees-sync";
/// Claude transcripts on the hub (`--with-sessions`).
const SESSIONS_DIR: &str = ".claude-sessions";
const SCHEMA: u32 = 1;
/// How many deletion paths the preview prints before "… and N more".
const DELETE_SAMPLE: usize = 15;

/// Rebuildable bulk — ported verbatim from the script. `.git` is deliberately
/// NOT here: ferrying gitignored state and `.git` itself is the whole point.
const EXCLUDES: &[&str] = &[
    "node_modules/",
    ".pnp",
    ".pnp.cjs",
    ".next/",
    ".next-e2e/",
    ".turbo/",
    ".expo/",
    ".expo-shared/",
    "dist/",
    "build/",
    "out/",
    "target/",
    "coverage/",
    ".nyc_output/",
    ".playwright-mcp/",
    "*.tsbuildinfo",
    "tsconfig.tsbuildinfo",
    "*.log",
    ".DS_Store",
    ".gradle/",
    ".cxx/",
    "Pods/",
    "DerivedData/",
    ".worktrees-sync/",
    "*.tar.gz",
];

/// Volumes that are never the hub: the system disk, Time Machine's mounts, and
/// the SMB share that is always up.
const HUB_DENYLIST: &[&str] =
    &["Macintosh HD", ".timemachine", "com.apple.TimeMachine.localsnapshots", "davidpena"];

// ── rsync discovery ──────────────────────────────────────────────────────────

pub struct Rsync {
    pub path: String,
    /// rsync 3.x/4.x (Homebrew). The system openrsync emits neither
    /// `--info=progress2` nor the "hiding … because of pattern" report.
    pub v3: bool,
}

/// Pure form: candidates paired with their `--version` stdout (`None` = the
/// binary did not run). The first v3/v4 wins outright; otherwise the first that
/// ran at all is the fallback.
fn pick_rsync_from(cands: &[(String, Option<String>)]) -> Option<Rsync> {
    let mut fallback: Option<String> = None;
    for (path, ver) in cands {
        let Some(v) = ver else { continue };
        if v.contains("version 3.") || v.contains("version 4.") {
            return Some(Rsync { path: path.clone(), v3: true });
        }
        if fallback.is_none() {
            fallback = Some(path.clone());
        }
    }
    fallback.map(|path| Rsync { path, v3: false })
}

/// Captured to a buffer, never through a pipe: the bash original SIGPIPEd rsync
/// that way and misreported the version.
fn rsync_version(path: &str) -> Option<String> {
    let o = Command::new(path).arg("--version").output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn find_rsync() -> Result<Rsync, String> {
    let mut cands: Vec<String> = Vec::new();
    // `$WORKTREES_RSYNC` pins the binary. The bats suite points it at its fake
    // shim, because /opt/homebrew/bin/rsync EXISTS on a developer's Mac and is
    // v3 — it would win outright over a PATH shim, and the suite would then run
    // `--delete` against real directories.
    if let Ok(p) = std::env::var("WORKTREES_RSYNC") {
        if !p.is_empty() {
            cands.push(p);
        }
    }
    if cands.is_empty() {
        for p in ["/opt/homebrew/bin/rsync", "/usr/local/bin/rsync"] {
            if Path::new(p).exists() {
                cands.push(p.to_string());
            }
        }
        // Bare name — resolved by PATH at CALL time, so a shim directory wins.
        cands.push("rsync".to_string());
    }
    let probed: Vec<(String, Option<String>)> =
        cands.into_iter().map(|c| { let v = rsync_version(&c); (c, v) }).collect();
    pick_rsync_from(&probed)
        .ok_or_else(|| "no rsync found (install rsync, or set $WORKTREES_RSYNC)".to_string())
}

// ── hub resolution ───────────────────────────────────────────────────────────

/// Pure: `(basename, is_symlink, path)` → the volumes that could be a hub.
fn hub_candidates_from(vols: &[(String, bool, PathBuf)]) -> Vec<PathBuf> {
    vols.iter()
        .filter(|(base, link, _)| !*link && !HUB_DENYLIST.contains(&base.as_str()))
        .map(|(_, _, p)| p.clone())
        .collect()
}

fn volumes() -> Vec<(String, bool, PathBuf)> {
    let mut out: Vec<(String, bool, PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir("/Volumes") else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue; // follows the link, like `[ -d ]`
        }
        let link =
            std::fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        out.push((e.file_name().to_string_lossy().into_owned(), link, p));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// flag > `$WORKTREES_SYNC_HUB` > `[sync] hub` > a single mounted volume.
fn resolve_hub(
    flag: Option<&str>,
    cfg: &SyncCfg,
    ui: &mut dyn Ui,
    quiet: bool,
) -> Result<PathBuf, String> {
    if let Some(f) = flag.filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(f));
    }
    if let Ok(v) = std::env::var("WORKTREES_SYNC_HUB") {
        if !v.is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    if let Some(h) = cfg.hub.as_deref().filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(h));
    }
    let cands = hub_candidates_from(&volumes());
    match cands.len() {
        1 => {
            if !quiet {
                ui.info(&format!("auto-detected SSD hub: {}", cands[0].display()));
            }
            Ok(cands[0].clone())
        }
        0 => Err("no external volume mounted; plug in the SSD or pass --hub PATH".to_string()),
        _ => {
            let list =
                cands.iter().map(|p| format!("  {}", p.display())).collect::<Vec<_>>().join("\n");
            Err(format!("multiple external volumes:\n{list}\nambiguous — pass --hub PATH"))
        }
    }
}

// ── manifest (data only — ADR 0001) ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub name: String,
    /// ABSOLUTE project root on the machine that pushed. Data: writing to it is
    /// gated by the adoption confirm.
    pub local_root: String,
    #[serde(default)]
    pub pushed_at: i64,
    #[serde(default)]
    pub host: String,
    /// Human-readable rebuild note. PRINTED, NEVER EXECUTED.
    #[serde(default)]
    pub hint: String,
}

/// Strict: an unknown key is a hard error, which is what keeps a command-shaped
/// key (`install_cmd = "…"`) from ever being read as configuration.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let m: Manifest = toml::from_str(text).map_err(|e| e.to_string())?;
    if m.schema != SCHEMA {
        return Err(format!(
            "manifest schema {} is not supported (this CLI reads {SCHEMA}) — update worktrees",
            m.schema
        ));
    }
    if m.name.is_empty() || m.local_root.is_empty() {
        return Err("manifest is missing name/local_root".to_string());
    }
    Ok(m)
}

/// Hand-templated because the `toml` dependency is PARSE-only (see the note in
/// worktrees-core/Cargo.toml) — there is no serializer to call.
fn render_manifest(m: &Manifest) -> String {
    format!(
        "# written by `worktrees sync push` — DATA ONLY. A manifest that rode in on\n\
         # removable media may never supply argv (ADR 0001); `hint` is printed, not run.\n\
         schema = {}\nname = {}\nlocal_root = {}\npushed_at = {}\nhost = {}\nhint = {}\n",
        m.schema,
        tstr(&m.name),
        tstr(&m.local_root),
        m.pushed_at,
        tstr(&m.host),
        tstr(&m.hint),
    )
}

/// TOML basic string.
fn tstr(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_manifest(&text).map_err(|e| format!("{}: {e}", path.display()))
}

// ── guard A: this tree is a hub copy ─────────────────────────────────────────

/// The manifest that proves `root` is a tree which ARRIVED on a hub —
/// `<hub>/<name>/<name>`, with the push manifest sitting in its parent.
///
/// A hub copy is a mirror of another machine: its `.git` registers worktrees at
/// that machine's absolute paths, so `worktree prune` inside it can unregister
/// the wrong repo's worktrees (the source script's worst documented gotcha), and
/// every other mutating op writes into a tree the next `pull` will overwrite.
///
/// The fast path is one failed read: a project whose parent holds no manifest is
/// out before anything is parsed. All three conditions have to hold — the file
/// parses, it is ABOUT this directory, and it says the project lives somewhere
/// ELSE. The last one is not paranoia: without it a tree standing at its own
/// declared native path would be refused with advice to go where it already is.
pub fn hub_copy_of(root: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(root.parent()?.join(MANIFEST)).ok()?;
    let m = parse_manifest(&text).ok()?;
    (m.name == basename(root) && !same_dir(&m.local_root, root)).then_some(m)
}

fn same_dir(a: &str, b: &Path) -> bool {
    a.trim_end_matches('/') == b.to_string_lossy().trim_end_matches('/')
}

fn pushed_from(m: &Manifest) -> &str {
    if m.host.is_empty() {
        "another machine"
    } else {
        &m.host
    }
}

/// What a mutating command prints inside a hub copy: what this tree is, and the
/// one way out — the path where the project natively lives.
pub fn hub_copy_refusal(root: &Path) -> Option<String> {
    hub_copy_of(root).map(|m| {
        format!(
            "this is a hub copy (pushed from {}) — run worktrees at the native path: {}",
            pushed_from(&m),
            m.local_root
        )
    })
}

/// `doctor`'s half of the same guard. `Error`, not `Warn`: this is not drift the
/// tool routes around — every mutating command refuses here, so a clean report
/// would be telling the user the tree is fine to work in.
pub fn hub_copy_finding(root: &Path) -> Option<Finding> {
    hub_copy_of(root).map(|m| {
        Finding::error(
            Code::HubCopy,
            format!(
                "this checkout is a hub copy (pushed from {}) — mutating commands are refused here; the project lives at {}",
                pushed_from(&m),
                m.local_root
            ),
        )
    })
}

// ── guard B: something is living in the destination ──────────────────────────

/// Live tmux sessions in the tree at `root` — the destination of a pull.
///
/// Resolved through `ops::place_session`, the same lookup `close` uses
/// (canonical `<prefix>-<slug>` first, then adoption by pane cwd), so every name
/// printed here is one the user can actually close by that name.
///
/// A destination that does not exist yet, or is not a git repo, has no sessions:
/// adoption onto a machine that has never seen the project passes silently.
fn live_sessions_in(root: &Path) -> Vec<String> {
    let Ok(p) = Project::discover(root) else { return Vec::new() };
    let mut slugs = vec!["(main)".to_string()];
    if let Ok(rd) = std::fs::read_dir(p.wt_root_dir()) {
        let mut wt: Vec<String> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        wt.sort();
        slugs.append(&mut wt);
    }
    let mut out: Vec<String> = Vec::new();
    for slug in slugs {
        // One session can hold panes in two places; name it once.
        if let Some(s) = crate::ops::place_session(&p, &slug) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Every project the hub has a readable manifest for, name-sorted.
fn hub_manifests(hub: &Path) -> Vec<(String, Manifest)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(hub) else { return out };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for d in dirs {
        if let Ok(text) = std::fs::read_to_string(d.join(MANIFEST)) {
            if let Ok(m) = parse_manifest(&text) {
                out.push((basename(&d), m));
            }
        }
    }
    out
}

// ── plan (preview) ───────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct SyncPlan {
    pub sends: u32,
    pub deletes: u32,
    /// Capped at `DELETE_SAMPLE`; the caller prints "… and N more".
    pub delete_paths: Vec<String>,
}

/// `rsync -ani` itemized output. A deletion line starts `*deleting`; a change
/// line's first two bytes are the itemize flags (`^[<>ch.][fdLDS]`).
pub fn parse_itemized(out: &str) -> SyncPlan {
    let mut p = SyncPlan::default();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("*deleting") {
            p.deletes += 1;
            if p.delete_paths.len() < DELETE_SAMPLE {
                p.delete_paths.push(rest.trim().to_string());
            }
            continue;
        }
        let b = line.as_bytes();
        if b.len() >= 2
            && matches!(b[0], b'<' | b'>' | b'c' | b'h' | b'.')
            && matches!(b[1], b'f' | b'd' | b'L' | b'D' | b'S')
        {
            p.sends += 1;
        }
    }
    p
}

/// The `-vv` scan's "hiding <what> … because of pattern <pat>" lines →
/// pattern→count, most-hit first.
pub fn parse_hiding(out: &str) -> Vec<(String, u32)> {
    const MARK: &str = " because of pattern ";
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for line in out.lines() {
        let l = line.strip_prefix("[sender] ").unwrap_or(line);
        if !(l.starts_with("hiding directory ") || l.starts_with("hiding file ")) {
            continue;
        }
        // LAST occurrence, like the sed's greedy `.*`: a path that itself
        // contains the phrase must not truncate the pattern.
        let Some(i) = l.rfind(MARK) else { continue };
        let pat = l[i + MARK.len()..].trim();
        if pat.is_empty() {
            continue;
        }
        *counts.entry(pat.to_string()).or_insert(0) += 1;
    }
    let mut v: Vec<(String, u32)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

// ── streaming progress ───────────────────────────────────────────────────────
// What `rsync --info=progress2,name1` says while it works, parsed into something
// a progress bar can render. Two shapes come down stdout:
//
//   src/App.tsx                                        ← name1: the file moving
//   \r    1,048,576  10%    1.00MB/s  0:00:01 (xfr#12, to-chk=900/1000)
//
// progress2 REWRITES its single line, and it emits the `\r` BEFORE the line
// rather than after it. Two consequences the parser is built around: a reader
// that splits on `\n` alone sees one enormous line at EOF (i.e. no progress at
// all, which is exactly the frozen "Pushing…" this phase exists to fix), and the
// LAST line drawn has no terminator — it is flushed by `ProgressReader::finish`.

/// One sample of a running transfer. Every field is optional because rsync
/// answers in fragments: a name line carries no percentage, and a progress line
/// carries no name.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct SyncProgress {
    /// Overall completion, 0–100. `None` until rsync has said anything (and for
    /// the whole run on openrsync, which cannot report it).
    pub percent: Option<f32>,
    /// As rsync printed it — `"12.34MB/s"`. Not re-derived: rsync's own average
    /// is the number the user compares against the last transfer.
    pub rate: Option<String>,
    /// The most recent file name (`name1`).
    pub current: Option<String>,
}

/// What one stdout fragment turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressFragment {
    Stats { percent: Option<f32>, rate: Option<String> },
    Name(String),
}

/// Lines rsync prints AROUND the transfer. Without this filter each one would
/// be taken for a file name (name1's contract is "anything that is not the
/// progress line is a name"), and the modal would finish by reporting that it
/// was copying a file called `sent 4,096 bytes  received 89 bytes`.
fn is_trailer(l: &str) -> bool {
    (l.starts_with("sent ") && l.contains(" bytes"))
        || l.starts_with("total size is ")
        || l.ends_with("incremental file list")
        || l.starts_with("created directory ")
}

/// `"1,048,576"` → 1048576. Grouping commas are rsync's, not a locale's.
fn comma_int(t: &str) -> Option<u64> {
    let s: String = t.chars().filter(|c| *c != ',').collect();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `"56%"` → 56.0.
fn pct_token(t: &str) -> Option<f32> {
    t.strip_suffix('%')?.parse::<f32>().ok()
}

/// `"(xfr#12,"` / `"to-chk=900/1000)"` → the same percentage from the other end:
/// files left over files total. The fallback for a progress line whose `%` token
/// is missing or unparseable.
fn chk_pct(t: &str) -> Option<f32> {
    let v = t.trim_end_matches(')').trim_end_matches(',');
    let rest = v.strip_prefix("to-chk=").or_else(|| v.strip_prefix("ir-chk="))?;
    let (left, total) = rest.split_once('/')?;
    let (left, total) = (left.parse::<f64>().ok()?, total.parse::<f64>().ok()?);
    if total <= 0.0 || left > total {
        return None;
    }
    Some((((total - left) / total) * 100.0) as f32)
}

/// One fragment (already split off at a `\r` or `\n`) → what it says, or `None`
/// for blank lines and rsync's own trailers.
pub fn parse_progress_fragment(frag: &str) -> Option<ProgressFragment> {
    let l = frag.trim();
    if l.is_empty() || is_trailer(l) {
        return None;
    }
    let toks: Vec<&str> = l.split_whitespace().collect();
    let pct = toks.iter().find_map(|t| pct_token(t));
    let chk = toks.iter().find_map(|t| chk_pct(t));
    // A progress line always OPENS with the byte count. Requiring that as well
    // as a percentage keeps a file literally named `90% done` out of the bar.
    if comma_int(toks[0]).is_some() && (pct.is_some() || chk.is_some()) {
        let rate = toks.iter().find(|t| t.contains("/s")).map(|t| (*t).to_string());
        return Some(ProgressFragment::Stats { percent: pct.or(chk), rate });
    }
    Some(ProgressFragment::Name(l.to_string()))
}

/// A stream with no separator in it at all must not grow a buffer forever.
/// 8K is orders of magnitude past any line rsync draws.
const PARTIAL_CAP: usize = 8192;

/// rsync stdout → a rolling `SyncProgress`. Bytes in, because a chunk boundary
/// can fall inside a multi-byte character; complete fragments are what get
/// decoded (lossily — a mojibake file name is a cosmetic loss, a panic is not).
#[derive(Default)]
pub struct ProgressReader {
    partial: Vec<u8>,
    state: SyncProgress,
}

impl ProgressReader {
    pub fn state(&self) -> &SyncProgress {
        &self.state
    }

    /// Absorb a chunk of stdout; `true` if the rolling state moved.
    pub fn feed(&mut self, chunk: &[u8]) -> bool {
        self.partial.extend_from_slice(chunk);
        let mut changed = false;
        // BOTH separators. `\r` is the one that matters: it is the only thing
        // progress2 ever terminates a line with mid-transfer.
        while let Some(i) = self.partial.iter().position(|b| *b == b'\r' || *b == b'\n') {
            let frag: Vec<u8> = self.partial.drain(..=i).collect();
            let text = String::from_utf8_lossy(&frag[..frag.len() - 1]).into_owned();
            changed |= self.absorb(&text);
        }
        if self.partial.len() > PARTIAL_CAP {
            self.partial.clear();
        }
        changed
    }

    /// EOF: parse whatever never got a terminator (see the module note — that is
    /// normally the final 100% line).
    pub fn finish(&mut self) -> bool {
        let rest = std::mem::take(&mut self.partial);
        self.absorb(&String::from_utf8_lossy(&rest))
    }

    /// Fields MERGE: a name line must not blank the percentage, and a progress
    /// line must not blank the name.
    fn absorb(&mut self, frag: &str) -> bool {
        let before = self.state.clone();
        match parse_progress_fragment(frag) {
            Some(ProgressFragment::Stats { percent, rate }) => {
                if percent.is_some() {
                    self.state.percent = percent;
                }
                if rate.is_some() {
                    self.state.rate = rate;
                }
            }
            Some(ProgressFragment::Name(n)) => self.state.current = Some(n),
            None => {}
        }
        self.state != before
    }
}

// ── exclude file ─────────────────────────────────────────────────────────────

struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn exclude_body(extra: &[String]) -> String {
    let mut s = String::new();
    for e in EXCLUDES {
        s.push_str(e);
        s.push('\n');
    }
    for e in extra {
        let t = e.trim();
        if !t.is_empty() {
            s.push_str(t);
            s.push('\n');
        }
    }
    s
}

fn write_exclude_file(extra: &[String]) -> Result<TempFile, String> {
    let p = std::env::temp_dir().join(format!(
        "worktrees-sync-exclude-{}-{}",
        std::process::id(),
        sysclock::now_epoch()
    ));
    std::fs::write(&p, exclude_body(extra))
        .map_err(|e| format!("cannot write exclude file {}: {e}", p.display()))?;
    Ok(TempFile(p))
}

// ── the three rsync invocations ──────────────────────────────────────────────

struct Job<'a> {
    rsync: &'a Rsync,
    src: PathBuf,
    dst: PathBuf,
    backup_dir: PathBuf,
    excl: PathBuf,
}

/// rsync's "contents of" form. Every path here is one the tool built or
/// validated — argv is assembled, never taken as a string (provision::compose_down).
fn slashed(p: &Path) -> String {
    format!("{}/", p.display())
}

fn preview(job: &Job) -> Result<SyncPlan, String> {
    let out = Command::new(&job.rsync.path)
        .args(["-ani", "--delete"])
        .arg(format!("--exclude-from={}", job.excl.display()))
        .arg(slashed(&job.src))
        .arg(slashed(&job.dst))
        .output()
        .map_err(|e| format!("rsync failed to start: {e}"))?;
    if !out.status.success() {
        return Err(format!("rsync preview failed (exit {})", out.status.code().unwrap_or(-1)));
    }
    Ok(parse_itemized(&String::from_utf8_lossy(&out.stdout)))
}

/// v3-only. A failure here costs a REPORT, not the sync — hence no error channel.
fn skipped(job: &Job) -> Vec<(String, u32)> {
    let Ok(out) = Command::new(&job.rsync.path)
        .args(["-an", "-vv"])
        .arg(format!("--exclude-from={}", job.excl.display()))
        .arg(slashed(&job.src))
        .arg(slashed(&job.dst))
        .output()
    else {
        return Vec::new();
    };
    parse_hiding(&String::from_utf8_lossy(&out.stdout))
}

fn isatty_stdout() -> bool {
    // SAFETY: isatty(3) on a borrowed fd; no memory is touched.
    unsafe { libc::isatty(1) == 1 }
}

/// The APPLY argv, assembled in exactly one place. `info` is the only thing the
/// two apply paths differ by, so the terminal's run and the app's streaming run
/// cannot drift into being different transfers — which, with `--delete` in the
/// argv, is the kind of drift that costs files rather than pixels.
///
/// Order is load-bearing: `test/sync.bats` matches the whole line as a regex.
fn apply_argv(job: &Job, info: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = vec![
        "-a".into(),
        "--delete".into(),
        format!("--exclude-from={}", job.excl.display()),
    ];
    v.extend(info.iter().map(|s| (*s).to_string()));
    v.push("--backup".into());
    v.push(format!("--backup-dir={}", job.backup_dir.display()));
    v.push(slashed(&job.src));
    v.push(slashed(&job.dst));
    v
}

/// The CLI's progress flags: a TTY only, since the point is a human watching.
fn tty_info(job: &Job) -> Vec<&'static str> {
    if !isatty_stdout() {
        return Vec::new();
    }
    vec![if job.rsync.v3 { "--info=progress2" } else { "--progress" }]
}

/// What the STREAMING apply adds — `progress2` for the overall percentage and
/// `name1` for the file currently moving.
///
/// openrsync (v3=false) has no equivalent flag at all, so a non-v3 stream emits
/// NOTHING and the app shows its indeterminate state. Faking it with `-v` was
/// considered and refused: it changes the argv shape and the verbosity of a
/// transfer for everyone, to win a progress bar on the one rsync that cannot
/// report progress.
const STREAM_INFO: &str = "--info=progress2,name1";

fn stream_info(v3: bool) -> Vec<&'static str> {
    if v3 { vec![STREAM_INFO] } else { Vec::new() }
}

/// One phrasing of an rsync failure for all three invocations — the exact string
/// the CLI has always printed (bats asserts it).
fn rsync_failure(code: Option<i32>, stderr: &[u8]) -> String {
    let err = String::from_utf8_lossy(stderr);
    let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("rsync failed");
    format!("rsync failed (exit {}): {}", code.unwrap_or(-1), first.trim())
}

fn apply(job: &Job) -> Result<(), String> {
    // stdout INHERITED so progress reaches the terminal; stderr captured so a
    // failure can be reported rather than lost (git_status_captured's split).
    let child = Command::new(&job.rsync.path)
        .args(apply_argv(job, &tty_info(job)))
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("rsync failed to start: {e}"))?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    Err(rsync_failure(out.status.code(), &out.stderr))
}

/// At most ~15 progress events a second. `progress2` rewrites its line hundreds
/// of times a second on a fast SSD, and every one of those crossing the IPC
/// channel is jank of its own making — the transfer would be paid for twice.
const PROGRESS_MIN_GAP: Duration = Duration::from_millis(66);

/// The same transfer as `apply`, with stdout piped and parsed. Used only by
/// `sync_apply`'s streaming caller (the app); the CLI keeps its inherited-stdout
/// run, where rsync draws its own progress line better than we could relay it.
///
/// The sink is called at most every `PROGRESS_MIN_GAP` — and ALWAYS once at the
/// end, whatever the clock says, because the final state is the one the modal is
/// left showing. Coalescing happens here rather than at the IPC boundary because
/// only this loop knows where a read chunk ends and when the last one arrived.
fn apply_streaming(job: &Job, cb: &mut dyn FnMut(SyncProgress)) -> Result<(), String> {
    let mut child = Command::new(&job.rsync.path)
        .args(apply_argv(job, &stream_info(job.rsync.v3)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("rsync failed to start: {e}"))?;
    let mut out = child.stdout.take().ok_or("rsync stdout unavailable")?;
    // stderr is drained on its OWN thread, not left for wait_with_output: this
    // path holds TWO pipes, and a transfer that spams stderr (a permission-
    // denied storm on a big pull) fills that pipe's buffer while this loop is
    // still on stdout — rsync blocks on write, the loop blocks on read, and the
    // modal freezes on the exact sync this streaming exists to make visible.
    let mut errpipe = child.stderr.take().ok_or("rsync stderr unavailable")?;
    let err_thread = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = errpipe.read_to_end(&mut v);
        v
    });

    let mut rd = ProgressReader::default();
    let mut buf = [0u8; 8192];
    let mut last: Option<Instant> = None;
    let mut pending = false;
    loop {
        match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if rd.feed(&buf[..n]) {
                    pending = true;
                }
                if pending && last.is_none_or(|t| t.elapsed() >= PROGRESS_MIN_GAP) {
                    cb(rd.state().clone());
                    pending = false;
                    last = Some(Instant::now());
                }
            }
        }
    }
    // EOF: progress2 writes `\r` BEFORE each update, so the last line it drew
    // never got a terminator and is still sitting in the buffer. Flushing it is
    // the difference between finishing on 100% and finishing on whatever the
    // second-to-last chunk said.
    rd.finish();
    cb(rd.state().clone());

    let status = child.wait().map_err(|e| e.to_string())?;
    let stderr = err_thread.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    Err(rsync_failure(status.code(), &stderr))
}

/// Additive copy — no `--delete`, no `--backup`. Used for the sessions ferry in
/// both directions, so neither machine's history is ever the loser.
fn rsync_additive(rsync: &Rsync, src: &Path, dst: &Path) -> Result<(), String> {
    let out = Command::new(&rsync.path)
        .arg("-a")
        .arg(slashed(src))
        .arg(slashed(dst))
        .output()
        .map_err(|e| format!("rsync failed to start: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(rsync_failure(out.status.code(), &out.stderr))
}

// ── sessions ferry ───────────────────────────────────────────────────────────

/// `~/.claude/projects/<escaped>` — the working directory with `/` → `-`. Both
/// machines keep the project at the SAME absolute path, so the keys match.
pub fn escaped_prefix(local_root: &str) -> String {
    local_root.replace('/', "-")
}

fn claude_projects_dir() -> PathBuf {
    if let Ok(v) = std::env::var("CLAUDE_PROJECTS") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude/projects")
}

/// Directories whose name starts with `prefix` (the trailing-`*` glob also
/// catches this project's worktree keys), name-sorted.
fn session_dirs(root: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && basename(p).starts_with(prefix))
        .collect();
    out.sort();
    out
}

fn ferry_sessions(
    ui: &mut dyn Ui,
    rsync: &Rsync,
    dir: SyncDirection,
    hub_proj: &Path,
    local_root: &Path,
    quiet: bool,
) -> Result<(), String> {
    let store = hub_proj.join(SESSIONS_DIR);
    let projects = claude_projects_dir();
    let mut n = 0usize;
    match dir {
        SyncDirection::Push => {
            let prefix = escaped_prefix(&local_root.to_string_lossy());
            let dirs = session_dirs(&projects, &prefix);
            if !dirs.is_empty() {
                mkdir_p(&store)?;
            }
            for d in dirs {
                rsync_additive(rsync, &d, &store.join(basename(&d)))?;
                n += 1;
            }
            if !quiet {
                ui.info(&format!("claude sessions: {n} dir(s) → hub (additive, no delete)"));
            }
        }
        SyncDirection::Pull => {
            if !store.is_dir() {
                if !quiet {
                    ui.info("claude sessions: (hub has no sessions yet)");
                }
                return Ok(());
            }
            let dirs = session_dirs(&store, "");
            if !dirs.is_empty() {
                mkdir_p(&projects)?;
            }
            for d in dirs {
                rsync_additive(rsync, &d, &projects.join(basename(&d)))?;
                n += 1;
            }
            if !quiet {
                ui.info(&format!(
                    "claude sessions: {n} dir(s) → {} (additive, no delete)",
                    projects.display()
                ));
            }
        }
    }
    Ok(())
}

// ── shared helpers ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyncDirection {
    Push,
    Pull,
}

impl SyncDirection {
    pub fn label(self) -> &'static str {
        match self {
            SyncDirection::Push => "push",
            SyncDirection::Pull => "pull",
        }
    }

    /// The wire form the app sends. Unknown strings are refused rather than
    /// defaulted: guessing a direction picks which tree gets mirrored over.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "push" => Ok(SyncDirection::Push),
            "pull" => Ok(SyncDirection::Pull),
            other => Err(format!("unknown sync direction: {other}")),
        }
    }
}

fn basename(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A project name becomes a path component on the hub, so it may not steer one.
fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n != "."
        && n != ".."
        && !n.contains('/')
        && !n.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn mkdir_p(p: &Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("cannot create {}: {e}", p.display()))
}

fn hostname() -> String {
    Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[derive(Default)]
struct Opts {
    hub: Option<String>,
    dry_run: bool,
    yes: bool,
    with_sessions: Option<bool>,
    install: bool,
    json: bool,
    name: Option<String>,
}

fn parse_opts(
    ui: &mut dyn Ui,
    args: &[String],
    allow_name: bool,
    allow_install: bool,
) -> Result<Opts, i32> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--hub" => {
                let Some(v) = args.get(i + 1) else {
                    ui.error("--hub needs a PATH.");
                    return Err(1);
                };
                o.hub = Some(v.clone());
                i += 2;
                continue;
            }
            "--dry-run" | "-n" => o.dry_run = true,
            "--yes" | "-y" => o.yes = true,
            "--with-sessions" => o.with_sessions = Some(true),
            "--json" => o.json = true,
            "--install" if allow_install => o.install = true,
            s if s.starts_with("--hub=") => o.hub = Some(s["--hub=".len()..].to_string()),
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return Err(1);
            }
            s => {
                if !allow_name {
                    ui.error(&format!("Unexpected argument: {s}"));
                    return Err(1);
                }
                if o.name.is_some() {
                    ui.error("sync pull takes at most one project name.");
                    return Err(1);
                }
                o.name = Some(s.to_string());
            }
        }
        i += 1;
    }
    Ok(o)
}

fn json_mode(flag: bool) -> bool {
    flag || std::env::var("WORKTREES_JSON").ok().as_deref() == Some("1")
}

// ── the command ──────────────────────────────────────────────────────────────

pub fn cmd_sync(ui: &mut dyn Ui, args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("push") => run_direction(ui, SyncDirection::Push, args.get(1..).unwrap_or(&[])),
        Some("pull") => run_direction(ui, SyncDirection::Pull, args.get(1..).unwrap_or(&[])),
        Some("status") => cmd_status(ui, args.get(1..).unwrap_or(&[])),
        Some(other) => {
            ui.error(&format!("Unknown sync command: {other}"));
            ui.plain(SYNC_USAGE);
            1
        }
        None => {
            ui.error("sync needs a subcommand (push, pull or status).");
            ui.plain(SYNC_USAGE);
            1
        }
    }
}

fn run_direction(ui: &mut dyn Ui, dir: SyncDirection, args: &[String]) -> i32 {
    let is_pull = dir == SyncDirection::Pull;
    let opts = match parse_opts(ui, args, is_pull, is_pull) {
        Ok(o) => o,
        Err(c) => return c,
    };
    match sync_one(ui, dir, &opts) {
        Ok(code) => code,
        Err(e) => {
            ui.error(&e);
            1
        }
    }
}

#[derive(Serialize)]
struct PlanJson<'a> {
    schema_version: u32,
    direction: &'a str,
    name: &'a str,
    hub: String,
    src: String,
    dst: String,
    sends: u32,
    deletes: u32,
    delete_paths: &'a [String],
    dry_run: bool,
    applied: bool,
}

/// Everything a direction needs before rsync is spawned: which project, where it
/// lives on THIS machine, and where on the hub it belongs.
///
/// Extracted so the CLI and the app resolve a sync exactly once, in one place:
/// the hub-copy guard, the adoption rules and the strict manifest read are
/// properties of `sync`, not of whoever called it. `sync_one` walks it in the
/// same order it always did — the CLI's output is byte-for-byte what it was.
struct Ctx {
    rsync: Rsync,
    hub: PathBuf,
    name: String,
    local_root: PathBuf,
    hub_proj: PathBuf,
    hub_copy: PathBuf,
    manifest_path: PathBuf,
    /// Read and STRICTLY parsed for a pull, before anything is transferred.
    manifest: Option<Manifest>,
    adopting: bool,
    proj_cfg: config::SyncProject,
    with_sessions: bool,
}

/// `from` is a directory to discover a project from — the cwd for the CLI, the
/// project root for the app. `None` is "not standing anywhere", which only a
/// pull (adoption) can survive.
fn resolve_ctx(
    ui: &mut dyn Ui,
    dir: SyncDirection,
    from: Option<&Path>,
    name_arg: Option<&str>,
    hub_flag: Option<&str>,
    with_sessions: Option<bool>,
    cfg: &SyncCfg,
    quiet: bool,
) -> Result<Ctx, String> {
    let here = from.and_then(|d| Project::discover(d).ok());

    // push is cwd-scoped: refuse before anything else, so the message is about
    // the missing repo rather than a missing SSD.
    if dir == SyncDirection::Push && here.is_none() {
        return Err("sync push runs inside a project — cd into the repo you want to push.".into());
    }

    // Guard A, sync's half: a hub copy is a MIRROR. Pushing FROM one sends the
    // copy back over the original; pulling INTO one is hub→hub. Checked before
    // rsync is even looked for, so a refused command spawns nothing. It lives
    // HERE rather than in each caller: the app reaches these paths without going
    // through main.rs's dispatch guard at all.
    if let Some(p) = &here {
        if let Some(msg) = hub_copy_refusal(Path::new(&p.main_root)) {
            return Err(msg);
        }
    }

    let rsync = find_rsync()?;
    let hub = resolve_hub(hub_flag, cfg, ui, quiet)?;

    // Who are we syncing, and where does it live on THIS machine?
    let (name, local_root, adopting) = match &here {
        Some(p) => {
            let root = PathBuf::from(&p.main_root);
            let name = basename(&root);
            if let Some(n) = name_arg {
                if n != name {
                    return Err(format!(
                        "this repo is '{name}', not '{n}' — cd out of it to adopt another project"
                    ));
                }
            }
            (name, root, false)
        }
        None => {
            let Some(n) = name_arg.map(str::to_string) else {
                let avail = hub_manifests(&hub);
                let list = if avail.is_empty() {
                    "  (none — push from the other Mac first)".to_string()
                } else {
                    avail
                        .iter()
                        .map(|(n, m)| format!("  {n}  → {}", m.local_root))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                return Err(format!(
                    "sync pull outside a repo needs a project name. On this hub:\n{list}"
                ));
            };
            if !valid_name(&n) {
                return Err(format!("bad project name '{n}'"));
            }
            let m = read_manifest(&hub.join(&n).join(MANIFEST)).map_err(|e| {
                format!("{e}\n(no push for '{n}' on this hub? run `worktrees sync push` there first)")
            })?;
            (n, PathBuf::from(&m.local_root), true)
        }
    };
    if !valid_name(&name) {
        return Err(format!("bad project name '{name}'"));
    }
    // Adoption resolves its destination from the manifest rather than from the
    // cwd, so it needs the same check against the path it is about to mirror
    // over — the guard above only saw the tree the user is standing in.
    if adopting {
        if let Some(msg) = hub_copy_refusal(&local_root) {
            return Err(msg);
        }
    }

    let hub_proj = hub.join(&name);
    let hub_copy = hub_proj.join(&name);
    let manifest_path = hub_proj.join(MANIFEST);

    // A pull ALWAYS parses the manifest first: an unknown key is a hard error,
    // and it has to stop the transfer rather than be noticed after it.
    let manifest = if dir == SyncDirection::Pull {
        if !manifest_path.is_file() {
            return Err(format!(
                "hub has no push for '{name}' at {} — run `worktrees sync push` on the other Mac first",
                hub_proj.display()
            ));
        }
        let m = read_manifest(&manifest_path)?;
        if !hub_copy.is_dir() {
            return Err(format!(
                "hub manifest for '{name}' has no tree at {} — push again",
                hub_copy.display()
            ));
        }
        if !adopting && m.local_root != local_root.to_string_lossy() {
            ui.warn(&format!(
                "hub says this project lives at {} — pulling into {} anyway (you are standing in it)",
                m.local_root,
                local_root.display()
            ));
        }
        Some(m)
    } else {
        None
    };

    let proj_cfg = cfg.projects.get(&name).cloned().unwrap_or_default();
    Ok(Ctx {
        rsync,
        hub,
        name,
        local_root,
        hub_proj,
        hub_copy,
        manifest_path,
        manifest,
        adopting,
        proj_cfg,
        with_sessions: with_sessions.unwrap_or(cfg.with_sessions),
    })
}

/// The rsync job for one direction, plus the exclude file that must outlive it
/// (dropping `TempFile` deletes it, so it is held, not returned by value).
struct Prepared<'a> {
    job: Job<'a>,
    _excl: TempFile,
}

fn prepare<'a>(ctx: &'a Ctx, dir: SyncDirection) -> Result<Prepared<'a>, String> {
    let stamp = sysclock::stamp_now();
    let (src, dst, bk_root) = match dir {
        SyncDirection::Push => (
            ctx.local_root.clone(),
            ctx.hub_copy.clone(),
            ctx.hub_proj.join(SYNC_DIR).join("backups").join(&stamp),
        ),
        SyncDirection::Pull => (
            ctx.hub_copy.clone(),
            ctx.local_root.clone(),
            ctx.local_root.join(SYNC_DIR).join("backups").join(&stamp),
        ),
    };
    if !src.is_dir() {
        return Err(format!("source missing: {}", src.display()));
    }
    let excl = write_exclude_file(&ctx.proj_cfg.extra_excludes)?;
    let job = Job {
        rsync: &ctx.rsync,
        src,
        dst,
        backup_dir: bk_root.join(&ctx.name),
        excl: excl.0.clone(),
    };
    Ok(Prepared { job, _excl: excl })
}

fn sync_one(ui: &mut dyn Ui, dir: SyncDirection, opts: &Opts) -> Result<i32, String> {
    let json = json_mode(opts.json);
    let cfg = config::sync_cfg();
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let ctx = resolve_ctx(
        ui,
        dir,
        Some(&cwd),
        opts.name.as_deref(),
        opts.hub.as_deref(),
        opts.with_sessions,
        &cfg,
        json,
    )?;
    let Ctx { name, local_root, hub, hub_proj, manifest_path, manifest, proj_cfg, rsync, .. } = &ctx;

    if ctx.adopting && !opts.yes && !opts.dry_run {
        ui.plain(&format!("Adopt '{name}' from {} → {}", hub_proj.display(), local_root.display()));
        if !ui.confirm(&format!("Write into {}? [y/N] ", local_root.display())) {
            ui.info(&format!("Skipped {name}."));
            return Ok(0);
        }
    }

    let prep = prepare(&ctx, dir)?;
    let job = &prep.job;
    let (src, dst) = (job.src.clone(), job.dst.clone());

    if !json {
        ui.header(&format!("sync {} — {name}", dir.label()));
        ui.info(&format!("{} → {}", src.display(), dst.display()));
        ui.info(&format!("rsync: {} (v3={})", rsync.path, u8::from(rsync.v3)));
    }

    let plan = preview(job)?;
    if !json {
        ui.plain(&format!("  {} to send/update, {} to delete", plan.sends, plan.deletes));
        for p in &plan.delete_paths {
            ui.plain(&format!("    - {p}"));
        }
        let shown = plan.delete_paths.len() as u32;
        if plan.deletes > shown {
            ui.info(&format!("    … and {} more deletions", plan.deletes - shown));
        }
        if rsync.v3 {
            let pats = skipped(job);
            if pats.is_empty() {
                ui.info("  skipped (excluded): nothing matched");
            } else {
                let s = pats
                    .iter()
                    .map(|(p, n)| format!("{p} ×{n}"))
                    .collect::<Vec<_>>()
                    .join("  ");
                ui.info(&format!("  skipped (excluded, rebuild locally): {s}"));
            }
        } else {
            ui.info("  skipped report unavailable (openrsync)");
        }
    }

    let emit = |ui: &mut dyn Ui, applied: bool| {
        let j = PlanJson {
            schema_version: SCHEMA,
            direction: dir.label(),
            name: name.as_str(),
            hub: hub.to_string_lossy().into_owned(),
            src: src.to_string_lossy().into_owned(),
            dst: dst.to_string_lossy().into_owned(),
            sends: plan.sends,
            deletes: plan.deletes,
            delete_paths: &plan.delete_paths,
            dry_run: opts.dry_run,
            applied,
        };
        ui.plain(&serde_json::to_string(&j).unwrap_or_default());
    };

    if opts.dry_run {
        if json {
            emit(&mut *ui, false);
        } else {
            ui.info("dry run — nothing was written.");
        }
        return Ok(0);
    }

    // Guard B: a pull mirrors with `--delete` over the destination. If something
    // is LIVING in that tree, whether to do it anyway is a question only a human
    // can answer — `--yes` is consent given by a script before it could know.
    // Nothing above this point writes, so `--dry-run` (which returned already)
    // never reaches it.
    if dir == SyncDirection::Pull {
        let live = live_sessions_in(local_root);
        if !live.is_empty() {
            if !json {
                ui.warn(&format!(
                    "{} live tmux session(s) in {}:",
                    live.len(),
                    local_root.display()
                ));
                for s in &live {
                    ui.plain(&format!("    ● {s}"));
                }
                ui.warn("pull mirrors over a tree these sessions are using.");
            }
            if opts.yes {
                // Named in the ERROR too: under --json the prose above is
                // suppressed, and stderr is the one stream the JSON line is not on.
                ui.error(&format!(
                    "refusing to pull with --yes while {} is live — close the session(s) (worktrees close <name>) or run without --yes and answer the prompt.",
                    live.join(", ")
                ));
                return Ok(crate::diag::EXIT_NEEDS_CONFIRM);
            }
        }
    }

    if !opts.yes && !ui.confirm(&format!("Apply to {name}? [y/N] ")) {
        ui.info(&format!("Skipped {name}."));
        if json {
            emit(&mut *ui, false);
        }
        return Ok(0);
    }

    mkdir_p(&dst)?;
    apply(job)?;
    if !json {
        ui.info("done.");
    }

    if dir == SyncDirection::Push {
        let m = Manifest {
            schema: SCHEMA,
            name: name.clone(),
            local_root: local_root.to_string_lossy().into_owned(),
            pushed_at: sysclock::now_epoch(),
            host: hostname(),
            hint: proj_cfg.install.clone().unwrap_or_default(),
        };
        std::fs::write(manifest_path, render_manifest(&m))
            .map_err(|e| format!("cannot write {}: {e}", manifest_path.display()))?;
    }

    if ctx.with_sessions {
        ferry_sessions(ui, rsync, dir, hub_proj, local_root, json)?;
    } else if !json {
        ui.info("claude sessions: skipped (pass --with-sessions to ferry them)");
    }

    match dir {
        SyncDirection::Push => {
            if !json {
                ui.info("push complete — eject the hub, plug it into the other Mac, then: worktrees sync pull --install");
            }
        }
        // `--install` still RUNS under --json (it is the point of the flag); only
        // its prose is suppressed so the JSON stays one line.
        SyncDirection::Pull => {
            after_pull(ui, opts.install, name, local_root, proj_cfg, manifest.as_ref(), json)
        }
    }
    if json {
        emit(&mut *ui, true);
    }
    Ok(0)
}

/// `--install` runs the USER's own command (never the manifest's `hint`, which
/// arrived on removable media and is printed only).
fn after_pull(
    ui: &mut dyn Ui,
    install: bool,
    name: &str,
    local_root: &Path,
    proj_cfg: &config::SyncProject,
    manifest: Option<&Manifest>,
    quiet: bool,
) {
    if install {
        if let Some(cmd) = proj_cfg.install.as_deref().filter(|s| !s.is_empty()) {
            if !quiet {
                ui.info(&format!("rebuilding: {cmd}"));
            }
            let ok = Command::new("bash")
                .arg("-c")
                .arg(cmd)
                .current_dir(local_root)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return;
            }
            ui.warn("rebuild failed — run it manually.");
            return;
        }
        ui.warn(&format!(
            "no rebuild registered for '{name}' — add [sync.projects.{name}] install to ~/.config/worktrees/config.toml"
        ));
    }
    if quiet {
        return;
    }
    let hint = manifest.map(|m| m.hint.as_str()).unwrap_or("");
    if hint.is_empty() {
        ui.info(&format!("Next: cd {} and rebuild.", local_root.display()));
    } else {
        // The hint is a STRING FROM THE HUB. It is shown, never handed to a shell.
        ui.info(&format!("Next: cd {} && {hint}", local_root.display()));
    }
}

// ── programmatic API (the app) ───────────────────────────────────────────────
// Data in, data out: no `Ui`, no prompts, no cwd discovery. The app knows the
// project root, and a GUI answers a question with a modal rather than with
// stdin. Everything below runs the SAME `resolve_ctx`/`prepare`/`preview`/
// `apply` the CLI does, so a guard cannot hold on one surface and not the other.

pub struct SyncRequest {
    /// Project root. The app always knows it; nothing here reads the cwd.
    pub root: PathBuf,
    pub direction: SyncDirection,
    /// `None` = the normal chain (`$WORKTREES_SYNC_HUB` → `[sync] hub` → autodetect).
    pub hub: Option<String>,
    /// `None` = the user's `[sync] with_sessions` default.
    pub with_sessions: Option<bool>,
}

#[derive(Serialize)]
pub struct SyncPreviewOut {
    pub name: String,
    pub direction: &'static str,
    pub hub: String,
    pub src: String,
    pub dst: String,
    pub plan: SyncPlan,
    /// Exclude patterns and how many paths each hid. Empty on openrsync, which
    /// cannot report it at all — `skipped_available` is how a caller tells the
    /// two apart instead of showing "nothing was skipped" for a missing report.
    pub skipped: Vec<(String, u32)>,
    pub skipped_available: bool,
    /// Guard B's data — PULL only; a push never mirrors over a live tree.
    pub live_sessions: Vec<String>,
    pub with_sessions: bool,
    /// The user's own `[sync.projects.<name>] install`, so a pull can OFFER to
    /// run it and name what it would run. `None` = nothing registered, and the
    /// app shows no checkbox rather than an unexplained one. Never the hub
    /// manifest's `hint`, which arrived on removable media and is printed only.
    pub install_cmd: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct SyncApplyOut {
    pub name: String,
    pub direction: &'static str,
    pub hub: String,
    pub src: String,
    pub dst: String,
    pub plan: SyncPlan,
    /// Non-empty with `applied == false` means Guard B stopped the transfer.
    pub live_sessions: Vec<String>,
    pub needs_confirm: bool,
    pub applied: bool,
    pub messages: Vec<String>,
    pub warnings: Vec<String>,
}

/// What a sync WOULD do. Runs the same two rsync dry passes the CLI prints, plus
/// the live-session lookup a pull has to answer for.
pub fn sync_preview(req: &SyncRequest) -> Result<SyncPreviewOut, String> {
    let mut ui = CaptureUi::default();
    let cfg = config::sync_cfg();
    let ctx = resolve_ctx(
        &mut ui,
        req.direction,
        Some(&req.root),
        None,
        req.hub.as_deref(),
        req.with_sessions,
        &cfg,
        false,
    )?;
    let prep = prepare(&ctx, req.direction)?;
    let plan = preview(&prep.job)?;
    let skipped_pats = if ctx.rsync.v3 { skipped(&prep.job) } else { Vec::new() };
    let live = match req.direction {
        SyncDirection::Pull => live_sessions_in(&ctx.local_root),
        SyncDirection::Push => Vec::new(),
    };
    Ok(SyncPreviewOut {
        name: ctx.name.clone(),
        direction: req.direction.label(),
        hub: ctx.hub.to_string_lossy().into_owned(),
        src: prep.job.src.to_string_lossy().into_owned(),
        dst: prep.job.dst.to_string_lossy().into_owned(),
        plan,
        skipped: skipped_pats,
        skipped_available: ctx.rsync.v3,
        live_sessions: live,
        with_sessions: ctx.with_sessions,
        install_cmd: ctx.proj_cfg.install.clone().filter(|s| !s.is_empty()),
        warnings: ui.warnings(),
    })
}

/// Apply it. `confirmed` means a HUMAN was shown the live sessions and said yes
/// — it is the GUI's spelling of the CLI's answered prompt, NOT of `--yes`
/// (which the CLI refuses in exactly this case).
///
/// The session list is re-read HERE, not taken from the preview: the preview may
/// be minutes old, and a session that started since is one this call has never
/// shown anyone. So a fresh list means a fresh question.
///
/// `progress` is the app's live bar. `None` is EXACTLY today's transfer — same
/// argv, stdout inherited — which is what the CLI runs and what `test/sync.bats`
/// pins; `Some(cb)` pipes stdout and adds `--info=progress2,name1` to the same
/// assembled argv (`apply_argv`), so the two can never become different syncs.
pub fn sync_apply(
    req: &SyncRequest,
    confirmed: bool,
    install: bool,
    progress: Option<&mut dyn FnMut(SyncProgress)>,
) -> Result<SyncApplyOut, String> {
    let mut ui = CaptureUi::default();
    let cfg = config::sync_cfg();
    let ctx = resolve_ctx(
        &mut ui,
        req.direction,
        Some(&req.root),
        None,
        req.hub.as_deref(),
        req.with_sessions,
        &cfg,
        false,
    )?;
    let prep = prepare(&ctx, req.direction)?;
    let job = &prep.job;
    let plan = preview(job)?;

    let out = |applied: bool, live: Vec<String>, ui: &CaptureUi| SyncApplyOut {
        name: ctx.name.clone(),
        direction: req.direction.label(),
        hub: ctx.hub.to_string_lossy().into_owned(),
        src: job.src.to_string_lossy().into_owned(),
        dst: job.dst.to_string_lossy().into_owned(),
        plan: plan.clone(),
        needs_confirm: !applied,
        applied,
        live_sessions: live,
        messages: ui.lines.clone(),
        warnings: ui.warnings(),
    };

    if req.direction == SyncDirection::Pull {
        let live = live_sessions_in(&ctx.local_root);
        if !live.is_empty() && !confirmed {
            return Ok(out(false, live, &ui));
        }
    }

    mkdir_p(&job.dst)?;
    match progress {
        Some(cb) => apply_streaming(job, cb)?,
        None => apply(job)?,
    }

    if req.direction == SyncDirection::Push {
        let m = Manifest {
            schema: SCHEMA,
            name: ctx.name.clone(),
            local_root: ctx.local_root.to_string_lossy().into_owned(),
            pushed_at: sysclock::now_epoch(),
            host: hostname(),
            hint: ctx.proj_cfg.install.clone().unwrap_or_default(),
        };
        std::fs::write(&ctx.manifest_path, render_manifest(&m))
            .map_err(|e| format!("cannot write {}: {e}", ctx.manifest_path.display()))?;
    }
    if ctx.with_sessions {
        ferry_sessions(&mut ui, &ctx.rsync, req.direction, &ctx.hub_proj, &ctx.local_root, false)?;
    }
    if req.direction == SyncDirection::Pull {
        after_pull(
            &mut ui,
            install,
            &ctx.name,
            &ctx.local_root,
            &ctx.proj_cfg,
            ctx.manifest.as_ref(),
            false,
        );
    }
    Ok(out(true, Vec::new(), &ui))
}

// ── status ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ManifestBrief {
    pub name: String,
    pub local_root: String,
    pub pushed_at: i64,
    pub host: String,
}

#[derive(Serialize)]
pub struct StatusJson {
    pub schema_version: u32,
    pub scope: &'static str,
    pub name: Option<String>,
    pub local_root: Option<String>,
    pub rsync: Option<String>,
    pub rsync_v3: bool,
    pub hub: Option<String>,
    pub hub_error: Option<String>,
    pub branch: Option<String>,
    pub dirty: Option<u32>,
    pub pushed_at: Option<i64>,
    pub pushed_host: Option<String>,
    pub hint: Option<String>,
    pub projects: Vec<ManifestBrief>,
}

/// The data half of `sync status`, for the CLI's printer and for the app.
///
/// `from` is a directory to discover a project from (the cwd for the CLI, the
/// project root for the app); `None`, or a directory in no repo, gives the
/// hub-level report. Nothing here is fatal — reporting a missing hub is most of
/// what status is for.
///
/// ⚠ Takes a `Ui` (the brief's shape did not) only because `resolve_hub` prints
/// `auto-detected SSD hub: …` through it, and dropping that line would change
/// CLI output.
pub fn sync_status_data(
    ui: &mut dyn Ui,
    from: Option<&Path>,
    hub_flag: Option<&str>,
    quiet: bool,
) -> StatusJson {
    let cfg = config::sync_cfg();
    let here = from.and_then(|d| Project::discover(d).ok());
    let rsync = find_rsync().ok();
    let hub = resolve_hub(hub_flag, &cfg, ui, quiet);

    let mut out = StatusJson {
        schema_version: SCHEMA,
        scope: if here.is_some() { "project" } else { "hub" },
        name: None,
        local_root: None,
        rsync: rsync.as_ref().map(|r| r.path.clone()),
        rsync_v3: rsync.as_ref().map(|r| r.v3).unwrap_or(false),
        hub: hub.as_ref().ok().map(|h| h.to_string_lossy().into_owned()),
        hub_error: hub.as_ref().err().cloned(),
        branch: None,
        dirty: None,
        pushed_at: None,
        pushed_host: None,
        hint: None,
        projects: Vec::new(),
    };

    match &here {
        Some(p) => {
            let root = PathBuf::from(&p.main_root);
            let name = basename(&root);
            out.branch = git::git_out(&p.main_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
            out.dirty = git::git_out(&p.main_root, &["status", "--porcelain"])
                .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as u32);
            if let Ok(h) = &hub {
                if let Ok(m) = read_manifest(&h.join(&name).join(MANIFEST)) {
                    out.pushed_at = Some(m.pushed_at);
                    out.pushed_host = Some(m.host.clone());
                    out.hint = Some(m.hint.clone());
                }
            }
            out.name = Some(name);
            out.local_root = Some(root.to_string_lossy().into_owned());
        }
        None => {
            if let Ok(h) = &hub {
                out.projects = hub_manifests(h)
                    .into_iter()
                    .map(|(name, m)| ManifestBrief {
                        name,
                        local_root: m.local_root,
                        pushed_at: m.pushed_at,
                        host: m.host,
                    })
                    .collect();
            }
        }
    }
    out
}

fn cmd_status(ui: &mut dyn Ui, args: &[String]) -> i32 {
    let opts = match parse_opts(ui, args, false, false) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let json = json_mode(opts.json);
    let Ok(cwd) = std::env::current_dir() else {
        ui.error("cannot read the current directory");
        return 1;
    };
    // A missing hub is REPORTED by status, not fatal — that is most of what the
    // command is for.
    let out = sync_status_data(ui, Some(&cwd), opts.hub.as_deref(), json);

    if json {
        ui.plain(&serde_json::to_string(&out).unwrap_or_default());
        return 0;
    }

    let clock = sysclock::SysClock::detect();
    let hub_cell = match (&out.hub, &out.hub_error) {
        (Some(h), _) => h.clone(),
        (None, Some(e)) => format!("<none> ({})", e.lines().next().unwrap_or("")),
        (None, None) => "<none>".to_string(),
    };
    match out.scope {
        "project" => {
            ui.header(&format!("sync status — {}", out.name.clone().unwrap_or_default()));
            ui.plain(&format!(
                "  rsync:      {}  (v3={})",
                out.rsync.clone().unwrap_or_else(|| "<none>".into()),
                u8::from(out.rsync_v3)
            ));
            ui.plain(&format!("  local root: {}", out.local_root.clone().unwrap_or_default()));
            ui.plain(&format!("  hub:        {hub_cell}"));
            ui.plain(&format!(
                "  branch:     {} [{} dirty]",
                out.branch.clone().unwrap_or_else(|| "?".into()),
                out.dirty.unwrap_or(0)
            ));
            match out.pushed_at {
                Some(at) => ui.plain(&format!(
                    "  last push:  {} from {}",
                    clock.fmt_date(at),
                    if out.pushed_host.as_deref().unwrap_or("").is_empty() {
                        "?"
                    } else {
                        out.pushed_host.as_deref().unwrap_or("?")
                    }
                )),
                None => ui.plain("  last push:  <never pushed to this hub>"),
            }
        }
        _ => {
            ui.header("sync status — hub");
            ui.plain(&format!("  rsync:      {}  (v3={})",
                out.rsync.clone().unwrap_or_else(|| "<none>".into()),
                u8::from(out.rsync_v3)));
            ui.plain(&format!("  hub:        {hub_cell}"));
            if out.projects.is_empty() {
                ui.info("  no projects pushed to this hub yet");
            } else {
                for p in &out.projects {
                    ui.plain(&format!(
                        "    {}  → {}  ({} from {})",
                        p.name,
                        p.local_root,
                        clock.fmt_date(p.pushed_at),
                        if p.host.is_empty() { "?" } else { &p.host }
                    ));
                }
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wtsync-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn itemized_preview_counts_sends_and_deletes() {
        // Real `rsync -ani --delete` shapes: itemize flags, a deletion, and the
        // noise lines a parser must ignore.
        let out = "\
.d..t...... ./
>f+++++++++ src/main.rs
>f..t...... README.md
cd+++++++++ newdir/
<f+++++++++ up.txt
hf          hard.link
*deleting   gone.txt
*deleting   old/dir/
sending incremental file list

sent 1,234 bytes  received 56 bytes";
        let p = parse_itemized(out);
        assert_eq!(p.sends, 6);
        assert_eq!(p.deletes, 2);
        assert_eq!(p.delete_paths, vec!["gone.txt", "old/dir/"]);
    }

    #[test]
    fn itemized_preview_caps_the_delete_sample_but_not_the_count() {
        let mut out = String::new();
        for i in 0..40 {
            out.push_str(&format!("*deleting   f{i}\n"));
        }
        let p = parse_itemized(&out);
        assert_eq!(p.deletes, 40);
        assert_eq!(p.delete_paths.len(), DELETE_SAMPLE);
        assert_eq!(p.delete_paths[0], "f0");
        assert_eq!(p.delete_paths[DELETE_SAMPLE - 1], format!("f{}", DELETE_SAMPLE - 1));
    }

    #[test]
    fn hiding_report_counts_patterns_most_hit_first() {
        let out = "\
[sender] hiding directory node_modules because of pattern node_modules/
hiding directory app/node_modules because of pattern node_modules/
[sender] hiding file debug.log because of pattern *.log
hiding directory target because of pattern target/
hiding directory a/node_modules because of pattern node_modules/
building file list
[sender] hiding file weird because of pattern name because of pattern *.odd";
        let v = parse_hiding(out);
        assert_eq!(v[0], ("node_modules/".to_string(), 3));
        // ties break by pattern, so the output is stable run to run
        assert_eq!(v[1], ("*.log".to_string(), 1));
        assert_eq!(v[2], ("*.odd".to_string(), 1), "greedy: the LAST marker wins");
        assert_eq!(v[3], ("target/".to_string(), 1));
        assert!(parse_hiding("nothing here").is_empty());
    }

    // ── streaming progress ──────────────────────────────────────────────────

    /// A real `--info=progress2` line, at four points in a transfer.
    const P0: &str = "         98,304   0%   93.75kB/s    0:00:01 (xfr#1, to-chk=999/1000)";
    const P50: &str = "      1,048,576  50%    1.00MB/s    0:00:01 (xfr#12, to-chk=500/1000)";
    const P100: &str = "      2,097,152 100%    2.00MB/s    0:00:02 (xfr#25, to-chk=0/1000)";

    #[test]
    fn progress_fragment_reads_a_progress2_line() {
        let got = parse_progress_fragment(P50).unwrap();
        assert_eq!(
            got,
            ProgressFragment::Stats { percent: Some(50.0), rate: Some("1.00MB/s".into()) },
            "percent from the % token, rate VERBATIM as rsync printed it"
        );
        assert_eq!(
            parse_progress_fragment(P0).unwrap(),
            ProgressFragment::Stats { percent: Some(0.0), rate: Some("93.75kB/s".into()) }
        );
    }

    #[test]
    fn progress_fragment_falls_back_to_to_chk_when_the_percent_token_is_gone() {
        // Same line with the % token mangled — the transfer is still 40% done
        // and the file counts say so.
        let line = "      1,048,576  ??    1.00MB/s    0:00:01 (xfr#12, to-chk=600/1000)";
        let ProgressFragment::Stats { percent, .. } = parse_progress_fragment(line).unwrap() else {
            panic!("not a progress line");
        };
        assert_eq!(percent, Some(40.0));
    }

    #[test]
    fn progress_fragment_treats_a_plain_line_as_a_file_name() {
        assert_eq!(
            parse_progress_fragment("app/src/App.tsx").unwrap(),
            ProgressFragment::Name("app/src/App.tsx".into())
        );
        // name1 prints directories with the trailing slash, and paths with
        // spaces are just paths
        assert_eq!(
            parse_progress_fragment("  docs/adr/my notes/ ").unwrap(),
            ProgressFragment::Name("docs/adr/my notes/".into())
        );
    }

    #[test]
    fn progress_fragment_ignores_blanks_and_rsyncs_own_trailers() {
        for l in [
            "",
            "   ",
            "sent 4,096 bytes  received 89 bytes  8,370.00 bytes/sec",
            "total size is 12,345,678  speedup is 9.99",
            "sending incremental file list",
            "receiving incremental file list",
            "created directory /Volumes/SanDisk/worktrees",
        ] {
            assert!(
                parse_progress_fragment(l).is_none(),
                "a trailer must never be reported as the file being copied: {l:?}"
            );
        }
        // …but a percentage-shaped thing that is NOT a progress line still reads
        // as a name rather than moving the bar
        assert_eq!(
            parse_progress_fragment("notes/90% done.md").unwrap(),
            ProgressFragment::Name("notes/90% done.md".into())
        );
    }

    #[test]
    fn progress_reader_splits_on_carriage_returns_and_merges_fields() {
        let mut rd = ProgressReader::default();
        // Exactly what rsync writes: a name terminated by \n, then progress
        // lines each PRECEDED by \r (so the last one has no terminator).
        rd.feed(b"src/main.rs\n");
        assert_eq!(rd.state().current.as_deref(), Some("src/main.rs"));
        assert_eq!(rd.state().percent, None);

        rd.feed(format!("\r{P0}\r{P50}").as_bytes());
        assert_eq!(rd.state().percent, Some(0.0), "only the FIRST line is terminated yet");
        assert_eq!(rd.state().current.as_deref(), Some("src/main.rs"), "a progress line keeps the name");

        rd.feed(format!("\rdocs/x.md\n\r{P100}").as_bytes());
        assert_eq!(rd.state().percent, Some(50.0));
        assert_eq!(rd.state().current.as_deref(), Some("docs/x.md"), "a name line keeps the percent");

        // EOF flushes the 100% line that never got a terminator of its own —
        // without it a finished sync is left showing 50%.
        assert!(rd.finish());
        assert_eq!(rd.state().percent, Some(100.0));
        assert_eq!(rd.state().rate.as_deref(), Some("2.00MB/s"));
    }

    /// A verbatim slice of what rsync 3.4.4 actually wrote through a pipe
    /// (captured 2026-08-16, `-a --delete --info=progress2,name1`): names are
    /// `\n`-terminated, every progress line is `\r`-PREFIXED, the last line is
    /// rewritten three times back-to-back with no separator between rewrites,
    /// and the whole stream ends in a single `\n`. The canned tests above are
    /// what we believe rsync does; this one is what it did.
    #[test]
    fn progress_reader_handles_a_real_rsync_capture() {
        let real = b"f1.bin\n\
            \r         32,768   0%    0.00kB/s    0:00:00  \
            \r      3,145,728  12%  989.58MB/s    0:00:00 (xfr#1, to-chk=9/11)\n\
            f2.bin\n\
            \r      6,291,456  24%  994.79MB/s    0:00:00 (xfr#2, to-chk=8/11)\n\
            sub/\n\
            sub/nested file with spaces.txt\n\
            \r     25,165,827 100%  887.73MB/s    0:00:00 (xfr#9, to-chk=0/11)\
            \r     25,165,827 100%  856.03MB/s    0:00:00 (xfr#9, to-chk=0/11)\
            \r     25,165,827 100%  856.03MB/s    0:00:00 (xfr#9, to-chk=0/11)\n";
        // fed in awkward 7-byte chunks, the way a pipe is allowed to deliver it
        let mut rd = ProgressReader::default();
        for chunk in real.chunks(7) {
            rd.feed(chunk);
        }
        rd.finish();
        assert_eq!(rd.state().percent, Some(100.0));
        assert_eq!(rd.state().rate.as_deref(), Some("856.03MB/s"));
        assert_eq!(rd.state().current.as_deref(), Some("sub/nested file with spaces.txt"));
    }

    #[test]
    fn progress_reader_survives_chunk_boundaries_and_junk() {
        let mut rd = ProgressReader::default();
        // one line delivered a byte at a time
        for b in format!("\r{P50}\r").bytes() {
            rd.feed(&[b]);
        }
        assert_eq!(rd.state().percent, Some(50.0));

        // a stream with no separator at all must not grow forever
        rd.feed(&vec![b'x'; PARTIAL_CAP * 2]);
        assert!(rd.partial.len() <= PARTIAL_CAP, "unterminated garbage is dropped, not buffered");
        // and the state it had is still the state it has
        assert_eq!(rd.state().percent, Some(50.0));
    }

    #[test]
    fn streaming_argv_is_the_apply_argv_plus_the_info_flags() {
        // The whole point of `apply_argv`: the app's transfer and the
        // terminal's are the same rsync run, one flag apart. If this ever
        // fails, `--delete` is running somewhere it was not reviewed.
        let rsync = Rsync { path: "/opt/homebrew/bin/rsync".into(), v3: true };
        let job = Job {
            rsync: &rsync,
            src: PathBuf::from("/src"),
            dst: PathBuf::from("/dst"),
            backup_dir: PathBuf::from("/dst/.worktrees-sync/backups/ts/name"),
            excl: PathBuf::from("/tmp/excl"),
        };
        let plain = apply_argv(&job, &[]);
        let streamed = apply_argv(&job, &stream_info(true));
        assert_eq!(streamed.len(), plain.len() + 1);
        let extra: Vec<&String> = streamed.iter().filter(|a| !plain.contains(a)).collect();
        assert_eq!(extra, vec![&STREAM_INFO.to_string()]);
        // …and it is inserted, not appended: paths stay last, where rsync wants them
        assert_eq!(streamed.last().unwrap(), "/dst/");
        assert_eq!(
            plain,
            vec![
                "-a",
                "--delete",
                "--exclude-from=/tmp/excl",
                "--backup",
                "--backup-dir=/dst/.worktrees-sync/backups/ts/name",
                "/src/",
                "/dst/",
            ]
        );
        // openrsync gets NO progress flags — it has none, and inventing one
        // (`-v`) would change the transfer for everybody.
        let old = Rsync { path: "/usr/bin/rsync".into(), v3: false };
        let job2 = Job { rsync: &old, ..job };
        assert_eq!(apply_argv(&job2, &stream_info(false)), plain);
    }

    #[test]
    fn hub_candidates_skip_the_denylist_and_symlinks() {
        let v = |n: &str, link: bool| (n.to_string(), link, PathBuf::from(format!("/Volumes/{n}")));
        // the shape a stock Mac shows: system disk (a symlink), TM, the share
        let only_system =
            vec![v("Macintosh HD", true), v(".timemachine", false), v("davidpena", false)];
        assert!(hub_candidates_from(&only_system).is_empty());

        let one = vec![v("Macintosh HD", true), v("FastSSD", false)];
        assert_eq!(hub_candidates_from(&one), vec![PathBuf::from("/Volumes/FastSSD")]);

        let many = vec![v("FastSSD", false), v("Backup", false), v(".timemachine", false)];
        assert_eq!(hub_candidates_from(&many).len(), 2);

        // a non-denylisted SYMLINK is still skipped (the 'Macintosh HD' → / trap
        // under a different name)
        let linked = vec![v("Whatever", true)];
        assert!(hub_candidates_from(&linked).is_empty());
    }

    #[test]
    fn rsync_pick_prefers_v3_over_an_earlier_fallback() {
        let c = |p: &str, v: Option<&str>| (p.to_string(), v.map(String::from));
        // openrsync first, homebrew 3.x second → 3.x wins OUTRIGHT
        let got = pick_rsync_from(&[
            c("/usr/bin/rsync", Some("openrsync: protocol version 29")),
            c("/opt/homebrew/bin/rsync", Some("rsync  version 3.4.4  protocol version 32")),
        ])
        .unwrap();
        assert_eq!(got.path, "/opt/homebrew/bin/rsync");
        assert!(got.v3);

        // nothing v3 → the first that RAN
        let got = pick_rsync_from(&[
            c("/opt/homebrew/bin/rsync", None),
            c("rsync", Some("openrsync: protocol version 29")),
        ])
        .unwrap();
        assert_eq!(got.path, "rsync");
        assert!(!got.v3);

        // version 4 counts too; nothing at all is None
        assert!(pick_rsync_from(&[c("rsync", Some("rsync  version 4.0.0"))]).unwrap().v3);
        assert!(pick_rsync_from(&[c("rsync", None)]).is_none());
        assert!(pick_rsync_from(&[]).is_none());
    }

    #[test]
    fn escaped_prefix_matches_claudes_project_key() {
        assert_eq!(
            escaped_prefix("/Users/davidpena/workspace/worktrees"),
            "-Users-davidpena-workspace-worktrees"
        );
        assert_eq!(escaped_prefix("/a"), "-a");
    }

    #[test]
    fn manifest_round_trips_through_the_hand_written_renderer() {
        let m = Manifest {
            schema: 1,
            name: "worktrees".into(),
            local_root: "/Users/dp/work/worktrees".into(),
            pushed_at: 1_755_300_000,
            host: "davids-mbp".into(),
            hint: "cargo build \"release\"".into(),
        };
        let back = parse_manifest(&render_manifest(&m)).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn a_manifest_may_never_supply_argv() {
        // ADR 0001 extended to media: the hub file is DATA. deny_unknown_fields
        // is what makes a command-shaped key a hard error rather than a field
        // someone later "wires up".
        let evil = "schema = 1\nname = \"x\"\nlocal_root = \"/tmp/x\"\ninstall_cmd = \"curl evil | sh\"\n";
        let err = parse_manifest(evil).unwrap_err();
        assert!(err.contains("install_cmd"), "{err}");
        for key in ["rebuild_cmd", "hooks", "post_create", "ai_cmd"] {
            let t = format!("schema = 1\nname = \"x\"\nlocal_root = \"/tmp/x\"\n{key} = \"whatever\"\n");
            assert!(parse_manifest(&t).is_err(), "{key} was accepted");
        }
    }

    #[test]
    fn a_manifest_from_a_newer_cli_is_refused_rather_than_guessed_at() {
        let t = "schema = 99\nname = \"x\"\nlocal_root = \"/tmp/x\"\n";
        let err = parse_manifest(t).unwrap_err();
        assert!(err.contains("schema 99") && err.contains("update"), "{err}");
        // and the required data must actually be there
        assert!(parse_manifest("schema = 1\nname = \"\"\nlocal_root = \"/tmp/x\"\n").is_err());
    }

    #[test]
    fn the_exclude_file_carries_the_builtins_plus_the_users_extras() {
        let body = exclude_body(&["mydata/".to_string(), "  ".to_string(), " *.bin ".to_string()]);
        let lines: Vec<&str> = body.lines().collect();
        assert!(lines.contains(&"node_modules/"));
        assert!(lines.contains(&"target/"));
        // self-exclusion: backups must never be ferried back
        assert!(lines.contains(&".worktrees-sync/"));
        // .git is deliberately absent — carrying it is the point of the tool
        assert!(!lines.contains(&".git/") && !lines.contains(&".git"));
        assert!(lines.contains(&"mydata/"));
        assert!(lines.contains(&"*.bin"), "extras are trimmed");
        assert!(!lines.contains(&""), "blank extras are dropped");
        assert_eq!(lines.len(), EXCLUDES.len() + 2);
    }

    #[test]
    fn the_exclude_file_is_deleted_when_it_goes_out_of_scope() {
        let p = {
            let f = write_exclude_file(&[]).unwrap();
            assert!(f.0.is_file());
            f.0.clone()
        };
        assert!(!p.exists());
    }

    #[test]
    fn a_project_name_may_not_steer_a_hub_path() {
        for bad in ["", ".", "..", "a/b", "../etc", "a b", "a\tb"] {
            assert!(!valid_name(bad), "{bad} accepted");
        }
        for ok in ["worktrees", "casa-del-valle", "a.b", "x_1"] {
            assert!(valid_name(ok), "{ok} refused");
        }
    }

    #[test]
    fn hub_manifests_lists_only_readable_ones() {
        let d = tmp("hubman");
        for n in ["alpha", "beta", "broken"] {
            std::fs::create_dir_all(d.join(n)).unwrap();
        }
        std::fs::write(
            d.join("alpha").join(MANIFEST),
            "schema = 1\nname = \"alpha\"\nlocal_root = \"/tmp/a\"\n",
        )
        .unwrap();
        std::fs::write(
            d.join("beta").join(MANIFEST),
            "schema = 1\nname = \"beta\"\nlocal_root = \"/tmp/b\"\npushed_at = 7\n",
        )
        .unwrap();
        std::fs::write(d.join("broken").join(MANIFEST), "schema = 1\nname = \"broken\"\nnope = 1\n")
            .unwrap();
        let got = hub_manifests(&d);
        assert_eq!(got.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["alpha", "beta"]);
        assert_eq!(got[1].1.pushed_at, 7);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_hub_copy_is_a_manifest_in_the_parent_that_is_about_this_dir_and_points_elsewhere() {
        let d = tmp("hubcopy");
        let root = d.join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let mf = d.join(MANIFEST);
        let write = |body: &str| std::fs::write(&mf, body).unwrap();

        // no manifest at all — the fast path, and the answer for every normal repo
        assert!(hub_copy_of(&root).is_none());

        // the shape a push leaves: <hub>/proj/.worktrees-sync.toml + <hub>/proj/proj
        write("schema = 1\nname = \"proj\"\nlocal_root = \"/Users/dp/work/proj\"\nhost = \"othermac\"\n");
        let m = hub_copy_of(&root).expect("a hub copy");
        assert_eq!(m.local_root, "/Users/dp/work/proj");
        assert!(hub_copy_refusal(&root).unwrap().contains("/Users/dp/work/proj"));
        assert!(hub_copy_refusal(&root).unwrap().contains("othermac"));

        // about ANOTHER project — this dir merely sits next to it
        write("schema = 1\nname = \"other\"\nlocal_root = \"/Users/dp/work/other\"\n");
        assert!(hub_copy_of(&root).is_none());

        // local_root IS this dir: a native tree with a manifest beside it. Not a
        // copy — the refusal's way out would be the path it is already standing in.
        write(&format!("schema = 1\nname = \"proj\"\nlocal_root = \"{}\"\n", root.display()));
        assert!(hub_copy_of(&root).is_none());
        // …and a trailing slash is the same path, not a different one
        write(&format!("schema = 1\nname = \"proj\"\nlocal_root = \"{}/\"\n", root.display()));
        assert!(hub_copy_of(&root).is_none());

        // unparseable, and strict-parse rejects (unknown key / bad schema): a file
        // we cannot read is not evidence of anything, so the tree stays usable
        write("this is not toml {{{");
        assert!(hub_copy_of(&root).is_none());
        write("schema = 1\nname = \"proj\"\nlocal_root = \"/elsewhere\"\ninstall_cmd = \"x\"\n");
        assert!(hub_copy_of(&root).is_none());
        write("schema = 99\nname = \"proj\"\nlocal_root = \"/elsewhere\"\n");
        assert!(hub_copy_of(&root).is_none());

        // a root with no parent can't be a copy either
        assert!(hub_copy_of(Path::new("/")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_hub_copy_finding_is_an_error_so_doctor_cannot_report_the_tree_as_fine() {
        let d = tmp("hubcopyfinding");
        let root = d.join("proj");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            d.join(MANIFEST),
            "schema = 1\nname = \"proj\"\nlocal_root = \"/Users/dp/work/proj\"\n",
        )
        .unwrap();
        let f = hub_copy_finding(&root).expect("a finding");
        assert_eq!(f.severity, crate::diag::Severity::Error);
        assert!(matches!(f.code, Code::HubCopy));
        // no host in the manifest ⇒ the message still reads as a sentence
        assert!(f.message.contains("another machine"), "{}", f.message);
        assert!(f.message.contains("/Users/dp/work/proj"), "{}", f.message);
        assert_eq!(crate::diag::Report::new(vec![f]).exit_code(), crate::diag::EXIT_FINDINGS);
        assert!(hub_copy_finding(&d).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The app never passes main.rs's dispatch guard, so the refusal has to live
    /// INSIDE the programmatic API rather than in whoever calls it. Both entry
    /// points stop before rsync is even looked for, which is why this can assert
    /// on the message without a shim: nothing is spawned to reach it.
    #[test]
    fn the_programmatic_api_refuses_a_hub_copy_before_it_touches_rsync() {
        let d = tmp("apiguard");
        let root = d.join("proj");
        std::fs::create_dir_all(&root).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .expect("git init")
            .success());
        std::fs::write(
            d.join(MANIFEST),
            "schema = 1\nname = \"proj\"\nlocal_root = \"/Users/dp/work/proj\"\nhost = \"othermac\"\n",
        )
        .unwrap();

        for dir in [SyncDirection::Push, SyncDirection::Pull] {
            let req = SyncRequest {
                root: root.clone(),
                direction: dir,
                hub: Some(d.join("hub").to_string_lossy().into_owned()),
                with_sessions: Some(false),
            };
            let e = sync_preview(&req).err().expect("preview refused");
            assert!(e.contains("hub copy") && e.contains("/Users/dp/work/proj"), "{e}");
            let e = sync_apply(&req, true, false, None).err().expect("apply refused");
            assert!(e.contains("hub copy"), "{e}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn session_dirs_glob_the_projects_worktree_keys_too() {
        let d = tmp("sessdirs");
        let prefix = escaped_prefix("/w/repo");
        for n in [prefix.clone(), format!("{prefix}--worktrees-feat-x"), "-other".to_string()] {
            std::fs::create_dir_all(d.join(n)).unwrap();
        }
        std::fs::write(d.join("a-file"), "x").unwrap();
        let got: Vec<String> = session_dirs(&d, &prefix).iter().map(|p| basename(p)).collect();
        assert_eq!(got, vec![prefix.clone(), format!("{prefix}--worktrees-feat-x")]);
    }
}
