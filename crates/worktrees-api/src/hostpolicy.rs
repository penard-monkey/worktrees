//! Host policy: the settings that decide what an agent THIS HOST launches may
//! do, kept on the host (`host-policy.json` in the config dir) and never in a
//! client's own settings.
//!
//! Two so far, both OWASP review M2 (`docs/proposals/server-edition.md`,
//! "Queued"): Settings → Codex → Permissions (`codex_permissions`) and
//! Settings → pi → Project trust (`pi_project_trust`). They used to live in
//! the frontend's `ui-state.json` and be pushed into core's process-global
//! overrides on every hydration. Under one engine per host that meant the
//! client that loaded LAST decided what Codex may run in every lane the host
//! launches — opening a page was enough to widen it.
//!
//! The rules:
//! - **Only an explicit act writes it**: `set_codex_permissions` /
//!   `set_pi_trust` in `project`, which only the Settings click calls.
//!   `app/scripts/hostpolicy-check.mjs` holds the frontend to that.
//! - **Only the ENGINE reads it.** `boot` loads it into core's overrides at
//!   startup; the CLI and MCP-launched lanes never call `boot`, so they keep
//!   env + user config exactly as before. The order is unchanged: this file >
//!   `$WORKTREES_CODEX_PERMISSIONS` / `$WORKTREES_PI_PROJECT_TRUST` > user
//!   config > default.
//! - **Never `ui-state.json`.** The frontend writes that whole-blob, so a
//!   backend write there is erased by the next settings save. The migration
//!   below READS it, once per key, and nothing here writes it.
//! - Written through core's `config::edit_user_config` (one writer at a time,
//!   temp + rename, a symlink edited through, the mode kept). A key this build
//!   does not know survives a write; a corrupt file is refused rather than
//!   clobbered; an unknown VALUE, which only a hand edit can produce, falls
//!   through to the next tier the way core's own readers treat one.

use std::path::{Path, PathBuf};

use serde_json::Value;
use worktrees_core::codex::{self, Permissions};
use worktrees_core::trust::{self, PiTrust};

use crate::log::log as applog;

pub const FILE: &str = "host-policy.json";
pub const CODEX_PERMISSIONS: &str = "codex_permissions";
pub const PI_PROJECT_TRUST: &str = "pi_project_trust";

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
pub fn set(config_dir: &Path, key: &str, value: &str) -> Result<(), String> {
    edit(config_dir, |m| {
        let v = Value::String(value.into());
        if m.get(key) == Some(&v) {
            return false;
        }
        m.insert(key.into(), v);
        true
    })
    .map(|_| ())
}

/// The file into core's overrides. A key that is absent, or names a value
/// core does not know, CLEARS its override, so the next tier decides.
pub fn apply(config_dir: &Path) {
    let m = read(config_dir);
    let codex = known(&m, CODEX_PERMISSIONS, Permissions::parse);
    codex::set_permissions_override(codex);
    let pi = known(&m, PI_PROJECT_TRUST, PiTrust::parse);
    trust::set_pi_trust_override(pi);
    applog(
        "info",
        &format!(
            "host-policy: codex_permissions={} pi_project_trust={}",
            codex.map_or("(env/config)", Permissions::as_str),
            pi.map_or("(env/config)", PiTrust::as_str),
        ),
    );
}

fn known<T>(m: &Policy, key: &str, parse: fn(&str) -> Option<T>) -> Option<T> {
    let raw = m.get(key)?;
    let t = raw.as_str().and_then(parse);
    if t.is_none() {
        applog("warn", &format!("host-policy: {key} = {raw} is not a value this build knows; ignoring it"));
    }
    t
}

/// The one-time copy from `ui-state.json`, where these lived before they were
/// host policy. A key is copied only when this file LACKS it and the old
/// value is known and NOT the default. The frontend writes every key,
/// chosen or not, so a default there is almost always just the default, and
/// copying it would turn it into a host setting that outranks env and user
/// config. Never overwrites; never writes `ui-state.json`. Returns what it
/// copied.
///
/// The frontend still keeps both keys there, as a copy of host policy for an
/// older build to find after a downgrade (`settings.ts`), written only once a
/// click has put the key HERE. So "only where this file lacks it" is also what
/// stops this build reading its own copy back; keep it that way.
pub fn migrate_from_ui_state(config_dir: &Path) -> Vec<&'static str> {
    let ui = std::fs::read(config_dir.join("ui-state.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let Some(ui) = ui else { return Vec::new() };
    let old = |key: &str| ui.get(key).and_then(Value::as_str);
    // The defaults are settings.ts's DEFAULTS, which are also core's.
    let mut want: Vec<(&'static str, &'static str)> = Vec::new();
    if let Some(p) = old(CODEX_PERMISSIONS).and_then(Permissions::parse).filter(|p| *p != Permissions::AutoReview) {
        want.push((CODEX_PERMISSIONS, p.as_str()));
    }
    if let Some(t) = old(PI_PROJECT_TRUST).and_then(PiTrust::parse).filter(|t| *t != PiTrust::Never) {
        want.push((PI_PROJECT_TRUST, t.as_str()));
    }
    if want.is_empty() {
        return Vec::new();
    }
    let mut copied = Vec::new();
    let r = edit(config_dir, |m| {
        for (key, value) in want {
            if !m.contains_key(key) {
                m.insert(key.into(), Value::String(value.into()));
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

/// The engine's own act at startup: migrate, then apply. Called from the
/// app's setup, before any client can connect — never from a command, so
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
        std::fs::write(&ui, r#"{"codex_permissions":"auto-review","pi_project_trust":"never"}"#).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        assert!(!path(&d.0).exists(), "a default must not become a host setting");

        std::fs::write(&ui, r#"{"codex_permissions":"fulll","pi_project_trust":7}"#).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        assert!(!path(&d.0).exists());

        std::fs::write(&ui, "{ not json").unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty());
        std::fs::remove_file(&ui).unwrap();
        assert!(migrate_from_ui_state(&d.0).is_empty(), "a fresh install has no ui-state.json");
        assert!(!path(&d.0).exists());
    }
}
