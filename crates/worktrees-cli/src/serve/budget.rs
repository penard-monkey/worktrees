//! What an AUTHENTICATED client may cost (Phase 2 §6, R6; gap 13).
//!
//! M1 (`conn.rs`) bounds a peer before it proves itself. This is the rest:
//! once in, a client could start a blocking task per request, grow its
//! outbox without limit and park a lane per stream key. Measured against
//! 72e7339, one loopback client did each of these in under 20 s:
//!
//! - 1,000 `list_places` in one burst held 615 threads (771 on Linux), and
//!   the desk's own `get_settings` waited 10.8 s behind them;
//! - `yes` in a dock shell, with a client that stopped reading, took serve
//!   to 2.5 GB in 10 s (1.3 GB on Linux);
//! - `term_write` to 200,000 distinct ids took it to 340 MB.
//!
//! Two kinds of caller pay. The CONSOLE is the loopback listener's client:
//! the desktop app, a dev page, whoever holds serve.json's token. A DEVICE
//! is a paired client of the remote listener (slice 6). Per-connection
//! budgets bind both. Per-device budgets bind devices, and remote bodies
//! share a host-wide budget of their own, so devices can never take the
//! threads the desk needs.
//!
//! A refusal is an `err` frame sent at once and never queued: `busy` (a slot
//! is taken; ask again once something has come back), `rate` (too much,
//! too fast; slow down), or, for a device's agent starts, `start-budget: <s>`
//! (the seconds until it may start one again: a person is told when, not
//! "slow down"). Each limit tripped is an EPISODE ([`Trips`]): one
//! serve.log line and one audit record (§5.1) as it begins, one with the
//! count as it ends, so a flood does not become a disk-writing flood.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use worktrees_api::log::info;
use worktrees_core::authz::{Caller, Device, DeviceId, Listener};

use super::audit::{ConnAudit, EPISODE_QUIET};

use super::dispatch::{self, Args, Exposure};
use super::provenance::LAUNCH_ROWS;

/// `err` for a request refused because a slot it needs is taken.
pub const BUSY: &str = "busy";
/// `err` for a request refused because its caller is over a rate.
pub const RATE: &str = "rate";
/// The word a device past its agent-start bucket is answered with, as
/// `start-budget: <seconds>` (`Account::admit`), and the one its episode
/// records: the start limit, told apart from the request rate.
pub const START_BUDGET: &str = "start-budget";

// ── counts ───────────────────────────────────────────────────────────────────

/// A count with a cap. `try_hold` takes one or says no; the `Held` gives it
/// back on drop, whichever way its holder ends: a body that returns, one
/// that panics, a job dropped unrun, a connection that goes.
pub struct Counter {
    held: AtomicUsize,
    pub cap: usize,
    peak: AtomicUsize,
}

pub struct Held(Arc<Counter>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.held.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Counter {
    pub fn new(cap: usize) -> Arc<Self> {
        Arc::new(Counter { held: AtomicUsize::new(0), cap, peak: AtomicUsize::new(0) })
    }

    pub fn try_hold(self: &Arc<Self>) -> Option<Held> {
        let was = self.held.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < self.cap).then_some(n + 1)).ok()?;
        self.peak.fetch_max(was + 1, Ordering::SeqCst);
        Some(Held(self.clone()))
    }

    pub fn held(&self) -> usize {
        self.held.load(Ordering::SeqCst)
    }

    /// The most ever held at once: what the caps were sized against.
    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

// ── the numbers ──────────────────────────────────────────────────────────────

/// A token bucket: `burst` at once, refilled at `per_sec`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub per_sec: f64,
    pub burst: f64,
}

/// What ONE connection may hold, on either listener.
#[derive(Clone, Copy, Debug)]
pub struct PerConnection {
    /// Requests sent and not yet answered: running, queued on a lane, or
    /// waiting on a shared read. Past it, `busy`.
    pub in_flight: usize,
    /// Bytes queued for the socket and not yet written. A reply or an event
    /// past it closes the connection: the client reconnects and asks again.
    pub outbox: usize,
    /// The share of `outbox` a byte stream may fill. Past it the stream's
    /// channel ends (`{end: true}`) and the client attaches again; tmux
    /// repaints. Smaller than `outbox` by more than the largest reply, so a
    /// busy terminal never closes a connection that is also reading a file.
    pub outbox_stream: usize,
    /// Streams with work queued or running at once. An idle lane is gone.
    pub lanes: usize,
}

/// What ONE paired device may hold, across all of its connections. The
/// console has none of these: it is the person at the desk.
#[derive(Clone, Copy, Debug)]
pub struct PerDevice {
    pub connections: usize,
    pub in_flight: usize,
    /// Non-stream requests (`Exposure` other than `Stream`).
    pub requests: Rate,
    /// Bytes written into one attach or dock shell (`term_write`,
    /// `shell_write`). A write past it is dropped whole, answered `rate`.
    pub input: Rate,
    pub attaches: usize,
    pub shells: usize,
    /// Agent launches (LLM10): a stolen or buggy device must not burn the
    /// user's plan. The quota gate (`set_quota_settings`) applies on top.
    pub launches: Rate,
}

#[derive(Clone, Copy, Debug)]
pub struct Budgets {
    pub loopback: PerConnection,
    pub remote: PerConnection,
    pub device: PerDevice,
    /// Remote request bodies in flight host-wide, every device together.
    pub remote_bodies: usize,
}

const MIB: usize = 1 << 20;

/// The numbers (§6's table, measured where it said "to be measured"):
///
/// - in-flight: 32 per remote connection and 48 per device, NOT §6's 16 and
///   32. The app's own frontend, driven against serve (load, the rail
///   panes, five places), peaked at 29 requests unanswered as the page saw
///   them (17 admitted at once as serve counted them), all in its startup
///   burst. 16 would have answered `busy` to a phone running that same
///   frontend before its first screen. 48 lets one device open two such
///   sockets at once and still leaves a second device room in the 64
///   remote bodies.
/// - remote bodies: 64. Tokio's blocking pool is 512 threads, so the desk
///   keeps 448 whatever every device does.
/// - outbox: 8 MiB, streams 2 MiB of it. The largest reply the frontend can
///   ask for is `read_file`'s 1 MB at 5.72 MiB of JSON (all control
///   characters); 5.72 + 2 < 8.
/// - lanes: 64 active. Idle lanes are reaped, so this counts streams
///   with keystrokes in flight right now, not streams ever named.
/// - requests: 50/s, burst 200. The same drive's busiest second was 41
///   requests, and it sent 74 in 49 s. Stream input: 4 MiB/s, burst 4 MiB,
///   which is one maximal paste (a 16 MiB message holds about 4 MB of pasted
///   ASCII).
/// - attaches 8 and shells 8 per device; launches 10 an hour, burst 10.
pub const BUDGETS: Budgets = Budgets {
    loopback: LOOPBACK,
    remote: PerConnection { in_flight: 32, outbox: 8 * MIB, outbox_stream: 2 * MIB, lanes: 64 },
    device: PerDevice {
        connections: 4,
        in_flight: 48,
        requests: Rate { per_sec: 50.0, burst: 200.0 },
        input: Rate { per_sec: (4 * MIB) as f64, burst: (4 * MIB) as f64 },
        attaches: 8,
        shells: 8,
        launches: Rate { per_sec: 10.0 / 3600.0, burst: 10.0 },
    },
    remote_bodies: 64,
};

/// The console's per-connection budget. The outbox and lanes are the
/// remote's: they bound memory, and the desk's own use sits far below them
/// (the drive above peaked at 217 KiB queued, a changelog, and no lane).
/// In-flight is 64, twice the frontend's measured 29, because the desktop app
/// is ONE socket carrying every pane at once.
pub const LOOPBACK: PerConnection = PerConnection { in_flight: 64, outbox: 8 * MIB, outbox_stream: 2 * MIB, lanes: 64 };

// ── buckets ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn full(rate: Rate, now: Instant) -> Self {
        Bucket { tokens: rate.burst, at: now }
    }

    /// Refill for the time since the last call, then take `n` if it is
    /// there. The clock is a parameter so the rule is tested on stamps, not
    /// on a race between two sleeps.
    fn take(&mut self, rate: Rate, n: f64, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + dt * rate.per_sec).min(rate.burst);
        self.at = now;
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }

    /// Whole seconds from `now` until `n` tokens are there, at least one:
    /// what a refusal says to wait. A hair under a whole second is that
    /// second, not the next, so float error never adds one.
    fn wait(&self, rate: Rate, n: f64, now: Instant) -> u64 {
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        let have = (self.tokens + dt * rate.per_sec).min(rate.burst);
        (((n - have) / rate.per_sec) - 1e-6).ceil().max(1.0) as u64
    }

    /// Back at its burst: as good as no entry at all.
    fn is_full(&self, rate: Rate, now: Instant) -> bool {
        self.tokens + now.saturating_duration_since(self.at).as_secs_f64() * rate.per_sec >= rate.burst
    }
}

/// Input buckets per stream key, kept only while they carry information: a
/// bucket back at its burst is dropped, so the map holds just the streams
/// written to in the last second. Past `MAX_INPUT_KEYS` live buckets, a new
/// key is `busy` until some refill.
const MAX_INPUT_KEYS: usize = 256;

// ── the host's budgets ───────────────────────────────────────────────────────

/// One device's share, across all of its connections, and the episodes of
/// the device's own limits, which span its connections too.
struct Share {
    trips: Trips,
    connections: Arc<Counter>,
    in_flight: Arc<Counter>,
    attaches: Arc<Counter>,
    shells: Arc<Counter>,
    requests: Mutex<Bucket>,
    launches: Mutex<Bucket>,
    inputs: Mutex<HashMap<String, Bucket>>,
}

/// One per server: the numbers, the remote body budget, and every device's
/// share. Devices are never forgotten while serve runs; there are at most as
/// many as are enrolled (16, §6).
pub struct Budget {
    pub budgets: Budgets,
    remote_bodies: Arc<Counter>,
    devices: Mutex<HashMap<DeviceId, Arc<Share>>>,
}

impl Budget {
    pub fn new(budgets: Budgets) -> Self {
        Budget { budgets, remote_bodies: Counter::new(budgets.remote_bodies), devices: Mutex::new(HashMap::new()) }
    }

    fn share(&self, id: &DeviceId) -> Arc<Share> {
        let d = self.budgets.device;
        let now = Instant::now();
        self.devices
            .lock()
            .unwrap()
            .entry(id.clone())
            .or_insert_with(|| {
                Arc::new(Share {
                    trips: Trips::new(format!("device {id}")),
                    connections: Counter::new(d.connections),
                    in_flight: Counter::new(d.in_flight),
                    attaches: Counter::new(d.attaches),
                    shells: Counter::new(d.shells),
                    requests: Mutex::new(Bucket::full(d.requests, now)),
                    launches: Mutex::new(Bucket::full(d.launches, now)),
                    inputs: Mutex::new(HashMap::new()),
                })
            })
            .clone()
    }

    /// The account a newly authenticated connection spends from, or `None`
    /// when its device already holds every connection it may — a limit
    /// tripped, which `au` records once per episode. `au` is the
    /// connection's audit, which every trip of its account is recorded
    /// through; `None` only in the unit tests below.
    ///
    /// Keyed on the connection's `Caller`, fixed by the listener that
    /// accepted it (§3.3): its LISTENER picks the per-connection budget, and
    /// a paired DEVICE spends its own share, across its connections, and the
    /// remote body budget every device shares. The console has no share: it
    /// is the person at the desk. The caller names who pays, never what they
    /// may do — that is the resolver's.
    pub fn open(&self, conn: u64, caller: &Caller, au: Option<Arc<ConnAudit>>) -> Option<Account> {
        let limits = match caller.listener() {
            Listener::Loopback => self.budgets.loopback,
            Listener::Remote => self.budgets.remote,
        };
        let (device, slot, bodies) = match caller.device() {
            Device::Console => (None, None, None),
            Device::Paired(id) => {
                let d = self.share(id);
                let now = Instant::now();
                d.trips.sweep(au.as_deref(), now, false);
                let Some(slot) = d.connections.try_hold() else {
                    d.trips.trip(au.as_deref(), "device_connections", "closed", now);
                    return None;
                };
                (Some(d), Some(slot), Some(self.remote_bodies.clone()))
            }
        };
        Some(Account {
            born: Instant::now(),
            pace: Pace::default(),
            limits,
            per_device: self.budgets.device,
            in_flight: Counter::new(limits.in_flight),
            device,
            bodies,
            trips: Trips::new(format!("client {conn}")),
            au,
            _connection: slot,
        })
    }
}

// ── one connection's account ─────────────────────────────────────────────────

/// What one connection has to spend, and what it holds right now.
pub struct Account {
    born: Instant,
    pace: Pace,
    pub limits: PerConnection,
    per_device: PerDevice,
    in_flight: Arc<Counter>,
    device: Option<Arc<Share>>,
    bodies: Option<Arc<Counter>>,
    /// The episodes of this connection's own limits (in-flight, lanes); a
    /// device's are its `Share`'s.
    trips: Trips,
    au: Option<Arc<ConnAudit>>,
    _connection: Option<Held>,
}

/// One admitted request's slots. Dropped when its reply is queued, when its
/// body panics, or when its job is dropped unrun: a `Held` cannot leak.
pub struct Ticket {
    _in_flight: Held,
    _device: Option<Held>,
    _body: Option<Held>,
    /// The attach or shell slot of `term_open` / `shell_open`: handed to the
    /// sink the body creates (`Frames::bytes`), so it is held for as long as
    /// the stream lives, not just the request.
    pub attach: Option<Held>,
}

/// The bytes a write row carries: `data` as the frontend sends it, a JSON
/// array of bytes, or a string from any other client.
fn input_len(args: &Args) -> usize {
    match args.0.get("data") {
        Some(Value::Array(a)) => a.len(),
        Some(Value::String(s)) => s.len(),
        _ => 0,
    }
}

impl Account {
    /// Admit one request, or say why not: the `err` to answer it with.
    /// Slots are taken before tokens, so a request refused `busy` costs its
    /// caller none of its rate.
    pub fn admit(&self, cmd: &str, args: &Args, lane: Option<&str>) -> Result<Ticket, Value> {
        let now = Instant::now();
        self.admit_at(cmd, args, lane, now).map_err(|e| self.answer(e, now))
    }

    /// A refusal's word as the client is told it: [`START_BUDGET`] with the
    /// seconds until the device's start bucket holds one again, every other
    /// word as it is.
    fn answer(&self, err: &'static str, now: Instant) -> Value {
        match (err, &self.device) {
            (START_BUDGET, Some(d)) => Value::from(format!("{START_BUDGET}: {}", d.launches.lock().unwrap().wait(self.per_device.launches, 1.0, now))),
            _ => Value::from(err),
        }
    }

    fn admit_at(&self, cmd: &str, args: &Args, lane: Option<&str>, now: Instant) -> Result<Ticket, &'static str> {
        self.pace.tick(now.saturating_duration_since(self.born).as_secs());
        self.sweep(now);
        let in_flight = self.check(&self.trips, "in_flight", BUSY, self.in_flight.try_hold(), now)?;
        let Some(d) = &self.device else {
            return Ok(Ticket { _in_flight: in_flight, _device: None, _body: None, attach: None });
        };
        let t = &d.trips;
        let device = self.check(t, "device_in_flight", BUSY, d.in_flight.try_hold(), now)?;
        let body = Some(self.check(t, "remote_bodies", BUSY, self.bodies.as_ref().and_then(|b| b.try_hold()), now)?);
        let attach = match cmd {
            "term_open" => Some(self.check(t, "attaches", BUSY, d.attaches.try_hold(), now)?),
            "shell_open" => Some(self.check(t, "shells", BUSY, d.shells.try_hold(), now)?),
            _ => None,
        };
        let p = self.per_device;
        if dispatch::exposure(cmd) != Some(Exposure::Stream) {
            let ok = d.requests.lock().unwrap().take(p.requests, 1.0, now);
            self.check(t, "request_rate", RATE, ok.then_some(()), now)?;
        }
        // The rows that start an agent (LLM10), the lane gate's own list.
        if LAUNCH_ROWS.contains(&cmd) {
            let ok = d.launches.lock().unwrap().take(p.launches, 1.0, now);
            self.check(t, "launch_rate", START_BUDGET, ok.then_some(()), now)?;
        }
        if matches!(cmd, "term_write" | "shell_write") {
            let ok = take_input(&d.inputs, lane.unwrap_or(cmd), p.input, input_len(args), now);
            self.check(t, "input_rate", ok.err().unwrap_or(RATE), ok.ok(), now)?;
        }
        Ok(Ticket { _in_flight: in_flight, _device: Some(device), _body: body, attach })
    }

    /// `v`, or the refusal `err`, counted into `limit`'s episode in `trips`.
    fn check<T>(&self, trips: &Trips, limit: &'static str, err: &'static str, v: Option<T>, now: Instant) -> Result<T, &'static str> {
        v.ok_or_else(|| {
            trips.trip(self.au.as_deref(), limit, err, now);
            err
        })
    }

    /// End the episodes that have gone quiet, this connection's and its
    /// device's.
    fn sweep(&self, now: Instant) {
        self.trips.sweep(self.au.as_deref(), now, false);
        if let Some(d) = &self.device {
            d.trips.sweep(self.au.as_deref(), now, false);
        }
    }

    /// A refusal decided outside `admit` (a lane past its cap), counted the
    /// same way.
    pub fn refuse(&self, limit: &'static str, err: &'static str) -> &'static str {
        self.trips.trip(self.au.as_deref(), limit, err, Instant::now());
        err
    }

    /// One request token, from a device, for a frame that is admitted to
    /// nothing: a request the resolver refused before admission
    /// (`dispatch::precheck`), which took no slot, or a listen frame. A
    /// device's refusals and declarations are rate-limited like any request,
    /// and so are the serve.log lines and audit records each can make.
    /// `Err(RATE)` when it has none left. The console pays nothing.
    pub fn charge(&self) -> Result<(), &'static str> {
        self.charge_at(Instant::now())
    }

    fn charge_at(&self, now: Instant) -> Result<(), &'static str> {
        self.pace.tick(now.saturating_duration_since(self.born).as_secs());
        let Some(d) = &self.device else { return Ok(()) };
        d.trips.sweep(self.au.as_deref(), now, false);
        let ok = d.requests.lock().unwrap().take(self.per_device.requests, 1.0, now);
        self.check(&d.trips, "request_rate", RATE, ok.then_some(()), now)
    }

    /// The connection is going: every episode of its own ends, and so do
    /// its device's when it is the device's last connection. What ended, as
    /// `limit: n refused`, for the disconnect line.
    pub fn close(&self) -> String {
        let mut ended = self.trips.sweep(self.au.as_deref(), Instant::now(), true);
        if let Some(d) = self.device.as_ref().filter(|d| d.connections.held() <= 1) {
            ended.extend(d.trips.sweep(self.au.as_deref(), Instant::now(), true));
        }
        ended.sort();
        ended.join(", ")
    }

    /// What this connection asked of its budgets, for the disconnect line:
    /// the measurement the numbers above are checked against.
    pub fn summary(&self) -> String {
        format!(
            "peak in-flight {}, {} request(s), busiest second {}",
            self.in_flight.peak(),
            self.pace.total.load(Ordering::SeqCst),
            self.pace.peak.load(Ordering::SeqCst)
        )
    }
}

/// Requests per wall second, roughly: the busiest second's count and the
/// total. Racy at a second's edge by a request or two, which is fine for a
/// number that is only ever logged.
#[derive(Default)]
struct Pace {
    second: AtomicU64,
    in_second: AtomicU64,
    peak: AtomicU64,
    total: AtomicU64,
}

impl Pace {
    fn tick(&self, second: u64) {
        self.total.fetch_add(1, Ordering::SeqCst);
        let n = if self.second.swap(second, Ordering::SeqCst) == second {
            self.in_second.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            self.in_second.store(1, Ordering::SeqCst);
            1
        };
        self.peak.fetch_max(n, Ordering::SeqCst);
    }
}

/// Take `n` bytes from `key`'s input bucket. `Err(BUSY)` when the map is at
/// its size and holds no bucket that has refilled.
fn take_input(inputs: &Mutex<HashMap<String, Bucket>>, key: &str, rate: Rate, n: usize, now: Instant) -> Result<(), &'static str> {
    let mut map = inputs.lock().unwrap();
    if !map.contains_key(key) && map.len() >= MAX_INPUT_KEYS {
        map.retain(|_, b| !b.is_full(rate, now));
        if map.len() >= MAX_INPUT_KEYS {
            return Err(BUSY);
        }
    }
    let b = map.entry(key.to_string()).or_insert_with(|| Bucket::full(rate, now));
    if b.take(rate, n as f64, now) {
        Ok(())
    } else {
        Err(RATE)
    }
}

// ── trips, once per episode ──────────────────────────────────────────────────

/// How long a limit goes untripped before its episode ends: the audit's own
/// quiet gap (`audit::EPISODE_QUIET`), so the two logs count alike.
const QUIET: Duration = Duration::from_secs(EPISODE_QUIET as u64);

/// The limits being tripped right now, each an episode: its first refusal is
/// a serve.log line and an audit record (§5.1, "a limit tripped"), the rest
/// are counted, and the episode ends — a line and a record with the count —
/// once its limit has gone [`QUIET`] without a refusal, or its holder goes.
///
/// NOT at the next request let through. A client sitting AT a cap is
/// admitted and refused by turns, one each per body that finishes, and an
/// episode that ended at each admission began again at each refusal: two
/// lines and two records per request, the disk-writing flood this exists to
/// stop.
pub struct Trips {
    /// Who trips them, for serve.log: `client <conn>` or `device <id>`.
    who: String,
    open: Mutex<HashMap<&'static str, Episode>>,
}

struct Episode {
    count: u64,
    /// Wall time, for the record; the monotonic stamps decide.
    began_epoch: i64,
    began: Instant,
    last: Instant,
}

impl Trips {
    fn new(who: String) -> Trips {
        Trips { who, open: Mutex::new(HashMap::new()) }
    }

    /// One refusal by `limit`, answered `err`.
    fn trip(&self, au: Option<&ConnAudit>, limit: &'static str, err: &'static str, now: Instant) {
        let mut open = self.open.lock().unwrap();
        if let Some(e) = open.get_mut(limit) {
            if now.saturating_duration_since(e.last) < QUIET {
                e.count += 1;
                e.last = now;
                return;
            }
            let e = open.remove(limit).expect("just found");
            self.ended(au, limit, &e);
        }
        info(&format!("serve: {}: {limit} reached; answering {err:?} until it clears", self.who));
        if let Some(au) = au {
            au.limit_began(limit, err);
        }
        let began_epoch = worktrees_core::sysclock::now_epoch();
        open.insert(limit, Episode { count: 1, began_epoch, began: now, last: now });
    }

    /// End every episode that has gone quiet — or every one, `all` — and
    /// say which, as `limit: n refused`.
    fn sweep(&self, au: Option<&ConnAudit>, now: Instant, all: bool) -> Vec<String> {
        let mut open = self.open.lock().unwrap();
        let done: Vec<&'static str> =
            open.iter().filter(|(_, e)| all || now.saturating_duration_since(e.last) >= QUIET).map(|(l, _)| *l).collect();
        done.into_iter()
            .map(|limit| {
                let e = open.remove(limit).expect("just listed");
                self.ended(au, limit, &e);
                format!("{limit}: {} refused", e.count)
            })
            .collect()
    }

    fn ended(&self, au: Option<&ConnAudit>, limit: &'static str, e: &Episode) {
        info(&format!("serve: {}: {limit} cleared; {} request(s) refused meanwhile", self.who, e.count));
        if let Some(au) = au {
            au.limit_ended(limit, e.count, e.began_epoch, e.last.saturating_duration_since(e.began).as_secs());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use worktrees_core::authz::PrincipalId;

    fn args(v: Value) -> Args {
        Args(v.as_object().cloned().unwrap_or_default())
    }

    fn paired(id: &str) -> Caller {
        Caller::paired(PrincipalId::owner(), DeviceId::new(id))
    }

    fn device(b: &Budget, id: &str) -> Account {
        b.open(1, &paired(id), None).expect("a connection slot")
    }

    /// The bucket on stamps: the burst at once, then the rate, never past
    /// the burst however long it sat.
    #[test]
    fn a_bucket_gives_its_burst_then_its_rate() {
        let r = Rate { per_sec: 2.0, burst: 3.0 };
        let t0 = Instant::now();
        let mut b = Bucket::full(r, t0);
        assert!((0..3).all(|_| b.take(r, 1.0, t0)), "the burst at once");
        assert!(!b.take(r, 1.0, t0), "then nothing");
        assert!(b.take(r, 1.0, t0 + Duration::from_millis(500)), "one token in half a second at 2/s");
        assert!(!b.take(r, 1.0, t0 + Duration::from_millis(500)));
        assert!(b.is_full(r, t0 + Duration::from_secs(60)), "an hour of rest is still only the burst");
        let mut b2 = b;
        assert!((0..3).all(|_| b2.take(r, 1.0, t0 + Duration::from_secs(60))));
        assert!(!b2.take(r, 1.0, t0 + Duration::from_secs(60)));
    }

    /// The console pays per connection only: no device caps, no rates,
    /// whatever it sends.
    #[test]
    fn the_console_has_no_rate_and_no_device_caps() {
        let b = Budget::new(BUDGETS);
        let a = b.open(1, &Caller::console(), None).unwrap();
        let t0 = Instant::now();
        let mut held = Vec::new();
        for _ in 0..LOOPBACK.in_flight {
            held.push(a.admit_at("get_settings", &args(json!({})), None, t0).expect("under the console's in-flight cap"));
        }
        assert_eq!(a.admit_at("get_settings", &args(json!({})), None, t0).err(), Some(BUSY), "past its in-flight cap");
        drop(held);
        for _ in 0..1000 {
            assert!(a.admit_at("new_place", &args(json!({})), None, t0).is_ok(), "no rate on the console");
        }
        for _ in 0..20 {
            let t = a.admit_at("term_open", &args(json!({})), None, t0).unwrap();
            assert!(t.attach.is_none(), "no attach cap on the console");
        }
    }

    /// A device's requests: its burst, then `rate`; streams are exempt from
    /// the request rate and pay by the byte instead.
    #[test]
    fn a_device_is_rate_limited_and_its_streams_pay_by_the_byte() {
        let mut budgets = BUDGETS;
        budgets.device.requests = Rate { per_sec: 1.0, burst: 5.0 };
        budgets.device.input = Rate { per_sec: 10.0, burst: 10.0 };
        let b = Budget::new(budgets);
        let a = device(&b, "phone");
        let t0 = Instant::now();
        let ok = (0..20).filter(|_| a.admit_at("get_settings", &args(json!({})), None, t0).is_ok()).count();
        assert_eq!(ok, 5, "the burst, then rate");
        assert_eq!(a.admit_at("get_settings", &args(json!({})), None, t0).err(), Some(RATE));
        assert!(a.admit_at("get_settings", &args(json!({})), None, t0 + Duration::from_secs(1)).is_ok(), "a second refills one");
        // Streams: not the request rate, the byte rate, per stream.
        let w = |n: usize| args(json!({ "id": 4, "data": vec![97u8; n] }));
        assert!(a.admit_at("term_write", &w(6), Some("term:4"), t0).is_ok(), "a stream is not request-rated");
        assert_eq!(a.admit_at("term_write", &w(6), Some("term:4"), t0).err(), Some(RATE), "6 + 6 bytes > a burst of 10");
        assert!(a.admit_at("term_write", &w(6), Some("term:5"), t0).is_ok(), "another stream has its own bucket");
        assert!(a.admit_at("term_write", &w(6), Some("term:4"), t0 + Duration::from_millis(300)).is_ok(), "3 bytes refilled");
    }

    /// A refusal decided before admission takes no slot, and a device pays a
    /// request token for it all the same: its refusals stop at its rate.
    /// The console pays nothing for one.
    #[test]
    fn a_refusal_before_admission_takes_no_slot_and_costs_a_device_a_token() {
        let mut budgets = BUDGETS;
        budgets.device.requests = Rate { per_sec: 1.0, burst: 3.0 };
        budgets.remote.in_flight = 1;
        let b = Budget::new(budgets);
        let a = device(&b, "phone");
        let t0 = Instant::now();
        let held = a.admit_at("get_settings", &args(json!({})), None, t0).expect("its one slot");
        assert!(a.charge_at(t0).is_ok(), "a refusal while the one slot is held: not busy");
        assert!(a.charge_at(t0).is_ok());
        assert_eq!(a.charge_at(t0).err(), Some(RATE), "three tokens: one admitted, two refusals");
        drop(held);
        assert_eq!(a.admit_at("get_settings", &args(json!({})), None, t0).err(), Some(RATE), "refusals spent the bucket");
        let desk = b.open(2, &Caller::console(), None).unwrap();
        assert!((0..1000).all(|_| desk.charge_at(t0).is_ok()), "the console pays nothing");
    }

    /// Per-device caps hold across connections, and a remote body budget
    /// across devices — while the console's account is untouched by both.
    #[test]
    fn device_caps_span_connections_and_the_body_budget_spans_devices() {
        let mut budgets = BUDGETS;
        budgets.device.in_flight = 3;
        budgets.remote.in_flight = 2;
        budgets.remote_bodies = 4;
        budgets.device.connections = 2;
        let b = Budget::new(budgets);
        let (a1, a2) = (device(&b, "phone"), device(&b, "phone"));
        assert!(b.open(3, &paired("phone"), None).is_none(), "a third connection for a device of two");
        let t0 = Instant::now();
        let g = || args(json!({}));
        let x1 = a1.admit_at("get_settings", &g(), None, t0).unwrap();
        let x2 = a1.admit_at("get_settings", &g(), None, t0).unwrap();
        assert_eq!(a1.admit_at("get_settings", &g(), None, t0).err(), Some(BUSY), "per connection: 2");
        let x3 = a2.admit_at("get_settings", &g(), None, t0).unwrap();
        assert_eq!(a2.admit_at("get_settings", &g(), None, t0).err(), Some(BUSY), "per device: 3");
        let tab = device(&b, "tablet");
        let x4 = tab.admit_at("get_settings", &g(), None, t0).unwrap();
        assert_eq!(tab.admit_at("get_settings", &g(), None, t0).err(), Some(BUSY), "remote bodies: 4, all devices");
        let desk = b.open(9, &Caller::console(), None).unwrap();
        assert!(desk.admit_at("get_settings", &g(), None, t0).is_ok(), "the desk is never starved by devices");
        drop((x1, x2, x3, x4));
        assert!(tab.admit_at("get_settings", &g(), None, t0).is_ok(), "every slot came back with its ticket");
    }

    /// Attaches and shells per device are held by the TICKET's attach slot,
    /// which the sink keeps for the stream's life.
    #[test]
    fn attach_slots_are_held_until_the_stream_lets_them_go() {
        let mut budgets = BUDGETS;
        budgets.device.attaches = 1;
        budgets.device.shells = 1;
        let b = Budget::new(budgets);
        let a = device(&b, "phone");
        let t0 = Instant::now();
        let mut t = a.admit_at("term_open", &args(json!({})), None, t0).unwrap();
        let attach = t.attach.take().expect("term_open carries an attach slot");
        drop(t);
        assert_eq!(a.admit_at("term_open", &args(json!({})), None, t0).err(), Some(BUSY), "the sink still holds it");
        assert!(a.admit_at("shell_open", &args(json!({})), None, t0).is_ok(), "shells are counted apart");
        drop(attach);
        assert!(a.admit_at("term_open", &args(json!({})), None, t0).is_ok(), "the stream ended, the slot came back");
    }

    /// LLM10: launches have their own hourly budget, on top of the request
    /// rate, and its refusal says so — `start-budget`, with the whole seconds
    /// until the bucket holds one start again, counted from the refill so
    /// far.
    #[test]
    fn agent_launches_have_an_hourly_budget() {
        let b = Budget::new(BUDGETS);
        let a = device(&b, "phone");
        let t0 = Instant::now();
        let ok = (0..15).filter(|_| a.admit_at("new_place", &args(json!({})), None, t0).is_ok()).count();
        assert_eq!(ok, 10, "ten an hour");
        for row in LAUNCH_ROWS {
            assert_eq!(a.admit_at(row, &args(json!({})), None, t0).err(), Some(START_BUDGET), "{row}: every launch row shares it");
        }
        assert_eq!(a.answer(START_BUDGET, t0), json!("start-budget: 360"));
        let later = t0 + Duration::from_secs(100);
        assert_eq!(a.admit_at("new_place", &args(json!({})), None, later).err(), Some(START_BUDGET));
        assert_eq!(a.answer(START_BUDGET, later), json!("start-budget: 260"), "100 s of the 360 have refilled");
        assert_eq!(a.answer(START_BUDGET, t0 + Duration::from_millis(359_500)), json!("start-budget: 1"), "never 0 while refused");
        assert_eq!(a.answer(RATE, later), json!("rate"), "every other word as it is");
        assert!(a.admit_at("open_place", &args(json!({})), None, t0 + Duration::from_secs(360)).is_ok(), "one back in six minutes");
        let desk = b.open(2, &Caller::console(), None).unwrap();
        assert_eq!(desk.answer(START_BUDGET, t0), json!(START_BUDGET), "no device, no bucket to count");
    }

    /// A client sitting AT its cap is admitted and refused by turns, one each
    /// per body that finishes. That is ONE episode in the audit log — a
    /// record as it begins, one with the count once it has gone quiet —
    /// never a record per refusal, and never an end and a beginning per
    /// admission. A refusal after the quiet gap begins a new one, and the
    /// connection's close ends what is still open.
    #[test]
    fn a_client_at_its_cap_is_one_episode_until_it_goes_quiet() {
        use super::super::audit::{self, Audit};
        let dir = std::env::temp_dir().join(format!("wt-budget-episode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (log, _) = Audit::open(&dir, None, audit::Fleet::none());
        let au = Arc::new(ConnAudit::new(log, 1, None, Caller::console()));
        let mut budgets = BUDGETS;
        budgets.loopback.in_flight = 1;
        let b = Budget::new(budgets);
        let a = b.open(1, &Caller::console(), Some(au)).unwrap();
        let get = || args(json!({}));
        let records = || -> Vec<Value> {
            let text = std::fs::read_to_string(dir.join(audit::FILE)).unwrap();
            text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).filter(|r| r["detail"]["limit"] == "in_flight").collect()
        };
        let t0 = Instant::now();
        for i in 0..20 {
            let t = t0 + Duration::from_millis(i);
            let held = a.admit_at("get_settings", &get(), None, t).expect("the slot came back");
            assert_eq!(a.admit_at("get_settings", &get(), None, t).err(), Some(BUSY));
            drop(held);
        }
        let r = records();
        assert_eq!(r.len(), 1, "twenty refusals between twenty admissions: one record: {r:#?}");
        assert_eq!((r[0]["detail"]["episode"].as_str(), r[0]["detail"]["answered"].as_str()), (Some("began"), Some(BUSY)));

        let late = t0 + QUIET + Duration::from_secs(1);
        let held = a.admit_at("get_settings", &get(), None, late).unwrap();
        let r = records();
        assert_eq!((r.len(), r[1]["detail"]["episode"].as_str(), r[1]["detail"]["count"].as_u64()), (2, Some("ended"), Some(20)), "quiet: {r:#?}");
        assert_eq!(a.admit_at("get_settings", &get(), None, late).err(), Some(BUSY));
        drop(held);
        assert_eq!(records()[2]["detail"]["episode"], json!("began"), "a refusal after the gap begins a new one");
        assert_eq!(a.close(), "in_flight: 1 refused");
        let r = records();
        assert_eq!((r.len(), r[3]["detail"]["count"].as_u64()), (4, Some(1)), "the close ends it: {r:#?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The input map stays small: a full bucket is dropped as soon as room is
    /// needed, and past the size a NEW stream is busy rather than reset.
    #[test]
    fn input_buckets_are_bounded_and_full_ones_are_forgotten() {
        let rate = Rate { per_sec: 1.0, burst: 1.0 };
        let m = Mutex::new(HashMap::new());
        let t0 = Instant::now();
        for i in 0..MAX_INPUT_KEYS {
            assert!(take_input(&m, &format!("k{i}"), rate, 1, t0).is_ok());
        }
        assert_eq!(take_input(&m, "new", rate, 1, t0), Err(BUSY), "no room, and nothing has refilled");
        assert_eq!(take_input(&m, "k0", rate, 1, t0), Err(RATE), "an old stream is still rated, not reset");
        assert!(take_input(&m, "new", rate, 1, t0 + Duration::from_secs(2)).is_ok(), "refilled buckets make room");
        assert_eq!(m.lock().unwrap().len(), 1);
    }
}
