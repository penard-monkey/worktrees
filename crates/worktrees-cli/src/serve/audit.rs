//! The audit log (Phase 2 §5.1, R5): `<config_dir>/audit.jsonl`.
//!
//! **Who writes it.** Only this module, through an `Audit` the connection
//! layer owns (`conn::Shared`). It is not in `Ctx`, and `worktrees-api` does
//! not depend on this crate, so no command body can reach it. A record is a
//! fixed set of fields serialized whole by serde: there is no `format!` of a
//! client string into a line, so a name, a path or a command with a newline
//! in it cannot add a line (L1's class, closed by construction). Every
//! `detail` is built HERE from typed values, under keys this file spells.
//!
//! **What it hears.** A request reaches it as dispatch's `Witness` ([`Seen`]),
//! which `dispatch::call_seen` calls with the resolver's verdict. It is handed
//! to dispatch BESIDE the `Call`, never inside it, so the body — which is
//! given the `Call` — still cannot reach it. A refusal is recorded where the
//! host makes it: the resolver, or a device launch's lane cap
//! (`device-lanes`). An admitted row whose capability (the table's own column)
//! is not `read` is recorded once its body is done, naming the caller and the
//! project its objects resolved to; a body that answers `ok: false` (a
//! `CmdResult`, a device launch core would not start unrecorded among them)
//! is recorded `err`, as one that answered `err` is.
//!
//! **The file.** Opened `O_APPEND | O_NOFOLLOW | O_NONBLOCK`, mode 0600 at
//! creation, and refused unless it is a regular file — a FIFO planted at the
//! name would otherwise hang serve's boot in `open(2)`. A file found with
//! another owner or a group/world bit is NOT chmodded: the writer carries on
//! (loopback is the owner's own console) and `healthy()` says no, which is
//! what the remote listener will refuse to start on.
//!
//! **The chain.** Every line carries `prev`, the SHA-256 (hex) of the exact
//! bytes of the line before it, newline excluded, and a `seq` one higher than
//! that line's. The very first line ever is a `checkpoint` with
//! `reason: "genesis"`, `seq: 1` and `prev` = 64 zeros. `verify` recomputes
//! the hash of each line from its bytes as stored — nothing is re-serialized —
//! so an edited, removed or reordered record breaks the chain at the record
//! after it. It cannot stop a same-user process rewriting the whole file, and
//! it does not claim to (§1). Nor can it see an edit to the LAST record, or a
//! cut tail, until something is written after it: the chain protects a record
//! from the moment the next one lands.
//!
//! **Rotation.** At `MAX_FILE` the live file becomes `audit.jsonl.1` (the
//! others shift up; `KEEP` files in all, the live one included), and the new
//! file opens with a `checkpoint` whose `prev` is the old file's final hash,
//! so the chain runs on across files. The checkpoint also carries the security
//! fields of every unrevoked device (§5.1), read from the registry through
//! [`Fleet`], so §5.2 can read from the newest checkpoint onward instead of
//! from a file rotation removed. EVERY file opens with a checkpoint, and the
//! oldest one left on disk is where verification anchors.
//!
//! **Devices** (§5.1). Every `DeviceEvent` the registry fires is a record
//! ([`Audit::hear`], subscribed in `conn::start`): `device` and `pairing`
//! records naming the device in `detail.subject` and a pairing's principal
//! in `detail.for_principal`, never the comparison code, the link or its
//! secret. A refused device key is an auth failure, in the same episodes as
//! a failed handshake. §5.3's alerts are `alert` records, emitted to the host.
//!
//! **A device's budget.** A paired device's request records spend a byte
//! budget of its own across its connections ([`ACTS`], [`REFUSALS`]): past it
//! they are counted into a `budget` episode, a record when it begins and one
//! with the counts when it ends, never dropped. The device and pairing records
//! and the first record of any episode are never charged.
//!
//! **A write that fails** (decided with (main), 2026-10-21; §5 is silent):
//! the console fails OPEN — the request is served, serve.log says so once per
//! episode, and the next write that succeeds is preceded by a `gap` record
//! with the count and the episode's first and last times. A remote device
//! will fail CLOSED on `healthy()`. A write that may have left part of a line
//! is never appended after: the next write rotates first, and the checkpoint
//! names the torn bytes, which is how `verify` tells a torn tail from an edit.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use worktrees_api::log::{info, warn};
use worktrees_api::Host;

use worktrees_core::authz::{Caller, Capability, Device, DeviceId, PrincipalId};

use super::devices::{Actor, ConnectRefusal, DeviceEvent, DeviceRegistry, SecurityFields, Subject};
use super::dispatch::{self, Args, Row, Witness};
use super::resolve::{Refusal, RefusalKind, Resolved};

/// The live file, in the config dir; rotated copies are `audit.jsonl.<n>`.
pub const FILE: &str = "audit.jsonl";

/// The size past which the live file is rotated (§5.1).
pub const MAX_FILE: u64 = 5 << 20;

/// How many files are kept, the live one included (§5.1: five, 25 MiB).
pub const KEEP: usize = 5;

/// `prev` of the first record ever written.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The most bytes of one client-supplied identifier (a path, a slug, a
/// session) a record keeps. A 16 MiB path is a legal message; a 16 MiB audit
/// line would be a disk-writing flood.
pub const MAX_IDENT: usize = 1024;

/// An episode of refusals ends after this long with none: the next refusal
/// starts a new one. Floods are aggregated the way `conn.rs`'s `capped` log
/// is, so they cannot become a disk-writing flood.
pub const EPISODE_QUIET: i64 = 60;

/// Failed handshakes in one episode before the host is alerted (§5.3).
pub const AUTH_ALERT_AT: u64 = 20;

/// What ONE paired device may make the log write, in bytes, across all of
/// its connections: a token bucket per budget, a record costing its line.
/// Past it a record is counted into an episode instead of written. Bytes,
/// not records, because rotation counts bytes, and a refusal's size is the
/// caller's to choose: three identifiers of `MAX_IDENT` each make one about
/// 3.3 KB, where a typical record is 370 B.
///
/// What one device flooding flat out can write, per budget, is the larger of
/// its refill and its episode records, which are always written and debited
/// (one `began` and one `ended` per `EPISODE_QUIET`, about 1.2 KB, so about
/// 20 B/s): 32 B/s for acts and 20 B/s for refusals, about 52 B/s or 4.3 MiB
/// a day. The four rotated files, 20 MiB, then last it 4.6 days; at 50
/// records a second (§6's request rate) they lasted 20 minutes, and under 3
/// with crafted refusals. Sixteen devices at once: about 7 hours. Measured
/// (`an_hour_of_flood_from_one_device_writes_its_ceiling`): an hour of 25
/// refusals and 25 acts a second wrote 227 KB, both bursts and 40 B/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Allowance {
    pub burst: f64,
    pub per_sec: f64,
}

/// What a device's grant let through (`call`, `stream`): about 175 records
/// at once, then one every 12 s.
pub const ACTS: Allowance = Allowance { burst: 64.0 * 1024.0, per_sec: 32.0 };

/// What a device can make with nothing granted at all (`refused`): about 40
/// at once, then one every 50 s. A budget of its own, so junk a device needs
/// no capability for cannot spend the one that records what its grant did.
pub const REFUSALS: Allowance = Allowance { burst: 16.0 * 1024.0, per_sec: 8.0 };

/// A device's coalesced episode is written out after this long even while it
/// goes on: a count held in memory longer is a count a crash loses.
pub const EPISODE_SPAN: Duration = Duration::from_secs(600);

/// Keys one coalesced episode's tally keeps; the rest are counted as
/// `other`, so a device cycling through every command cannot make the record
/// that ends the episode large.
const MAX_TALLY: usize = 16;

/// Device knocks (`ConnectRefused`) one auth episode alerts for, one per
/// device and reason; past it they are only counted.
const MAX_KNOCK_ALERTS: usize = 64;

/// How long a writer that could not open its file waits before trying again,
/// so a log that stays broken costs one attempt a second, not one a record.
const REOPEN_EVERY: Duration = Duration::from_secs(1);

/// Where a checkpoint reads the devices it carries (§5.1): every unrevoked
/// device's security fields, which §5.2 compares a `devices.json` with
/// instead of reading a file rotation removed. Called under the log's lock;
/// the registry's [`DeviceRegistry::checkpoint`] takes only its state's read
/// lock, and the registry hands its events to the log after releasing its
/// locks, so the two never wait on each other.
#[derive(Clone)]
pub struct Fleet(Arc<dyn Fn() -> Vec<SecurityFields> + Send + Sync>);

impl Fleet {
    /// The registry's devices: serve's.
    pub fn of(devices: &Arc<DeviceRegistry>) -> Fleet {
        let d = devices.clone();
        Fleet(Arc::new(move || d.checkpoint()))
    }

    /// No device at all: a log with no registry behind it (the unit tests).
    #[cfg(test)]
    pub fn none() -> Fleet {
        Fleet(Arc::new(Vec::new))
    }
}

// ── who acted ───────────────────────────────────────────────────────────────

/// A record's actor fields (§2.8, §5.1): slice 1's `Caller`, as §5.1 spells
/// it — the principal's id, and the device's: `loopback` for the console
/// (`Device::Console`), a paired device's own id otherwise. Before
/// authentication there is no caller, and both fields are null.
fn actor(c: &Caller) -> (&str, &str) {
    let device = match c.device() {
        Device::Console => "loopback",
        Device::Paired(id) => id.as_str(),
    };
    (c.principal().as_str(), device)
}

/// The actor of a registry change, as a record names it: the console, a
/// paired device, or nobody — serve itself (an expiry, the cross-check, a
/// reload), whose records say so in `detail.by`.
fn by_caller(by: &Actor) -> Option<Caller> {
    match by {
        Actor::Console => Some(Caller::console()),
        Actor::Device { principal, device } => Some(Caller::paired(principal.clone(), device.clone())),
        Actor::Serve => None,
    }
}

/// The stream rows, by what they do to an attach. Opens and closes are
/// records; the traffic in between is NOT — one record per keystroke is what
/// §5.1 rules out — and is totalled into the close.
const STREAM_OPEN: &[&str] = &["term_open", "shell_open"];
const STREAM_CLOSE: &[&str] = &["term_close", "shell_detach", "close_shell_session"];
const STREAM_TRAFFIC: &[&str] = &["term_write", "shell_write", "term_wheel", "term_resize", "shell_resize"];

/// The argument keys a record names its object from, first match wins, and
/// its project when the resolver resolved none (a refusal, a `new_path`). Read
/// from the arguments the resolver handed on, which hold the CANONICAL values
/// it checked once it admits a row. An allowlist on purpose: only identifiers
/// are read, so a note's text, a file's content, a paste or a clone URL
/// (which can carry a credential) can never reach a record by being the
/// argument somebody forgot.
const PROJECT_KEYS: &[&str] = &["repo", "root", "dir", "location", "parent"];
const OBJECT_KEYS: &[&str] = &["slug", "path", "session", "branch", "name", "id"];

// ── one record ──────────────────────────────────────────────────────────────

/// A record before the writer stamps it. Built only in this file.
struct Record {
    kind: &'static str,
    actor: Option<Caller>,
    conn: Option<u64>,
    peer: Option<String>,
    cmd: Option<&'static str>,
    cap: Option<&'static str>,
    project: Option<String>,
    object: Option<String>,
    outcome: Option<&'static str>,
    ms: Option<u64>,
    detail: Value,
}

impl Record {
    fn new(kind: &'static str, detail: Value) -> Record {
        Record {
            kind,
            actor: None,
            conn: None,
            peer: None,
            cmd: None,
            cap: None,
            project: None,
            object: None,
            outcome: None,
            ms: None,
            detail,
        }
    }
}

/// The line as written: every field in every record, null when it does not
/// apply, so a reader can filter on any of them without guessing which
/// records carry it. Field order is the struct's.
#[derive(Serialize)]
struct Line<'a> {
    seq: u64,
    ts: &'a str,
    pid: u32,
    kind: &'a str,
    principal: Option<&'a str>,
    device: Option<&'a str>,
    conn: Option<u64>,
    peer: Option<&'a str>,
    cmd: Option<&'a str>,
    cap: Option<&'a str>,
    project: Option<&'a str>,
    object: Option<&'a str>,
    outcome: Option<&'a str>,
    ms: Option<u64>,
    detail: &'a Value,
    prev: &'a str,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn now() -> i64 {
    worktrees_core::sysclock::now_epoch()
}

/// `2026-10-20T14:03:11Z`.
fn rfc3339(epoch: i64) -> String {
    format!("{}Z", worktrees_api::util::fmt_utc(epoch).replacen(' ', "T", 1))
}

/// A client-supplied identifier, cut to `MAX_IDENT` bytes on a character
/// boundary. JSON escaping is serde's, so its content cannot break the line.
fn ident(s: &str) -> String {
    if s.len() <= MAX_IDENT {
        return s.to_string();
    }
    let mut end = MAX_IDENT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// One argument as an identifier: a string, or a number spelled out. Read
/// the way `Args` reads it (as written, then camelCase), and never cloned
/// whole — a `content` of 16 MiB is never in the key list anyway.
fn arg_ident(args: &Args, keys: &[&str]) -> Option<String> {
    for k in keys {
        let v = args.0.get(*k).or_else(|| args.0.get(&dispatch::camel(k)));
        match v {
            Some(Value::String(s)) => return Some(ident(s)),
            Some(Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

fn arg<'a>(args: &'a Args, key: &str) -> Option<&'a Value> {
    args.0.get(key).or_else(|| args.0.get(&dispatch::camel(key)))
}

/// A lock that a panic elsewhere cannot take down with it. Every request on
/// every connection passes through these locks, and a poisoned one under
/// `lock().unwrap()` would panic each of them in turn — outside the
/// `catch_unwind` that guarantees a reply. The state behind them is counts,
/// a file handle and a hash, each written in one assignment, so what a panic
/// leaves is still coherent enough to carry on from.
fn guard<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ── the files ───────────────────────────────────────────────────────────────

/// Open for append, creating it 0600. `O_NOFOLLOW` refuses a link at the
/// name, `O_NONBLOCK` makes a FIFO there an error instead of a hang, and the
/// `fstat` after it refuses anything that is not a regular file.
fn open_append(path: &Path) -> std::io::Result<File> {
    let f = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !f.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok(f)
}

/// The whole of a file, or `None` if there is none; refused unless regular,
/// for `open_append`'s reasons.
fn read_regular(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let f = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path);
    let mut f = match f {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !f.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(Some(buf))
}

/// Why the live file is not fit to hold the log, if it is not: §5.1 refuses
/// the remote listener on another owner or a group/world bit, "rather than
/// chmod it silently". Absent is fine — it is created 0600.
fn mode_problem(path: &Path) -> Option<String> {
    let m = std::fs::symlink_metadata(path).ok()?;
    if !m.file_type().is_file() {
        return Some("is not a regular file".into());
    }
    // SAFETY: a plain syscall with no arguments.
    let me = unsafe { libc::geteuid() };
    if m.uid() != me {
        return Some(format!("is owned by uid {}, not {me}", m.uid()));
    }
    if m.mode() & 0o077 != 0 {
        return Some(format!("has mode {:o}, not 600", m.mode() & 0o777));
    }
    None
}

fn rotated_name(n: usize) -> String {
    format!("{FILE}.{n}")
}

// ── the verifier ────────────────────────────────────────────────────────────

/// Where the chain first fails: the file, the 1-based line, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Break {
    pub file: String,
    pub line: usize,
    pub why: &'static str,
}

/// What `verify` found. The head is computed past a break as well, from the
/// lines as they are, because the writer chains on from what is there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Files read, oldest first.
    pub files: Vec<String>,
    pub records: u64,
    /// The `seq` of the oldest record on disk: where the chain is anchored.
    pub first_seq: Option<u64>,
    pub last_seq: u64,
    /// The hash of the last complete line; `None` when there is none.
    pub head: Option<String>,
    /// The first break, if any.
    pub broken: Option<Break>,
    /// Bytes of complete lines in the live file: where the next line starts.
    pub live_size: u64,
    /// Bytes after the live file's last newline: a write in flight, or torn.
    pub live_torn: Option<u64>,
    /// A torn tail at the end of the newest rotated file with no record after
    /// it yet: the next checkpoint must name it.
    pub dangling_torn: Option<u64>,
}

impl Verdict {
    /// The chain's state for the `started` record.
    fn summary(&self) -> Value {
        match &self.broken {
            None => json!({ "ok": true, "records": self.records, "files": self.files.len(), "first_seq": self.first_seq }),
            Some(b) => json!({ "ok": false, "records": self.records, "files": self.files.len(), "break": b }),
        }
    }
}

/// Keep the FIRST break: everything after it follows from it.
fn fail(v: &mut Verdict, file: &str, line: usize, why: &'static str) {
    if v.broken.is_none() {
        v.broken = Some(Break { file: file.to_string(), line, why });
    }
}

/// The fields a check reads from one line. `None` for a line that is not a
/// JSON object with a `u64` `seq`, a string `prev` and a string `kind`.
struct Parsed {
    seq: u64,
    prev: String,
    kind: String,
    reason: Option<String>,
    torn_bytes: Option<u64>,
}

fn parse(line: &[u8]) -> Option<Parsed> {
    let v: Value = serde_json::from_slice(line).ok()?;
    let o = v.as_object()?;
    Some(Parsed {
        seq: o.get("seq")?.as_u64()?,
        prev: o.get("prev")?.as_str()?.to_string(),
        kind: o.get("kind")?.as_str()?.to_string(),
        reason: o.get("detail").and_then(|d| d.get("reason")).and_then(Value::as_str).map(str::to_string),
        torn_bytes: o.get("detail").and_then(|d| d.get("torn_bytes")).and_then(Value::as_u64),
    })
}

/// Check the whole chain in `dir`: every rotated file oldest first, then the
/// live one. Each line's `prev` must be the hash of the line before it and
/// its `seq` one more; every file must open with a checkpoint, and only
/// there; the oldest file present opens with genesis or is the anchor (its
/// predecessor was rotated away); a torn tail at the end of a rotated file
/// must be named, to the byte, by the checkpoint that follows it.
pub fn verify(dir: &Path) -> Verdict {
    let mut v = Verdict::default();
    let mut names: Vec<String> = (1..KEEP)
        .rev()
        .map(rotated_name)
        .filter(|n| std::fs::symlink_metadata(dir.join(n)).is_ok())
        .collect();
    names.push(FILE.to_string());

    let mut prev_hash: Option<String> = None;
    let mut prev_seq: Option<u64> = None;
    let mut pending_torn: Option<u64> = None;

    for name in &names {
        let live = name == FILE;
        let bytes = match read_regular(&dir.join(name)) {
            Ok(Some(b)) => b,
            Ok(None) => continue,
            Err(_) => {
                fail(&mut v, name, 0, "not a readable regular file");
                continue;
            }
        };
        v.files.push(name.clone());
        let mut pieces: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
        let fragment = pieces.pop().unwrap_or_default();
        if pieces.is_empty() && fragment.is_empty() && !live {
            fail(&mut v, name, 0, "an empty rotated file");
        }
        for (i, line) in pieces.iter().enumerate() {
            let n = i + 1;
            let h = sha256_hex(line);
            let Some(p) = parse(line) else {
                fail(&mut v, name, n, "not a record");
                prev_hash = Some(h);
                prev_seq = prev_seq.map(|s| s + 1);
                continue;
            };
            if i == 0 {
                if p.kind != "checkpoint" {
                    fail(&mut v, name, n, "a file that does not open with a checkpoint");
                }
                let torn = p.reason.as_deref() == Some("torn");
                match pending_torn.take() {
                    Some(want) if !(torn && p.torn_bytes == Some(want)) => {
                        fail(&mut v, name, n, "a torn record the next checkpoint does not account for")
                    }
                    None if torn => fail(&mut v, name, n, "a checkpoint names torn bytes that are not there"),
                    _ => {}
                }
                match (&prev_hash, p.reason.as_deref()) {
                    (None, Some("genesis")) => {
                        if p.prev != GENESIS || p.seq != 1 {
                            fail(&mut v, name, n, "a genesis checkpoint that is not the first record");
                        }
                    }
                    // The anchor: what came before it was rotated away.
                    (None, Some("rotated" | "torn" | "resumed")) => {}
                    (None, _) => fail(&mut v, name, n, "a checkpoint with no reason it knows"),
                    (Some(_), Some("genesis")) => fail(&mut v, name, n, "a second genesis"),
                    (Some(_), _) => {}
                }
            } else if p.kind == "checkpoint" {
                fail(&mut v, name, n, "a checkpoint inside a file");
            }
            if let Some(ph) = &prev_hash {
                if &p.prev != ph {
                    fail(&mut v, name, n, "prev is not the hash of the record before it");
                }
                if prev_seq.map(|s| s + 1) != Some(p.seq) {
                    fail(&mut v, name, n, "seq does not follow the record before it");
                }
            }
            if v.first_seq.is_none() {
                v.first_seq = Some(p.seq);
            }
            prev_hash = Some(h);
            prev_seq = Some(p.seq);
            v.records += 1;
        }
        if live {
            v.live_size = (bytes.len() - fragment.len()) as u64;
            if !fragment.is_empty() {
                v.live_torn = Some(fragment.len() as u64);
            }
        } else if !fragment.is_empty() {
            pending_torn = Some(fragment.len() as u64);
        }
    }
    v.dangling_torn = pending_torn;
    v.last_seq = prev_seq.unwrap_or(0);
    v.head = prev_hash;
    v
}

// ── the writer ──────────────────────────────────────────────────────────────

/// A run of records that could not be written (the console's fail-open).
struct Lost {
    count: u64,
    from: i64,
    to: i64,
}

/// A run of refusals of one class with no quiet gap of `EPISODE_QUIET`.
struct Episode {
    began: i64,
    last: i64,
    count: u64,
    reasons: BTreeMap<&'static str, u64>,
    alerted: bool,
    /// The devices (and why) whose refused knock this episode has alerted
    /// for: a revoked phone knocking all night is one alert, not thousands.
    knocks: std::collections::BTreeSet<String>,
}

/// A refused device key, as an auth episode hears it: what names it in the
/// record that begins an episode, and, for a known device, its alert, keyed
/// by device and reason.
#[derive(Default)]
struct Knock {
    detail: Map<String, Value>,
    alert: Option<(String, Value)>,
}

// ── a device's budget ───────────────────────────────────────────────────────

/// Which of a device's two budgets a record spends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Charge {
    /// `call` and `stream`: what its grant let through.
    Acts,
    /// `refused`: what it can make with nothing granted.
    Refusals,
}

impl Charge {
    fn name(self) -> &'static str {
        match self {
            Charge::Acts => "acts",
            Charge::Refusals => "refusals",
        }
    }
    fn allowance(self) -> Allowance {
        match self {
            Charge::Acts => ACTS,
            Charge::Refusals => REFUSALS,
        }
    }
}

/// Bytes a device may still have written, refilled with time. It may go
/// into debt — the episode records are always written, and paid for out of
/// later detail — but no deeper than one burst, so a device that stopped
/// flooding is back in full detail within two bursts' refill.
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn full(a: Allowance, now: Instant) -> Bucket {
        Bucket { tokens: a.burst, at: now }
    }

    fn refill(&mut self, a: Allowance, now: Instant) {
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + dt * a.per_sec).min(a.burst);
        self.at = now;
    }

    /// Take `n` if it is there.
    fn take(&mut self, a: Allowance, n: f64, now: Instant) -> bool {
        self.refill(a, now);
        let ok = self.tokens >= n;
        if ok {
            self.tokens -= n;
        }
        ok
    }

    /// Take `n` whether or not it is there.
    fn debit(&mut self, a: Allowance, n: f64, now: Instant) {
        self.refill(a, now);
        self.tokens = (self.tokens - n).max(-a.burst);
    }
}

/// The records one budget counted instead of writing, from the first of them
/// until `EPISODE_QUIET` passes with none (or `EPISODE_SPAN`, or the device
/// goes, or serve stops).
struct Coalesced {
    who: Caller,
    began: Instant,
    began_epoch: i64,
    last: Instant,
    last_epoch: i64,
    count: u64,
    bytes: u64,
    tally: BTreeMap<String, u64>,
}

/// One paired device's budgets, across all of its connections.
struct Spend {
    conns: usize,
    acts: Bucket,
    refusals: Bucket,
    open: HashMap<Charge, Coalesced>,
}

impl Spend {
    fn new(now: Instant) -> Spend {
        Spend { conns: 0, acts: Bucket::full(ACTS, now), refusals: Bucket::full(REFUSALS, now), open: HashMap::new() }
    }

    fn bucket(&mut self, c: Charge) -> &mut Bucket {
        match c {
            Charge::Acts => &mut self.acts,
            Charge::Refusals => &mut self.refusals,
        }
    }
}

/// What a coalesced record is counted as: its kind, its command and how it
/// ended, each one of OUR names — a row's, a refusal kind's, an outcome's —
/// never a client's string.
fn tally_key(r: &Record) -> String {
    let how = match r.kind {
        "refused" => r.detail.get("reason").and_then(Value::as_str).unwrap_or("refused"),
        _ => r.outcome.unwrap_or("-"),
    };
    format!("{} {} {how}", r.kind, r.cmd.unwrap_or("-"))
}

/// The episode classes: what a run of them is recorded as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Class {
    /// A handshake v1 that did not complete: no hello, a bad proof, the
    /// deadline, the pre-auth byte budget. §5.1's "authentication failures".
    Auth,
    /// Refused at the HTTP upgrade: Host, Origin, a duplicate or unreadable
    /// header, not a WebSocket upgrade, or no request inside its deadline.
    Upgrade,
    /// A connection cap (`conn::Limits`): closed unanswered at accept.
    Limit,
}

impl Class {
    fn kind(self) -> &'static str {
        match self {
            Class::Auth => "auth",
            Class::Upgrade | Class::Limit => "refused",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Class::Auth => "auth",
            Class::Upgrade => "upgrade",
            Class::Limit => "limit",
        }
    }
}

struct State {
    file: Option<File>,
    /// Bytes of complete lines in the live file.
    size: u64,
    seq: u64,
    head: String,
    /// A write failed and part of it may have landed: look before the next.
    suspect: bool,
    lost: Option<Lost>,
    /// Why the live file is unfit (owner, mode), if it is.
    unfit: Option<String>,
    /// From `verify`: the newest rotated file ends torn, nothing after it.
    dangling_torn: Option<u64>,
    /// When the writer may next try to open a file it could not.
    reopen_at: Option<Instant>,
    episodes: HashMap<Class, Episode>,
    /// Every paired device's budget (`Spend`): the devices connected now, in
    /// an episode, or still refilling; a full one with neither is forgotten.
    budgets: HashMap<DeviceId, Spend>,
    /// The principal the open pairing window was opened for, which every
    /// pairing record names (§5.1's `detail.for_principal`).
    pairing_for: Option<PrincipalId>,
    #[cfg(test)]
    fail: Option<(usize, usize)>,
    /// How far the tests have moved the log's clocks.
    #[cfg(test)]
    skew: Duration,
}

impl State {
    /// The monotonic clock a budget and its episodes are decided by.
    fn mono(&self) -> Instant {
        let t = Instant::now();
        #[cfg(test)]
        let t = t + self.skew;
        t
    }

    /// The wall clock an episode is dated by, and the handshake, upgrade and
    /// limit episodes are decided by.
    fn wall(&self) -> i64 {
        let t = now();
        #[cfg(test)]
        let t = t + self.skew.as_secs() as i64;
        t
    }
}

/// The audit log's one writer. Every method takes the lock for one record
/// (or a checkpoint and a record): order in the file is the order of the
/// lock, and `seq`/`prev` are assigned under it.
pub struct Audit {
    dir: PathBuf,
    max_file: u64,
    host: Option<Arc<dyn Host>>,
    fleet: Fleet,
    st: Mutex<State>,
}

/// An alert to emit once the lock is released (§5.3).
type Alerts = Vec<Value>;

impl Audit {
    /// Read the chain in `config_dir` and open the live file. Never fails:
    /// a log that cannot be opened is an unhealthy one, and the console is
    /// served regardless (fail open). The `Verdict` is the boot check.
    /// `fleet` is where every checkpoint reads the devices it carries.
    pub fn open(config_dir: &Path, host: Option<Arc<dyn Host>>, fleet: Fleet) -> (Arc<Audit>, Verdict) {
        Self::open_with(config_dir, host, fleet, MAX_FILE)
    }

    pub(crate) fn open_with(config_dir: &Path, host: Option<Arc<dyn Host>>, fleet: Fleet, max_file: u64) -> (Arc<Audit>, Verdict) {
        let _ = std::fs::create_dir_all(config_dir);
        let a = Audit {
            dir: config_dir.to_path_buf(),
            max_file,
            host,
            fleet,
            st: Mutex::new(State {
                file: None,
                size: 0,
                seq: 0,
                head: GENESIS.to_string(),
                suspect: false,
                lost: None,
                unfit: None,
                dangling_torn: None,
                reopen_at: None,
                episodes: HashMap::new(),
                budgets: HashMap::new(),
                pairing_for: None,
                #[cfg(test)]
                fail: None,
                #[cfg(test)]
                skew: Duration::ZERO,
            }),
        };
        let v = {
            let mut st = guard(&a.st);
            a.load(&mut st)
        };
        if let Some(b) = &v.broken {
            warn(&format!(
                "serve: audit: the chain in {} is broken at {} line {}: {}",
                a.dir.join(FILE).display(), b.file, b.line, b.why
            ));
        }
        (Arc::new(a), v)
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE)
    }

    /// Whether a REMOTE request may be served under this log: the file is open
    /// and fit (owner, mode), and no record has been lost since the last one
    /// that landed. The console never asks — it fails open — and the remote
    /// listener (slice 6) refuses to start, and a remote non-read request is
    /// refused, while this is false (decided with (main), 2026-10-21). That
    /// listener is its first caller; until it exists only the tests are.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn healthy(&self) -> bool {
        let st = guard(&self.st);
        st.file.is_some() && st.lost.is_none() && st.unfit.is_none()
    }

    /// (Re)read the chain and open the live file: at boot, and whenever the
    /// path no longer names the file this process holds.
    fn load(&self, st: &mut State) -> Verdict {
        let v = verify(&self.dir);
        st.seq = v.last_seq;
        st.head = v.head.clone().unwrap_or_else(|| GENESIS.to_string());
        st.size = v.live_size;
        st.dangling_torn = v.dangling_torn;
        st.suspect = v.live_torn.is_some();
        st.file = match open_append(&self.path()) {
            Ok(f) => Some(f),
            Err(e) => {
                warn(&format!("serve: audit: cannot open {}: {e}", self.path().display()));
                st.reopen_at = Some(Instant::now() + REOPEN_EVERY);
                None
            }
        };
        let unfit = mode_problem(&self.path());
        if unfit.is_some() && unfit != st.unfit {
            warn(&format!(
                "serve: audit: {} {}; left as it is, and remote access will refuse to start on it",
                self.path().display(),
                unfit.as_deref().unwrap_or_default()
            ));
        }
        st.unfit = unfit;
        v
    }

    /// Whether the path still names the file this process holds open.
    fn holds_live(&self, st: &State) -> bool {
        let Some(f) = &st.file else { return false };
        match (f.metadata(), std::fs::symlink_metadata(self.path())) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }

    /// Make the live file ready for one more line: reopened if it went away,
    /// rotated past a torn write or past `max_file`, and opened with a
    /// checkpoint if it holds nothing yet.
    fn prepare(&self, st: &mut State) -> std::io::Result<()> {
        if !self.holds_live(st) {
            if st.file.is_none() && st.reopen_at.is_some_and(|t| Instant::now() < t) {
                return Err(std::io::Error::other("the audit log could not be opened"));
            }
            self.load(st);
            if st.file.is_none() {
                return Err(std::io::Error::other("the audit log could not be opened"));
            }
        }
        if st.suspect {
            let len = st.file.as_ref().map(|f| f.metadata().map(|m| m.len())).transpose()?.unwrap_or(0);
            st.suspect = false;
            if len > st.size {
                return self.rotate(st, "torn", Some(len - st.size));
            }
        }
        if st.size == 0 {
            let torn = st.dangling_torn.take();
            let reason = if torn.is_some() {
                "torn"
            } else if st.seq == 0 {
                "genesis"
            } else {
                "resumed"
            };
            return self.checkpoint(st, reason, torn);
        }
        if st.size >= self.max_file {
            return self.rotate(st, "rotated", None);
        }
        Ok(())
    }

    /// The live file becomes `.1`, the rest shift up, the oldest past `KEEP`
    /// goes, and a new live file opens with a checkpoint.
    fn rotate(&self, st: &mut State, reason: &'static str, torn: Option<u64>) -> std::io::Result<()> {
        let at = |n: usize| self.dir.join(rotated_name(n));
        let _ = std::fs::remove_file(at(KEEP - 1));
        for n in (1..KEEP - 1).rev() {
            if std::fs::symlink_metadata(at(n)).is_ok() {
                std::fs::rename(at(n), at(n + 1))?;
            }
        }
        std::fs::rename(self.path(), at(1))?;
        st.file = Some(open_append(&self.path())?);
        st.size = 0;
        st.dangling_torn = None;
        self.checkpoint(st, reason, torn)
    }

    /// The record that opens every file. `devices` is the security fields of
    /// every unrevoked device (§5.1), so §5.2 never needs a file rotation
    /// removed. Read inside `quietly`: a registry that panics costs this
    /// checkpoint its list (`null`, which no reader can take for "none"),
    /// never the record or the chain.
    fn checkpoint(&self, st: &mut State, reason: &'static str, torn: Option<u64>) -> std::io::Result<()> {
        let devices = quietly(|| (self.fleet.0)()).map(|d| json!(d)).unwrap_or(Value::Null);
        let mut d = json!({ "reason": reason, "devices": devices });
        if let Some(n) = torn {
            d["torn_bytes"] = json!(n);
        }
        self.write_line(st, &Record::new("checkpoint", d))
    }

    /// What `r` costs a budget: its line as the next one, newline included.
    fn cost(st: &State, r: &Record) -> f64 {
        Self::render(st, r).map(|l| l.len() + 1).unwrap_or(0) as f64
    }

    /// `r` as the next line of the chain, without its newline.
    fn render(st: &State, r: &Record) -> std::io::Result<String> {
        let ts = rfc3339(now());
        let who = r.actor.as_ref().map(actor);
        serde_json::to_string(&Line {
            seq: st.seq + 1,
            ts: &ts,
            pid: std::process::id(),
            kind: r.kind,
            principal: who.map(|a| a.0),
            device: who.map(|a| a.1),
            conn: r.conn,
            peer: r.peer.as_deref(),
            cmd: r.cmd,
            cap: r.cap,
            project: r.project.as_deref(),
            object: r.object.as_deref(),
            outcome: r.outcome,
            ms: r.ms,
            detail: &r.detail,
            prev: &st.head,
        })
        .map_err(std::io::Error::other)
    }

    /// One line, in ONE `write(2)`: a short or failed write marks the file
    /// suspect, and the chain's head moves only when the whole line landed.
    fn write_line(&self, st: &mut State, r: &Record) -> std::io::Result<()> {
        let seq = st.seq + 1;
        let line = Self::render(st, r)?;
        let head = sha256_hex(line.as_bytes());
        let mut buf = line.into_bytes();
        buf.push(b'\n');
        let f = st.file.as_mut().ok_or_else(|| std::io::Error::other("the audit log is not open"))?;
        #[cfg(test)]
        if let Some((n, land)) = st.fail.take() {
            let _ = f.write(&buf[..land.min(buf.len())]);
            st.fail = (n > 1).then_some((n - 1, land));
            st.suspect = true;
            return Err(std::io::Error::other("an injected write failure"));
        }
        match f.write(&buf) {
            Ok(n) if n == buf.len() => {
                st.seq = seq;
                st.head = head;
                st.size += buf.len() as u64;
                Ok(())
            }
            Ok(_) => {
                st.suspect = true;
                Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "a short write"))
            }
            Err(e) => {
                st.suspect = true;
                Err(e)
            }
        }
    }

    /// Write one record: the file made ready, then a `gap` if records were
    /// lost, then the record. A failure is counted, said once per episode in
    /// serve.log, and otherwise swallowed — the console fails open.
    fn append(&self, st: &mut State, r: &Record) {
        let res = self.prepare(st).and_then(|()| {
            if let Some(l) = st.lost.take() {
                let gap = Record::new("gap", json!({ "lost": l.count, "from": rfc3339(l.from), "to": rfc3339(l.to) }));
                if let Err(e) = self.write_line(st, &gap) {
                    st.lost = Some(l);
                    return Err(e);
                }
                info(&format!(
                    "serve: audit: writing again; {} record(s) lost between {} and {}",
                    l.count, rfc3339(l.from), rfc3339(l.to)
                ));
            }
            self.write_line(st, r)
        });
        if let Err(e) = res {
            let t = now();
            match &mut st.lost {
                Some(l) => {
                    l.count += 1;
                    l.to = t;
                }
                None => {
                    warn(&format!(
                        "serve: audit: could not write {}: {e}; serving on, and counting what is lost",
                        self.path().display()
                    ));
                    st.lost = Some(Lost { count: 1, from: t, to: t });
                }
            }
        }
    }

    /// End every episode that has been quiet long enough (or all of them).
    fn sweep(&self, st: &mut State, all: bool) {
        let t = st.wall();
        let done: Vec<Class> = st
            .episodes
            .iter()
            .filter(|(c, e)| all || (**c != Class::Limit && t - e.last > EPISODE_QUIET))
            .map(|(c, _)| *c)
            .collect();
        for c in done {
            self.end_episode(st, c);
        }
        self.sweep_budgets(st, all.then_some("stopped"));
    }

    fn end_episode(&self, st: &mut State, class: Class) {
        let Some(e) = st.episodes.remove(&class) else { return };
        let r = Record::new(
            class.kind(),
            json!({
                "class": class.name(), "episode": "ended", "count": e.count, "reasons": e.reasons,
                "began": rfc3339(e.began), "ended": rfc3339(e.last),
            }),
        );
        self.append(st, &r);
    }

    /// One refusal of `class`. The first of an episode is a record at once,
    /// with what is known of the peer; the rest are counted into the record
    /// that ends it. Returns the alert to emit, if this one crossed a line.
    fn refusal(&self, class: Class, reason: &'static str, conn: Option<u64>, peer: Option<String>) -> Alerts {
        self.refusal_in(class, reason, conn, peer, None)
    }

    /// [`Audit::refusal`], for a refused device key as well (`knock`): what
    /// names it goes into the record that begins an episode, and a known
    /// device's knock alerts once per episode for each reason (§5.3).
    fn refusal_in(&self, class: Class, reason: &'static str, conn: Option<u64>, peer: Option<String>, knock: Option<Knock>) -> Alerts {
        let mut alerts = Alerts::new();
        let mut st = guard(&self.st);
        self.sweep(&mut st, false);
        let t = st.wall();
        let (named, alert) = knock.map(|k| (k.detail, k.alert)).unwrap_or_default();
        if let Some(e) = st.episodes.get_mut(&class) {
            e.count += 1;
            e.last = t;
            *e.reasons.entry(reason).or_insert(0) += 1;
            if class == Class::Auth && e.count >= AUTH_ALERT_AT && !e.alerted {
                e.alerted = true;
                let a = json!({ "alert": "auth_failures", "count": e.count, "since": rfc3339(e.began) });
                self.append(&mut st, &Record::new("alert", a.clone()));
                alerts.push(a);
            }
        } else {
            let mut d = Map::from_iter([
                ("class".to_string(), json!(class.name())),
                ("episode".to_string(), json!("began")),
                ("reason".to_string(), json!(reason)),
            ]);
            d.extend(named);
            let mut r = Record::new(class.kind(), Value::Object(d));
            r.conn = conn;
            r.peer = peer;
            r.outcome = Some("refused");
            self.append(&mut st, &r);
            let e = Episode { began: t, last: t, count: 1, reasons: BTreeMap::from([(reason, 1)]), alerted: false, knocks: Default::default() };
            st.episodes.insert(class, e);
        }
        if let Some((key, a)) = alert {
            let fresh = st.episodes.get_mut(&class).is_some_and(|e| e.knocks.len() < MAX_KNOCK_ALERTS && e.knocks.insert(key));
            if fresh {
                self.append(&mut st, &Record::new("alert", a.clone()));
                alerts.push(a);
            }
        }
        alerts
    }

    fn emit(&self, alerts: Alerts) {
        if let Some(h) = &self.host {
            for a in alerts {
                h.emit("audit:alert", a);
            }
        }
    }

    fn record(&self, r: Record) {
        let mut st = guard(&self.st);
        self.sweep(&mut st, false);
        self.append(&mut st, &r);
    }

    // ── a device's budget ───────────────────────────────────────────────────

    /// A request record against its actor's budget: a paired device's is
    /// written if `charge` has its bytes and COUNTED into an episode if not,
    /// never dropped. A record that finds bytes is written even while an
    /// episode is open, so a device that slows down is back in full detail
    /// at once, and the episode ends `EPISODE_QUIET` after the last record it
    /// counted — not at the next one let through, which would begin and end
    /// an episode per request. Anyone else's is written as it is: the console
    /// is the owner, at the host.
    fn charged(&self, charge: Charge, r: Record) {
        let mut st = guard(&self.st);
        self.sweep(&mut st, false);
        let (who, id) = match r.actor.as_ref().map(|c| (c, c.device())) {
            Some((c, Device::Paired(id))) => (c.clone(), id.clone()),
            _ => return self.append(&mut st, &r),
        };
        let (now, epoch) = (st.mono(), st.wall());
        let cost = Self::cost(&st, &r);
        let spend = st.budgets.entry(id.clone()).or_insert_with(|| Spend::new(now));
        if spend.bucket(charge).take(charge.allowance(), cost, now) {
            return self.append(&mut st, &r);
        }
        let began = !spend.open.contains_key(&charge);
        let e = spend.open.entry(charge).or_insert_with(|| Coalesced {
            who: who.clone(),
            began: now,
            began_epoch: epoch,
            last: now,
            last_epoch: epoch,
            count: 0,
            bytes: 0,
            tally: BTreeMap::new(),
        });
        e.count += 1;
        e.bytes += cost as u64;
        e.last = now;
        e.last_epoch = epoch;
        let key = tally_key(&r);
        let key = if e.tally.len() < MAX_TALLY || e.tally.contains_key(&key) { key } else { "other".to_string() };
        *e.tally.entry(key).or_insert(0) += 1;
        if began {
            let a = charge.allowance();
            let mut b = Record::new(
                "budget",
                json!({
                    "class": charge.name(), "episode": "began",
                    "first": { "kind": r.kind, "cmd": r.cmd, "outcome": r.outcome },
                    "allowance": { "burst": a.burst, "per_sec": a.per_sec },
                }),
            );
            b.actor = Some(who);
            self.forced(&mut st, &id, charge, b);
        }
    }

    /// An episode record: always written, and paid for by the device out of
    /// `charge`, so the records that report a flood are inside its ceiling.
    fn forced(&self, st: &mut State, id: &DeviceId, charge: Charge, r: Record) {
        let (cost, now) = (Self::cost(st, &r), st.mono());
        if let Some(s) = st.budgets.get_mut(id) {
            s.bucket(charge).debit(charge.allowance(), cost, now);
        }
        self.append(st, &r);
    }

    /// End every device episode that has gone quiet or run its span — or
    /// every one, saying `all` — then forget the devices with nothing left
    /// to remember: no connection, no episode, both budgets full.
    fn sweep_budgets(&self, st: &mut State, all: Option<&'static str>) {
        let now = st.mono();
        let quiet = Duration::from_secs(EPISODE_QUIET as u64);
        let mut done = Vec::new();
        for (id, s) in &st.budgets {
            for (c, e) in &s.open {
                let why = all
                    .or_else(|| (now.saturating_duration_since(e.last) >= quiet).then_some("quiet"))
                    .or_else(|| (now.saturating_duration_since(e.began) >= EPISODE_SPAN).then_some("span"));
                if let Some(why) = why {
                    done.push((id.clone(), *c, why));
                }
            }
        }
        for (id, c, why) in done {
            self.end_coalesced(st, &id, c, why);
        }
        st.budgets.retain(|_, s| {
            s.acts.refill(ACTS, now);
            s.refusals.refill(REFUSALS, now);
            s.conns > 0 || !s.open.is_empty() || s.acts.tokens < ACTS.burst || s.refusals.tokens < REFUSALS.burst
        });
    }

    /// The record that ends a device episode: how many records it counted,
    /// what they would have cost, what they were, and what ended it.
    fn end_coalesced(&self, st: &mut State, id: &DeviceId, charge: Charge, why: &'static str) {
        let Some(e) = st.budgets.get_mut(id).and_then(|s| s.open.remove(&charge)) else { return };
        let mut r = Record::new(
            "budget",
            json!({
                "class": charge.name(), "episode": "ended", "count": e.count, "bytes": e.bytes, "tally": e.tally,
                "began": rfc3339(e.began_epoch), "ended": rfc3339(e.last_epoch), "closed_by": why,
            }),
        );
        r.actor = Some(e.who);
        self.forced(st, id, charge, r);
    }

    /// A connection of `c` was made.
    fn joined(&self, c: &Caller) {
        let Device::Paired(id) = c.device() else { return };
        let mut st = guard(&self.st);
        let now = st.mono();
        st.budgets.entry(id.clone()).or_insert_with(|| Spend::new(now)).conns += 1;
    }

    /// A connection of `c` went: at the device's last, its episodes end, so
    /// what it did is on disk by the time anyone looks.
    fn left(&self, c: &Caller) {
        let Device::Paired(id) = c.device() else { return };
        let mut st = guard(&self.st);
        let last = st.budgets.get_mut(id).is_some_and(|s| {
            s.conns = s.conns.saturating_sub(1);
            s.conns == 0
        });
        if last {
            for charge in [Charge::Acts, Charge::Refusals] {
                self.end_coalesced(&mut st, id, charge, "disconnect");
            }
        }
    }

    // ── the devices (§5.1) ──────────────────────────────────────────────────

    /// Hear the device registry: every event it fires becomes a record.
    /// Weakly, as `Clients::listen` subscribes, so the registry's hook list
    /// does not keep the log alive; inside `quietly`, so an audit bug never
    /// reaches the registry's caller.
    pub fn hear(self: &Arc<Self>, devices: &DeviceRegistry) {
        let me = Arc::downgrade(self);
        devices.subscribe(Arc::new(move |e: &DeviceEvent| {
            if let Some(a) = me.upgrade() {
                quietly(|| a.device_event(e));
            }
        }));
    }

    /// One registry event, recorded. These records are NEVER charged to a
    /// budget, whoever acted: they are the evidence §5.2 reads, and they are
    /// bounded by state changes and the host's own acts, not by a request
    /// rate — a device revokes itself once and narrows a peer only so far.
    /// A refused device key is the one event that is not a record each: it
    /// joins the auth episodes, whose first is.
    pub fn device_event(&self, e: &DeviceEvent) {
        if let DeviceEvent::ConnectRefused { subject, spki_sha256, why } = e {
            return self.knock(subject.as_ref(), spki_sha256, *why);
        }
        let mut st = guard(&self.st);
        self.sweep(&mut st, false);
        let Some((r, alert)) = device_record(&mut st.pairing_for, e) else { return };
        self.append(&mut st, &r);
        if let Some(a) = alert {
            self.append(&mut st, &Record::new("alert", a.clone()));
            drop(st);
            self.emit(vec![a]);
        }
    }

    /// A device key refused at the handshake (§5.1's "an unknown certificate,
    /// a revoked certificate"): an auth failure like a failed handshake v1,
    /// in the same episodes. A known device's knock alerts (§5.3), once per
    /// episode for each device and reason.
    fn knock(&self, subject: Option<&Subject>, key: &str, why: ConnectRefusal) {
        let reason = match why {
            ConnectRefusal::Unknown => "unknown-key",
            ConnectRefusal::Revoked => "revoked",
            ConnectRefusal::Disabled => "disabled",
            ConnectRefusal::Expired => "expired",
            ConnectRefusal::Unreadable => "unreadable",
        };
        let detail = Map::from_iter([("subject".to_string(), json!(subject)), ("key".to_string(), json!(ident(key)))]);
        let alert = (why != ConnectRefusal::Unknown).then(|| {
            let who = subject.map(|s| s.device.as_str()).unwrap_or("-");
            (format!("{who} {reason}"), json!({ "alert": "device", "event": "connect_refused", "reason": reason, "subject": subject }))
        });
        let a = self.refusal_in(Class::Auth, reason, None, None, Some(Knock { detail, alert }));
        self.emit(a);
    }

    /// Move the log's clocks on, for the tests that wait out a quiet gap.
    #[cfg(test)]
    pub(crate) fn advance(&self, by: Duration) {
        guard(&self.st).skew += by;
    }

    // ── the events, one method each ─────────────────────────────────────────

    /// The listener is up: version, address, and what the boot check found.
    pub fn started(&self, port: u16, chain: &Verdict) {
        self.record(Record::new(
            "lifecycle",
            json!({
                "event": "started", "version": env!("CARGO_PKG_VERSION"), "interface": "loopback",
                "address": format!("127.0.0.1:{port}"), "pin": null, "chain": chain.summary(),
            }),
        ));
    }

    /// The listener is down. Open episodes end first, so their counts land.
    pub fn stopped(&self) {
        let mut st = guard(&self.st);
        self.sweep(&mut st, true);
        self.append(&mut st, &Record::new("lifecycle", json!({ "event": "stopped" })));
    }

    /// A connection refused at the HTTP upgrade.
    pub fn upgrade_refused(&self, reason: &'static str, peer: Option<String>) {
        let a = self.refusal(Class::Upgrade, reason, None, peer);
        self.emit(a);
    }

    /// A handshake v1 that did not complete.
    pub fn auth_failed(&self, reason: &'static str, conn: u64, peer: Option<String>) {
        let a = self.refusal(Class::Auth, reason, Some(conn), peer);
        self.emit(a);
    }

    /// A connection cap reached: the first of an episode is a record.
    pub fn limit_reached(&self, limit: &'static str) {
        let a = self.refusal(Class::Limit, limit, None, None);
        self.emit(a);
    }

    /// Below the caps again: the episode ends with its count.
    pub fn limit_cleared(&self) {
        let mut st = guard(&self.st);
        self.end_episode(&mut st, Class::Limit);
    }

    #[cfg(test)]
    pub(crate) fn fail_writes(&self, writes: usize, land: usize) {
        guard(&self.st).fail = Some((writes, land));
    }
}

/// A registry event as its record (§5.1), with its alert (§5.3) if it raises
/// one. Exhaustive, so an event the registry gains is a compile error here,
/// not a kind left unrecorded. A record about a device names it in
/// `detail.subject`; serve's own acts have no actor and say `by: serve`.
/// `pairing_for` is the open window's principal, which every pairing record
/// names (`detail.for_principal`). `None` only for a refused key, which is
/// an episode's to count ([`Audit::knock`]).
fn device_record(pairing_for: &mut Option<PrincipalId>, e: &DeviceEvent) -> Option<(Record, Option<Value>)> {
    use DeviceEvent as E;
    let console = || Some(Caller::console());
    let serve = json!(Actor::Serve);
    let (kind, actor, peer, detail) = match e {
        E::Enrolled { subject, name, platform, fields } => (
            "device",
            console(),
            None,
            json!({ "event": "enrolled", "subject": subject, "name": ident(name), "platform": platform, "fields": fields }),
        ),
        E::Revoked { subject, by } => ("device", by_caller(by), None, json!({ "event": "revoked", "subject": subject, "by": by })),
        E::Disabled { subject, reason, by } => {
            ("device", by_caller(by), None, json!({ "event": "disabled", "subject": subject, "reason": reason, "by": by }))
        }
        E::Enabled { subject } => ("device", console(), None, json!({ "event": "enabled", "subject": subject })),
        E::ExpiryChanged { subject, from, to } => {
            ("device", console(), None, json!({ "event": "expiry_changed", "subject": subject, "from": from, "to": to }))
        }
        E::StepupChanged { subject, required } => {
            ("device", console(), None, json!({ "event": "stepup_changed", "subject": subject, "required": required }))
        }
        E::ScopeChanged { subject, by, from, to, widened, reduced } => (
            "device",
            by_caller(by),
            None,
            json!({
                "event": "scope_changed", "subject": subject, "by": by, "from": from, "to": to,
                "widened": widened, "reduced": reduced,
            }),
        ),
        E::IdentityRotated { pin } => ("device", console(), None, json!({ "event": "identity_rotated", "pin": pin })),
        E::Forgotten { subject } => ("device", None, None, json!({ "event": "forgotten", "subject": subject, "by": serve })),
        E::Unreadable { why } => ("device", None, None, json!({ "event": "unreadable", "why": ident(why), "by": serve })),
        E::Reloaded => ("device", None, None, json!({ "event": "reloaded", "by": serve })),
        E::ConnectRefused { .. } => return None,
        // The event holds neither the link nor its secret, and the record
        // could not name them if it did.
        E::PairingOpened { for_principal, expires_at, grant, expiry } => {
            *pairing_for = Some(for_principal.clone());
            let d = json!({
                "event": "opened", "for_principal": for_principal, "expires_at": rfc3339(*expires_at),
                "grant": grant, "expiry": expiry,
            });
            ("pairing", console(), None, d)
        }
        E::PairingAttempt { for_principal, peer } => {
            ("pairing", None, Some(ident(peer)), json!({ "event": "attempt", "for_principal": for_principal }))
        }
        // The comparison code is the host's dialog's, and never a record's.
        E::PairingPending { name, platform, code: _, peer } => (
            "pairing",
            None,
            Some(ident(peer)),
            json!({ "event": "verified", "for_principal": pairing_for, "name": ident(name), "platform": platform }),
        ),
        E::PairingDecided { allow } => ("pairing", console(), None, json!({ "event": "decided", "for_principal": pairing_for, "allow": allow })),
        E::PairingClosed { ended, device } => {
            let subject = device.as_ref().map(|d| json!({ "principal": pairing_for, "device": d }));
            ("pairing", None, None, json!({ "event": "closed", "for_principal": pairing_for, "ended": ended, "subject": subject, "by": serve }))
        }
    };
    let alert = e.alerts().then(|| {
        json!({ "alert": "device", "event": detail["event"], "subject": detail.get("subject"), "reason": detail.get("reason") })
    });
    let mut r = Record::new(kind, detail);
    r.actor = actor;
    r.peer = peer;
    Some((r, alert))
}

// ── one connection's view ───────────────────────────────────────────────────

/// The most attaches one connection's tally holds. Each entry is an attach
/// this connection OPENED and has not closed, so the real number is small;
/// the cap is the bound for a client that opens and never closes. Past it an
/// open is still recorded, only its close comes without totals.
const MAX_TALLIES: usize = 256;

/// One attach's running total, from its open to its close. Only for an
/// attach this connection opened: traffic to any other key starts nothing,
/// or a client could grow the map one made-up id at a time (and the
/// resolver's `attach(id)` refuses that traffic anyway).
struct Tally {
    project: Option<String>,
    object: Option<String>,
    bytes: u64,
    writes: u64,
    since: Instant,
}

/// What `begin` saw before the body ran, for `end` to finish.
pub struct Pending {
    cmd: &'static str,
    cap: Option<&'static str>,
    project: Option<String>,
    object: Option<String>,
    detail: Map<String, Value>,
    what: What,
    t0: Instant,
}

enum What {
    Call,
    Open(Option<String>),
    Close(String),
}

/// How a body ended, as a record says it.
pub enum Outcome<'a> {
    Ok(&'a Value),
    Err,
    Panic,
}

/// The audit, seen from one authenticated connection: who it is and the
/// attaches it holds. Made when the client's proof verifies.
pub struct ConnAudit {
    audit: Arc<Audit>,
    conn: u64,
    peer: Option<String>,
    caller: Caller,
    streams: Mutex<HashMap<String, Tally>>,
}

/// The tally key of a dock shell, as the lane key spells it, each part cut
/// to `MAX_IDENT`: the values are the client's, and only looked up with.
fn shell_key(args: &Args) -> String {
    let part = |k: &str| ident(&arg(args, k).cloned().unwrap_or(Value::Null).to_string());
    format!("shell:{}:{}:{}", part("repo"), part("slug"), part("index"))
}

fn term_key(id: &Value) -> String {
    format!("term:{}", ident(&id.to_string()))
}

/// Whether an admitted call of `row` is a record: every row whose capability
/// is not `read` (§5.1), the row's own column — so the `HostConsole` and
/// `Loopback` rows, which declare none, are recorded — except stream
/// traffic, which is totalled into its attach's close instead.
fn recorded(row: &Row) -> bool {
    row.cap != Some(Capability::Read) && !STREAM_TRAFFIC.contains(&row.name)
}

/// A refusal record's `reason`. Exhaustive, so a kind the host gains is a
/// compile error here and not a record with no reason.
fn reason(kind: RefusalKind) -> &'static str {
    match kind {
        RefusalKind::ClientLocal => "client-local",
        RefusalKind::ConsoleOnly => "console-only",
        RefusalKind::LoopbackOnly => "loopback-only",
        RefusalKind::UnavailableRemote => "unavailable-remote",
        RefusalKind::Denied => "denied",
        RefusalKind::NotFound => "not-found",
        RefusalKind::BadArgs => "bad-args",
        RefusalKind::NoopNotOwned => "not-owned",
        RefusalKind::LocalFetch => "local-fetch",
        RefusalKind::DeviceLanes => "device-lanes",
        RefusalKind::StepUp => "step-up",
    }
}

/// Whether a refusal's own words (`Refusal::detail`, the reason the caller is
/// not told) may go into its record. They name what was asked for — a path, a
/// slug, a session, a terminal id — except for two kinds: a malformed
/// argument's error quotes its value (serde's `invalid type: string "…"`),
/// and a refused fetch's can quote the URL, which can carry a credential, or
/// git's stderr.
fn why_is_safe(kind: RefusalKind) -> bool {
    !matches!(kind, RefusalKind::BadArgs | RefusalKind::LocalFetch)
}

impl ConnAudit {
    /// A paired device's connections are counted, so its budget's episodes
    /// end as its last one goes (`Drop`).
    pub fn new(audit: Arc<Audit>, conn: u64, peer: Option<String>, caller: Caller) -> ConnAudit {
        audit.joined(&caller);
        ConnAudit { audit, conn, peer, caller, streams: Mutex::new(HashMap::new()) }
    }

    fn base(&self, kind: &'static str, cmd: Option<&'static str>, detail: Value) -> Record {
        let mut r = Record::new(kind, detail);
        r.actor = Some(self.caller.clone());
        r.conn = Some(self.conn);
        r.peer = self.peer.clone();
        r.cmd = cmd;
        r
    }

    /// One request's witness, for `dispatch::call_seen`.
    pub fn request(&self) -> Seen<'_> {
        Seen { au: self, pending: RefCell::new(None) }
    }

    /// A name no row has, logged as `unknown` with the length of what was
    /// sent (§5.1): the name itself is the client's, and not ours to write.
    pub fn unknown(&self, cmd: &str) {
        let mut r = self.base("refused", Some("unknown"), json!({ "len": cmd.len(), "reason": "unknown command" }));
        r.outcome = Some("refused");
        self.audit.charged(Charge::Refusals, r);
    }

    /// A request the resolver refused (§5.1: "every refusal: an exposure, a
    /// capability or scope"), with its kind and, where they hold no argument's
    /// contents, the words the caller was not told. The stale close is one
    /// (`not-owned`): its caller was answered `ok`, and the record says so.
    ///
    /// The one refusal NOT recorded is the console's `not-found` on a row
    /// whose call would not be: nothing is outside the console's scope, so for
    /// it that is a miss — a brief not written yet, an attach already closed —
    /// the body error it always was, and a reader polling for one would
    /// otherwise write a line a poll.
    pub fn refused(&self, row: &'static Row, args: &Args, r: &Refusal) {
        if r.kind == RefusalKind::NotFound && self.caller.is_console() && !recorded(row) {
            return;
        }
        let mut detail = Map::new();
        detail.insert("reason".into(), json!(reason(r.kind)));
        if why_is_safe(r.kind) {
            detail.insert("why".into(), json!(ident(&r.detail)));
        }
        if r.kind == RefusalKind::NoopNotOwned {
            detail.insert("answered".into(), json!("ok"));
        }
        let mut rec = self.base("refused", Some(row.name), Value::Object(detail));
        rec.cap = row.cap.map(Capability::name);
        rec.project = arg_ident(args, PROJECT_KEYS);
        rec.object = arg_ident(args, OBJECT_KEYS);
        rec.outcome = Some("refused");
        self.audit.charged(Charge::Refusals, rec);
    }

    /// The resolver admitted `row`, whose body is about to run on `args` (the
    /// canonical values) for `projects` (the registry keys its objects
    /// resolved to): what to record once it is done, if anything. Stream
    /// traffic is totalled into its attach's tally, if this connection opened
    /// it, and never recorded on its own.
    pub fn begin(&self, row: &'static Row, args: &Args, projects: &[String]) -> Option<Pending> {
        let name = row.name;
        if STREAM_TRAFFIC.contains(&name) {
            if let Some(Value::Array(data)) = arg(args, "data") {
                let key = if name == "shell_write" { shell_key(args) } else { term_key(arg(args, "id").unwrap_or(&Value::Null)) };
                if let Some(t) = guard(&self.streams).get_mut(&key) {
                    t.bytes += data.len() as u64;
                    t.writes += 1;
                }
            }
            return None;
        }
        if !recorded(row) {
            return None;
        }
        let mut detail = Map::new();
        let what = if STREAM_OPEN.contains(&name) {
            What::Open((name == "shell_open").then(|| shell_key(args)))
        } else if STREAM_CLOSE.contains(&name) {
            What::Close(if name == "term_close" { term_key(arg(args, "id").unwrap_or(&Value::Null)) } else { shell_key(args) })
        } else {
            What::Call
        };
        if matches!(what, What::Open(_) | What::Close(_)) {
            if let Some(i) = arg(args, "index").and_then(Value::as_u64) {
                detail.insert("index".into(), json!(i));
            }
        }
        match name {
            // The path, the size and a SHA-256 of the content: a write can be
            // matched against a file later without the log holding the file.
            "write_file" => {
                if let Some(Value::String(c)) = arg(args, "content") {
                    detail.insert("bytes".into(), json!(c.len()));
                    detail.insert("sha256".into(), json!(sha256_hex(c.as_bytes())));
                }
            }
            // The one remove that can destroy commits (AGENTS.md): what it
            // destroys, read BEFORE the body, in the form that restores it.
            "remove_place" => {
                let flag = |k: &str| arg(args, k).and_then(Value::as_bool).unwrap_or(false);
                detail.insert("del_branch".into(), json!(flag("del_branch")));
                detail.insert("force".into(), json!(flag("force")));
                if flag("del_branch") {
                    if let Some((branch, tip)) = branch_tip(args) {
                        detail.insert("branch".into(), json!(ident(&branch)));
                        detail.insert("tip".into(), json!(tip));
                    }
                }
            }
            _ => {}
        }
        Some(Pending {
            cmd: name,
            cap: row.cap.map(Capability::name),
            // The first project the row's objects resolved to; the argument
            // only where nothing resolved one (a `new_path`, a global row).
            project: projects.first().map(|p| ident(p)).or_else(|| arg_ident(args, PROJECT_KEYS)),
            object: arg_ident(args, OBJECT_KEYS),
            detail,
            what,
            t0: Instant::now(),
        })
    }

    /// After the body: the record, with its outcome and time.
    pub fn end(&self, p: Pending, outcome: Outcome) {
        let ms = p.t0.elapsed().as_millis() as u64;
        let label = match outcome {
            // A CmdResult-shaped answer is `Ok` on the wire whether or not the
            // act happened: its own `ok` says which.
            Outcome::Ok(v) if v.get("ok") == Some(&Value::Bool(false)) => "err",
            Outcome::Ok(_) => "ok",
            Outcome::Err => "err",
            Outcome::Panic => "panic",
        };
        let mut detail = p.detail;
        let mut kind = "call";
        match &p.what {
            What::Call => {}
            What::Open(shell) => {
                kind = "stream";
                detail.insert("event".into(), json!("opened"));
                let key = match (shell, &outcome) {
                    (Some(k), Outcome::Ok(_)) => Some(k.clone()),
                    (None, Outcome::Ok(id)) => {
                        detail.insert("attach".into(), (*id).clone());
                        Some(term_key(id))
                    }
                    _ => None,
                };
                if let Some(key) = key {
                    let tally = Tally { project: p.project.clone(), object: p.object.clone(), bytes: 0, writes: 0, since: Instant::now() };
                    let old = {
                        let mut s = guard(&self.streams);
                        if s.len() < MAX_TALLIES || s.contains_key(&key) {
                            s.insert(key, tally)
                        } else {
                            detail.insert("tallied".into(), json!(false));
                            None
                        }
                    };
                    // A re-attach of a shell this connection already held
                    // (StrictMode, a tab flip) closes the old tally first.
                    if let Some(t) = old {
                        self.closed(None, t, "reattached");
                    }
                }
            }
            What::Close(key) => {
                kind = "stream";
                detail.insert("event".into(), json!("closed"));
                detail.insert("how".into(), json!(p.cmd));
                let t = guard(&self.streams).remove(key);
                detail.insert("opened_here".into(), json!(t.is_some()));
                if let Some(t) = t {
                    detail.insert("bytes_written".into(), json!(t.bytes));
                    detail.insert("writes".into(), json!(t.writes));
                    detail.insert("secs".into(), json!(t.since.elapsed().as_secs()));
                }
            }
        }
        let mut r = self.base(kind, Some(p.cmd), Value::Object(detail));
        r.cap = p.cap;
        r.project = p.project;
        r.object = p.object;
        r.outcome = Some(label);
        r.ms = Some(ms);
        self.audit.charged(Charge::Acts, r);
    }

    fn closed(&self, cmd: Option<&'static str>, t: Tally, how: &'static str) {
        let mut r = self.base(
            "stream",
            cmd,
            json!({
                "event": "closed", "how": how, "bytes_written": t.bytes, "writes": t.writes,
                "secs": t.since.elapsed().as_secs(), "opened_here": true,
            }),
        );
        r.cap = Some(Capability::Stream.name());
        r.project = t.project;
        r.object = t.object;
        r.outcome = Some("ok");
        self.audit.charged(Charge::Acts, r);
    }

    /// A limit an authenticated client tripped that ends its connection (a
    /// message past `MAX_MESSAGE`, an outbox past its budget): once, as the
    /// connection goes.
    pub fn limit(&self, limit: &'static str) {
        let mut r = self.base("refused", None, json!({ "limit": limit }));
        r.outcome = Some("refused");
        self.audit.record(r);
    }

    /// A budget limit (`budget.rs`, §6) began refusing this caller: the
    /// first refusal of an episode, answered `answered` (`busy`, `rate`, or
    /// `closed` for a device's connection past its cap). The rest are
    /// counted by the budget into [`ConnAudit::limit_ended`], never a record
    /// each, so a flood stays two records (§5.1).
    pub fn limit_began(&self, limit: &'static str, answered: &'static str) {
        let detail = json!({ "class": "limit", "episode": "began", "limit": limit, "answered": answered });
        let mut r = self.base("refused", None, detail);
        r.outcome = Some("refused");
        self.audit.record(r);
    }

    /// The episode `limit_began` opened has ended: `count` refusals, from
    /// `began` (epoch seconds) over `secs`.
    pub fn limit_ended(&self, limit: &'static str, count: u64, began: i64, secs: u64) {
        let detail = json!({
            "class": "limit", "episode": "ended", "limit": limit, "count": count,
            "began": rfc3339(began), "secs": secs,
        });
        let mut r = self.base("refused", None, detail);
        r.outcome = Some("refused");
        self.audit.record(r);
    }

    /// The client went: every attach it still held is closed, with its total.
    pub fn disconnected(&self) {
        let open: Vec<Tally> = guard(&self.streams).drain().map(|(_, t)| t).collect();
        for t in open {
            self.closed(None, t, "disconnect");
        }
    }
}

impl Drop for ConnAudit {
    fn drop(&mut self) {
        quietly(|| self.audit.left(&self.caller));
    }
}

/// One request, as the audit sees it: dispatch's [`Witness`], made by
/// [`ConnAudit::request`] and finished by `conn::answer` once the body is
/// done. The hooks run INSIDE the body's `catch_unwind`, where a panic would
/// be answered as the body's, so each keeps its own: an audit bug loses its
/// record, never the reply and never the body.
pub struct Seen<'a> {
    au: &'a ConnAudit,
    pending: RefCell<Option<Pending>>,
}

fn quietly<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

impl Witness for Seen<'_> {
    fn unknown(&self, cmd: &str) {
        quietly(|| self.au.unknown(cmd));
    }

    fn refused(&self, row: &'static Row, args: &Args, r: &Refusal) {
        quietly(|| self.au.refused(row, args, r));
    }

    fn admitted(&self, row: &'static Row, args: &Args, resolved: &Resolved) {
        let p = quietly(|| self.au.begin(row, args, &resolved.projects)).flatten();
        *self.pending.borrow_mut() = p;
    }
}

impl Seen<'_> {
    /// After the body, or its panic: the record `admitted` left, if any.
    pub fn finish(self, outcome: Outcome) {
        if let Some(p) = self.pending.into_inner() {
            self.au.end(p, outcome);
        }
    }
}

/// The branch a place has checked out and the commit at its tip, read before
/// `remove_place --branch` runs: `git branch <name> <sha>` restores it while
/// the object exists. `None` for a detached place, a slug that is not one
/// path component, or anything git will not answer.
fn branch_tip(args: &Args) -> Option<(String, String)> {
    let repo = arg(args, "repo")?.as_str()?;
    let slug = arg(args, "slug")?.as_str()?;
    if slug.is_empty() || slug.contains('/') || slug == "." || slug == ".." || slug.starts_with('-') {
        return None;
    }
    let p = worktrees_core::project::Project::discover(Path::new(repo)).ok()?;
    let dir = p.place_dir(slug);
    let branch = p.wt_branch(&dir);
    if branch.is_empty() || branch == "(detached)" {
        return None;
    }
    let tip = worktrees_core::git::git_out(&p.main_root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}^{{commit}}")])?;
    Some((branch, tip))
}

// Inline, not `mod tests;` in a file of its own: `fsx`'s temp-name scan skips
// a column-0 `#[cfg(test)]` block, and cannot see one declared from another
// file (serve/tests.rs needed an allowlist row for exactly that).
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    use super::super::resolve::fixture::{call_as, git_repo, s, scratch, Devices, Fixture};
    use super::super::resolve::World;
    use super::super::scope;
    use worktrees_core::authz::{Caps, DeviceId, Grant, NoDevices, PrincipalId, Projects};

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-audit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The live file's lines, parsed.
    fn live(d: &Path) -> Vec<Value> {
        file_lines(&d.join(FILE))
    }

    fn file_lines(p: &Path) -> Vec<Value> {
        std::fs::read_to_string(p).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn raw_lines(p: &Path) -> Vec<String> {
        std::fs::read_to_string(p).unwrap().lines().map(str::to_string).collect()
    }

    fn write_lines(p: &Path, lines: &[String]) {
        std::fs::write(p, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    }

    fn args(v: Value) -> Args {
        Args(v.as_object().unwrap().clone())
    }

    /// A connection's view, as conn.rs makes one once a client is in.
    fn console(a: &Arc<Audit>) -> ConnAudit {
        ConnAudit::new(a.clone(), 7, Some("127.0.0.1".into()), Caller::console())
    }

    fn row(cmd: &str) -> &'static Row {
        dispatch::row(cmd).unwrap_or_else(|| panic!("no row {cmd}"))
    }

    /// What `Seen::admitted` asks once the resolver has let `cmd` through,
    /// with no project resolved: the record's project is then the argument.
    fn begin(c: &ConnAudit, cmd: &str, a: &Args) -> Option<Pending> {
        c.begin(row(cmd), a, &[])
    }

    /// One admitted mutation, begun and ended as `Seen` does it.
    fn call(c: &ConnAudit, cmd: &str, a: Value) {
        let a = args(a);
        if let Some(p) = begin(c, cmd, &a) {
            c.end(p, Outcome::Ok(&Value::Null));
        }
    }

    /// One request through dispatch, as `conn::answer` makes it: the
    /// resolver's verdict reaches the audit through `Seen`, the outcome after.
    fn through(cx: &dispatch::Call, c: &ConnAudit, w: &dyn World, cmd: &str, a: Value) -> Result<Value, Value> {
        let seen = c.request();
        let r = dispatch::call_in_seen(cx, cmd, &args(a), w, &seen);
        seen.finish(match &r {
            Ok(v) => Outcome::Ok(v),
            Err(_) => Outcome::Err,
        });
        r
    }

    /// `n` records on a fresh log in `d`.
    fn some_records(d: &Path, n: usize) -> Arc<Audit> {
        let (a, _) = Audit::open(d, None, Fleet::none());
        let c = console(&a);
        for i in 0..n {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
        }
        a
    }

    #[derive(Default)]
    struct Catch(Mutex<Vec<(String, Value)>>);
    impl Host for Catch {
        fn emit(&self, event: &str, payload: Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    #[test]
    fn a_fresh_log_opens_with_genesis_is_owner_only_and_verifies() {
        let d = dir("fresh");
        some_records(&d, 3);
        let l = live(&d);
        assert_eq!(l.len(), 4, "a checkpoint and three records: {l:?}");
        assert_eq!((l[0]["kind"].as_str(), l[0]["detail"]["reason"].as_str()), (Some("checkpoint"), Some("genesis")));
        assert_eq!((l[0]["seq"].as_u64(), l[0]["prev"].as_str()), (Some(1), Some(GENESIS)));
        assert_eq!(l[1]["prev"].as_str().unwrap(), sha256_hex(raw_lines(&d.join(FILE))[0].as_bytes()));
        let mode = std::fs::metadata(d.join(FILE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "audit.jsonl is {mode:o}");
        let v = verify(&d);
        assert_eq!(v.broken, None);
        assert_eq!((v.records, v.last_seq, v.first_seq), (4, 4, Some(1)));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Every record names an actor, principal AND device, in every line —
    /// null before authentication, `owner`/`loopback` for the console.
    #[test]
    fn every_record_carries_principal_and_device() {
        let d = dir("actor");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        a.auth_failed("handshake", 3, Some("127.0.0.1".into()));
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
        for l in live(&d) {
            let o = l.as_object().unwrap();
            assert!(o.contains_key("principal") && o.contains_key("device"), "{l}");
        }
        let l = live(&d);
        assert_eq!((l[1]["principal"].clone(), l[1]["device"].clone()), (Value::Null, Value::Null), "{}", l[1]);
        assert_eq!((l[2]["principal"].as_str(), l[2]["device"].as_str()), (Some("owner"), Some("loopback")), "{}", l[2]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The brief's test: edit a record in the MIDDLE and the chain breaks at
    /// the record after it, which still names the old line's hash.
    #[test]
    fn an_edited_middle_record_breaks_the_chain_at_the_next_one() {
        let d = dir("edit");
        some_records(&d, 5);
        let p = d.join(FILE);
        let mut lines = raw_lines(&p);
        lines[3] = lines[3].replace("\"s2\"", "\"s9\"");
        write_lines(&p, &lines);
        let b = verify(&d).broken.expect("an edited record must break the chain");
        assert_eq!((b.file.as_str(), b.line), (FILE, 5), "{b:?}");
        assert_eq!(b.why, "prev is not the hash of the record before it");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_removed_middle_record_breaks_the_chain() {
        let d = dir("remove");
        some_records(&d, 5);
        let p = d.join(FILE);
        let mut lines = raw_lines(&p);
        lines.remove(3);
        write_lines(&p, &lines);
        let b = verify(&d).broken.expect("a removed record must break the chain");
        assert_eq!(b.line, 4, "{b:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn reordered_records_break_the_chain() {
        let d = dir("reorder");
        some_records(&d, 5);
        let p = d.join(FILE);
        let mut lines = raw_lines(&p);
        lines.swap(2, 3);
        write_lines(&p, &lines);
        let b = verify(&d).broken.expect("reordered records must break the chain");
        assert_eq!(b.line, 3, "{b:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A line replayed after itself: its `prev` names the line before the
    /// original, not the original.
    #[test]
    fn a_duplicated_record_breaks_the_chain() {
        let d = dir("dup");
        some_records(&d, 3);
        let p = d.join(FILE);
        let mut lines = raw_lines(&p);
        let last = lines.last().unwrap().clone();
        lines.push(last);
        write_lines(&p, &lines);
        assert!(verify(&d).broken.is_some(), "a replayed line must not verify");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A record dropped and every line after it RE-HASHED, so each `prev`
    /// holds: the hashes cannot see it, the `seq` jump can. (A writer that
    /// loses records quietly looks exactly like this; it must write `gap`.)
    #[test]
    fn a_rechained_tail_with_a_seq_jump_breaks_the_chain() {
        let d = dir("rechain");
        some_records(&d, 5);
        let p = d.join(FILE);
        let mut lines = raw_lines(&p);
        lines.remove(3);
        for i in 3..lines.len() {
            let mut v: Value = serde_json::from_str(&lines[i]).unwrap();
            v["prev"] = json!(sha256_hex(lines[i - 1].as_bytes()));
            lines[i] = serde_json::to_string(&v).unwrap();
        }
        write_lines(&p, &lines);
        let b = verify(&d).broken.expect("a seq jump must break the chain");
        assert_eq!((b.line, b.why), (4, "seq does not follow the record before it"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 5 MiB × 5 in production; a few hundred bytes here. Five files, each
    /// opening with a checkpoint, one chain across them, anchored at the
    /// oldest checkpoint left — and a reopened writer carries it on.
    #[test]
    fn rotation_keeps_five_files_and_one_chain_across_them() {
        let d = dir("rotate");
        let (a, _) = Audit::open_with(&d, None, Fleet::none(), 900);
        let c = console(&a);
        for i in 0..60 {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
        }
        let mut names: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, vec!["audit.jsonl", "audit.jsonl.1", "audit.jsonl.2", "audit.jsonl.3", "audit.jsonl.4"]);
        for n in &names {
            let first = &file_lines(&d.join(n))[0];
            assert_eq!(first["kind"].as_str(), Some("checkpoint"), "{n}: {first}");
            assert_eq!(first["detail"]["devices"], json!([]), "{n}: {first}");
        }
        let v = verify(&d);
        assert_eq!(v.broken, None, "{v:?}");
        assert!(v.first_seq.unwrap() > 1, "the genesis file was rotated away, so the oldest checkpoint anchors: {v:?}");
        drop(c);
        drop(a);
        let (a, again) = Audit::open_with(&d, None, Fleet::none(), 900);
        assert_eq!(again.broken, None);
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "after", "note": "x" }));
        let v2 = verify(&d);
        assert_eq!(v2.broken, None, "{v2:?}");
        assert!(v2.last_seq > v.last_seq);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Rotation removes the OLDEST file legitimately; a missing file in the
    /// MIDDLE is evidence removed, and the next file's checkpoint says so.
    #[test]
    fn a_missing_middle_file_breaks_the_chain() {
        let d = dir("hole");
        let (a, _) = Audit::open_with(&d, None, Fleet::none(), 900);
        let c = console(&a);
        for i in 0..60 {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
        }
        std::fs::remove_file(d.join("audit.jsonl.2")).unwrap();
        let b = verify(&d).broken.expect("a hole in the rotated files must break the chain");
        assert_eq!((b.file.as_str(), b.line), ("audit.jsonl.1", 1), "{b:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The fail-open policy, end to end: a write fails, the request is not
    /// refused (nothing here can refuse it), the writer goes unhealthy, and
    /// the next write that lands is preceded by a `gap` with the count and
    /// the episode's first and last times.
    #[test]
    fn a_failed_write_fails_open_and_the_gap_names_count_and_times() {
        let d = dir("gap");
        let a = some_records(&d, 1);
        assert!(a.healthy());
        a.fail_writes(3, 0);
        let c = console(&a);
        for i in 0..3 {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("lost{i}"), "note": "x" }));
        }
        assert!(!a.healthy(), "records were lost: a remote device must be refused now");
        call(&c, "set_note", json!({ "repo": "/r", "slug": "back", "note": "x" }));
        assert!(a.healthy());
        let l = live(&d);
        let gap = &l[l.len() - 2];
        assert_eq!(gap["kind"].as_str(), Some("gap"), "{l:?}");
        assert_eq!(gap["detail"]["lost"], json!(3), "{gap}");
        assert!(gap["detail"]["from"].as_str().is_some_and(|s| s.ends_with('Z')), "{gap}");
        assert!(gap["detail"]["to"].as_str().is_some_and(|s| s.ends_with('Z')), "{gap}");
        assert_eq!(l.last().unwrap()["object"].as_str(), Some("back"));
        assert_eq!(verify(&d).broken, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A write that left part of a line: the next write rotates first, the
    /// fragment stays where it was as evidence, and the new file's checkpoint
    /// names it to the byte — so the chain verifies.
    #[test]
    fn a_torn_write_is_rotated_past_and_named_by_the_checkpoint() {
        let d = dir("torn");
        let a = some_records(&d, 2);
        a.fail_writes(1, 10);
        let c = console(&a);
        call(&c, "set_note", json!({ "repo": "/r", "slug": "torn", "note": "x" }));
        call(&c, "set_note", json!({ "repo": "/r", "slug": "next", "note": "x" }));
        let old = std::fs::read(d.join("audit.jsonl.1")).unwrap();
        assert!(!old.ends_with(b"\n"), "the fragment is left in place");
        let l = live(&d);
        assert_eq!((l[0]["detail"]["reason"].as_str(), l[0]["detail"]["torn_bytes"].as_u64()), (Some("torn"), Some(10)), "{}", l[0]);
        assert_eq!(l[1]["kind"].as_str(), Some("gap"));
        let v = verify(&d);
        assert_eq!(v.broken, None, "{v:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The other half: a fragment no checkpoint accounts for. Planted at the
    /// end of a cleanly rotated file it changes no complete line, so the
    /// hashes cannot see it — only the torn rule can.
    #[test]
    fn a_fragment_the_checkpoint_does_not_name_is_a_break() {
        let d = dir("fragment");
        let (a, _) = Audit::open_with(&d, None, Fleet::none(), 900);
        let c = console(&a);
        for i in 0..12 {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
        }
        assert_eq!(verify(&d).broken, None);
        let mut f = OpenOptions::new().append(true).open(d.join("audit.jsonl.1")).unwrap();
        f.write_all(b"{\"seq\":").unwrap();
        drop(f);
        let b = verify(&d).broken.expect("an unnamed fragment must break the chain");
        assert_eq!((b.file.as_str(), b.line, b.why), (FILE, 1, "a torn record the next checkpoint does not account for"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A torn tail found at BOOT (a crash mid-line) is handled the same way.
    #[test]
    fn a_torn_tail_found_at_boot_is_rotated_past() {
        let d = dir("boot-torn");
        drop(some_records(&d, 2));
        let mut f = OpenOptions::new().append(true).open(d.join(FILE)).unwrap();
        f.write_all(b"{\"seq\":4,\"ts").unwrap();
        drop(f);
        let (a, v) = Audit::open(&d, None, Fleet::none());
        assert_eq!(v.live_torn, Some(12));
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "after", "note": "x" }));
        let l = live(&d);
        assert_eq!(l[0]["detail"]["torn_bytes"].as_u64(), Some(12), "{}", l[0]);
        assert_eq!(verify(&d).broken, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// §5.1: a file found wider than 0600 is NOT chmodded. The console carries
    /// on (fail open), and `healthy()` says no — what the remote listener
    /// will refuse to start on.
    #[test]
    fn a_wider_file_is_left_as_it_is_and_is_not_healthy() {
        let d = dir("wide");
        drop(some_records(&d, 1));
        std::fs::set_permissions(d.join(FILE), std::fs::Permissions::from_mode(0o644)).unwrap();
        let (a, _) = Audit::open(&d, None, Fleet::none());
        assert!(!a.healthy());
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
        let mode = std::fs::metadata(d.join(FILE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "the mode was changed behind the user's back");
        assert_eq!(live(&d).last().unwrap()["object"].as_str(), Some("s"), "the console fails open");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_symlink_at_the_name_is_refused_and_nothing_is_written_through_it() {
        let d = dir("link");
        let target = d.join("elsewhere");
        std::fs::write(&target, "untouched\n").unwrap();
        std::os::unix::fs::symlink(&target, d.join(FILE)).unwrap();
        let (a, _) = Audit::open(&d, None, Fleet::none());
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
        assert!(!a.healthy());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A FIFO at the name would hang a plain `open` for write until something
    /// read it — at serve's boot, before the listener. Bounded here, so a
    /// regression is a failure and not a hung suite.
    #[test]
    fn a_fifo_at_the_name_does_not_hang_the_boot() {
        let d = dir("fifo");
        let p = std::ffi::CString::new(d.join(FILE).to_string_lossy().as_bytes()).unwrap();
        // SAFETY: a valid C string; a plain syscall.
        assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let dd = d.clone();
        std::thread::spawn(move || {
            let (a, _) = Audit::open(&dd, None, Fleet::none());
            let _ = tx.send(a.healthy());
        });
        let healthy = rx.recv_timeout(Duration::from_secs(5)).expect("Audit::open hung on a FIFO");
        assert!(!healthy);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// L1's class, closed by construction: a client string with a newline and
    /// a forged record in it is one escaped field of one line, and argument
    /// CONTENT (a note's text) never reaches the file.
    #[test]
    fn a_client_string_cannot_add_a_line_and_content_is_never_recorded() {
        let d = dir("inject");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let forged = "s\n{\"seq\":99,\"kind\":\"forged\"}";
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": forged, "note": "the secret note text" }));
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert_eq!(text.lines().count(), 2, "a checkpoint and ONE record: {text}");
        assert!(!text.contains("the secret note text"), "{text}");
        assert_eq!(live(&d).last().unwrap()["object"].as_str(), Some(forged));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn identifiers_are_cut_to_a_bound_on_a_character_boundary() {
        let d = dir("bound");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let path = "é".repeat(40_000);
        call(&console(&a), "write_file", json!({ "path": path, "content": "abc" }));
        let o = live(&d).last().unwrap()["object"].as_str().unwrap().to_string();
        assert!(o.len() <= MAX_IDENT + '…'.len_utf8(), "{} bytes", o.len());
        assert!(o.ends_with('…'));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// §5.1: for `write_file`, the path, the size and a SHA-256 — never the
    /// content.
    #[test]
    fn a_file_write_is_recorded_by_size_and_hash() {
        let d = dir("write");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        call(&console(&a), "write_file", json!({ "path": "/r/AGENTS.md", "content": "be careful\n" }));
        let l = live(&d).last().unwrap().clone();
        assert_eq!(l["cmd"].as_str(), Some("write_file"));
        assert_eq!(l["object"].as_str(), Some("/r/AGENTS.md"));
        assert_eq!(l["detail"], json!({ "bytes": 11, "sha256": sha256_hex(b"be careful\n") }));
        assert!(!std::fs::read_to_string(d.join(FILE)).unwrap().contains("be careful"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The row's own capability decides (§5.1): a read is not recorded, a
    /// `write` row is, with its outcome and time, and so is a console act,
    /// whose row declares no capability at all.
    #[test]
    fn reads_are_not_recorded_and_every_other_row_is() {
        let d = dir("which");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        assert!(begin(&c, "list_workspace", &args(json!({}))).is_none());
        assert!(begin(&c, "read_file", &args(json!({ "path": "/x" }))).is_none());
        let p = begin(&c, "set_lifecycle", &args(json!({ "repo": "/r", "slug": "s", "lifecycle": "parked" }))).unwrap();
        c.end(p, Outcome::Err);
        let p = begin(&c, "update_cli", &args(json!({}))).unwrap();
        c.end(p, Outcome::Ok(&Value::Null));
        let l = live(&d);
        let tail: Vec<_> = l[1..].iter().map(|r| (r["kind"].as_str(), r["cmd"].as_str(), r["cap"].as_str(), r["outcome"].as_str())).collect();
        assert_eq!(
            tail,
            vec![(Some("call"), Some("set_lifecycle"), Some("write"), Some("err")), (Some("call"), Some("update_cli"), None, Some("ok"))]
        );
        assert!(l[1]["ms"].as_u64().is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A registered project `a` with a place `feat` and a file, as the
    /// resolver's world sees it.
    fn project(tag: &str) -> (PathBuf, String, Fixture) {
        let top = scratch(&format!("audit-{tag}"));
        let a = top.join("a");
        git_repo(&a);
        std::fs::create_dir_all(a.join(".worktrees/feat")).unwrap();
        std::fs::create_dir_all(a.join("src")).unwrap();
        std::fs::write(a.join("src/lib.rs"), "x").unwrap();
        let root = s(&a);
        (top, root.clone(), Fixture { roots: vec![root], panes: vec![] })
    }

    /// The resolver's refusals reach the log from the branch that makes them
    /// (`dispatch::call_in_seen`), each with its kind and the words the caller
    /// is not told: a write naming an unregistered project, a client-local
    /// row, a name no row has — and the stale close, which its caller is
    /// answered `ok` for. The console's ordinary miss on a read is not one.
    #[test]
    fn the_resolvers_refusals_are_recorded_where_it_makes_them() {
        let (top, root, w) = project("refuse");
        let d = dir("refuse");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        let cx = call_as(Caller::console(), Arc::new(NoDevices));
        let r = through(&cx, &c, &w, "set_note", json!({ "repo": "/not/registered", "slug": "feat", "note": "the secret note" }));
        assert!(r.is_err(), "{r:?}");
        assert_eq!(through(&cx, &c, &w, "copy_text", json!({ "text": "clipboard secret" })), Err(json!("client-local")));
        assert!(through(&cx, &c, &w, "no_such_cmd_at_all", json!({})).is_err());
        assert!(through(&cx, &c, &w, "read_file", json!({ "path": format!("{root}/no-brief-yet.md") })).is_err());
        assert_eq!(through(&cx, &c, &w, "term_close", json!({ "id": 7 })), Ok(Value::Null), "the stale close is still ok");
        let l = live(&d);
        let tail: Vec<_> = l[1..].iter().map(|r| (r["kind"].as_str(), r["cmd"].as_str(), r["detail"]["reason"].as_str())).collect();
        assert_eq!(
            tail,
            vec![
                (Some("refused"), Some("set_note"), Some("not-found")),
                (Some("refused"), Some("copy_text"), Some("client-local")),
                (Some("refused"), Some("unknown"), Some("unknown command")),
                (Some("refused"), Some("term_close"), Some("not-owned")),
            ],
            "{l:#?}"
        );
        assert_eq!(l[1]["detail"]["why"].as_str(), Some("not a registered project: /not/registered"));
        assert_eq!((l[1]["cap"].as_str(), l[1]["project"].as_str(), l[1]["object"].as_str()), (Some("write"), Some("/not/registered"), Some("feat")));
        assert_eq!((l[4]["detail"]["answered"].as_str(), l[4]["cap"].as_str(), l[4]["object"].as_str()), (Some("ok"), Some("stream"), Some("7")));
        assert!(l[1..].iter().all(|r| r["outcome"] == "refused" && r["principal"] == "owner" && r["device"] == "loopback"), "{l:#?}");
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert!(!text.contains("the secret note") && !text.contains("clipboard secret"), "{text}");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&top);
    }

    /// An admitted call names the project its objects RESOLVED to, not one
    /// spelled in an argument: a file write is its owning project's, though
    /// no project argument was sent, and its object is the canonical path the
    /// resolver checked and the body was handed.
    #[test]
    fn an_admitted_call_names_the_project_the_resolver_found() {
        let (top, root, w) = project("admit");
        let d = dir("admit");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        let cx = call_as(Caller::console(), Arc::new(NoDevices));
        let _ = through(&cx, &c, &w, "write_file", json!({ "path": format!("{root}/./src/../src/lib.rs"), "content": "y" }));
        let last = live(&d).last().unwrap().clone();
        assert_eq!(last["kind"].as_str(), Some("call"), "{last}");
        assert_eq!((last["cmd"].as_str(), last["cap"].as_str()), (Some("write_file"), Some("write")), "{last}");
        assert_eq!(last["project"].as_str(), Some(root.as_str()), "{last}");
        assert_eq!(last["object"], json!(format!("{root}/src/lib.rs")), "{last}");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&top);
    }

    /// A paired device's records name it — principal and its own device id —
    /// and every refusal it meets is recorded, a scope miss on a read
    /// included: for a device that is the probe the log is kept for.
    #[test]
    fn a_devices_refusals_name_it_and_include_its_misses() {
        let (top, root, w) = project("device");
        let d = dir("device");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let mut reg = Devices::default();
        let g = Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::only([root.clone()]) };
        reg.devices.insert(DeviceId::new("d_phone"), (PrincipalId::owner(), g));
        let phone = Caller::paired(PrincipalId::owner(), DeviceId::new("d_phone"));
        let cx = call_as(phone.clone(), Arc::new(reg));
        let c = ConnAudit::new(a.clone(), 9, Some("100.101.7.3".into()), phone);
        assert_eq!(through(&cx, &c, &w, "set_note", json!({ "repo": root, "slug": "feat", "note": "x" })), Err(json!("denied")));
        assert!(through(&cx, &c, &w, "read_file", json!({ "path": "/etc/hosts" })).is_err());
        let l = live(&d);
        let tail: Vec<_> = l[1..].iter().map(|r| (r["cmd"].as_str(), r["detail"]["reason"].as_str(), r["principal"].as_str(), r["device"].as_str())).collect();
        assert_eq!(
            tail,
            vec![
                (Some("set_note"), Some("denied"), Some("owner"), Some("d_phone")),
                (Some("read_file"), Some("not-found"), Some("owner"), Some("d_phone")),
            ],
            "{l:#?}"
        );
        assert_eq!(l[1]["detail"]["why"].as_str(), Some("set_note needs `write`"));
        assert_eq!(l[2]["peer"].as_str(), Some("100.101.7.3"));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&top);
    }

    /// The device-lane cap (§6, LLM10) is the host refusing, not a body that
    /// failed: with four device-launched lanes live, each launch row a device
    /// sends is recorded `refused` with reason `device-lanes` and the live
    /// list the device is not told, and there is no `call` record for it.
    /// Its project and object are the resolver's canonical values, as an
    /// admitted launch's are — the console's `open_place`, which the cap never
    /// meets, is a call that failed in its body.
    #[test]
    fn a_device_launch_over_the_lane_cap_is_recorded_as_a_refusal() {
        use super::super::provenance::{ERR_DEVICE_LANES, LAUNCH_ROWS};
        use worktrees_core::store::{self, By, LaunchedBy};
        let (top, root, mut w) = project("lanes");
        let phone = Caller::paired(PrincipalId::owner(), DeviceId::new("d_phone"));
        for slug in ["l1", "l2", "l3", "l4"] {
            let dir = Path::new(&root).join(".worktrees").join(slug);
            std::fs::create_dir_all(&dir).unwrap();
            let by = LaunchedBy { by: By::of(&phone), cross_project: None };
            store::edit(&root, slug, |d| d.launched_by = Some(by)).unwrap();
            w.panes.push((format!("p-{slug}"), s(&dir), "claude".into()));
        }
        let mut reg = Devices::default();
        let g = Grant { capabilities: Caps::of(&[Capability::Read, Capability::Write]), projects: Projects::All };
        reg.devices.insert(DeviceId::new("d_phone"), (PrincipalId::owner(), g));
        let d = dir("lanes");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = ConnAudit::new(a.clone(), 9, Some("100.101.7.3".into()), phone.clone());
        let cx = call_as(phone, Arc::new(reg));
        // No such harness: a launch that got past the cap would fail in its
        // body before it reached git or tmux.
        let launch = json!({ "repo": format!("{root}/."), "slug": "feat", "branch": "x", "provider": "no-such-harness" });
        for cmd in LAUNCH_ROWS {
            assert_eq!(through(&cx, &c, &w, cmd, launch.clone()), Err(json!(ERR_DEVICE_LANES)), "{cmd}");
        }
        let con = call_as(Caller::console(), Arc::new(NoDevices));
        assert!(through(&con, &console(&a), &w, "open_place", launch).is_err());
        let l = live(&d);
        let got: Vec<_> =
            l[1..].iter().map(|r| (r["kind"].as_str(), r["cmd"].as_str(), r["outcome"].as_str(), r["detail"]["reason"].as_str())).collect();
        assert_eq!(
            got,
            vec![
                (Some("refused"), Some("new_place"), Some("refused"), Some("device-lanes")),
                (Some("refused"), Some("open_place"), Some("refused"), Some("device-lanes")),
                (Some("refused"), Some("rules_review"), Some("refused"), Some("device-lanes")),
                (Some("call"), Some("open_place"), Some("err"), None),
            ],
            "{l:#?}"
        );
        let why = l[1]["detail"]["why"].as_str().unwrap_or_default();
        assert!(why.starts_with("device lanes live: ") && why.contains(&format!("{root}:l4")), "{why}");
        assert!(l[1..4].iter().all(|r| r["device"] == "d_phone" && r["cap"] == "write"), "{l:#?}");
        assert!(l[1..].iter().all(|r| r["project"].as_str() == Some(root.as_str())), "canonical, not `{root}/.`: {l:#?}");
        assert_eq!((l[2]["object"].as_str(), l[4]["object"].as_str()), (Some("feat"), Some("feat")));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&top);
    }

    /// A CmdResult-shaped answer is `Ok` on the wire whether or not the act
    /// happened, so its record reads the answer's own `ok`: a `new_place` its
    /// body refuses (a `-`-led branch, R3) — or a device launch core would not
    /// start without its `launched_by` — is an admitted call recorded `err`.
    #[test]
    fn a_body_that_answers_ok_false_is_recorded_err() {
        let (top, root, w) = project("okfalse");
        let d = dir("okfalse");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let cx = call_as(Caller::console(), Arc::new(NoDevices));
        let r = through(&cx, &console(&a), &w, "new_place", json!({ "repo": root, "branch": "--all" })).unwrap();
        assert_eq!(r["ok"], json!(false), "the caller is answered as the body answered: {r}");
        let last = live(&d).last().unwrap().clone();
        assert_eq!((last["kind"].as_str(), last["cmd"].as_str(), last["outcome"].as_str()), (Some("call"), Some("new_place"), Some("err")), "{last}");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&top);
    }

    /// A refusal's own words are left out where they can hold an argument's
    /// contents: serde quotes a malformed value, and a refused fetch can
    /// quote its URL, credential and all.
    #[test]
    fn a_refusal_that_would_quote_an_argument_keeps_only_its_kind() {
        let d = dir("why");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        for (kind, words) in [
            (RefusalKind::BadArgs, "invalid args `index`: invalid type: string \"a-secret\", expected u32"),
            (RefusalKind::LocalFetch, "not a URL: \"https://user:a-secret@host\""),
        ] {
            c.refused(row("clone_project"), &args(json!({})), &Refusal { kind, detail: words.into() });
        }
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert!(!text.contains("a-secret"), "{text}");
        let reasons: Vec<_> = live(&d)[1..].iter().map(|r| r["detail"].clone()).collect();
        assert_eq!(reasons, vec![json!({ "reason": "bad-args" }), json!({ "reason": "local-fetch" })]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// §5.3's alert is for the person at the host: every event this file
    /// emits is projected to the console alone, whatever a device's scope.
    #[test]
    fn the_audits_events_reach_the_console_only() {
        let shipped = include_str!("audit.rs").split("\n#[cfg(test)]\nmod tests {").next().unwrap();
        let emitted: Vec<&str> = shipped.split("emit(\"").skip(1).map(|p| p.split('"').next().unwrap()).collect();
        assert_eq!(emitted, vec!["audit:alert"]);
        let phone = Caller::paired(PrincipalId::owner(), DeviceId::new("d_phone"));
        for e in emitted {
            assert_eq!(scope::EVENTS.iter().find(|(n, _)| *n == e).map(|(_, h)| *h), Some(scope::Event::Console), "{e}");
            assert_eq!(scope::event(e, &json!({}), &phone, &Grant::everything(), &mut Vec::new), None, "{e}");
            assert!(scope::event(e, &json!({}), &Caller::console(), &Grant::everything(), &mut Vec::new).is_some(), "{e}");
        }
    }

    /// Streams: an open and a close, the writes in between totalled into the
    /// close — not one record per keystroke.
    #[test]
    fn stream_writes_are_totalled_into_the_close() {
        let d = dir("stream");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        let p = begin(&c, "term_open", &args(json!({ "session": "wt-x", "cols": 80, "rows": 24, "on_bytes": 1 }))).unwrap();
        c.end(p, Outcome::Ok(&json!(4)));
        for data in [json!([104, 105]), json!([10]), json!([1, 2, 3])] {
            assert!(begin(&c, "term_write", &args(json!({ "id": 4, "data": data }))).is_none());
        }
        assert!(begin(&c, "term_resize", &args(json!({ "id": 4, "cols": 90, "rows": 30 }))).is_none());
        call(&c, "term_close", json!({ "id": 4 }));
        let l = live(&d);
        assert_eq!(l.len(), 3, "a checkpoint, the open and the close: {l:?}");
        assert_eq!((l[1]["detail"]["event"].as_str(), l[1]["object"].as_str()), (Some("opened"), Some("wt-x")));
        assert_eq!(l[1]["detail"]["attach"], json!(4));
        assert_eq!(l[2]["detail"]["event"].as_str(), Some("closed"));
        assert_eq!((l[2]["detail"]["bytes_written"].as_u64(), l[2]["detail"]["writes"].as_u64()), (Some(6), Some(3)));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The security review's finding: traffic to attaches this connection
    /// never opened — made-up ids, made-up shells, writes the body refuses —
    /// starts no tally, so a client cannot grow the map; and the map of the
    /// ones it did open is capped.
    #[test]
    fn only_attaches_this_connection_opened_are_tallied_and_the_tally_is_capped() {
        let d = dir("tally-cap");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        for id in 0..1000 {
            assert!(begin(&c, "term_write", &args(json!({ "id": id, "data": [1] }))).is_none());
            let shell = json!({ "repo": format!("/r{id}"), "slug": "s", "index": 0, "data": [1] });
            assert!(begin(&c, "shell_write", &args(shell)).is_none());
        }
        assert_eq!(guard(&c.streams).len(), 0);
        for id in 0..(MAX_TALLIES + 10) {
            let p = begin(&c, "term_open", &args(json!({ "session": "wt-x", "cols": 80, "rows": 24, "on_bytes": 1 }))).unwrap();
            c.end(p, Outcome::Ok(&json!(id)));
        }
        assert_eq!(guard(&c.streams).len(), MAX_TALLIES);
        assert_eq!(live(&d).last().unwrap()["detail"]["tallied"], json!(false));
        // A detail value is never a client's raw value: an `index` that is
        // not a number is left out, not copied in at whatever size it came.
        let p = begin(&c, "shell_open", &args(json!({ "repo": "/r", "slug": "s", "index": "9".repeat(100_000) }))).unwrap();
        c.end(p, Outcome::Err);
        let last = raw_lines(&d.join(FILE)).pop().unwrap();
        assert!(last.len() < 4096, "{} bytes", last.len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_disconnect_closes_every_attach_still_held() {
        let d = dir("disconnect");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        let shell = json!({ "repo": "/r", "slug": "s", "index": 0, "cols": 80, "rows": 24, "on_bytes": 2 });
        let p = begin(&c, "shell_open", &args(shell)).unwrap();
        c.end(p, Outcome::Ok(&json!({ "replay": 0 })));
        assert!(begin(&c, "shell_write", &args(json!({ "repo": "/r", "slug": "s", "index": 0, "data": [97, 98] }))).is_none());
        c.disconnected();
        let last = live(&d).last().unwrap().clone();
        assert_eq!((last["detail"]["event"].as_str(), last["detail"]["how"].as_str()), (Some("closed"), Some("disconnect")), "{last}");
        assert_eq!(last["detail"]["bytes_written"].as_u64(), Some(2));
        assert_eq!((last["project"].as_str(), last["object"].as_str()), (Some("/r"), Some("s")));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A flood of failed handshakes is ONE record when it begins and one when
    /// it ends, with the count — and, past the threshold, one alert, which is
    /// also emitted to the host.
    #[test]
    fn failed_handshakes_are_an_episode_and_alert_past_the_threshold() {
        let d = dir("episode");
        let host = Arc::new(Catch::default());
        let (a, _) = Audit::open(&d, Some(host.clone()), Fleet::none());
        for i in 0..(AUTH_ALERT_AT + 5) {
            a.auth_failed(if i % 2 == 0 { "handshake" } else { "deadline" }, i, Some("127.0.0.1".into()));
        }
        a.stopped();
        let l = live(&d);
        let kinds: Vec<&str> = l.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["checkpoint", "auth", "alert", "auth", "lifecycle"], "{l:?}");
        assert_eq!(l[1]["detail"]["episode"].as_str(), Some("began"));
        assert_eq!(l[2]["detail"]["count"].as_u64(), Some(AUTH_ALERT_AT));
        assert_eq!(l[3]["detail"]["count"].as_u64(), Some(AUTH_ALERT_AT + 5));
        assert_eq!(l[3]["detail"]["reasons"], json!({ "deadline": 12, "handshake": 13 }));
        let emitted = host.0.lock().unwrap();
        assert_eq!(emitted.len(), 1, "{emitted:?}");
        assert_eq!(emitted[0].0, "audit:alert");
        assert_eq!(emitted[0].1["alert"].as_str(), Some("auth_failures"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// What is recorded is the table's capability column, with nothing kept
    /// beside it but the stream sets — so those must name stream rows, or a
    /// renamed one turns keystrokes into records. Every row a request can
    /// reach is a record but the reads and the traffic.
    #[test]
    fn what_is_recorded_is_the_tables_capability_column() {
        use super::super::dispatch::{Exposure, TABLE};
        for n in STREAM_OPEN.iter().chain(STREAM_CLOSE).chain(STREAM_TRAFFIC) {
            assert_eq!(row(n).cap, Some(Capability::Stream), "{n}");
        }
        let reads = TABLE.iter().filter(|r| r.cap == Some(Capability::Read)).count();
        let quiet: Vec<&str> = TABLE.iter().filter(|r| r.exposure != Exposure::ClientLocal && !recorded(r)).map(|r| r.name).collect();
        assert_eq!(quiet.len(), reads + STREAM_TRAFFIC.len(), "{quiet:?}");
        assert!(recorded(row("update_cli")) && recorded(row("ui_events_append")), "a console or loopback row declares no capability");
    }

    /// The one remove that can destroy commits records, BEFORE it runs, the
    /// branch and its tip: `git branch <name> <sha>` brings it back.
    #[test]
    fn removing_a_place_with_its_branch_records_the_tip() {
        let d = dir("tip");
        let repo = d.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |dir: &Path, a: &[&str]| {
            let o = std::process::Command::new("git").arg("-C").arg(dir).args(a).output().unwrap();
            assert!(o.status.success(), "git {a:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "one"]);
        git(&repo, &["worktree", "add", "-q", "-b", "feat/x", ".worktrees/feat-x"]);
        let tip = git(&repo, &["rev-parse", "feat/x"]);
        let repo_s = std::fs::canonicalize(&repo).unwrap().to_string_lossy().into_owned();
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        let p = begin(&c, "remove_place", &args(json!({ "repo": repo_s, "slug": "feat-x", "del_branch": true, "force": true }))).unwrap();
        c.end(p, Outcome::Ok(&Value::Null));
        let l = live(&d).last().unwrap().clone();
        assert_eq!(l["detail"], json!({ "del_branch": true, "force": true, "branch": "feat/x", "tip": tip }), "{l}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A panic while the writer's lock was held must not take the log — or,
    /// through `conn::answer`, every later reply — down with it.
    #[test]
    fn a_poisoned_lock_does_not_stop_the_log() {
        let d = dir("poison");
        let a = some_records(&d, 1);
        let a2 = a.clone();
        let _ = std::thread::spawn(move || {
            let _held = a2.st.lock().unwrap();
            panic!("a panic with the audit lock held");
        })
        .join();
        assert!(a.st.is_poisoned());
        let c = console(&a);
        call(&c, "set_note", json!({ "repo": "/r", "slug": "after", "note": "x" }));
        assert_eq!(live(&d).last().unwrap()["object"].as_str(), Some("after"));
        assert!(a.healthy());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `healthy()` is what a remote request will be refused on: true for a
    /// log that is open, fit and has lost nothing.
    #[test]
    fn healthy_is_open_fit_and_nothing_lost() {
        let d = dir("healthy");
        let a = some_records(&d, 1);
        assert!(a.healthy());
        a.fail_writes(1, 0);
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
        assert!(!a.healthy());
        let _ = std::fs::remove_dir_all(&d);
    }

    // ── devices (§5.1) and their budgets ────────────────────────────────────

    use super::super::devices::{DisabledReason, Enrollment, Expiry, Platform};
    use super::super::pairing::Ended;

    fn owner_subject(d: &str) -> Subject {
        Subject { principal: PrincipalId::owner(), device: DeviceId::new(d) }
    }

    fn fields(d: &str) -> SecurityFields {
        SecurityFields {
            principal: PrincipalId::owner(),
            device: DeviceId::new(d),
            spki_sha256: "a".repeat(64),
            stepup_spki_sha256: None,
            stepup_required: false,
            caps: Caps::of(&[Capability::Read]),
            projects: Projects::All,
            expiry: Expiry::default(),
            disabled: None,
            revoked_at: None,
        }
    }

    fn phone_caller() -> Caller {
        Caller::paired(PrincipalId::owner(), DeviceId::new("d_phone"))
    }

    /// A paired device's connection, as the remote listener will make one.
    fn phone(a: &Arc<Audit>, conn: u64) -> ConnAudit {
        ConnAudit::new(a.clone(), conn, Some("100.64.0.2".into()), phone_caller())
    }

    /// The comparison code the pairing event carries, which no line may.
    const CODE: &str = "864213";

    /// One of every event the registry fires, and the record each must make
    /// (kind, `detail.event`) — a `match`, so an event the registry gains
    /// fails to compile here until it has a record.
    fn every_event() -> Vec<(DeviceEvent, &'static str, Option<&'static str>)> {
        use DeviceEvent as E;
        let peer = Actor::Device { principal: PrincipalId::owner(), device: DeviceId::new("d_peer") };
        let grant = Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All };
        let events = vec![
            E::PairingOpened { for_principal: PrincipalId::owner(), expires_at: 1_800_000_000, grant: grant.clone(), expiry: Expiry::default() },
            E::PairingAttempt { for_principal: PrincipalId::owner(), peer: "100.64.0.9".into() },
            E::PairingPending { name: "Phone".into(), platform: Platform::Ios, code: CODE.into(), peer: "100.64.0.9".into() },
            E::PairingDecided { allow: true },
            E::Enrolled { subject: owner_subject("d_new"), name: "Phone".into(), platform: Platform::Ios, fields: fields("d_new") },
            E::PairingClosed { ended: Ended::Paired, device: Some(DeviceId::new("d_new")) },
            E::ExpiryChanged { subject: owner_subject("d_new"), from: Expiry::default(), to: Expiry::Never },
            E::StepupChanged { subject: owner_subject("d_new"), required: false },
            E::ScopeChanged { subject: owner_subject("d_new"), by: peer.clone(), from: grant.clone(), to: Grant::nothing(), widened: false, reduced: true },
            E::Disabled { subject: owner_subject("d_new"), reason: DisabledReason::Idle, by: Actor::Serve },
            E::Enabled { subject: owner_subject("d_new") },
            E::Revoked { subject: owner_subject("d_new"), by: peer },
            E::IdentityRotated { pin: "b".repeat(64) },
            E::Unreadable { why: "devices.json does not parse".into() },
            E::Reloaded,
            E::Forgotten { subject: owner_subject("d_gone") },
            E::ConnectRefused { subject: Some(owner_subject("d_new")), spki_sha256: "a".repeat(64), why: ConnectRefusal::Revoked },
        ];
        events
            .into_iter()
            .map(|e| {
                let (kind, event) = match &e {
                    E::Enrolled { .. } => ("device", Some("enrolled")),
                    E::Revoked { .. } => ("device", Some("revoked")),
                    E::Disabled { .. } => ("device", Some("disabled")),
                    E::Enabled { .. } => ("device", Some("enabled")),
                    E::ExpiryChanged { .. } => ("device", Some("expiry_changed")),
                    E::StepupChanged { .. } => ("device", Some("stepup_changed")),
                    E::ScopeChanged { .. } => ("device", Some("scope_changed")),
                    E::IdentityRotated { .. } => ("device", Some("identity_rotated")),
                    E::Forgotten { .. } => ("device", Some("forgotten")),
                    E::Unreadable { .. } => ("device", Some("unreadable")),
                    E::Reloaded => ("device", Some("reloaded")),
                    E::ConnectRefused { .. } => ("auth", None),
                    E::PairingOpened { .. } => ("pairing", Some("opened")),
                    E::PairingAttempt { .. } => ("pairing", Some("attempt")),
                    E::PairingPending { .. } => ("pairing", Some("verified")),
                    E::PairingDecided { .. } => ("pairing", Some("decided")),
                    E::PairingClosed { .. } => ("pairing", Some("closed")),
                };
                (e, kind, event)
            })
            .collect()
    }

    /// §5.1's device and pairing records: every event the registry fires is
    /// a record of its own, naming the device it is about in
    /// `detail.subject`, its actor in the actor fields (`by` for serve's own
    /// acts), a pairing's principal in `detail.for_principal` — and §5.3's
    /// alerts are alert records too, emitted to the host.
    #[test]
    fn every_device_event_is_a_record_naming_its_subject() {
        let d = dir("devev");
        let host = Arc::new(Catch::default());
        let (a, _) = Audit::open(&d, Some(host.clone()), Fleet::none());
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
        for (e, kind, event) in every_event() {
            let before = live(&d).len();
            a.device_event(&e);
            let new = live(&d)[before..].to_vec();
            let r = new.first().unwrap_or_else(|| panic!("{e:?} wrote no record"));
            assert_eq!(r["kind"].as_str(), Some(kind), "{e:?}: {r}");
            if let Some(ev) = event {
                assert_eq!(r["detail"]["event"].as_str(), Some(ev), "{e:?}: {r}");
            }
            if let Some(s) = serde_json::to_value(&e).unwrap().get("subject").filter(|s| !s.is_null()) {
                assert_eq!(&r["detail"]["subject"], s, "{e:?}: the subject: {r}");
            }
            if kind == "pairing" {
                assert_eq!(r["detail"]["for_principal"], json!("owner"), "{e:?}: {r}");
            }
            assert_eq!(new.iter().any(|l| l["kind"] == "alert"), e.alerts(), "{e:?}: its alert: {new:?}");
        }
        let l = live(&d);
        let rec = |ev: &str| l.iter().find(|r| r["detail"]["event"] == json!(ev)).cloned().unwrap();
        assert_eq!((rec("revoked")["principal"].as_str(), rec("revoked")["device"].as_str()), (Some("owner"), Some("d_peer")), "a device acted");
        assert_eq!((rec("disabled")["principal"].clone(), rec("disabled")["detail"]["by"].clone()), (Value::Null, json!({ "kind": "serve" })));
        assert_eq!((rec("enrolled")["device"].as_str(), rec("enrolled")["detail"]["fields"]["spki_sha256"].clone()), (Some("loopback"), json!("a".repeat(64))));
        assert_eq!((rec("attempt")["peer"].as_str(), rec("attempt")["principal"].clone()), (Some("100.64.0.9"), Value::Null), "before any authentication");
        assert_eq!(rec("closed")["detail"]["subject"], json!({ "principal": "owner", "device": "d_new" }));
        let knock = l.iter().find(|r| r["kind"] == "auth").unwrap();
        assert_eq!((knock["detail"]["reason"].as_str(), knock["detail"]["subject"]["device"].as_str()), (Some("revoked"), Some("d_new")), "{knock}");
        let emitted = host.0.lock().unwrap();
        assert!(emitted.iter().all(|(n, _)| n == "audit:alert"), "{emitted:?}");
        assert_eq!(emitted.len(), every_event().iter().filter(|(e, ..)| e.alerts()).count(), "{emitted:?}");
        assert_eq!(verify(&d).broken, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The comparison code is for the host's dialog, never a record — and no
    /// pairing record can name the link or its secret, which no event holds.
    #[test]
    fn the_comparison_code_never_reaches_the_log() {
        let d = dir("devcode");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        for (e, ..) in every_event() {
            a.device_event(&e);
        }
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert!(!text.contains(CODE), "{text}");
        assert!(text.contains("\"verified\""), "the attempt that carried it is recorded: {text}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// §5.1's checkpoint carries every unrevoked device's security fields —
    /// at genesis and at every rotation — read from the registry as it
    /// stands, so §5.2 never needs a file rotation removed.
    #[test]
    fn the_checkpoint_carries_every_unrevoked_device() {
        let d = dir("devcheck");
        let reg = Arc::new(DeviceRegistry::in_memory());
        let enroll = |pin: char, name: &str| {
            let e = Enrollment {
                principal: PrincipalId::owner(),
                name: name.into(),
                platform: Platform::Macos,
                spki_sha256: pin.to_string().repeat(64),
                stepup_spki_sha256: None,
                grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All },
                expiry: Expiry::default(),
                addr: None,
            };
            reg.enroll(e, now()).unwrap()
        };
        let kept = enroll('c', "Mac");
        let gone = enroll('d', "Old Mac");
        reg.revoke(&Caller::console(), &Grant::everything(), &gone.id, now()).unwrap();
        let (a, _) = Audit::open_with(&d, None, Fleet::of(&reg), 900);
        let c = console(&a);
        for i in 0..20 {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
        }
        let want = json!([kept.security()]);
        let mut seen = 0;
        for n in [FILE.to_string(), rotated_name(1), rotated_name(2)] {
            let first = &file_lines(&d.join(&n))[0];
            assert_eq!(first["kind"].as_str(), Some("checkpoint"), "{n}");
            assert_eq!(first["detail"]["devices"], want, "{n}: {first}");
            seen += 1;
        }
        assert_eq!(seen, 3);
        assert_eq!(verify(&d).broken, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The writer's lock order: an event the registry fires while serve
    /// rotates the log reads the registry from under the log's lock. A
    /// registry change made by a THIRD thread at the same time must not
    /// deadlock with either.
    #[test]
    fn registry_events_and_rotation_checkpoints_do_not_deadlock() {
        let d = dir("devlock");
        let reg = Arc::new(DeviceRegistry::in_memory());
        let (a, _) = Audit::open_with(&d, None, Fleet::of(&reg), 2000);
        a.hear(&reg);
        let (tx, rx) = std::sync::mpsc::channel();
        let (r2, a2) = (reg.clone(), a.clone());
        std::thread::spawn(move || {
            for i in 0..50u8 {
                let e = Enrollment {
                    principal: PrincipalId::owner(),
                    name: format!("Mac {i}"),
                    platform: Platform::Macos,
                    spki_sha256: format!("{i:02x}").repeat(32),
                    stepup_spki_sha256: None,
                    grant: Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All },
                    expiry: Expiry::default(),
                    addr: None,
                };
                if let Ok(rec) = r2.enroll(e, now()) {
                    let _ = r2.revoke(&Caller::console(), &Grant::everything(), &rec.id, now());
                }
                call(&console(&a2), "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
            }
            tx.send(()).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(30)).expect("registry events and checkpoints deadlocked");
        assert_eq!(verify(&d).broken, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The `budget` records of `d`'s live file and the rotated ones.
    fn budget_lines(d: &Path) -> Vec<Value> {
        let mut all = Vec::new();
        for n in (1..KEEP).rev().map(rotated_name).chain([FILE.to_string()]) {
            if d.join(&n).exists() {
                all.extend(file_lines(&d.join(n)));
            }
        }
        all.into_iter().filter(|l| l["kind"] == "budget").collect()
    }

    /// A device's records written one by one: everything but the budget
    /// episodes, the checkpoints and the console's.
    fn written_by_phone(d: &Path, kind: &str) -> usize {
        live(d).iter().filter(|l| l["device"] == json!("d_phone") && l["kind"] == json!(kind)).count()
    }

    /// The budget's rule, end to end: a device at full rate gets its burst
    /// written one by one, and every record past it COUNTED into one episode
    /// — a record when it begins and one when it ends, with the count and
    /// what the records were — so nothing is dropped, and the chain verifies
    /// after the burst.
    #[test]
    fn a_device_past_its_budget_is_coalesced_and_nothing_is_dropped() {
        let d = dir("budget");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = phone(&a, 9);
        const N: usize = 2000;
        let t0 = Instant::now();
        for i in 0..N {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
        }
        let written = written_by_phone(&d, "call");
        assert!(written > 100 && written < N, "the burst is written, the rest is not: {written}");
        let bytes: usize = raw_lines(&d.join(FILE)).iter().filter(|l| l.contains("\"set_note\"")).map(|l| l.len() + 1).sum();
        // The burst, and what refilled while the loop ran.
        let refilled = ACTS.per_sec * t0.elapsed().as_secs_f64() + 400.0;
        assert!(bytes as f64 <= ACTS.burst + refilled, "{bytes} bytes written past a {} burst", ACTS.burst);
        a.advance(Duration::from_secs(EPISODE_QUIET as u64));
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "later", "note": "x" }));
        let b = budget_lines(&d);
        assert_eq!(b.len(), 2, "one began, one ended: {b:#?}");
        assert_eq!((b[0]["detail"]["episode"].as_str(), b[0]["detail"]["class"].as_str()), (Some("began"), Some("acts")));
        assert_eq!((b[0]["principal"].as_str(), b[0]["device"].as_str()), (Some("owner"), Some("d_phone")), "{}", b[0]);
        assert_eq!(b[0]["detail"]["first"], json!({ "kind": "call", "cmd": "set_note", "outcome": "ok" }));
        let end = &b[1]["detail"];
        assert_eq!((end["episode"].as_str(), end["closed_by"].as_str()), (Some("ended"), Some("quiet")), "{end}");
        assert_eq!(end["count"].as_u64(), Some((N - written) as u64), "every record not written is counted: {end}");
        assert_eq!(end["tally"], json!({ "call set_note ok": N - written }), "{end}");
        assert!(end["bytes"].as_u64().unwrap() > 0);
        assert_eq!(verify(&d).broken, None, "the chain after a burst");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The four ways a device episode ends: `EPISODE_QUIET` with nothing
    /// counted, `EPISODE_SPAN` while it goes on (a count held in memory
    /// longer is a count a crash loses), the device's last connection going,
    /// and serve stopping. Whichever it is, its count lands.
    #[test]
    fn a_coalesced_episode_ends_quiet_spanned_disconnected_or_stopped() {
        let flood = |c: &ConnAudit| {
            for _ in 0..200 {
                c.unknown("nope");
            }
        };
        // Quiet: ended by the next record anyone writes, dated by its last.
        let d = dir("budget-quiet");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = phone(&a, 9);
        flood(&c);
        a.advance(Duration::from_secs(EPISODE_QUIET as u64 - 1));
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
        assert_eq!(budget_lines(&d).len(), 1, "not quiet long enough yet");
        a.advance(Duration::from_secs(1));
        call(&console(&a), "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
        let b = budget_lines(&d);
        assert_eq!((b.len(), b[1]["detail"]["closed_by"].as_str(), b[1]["detail"]["class"].as_str()), (2, Some("quiet"), Some("refusals")), "{b:#?}");
        assert_eq!(b[1]["detail"]["tally"], json!({ "refused unknown unknown command": b[1]["detail"]["count"] }));
        drop(c);
        let _ = std::fs::remove_dir_all(&d);

        // Span: a flood that never pauses — always past its refill — still
        // lands its count.
        let d = dir("budget-span");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = phone(&a, 9);
        flood(&c);
        for _ in 0..(EPISODE_SPAN.as_secs() / 30) {
            a.advance(Duration::from_secs(30));
            for _ in 0..5 {
                c.unknown("nope");
            }
        }
        let b = budget_lines(&d);
        let ended: Vec<&Value> = b.iter().filter(|r| r["detail"]["episode"] == "ended").collect();
        assert_eq!(ended.first().map(|r| r["detail"]["closed_by"].clone()), Some(json!("span")), "{b:#?}");
        drop(c);
        let _ = std::fs::remove_dir_all(&d);

        // Disconnect: at the device's LAST connection, not its first.
        let d = dir("budget-gone");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let (c1, c2) = (phone(&a, 9), phone(&a, 10));
        flood(&c1);
        drop(c1);
        assert_eq!(budget_lines(&d).len(), 1, "its other connection is still up");
        drop(c2);
        let b = budget_lines(&d);
        assert_eq!(b.last().map(|r| r["detail"]["closed_by"].clone()), Some(json!("disconnect")), "{b:#?}");
        let _ = std::fs::remove_dir_all(&d);

        // Stopped: the listener's stop ends them before its own record.
        let d = dir("budget-stop");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = phone(&a, 9);
        flood(&c);
        a.stopped();
        let l = live(&d);
        assert_eq!(l.last().unwrap()["detail"]["event"], json!("stopped"));
        assert_eq!(l[l.len() - 2]["detail"]["closed_by"], json!("stopped"), "{l:#?}");
        drop(c);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// What the budget never coalesces, whoever acted: a device over both of
    /// its budgets still has every registry and pairing record written one
    /// by one — the evidence §5.2 reads — and so is the first refused key of
    /// an auth episode.
    #[test]
    fn security_records_are_written_whatever_the_budget() {
        let d = dir("budget-sec");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = phone(&a, 9);
        for i in 0..1000 {
            c.unknown("nope");
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
        }
        assert_eq!(budget_lines(&d).len(), 2, "both budgets are spent: {:#?}", budget_lines(&d));
        let me = Actor::Device { principal: PrincipalId::owner(), device: DeviceId::new("d_phone") };
        let grant = Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All };
        let before = live(&d).len();
        a.device_event(&DeviceEvent::ScopeChanged {
            subject: owner_subject("d_peer"), by: me.clone(), from: grant, to: Grant::nothing(), widened: false, reduced: true,
        });
        a.device_event(&DeviceEvent::Revoked { subject: owner_subject("d_phone"), by: me });
        a.device_event(&DeviceEvent::Disabled { subject: owner_subject("d_other"), reason: DisabledReason::Idle, by: Actor::Serve });
        a.device_event(&DeviceEvent::PairingDecided { allow: false });
        a.device_event(&DeviceEvent::Enrolled { subject: owner_subject("d_new"), name: "Mac".into(), platform: Platform::Macos, fields: fields("d_new") });
        a.device_event(&DeviceEvent::IdentityRotated { pin: "b".repeat(64) });
        a.device_event(&DeviceEvent::ConnectRefused { subject: None, spki_sha256: "e".repeat(64), why: ConnectRefusal::Unknown });
        let new = live(&d)[before..].to_vec();
        let events: Vec<&str> = new.iter().filter(|r| r["kind"] != "alert").filter_map(|r| r["detail"]["event"].as_str()).collect();
        assert_eq!(events, vec!["scope_changed", "revoked", "disabled", "decided", "enrolled", "identity_rotated"], "{new:#?}");
        assert!(new.iter().any(|r| r["kind"] == "auth" && r["detail"]["episode"] == "began"), "{new:#?}");
        let revoked = new.iter().find(|r| r["detail"]["event"] == "revoked").unwrap();
        assert_eq!(revoked["device"].as_str(), Some("d_phone"), "the over-budget device is its actor: {revoked}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Acts and refusals spend separate budgets: junk a device can make
    /// with nothing granted does not cost what its grant did its detail. And
    /// a device back under its budget is written one by one again while its
    /// episode is still open (trickle-through).
    #[test]
    fn refusals_cannot_spend_the_acts_budget_and_a_refill_writes_again() {
        let d = dir("budget-split");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = phone(&a, 9);
        for _ in 0..1000 {
            c.unknown("nope");
        }
        call(&c, "set_note", json!({ "repo": "/r", "slug": "after-junk", "note": "x" }));
        assert_eq!(live(&d).last().unwrap()["object"].as_str(), Some("after-junk"), "an act after a flood of junk is written");
        // Acts past their own budget, then half a quiet gap's refill: the
        // next act is written one by one, inside the episode still open.
        for i in 0..1000 {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
        }
        let acts = written_by_phone(&d, "call");
        a.advance(Duration::from_secs(EPISODE_QUIET as u64 / 2));
        call(&c, "set_note", json!({ "repo": "/r", "slug": "refilled", "note": "x" }));
        assert_eq!(written_by_phone(&d, "call"), acts + 1, "refilled: written one by one again");
        let open: Vec<Value> = budget_lines(&d).into_iter().filter(|r| r["detail"]["class"] == "acts").collect();
        assert_eq!(open.len(), 1, "and the acts episode is still the one that began: {open:#?}");
        drop(c);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The console is the owner at the host: never budgeted, whatever it
    /// does (a same-user process can rewrite the file anyway, §1).
    #[test]
    fn the_console_is_never_budgeted() {
        let d = dir("budget-console");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = console(&a);
        for i in 0..500 {
            call(&c, "set_note", json!({ "repo": "/r", "slug": format!("s{i}"), "note": "x" }));
            c.unknown("nope");
        }
        assert!(budget_lines(&d).is_empty());
        assert_eq!(live(&d).iter().filter(|l| l["device"] == json!("loopback")).count(), 1000);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The ceiling the budget promises: a device flooding both budgets flat
    /// out for an hour writes about what `ACTS` and `REFUSALS` refill plus
    /// their episode records — kilobytes, where 50 records a second would
    /// have been tens of megabytes.
    #[test]
    fn an_hour_of_flood_from_one_device_writes_its_ceiling() {
        let d = dir("budget-hour");
        let (a, _) = Audit::open(&d, None, Fleet::none());
        let c = phone(&a, 9);
        for _ in 0..3600 {
            a.advance(Duration::from_secs(1));
            for i in 0..50 {
                if i % 2 == 0 {
                    c.unknown("nope");
                } else {
                    call(&c, "set_note", json!({ "repo": "/r", "slug": "s", "note": "x" }));
                }
            }
        }
        drop(c);
        let bytes: u64 = (1..KEEP).map(rotated_name).chain([FILE.to_string()]).filter_map(|n| std::fs::metadata(d.join(n)).ok()).map(|m| m.len()).sum();
        let ceiling = ACTS.burst + REFUSALS.burst + 3600.0 * (ACTS.per_sec + REFUSALS.per_sec) + 8.0 * 1024.0;
        assert!((bytes as f64) < ceiling, "{bytes} bytes in an hour, ceiling {ceiling}");
        let counted: u64 = budget_lines(&d).iter().filter_map(|r| r["detail"]["count"].as_u64()).sum();
        let written = written_by_phone(&d, "refused") as u64 + written_by_phone(&d, "call") as u64;
        assert_eq!(counted + written, 3600 * 50, "every record is written or counted");
        assert_eq!(verify(&d).broken, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Refused device keys are auth failures (§5.1): one episode with the
    /// failed handshakes, its first a record naming the key, the rest
    /// counted — and a known device's knock alerts once per episode for
    /// each reason, not once per knock (§5.3).
    #[test]
    fn refused_keys_join_the_auth_episode_and_alert_once_per_device() {
        let d = dir("knock");
        let host = Arc::new(Catch::default());
        let (a, _) = Audit::open(&d, Some(host.clone()), Fleet::none());
        let knock = |subject: Option<Subject>, why| {
            a.device_event(&DeviceEvent::ConnectRefused { subject, spki_sha256: "f".repeat(64), why });
        };
        for _ in 0..5 {
            knock(None, ConnectRefusal::Unknown);
            knock(Some(owner_subject("d_lost")), ConnectRefusal::Revoked);
            a.auth_failed("handshake", 1, Some("127.0.0.1".into()));
        }
        knock(Some(owner_subject("d_lost")), ConnectRefusal::Disabled);
        a.stopped();
        let l = live(&d);
        let auth: Vec<&Value> = l.iter().filter(|r| r["kind"] == "auth").collect();
        assert_eq!(auth.len(), 2, "one began, one ended: {auth:#?}");
        assert_eq!((auth[0]["detail"]["reason"].as_str(), auth[0]["detail"]["key"].clone()), (Some("unknown-key"), json!("f".repeat(64))));
        assert_eq!(auth[1]["detail"]["count"].as_u64(), Some(16));
        assert_eq!(auth[1]["detail"]["reasons"], json!({ "disabled": 1, "handshake": 5, "revoked": 5, "unknown-key": 5 }));
        let alerts: Vec<&Value> = l.iter().filter(|r| r["kind"] == "alert").collect();
        assert_eq!(alerts.len(), 2, "revoked once, disabled once: {alerts:#?}");
        assert_eq!(alerts[0]["detail"], json!({ "alert": "device", "event": "connect_refused", "reason": "revoked", "subject": { "principal": "owner", "device": "d_lost" } }));
        assert_eq!(host.0.lock().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }
}
