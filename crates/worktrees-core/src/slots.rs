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
//! dir. Capacity comes from the user's `config.toml` (`[slots]`, written by
//! `worktrees slots set` and Settings) and never from a repo — `slots` is in
//! `projcfg`'s `USER_ONLY_KEYS` (ADR 0001). With nothing set, it is sized from
//! the machine's cores and memory (`machine_capacity`, measured below).
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
//! CLOEXEC acts at EXEC: a process forked by another thread shares the lock
//! until it execs, which every spawn here does at once. Never add a fork that
//! does not exec (a detached helper) while a slot or launch-lock `File` may be
//! alive on another thread.
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
/// The key every name the table does not list falls back to.
pub const DEFAULT_KEY: &str = "default";
/// The slot every lane's gate takes (`worktrees run --slot gate`).
pub const GATE: &str = "gate";
/// Capacity when neither the config nor the machine can say (`built-in`).
pub const BUILT_IN: u32 = 2;
/// The upper bound a capacity may take — past this it is not a limit.
pub const CAPACITY_MAX: u32 = 64;
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

/// A capacity a caller may ask for: `1..=CAPACITY_MAX`.
pub fn valid_capacity(n: u32) -> Result<(), String> {
    if (1..=CAPACITY_MAX).contains(&n) { Ok(()) } else { Err(format!("capacity must be a whole number from 1 to {CAPACITY_MAX} (got {n})")) }
}

// ── the machine ─────────────────────────────────────────────────────────────

const GIB: u64 = 1 << 30;

/// What the machine-sized default is computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    /// Cores a gate can use at full speed: macOS's performance cores
    /// (`hw.perflevel0.physicalcpu`, else `hw.physicalcpu` on a machine with
    /// one kind), Linux's `available_parallelism`, which counts an affinity
    /// mask AND a cgroup CPU quota — more than `nproc` does: under `docker
    /// --cpus 4` on a 14-core host `nproc` printed 14 and this reads 4.
    pub cores: u32,
    /// Physical memory: `hw.memsize`, `/proc/meminfo` MemTotal (which is the
    /// host's or VM's, not a container's cgroup limit).
    pub ram_bytes: u64,
}

impl Machine {
    /// Whole GiB, rounded down — what a label says and the formula reads.
    pub fn ram_gib(&self) -> u64 {
        self.ram_bytes / GIB
    }

    /// The default capacity this machine gets.
    pub fn capacity(&self) -> u32 {
        machine_capacity(self.cores, self.ram_bytes)
    }
}

// The machine default, and what each number stands on. Measured on 2026-10-22
// with this repository's own full gate (AGENTS.md's list, cold: a fresh clone
// and an empty target/) on a 10+4-core, 36 GiB Mac that was already carrying
// ~27 GiB of other work — about fifteen agent sessions, which is the normal
// load here (`guard` exists because of it):
//
// - one gate: 7m34s, peak 3.2 GiB summed over its process tree (2.7 GiB on a
//   second run); 69% of the wall is the bats suite (serial, ~60 MiB), the
//   rest compiles that load every core. Over its whole life it averaged 1.65
//   busy cores (834 s of user+sys CPU in a 505 s run);
// - two at once: 8m57s each (×1.18 wall, ×1.69 throughput), peak 5.3 GiB
//   for the pair; the compile steps ran 1.6–2.1× slower, bats 1.07×, and
//   swap grew by 1.3 GB.
//
// So memory decides first: a GATE_RAM_PCT share of RAM — guard's default
// agent cap (half the RAM in GiB, at ~1.2 GB a session) leaves about that
// much for the work the sessions start — at GIB_PER_GATE each, the highest
// peak seen (3.3 GiB) with a margin for a heavier branch and a spike between
// samples. Cores decide next: CORES_PER_GATE is about twice that 1.65-core
// average, so the gates' average demand stays near half the fast cores and
// the rest is left to the agent sessions, which are the user's actual work;
// and when the compiles coincide, as they do when lanes start together, each
// shares the machine with at most a third as many gates as there are cores.
// MACHINE_MAX caps what was not measured: past two, each further gate buys
// less throughput (only the compiles overlap badly, and they are what grows)
// while memory grows by a whole gate; the 2026-10-14 freeze was seven.
// `[slots]` goes higher on a machine its owner knows can take it.
const GATE_RAM_PCT: u64 = 40;
const GIB_PER_GATE: u64 = 4;
const CORES_PER_GATE: u32 = 3;
/// The most the machine default ever gives; past it only `[slots]` goes.
pub const MACHINE_MAX: u32 = 4;

/// The machine default: the lower of `cores / CORES_PER_GATE` and
/// `RAM GiB × GATE_RAM_PCT% / GIB_PER_GATE`, between 1 and `MACHINE_MAX`. A
/// pure function of the two numbers, so the table test is the whole argument.
pub fn machine_capacity(cores: u32, ram_bytes: u64) -> u32 {
    let by_cpu = cores / CORES_PER_GATE;
    let by_mem = (ram_bytes / GIB) * GATE_RAM_PCT / 100 / GIB_PER_GATE;
    by_cpu.min(by_mem.min(u64::from(MACHINE_MAX)) as u32).clamp(1, MACHINE_MAX)
}

/// The machine, read once per process. The test seam is
/// `WORKTREES_SLOTS_PROBE` (guard's `WORKTREES_GUARD_PROBE` shape): `off`
/// reads nothing, which is the `built-in` answer, and the bats suite pins it so
/// a test's capacity does not depend on the runner; a path reads a JSON
/// `Machine` from that file. Unset reads the machine.
pub fn machine() -> Option<Machine> {
    match std::env::var("WORKTREES_SLOTS_PROBE").ok().filter(|s| !s.is_empty()) {
        Some(v) if v == "off" => None,
        Some(path) => fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()),
        None => {
            static READ: std::sync::OnceLock<Option<Machine>> = std::sync::OnceLock::new();
            *READ.get_or_init(read_machine)
        }
    }
}

/// Through libc, never a subprocess, like guard's memory reading.
#[cfg(target_os = "macos")]
fn read_machine() -> Option<Machine> {
    let cores = crate::guard::sysctl::<u32>("hw.perflevel0.physicalcpu")
        .filter(|n| *n > 0)
        .or_else(|| crate::guard::sysctl::<u32>("hw.physicalcpu").filter(|n| *n > 0))?;
    let ram_bytes = crate::guard::sysctl::<u64>("hw.memsize").filter(|b| *b > 0)?;
    Some(Machine { cores, ram_bytes })
}

#[cfg(not(target_os = "macos"))]
fn read_machine() -> Option<Machine> {
    let cores = std::thread::available_parallelism().ok()?.get() as u32;
    let ram_bytes = fs::read_to_string("/proc/meminfo").ok().and_then(|t| crate::guard::meminfo(&t).ram_bytes)?;
    Some(Machine { cores, ram_bytes })
}

// ── config ──────────────────────────────────────────────────────────────────

/// Where a slot's capacity came from — what `worktrees slots` and Settings say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// `[slots] <key>`: the slot's own name, or `default`.
    Config(String),
    Machine(Machine),
    /// Neither the config nor the machine could say: `BUILT_IN`.
    BuiltIn,
}

impl Source {
    /// `config`, `machine` or `built-in`: what a client switches on.
    pub fn kind(&self) -> &'static str {
        match self {
            Source::Config(_) => "config",
            Source::Machine(_) => "machine",
            Source::BuiltIn => "built-in",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Source::Config(key) => format!("config [{TABLE}] {key}"),
            Source::Machine(m) => format!("machine ({} cores, {} GiB)", m.cores, m.ram_gib()),
            Source::BuiltIn => "built-in".into(),
        }
    }
}

/// A slot's capacity, where it came from, and what was wrong on the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub n: u32,
    pub source: Source,
    pub problems: Vec<String>,
}

impl Capacity {
    /// Set for this very name, i.e. what `slots unset <name>` would remove.
    pub fn explicit(&self, name: &str) -> bool {
        matches!(&self.source, Source::Config(k) if k == name)
    }
}

/// `[slots]` from `config.toml`'s text, or why there is none to read.
fn table_from(text: &str, problems: &mut Vec<String>) -> Option<toml::Table> {
    let root: toml::Table = match toml::from_str(text) {
        Ok(r) => r,
        Err(_) => {
            problems.push("config.toml does not parse, so [slots] could not be read — the defaults apply".into());
            return None;
        }
    };
    match root.get(TABLE)? {
        toml::Value::Table(t) => Some(t.clone()),
        _ => {
            problems.push("slots must be a table ([slots]) — ignored".into());
            None
        }
    }
}

/// One key's capacity, or a problem line when it is set to nonsense.
fn read_key(t: &toml::Table, key: &str, problems: &mut Vec<String>) -> Option<u32> {
    let v = t.get(key)?;
    match v.as_integer() {
        Some(n) if (1..=CAPACITY_MAX as i64).contains(&n) => Some(n as u32),
        _ => {
            problems.push(format!("[slots] {key} is not a whole number from 1 to {CAPACITY_MAX} — ignored"));
            None
        }
    }
}

/// What a name gets once its own key is out of the way: `[slots] default`
/// (for any name but `default` itself), else the machine's, else `BUILT_IN`.
fn fallback(t: Option<&toml::Table>, name: &str, machine: Option<Machine>, problems: &mut Vec<String>) -> (u32, Source) {
    if name != DEFAULT_KEY {
        if let Some(n) = t.and_then(|t| read_key(t, DEFAULT_KEY, problems)) {
            return (n, Source::Config(DEFAULT_KEY.into()));
        }
    }
    match machine {
        Some(m) => (m.capacity(), Source::Machine(m)),
        None => (BUILT_IN, Source::BuiltIn),
    }
}

/// The capacity for `name` from `config.toml`'s text: `[slots] <name>`, else
/// `[slots] default`, else the machine's, else `BUILT_IN`. An explicit entry
/// always wins, whatever the machine says. Lenient like `[quota]`: a typo is
/// a problem line, never a refusal to run.
pub fn resolve(text: &str, name: &str, machine: Option<Machine>) -> Capacity {
    let mut problems = Vec::new();
    let t = table_from(text, &mut problems);
    if let Some(n) = t.as_ref().and_then(|t| read_key(t, name, &mut problems)) {
        return Capacity { n, source: Source::Config(name.into()), problems };
    }
    let (n, source) = fallback(t.as_ref(), name, machine, &mut problems);
    Capacity { n, source, problems }
}

/// The user's config.toml, "" when there is none.
fn config_text() -> String {
    fs::read_to_string(crate::config::config_toml_path()).unwrap_or_default()
}

/// The capacity for `name`, read now from the user's config and the machine.
pub fn capacity(name: &str) -> Capacity {
    resolve(&config_text(), name, machine())
}

// ── writing [slots] ─────────────────────────────────────────────────────────

/// Set (`Some`) or remove (`None`) `[slots] <name>` in the user's config.toml —
/// the user's act only: the CLI verb and Settings; the MCP server never calls
/// it. Returns whether anything changed.
pub fn set_capacity(name: &str, n: Option<u32>) -> Result<bool, String> {
    set_capacity_at(&crate::config::config_toml_path(), name, n)
}

pub fn set_capacity_at(path: &Path, name: &str, n: Option<u32>) -> Result<bool, String> {
    valid_name(name)?;
    if let Some(n) = n {
        valid_capacity(n)?;
    }
    crate::config::edit_user_config(path, |text| {
        let root: toml::Table = toml::from_str(text).map_err(|_| unparsable())?;
        let cur = root.get(TABLE).and_then(|v| v.as_table()).and_then(|t| t.get(name));
        let same = match (cur, n) {
            (None, None) => true,
            (Some(v), Some(n)) => v.as_integer() == Some(i64::from(n)),
            _ => false,
        };
        if same {
            return Ok(None);
        }
        with_slot(text, name, n).map(Some)
    })
}

fn unparsable() -> String {
    "config.toml does not parse — fix it by hand before changing [slots]".into()
}

/// `text` with `[slots] <name>` set to `n`, or removed for `None`. Pure, for
/// the tests.
///
/// Like `quota::with_policy`: edits the key's VALUE in place, so its inline
/// comment and indentation survive; keeps the file's line ending; adds a
/// missing key after the table's last key (or under its header), or the table
/// at the end. Every other line is kept byte for byte, and the result must
/// re-parse to exactly the old document with this one key changed, or nothing
/// is written — so a dotted `slots.gate = …`, an inline `slots = {…}` or a
/// quoted `"gate" = …` refuses rather than guess.
pub fn with_slot(text: &str, name: &str, n: Option<u32>) -> Result<String, String> {
    valid_name(name)?;
    if let Some(n) = n {
        valid_capacity(n)?;
    }
    let before: toml::Table = toml::from_str(text).map_err(|_| unparsable())?;
    let refuse = || format!("could not set [slots] {name} in your config.toml safely (an unusual layout?) — edit it by hand");
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let is_ours = |l: &str| l.trim_start().strip_prefix(name).is_some_and(|r| r.trim_start().starts_with('='));
    let mut out: Vec<String> = Vec::new();
    let mut in_slots = false;
    // Where a missing key goes: after the table's last key, else under its header.
    let mut header: Option<usize> = None;
    let mut last_key: Option<usize> = None;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with('[') {
            // `[slots]  # note` is ours; `[slots.sub]` and `[[slots]]` are not.
            in_slots = t.split('#').next().unwrap_or("").trim_end() == "[slots]";
            out.push(l.to_string());
            if in_slots && header.is_none() {
                header = Some(out.len());
            }
            continue;
        }
        if in_slots && is_ours(l) {
            if let Some(n) = n {
                let indent = &l[..l.len() - t.len()];
                let after_eq = &t[t.find('=').map_or(t.len(), |i| i + 1)..];
                // From the whitespace run before the first `#` that follows
                // whitespace, so the comment keeps its alignment.
                let comment = after_eq.find(" #").or_else(|| after_eq.find("\t#")).map_or("", |i| {
                    let start = after_eq[..i].trim_end_matches([' ', '\t']).len();
                    &after_eq[start..]
                });
                out.push(format!("{indent}{name} = {n}{comment}"));
                last_key = Some(out.len());
            }
            continue;
        }
        out.push(l.to_string());
        if in_slots && !t.is_empty() && !t.starts_with('#') {
            last_key = Some(out.len());
        }
    }
    let had = before.get(TABLE).and_then(|v| v.as_table()).is_some_and(|t| t.contains_key(name));
    if let (Some(n), false) = (n, had) {
        let line = format!("{name} = {n}");
        match last_key.or(header) {
            Some(at) => out.insert(at, line),
            None => {
                if out.last().is_some_and(|l| !l.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push(format!("[{TABLE}]"));
                out.push(line);
            }
        }
    }
    let mut new = out.join(eol);
    new.push_str(eol);
    let after: toml::Table = toml::from_str(&new).map_err(|_| refuse())?;
    let mut want = before;
    match n {
        Some(n) => {
            let slots = want.entry(TABLE).or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let Some(slots) = slots.as_table_mut() else { return Err(refuse()) };
            slots.insert(name.into(), toml::Value::Integer(i64::from(n)));
        }
        None => {
            if let Some(slots) = want.get_mut(TABLE).and_then(|v| v.as_table_mut()) {
                slots.remove(name);
            }
        }
    }
    if after != want {
        return Err(refuse());
    }
    Ok(new)
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

/// One slot as `worktrees slots` and Settings show it.
#[derive(Debug, Serialize)]
pub struct SlotState {
    pub name: String,
    pub capacity: u32,
    /// Where `capacity` came from (`Source::label`).
    pub source: String,
    /// `[slots] <name>` is set, so `slots unset <name>` would change it.
    pub explicit: bool,
    /// What the slot gets without an entry of its own, and from where.
    pub fallback: u32,
    pub fallback_source: String,
    /// `fallback_source`'s `Source::kind`.
    pub fallback_kind: &'static str,
    pub holders: Vec<Record>,
    pub waiters: Vec<Record>,
}

/// The machine, as the status view describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineView {
    pub cores: u32,
    pub ram_gib: u64,
    /// What the machine default comes to here.
    pub capacity: u32,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    /// `gate` first, then every name `[slots]` lists and (`in_use`) every
    /// slot someone holds or waits for, sorted, and `default` last.
    pub slots: Vec<SlotState>,
    pub machine: Option<MachineView>,
    pub problems: Vec<String>,
}

/// Every slot worth showing, under the slot root `root` (`None`: no state
/// dir, so nothing is held). One derivation for the CLI and Settings.
pub fn overview_in(root: Option<&Path>, text: &str, machine: Option<Machine>, in_use: bool) -> Overview {
    let mut problems = Vec::new();
    let t = table_from(text, &mut problems);
    let mut names: Vec<String> = Vec::new();
    for k in t.iter().flat_map(|t| t.keys()) {
        match valid_name(k) {
            Ok(()) => names.push(k.clone()),
            Err(_) => problems.push(format!("[slots] `{k}` is not a slot name (lowercase letters, digits and dashes) — ignored")),
        }
    }
    let state = |name: &str| -> (Vec<Record>, Vec<Record>) {
        match root {
            Some(r) => (holders_in(&r.join(name)).into_iter().map(|(_, r)| r).collect(), waiters_in(&r.join(name))),
            None => (Vec::new(), Vec::new()),
        }
    };
    if in_use {
        if let Some(rd) = root.and_then(|r| fs::read_dir(r).ok()) {
            for e in rd.flatten().filter(|e| e.path().is_dir()) {
                let name = e.file_name().to_string_lossy().into_owned();
                let (h, w) = state(&name);
                if !h.is_empty() || !w.is_empty() {
                    names.push(name);
                }
            }
        }
    }
    names.retain(|n| n != GATE && n != DEFAULT_KEY);
    names.sort();
    names.dedup();
    names.insert(0, GATE.into());
    names.push(DEFAULT_KEY.into());
    let mut slots = Vec::new();
    for name in names {
        let c = resolve(text, &name, machine);
        let (fallback_n, fallback_source) = fallback(t.as_ref(), &name, machine, &mut Vec::new());
        problems.extend(c.problems.iter().cloned());
        let (holders, waiters) = state(&name);
        slots.push(SlotState {
            explicit: c.explicit(&name),
            capacity: c.n,
            source: c.source.label(),
            fallback: fallback_n,
            fallback_source: fallback_source.label(),
            fallback_kind: fallback_source.kind(),
            name,
            holders,
            waiters,
        });
    }
    let mut seen = std::collections::HashSet::new();
    problems.retain(|p| seen.insert(p.clone()));
    Overview {
        slots,
        machine: machine.map(|m| MachineView { cores: m.cores, ram_gib: m.ram_gib(), capacity: m.capacity() }),
        problems,
    }
}

/// `overview_in` for this user, now.
pub fn overview(in_use: bool) -> Overview {
    overview_in(root().as_deref(), &config_text(), machine(), in_use)
}

const SLOTS_USAGE: &str = "usage: worktrees slots [--json] | worktrees slots set <name> <N> | worktrees slots unset <name>";

pub fn cmd_slots(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("set") => return cmd_set(&args[1..], true),
        Some("unset") => return cmd_set(&args[1..], false),
        _ => {}
    }
    let json = args.iter().any(|a| a == "--json");
    if let Some(bad) = args.iter().find(|a| *a != "--json") {
        eprintln!("{}", crate::render::error_line(&format!("slots: unknown argument {bad}")));
        eprintln!("{SLOTS_USAGE}");
        return 1;
    }
    let ov = overview(true);
    if json {
        println!("{}", serde_json::to_string_pretty(&ov).unwrap_or_default());
        return 0;
    }
    let now = now_ms();
    let w = ov.slots.iter().map(|s| s.name.len()).max().unwrap_or(0);
    for s in &ov.slots {
        if s.name == DEFAULT_KEY && s.holders.is_empty() && s.waiters.is_empty() {
            println!("{:<w$}  {} for any other name  ({})", s.name, s.capacity, s.source);
        } else {
            println!("{:<w$}  {}/{} held  ({})", s.name, s.holders.len(), s.capacity, s.source);
        }
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
                "  note: callers here asked for {} while this machine gives {}; run `worktrees slots set {} N` so every caller agrees",
                asked.join(", "),
                s.capacity,
                s.name
            );
        }
    }
    for p in &ov.problems {
        println!("problem: {p}");
    }
    if ov.slots.iter().all(|s| s.holders.is_empty() && s.waiters.is_empty()) {
        println!("no slots in use");
    }
    0
}

/// `worktrees slots set <name> <N>` (`set`) and `worktrees slots unset <name>`.
fn cmd_set(args: &[String], set: bool) -> i32 {
    let verb = if set { "set" } else { "unset" };
    let fail = |e: &str| {
        eprintln!("{}", crate::render::error_line(&format!("slots {verb}: {e}")));
        1
    };
    let want = if set { 2 } else { 1 };
    if args.len() != want {
        eprintln!("{}", crate::render::error_line(&format!("slots {verb}: wrong number of arguments")));
        eprintln!("{SLOTS_USAGE}");
        return 1;
    }
    let name = args[0].as_str();
    if let Err(e) = valid_name(name) {
        return fail(&e);
    }
    let n = if set {
        match args[1].parse::<u32>().ok().filter(|n| valid_capacity(*n).is_ok()) {
            Some(n) => Some(n),
            None => return fail(&format!("capacity must be a whole number from 1 to {CAPACITY_MAX} (got {})", args[1])),
        }
    } else {
        None
    };
    let before = capacity(name);
    let changed = match set_capacity(name, n) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let after = capacity(name);
    for p in &after.problems {
        eprintln!("worktrees: {p}");
    }
    let path = crate::config::config_toml_path();
    match (changed, set) {
        (false, true) => println!("{name}: already {} ({})", after.n, after.source.label()),
        (false, false) => println!("{name}: [slots] {name} is not set; it is {} ({})", after.n, after.source.label()),
        (true, _) => {
            println!("{name}: {} ({}), was {} ({})", after.n, after.source.label(), before.n, before.source.label());
            println!("written to {}; a job already waiting keeps the capacity it started with", path.display());
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
    let configured = capacity(&ra.slot);
    for p in &configured.problems {
        eprintln!("worktrees: {p}");
    }
    let cap = ra.slots.unwrap_or(configured.n);
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
    let mut child = match crate::childenv::command(&ra.cmd[0]).args(&ra.cmd[1..]).spawn() {
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

    const MAC: Machine = Machine { cores: 10, ram_bytes: 36 * GIB };

    #[test]
    fn capacity_reads_the_name_then_default_then_the_machine_then_built_in() {
        let r = |text: &str, m: Option<Machine>| {
            let c = resolve(text, "gate", m);
            (c.n, c.source.label())
        };
        let mac = MAC.capacity();
        assert_eq!(r("", None), (BUILT_IN, "built-in".into()));
        assert_eq!(r("", Some(MAC)), (mac, "machine (10 cores, 36 GiB)".into()));
        assert_eq!(r("[slots]\ndefault = 3\n", Some(MAC)), (3, "config [slots] default".into()));
        assert_eq!(r("[slots]\ndefault = 3\ngate = 1\n", Some(MAC)), (1, "config [slots] gate".into()));
        // An explicit entry beats the machine in BOTH directions: below it and
        // above its ceiling.
        assert_eq!(r("[slots]\ngate = 1\n", Some(Machine { cores: 512, ram_bytes: 4096 * GIB })).0, 1);
        assert_eq!(r("[slots]\ngate = 40\n", Some(Machine { cores: 1, ram_bytes: GIB })).0, 40);
        assert_eq!(r("[slots]\ndefault = 40\n", Some(Machine { cores: 1, ram_bytes: GIB })).0, 40);
        let c = resolve("[slots]\ngate = 0\n", "gate", Some(MAC));
        assert_eq!((c.n, c.problems.len()), (mac, 1), "{c:?}");
        let c = resolve("slots = 4\n", "gate", None);
        assert_eq!((c.n, c.problems.len()), (BUILT_IN, 1));
        let c = resolve("not toml [", "gate", Some(MAC));
        assert_eq!((c.n, c.problems.len()), (mac, 1), "a broken file still gets the machine");
        // `default` resolves to itself, then the machine — and says so once.
        let c = resolve("[slots]\ndefault = 0\n", "default", Some(MAC));
        assert_eq!((c.n, c.source, c.problems.len()), (mac, Source::Machine(MAC), 1));
        assert!(resolve("[slots]\ngate = 3\n", "gate", None).explicit("gate"));
        assert!(!resolve("[slots]\ndefault = 3\n", "gate", None).explicit("gate"));
        assert!(resolve("[slots]\ndefault = 3\n", "default", None).explicit("default"));
    }

    #[test]
    fn the_machine_default_is_bounded_on_tiny_and_huge_machines() {
        let g = |cores: u32, gib: u64| machine_capacity(cores, gib * GIB);
        let table: &[(u32, u64, u32, &str)] = &[
            (0, 0, 1, "nothing readable still runs one"),
            (1, 1, 1, "a 1-core 1 GiB VM"),
            (2, 4, 1, "a small CI runner"),
            (4, 16, 1, "GitHub's ubuntu-latest"),
            (8, 8, 1, "many cores, little memory: memory decides"),
            (2, 256, 1, "much memory, few cores: cores decide"),
            (10, 36, 3, "this Mac: 10 performance cores, 36 GiB"),
            (12, 64, 4, "a 64 GiB workstation"),
            (128, 1024, MACHINE_MAX, "huge: the ceiling, never more"),
            (u32::MAX, u64::MAX / GIB, MACHINE_MAX, "no overflow at the limits"),
        ];
        for &(cores, gib, want, why) in table {
            assert_eq!(g(cores, gib), want, "{cores} cores, {gib} GiB: {why}");
        }
        for cores in 0..200 {
            for gib in [0, 1, 2, 3, 7, 15, 31, 63, 127, 511, 4096] {
                let n = g(cores, gib);
                assert!((1..=MACHINE_MAX).contains(&n), "{cores} cores, {gib} GiB gave {n}");
            }
        }
    }

    /// The probe this OS uses (sysctl on macOS, nproc + /proc/meminfo on
    /// Linux) reads something sane — CI runs it on both.
    #[test]
    fn this_machine_can_be_read() {
        let m = read_machine().expect("cores and memory are readable here");
        assert!(m.cores >= 1 && m.cores <= 4096, "{m:?}");
        assert!(m.ram_bytes >= 256 << 20, "{m:?}");
        assert!((1..=MACHINE_MAX).contains(&m.capacity()));
    }

    #[test]
    fn the_machine_label_names_cores_and_whole_gib() {
        let m = Machine { cores: 10, ram_bytes: 38_654_705_664 };
        assert_eq!(Source::Machine(m).label(), "machine (10 cores, 36 GiB)");
        assert_eq!(Source::Config("default".into()).label(), "config [slots] default");
        assert_eq!(Source::BuiltIn.label(), "built-in");
    }

    #[test]
    fn with_slot_edits_one_key_and_keeps_every_other_line() {
        let base = "# mine\nai_cmd = \"claude\" # inline\n\n[slots]\ngate = 2   # tuned\ne2e = 1\n\n# about quota\n[quota]\ngate = false\n";
        let set = with_slot(base, "gate", Some(3)).unwrap();
        assert_eq!(set, base.replace("gate = 2   # tuned", "gate = 3   # tuned"), "in place, comment kept");
        let added = with_slot(base, "lint", Some(1)).unwrap();
        assert_eq!(added, base.replace("e2e = 1\n", "e2e = 1\nlint = 1\n"), "after the table's last key");
        let gone = with_slot(base, "gate", None).unwrap();
        assert_eq!(gone, base.replace("gate = 2   # tuned\n", ""), "only that line goes; [quota] gate stays");
        assert_eq!(with_slot(base, "lint", None).unwrap(), base, "removing an absent key changes nothing");
        // No table yet: one is added at the end, after a blank line.
        let fresh = with_slot("ai_cmd = \"claude\"\n", "default", Some(4)).unwrap();
        assert_eq!(fresh, "ai_cmd = \"claude\"\n\n[slots]\ndefault = 4\n");
        assert_eq!(with_slot("", "gate", Some(3)).unwrap(), "[slots]\ngate = 3\n");
        // An empty table gets the key under its header.
        assert_eq!(with_slot("[slots] # mine\n\n[x]\n", "gate", Some(2)).unwrap(), "[slots] # mine\ngate = 2\n\n[x]\n");
        // CRLF stays CRLF.
        assert_eq!(with_slot("[slots]\r\ngate = 2\r\n", "gate", Some(5)).unwrap(), "[slots]\r\ngate = 5\r\n");
        // `gate-x` is not `gate`.
        assert_eq!(with_slot("[slots]\ngate-x = 2\n", "gate", Some(1)).unwrap(), "[slots]\ngate-x = 2\ngate = 1\n");
    }

    #[test]
    fn with_slot_refuses_a_layout_it_cannot_edit_or_a_value_it_must_not_write() {
        for layout in ["slots.gate = 2\n", "slots = { gate = 2 }\n", "[slots]\n\"gate\" = 2\n"] {
            let e = with_slot(layout, "gate", Some(3)).unwrap_err();
            assert!(e.contains("edit it by hand"), "{layout:?}: {e}");
        }
        assert!(with_slot("slots.gate = 2\n", "gate", None).is_err(), "a dotted key cannot be removed line-wise either");
        assert!(with_slot("not = [toml", "gate", Some(3)).unwrap_err().contains("does not parse"));
        for n in [0, CAPACITY_MAX + 1] {
            assert!(with_slot("", "gate", Some(n)).is_err(), "{n}");
        }
        assert!(with_slot("", "gate", Some(CAPACITY_MAX)).is_ok());
        for bad in ["", "Gate", "../x", "-x", "a b"] {
            assert!(with_slot("", bad, Some(2)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn set_capacity_at_writes_unsets_and_reports_no_change() {
        let d = tmp("set");
        let f = d.join("cfg").join("config.toml");
        assert_eq!(set_capacity_at(&f, "gate", Some(3)), Ok(true), "creates the file and its dir");
        assert_eq!(fs::read_to_string(&f).unwrap(), "[slots]\ngate = 3\n");
        assert_eq!(set_capacity_at(&f, "gate", Some(3)), Ok(false));
        fs::write(&f, "prefix = \"wt\"\n[slots]\ngate = 3\n").unwrap();
        assert_eq!(set_capacity_at(&f, "gate", None), Ok(true));
        assert_eq!(fs::read_to_string(&f).unwrap(), "prefix = \"wt\"\n[slots]\n");
        assert_eq!(set_capacity_at(&f, "gate", None), Ok(false));
        // A nonsense value is still a key to remove.
        fs::write(&f, "[slots]\ngate = \"lots\"\n").unwrap();
        assert_eq!(set_capacity_at(&f, "gate", None), Ok(true));
        assert!(set_capacity_at(&f, "gate", Some(0)).is_err());
        assert!(set_capacity_at(&f, "../gate", Some(2)).is_err());
        fs::write(&f, "broken = [").unwrap();
        assert!(set_capacity_at(&f, "gate", None).unwrap_err().contains("does not parse"));
        assert_eq!(fs::read_to_string(&f).unwrap(), "broken = [", "a refusal writes nothing");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_overview_lists_gate_the_config_names_and_default_with_sources() {
        let root = tmp("overview");
        let text = "[slots]\ne2e = 1\ndefault = 3\nBad = 2\n";
        let _h = try_take(&mkdir(root.join("gate")), 0, &rec(7, 1)).unwrap();
        let ov = overview_in(Some(&root), text, Some(MAC), false);
        let rows: Vec<(&str, u32, &str, bool, u32, usize)> = ov
            .slots
            .iter()
            .map(|s| (s.name.as_str(), s.capacity, s.source.as_str(), s.explicit, s.fallback, s.holders.len()))
            .collect();
        let mac = MAC.capacity();
        assert_eq!(
            rows,
            vec![
                ("gate", 3, "config [slots] default", false, 3, 1),
                ("e2e", 1, "config [slots] e2e", true, 3, 0),
                ("default", 3, "config [slots] default", true, mac, 0),
            ]
        );
        assert_eq!(ov.slots[2].fallback_source, "machine (10 cores, 36 GiB)");
        assert_eq!(ov.machine, Some(MachineView { cores: 10, ram_gib: 36, capacity: mac }));
        assert_eq!(ov.problems.len(), 1, "{:?}", ov.problems);
        assert!(ov.problems[0].contains("`Bad` is not a slot name"));
        // `in_use` adds a slot nobody configured, once someone holds it.
        let _o = try_take(&mkdir(root.join("other")), 0, &rec(8, 1)).unwrap();
        let names: Vec<String> = overview_in(Some(&root), text, None, true).slots.into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["gate", "e2e", "other", "default"]);
        let names: Vec<String> = overview_in(Some(&root), text, None, false).slots.into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["gate", "e2e", "default"], "Settings lists only what the config names");
        let _ = fs::remove_dir_all(&root);
    }

    fn mkdir(p: PathBuf) -> PathBuf {
        fs::create_dir_all(&p).unwrap();
        p
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
