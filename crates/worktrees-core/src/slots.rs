//! Machine-wide slots for heavy jobs: `worktrees run --slot <name> -- <cmd…>`.
//!
//! # Why this exists
//!
//! On 2026-10-14 an orchestrator nudged eleven lanes in one `tmux send-keys`
//! loop and, within 22 seconds, seven of them started the same uncached
//! monorepo gate — every core each — while five lanes of a SECOND repository
//! were working too. The machine locked up and needed a reboot. Nothing in
//! the tool could have stopped the nudge (it never went through `send`), and
//! a stagger would only have spread seven multi-minute gates over half a
//! minute. The thing that bounds that shape, whoever sends the nudge, is a
//! cap on how many heavy jobs run AT ONCE — so a lane wraps its gate, and the
//! eighth one waits.
//!
//! # Per machine, never per repo
//!
//! The incident spanned two repositories, so the slots live in the user's
//! state dir (`$XDG_STATE_HOME/worktrees/slots/<name>/`), not in a git common
//! dir. Capacity comes from the user's `config.toml` (`[slots]`) and never
//! from a repo — `slots` is in `projcfg`'s `USER_ONLY_KEYS` (ADR 0001).
//!
//! # The lock is the kernel's, so it cannot go stale
//!
//! Slot k of N is the file `k.lock`, held with `flock(LOCK_EX)` for as long as
//! the command runs. The kernel drops a flock when the last descriptor closes,
//! which happens on every kind of death — exit, crash, SIGKILL — so a held
//! slot cannot outlive its process and there is nothing to clean up. The
//! holder's details are written INTO the lock file for display only; a file
//! that can be locked is free whatever it still says.
//!
//! The QUEUE works the same way. A waiter's `wait/<pid>.json` is held with
//! `flock(LOCK_EX)` from before it is renamed into place until the waiter
//! takes a slot or dies, so a file nobody holds is a dead waiter's, however
//! alive its pid looks. It used to be judged by pid, and a waiter killed
//! while waiting left a file whose pid was later reused by a long-lived
//! process: an "older live waiter" that never took anything, in front of a
//! FREE slot, for everyone (review of #485).
//!
//! Not for a networked `$XDG_STATE_HOME`: flock over NFS is not reliable, and
//! a state dir is local by definition.
//!
//! # The descriptor must not be inherited
//!
//! A gate starts daemons (turbo's, metro, gradle) that outlive it. Had the
//! child inherited the lock fd, the first such daemon would hold the slot
//! until reboot. Rust opens files `O_CLOEXEC`; `held_fd_is_cloexec` pins it.
//! The price is that SIGKILLing `worktrees run` itself frees the slot while
//! its child runs on. A kill aimed at the process GROUP — a terminal's ^C —
//! reaches both. Measured with Claude Code on macOS (2026-10-14): its Bash
//! tool does not kill a command that outlives the call's timeout, it moves it
//! to the background, where it keeps running and keeps its slot; stopping
//! that background task took the wrapped child with it and freed the slot.
//!
//! # Waiting is first come, first served
//!
//! Waiters poll every `POLL`. Without an order, seven pollers race for each
//! freed slot and one can lose for the whole afternoon, so a waiter takes a
//! free slot only when fewer live waiters are older than it than there are
//! free slots. That is bounded overtaking, not a strict queue: two newcomers
//! polling in the same instant can both take a slot an elder was about to,
//! delaying it one round — after which it is strictly first. Nobody starves,
//! and making it strict would need a ticket server; do not "fix" it.

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The config table, shared by the reader and `USER_ONLY_KEYS`.
pub const TABLE: &str = "slots";
/// Capacity for a name the user's `[slots]` does not list (and for `gate`).
pub const DEFAULT_CAPACITY: u32 = 2;
/// The upper bound a capacity may take — past this it is not a limit.
const CAPACITY_MAX: u32 = 64;
const POLL: Duration = Duration::from_millis(500);
/// `worktrees run --timeout` expiry: EX_TEMPFAIL, "try again later".
pub const EXIT_TIMEOUT: i32 = 75;

/// What a slot's holder (or a waiter) wrote about itself. Display only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub pid: u32,
    /// `<project>:<slug>` when the command runs inside a managed repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// The worktree the command runs in (git toplevel), for matching places.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default)]
    pub cmd: String,
    /// Epoch milliseconds: when it took the slot, or started waiting.
    #[serde(default)]
    pub since_ms: i64,
    /// The capacity this caller believed in.
    #[serde(default)]
    pub slots: u32,
}

impl Record {
    /// How a line names it: the place, else the pid.
    pub fn who(&self) -> String {
        self.place.clone().unwrap_or_else(|| format!("pid {}", self.pid))
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `$XDG_STATE_HOME/worktrees/slots`.
pub fn root() -> Option<PathBuf> {
    crate::init::state_dir().map(|s| s.join("slots"))
}

/// Slot names become directory names: lowercase words and dashes only.
pub fn valid_name(n: &str) -> Result<(), String> {
    let ok = !n.is_empty()
        && n.len() <= 40
        && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !n.starts_with('-');
    if ok { Ok(()) } else { Err(format!("slot name `{n}` must be lowercase letters, digits and dashes")) }
}

// ── config ──────────────────────────────────────────────────────────────────

/// The capacity for `name` from `config.toml`'s text: `[slots] <name>`, else
/// `[slots] default`, else `DEFAULT_CAPACITY`. Lenient like `[quota]`: a typo
/// is a problem line, never a refusal to run.
pub fn capacity_from(text: &str, name: &str) -> (u32, Vec<String>) {
    let mut problems = Vec::new();
    let root: std::collections::BTreeMap<String, toml::Value> = match toml::from_str(text) {
        Ok(r) => r,
        Err(_) => {
            problems.push("config.toml does not parse, so [slots] could not be read — the defaults apply".into());
            return (DEFAULT_CAPACITY, problems);
        }
    };
    let Some(v) = root.get(TABLE) else { return (DEFAULT_CAPACITY, problems) };
    let Some(t) = v.as_table() else {
        problems.push("slots must be a table ([slots]) — ignored".into());
        return (DEFAULT_CAPACITY, problems);
    };
    let read = |key: &str, problems: &mut Vec<String>| -> Option<u32> {
        let v = t.get(key)?;
        match v.as_integer() {
            Some(n) if (1..=CAPACITY_MAX as i64).contains(&n) => Some(n as u32),
            _ => {
                problems.push(format!("[slots] {key} is not a whole number from 1 to {CAPACITY_MAX} — ignored"));
                None
            }
        }
    };
    let cap = read(name, &mut problems).or_else(|| read("default", &mut problems)).unwrap_or(DEFAULT_CAPACITY);
    (cap, problems)
}

/// The capacity for `name`, read now from the user's config.
pub fn capacity(name: &str) -> (u32, Vec<String>) {
    match fs::read_to_string(crate::config::config_toml_path()) {
        Ok(t) => capacity_from(&t, name),
        Err(_) => (DEFAULT_CAPACITY, Vec::new()),
    }
}

// ── locks ───────────────────────────────────────────────────────────────────

fn flock(f: &File, op: libc::c_int) -> bool {
    // SAFETY: flock receives a live file descriptor owned by `f`.
    unsafe { libc::flock(f.as_raw_fd(), op) == 0 }
}

fn read_record(f: &mut File) -> Option<Record> {
    let mut s = String::new();
    f.seek(std::io::SeekFrom::Start(0)).ok()?;
    f.read_to_string(&mut s).ok()?;
    serde_json::from_str(s.trim()).ok()
}

/// A held slot. Dropping it empties the file (so nothing stale is left to
/// read) and closes the descriptor, which is what releases the lock.
#[derive(Debug)]
pub struct Held {
    file: File,
    pub index: u32,
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
    }
}

#[cfg(test)]
impl Held {
    fn raw_fd(&self) -> i32 {
        self.file.as_raw_fd()
    }
}

fn lock_path(dir: &Path, k: u32) -> PathBuf {
    dir.join(format!("{k}.lock"))
}

/// Try slot `k`: on success write `me` into it — stamped with the moment it
/// was TAKEN, so ages read as "running for", not "around for" — and hand it
/// back.
fn try_take(dir: &Path, k: u32, me: &Record) -> Option<Held> {
    let mut f = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(lock_path(dir, k)).ok()?;
    if !flock(&f, libc::LOCK_EX | libc::LOCK_NB) {
        return None;
    }
    let took = Record { since_ms: now_ms(), ..me.clone() };
    let _ = f.set_len(0);
    let _ = f.seek(std::io::SeekFrom::Start(0));
    let _ = f.write_all(serde_json::to_string(&took).unwrap_or_default().as_bytes());
    let _ = f.flush();
    Some(Held { file: f, index: k })
}

/// Who holds a slot under `dir`, with the index each holds. A lock file that
/// can be share-locked is free, whatever it still says. The probe takes the
/// lock for an instant — an acquirer racing it just waits one more poll.
pub fn holders_in(dir: &Path) -> Vec<(u32, Record)> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(k) = name.strip_suffix(".lock").and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(mut f) = File::open(e.path()) else { continue };
        if flock(&f, libc::LOCK_SH | libc::LOCK_NB) {
            continue; // free
        }
        // Held. The holder may not have written its record yet.
        let rec = read_record(&mut f).unwrap_or_default();
        out.push((k, rec));
    }
    out.sort_by_key(|(k, _)| *k);
    out
}

/// Live waiters under `dir`, oldest first. A wait file nobody holds a lock
/// on is a dead waiter's, and is deleted on sight.
pub fn waiters_in(dir: &Path) -> Vec<Record> {
    use std::os::unix::fs::MetadataExt;
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir.join("wait")) else { return out };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with('.') {
            continue; // a waiter's file before its rename
        }
        let Ok(mut f) = File::open(e.path()) else { continue };
        if flock(&f, libc::LOCK_SH | libc::LOCK_NB) {
            // Nobody holds it: dead. Delete only if the path still names the
            // file we probed — a new waiter reusing the pid may have renamed
            // its own (locked) file over it since.
            let same = match (f.metadata(), fs::metadata(e.path())) {
                (Ok(a), Ok(b)) => a.ino() == b.ino() && a.dev() == b.dev(),
                _ => false,
            };
            if same {
                let _ = fs::remove_file(e.path());
            }
            continue;
        }
        // Held, so live; skip one whose record does not parse rather than
        // guess at it.
        if let Some(rec) = read_record(&mut f) {
            out.push(rec);
        }
    }
    out.sort_by_key(|r| (r.since_ms, r.pid));
    out
}

/// A registered waiter: its file, and the lock on it that says it is alive.
/// Dropping it removes the file and then closes the descriptor; a death
/// skips the removal, and the closed descriptor is what tells a reader.
struct Waiting(PathBuf, #[allow(dead_code)] File);

impl Drop for Waiting {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Locked BEFORE the rename, so the file is never visible unlocked: the lock
/// lives on the inode and survives the rename.
fn register_wait(dir: &Path, me: &Record) -> Option<Waiting> {
    let wd = dir.join("wait");
    fs::create_dir_all(&wd).ok()?;
    let path = wd.join(format!("{}.json", me.pid));
    let tmp = wd.join(format!(".{}.tmp", me.pid));
    let mut f = OpenOptions::new().create(true).truncate(true).read(true).write(true).open(&tmp).ok()?;
    if !flock(&f, libc::LOCK_EX | libc::LOCK_NB) {
        return None;
    }
    f.write_all(serde_json::to_string(me).ok()?.as_bytes()).ok()?;
    fs::rename(&tmp, &path).ok()?;
    Some(Waiting(path, f))
}

/// What a waiting caller is told on each change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitView {
    pub name: String,
    pub capacity: u32,
    pub holders: Vec<Record>,
    /// Live waiters ahead of this one.
    pub ahead: usize,
}

pub fn age(ms: i64) -> String {
    let s = (ms.max(0) / 1000) as u64;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m{:02}s", s / 60, s % 60),
        _ => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

impl WaitView {
    pub fn line(&self, now: i64) -> String {
        let held: Vec<String> = self.holders.iter().map(|r| format!("{} {}", r.who(), age(now - r.since_ms))).collect();
        let queue = if self.ahead > 0 { format!("; {} waiting ahead", self.ahead) } else { String::new() };
        format!(
            "waiting for slot {} ({}/{} held by {}{queue})",
            self.name,
            self.holders.len(),
            self.capacity,
            if held.is_empty() { "nobody yet".into() } else { held.join(", ") },
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AcquireError {
    /// The slot dir could not be made — reason given.
    Io(String),
    /// `timeout` passed with no slot free.
    TimedOut,
}

/// Block until one of `capacity` slots of `dir` is ours. `on_wait` is called
/// whenever what we are waiting behind changes (first time included).
pub fn acquire_in(
    dir: &Path,
    name: &str,
    capacity: u32,
    me: &Record,
    timeout: Option<Duration>,
    mut on_wait: impl FnMut(&WaitView),
) -> Result<Held, AcquireError> {
    fs::create_dir_all(dir).map_err(|e| AcquireError::Io(format!("could not create {}: {e}", dir.display())))?;
    let start = Instant::now();
    let mut waiting: Option<Waiting> = None;
    let mut last: Option<WaitView> = None;
    loop {
        let held = holders_in(dir);
        let held_low = held.iter().filter(|(k, _)| *k < capacity).count() as u32;
        let free = capacity.saturating_sub(held_low) as usize;
        let ahead = waiters_in(dir)
            .iter()
            .filter(|w| w.pid != me.pid && (w.since_ms, w.pid) < (me.since_ms, me.pid))
            .count();
        if ahead < free {
            for k in 0..capacity {
                if let Some(h) = try_take(dir, k, me) {
                    drop(waiting);
                    return Ok(h);
                }
            }
        }
        if timeout.is_some_and(|t| start.elapsed() >= t) {
            return Err(AcquireError::TimedOut);
        }
        if waiting.is_none() {
            waiting = register_wait(dir, me);
        }
        let view = WaitView {
            name: name.to_string(),
            capacity,
            holders: held.into_iter().map(|(_, r)| r).collect(),
            ahead,
        };
        let changed = last.as_ref().is_none_or(|l| {
            l.ahead != view.ahead
                || l.holders.iter().map(|r| r.pid).collect::<Vec<_>>() != view.holders.iter().map(|r| r.pid).collect::<Vec<_>>()
        });
        if changed {
            on_wait(&view);
            last = Some(view);
        }
        let nap = match timeout {
            Some(t) => POLL.min(t.saturating_sub(start.elapsed())).max(Duration::from_millis(10)),
            None => POLL,
        };
        std::thread::sleep(nap);
    }
}

// ── who is asking ───────────────────────────────────────────────────────────

/// `(<project>:<slug>, worktree)` for `cwd`, when it is inside a git repo.
/// The project part is the registered name, else the main checkout's folder.
pub fn place_of(cwd: &Path) -> (Option<String>, Option<String>) {
    let cwd_s = cwd.to_string_lossy().into_owned();
    let Some(top) = crate::git::git_out(&cwd_s, &["rev-parse", "--show-toplevel"]).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    let Ok(p) = crate::project::Project::discover(cwd) else { return (None, Some(top)) };
    let reg = crate::registry::read_lenient();
    let proj = reg
        .by_root(&p.main_root)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| Path::new(&p.main_root).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let slug = if same_path(&top, &p.main_root) {
        "(main)".to_string()
    } else {
        Path::new(&top).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    };
    (Some(format!("{proj}:{slug}")), Some(top))
}

fn same_path(a: &str, b: &str) -> bool {
    let c = |s: &str| fs::canonicalize(s).unwrap_or_else(|_| PathBuf::from(s));
    a == b || c(a) == c(b)
}

// ── what a place is doing ───────────────────────────────────────────────────

/// One slot a worktree holds or waits for — `place_status`'s `slot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaceSlot {
    pub state: &'static str, // "holding" | "waiting"
    pub name: String,
    pub since_ms: i64,
    pub cmd: String,
    /// For a waiter: who holds the slots it waits for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub held_by: Vec<String>,
}

/// Every slot `worktree` holds or waits for, under the slot root `root`.
pub fn for_worktree_in(root: &Path, worktree: &str) -> Vec<PlaceSlot> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else { return out };
    let mine = |r: &Record| r.worktree.as_deref().is_some_and(|w| same_path(w, worktree));
    let mut names: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).collect();
    names.sort_by_key(|e| e.file_name());
    for e in names {
        let name = e.file_name().to_string_lossy().into_owned();
        let held = holders_in(&e.path());
        for (_, r) in held.iter().filter(|(_, r)| mine(r)) {
            out.push(PlaceSlot { state: "holding", name: name.clone(), since_ms: r.since_ms, cmd: r.cmd.clone(), held_by: Vec::new() });
        }
        for r in waiters_in(&e.path()).iter().filter(|r| mine(r)) {
            out.push(PlaceSlot {
                state: "waiting",
                name: name.clone(),
                since_ms: r.since_ms,
                cmd: r.cmd.clone(),
                held_by: held.iter().map(|(_, h)| h.who()).collect(),
            });
        }
    }
    out
}

pub fn for_worktree(worktree: &str) -> Vec<PlaceSlot> {
    root().map(|r| for_worktree_in(&r, worktree)).unwrap_or_default()
}

// ── `worktrees slots` ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SlotState {
    pub name: String,
    pub capacity: u32,
    pub holders: Vec<Record>,
    pub waiters: Vec<Record>,
}

pub fn states_in(root: &Path, cap: impl Fn(&str) -> u32) -> Vec<SlotState> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else { return out };
    let mut dirs: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).collect();
    dirs.sort_by_key(|e| e.file_name());
    for e in dirs {
        let name = e.file_name().to_string_lossy().into_owned();
        let holders: Vec<Record> = holders_in(&e.path()).into_iter().map(|(_, r)| r).collect();
        let waiters = waiters_in(&e.path());
        if holders.is_empty() && waiters.is_empty() {
            continue;
        }
        out.push(SlotState { capacity: cap(&name), name, holders, waiters });
    }
    out
}

pub fn cmd_slots(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    if let Some(bad) = args.iter().find(|a| *a != "--json") {
        eprintln!("{}", crate::render::error_line(&format!("slots: unknown argument {bad} (usage: worktrees slots [--json])")));
        return 1;
    }
    let states = root().map(|r| states_in(&r, |n| capacity(n).0)).unwrap_or_default();
    if json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "slots": states })).unwrap_or_default());
        return 0;
    }
    if states.is_empty() {
        println!("no slots in use");
        return 0;
    }
    let now = now_ms();
    for s in &states {
        println!("{}  {}/{} held", s.name, s.holders.len(), s.capacity);
        for h in &s.holders {
            println!("  holds  {}  pid {}  {}  {}", h.who(), h.pid, age(now - h.since_ms), h.cmd);
        }
        for w in &s.waiters {
            println!("  waits  {}  pid {}  {}  {}", w.who(), w.pid, age(now - w.since_ms), w.cmd);
        }
        let mut odd: Vec<u32> = s.holders.iter().chain(&s.waiters).map(|r| r.slots).filter(|n| *n != 0 && *n != s.capacity).collect();
        odd.sort_unstable();
        odd.dedup();
        if !odd.is_empty() {
            let asked: Vec<String> = odd.iter().map(|n| format!("--slots {n}")).collect();
            println!(
                "  note: callers here asked for {} while your config gives {}; set [slots] {} = N in config.toml so every caller agrees",
                asked.join(", "),
                s.capacity,
                s.name
            );
        }
    }
    0
}

// ── `worktrees run` ─────────────────────────────────────────────────────────

const RUN_USAGE: &str = "usage: worktrees run --slot <name> [--slots N] [--timeout SECS] -- <cmd> [args…]";

#[derive(Debug, PartialEq, Eq)]
struct RunArgs {
    slot: String,
    slots: Option<u32>,
    timeout: Option<u64>,
    cmd: Vec<String>,
}

fn parse_run(args: &[String]) -> Result<RunArgs, String> {
    let mut slot = None;
    let mut slots = None;
    let mut timeout = None;
    let mut i = 0;
    let value = |i: usize, flag: &str| args.get(i + 1).cloned().ok_or_else(|| format!("{flag} needs a value"));
    while i < args.len() {
        let a = args[i].as_str();
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a, None),
        };
        let take = |i: &mut usize| -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => {
                    let v = value(*i, flag)?;
                    *i += 1;
                    Ok(v)
                }
            }
        };
        match flag {
            "--" => {
                i += 1;
                break;
            }
            "--slot" => slot = Some(take(&mut i)?),
            "--slots" => {
                let v = take(&mut i)?;
                slots = Some(v.parse::<u32>().ok().filter(|n| (1..=CAPACITY_MAX).contains(n))
                    .ok_or_else(|| format!("--slots must be a whole number from 1 to {CAPACITY_MAX}"))?);
            }
            "--timeout" => {
                let v = take(&mut i)?;
                timeout = Some(v.parse::<u64>().map_err(|_| "--timeout takes whole seconds".to_string())?);
            }
            _ if a.starts_with("--") => return Err(format!("unknown flag {a}")),
            _ => break,
        }
        i += 1;
    }
    let cmd: Vec<String> = args[i.min(args.len())..].to_vec();
    let slot = slot.ok_or("--slot <name> is required")?;
    valid_name(&slot)?;
    if cmd.is_empty() {
        return Err("no command to run".into());
    }
    Ok(RunArgs { slot, slots, timeout, cmd })
}

use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

static CHILD: AtomicI32 = AtomicI32::new(0);
/// A signal that arrived before the child existed, to pass on once it does.
static PENDING: AtomicI32 = AtomicI32::new(0);
static INTS: AtomicU32 = AtomicU32::new(0);

/// TERM and HUP are passed on to the child, and we keep waiting for it: the
/// slot is freed when the WORK stops, not when we are asked to. The FIRST INT
/// is not passed on — a terminal's ^C already went to the whole process
/// group, and a second one makes some tools (pnpm, turbo) abandon their own
/// cleanup. A SECOND ^C means the child is ignoring them, so it gets a TERM:
/// otherwise a child that traps INT makes `run` impossible to interrupt.
extern "C" fn forward(sig: libc::c_int) {
    let sig = if sig == libc::SIGINT {
        if INTS.fetch_add(1, Ordering::SeqCst) == 0 {
            return;
        }
        libc::SIGTERM
    } else {
        sig
    };
    let pid = CHILD.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill is async-signal-safe.
        unsafe { libc::kill(pid, sig) };
    } else {
        PENDING.store(sig, Ordering::SeqCst);
    }
}

pub fn cmd_run(args: &[String]) -> i32 {
    let ra = match parse_run(args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", crate::render::error_line(&format!("run: {e}")));
            eprintln!("{RUN_USAGE}");
            return 2;
        }
    };
    let (configured, problems) = capacity(&ra.slot);
    for p in problems {
        eprintln!("worktrees: {p}");
    }
    let cap = ra.slots.unwrap_or(configured);
    let Some(root) = root() else {
        eprintln!("{}", crate::render::error_line("run: no HOME or XDG_STATE_HOME to keep slots in"));
        return 1;
    };
    let dir = root.join(&ra.slot);
    let cwd = std::env::current_dir().unwrap_or_default();
    let (place, worktree) = place_of(&cwd);
    let me = Record {
        pid: std::process::id(),
        place,
        worktree,
        cmd: ra.cmd.join(" "),
        since_ms: now_ms(),
        slots: cap,
    };
    let mut waited = false;
    let got = acquire_in(&dir, &ra.slot, cap, &me, ra.timeout.map(Duration::from_secs), |v| {
        waited = true;
        eprintln!("worktrees: {}", v.line(now_ms()));
    });
    let held = match got {
        Ok(h) => h,
        Err(AcquireError::TimedOut) => {
            eprintln!(
                "worktrees: slot {} still busy after {}s; nothing ran",
                ra.slot,
                ra.timeout.unwrap_or(0)
            );
            return EXIT_TIMEOUT;
        }
        Err(AcquireError::Io(e)) => {
            eprintln!("{}", crate::render::error_line(&format!("run: {e}")));
            return 1;
        }
    };
    if waited {
        eprintln!("worktrees: got slot {} ({}/{}) after {}", ra.slot, held.index + 1, cap, age(now_ms() - me.since_ms));
    }
    // SAFETY: installing a handler that only calls kill(2).
    unsafe {
        libc::signal(libc::SIGTERM, forward as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, forward as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, forward as *const () as libc::sighandler_t);
    }
    let mut child = match std::process::Command::new(&ra.cmd[0]).args(&ra.cmd[1..]).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", crate::render::error_line(&format!("run: could not start {}: {e}", ra.cmd[0])));
            return 127;
        }
    };
    CHILD.store(child.id() as i32, Ordering::SeqCst);
    // A TERM that landed between the handlers going in and the line above
    // found no child to pass on to; it is owed now.
    let owed = PENDING.swap(0, Ordering::SeqCst);
    if owed != 0 {
        // SAFETY: signalling the child we just spawned.
        unsafe { libc::kill(child.id() as libc::pid_t, owed) };
    }
    let status = loop {
        match child.wait() {
            Ok(s) => break s,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("{}", crate::render::error_line(&format!("run: {e}")));
                return 1;
            }
        }
    };
    drop(held);
    use std::os::unix::process::ExitStatusExt;
    status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-slots-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn rec(pid: u32, since: i64) -> Record {
        Record { pid, place: Some(format!("p:{pid}")), worktree: None, cmd: "x".into(), since_ms: since, slots: 2 }
    }

    #[test]
    fn capacity_reads_the_name_then_default_then_two() {
        assert_eq!(capacity_from("", "gate").0, DEFAULT_CAPACITY);
        assert_eq!(capacity_from("[slots]\ndefault = 3\n", "gate").0, 3);
        assert_eq!(capacity_from("[slots]\ndefault = 3\ngate = 1\n", "gate").0, 1);
        let (n, p) = capacity_from("[slots]\ngate = 0\n", "gate");
        assert_eq!(n, DEFAULT_CAPACITY);
        assert_eq!(p.len(), 1, "{p:?}");
        let (n, p) = capacity_from("slots = 4\n", "gate");
        assert_eq!((n, p.len()), (DEFAULT_CAPACITY, 1));
    }

    #[test]
    fn slot_names_are_path_safe() {
        assert!(valid_name("gate").is_ok());
        assert!(valid_name("e2e-ios").is_ok());
        for bad in ["", "../x", "Gate", "a/b", "-x", "a b"] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn run_args_parse() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse_run(&s(&["--slot", "gate", "--", "pnpm", "gate"])).unwrap(),
            RunArgs { slot: "gate".into(), slots: None, timeout: None, cmd: s(&["pnpm", "gate"]) }
        );
        assert_eq!(
            parse_run(&s(&["--slot=gate", "--slots", "3", "--timeout=5", "make", "--test"])).unwrap(),
            RunArgs { slot: "gate".into(), slots: Some(3), timeout: Some(5), cmd: s(&["make", "--test"]) }
        );
        assert!(parse_run(&s(&["--", "make"])).is_err(), "slot is required");
        assert!(parse_run(&s(&["--slot", "gate", "--"])).is_err(), "command is required");
        assert!(parse_run(&s(&["--slot", "gate", "--slots", "0", "--", "x"])).is_err());
        assert!(parse_run(&s(&["--slot", "gate", "--bogus", "--", "x"])).is_err());
    }

    #[test]
    fn a_held_slot_is_seen_and_a_dropped_one_is_free() {
        let d = tmp("held");
        let a = try_take(&d, 0, &rec(11, 1)).expect("free slot");
        assert!(try_take(&d, 0, &rec(12, 2)).is_none(), "an flock is exclusive across descriptors");
        let h = holders_in(&d);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].1.pid, 11);
        drop(a);
        // Not the instant after: this is a multi-threaded test process, and a
        // subprocess another test forks in that instant shares the open file
        // description — and with it the lock — until its exec closes the
        // CLOEXEC fd. Seen once in ~7 full-suite runs. `worktrees run` forks
        // only its own child, so production has no such window.
        let start = Instant::now();
        while !holders_in(&d).is_empty() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(holders_in(&d).is_empty(), "dropping releases");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn held_fd_is_cloexec() {
        let d = tmp("cloexec");
        let h = try_take(&d, 0, &rec(1, 1)).unwrap();
        // SAFETY: F_GETFD on a live descriptor.
        let flags = unsafe { libc::fcntl(h.raw_fd(), libc::F_GETFD) };
        assert!(flags & libc::FD_CLOEXEC != 0, "a child must never inherit the slot");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn acquire_takes_a_free_slot_and_times_out_on_a_full_one() {
        let d = tmp("acq");
        let me = rec(std::process::id(), now_ms());
        let a = acquire_in(&d, "t", 2, &me, Some(Duration::from_millis(50)), |_| panic!("must not wait")).unwrap();
        let b = acquire_in(&d, "t", 2, &me, Some(Duration::from_millis(50)), |_| panic!("must not wait")).unwrap();
        assert_ne!(a.index, b.index);
        let mut saw = Vec::new();
        let r = acquire_in(&d, "t", 2, &me, Some(Duration::from_millis(300)), |v| saw.push(v.clone()));
        assert_eq!(r.unwrap_err(), AcquireError::TimedOut);
        assert_eq!(saw.len(), 1, "told once, not every poll");
        assert_eq!(saw[0].holders.len(), 2);
        assert!(waiters_in(&d).is_empty(), "a waiter that gave up leaves no file");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn an_older_live_waiter_goes_first() {
        let d = tmp("fifo");
        // Our own parent is alive and older than us in the queue.
        let elder = Record { pid: std::os::unix::process::parent_id(), since_ms: 1, ..rec(0, 0) };
        let _w = register_wait(&d, &elder).unwrap();
        let me = rec(std::process::id(), now_ms());
        let r = acquire_in(&d, "t", 1, &me, Some(Duration::from_millis(200)), |v| assert_eq!(v.ahead, 1));
        assert_eq!(r.unwrap_err(), AcquireError::TimedOut, "a free slot belongs to the elder");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_dead_waiter_is_dropped_and_does_not_block() {
        // A waiter killed while waiting leaves its file, and its pid can be
        // reused by a live process (review B1). Liveness must come from the
        // waiter's LOCK, not its pid: plant a file naming a LIVE pid (our
        // parent) that nobody holds a lock on.
        let d = tmp("dead");
        let ghost = Record { pid: std::os::unix::process::parent_id(), since_ms: 1, ..rec(0, 0) };
        fs::create_dir_all(d.join("wait")).unwrap();
        fs::write(d.join("wait").join(format!("{}.json", ghost.pid)), serde_json::to_string(&ghost).unwrap()).unwrap();
        let me = rec(std::process::id(), now_ms());
        assert!(acquire_in(&d, "t", 1, &me, Some(Duration::from_millis(50)), |_| {}).is_ok());
        assert!(!d.join("wait").join(format!("{}.json", ghost.pid)).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn place_slots_match_by_worktree() {
        let root = tmp("place");
        let gate = root.join("gate");
        fs::create_dir_all(&gate).unwrap();
        let wt = root.to_string_lossy().into_owned();
        let mine = Record { worktree: Some(wt.clone()), ..rec(5, 1) };
        let _h = try_take(&gate, 0, &mine).unwrap();
        let other = Record { worktree: Some("/elsewhere".into()), ..rec(6, 1) };
        let _o = try_take(&gate, 1, &other).unwrap();
        let s = for_worktree_in(&root, &wt);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].state, s[0].name.as_str()), ("holding", "gate"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_waiting_line_names_holders_and_ages() {
        let v = WaitView { name: "gate".into(), capacity: 2, holders: vec![rec(1, 0), rec(2, 160_000)], ahead: 0 };
        assert_eq!(v.line(200_000), "waiting for slot gate (2/2 held by p:1 3m20s, p:2 40s)");
    }
}
