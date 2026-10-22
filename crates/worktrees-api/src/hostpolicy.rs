//! Host policy: the settings that decide what THIS HOST does on everyone's
//! behalf — what an agent it launches may do, how often it fetches, what it
//! keeps of a dock shell — kept on the host (`host-policy.json` in the config
//! dir) and never in a client's own settings.
//!
//! Five, in two kinds. Two decide what an agent may do (OWASP review M2,
//! `docs/proposals/server-edition.md`, "Queued"): Settings → Codex →
//! Permissions (`codex_permissions`) and Settings → pi → Project trust
//! (`pi_project_trust`). Three configure an object only the engine has
//! (Appendix B, "One fix for all five"): the poll's auto-fetch cadence
//! (`fetch_interval_min`) and the two switches every dock shell is started
//! and saved under (`term_persist_scrollback`, `term_per_tab_history`). All
//! five used to live in the frontend's `ui-state.json` and be pushed into
//! process-global statics by the page. Under one engine per host that meant
//! the client that loaded LAST decided for every client — opening a page was
//! enough to widen Codex, or to turn scrollback persistence back on until
//! hydration landed.
//!
//! The rules:
//! - **Only an explicit act writes it**: `set_codex_permissions` /
//!   `set_pi_trust` in `project`, `set_fetch_interval` in `ops`,
//!   `set_term_history_opts` in `prefs` — each persists here, then applies,
//!   and refuses a value it does not know. Only a Settings control the user
//!   operates calls them; `app/scripts/hostpolicy-check.mjs` holds the
//!   frontend to that.
//! - **Only the ENGINE reads it.** `boot` loads it at startup; the CLI and
//!   MCP-launched lanes never call `boot`, so for the two agent policies they
//!   keep env + user config exactly as before. The order is unchanged: this
//!   file > `$WORKTREES_CODEX_PERMISSIONS` / `$WORKTREES_PI_PROJECT_TRUST` >
//!   user config > default. The three engine policies have no env or config
//!   tier: this file > the built-in default ([`EngineOpts::default`]).
//! - **Never `ui-state.json`.** The frontend writes that whole-blob, so a
//!   backend write there is erased by the next settings save. The migration
//!   below READS it, once per key, and nothing here writes it.
//! - Written through core's `config::edit_user_config` (one writer at a time,
//!   temp + rename, a symlink edited through, the mode kept). A key this build
//!   does not know survives a write; a corrupt file is refused rather than
//!   clobbered; an unknown VALUE, which only a hand edit can produce, falls
//!   through to the next tier the way core's own readers treat one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::Serialize;
use serde_json::Value;
use worktrees_core::codex::{self, Permissions};
use worktrees_core::trust::{self, PiTrust};

use crate::log::log as applog;

pub const FILE: &str = "host-policy.json";
pub const CODEX_PERMISSIONS: &str = "codex_permissions";
pub const PI_PROJECT_TRUST: &str = "pi_project_trust";
pub const FETCH_INTERVAL_MIN: &str = "fetch_interval_min";
pub const TERM_PERSIST_SCROLLBACK: &str = "term_persist_scrollback";
pub const TERM_PER_TAB_HISTORY: &str = "term_per_tab_history";

/// The auto-fetch cadences Settings → Behavior offers, in minutes; 0 is off.
/// The setter refuses anything else, and the file's value is ignored if it
/// holds anything else.
pub const FETCH_INTERVALS_MIN: [u64; 4] = [0, 5, 15, 60];

pub type Policy = serde_json::Map<String, Value>;

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE)
}

/// For applying: absent is empty, and so is corrupt (said in the log) — a
/// broken file must never stop the engine from starting.
pub fn read(config_dir: &Path) -> Policy {
    let p = path(config_dir);
    let Ok(bytes) = std::fs::read(&p) else { return Policy::new() };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(m)) => m,
        _ => {
            applog("error", &format!("{} is not a JSON object; ignoring it, so env and user config decide", p.display()));
            Policy::new()
        }
    }
}

/// Locked read-modify-write. `f` answers whether it changed anything; nothing
/// changed is no write at all.
fn edit(config_dir: &Path, f: impl FnOnce(&mut Policy) -> bool) -> Result<bool, String> {
    let p = path(config_dir);
    worktrees_core::config::edit_user_config(&p, |text| {
        let mut map = if text.trim().is_empty() {
            Policy::new()
        } else {
            match serde_json::from_str::<Value>(text) {
                Ok(Value::Object(m)) => m,
                Ok(_) => return Err(format!("{} is not a JSON object; not overwriting it", p.display())),
                Err(e) => return Err(format!("{} is not valid JSON ({e}); not overwriting it", p.display())),
            }
        };
        if !f(&mut map) {
            return Ok(None);
        }
        let mut out = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
        out.push('\n');
        Ok(Some(out))
    })
}

/// Persist one key. The caller has already validated the value.
pub fn set(config_dir: &Path, key: &str, value: impl Into<Value>) -> Result<(), String> {
    set_many(config_dir, &[(key, value.into())])
}

/// Persist several keys in ONE locked write, so a reader never sees half of a
/// change. The caller has already validated every value.
pub fn set_many(config_dir: &Path, kv: &[(&str, Value)]) -> Result<(), String> {
    edit(config_dir, |m| {
        let mut changed = false;
        for (key, v) in kv {
            if m.get(*key) != Some(v) {
                m.insert((*key).into(), v.clone());
                changed = true;
            }
        }
        changed
    })
    .map(|_| ())
}

/// The three policies only the engine's own loops read: the poll's
/// auto-fetch cadence, and how a dock shell is saved and started. Unlike the
/// two agent policies they have no env or user-config tier, so a key the file
/// lacks — or holds a value this build does not know — means the default
/// here, which is what the engine ran with before any of this was persisted
/// (and what `settings.ts`'s DEFAULTS were).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct EngineOpts {
    pub fetch_interval_min: u64,
    pub term_persist_scrollback: bool,
    pub term_per_tab_history: bool,
}

impl Default for EngineOpts {
    fn default() -> Self {
        EngineOpts { fetch_interval_min: 0, term_persist_scrollback: true, term_per_tab_history: true }
    }
}

impl EngineOpts {
    pub fn from_policy(m: &Policy) -> Self {
        let d = EngineOpts::default();
        EngineOpts {
            fetch_interval_min: known(m, FETCH_INTERVAL_MIN, fetch_interval).unwrap_or(d.fetch_interval_min),
            term_persist_scrollback: known(m, TERM_PERSIST_SCROLLBACK, Value::as_bool).unwrap_or(d.term_persist_scrollback),
            term_per_tab_history: known(m, TERM_PER_TAB_HISTORY, Value::as_bool).unwrap_or(d.term_per_tab_history),
        }
    }
}

fn fetch_interval(v: &Value) -> Option<u64> {
    v.as_u64().filter(|m| FETCH_INTERVALS_MIN.contains(m))
}

/// Where [`EngineOpts`] live while the engine runs: the statics the poll
/// (`ops::FETCH_INTERVAL_SECS`) and the shell path (`term::PERSIST_SCROLLBACK`,
/// `term::PER_TAB_HISTORY`) read. Boot and the setters are their only writers.
/// A seam so this file's tests apply to atomics of their own: term.rs's tests
/// read `PERSIST_SCROLLBACK` in parallel, and a test here that turned it off
/// under them would fail one of theirs for no reason of its own.
pub struct Statics<'a> {
    pub fetch_secs: &'a AtomicU64,
    pub persist_scrollback: &'a AtomicBool,
    pub per_tab_history: &'a AtomicBool,
}

impl Statics<'_> {
    fn store(&self, o: &EngineOpts) {
        self.fetch_secs.store(o.fetch_interval_min.saturating_mul(60), Ordering::Relaxed);
        self.persist_scrollback.store(o.term_persist_scrollback, Ordering::Relaxed);
        self.per_tab_history.store(o.term_per_tab_history, Ordering::Relaxed);
    }

    fn load(&self) -> EngineOpts {
        EngineOpts {
            fetch_interval_min: self.fetch_secs.load(Ordering::Relaxed) / 60,
            term_persist_scrollback: self.persist_scrollback.load(Ordering::Relaxed),
            term_per_tab_history: self.per_tab_history.load(Ordering::Relaxed),
        }
    }
}

/// The engine's own statics.
pub fn engine_statics() -> Statics<'static> {
    Statics {
        fetch_secs: &crate::ops::FETCH_INTERVAL_SECS,
        persist_scrollback: &crate::term::PERSIST_SCROLLBACK,
        per_tab_history: &crate::term::PER_TAB_HISTORY,
    }
}

/// What the engine runs with now. The EFFECTIVE value — the file's, or the
/// default where the file has none — so it is what Settings renders, and NOT
/// a record of what the host holds: the frontend copies it into ui-state.json
/// only after a set the host accepted (`settings.ts`).
pub fn engine_opts() -> EngineOpts {
    engine_statics().load()
}

/// Settings → Behavior → Auto-fetch origin. Persists, then applies; a cadence
/// Settings does not offer is refused before anything is written.
pub fn set_fetch_interval(config_dir: &Path, mins: u64) -> Result<(), String> {
    set_fetch_interval_in(config_dir, mins, &engine_statics())
}

fn set_fetch_interval_in(config_dir: &Path, mins: u64, s: &Statics) -> Result<(), String> {
    if !FETCH_INTERVALS_MIN.contains(&mins) {
        return Err(format!("unknown auto-fetch interval {mins} (expected 0, 5, 15 or 60 minutes)"));
    }
    set(config_dir, FETCH_INTERVAL_MIN, mins)?;
    s.fetch_secs.store(mins * 60, Ordering::Relaxed);
    Ok(())
}

/// Settings → Terminal → "What a shell tab remembers". A PATCH: each switch
/// is its own act and sends only itself, so one client turning one switch
/// never re-sends a stale copy of the other over another client's change.
/// Naming neither is refused, as a value this build does not know would be.
pub fn set_term_history(config_dir: &Path, persist_scrollback: Option<bool>, per_tab_history: Option<bool>) -> Result<(), String> {
    set_term_history_in(config_dir, persist_scrollback, per_tab_history, &engine_statics())
}

fn set_term_history_in(
    config_dir: &Path,
    persist_scrollback: Option<bool>,
    per_tab_history: Option<bool>,
    s: &Statics,
) -> Result<(), String> {
    let mut kv: Vec<(&str, Value)> = Vec::new();
    if let Some(on) = persist_scrollback {
        kv.push((TERM_PERSIST_SCROLLBACK, on.into()));
    }
    if let Some(on) = per_tab_history {
        kv.push((TERM_PER_TAB_HISTORY, on.into()));
    }
    if kv.is_empty() {
        return Err("nothing to set: name persist_scrollback, per_tab_history or both".into());
    }
    set_many(config_dir, &kv)?;
    if let Some(on) = persist_scrollback {
        s.persist_scrollback.store(on, Ordering::Relaxed);
    }
    if let Some(on) = per_tab_history {
        s.per_tab_history.store(on, Ordering::Relaxed);
    }
    Ok(())
}

/// The file into the engine: the agent policies into core's overrides, the
/// engine policies into the statics. A key that is absent, or names a value
/// this build does not know, CLEARS its override, so the next tier decides —
/// for the engine policies, the default.
pub fn apply(config_dir: &Path) {
    apply_in(config_dir, &engine_statics())
}

fn apply_in(config_dir: &Path, s: &Statics) {
    let m = read(config_dir);
    let codex = known(&m, CODEX_PERMISSIONS, |v| v.as_str().and_then(Permissions::parse));
    codex::set_permissions_override(codex);
    let pi = known(&m, PI_PROJECT_TRUST, |v| v.as_str().and_then(PiTrust::parse));
    trust::set_pi_trust_override(pi);
    let engine = EngineOpts::from_policy(&m);
    s.store(&engine);
    applog(
        "info",
        &format!(
            "host-policy: codex_permissions={} pi_project_trust={} fetch_interval_min={} term_persist_scrollback={} term_per_tab_history={}",
            codex.map_or("(env/config)", Permissions::as_str),
            pi.map_or("(env/config)", PiTrust::as_str),
            engine.fetch_interval_min,
            engine.term_persist_scrollback,
            engine.term_per_tab_history,
        ),
    );
}

fn known<T>(m: &Policy, key: &str, parse: impl Fn(&Value) -> Option<T>) -> Option<T> {
    let raw = m.get(key)?;
    let t = parse(raw);
    if t.is_none() {
        applog("warn", &format!("host-policy: {key} = {raw} is not a value this build knows; ignoring it"));
    }
    t
}

/// The one-time copy from `ui-state.json`, where all five lived before they
/// were host policy.
///
/// **When:** inside `boot`, which `worktrees serve` (`cmd_serve`) calls once
/// per start, after the one-engine refusal and before its listener binds — so
/// before any client can connect, and never because one did.
///
/// **What it reads:** `<config_dir>/ui-state.json` (the frontend's whole-blob
/// settings, in the same config dir as this file), and in it only the five
/// keys, under the names they have here.
///
/// **What it copies:** a key only when THIS file lacks it, and only a value
/// this build knows that is NOT the default. The frontend has written every
/// key, chosen or not, so a default there is almost always just the default;
/// for the agent policies copying it would make it a host setting that
/// outranks env and user config, and for the engine policies it would change
/// nothing but the file. So "once" is per key: the first boot that finds the
/// key missing here and chosen there. Never overwrites; never writes
/// `ui-state.json`. Returns what it copied.
///
/// The frontend still keeps all five there, as a copy of host policy for an
/// older build to find after a downgrade (`settings.ts`), written only once a
/// set the host accepted has put the key HERE. So "only where this file lacks
/// it" is also what stops this build reading its own copy back; keep it that
/// way.
pub fn migrate_from_ui_state(config_dir: &Path) -> Vec<&'static str> {
    let ui = std::fs::read(config_dir.join("ui-state.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let Some(ui) = ui else { return Vec::new() };
    let old = |key: &str| ui.get(key);
    // The defaults are settings.ts's DEFAULTS, which are also core's and
    // `EngineOpts::default`.
    let d = EngineOpts::default();
    let mut want: Vec<(&'static str, Value)> = Vec::new();
    if let Some(p) = old(CODEX_PERMISSIONS).and_then(Value::as_str).and_then(Permissions::parse).filter(|p| *p != Permissions::AutoReview) {
        want.push((CODEX_PERMISSIONS, p.as_str().into()));
    }
    if let Some(t) = old(PI_PROJECT_TRUST).and_then(Value::as_str).and_then(PiTrust::parse).filter(|t| *t != PiTrust::Never) {
        want.push((PI_PROJECT_TRUST, t.as_str().into()));
    }
    if let Some(m) = old(FETCH_INTERVAL_MIN).and_then(fetch_interval).filter(|m| *m != d.fetch_interval_min) {
        want.push((FETCH_INTERVAL_MIN, m.into()));
    }
    if let Some(on) = old(TERM_PERSIST_SCROLLBACK).and_then(Value::as_bool).filter(|on| *on != d.term_persist_scrollback) {
        want.push((TERM_PERSIST_SCROLLBACK, on.into()));
    }
    if let Some(on) = old(TERM_PER_TAB_HISTORY).and_then(Value::as_bool).filter(|on| *on != d.term_per_tab_history) {
        want.push((TERM_PER_TAB_HISTORY, on.into()));
    }
    if want.is_empty() {
        return Vec::new();
    }
    let mut copied = Vec::new();
    let r = edit(config_dir, |m| {
        for (key, value) in want {
            if !m.contains_key(key) {
                m.insert(key.into(), value);
                copied.push(key);
            }
        }
        !copied.is_empty()
    });
    match r {
        Ok(_) => {
            for key in &copied {
                applog("info", &format!("host-policy: copied {key} from ui-state.json (once; the host owns it from now on)"));
            }
            copied
        }
        Err(e) => {
            applog("error", &format!("host-policy: migration from ui-state.json failed: {e}"));
            Vec::new()
        }
    }
}

/// The engine's own act at startup: migrate, then apply. Called once by
/// `cmd_serve`, before any client can connect — never from a command, so
/// nothing a client does can cause it.
pub fn boot(config_dir: &Path) {
    migrate_from_ui_state(config_dir);
    apply(config_dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, MutexGuard};
    use worktrees_core::codex::{self, Permissions};
    use worktrees_core::trust::{self, PiTrust};

    /// Core's two overrides and the two env vars are process-global and the
    /// test runner is parallel, so every test that touches them holds this.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// SERIAL, plus a process put back the way the test found it — no
    /// override, no env var — at both ends, so no test inherits another's host.
    struct Globals(#[allow(dead_code)] MutexGuard<'static, ()>);
    impl Globals {
        fn take() -> Self {
            let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            reset();
            Globals(g)
        }
    }
    impl Drop for Globals {
        fn drop(&mut self) {
            reset();
        }
    }
    fn reset() {
        codex::set_permissions_override(None);
        trust::set_pi_trust_override(None);
        std::env::remove_var("WORKTREES_CODEX_PERMISSIONS");
        std::env::remove_var("WORKTREES_PI_PROJECT_TRUST");
    }

    /// A config dir of the test's own, so nothing here reaches the real app's.
    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!("wt-hostpolicy-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct NoHost;
    impl crate::Host for NoHost {
        fn emit(&self, _: &str, _: serde_json::Value) {}
    }
    fn ctx(dir: &Path) -> crate::Ctx {
        crate::Ctx::new(dir.to_path_buf(), Arc::new(NoHost))
    }

    fn on_disk(dir: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(path(dir)).expect("host-policy.json exists")).unwrap()
    }

    /// Engine statics of the test's own (see `Statics`), starting where the
    /// real ones do.
    struct Own(AtomicU64, AtomicBool, AtomicBool);
    impl Own {
        fn new() -> Self {
            let d = EngineOpts::default();
            Own(AtomicU64::new(d.fetch_interval_min * 60), AtomicBool::new(d.term_persist_scrollback), AtomicBool::new(d.term_per_tab_history))
        }
        fn statics(&self) -> Statics<'_> {
            Statics { fetch_secs: &self.0, persist_scrollback: &self.1, per_tab_history: &self.2 }
        }
        fn opts(&self) -> EngineOpts {
            self.statics().load()
        }
    }
    fn opts(fetch_interval_min: u64, term_persist_scrollback: bool, term_per_tab_history: bool) -> EngineOpts {
        EngineOpts { fetch_interval_min, term_persist_scrollback, term_per_tab_history }
    }

    /// The Settings click is the one act that changes host policy, and it has
    /// to reach DISK. Before, the value lived only in core's in-process
    /// override and in the client's own ui-state.json, so the host forgot it
    /// at every restart and learned it again from whichever client loaded.
    #[test]
    fn the_setter_persists_and_keeps_unknown_keys() {
        let _g = Globals::take();
        let d = Dir::new("persist");
        std::fs::write(path(&d.0), r#"{ "later_policy": { "x": 1 } }"#).unwrap();
        let c = ctx(&d.0);
        crate::project::set_codex_permissions(&c, "full".into()).unwrap();
        crate::project::set_pi_trust(&c, "ask".into()).unwrap();

        let v = on_disk(&d.0);
        assert_eq!(v["codex_permissions"], "full");
        assert_eq!(v["pi_project_trust"], "ask");
        assert_eq!(v["later_policy"], serde_json::json!({ "x": 1 }), "a key this build does not know survives");
        // …and the running engine uses it at once, as it always did.
        assert_eq!(codex::permissions(), Permissions::Full);
        assert_eq!(trust::pi_trust(), PiTrust::Ask);
    }

    /// A restart is a fresh process: no override, and the env tier set to
    /// something else so the answer cannot come from there by accident.
    /// `boot`, which the engine runs at startup, is what brings it back.
    #[test]
    fn a_fresh_engine_reads_the_persisted_value_at_boot() {
        let _g = Globals::take();
        let d = Dir::new("fresh");
        let c = ctx(&d.0);
        crate::project::set_codex_permissions(&c, "full".into()).unwrap();
        crate::project::set_pi_trust(&c, "ask".into()).unwrap();

        codex::set_permissions_override(None);
        trust::set_pi_trust_override(None);
        std::env::set_var("WORKTREES_CODEX_PERMISSIONS", "ask");
        std::env::set_var("WORKTREES_PI_PROJECT_TRUST", "never");
        assert_eq!(codex::permissions(), Permissions::Ask, "precondition: with the override cleared, env decides");

        boot(&d.0);
        assert_eq!(codex::permissions(), Permissions::Full);
        assert_eq!(trust::pi_trust(), PiTrust::Ask);
    }

    /// The order users already had: host setting > env > user config >
    /// default. The file takes the place the app's pushed value used to, and
    /// only for the keys it HOLDS — a key it lacks leaves env in charge rather
    /// than defaulting over it, and a value core does not know (a hand edit)
    /// falls through the same way core's own readers do.
    #[test]
    fn the_host_value_outranks_env_only_where_it_is_set() {
        let _g = Globals::take();
        let d = Dir::new("order");
        std::env::set_var("WORKTREES_CODEX_PERMISSIONS", "ask");
        std::env::set_var("WORKTREES_PI_PROJECT_TRUST", "ask");
        std::fs::write(path(&d.0), r#"{ "codex_permissions": "full" }"#).unwrap();

        boot(&d.0);
        assert_eq!(codex::permissions(), Permissions::Full, "the host value beats env");
        assert_eq!(trust::pi_trust(), PiTrust::Ask, "no host value for pi: env still decides");

        std::fs::write(path(&d.0), r#"{ "codex_permissions": "fulll" }"#).unwrap();
        boot(&d.0);
        assert_eq!(codex::permissions(), Permissions::Ask, "an unknown host value falls through to env");
    }

    /// A typo must not quietly widen (or narrow) what an agent may do, and a
    /// refused value must not reach the file either — or the next restart
    /// would apply what the click was told it could not have.
    #[test]
    fn an_unknown_value_is_refused_and_leaves_the_file_untouched() {
        let _g = Globals::take();
        let d = Dir::new("unknown");
        let before = "{\n  \"codex_permissions\": \"ask\",\n  \"pi_project_trust\": \"never\"\n}\n";
        std::fs::write(path(&d.0), before).unwrap();
        codex::set_permissions_override(Some(Permissions::Ask));
        trust::set_pi_trust_override(Some(PiTrust::Never));
        let c = ctx(&d.0);

        for bad in ["fulll", "", "FULL", "auto_review"] {
            assert!(crate::project::set_codex_permissions(&c, bad.into()).is_err(), "{bad:?} must be refused");
        }
        for bad in ["always", "", "Ask"] {
            assert!(crate::project::set_pi_trust(&c, bad.into()).is_err(), "{bad:?} must be refused");
        }
        assert_eq!(std::fs::read_to_string(path(&d.0)).unwrap(), before, "a refused value must not be written");
        assert_eq!(codex::permissions(), Permissions::Ask);
        assert_eq!(trust::pi_trust(), PiTrust::Never);
    }

    /// A file someone broke by hand stays human-repairable: the write is
    /// refused, says which file, and does not take effect in the engine.
    #[test]
    fn a_corrupt_file_is_refused_rather_than_clobbered() {
        let _g = Globals::take();
        let d = Dir::new("corrupt");
        std::env::set_var("WORKTREES_CODEX_PERMISSIONS", "ask");
        let broken = "{ \"codex_permissions\": \"ask\", ";
        std::fs::write(path(&d.0), broken).unwrap();

        let err = crate::project::set_codex_permissions(&ctx(&d.0), "full".into()).unwrap_err();
        assert!(err.contains(FILE), "the error names the file: {err}");
        assert_eq!(std::fs::read_to_string(path(&d.0)).unwrap(), broken);
        assert_eq!(codex::permissions(), Permissions::Ask, "a write that failed must not take effect");
    }

    /// Where the two lived before: the frontend's ui-state.json. The host
    /// copies a CHOSEN value across once, at boot, and only READS that file —
    /// the frontend writes it whole and would erase anything put there.
    #[test]
    fn the_migration_copies_a_chosen_value_once_and_never_overwrites() {
        let d = Dir::new("migrate");
        let ui = d.0.join("ui-state.json");
        let ui_text = r#"{"theme":"nord","codex_permissions":"full","pi_project_trust":"ask"}"#;
        std::fs::write(&ui, ui_text).unwrap();

        assert_eq!(migrate_from_ui_state(&d.0), vec![CODEX_PERMISSIONS, PI_PROJECT_TRUST]);
        let v = on_disk(&d.0);
        assert_eq!(v["codex_permissions"], "full");
        assert_eq!(v["pi_project_trust"], "ask");
        assert_eq!(std::fs::read_to_string(&ui).unwrap(), ui_text, "ui-state.json is read, never written");

        // Once: the next boot finds the host value and leaves it, whatever
        // the client's copy says by then.
        std::fs::write(&ui, r#"{"codex_permissions":"ask","pi_project_trust":"never"}"#).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        let v = on_disk(&d.0);
        assert_eq!((v["codex_permissions"].as_str(), v["pi_project_trust"].as_str()), (Some("full"), Some("ask")));

        // …and a value the click set is never overwritten by a stale copy —
        // which is also what keeps the frontend's downgrade copy of host
        // policy in ui-state.json (`settings.ts`) from ever being read back.
        set(&d.0, CODEX_PERMISSIONS, "ask").unwrap();
        std::fs::write(&ui, r#"{"codex_permissions":"full"}"#).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        assert_eq!(on_disk(&d.0)["codex_permissions"], "ask");
    }

    /// The frontend writes every key whether or not it was chosen, so a
    /// default there is almost always just the default. Copying it would
    /// make it a host setting that outranks env and user config — a
    /// precedence change nobody asked for. Nothing to copy writes no file.
    #[test]
    fn the_migration_copies_no_default_and_no_garbage() {
        let d = Dir::new("migrate-defaults");
        let ui = d.0.join("ui-state.json");
        std::fs::write(
            &ui,
            r#"{"codex_permissions":"auto-review","pi_project_trust":"never","fetch_interval_min":0,"term_persist_scrollback":true,"term_per_tab_history":true}"#,
        )
        .unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        assert!(!path(&d.0).exists(), "a default must not become a host setting");

        std::fs::write(
            &ui,
            r#"{"codex_permissions":"fulll","pi_project_trust":7,"fetch_interval_min":7,"term_persist_scrollback":"no","term_per_tab_history":0}"#,
        )
        .unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        assert!(!path(&d.0).exists());

        std::fs::write(&ui, r#"{"fetch_interval_min":"15","term_persist_scrollback":null}"#).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        assert!(!path(&d.0).exists());

        std::fs::write(&ui, "{ not json").unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        std::fs::remove_file(&ui).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty(), "a fresh install has no ui-state.json");
        assert!(!path(&d.0).exists());
    }

    /// The other three host policies: the auto-fetch cadence and the two
    /// dock-shell switches. The click has to reach DISK and then the engine,
    /// in that order. Before, the page pushed them into the statics from its
    /// own ui-state.json on every load, so the host held them only in memory
    /// and learned them again from whichever client loaded last.
    #[test]
    fn the_engine_policy_setters_persist_then_apply() {
        let d = Dir::new("engine-persist");
        std::fs::write(path(&d.0), r#"{ "later_policy": [1] }"#).unwrap();
        let own = Own::new();

        set_fetch_interval_in(&d.0, 15, &own.statics()).unwrap();
        assert_eq!(on_disk(&d.0)["fetch_interval_min"], 15);
        assert_eq!(own.0.load(Ordering::Relaxed), 900, "applied, in the seconds the poll reads");

        // A patch: the switch the call names, and nothing else.
        set_term_history_in(&d.0, Some(false), None, &own.statics()).unwrap();
        let v = on_disk(&d.0);
        assert_eq!(v["term_persist_scrollback"], false);
        assert!(v.get("term_per_tab_history").is_none(), "a switch the call did not name is not written: {v}");
        assert_eq!(own.opts(), opts(15, false, true));

        set_term_history_in(&d.0, None, Some(false), &own.statics()).unwrap();
        let v = on_disk(&d.0);
        assert_eq!((v["term_persist_scrollback"].as_bool(), v["term_per_tab_history"].as_bool()), (Some(false), Some(false)));
        assert_eq!(own.opts(), opts(15, false, false));

        // Back to the defaults is a choice like any other, and is kept.
        set_fetch_interval_in(&d.0, 0, &own.statics()).unwrap();
        set_term_history_in(&d.0, Some(true), Some(true), &own.statics()).unwrap();
        let v = on_disk(&d.0);
        assert_eq!(v["fetch_interval_min"], 0);
        assert_eq!((v["term_persist_scrollback"].as_bool(), v["term_per_tab_history"].as_bool()), (Some(true), Some(true)));
        assert_eq!(own.opts(), EngineOpts::default());
        assert_eq!(v["later_policy"], serde_json::json!([1]), "a key this build does not know survives");
    }

    /// A restart is a fresh process, whose statics start at the defaults.
    /// `boot` is what brings the host's values back — and a key the file
    /// lacks is the default, whatever the statics held before.
    #[test]
    fn a_fresh_engine_loads_the_engine_policies_at_boot() {
        let _g = Globals::take();
        let d = Dir::new("engine-boot");
        let before = Own::new();
        set_fetch_interval_in(&d.0, 60, &before.statics()).unwrap();
        set_term_history_in(&d.0, Some(false), Some(false), &before.statics()).unwrap();

        let fresh = Own::new();
        assert_eq!(fresh.opts(), EngineOpts::default(), "precondition: a fresh engine starts at the defaults");
        apply_in(&d.0, &fresh.statics());
        assert_eq!(fresh.opts(), opts(60, false, false));

        std::fs::write(path(&d.0), r#"{ "term_persist_scrollback": false }"#).unwrap();
        apply_in(&d.0, &fresh.statics());
        assert_eq!(fresh.opts(), opts(0, false, true), "an absent key is the default, not what the engine held");
    }

    /// Settings offers four cadences and two booleans. Anything else — a
    /// typo, a client that is not ours — is refused before the file is
    /// touched and does not take effect; a value only a hand edit can put in
    /// the file falls back to the default rather than being guessed at; and
    /// a file someone broke by hand stays theirs to repair.
    #[test]
    fn unknown_engine_policy_values_are_refused_and_never_applied() {
        let d = Dir::new("engine-unknown");
        let before = "{\n  \"fetch_interval_min\": 15,\n  \"term_persist_scrollback\": false\n}\n";
        std::fs::write(path(&d.0), before).unwrap();
        let c = ctx(&d.0);
        let own = Own::new();
        own.statics().store(&opts(15, false, true));

        for bad in [1, 7, 10, 30, 90, u64::MAX] {
            assert!(crate::ops::set_fetch_interval(&c, bad).is_err(), "{bad} must be refused");
            assert!(set_fetch_interval_in(&d.0, bad, &own.statics()).is_err());
        }
        assert!(crate::prefs::set_term_history_opts(&c, None, None).is_err(), "a set that names nothing is refused");
        assert!(set_term_history_in(&d.0, None, None, &own.statics()).is_err());
        assert_eq!(std::fs::read_to_string(path(&d.0)).unwrap(), before, "a refused value must not be written");
        assert_eq!(own.opts(), opts(15, false, true), "…nor applied");

        let hand = r#"{ "fetch_interval_min": 7, "term_persist_scrollback": "no", "term_per_tab_history": null }"#;
        std::fs::write(path(&d.0), hand).unwrap();
        assert_eq!(EngineOpts::from_policy(&read(&d.0)), EngineOpts::default());
        std::fs::write(path(&d.0), r#"{ "fetch_interval_min": "15" }"#).unwrap();
        assert_eq!(EngineOpts::from_policy(&read(&d.0)), EngineOpts::default());

        let broken = "{ \"fetch_interval_min\": 15, ";
        std::fs::write(path(&d.0), broken).unwrap();
        let err = set_fetch_interval_in(&d.0, 5, &own.statics()).unwrap_err();
        assert!(err.contains(FILE), "the error names the file: {err}");
        assert!(set_term_history_in(&d.0, Some(true), None, &own.statics()).is_err());
        assert_eq!(std::fs::read_to_string(path(&d.0)).unwrap(), broken);
        assert_eq!(own.opts(), opts(15, false, true), "a write that failed must not take effect");
    }

    /// The three came from the same ui-state.json as the agent policies, and
    /// move the same way: a CHOSEN value, once, at boot, never over a value
    /// the host already holds, and that file is only read.
    #[test]
    fn the_migration_copies_chosen_engine_policies_once_and_never_overwrites() {
        let d = Dir::new("engine-migrate");
        let ui = d.0.join("ui-state.json");
        let ui_text = r#"{"theme":"nord","fetch_interval_min":15,"term_persist_scrollback":false,"term_per_tab_history":false}"#;
        std::fs::write(&ui, ui_text).unwrap();

        assert_eq!(migrate_from_ui_state(&d.0), vec![FETCH_INTERVAL_MIN, TERM_PERSIST_SCROLLBACK, TERM_PER_TAB_HISTORY]);
        assert_eq!(EngineOpts::from_policy(&read(&d.0)), opts(15, false, false));
        assert_eq!(std::fs::read_to_string(&ui).unwrap(), ui_text, "ui-state.json is read, never written");

        // Once: the next boot finds the host's values and leaves them.
        std::fs::write(&ui, r#"{"fetch_interval_min":60,"term_persist_scrollback":true,"term_per_tab_history":false}"#).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        assert_eq!(EngineOpts::from_policy(&read(&d.0)), opts(15, false, false));

        // A key the host LACKS is still copied while one it holds is not:
        // "once" is per key.
        std::fs::write(path(&d.0), r#"{ "fetch_interval_min": 5 }"#).unwrap();
        std::fs::write(&ui, r#"{"fetch_interval_min":60,"term_persist_scrollback":false}"#).unwrap();
        assert_eq!(migrate_from_ui_state(&d.0), vec![TERM_PERSIST_SCROLLBACK]);
        assert_eq!(EngineOpts::from_policy(&read(&d.0)), opts(5, false, true));

        // And a value a click set is never overwritten by the client's
        // (possibly stale) downgrade copy.
        let own = Own::new();
        set_term_history_in(&d.0, None, Some(true), &own.statics()).unwrap();
        std::fs::write(&ui, r#"{"term_per_tab_history":false}"#).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        assert_eq!(on_disk(&d.0)["term_per_tab_history"], true);
    }
}
