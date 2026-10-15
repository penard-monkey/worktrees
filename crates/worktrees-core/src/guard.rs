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
fn sysctl<T: Copy + Default>(name: &str) -> Option<T> {
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
    let named: Vec<&str> = probes.iter().filter_map(|p| p.tmux.as_deref().and_then(crate::agent::session_name)).collect();
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
        let session = p.tmux.as_deref().and_then(crate::agent::session_name);
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
    let probes = crate::agent::live_probes();
    let live = live_agents(panes.as_ref(), &probes, exclude);
    match decide(provider, &live, &mem, &limits, &how_to_change()) {
        Some(why) => Err(crate::harness::Refusal::Soft(format!("Not started: {why}"))),
        None => Ok(problems),
    }
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
}
