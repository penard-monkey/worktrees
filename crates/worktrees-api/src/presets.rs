//! Settings → Agent policy → Preset (docs/proposals/settings-reorg.md §5): the
//! two rows over core's `policy_presets` — a READ of which preset this host is
//! on and what moved, and the console-only APPLY.
//!
//! Apply calls each value's existing setter, in [`KEYS`] order, exactly as a
//! Settings click would: host policy through `hostpolicy` (with its in-process
//! override), the guard through `agent-guidance.json`, the rest through
//! config.toml. No setter learns about presets. The name is recorded in
//! `host-policy.json` only when EVERY step took; a step that fails leaves the
//! steps before it in effect, unsets the name, and says which step failed —
//! the page then shows Custom and the marker, which is the truth.
//!
//! [`KEYS`]: worktrees_core::policy_presets::KEYS

use serde::Serialize;
use serde_json::{json, Value};
use worktrees_core::policy_presets::{self, Applied, KEYS};

use crate::hostpolicy;
use crate::log::log as applog;
use crate::Ctx;

/// The status, plus what decided the two host-policy values (§5.4: an
/// environment value is never pretended to be the preset's).
#[derive(Serialize)]
pub struct PresetStatus {
    #[serde(flatten)]
    pub status: policy_presets::Status,
    pub sources: serde_json::Map<String, Value>,
}

fn applied(ctx: &Ctx) -> Option<Applied> {
    hostpolicy::read(&ctx.config_dir)
        .get(hostpolicy::POLICY_PRESET)
        .and_then(|v| serde_json::from_value::<Applied>(v.clone()).ok())
}

pub fn status_now(ctx: &Ctx) -> PresetStatus {
    let mut sources = serde_json::Map::new();
    sources.insert("codex_access".into(), json!(worktrees_core::codex::permissions_origin().1));
    sources.insert("pi_trust".into(), json!(worktrees_core::trust::pi_trust_origin().1));
    PresetStatus { status: policy_presets::status_from(applied(ctx), policy_presets::current()), sources }
}

pub fn policy_preset_status(ctx: &Ctx) -> Result<PresetStatus, String> {
    Ok(status_now(ctx))
}

/// The user's act at the host: apply `name` — every key, or only `keys` (the
/// page's "back to <preset>" on one control, which leaves the recorded name
/// alone). `name = "custom"` with no keys only forgets the recorded name
/// ("Keep as custom"). `keys` are checked against the preset's own list; a
/// name or key this build does not know is refused before anything is set.
pub fn apply_policy_preset(ctx: &Ctx, name: String, keys: Option<Vec<String>>) -> Result<PresetStatus, String> {
    if name == "custom" && keys.is_none() {
        hostpolicy::remove(&ctx.config_dir, hostpolicy::POLICY_PRESET)?;
        applog("info", "policy preset: kept as custom");
        return Ok(status_now(ctx));
    }
    let Some(p) = policy_presets::by_name(&name) else {
        return Err(format!("unknown policy preset '{name}' (expected cautious, balanced or autonomous)"));
    };
    if let Some(bad) = keys.iter().flatten().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("a policy preset sets no '{bad}' (it sets {})", KEYS.join(", ")));
    }
    let want = |k: &str| keys.as_ref().is_none_or(|ks| ks.iter().any(|x| x == k));
    let v = &p.values;
    let mut done: Vec<&str> = Vec::new();
    let mut step = |k: &'static str, f: &mut dyn FnMut() -> Result<(), String>| -> Result<(), String> {
        if !want(k) {
            return Ok(());
        }
        f().map_err(|e| {
            let before = if done.is_empty() { "nothing was changed".to_string() } else { format!("{} took effect", done.join(", ")) };
            format!("{} preset: setting {k} failed ({e}); {before}", p.label)
        })?;
        done.push(k);
        Ok(())
    };
    let result = (|| -> Result<(), String> {
        step("codex_access", &mut || crate::project::set_codex_permissions(ctx, v.codex_access.clone()))?;
        step("main_guard", &mut || {
            let s = worktrees_core::guidance::settings();
            crate::project::set_agent_guidance(s.enabled, v.main_guard).map(|_| ())
        })?;
        step("pi_trust", &mut || crate::project::set_pi_trust(ctx, v.pi_trust.clone()))?;
        step("cross_project", &mut || crate::ops::set_cross_project(v.cross_project.clone()).map(|_| ()))?;
        // The gate and its threshold are one setter: a restore of either sends
        // the other as it stands now, so "back to preset" on one leaves the other.
        if want("quota_gate") || want("quota_weekly_pct") {
            let cur = worktrees_core::quota::user_policy().policy;
            let gate = if want("quota_gate") { v.quota_gate } else { cur.gate };
            let pct = if want("quota_weekly_pct") { v.quota_weekly_pct } else { cur.weekly_warn_pct };
            let k = if want("quota_gate") { "quota_gate" } else { "quota_weekly_pct" };
            step(k, &mut || crate::ops::set_quota_settings(gate, pct).map(|_| ()))?;
        }
        step("max_live_agents", &mut || {
            crate::ops::set_limits("max_live_agents".into(), v.max_live_agents).map(|_| ())
        })?;
        Ok(())
    })();
    match (&result, keys.is_none()) {
        (Ok(()), true) => {
            hostpolicy::set(&ctx.config_dir, hostpolicy::POLICY_PRESET, json!({ "name": p.name, "version": policy_presets::VERSION }))?;
            applog("info", &format!("policy preset: applied {}", p.name));
        }
        (Ok(()), false) => applog("info", &format!("policy preset: {} back to {}", keys.unwrap_or_default().join(", "), p.name)),
        (Err(e), whole) => {
            applog("error", &format!("apply_policy_preset: {e}"));
            if whole {
                // Half a preset is not that preset: the page says Custom.
                let _ = hostpolicy::remove(&ctx.config_dir, hostpolicy::POLICY_PRESET);
            }
        }
    }
    result.map(|()| status_now(ctx))
}
