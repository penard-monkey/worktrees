//! Refuse to START another agent when the machine already runs too many, or is
//! short of memory.
//!
//! # Why this exists
//!
//! On 2026-10-14 a 36 GB Mac locked up with about 35 Claude sessions alive
//! across two repositories. Each costs ~1.2 GB and four node processes before
//! it does any work (claude itself, the MCP servers it starts), so the machine
//! was in swap before any lane ran a gate; seven gates then finished it.
//! `quota` could not have helped: it reads a plan window, which lags. The
//! lag-free signals are how many agents are alive right now and how much
//! memory is left, and both are read here.
//!
//! # Where it sits: `ops::launch`, beside `Adapter::prepare`
//!
//! Same launch-refusal path as `quota`, with the same three properties: the
//! place and its brief are created and only the launch is refused; ONE
//! override (`launch.force`: `--force`, the app's "Launch anyway", MCP
//! `force`); and the refusal leaves whatever was running untouched. It is
//! deliberately NOT inside each adapter: memory is spent per session, not per
//! provider, so a harness added tomorrow is capped by default — the opposite
//! polarity from `usage()`'s opt-in, for that reason.
//!
//! # Counting is race-free because it happens under a machine lock
//!
//! Eight parallel `create_worktree` calls would otherwise all count the same
//! N and all launch. `ops::launch` counts twice: once unlocked, as a cheap
//! early refusal, then — after the harness's own probes, which can wait 15s on
//! a network and must not hold up every launch on the machine — under
//! `launch_lock()` (an flock in the user's state dir, shared by every CLI, MCP
//! server and the app), held from before that SECOND count until
//! `new-session` has returned, so the next caller counts the session the
//! previous one just made. The reservation IS the lock: nothing to expire,
//! nothing to reconcile.
//!
//! # It FAILS OPEN
//!
//! No tmux, no readable memory, no RAM size: allow. Same reasoning as
//! `quota` — not being able to measure is not evidence of a problem.
//!
//! # The user owns the numbers (`[limits]`, `USER_ONLY_KEYS`)
//!
//! ```toml
//! [limits]
//! max_live_agents = 18          # default: half the machine's RAM in GB; 0 = off
//! max_live = { claude = 12 }    # optional, per provider
//! min_free_memory_pct = 20      # default 20; 0 = off
//! max_swap_used_pct = 0         # default off — macOS grows swap, so % is noisy
//! ```

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

pub const TABLE: &str = "limits";
/// Refuse when less than this share of memory is free (macOS
/// `kern.memorystatus_level`, Linux `MemAvailable / MemTotal`).
pub const DEFAULT_MIN_FREE_PCT: u8 = 20;
const GIB: u64 = 1 << 30;

/// `[limits]` as `admit` reads it. `max_live: None` is "no cap".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub max_live: Option<u32>,
    pub per_provider: BTreeMap<String, u32>,
    pub min_free_pct: u8,
    pub max_swap_pct: u8,
}

/// The default cap: half the RAM in GiB. Each idle session is ~1.2 GB, so
/// this leaves roughly 40% of memory for the work the sessions start. `None`
/// when the size is unknown (fail open).
pub fn default_cap(ram_bytes: Option<u64>) -> Option<u32> {
    ram_bytes.map(|b| ((b / GIB) / 2).max(1) as u32)
}

/// `[limits]` from `config.toml`'s text, defaults filled from `ram_bytes`.
/// Lenient like `[quota]`: a typo is a problem line, never a lock-out.
pub fn limits_from(text: &str, ram_bytes: Option<u64>) -> (Limits, Vec<String>) {
    let mut l = Limits {
        max_live: default_cap(ram_bytes),
        per_provider: BTreeMap::new(),
        min_free_pct: DEFAULT_MIN_FREE_PCT,
        max_swap_pct: 0,
    };
    let mut problems = Vec::new();
    let root: BTreeMap<String, toml::Value> = match toml::from_str(text) {
        Ok(r) => r,
        Err(_) => {
            problems.push("config.toml does not parse, so [limits] could not be read — the defaults apply".into());
            return (l, problems);
        }
    };
    let Some(v) = root.get(TABLE) else { return (l, problems) };
    let Some(t) = v.as_table() else {
        problems.push("limits must be a table ([limits]) — ignored".into());
        return (l, problems);
    };
    let int = |v: &toml::Value, lo: i64, hi: i64| v.as_integer().filter(|n| (lo..=hi).contains(n));
    for (k, v) in t {
        match k.as_str() {
            "max_live_agents" => match int(v, 0, 1000) {
                Some(0) => l.max_live = None,
                Some(n) => l.max_live = Some(n as u32),
                None => problems.push("[limits] max_live_agents is not a whole number from 0 to 1000 — ignored".into()),
            },
            "max_live" => match v.as_table() {
                Some(pt) => {
                    for (prov, n) in pt {
                        if !crate::provider::PROVIDERS.iter().any(|p| p.id == prov) {
                            let known: Vec<&str> = crate::provider::PROVIDERS.iter().map(|p| p.id).collect();
                            problems.push(format!("[limits] max_live.{prov}: no such provider ({}) — ignored", known.join(", ")));
                            continue;
                        }
                        match int(n, 0, 1000) {
                            Some(0) => {}
                            Some(n) => {
                                l.per_provider.insert(prov.clone(), n as u32);
                            }
                            None => problems.push(format!("[limits] max_live.{prov} is not a whole number from 0 to 1000 — ignored")),
                        }
                    }
                }
                None => problems.push("[limits] max_live must be a table, e.g. { claude = 12 } — ignored".into()),
            },
            "min_free_memory_pct" => match int(v, 0, 90) {
                Some(n) => l.min_free_pct = n as u8,
                None => problems.push("[limits] min_free_memory_pct is not a whole number from 0 to 90 — ignored".into()),
            },
            "max_swap_used_pct" => match int(v, 0, 100) {
                Some(n) => l.max_swap_pct = n as u8,
                None => problems.push("[limits] max_swap_used_pct is not a whole number from 0 to 100 — ignored".into()),
            },
            other => problems.push(format!("[limits] unknown key `{other}` — ignored")),
        }
    }
    (l, problems)
}

// ── memory ──────────────────────────────────────────────────────────────────

/// What the machine says about its memory. Every field may be unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Memory {
    pub ram_bytes: Option<u64>,
    pub free_pct: Option<u8>,
    pub swap_used_pct: Option<u8>,
}

/// The test seam: `WORKTREES_GUARD_PROBE=off` reads nothing (the bats suite
/// pins it, so a busy developer machine cannot fail a test), a path reads a
/// JSON `Memory` from that file, unset reads the machine.
pub fn memory() -> Memory {
    match std::env::var("WORKTREES_GUARD_PROBE").ok().filter(|s| !s.is_empty()) {
        Some(v) if v == "off" => Memory::default(),
        Some(path) => std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default(),
        None => read_memory(),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn sysctl<T: Copy + Default>(name: &str) -> Option<T> {
    let cname = std::ffi::CString::new(name).ok()?;
    let mut out = T::default();
    let mut len = std::mem::size_of::<T>();
    // SAFETY: `out` is a valid, writable T of `len` bytes; sysctlbyname
    // writes at most `len` bytes and reports how many it wrote.
    let r = unsafe {
        libc::sysctlbyname(cname.as_ptr(), &mut out as *mut T as *mut libc::c_void, &mut len, std::ptr::null_mut(), 0)
    };
    (r == 0 && len == std::mem::size_of::<T>()).then_some(out)
}

/// `struct xsw_usage` from <sys/sysctl.h>.
#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XswUsage {
    total: u64,
    avail: u64,
    used: u64,
    pagesize: u32,
    encrypted: u32,
}

/// Through libc, never a subprocess: `memory_pressure` takes about a second.
#[cfg(target_os = "macos")]
fn read_memory() -> Memory {
    let level: Option<u32> = sysctl("kern.memorystatus_level");
    let swap: Option<XswUsage> = sysctl("vm.swapusage");
    Memory {
        ram_bytes: sysctl::<u64>("hw.memsize"),
        free_pct: level.filter(|l| *l <= 100).map(|l| l as u8),
        swap_used_pct: swap.filter(|s| s.total > 0).map(|s| ((s.used * 100) / s.total).min(100) as u8),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_memory() -> Memory {
    std::fs::read_to_string("/proc/meminfo").map(|t| meminfo(&t)).unwrap_or_default()
}

/// `/proc/meminfo` → `Memory`. kB values; MemAvailable is the kernel's own
/// estimate of what can be had without swapping.
pub fn meminfo(text: &str) -> Memory {
    let kb = |key: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(key) && l[key.len()..].starts_with(':'))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|n| n.parse().ok())
    };
    let pct = |part: Option<u64>, whole: Option<u64>| match (part, whole) {
        (Some(p), Some(w)) if w > 0 => Some(((p * 100) / w).min(100) as u8),
        _ => None,
    };
    let (total, avail) = (kb("MemTotal"), kb("MemAvailable"));
    let (st, sf) = (kb("SwapTotal"), kb("SwapFree"));
    Memory {
        ram_bytes: total.map(|k| k * 1024),
        free_pct: pct(avail, total),
        swap_used_pct: pct(st.zip(sf).map(|(t, f)| t.saturating_sub(f)), st),
    }
}

// ── counting ────────────────────────────────────────────────────────────────

/// One live agent session, as a refusal names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub label: String,
    pub provider: String,
}

/// Every live agent on the machine: tmux sessions with an agent pane, plus
/// Claude sessions whose probe names no session in that list (started in a
/// terminal, an IDE, another tmux server) — they cost the same memory.
/// A session whose only evidence is a bare `node` pane counts only when a
/// claude probe names it: a metro bundler is not an agent (review of #486).
/// `exclude` are the sessions the launch itself would replace or reuse.
pub fn live_agents(panes: Option<&crate::tmux::PaneList>, probes: &[crate::agent::ClaudeProbe], exclude: &[String]) -> Vec<Live> {
    let mut out: Vec<Live> = Vec::new();
    // By the session each Claude runs in NOW (`ClaudeProbe::session`): after a
    // rename its probe still names the old one, and it would count twice.
    let named: Vec<&str> = probes.iter().filter_map(|p| p.session()).collect();
    let in_tmux: Vec<(String, &'static str)> = panes
        .map(|p| p.agent_sessions())
        .unwrap_or_default()
        .into_iter()
        .filter(|(name, _, only_node)| !*only_node || named.contains(&name.as_str()))
        .map(|(name, provider, _)| (name, provider))
        .collect();
    for (name, provider) in &in_tmux {
        if !exclude.contains(name) {
            out.push(Live { label: name.clone(), provider: provider.to_string() });
        }
    }
    for p in probes {
        let session = p.session();
        if session.is_some_and(|s| in_tmux.iter().any(|(n, _)| n == s) || exclude.iter().any(|e| e == s)) {
            continue;
        }
        out.push(Live { label: format!("claude pid {} ({})", p.pid, p.cwd), provider: crate::provider::CLAUDE.id.into() });
    }
    out
}

// ── the decision ────────────────────────────────────────────────────────────

fn names(live: &[&Live]) -> String {
    const SHOWN: usize = 6;
    let mut s: Vec<String> = live.iter().take(SHOWN).map(|l| l.label.clone()).collect();
    if live.len() > SHOWN {
        s.push(format!("and {} more", live.len() - SHOWN));
    }
    s.join(", ")
}

/// `Some(reason)` to refuse. Pure: the caller gathers the inputs.
pub fn decide(provider: Option<&str>, live: &[Live], mem: &Memory, l: &Limits, how: &str) -> Option<String> {
    let all: Vec<&Live> = live.iter().collect();
    let close = "Close finished agents first (`worktrees close <slug>`, MCP close_session)";
    if let Some(cap) = l.max_live {
        if all.len() as u32 >= cap {
            return Some(format!(
                "{} agents are already live on this machine (limit {cap}, `[limits] max_live_agents`): {}. {close}. {how}",
                all.len(),
                names(&all)
            ));
        }
    }
    if let Some((prov, cap)) = provider.and_then(|p| l.per_provider.get_key_value(p)) {
        let mine: Vec<&Live> = live.iter().filter(|x| &x.provider == prov).collect();
        if mine.len() as u32 >= *cap {
            return Some(format!(
                "{} {prov} agents are already live on this machine (limit {cap}, `[limits] max_live.{prov}`): {}. {close}. {how}",
                mine.len(),
                names(&mine)
            ));
        }
    }
    if l.min_free_pct > 0 {
        if let Some(free) = mem.free_pct.filter(|f| *f < l.min_free_pct) {
            return Some(format!(
                "this machine has {free}% of its memory free (limit {}%, `[limits] min_free_memory_pct`) with {} agents live. {close}. {how}",
                l.min_free_pct,
                all.len()
            ));
        }
    }
    if l.max_swap_pct > 0 {
        if let Some(swap) = mem.swap_used_pct.filter(|s| *s >= l.max_swap_pct) {
            return Some(format!(
                "this machine's swap is {swap}% used (limit {}%, `[limits] max_swap_used_pct`) with {} agents live. {close}. {how}",
                l.max_swap_pct,
                all.len()
            ));
        }
    }
    None
}

fn how_to_change() -> String {
    let path = crate::config::config_toml_path();
    let shown = path.to_string_lossy().into_owned();
    let home = std::env::var("HOME").unwrap_or_default();
    let shown = match shown.strip_prefix(&format!("{}/", home.trim_end_matches('/'))) {
        Some(rest) if !home.is_empty() => format!("~/{rest}"),
        _ => shown,
    };
    format!("The user can change these limits under `[limits]` in {shown}")
}

/// The user's limits, read now, defaults filled from this machine.
pub fn user_limits(mem: &Memory) -> (Limits, Vec<String>) {
    let text = std::fs::read_to_string(crate::config::config_toml_path()).unwrap_or_default();
    limits_from(&text, mem.ram_bytes)
}

/// The gate `ops::launch` calls — twice: once unlocked, as a cheap early
/// refusal before the harness's own (network) probes, and once under
/// `launch_lock()`, which is the count that decides. `exclude`: the sessions
/// this launch replaces or reuses. `Ok` carries `[limits]` problems for the
/// caller to show.
pub fn admit(provider: Option<&str>, force: bool, exclude: &[String]) -> Result<Vec<String>, crate::harness::Refusal> {
    if force {
        return Ok(Vec::new());
    }
    let mem = memory();
    let (limits, problems) = user_limits(&mem);
    if limits.max_live.is_none() && limits.per_provider.is_empty() && limits.min_free_pct == 0 && limits.max_swap_pct == 0 {
        return Ok(problems); // all off: pay for no tmux listing
    }
    let panes = crate::tmux::PaneList::fetch();
    let probes = crate::agent::live_probes_in(panes.as_ref());
    let live = live_agents(panes.as_ref(), &probes, exclude);
    match decide(provider, &live, &mem, &limits, &how_to_change()) {
        Some(why) => Err(crate::harness::Refusal::Soft(format!("Not started: {why}"))),
        None => Ok(problems),
    }
}

// ── writing [limits] (Settings → Agent policy → Limits) ─────────────────────

/// A `[limits]` key Settings sets, each with the range `limits_from` accepts
/// for it. The setter REFUSES whatever the reader would ignore (a value out of
/// range, a provider that does not exist), with the reader's own words, so a
/// write can never be a value the guard then reports as a problem and drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitKey {
    MaxLiveAgents,
    MaxLive(String),
    MinFreeMemoryPct,
    MaxSwapUsedPct,
}

fn provider_ids() -> String {
    crate::provider::PROVIDERS.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")
}

impl LimitKey {
    /// `max_live_agents`, `max_live.<provider>`, `min_free_memory_pct` or
    /// `max_swap_used_pct`: the name Settings sends, which is data (it names a
    /// key in a file, never argv) and is checked against this fixed set.
    pub fn parse(s: &str) -> Result<LimitKey, String> {
        match s {
            "max_live_agents" => Ok(LimitKey::MaxLiveAgents),
            "min_free_memory_pct" => Ok(LimitKey::MinFreeMemoryPct),
            "max_swap_used_pct" => Ok(LimitKey::MaxSwapUsedPct),
            _ => match s.strip_prefix("max_live.") {
                Some(p) if crate::provider::PROVIDERS.iter().any(|x| x.id == p) => Ok(LimitKey::MaxLive(p.to_string())),
                Some(p) => Err(format!("[limits] max_live.{p}: no such provider ({})", provider_ids())),
                None => Err(format!("[limits] has no key `{s}`")),
            },
        }
    }

    /// The key as the file spells it.
    pub fn name(&self) -> String {
        match self {
            LimitKey::MaxLiveAgents => "max_live_agents".into(),
            LimitKey::MaxLive(p) => format!("max_live.{p}"),
            LimitKey::MinFreeMemoryPct => "min_free_memory_pct".into(),
            LimitKey::MaxSwapUsedPct => "max_swap_used_pct".into(),
        }
    }

    /// The inclusive range `limits_from` accepts — the same numbers, so the
    /// two cannot disagree (`a_written_limit_is_one_the_guard_reads`).
    pub fn range(&self) -> (u32, u32) {
        match self {
            LimitKey::MaxLiveAgents | LimitKey::MaxLive(_) => (0, 1000),
            LimitKey::MinFreeMemoryPct => (0, 90),
            LimitKey::MaxSwapUsedPct => (0, 100),
        }
    }

    /// `n` is one the guard would apply, or the reason it would not.
    pub fn check(&self, n: u32) -> Result<(), String> {
        let (lo, hi) = self.range();
        if (lo..=hi).contains(&n) {
            Ok(())
        } else {
            Err(format!("[limits] {} must be a whole number from {lo} to {hi} (got {n})", self.name()))
        }
    }
}

/// The `[limits]` keys the file sets, as the guard would read them (a value it
/// ignores is not here; it is in `limits_from`'s problems).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ExplicitLimits {
    pub max_live_agents: Option<u32>,
    pub max_live: BTreeMap<String, u32>,
    pub min_free_memory_pct: Option<u32>,
    pub max_swap_used_pct: Option<u32>,
}

/// What the file says about each key, unfilled — for Settings to tell a
/// default from a choice, and for Reset to have something to remove.
pub fn explicit_from(text: &str) -> ExplicitLimits {
    let mut e = ExplicitLimits::default();
    let Ok(root) = toml::from_str::<toml::Table>(text) else { return e };
    let Some(t) = root.get(TABLE).and_then(|v| v.as_table()) else { return e };
    let get = |k: &LimitKey, v: Option<&toml::Value>| {
        v.and_then(|v| v.as_integer()).and_then(|n| u32::try_from(n).ok()).filter(|n| k.check(*n).is_ok())
    };
    e.max_live_agents = get(&LimitKey::MaxLiveAgents, t.get("max_live_agents"));
    e.min_free_memory_pct = get(&LimitKey::MinFreeMemoryPct, t.get("min_free_memory_pct"));
    e.max_swap_used_pct = get(&LimitKey::MaxSwapUsedPct, t.get("max_swap_used_pct"));
    if let Some(pt) = t.get("max_live").and_then(|v| v.as_table()) {
        for (p, v) in pt {
            if let Ok(k) = LimitKey::parse(&format!("max_live.{p}")) {
                if let Some(n) = get(&k, Some(v)) {
                    e.max_live.insert(p.clone(), n);
                }
            }
        }
    }
    e
}

/// Set (`Some`) or remove (`None`) one `[limits]` key in the user's
/// config.toml — the user's act only: Settings, and a policy preset applied
/// from it; the MCP server never calls it. Returns whether anything changed.
pub fn set_limit(key: &LimitKey, n: Option<u32>) -> Result<bool, String> {
    set_limit_at(&crate::config::config_toml_path(), key, n)
}

pub fn set_limit_at(path: &std::path::Path, key: &LimitKey, n: Option<u32>) -> Result<bool, String> {
    if let Some(n) = n {
        key.check(n)?;
    }
    crate::config::edit_user_config(path, |text| {
        let root: toml::Table = toml::from_str(text).map_err(|_| limits_unparsable())?;
        let t = root.get(TABLE).and_then(|v| v.as_table());
        let cur = match key {
            LimitKey::MaxLive(p) => t.and_then(|t| t.get("max_live")).and_then(|v| v.as_table()).and_then(|m| m.get(p)),
            _ => t.and_then(|t| t.get(&key.name())),
        };
        let same = match (cur, n) {
            (None, None) => true,
            (Some(v), Some(n)) => v.as_integer() == Some(i64::from(n)),
            _ => false,
        };
        if same {
            return Ok(None);
        }
        with_limit(text, key, n).map(Some)
    })
}

fn limits_unparsable() -> String {
    "config.toml does not parse — fix it by hand before changing [limits]".into()
}

/// `text` with one `[limits]` key set to `n`, or removed for `None`. Pure, for
/// the tests.
///
/// `slots::with_slot`'s rules: the key's VALUE is edited in place, so its
/// inline comment and indentation survive; the file's line ending is kept; a
/// missing key goes after the table's last key (or under its header), or the
/// table at the end. A provider's cap is written as a dotted key
/// (`max_live.claude = 12`), unless the file already holds an inline
/// `max_live = { … }`, which is rewritten whole. The result must re-parse to
/// exactly the old document with this one key changed, or nothing is written
/// — a `[limits.max_live]` sub-table, say, refuses rather than guess.
pub fn with_limit(text: &str, key: &LimitKey, n: Option<u32>) -> Result<String, String> {
    if let Some(n) = n {
        key.check(n)?;
    }
    let before: toml::Table = toml::from_str(text).map_err(|_| limits_unparsable())?;
    let refuse = || format!("could not set [limits] {} in your config.toml safely (an unusual layout?) — edit it by hand", key.name());
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let starts = |l: &str, name: &str| l.trim_start().strip_prefix(name).is_some_and(|r| r.trim_start().starts_with('='));
    // An inline `max_live = { … }` already in [limits]: that line is rewritten
    // with the new map, rather than a dotted key added that TOML would refuse.
    let inline_map: Option<BTreeMap<String, i64>> = match key {
        LimitKey::MaxLive(_) => before
            .get(TABLE)
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("max_live"))
            .and_then(|v| v.as_table())
            .map(|m| m.iter().filter_map(|(k, v)| v.as_integer().map(|n| (k.clone(), n))).collect()),
        _ => None,
    };
    let comment_of = |t: &str| -> String {
        let after_eq = &t[t.find('=').map_or(t.len(), |i| i + 1)..];
        // A `#` inside the inline table's braces is not a comment; only look
        // past the closing brace when there is one.
        let tail_from = if after_eq.trim_start().starts_with('{') { after_eq.rfind('}').map_or(0, |i| i + 1) } else { 0 };
        let tail = &after_eq[tail_from..];
        tail.find(" #").or_else(|| tail.find("\t#")).map_or(String::new(), |i| {
            let start = tail[..i].trim_end_matches([' ', '\t']).len();
            tail[start..].to_string()
        })
    };
    let name = key.name();
    let mut out: Vec<String> = Vec::new();
    let mut in_limits = false;
    let mut header: Option<usize> = None;
    let mut last_key: Option<usize> = None;
    let mut handled = false;
    for l in text.lines() {
        let t = l.trim_start();
        let indent = &l[..l.len() - t.len()];
        if t.starts_with('[') {
            // `[limits]  # note` is ours; `[limits.x]` and `[[limits]]` are not.
            in_limits = t.split('#').next().unwrap_or("").trim_end() == format!("[{TABLE}]");
            out.push(l.to_string());
            if in_limits && header.is_none() {
                header = Some(out.len());
            }
            continue;
        }
        if in_limits {
            // `max_live = { … }` (inline) is rewritten whole; a dotted
            // `max_live.<p> = n` is an ordinary key, handled below.
            if let (LimitKey::MaxLive(p), Some(map)) = (key, inline_map.as_ref()) {
                if starts(l, "max_live") {
                    handled = true;
                    let mut m = map.clone();
                    match n {
                        Some(n) => {
                            m.insert(p.clone(), i64::from(n));
                        }
                        None => {
                            m.remove(p);
                        }
                    }
                    if !m.is_empty() {
                        let body: Vec<String> = m.iter().map(|(k, v)| format!("{k} = {v}")).collect();
                        out.push(format!("{indent}max_live = {{ {} }}{}", body.join(", "), comment_of(t)));
                        last_key = Some(out.len());
                    }
                    continue;
                }
            }
            if starts(l, &name) {
                handled = true;
                if let Some(n) = n {
                    out.push(format!("{indent}{name} = {n}{}", comment_of(t)));
                    last_key = Some(out.len());
                }
                continue;
            }
        }
        out.push(l.to_string());
        if in_limits && !t.is_empty() && !t.starts_with('#') {
            last_key = Some(out.len());
        }
    }
    if let (Some(n), false) = (n, handled) {
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
    // What the document must be: the old one with exactly this key changed.
    let mut want = before;
    let limits = want.entry(TABLE).or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(limits) = limits.as_table_mut() else { return Err(refuse()) };
    match (key, n) {
        (LimitKey::MaxLive(p), Some(n)) => {
            let m = limits.entry("max_live").or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let Some(m) = m.as_table_mut() else { return Err(refuse()) };
            m.insert(p.clone(), toml::Value::Integer(i64::from(n)));
        }
        (LimitKey::MaxLive(p), None) => {
            if let Some(m) = limits.get_mut("max_live").and_then(|v| v.as_table_mut()) {
                m.remove(p);
                if m.is_empty() {
                    limits.remove("max_live");
                }
            }
        }
        (k, Some(n)) => {
            limits.insert(k.name(), toml::Value::Integer(i64::from(n)));
        }
        (k, None) => {
            limits.remove(&k.name());
        }
    }
    if after != want {
        return Err(refuse());
    }
    Ok(new)
}

/// The machine-wide launch lock: an exclusive flock on
/// `$XDG_STATE_HOME/worktrees/launch.lock`, held by the caller from the count
/// that decides until its session exists — never across a network probe.
/// `on_wait` is told when another launch holds it, so a stuck holder is not
/// a silent stall for every launch on the machine. `None` when there is no
/// state dir or the lock cannot be taken — fail open, an unlocked launch is
/// the behaviour before this existed.
pub fn launch_lock(on_wait: impl FnOnce()) -> Option<File> {
    let dir = crate::init::state_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let f = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(dir.join("launch.lock")).ok()?;
    // SAFETY: flock on a live descriptor owned by `f`.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Some(f);
    }
    on_wait();
    // SAFETY: as above.
    (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0).then_some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lim(max: Option<u32>) -> Limits {
        Limits { max_live: max, per_provider: BTreeMap::new(), min_free_pct: 0, max_swap_pct: 0 }
    }

    fn live(n: usize, provider: &str) -> Vec<Live> {
        (0..n).map(|i| Live { label: format!("s{i}"), provider: provider.into() }).collect()
    }

    #[test]
    fn the_default_cap_is_half_the_ram_and_off_when_unknown() {
        assert_eq!(default_cap(Some(36 * GIB)), Some(18), "the incident's machine");
        assert_eq!(default_cap(Some(38_654_705_664)), Some(18), "hw.memsize of that machine");
        assert_eq!(default_cap(Some(GIB)), Some(1), "never zero");
        assert_eq!(default_cap(None), None);
        let (l, p) = limits_from("", Some(16 * GIB));
        assert_eq!((l.max_live, l.min_free_pct, l.max_swap_pct), (Some(8), DEFAULT_MIN_FREE_PCT, 0));
        assert!(p.is_empty());
    }

    #[test]
    fn limits_parse_and_zero_turns_each_off() {
        let (l, p) = limits_from(
            "[limits]\nmax_live_agents = 0\nmax_live = { claude = 3, codex = 0 }\nmin_free_memory_pct = 0\nmax_swap_used_pct = 80\n",
            Some(36 * GIB),
        );
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(l.max_live, None);
        assert_eq!(l.per_provider.get("claude"), Some(&3));
        assert!(!l.per_provider.contains_key("codex"));
        assert_eq!((l.min_free_pct, l.max_swap_pct), (0, 80));
        let (l, p) = limits_from("[limits]\nmax_live_agents = \"many\"\nwat = 1\n", Some(36 * GIB));
        assert_eq!(l.max_live, Some(18), "a typo keeps the default");
        assert_eq!(p.len(), 2, "{p:?}");
        let (l, p) = limits_from("[limits]\nmax_live = { cluade = 2 }\n", Some(36 * GIB));
        assert!(l.per_provider.is_empty());
        assert!(p.len() == 1 && p[0].contains("no such provider"), "a misspelt provider is said, not dropped: {p:?}");
    }

    #[test]
    fn the_cap_refuses_at_the_limit_and_not_below() {
        let m = Memory::default();
        assert!(decide(Some("claude"), &live(2, "claude"), &m, &lim(Some(3)), "").is_none());
        let why = decide(Some("claude"), &live(3, "claude"), &m, &lim(Some(3)), "HOW").unwrap();
        assert!(why.starts_with("3 agents are already live on this machine (limit 3"), "{why}");
        assert!(why.contains("s0, s1, s2") && why.ends_with("HOW"), "{why}");
        assert!(decide(Some("claude"), &live(99, "claude"), &m, &lim(None), "").is_none(), "no cap, no refusal");
    }

    #[test]
    fn a_provider_cap_counts_only_that_provider() {
        let mut l = lim(None);
        l.per_provider.insert("claude".into(), 2);
        let mut v = live(1, "claude");
        v.extend(live(5, "codex"));
        assert!(decide(Some("claude"), &v, &Memory::default(), &l, "").is_none());
        assert!(decide(Some("codex"), &v, &Memory::default(), &l, "").is_none(), "codex has no cap of its own");
        v.extend(live(1, "claude"));
        let why = decide(Some("claude"), &v, &Memory::default(), &l, "").unwrap();
        assert!(why.starts_with("2 claude agents"), "{why}");
    }

    #[test]
    fn memory_refuses_below_the_floor_and_fails_open_when_unknown() {
        let mut l = lim(None);
        l.min_free_pct = 20;
        let low = Memory { free_pct: Some(14), ..Memory::default() };
        let why = decide(None, &live(4, "claude"), &low, &l, "").unwrap();
        assert!(why.starts_with("this machine has 14% of its memory free (limit 20%"), "{why}");
        assert!(decide(None, &[], &Memory { free_pct: Some(20), ..Memory::default() }, &l, "").is_none());
        assert!(decide(None, &[], &Memory::default(), &l, "").is_none(), "unreadable → allow");
        l.max_swap_pct = 50;
        assert!(decide(None, &[], &Memory { swap_used_pct: Some(83), ..Memory::default() }, &l, "").is_some());
    }

    #[test]
    fn meminfo_parses_linux() {
        let m = meminfo("MemTotal:       16000000 kB\nMemFree:  100 kB\nMemAvailable:    2400000 kB\nSwapTotal: 1000 kB\nSwapFree: 250 kB\n");
        assert_eq!(m, Memory { ram_bytes: Some(16_000_000 * 1024), free_pct: Some(15), swap_used_pct: Some(75) });
        assert_eq!(meminfo("garbage"), Memory::default());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn this_mac_answers_all_three() {
        let m = read_memory();
        assert!(m.ram_bytes.is_some_and(|b| b > GIB), "{m:?}");
        assert!(m.free_pct.is_some(), "kern.memorystatus_level: {m:?}");
        assert!(m.swap_used_pct.is_some() || sysctl::<XswUsage>("vm.swapusage").is_some_and(|s| s.total == 0), "{m:?}");
    }

    #[test]
    fn live_agents_count_tmux_and_claude_outside_it_once() {
        let panes = crate::tmux::PaneList::from_rows(vec![
            ("repo-a".into(), "/r/a".into(), "claude".into()),
            ("repo-a".into(), "/r/a".into(), "zsh".into()),
            ("repo-b".into(), "/r/b".into(), "codex".into()),
            ("repo-b-term".into(), "/r/b".into(), "zsh".into()),
            ("notes".into(), "/n".into(), "vim".into()),
            ("repo-c".into(), "/r/c".into(), "claude".into()),
        ]);
        let probe = |pid: i32, tmux: Option<&str>| -> crate::agent::ClaudeProbe {
            serde_json::from_value(serde_json::json!({"pid": pid, "cwd": "/x", "status": "idle", "tmux": tmux})).unwrap()
        };
        let probes = vec![probe(10, Some("repo-a:@1.%1")), probe(11, None), probe(12, Some("elsewhere:@1.%3"))];
        let got = live_agents(Some(&panes), &probes, &["repo-c".into()]);
        let labels: Vec<&str> = got.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["repo-a", "repo-b", "claude pid 11 (/x)", "claude pid 12 (/x)"]);
        assert_eq!(got[1].provider, "codex");
    }

    /// A Claude whose session `adopt` renamed still names the old one in its
    /// probe: located by its pane, it counts once, under the session it runs
    /// in now — not once for tmux and again as a "claude outside tmux". An
    /// npm Claude (`node`) needs that probe to vouch for its session at all.
    #[test]
    fn a_renamed_claude_counts_once() {
        for cmd in ["2.1.309", "node"] {
            let panes = crate::tmux::PaneList::from_panes(vec![crate::tmux::Pane {
                session: "new-(main)".into(),
                path: "/u/new".into(),
                cmd: cmd.into(),
                id: Some("%115".into()),
                ..crate::tmux::Pane::default()
            }]);
            let mut probes: Vec<crate::agent::ClaudeProbe> =
                vec![serde_json::from_value(serde_json::json!({"pid": 10, "cwd": "/no/such/old", "status": "idle", "tmux": "old-(main):@114.%115"})).unwrap()];
            crate::agent::locate(&mut probes, &panes);
            let labels: Vec<String> = live_agents(Some(&panes), &probes, &[]).into_iter().map(|l| l.label).collect();
            assert_eq!(labels, ["new-(main)"], "{cmd}");
        }
    }

    /// C15 (idea sessions, phase S): an idea conversation is a Claude in its
    /// own tmux session, `worktrees~idea~<conv>`, which is no place's. It
    /// counts against `max_live_agents` like a lane — once, by its session,
    /// with its probe vouching for it — and a start past the cap gets a lane's
    /// words. Measured: the counter needed nothing new; `~idea~` is neither a
    /// dock shell nor a provider sidecar, so `agent_sessions` takes it.
    #[test]
    fn an_idea_session_counts_against_the_cap_like_a_lane() {
        let idea = crate::ideasessions::session_name("01JCB2Z4K3M9QX2T7VBN5R6WYH");
        for cmd in ["claude", "2.1.322"] {
            let panes = crate::tmux::PaneList::from_panes(vec![
                crate::tmux::Pane { session: "repo-a".into(), path: "/r/a".into(), cmd: "claude".into(), id: Some("%1".into()), ..crate::tmux::Pane::default() },
                crate::tmux::Pane { session: idea.clone(), path: "/ideas".into(), cmd: cmd.into(), id: Some("%2".into()), ..crate::tmux::Pane::default() },
            ]);
            let mut probes: Vec<crate::agent::ClaudeProbe> =
                vec![serde_json::from_value(serde_json::json!({"pid": 10, "cwd": "/ideas", "status": "busy", "tmux": format!("{idea}:@2.%2")})).unwrap()];
            crate::agent::locate(&mut probes, &panes);
            let live = live_agents(Some(&panes), &probes, &[]);
            let labels: Vec<&str> = live.iter().map(|l| l.label.as_str()).collect();
            assert_eq!(labels, ["repo-a", idea.as_str()], "{cmd}");
            let why = decide(Some("claude"), &live, &Memory::default(), &lim(Some(2)), "HOW").unwrap();
            assert!(why.starts_with("2 agents are already live on this machine (limit 2, `[limits] max_live_agents`)"), "{why}");
            assert!(why.contains(&idea), "{why}");
            // Excluded as the session a launch would reuse, it does not count.
            assert_eq!(live_agents(Some(&panes), &probes, std::slice::from_ref(&idea)).len(), 1);
        }
    }

    /// Review of #486, B1: `node` is claude only where something says so. A
    /// bare `node` session (a bundler, a dev server) with no claude probe
    /// naming it is not an agent; one a probe names is, once.
    #[test]
    fn a_bare_node_session_counts_only_when_a_claude_probe_names_it() {
        let panes = crate::tmux::PaneList::from_rows(vec![
            ("dev-vite".into(), "/x".into(), "node".into()),
            ("lane".into(), "/l".into(), "node".into()),
            ("repo-x~agent~codex".into(), "/r".into(), "node".into()),
        ]);
        let probe: crate::agent::ClaudeProbe =
            serde_json::from_value(serde_json::json!({"pid": 9, "cwd": "/l", "status": "idle", "tmux": "lane:@1.%2"})).unwrap();
        let got = live_agents(Some(&panes), &[probe], &[]);
        let labels: Vec<&str> = got.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["lane", "repo-x~agent~codex"], "dev-vite is a dev server, lane is claude once, a sidecar is ours");
    }
    // ── the [limits] writer ────────────────────────────────────────────────

    #[test]
    fn a_limit_key_is_one_of_the_four_or_refused() {
        assert_eq!(LimitKey::parse("max_live_agents"), Ok(LimitKey::MaxLiveAgents));
        assert_eq!(LimitKey::parse("max_live.claude"), Ok(LimitKey::MaxLive("claude".into())));
        assert!(LimitKey::parse("max_live.cluade").unwrap_err().contains("no such provider"));
        assert!(LimitKey::parse("--force").is_err());
        assert!(LimitKey::parse("max_live").is_err());
    }

    #[test]
    fn a_written_limit_is_one_the_guard_reads() {
        // For every key, every value the setter accepts reads back with no
        // problem and is applied; every value just outside is refused, and is
        // one the reader would also have ignored.
        let keys = [LimitKey::MaxLiveAgents, LimitKey::MaxLive("codex".into()), LimitKey::MinFreeMemoryPct, LimitKey::MaxSwapUsedPct];
        for k in &keys {
            let (lo, hi) = k.range();
            for n in [lo, lo + 1, hi / 2, hi] {
                let text = with_limit("", k, Some(n)).unwrap_or_else(|e| panic!("{} = {n}: {e}", k.name()));
                let (l, problems) = limits_from(&text, Some(32 * GIB));
                assert!(problems.is_empty(), "{} = {n} written, then read with {problems:?}", k.name());
                match k {
                    LimitKey::MaxLiveAgents => assert_eq!(l.max_live, if n == 0 { None } else { Some(n) }),
                    LimitKey::MaxLive(p) => assert_eq!(l.per_provider.get(p).copied(), if n == 0 { None } else { Some(n) }),
                    LimitKey::MinFreeMemoryPct => assert_eq!(u32::from(l.min_free_pct), n),
                    LimitKey::MaxSwapUsedPct => assert_eq!(u32::from(l.max_swap_pct), n),
                }
                assert_eq!(explicit_from(&text), {
                    let mut e = ExplicitLimits::default();
                    match k {
                        LimitKey::MaxLiveAgents => e.max_live_agents = Some(n),
                        LimitKey::MaxLive(p) => {
                            e.max_live.insert(p.clone(), n);
                        }
                        LimitKey::MinFreeMemoryPct => e.min_free_memory_pct = Some(n),
                        LimitKey::MaxSwapUsedPct => e.max_swap_used_pct = Some(n),
                    }
                    e
                });
            }
            let over = hi + 1;
            assert!(with_limit("", k, Some(over)).unwrap_err().contains("must be a whole number"), "{} = {over}", k.name());
            let by_hand = match k {
                LimitKey::MaxLive(p) => format!("[limits]\nmax_live = {{ {p} = {over} }}\n"),
                _ => format!("[limits]\n{} = {over}\n", k.name()),
            };
            assert!(!limits_from(&by_hand, Some(32 * GIB)).1.is_empty(), "the reader ignores {} = {over} too", k.name());
        }
    }

    #[test]
    fn with_limit_edits_one_key_and_keeps_every_other_line() {
        let text = "# mine\nai_cmd = \"claude\"\n\n[limits]\nmax_live_agents = 8   # tight\nmin_free_memory_pct = 15\n\n[quota]\ngate = true\n";
        let set = with_limit(text, &LimitKey::MaxLiveAgents, Some(12)).unwrap();
        assert_eq!(set, text.replace("max_live_agents = 8   # tight", "max_live_agents = 12   # tight"));
        let removed = with_limit(text, &LimitKey::MinFreeMemoryPct, None).unwrap();
        assert_eq!(removed, text.replace("min_free_memory_pct = 15\n", ""));
        let added = with_limit(text, &LimitKey::MaxSwapUsedPct, Some(50)).unwrap();
        assert_eq!(added, text.replace("min_free_memory_pct = 15\n", "min_free_memory_pct = 15\nmax_swap_used_pct = 50\n"));
        let fresh = with_limit("ai_cmd = \"claude\"\n", &LimitKey::MaxLiveAgents, Some(2)).unwrap();
        assert_eq!(fresh, "ai_cmd = \"claude\"\n\n[limits]\nmax_live_agents = 2\n");
        // CRLF stays CRLF.
        let crlf = with_limit("[limits]\r\nmax_live_agents = 3\r\n", &LimitKey::MaxLiveAgents, Some(4)).unwrap();
        assert_eq!(crlf, "[limits]\r\nmax_live_agents = 4\r\n");
    }

    #[test]
    fn a_providers_cap_is_dotted_or_rewrites_an_inline_table() {
        let dotted = with_limit("[limits]\nmax_live_agents = 8\n", &LimitKey::MaxLive("claude".into()), Some(5)).unwrap();
        assert_eq!(dotted, "[limits]\nmax_live_agents = 8\nmax_live.claude = 5\n");
        assert_eq!(limits_from(&dotted, Some(32 * GIB)).0.per_provider.get("claude"), Some(&5));
        let again = with_limit(&dotted, &LimitKey::MaxLive("claude".into()), Some(7)).unwrap();
        assert_eq!(again, "[limits]\nmax_live_agents = 8\nmax_live.claude = 7\n");
        let dropped = with_limit(&again, &LimitKey::MaxLive("claude".into()), None).unwrap();
        assert_eq!(dropped, "[limits]\nmax_live_agents = 8\n");
        let inline = "[limits]\nmax_live = { claude = 12 }  # cap claude\n";
        let both = with_limit(inline, &LimitKey::MaxLive("codex".into()), Some(3)).unwrap();
        assert_eq!(both, "[limits]\nmax_live = { claude = 12, codex = 3 }  # cap claude\n");
        let gone = with_limit("[limits]\nmax_live = { claude = 12 }\nmin_free_memory_pct = 10\n", &LimitKey::MaxLive("claude".into()), None).unwrap();
        assert_eq!(gone, "[limits]\nmin_free_memory_pct = 10\n");
        // A layout it cannot edit safely is refused, never guessed at.
        let sub = "[limits]\nmax_live_agents = 8\n\n[limits.max_live]\nclaude = 2\n";
        assert!(with_limit(sub, &LimitKey::MaxLive("codex".into()), Some(3)).unwrap_err().contains("edit it by hand"));
        assert!(with_limit("not [ toml", &LimitKey::MaxLiveAgents, Some(3)).unwrap_err().contains("does not parse"));
    }

    #[test]
    fn set_limit_writes_only_a_change() {
        let dir = std::env::temp_dir().join(format!("wt-limits-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        assert!(set_limit_at(&path, &LimitKey::MaxLiveAgents, Some(6)).unwrap());
        assert!(!set_limit_at(&path, &LimitKey::MaxLiveAgents, Some(6)).unwrap(), "the same value is no write");
        assert_eq!(explicit_from(&std::fs::read_to_string(&path).unwrap()).max_live_agents, Some(6));
        assert!(set_limit_at(&path, &LimitKey::MaxLiveAgents, None).unwrap());
        assert!(!set_limit_at(&path, &LimitKey::MaxLiveAgents, None).unwrap(), "removing an absent key is no write");
        assert!(set_limit_at(&path, &LimitKey::MinFreeMemoryPct, Some(91)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
