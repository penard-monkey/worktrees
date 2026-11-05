//! System policy presets (docs/proposals/settings-reorg.md §5): Cautious,
//! Balanced, Autonomous — one word for "how much may my agents do", over the
//! values that already decide it, each in its own home. A preset is a SET of
//! those values and nothing more: applying one calls each value's existing
//! setter (`worktrees-api` `presets`), so no setter learns about presets, and
//! an older build that reads the values reads them as it always did.
//!
//! What a preset sets, and why exactly these (§5.2): Codex permissions, the
//! (main) branching guard, pi's project trust, cross-project reach, the plan
//! gate and its weekly threshold, the live-agent cap. NOT in a preset: the
//! heavy-job slots (machine-derived, not a trust question), the pi/Kimi
//! allowed-repo lists (per repo, additive), and the window's own settings
//! (`ai_auto_resume`, `chat_view` — a preset writes the HOST, and the blob is
//! the window's, a device's on a remote window).
//!
//! **Balanced is exactly the shipped defaults** (owner, 2026-11-04), so a
//! fresh install reads as Balanced and applying it changes nothing —
//! `balanced_is_exactly_the_shipped_defaults` pins it to each value's own
//! default. Cautious turns the guard on.
//!
//! [`VERSION`] moves when a preset's MEANING changes, so a later release can
//! say so (an offer fingerprinted on it) rather than silently.

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

/// The values a preset sets, in their own words and types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Values {
    /// `ask` / `auto-review` / `full` (`codex::Permissions`). Not named
    /// `codex_permissions`: that is the window blob's mirror key, which
    /// hostpolicy-check.mjs keeps the frontend from reading anywhere else.
    pub codex_access: String,
    /// `agent-guidance.json`'s `guard`.
    pub main_guard: bool,
    /// `never` / `ask` (`trust::PiTrust`).
    pub pi_trust: String,
    /// `off` / `read` / `full` (`reach::Level`).
    pub cross_project: String,
    pub quota_gate: bool,
    /// `None`: the provider's own warning.
    pub quota_weekly_pct: Option<u8>,
    /// `None`: no `[limits] max_live_agents`, i.e. this machine's default.
    pub max_live_agents: Option<u32>,
}

/// Every key a preset sets, in the order `apply` sets them.
pub const KEYS: [&str; 7] =
    ["codex_access", "main_guard", "pi_trust", "cross_project", "quota_gate", "quota_weekly_pct", "max_live_agents"];

impl Values {
    /// The keys whose values differ from `other`'s, in [`KEYS`] order.
    pub fn differs(&self, other: &Values) -> Vec<&'static str> {
        KEYS.into_iter()
            .filter(|k| match *k {
                "codex_access" => self.codex_access != other.codex_access,
                "main_guard" => self.main_guard != other.main_guard,
                "pi_trust" => self.pi_trust != other.pi_trust,
                "cross_project" => self.cross_project != other.cross_project,
                "quota_gate" => self.quota_gate != other.quota_gate,
                // The threshold means nothing while the gate is off, so it
                // differs only where the gate is on on both sides.
                "quota_weekly_pct" => self.quota_gate && other.quota_gate && self.quota_weekly_pct != other.quota_weekly_pct,
                "max_live_agents" => self.max_live_agents != other.max_live_agents,
                _ => false,
            })
            .collect()
    }
}

/// One system preset, with the two client-side defaults it suggests (§5.2's
/// last rows: the "Allow create and close tools" box on a Set up button, and
/// the scope a Pair sheet starts on). Those two are never stored; a window
/// reads them from the status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Preset {
    pub name: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    pub values: Values,
    pub mutations_default: bool,
    /// `observe` / `operate` — `devices::PRESETS` in the app.
    pub pair_scope: &'static str,
}

pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "cautious",
            label: "Cautious",
            summary: "Codex asks before each command, Claude may not branch in (main), pi skips repo resources, agents see only their own project and stop at 80% of a weekly plan, and two may be live at once.",
            values: Values {
                codex_access: "ask".into(),
                main_guard: true,
                pi_trust: "never".into(),
                cross_project: "off".into(),
                quota_gate: true,
                quota_weekly_pct: Some(80),
                max_live_agents: Some(2),
            },
            mutations_default: false,
            pair_scope: "observe",
        },
        Preset {
            name: "balanced",
            label: "Balanced",
            summary: "Codex's reviewer approves or refuses each command, pi skips repo resources, agents see only their own project and stop on a nearly spent plan, up to this machine's default number at once. Today's defaults.",
            values: Values {
                codex_access: "auto-review".into(),
                main_guard: false,
                pi_trust: "never".into(),
                cross_project: "off".into(),
                quota_gate: true,
                quota_weekly_pct: None,
                max_live_agents: None,
            },
            mutations_default: true,
            pair_scope: "observe",
        },
        Preset {
            name: "autonomous",
            label: "Autonomous",
            summary: "Codex runs with no prompts and no sandbox, pi asks about each repo's resources, agents may read your other projects, and no plan check stops a launch.",
            values: Values {
                codex_access: "full".into(),
                main_guard: false,
                pi_trust: "ask".into(),
                cross_project: "read".into(),
                quota_gate: false,
                quota_weekly_pct: None,
                max_live_agents: None,
            },
            mutations_default: true,
            pair_scope: "operate",
        },
    ]
}

pub fn by_name(name: &str) -> Option<Preset> {
    presets().into_iter().find(|p| p.name == name)
}

/// What the host applies now, read from each value's own home.
pub fn current() -> Values {
    let text = std::fs::read_to_string(crate::config::config_toml_path()).unwrap_or_default();
    let quota = crate::quota::policy_from(&text).policy;
    Values {
        codex_access: crate::codex::permissions().as_str().into(),
        main_guard: crate::guidance::settings().guard,
        pi_trust: crate::trust::pi_trust().as_str().into(),
        cross_project: crate::reach::user_level().as_str().into(),
        quota_gate: quota.gate,
        quota_weekly_pct: quota.weekly_warn_pct,
        max_live_agents: crate::guard::explicit_from(&text).max_live_agents,
    }
}

/// The preset last applied on this host (`host-policy.json`'s
/// `policy_preset`), kept by the api crate, which owns that file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub name: String,
    pub version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The preset this host is on — the one applied, else the one whose
    /// values all match (a fresh install is Balanced) — or `custom`.
    pub name: String,
    pub applied: Option<Applied>,
    pub version: u32,
    /// Applied under an older [`VERSION`]: the preset may mean something else now.
    pub stale: bool,
    /// The keys whose value is not `name`'s ("Balanced (modified: …)").
    pub modified: Vec<&'static str>,
    /// Every preset whose values all equal the current ones.
    pub matches: Vec<&'static str>,
    pub current: Values,
    pub presets: Vec<Preset>,
}

/// Pure, for the tests: the status of `current` against what was `applied`.
pub fn status_from(applied: Option<Applied>, current: Values) -> Status {
    let presets = presets();
    let matches: Vec<&'static str> = presets.iter().filter(|p| p.values == current).map(|p| p.name).collect();
    let known = applied.as_ref().and_then(|a| presets.iter().find(|p| p.name == a.name));
    let (name, modified) = match known {
        Some(p) => (p.name.to_string(), p.values.differs(&current)),
        None => (matches.first().map_or("custom", |m| *m).to_string(), Vec::new()),
    };
    Status {
        name,
        stale: applied.as_ref().is_some_and(|a| a.version < VERSION),
        applied,
        version: VERSION,
        modified,
        matches,
        current,
        presets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each value's OWN default, from its own reader — so a default changed in
    /// one module breaks this rather than leaving Balanced quietly wrong.
    fn shipped_defaults() -> Values {
        let quota = crate::quota::policy_from("").policy;
        Values {
            codex_access: crate::codex::permissions_from(None, None).as_str().into(),
            main_guard: crate::guidance::settings_from(None, None).guard,
            pi_trust: crate::trust::pi_trust_origin_from(None, None, None).0.as_str().into(),
            cross_project: crate::reach::level_from(None).as_str().into(),
            quota_gate: quota.gate,
            quota_weekly_pct: quota.weekly_warn_pct,
            max_live_agents: crate::guard::explicit_from("").max_live_agents,
        }
    }

    #[test]
    fn balanced_is_exactly_the_shipped_defaults() {
        assert_eq!(by_name("balanced").unwrap().values, shipped_defaults());
        let fresh = status_from(None, shipped_defaults());
        assert_eq!((fresh.name.as_str(), fresh.modified.len()), ("balanced", 0), "a fresh install reads as Balanced, unmodified");
    }

    #[test]
    fn every_value_a_preset_sets_is_one_its_setter_takes() {
        for p in presets() {
            let v = &p.values;
            assert!(crate::codex::Permissions::parse(&v.codex_access).is_some(), "{}", p.name);
            assert!(crate::trust::PiTrust::parse(&v.pi_trust).is_some(), "{}", p.name);
            assert!(crate::reach::Level::parse(&v.cross_project).is_some(), "{}", p.name);
            if let Some(n) = v.quota_weekly_pct {
                assert!((crate::quota::PCT_MIN..=crate::quota::PCT_MAX).contains(&n), "{}", p.name);
            }
            if let Some(n) = v.max_live_agents {
                assert!(crate::guard::LimitKey::MaxLiveAgents.check(n).is_ok(), "{}", p.name);
            }
            assert!(["observe", "operate"].contains(&p.pair_scope), "{}", p.name);
        }
        let names: Vec<_> = presets().iter().map(|p| p.name).collect();
        assert_eq!(names, ["cautious", "balanced", "autonomous"]);
        // No two presets are the same set, or "matches" could not say which.
        let all = presets();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(!a.values.differs(&b.values).is_empty(), "{} = {}", a.name, b.name);
            }
        }
    }

    #[test]
    fn the_status_names_the_preset_and_what_moved() {
        let cautious = by_name("cautious").unwrap().values;
        let applied = Some(Applied { name: "cautious".into(), version: VERSION });
        let st = status_from(applied.clone(), cautious.clone());
        assert_eq!((st.name.as_str(), st.modified.clone()), ("cautious", vec![]));
        let mut moved = cautious.clone();
        moved.codex_access = "auto-review".into();
        moved.max_live_agents = Some(4);
        let st = status_from(applied, moved.clone());
        assert_eq!(st.name, "cautious", "the applied name stays: the marker says what moved");
        assert_eq!(st.modified, vec!["codex_access", "max_live_agents"]);
        assert!(st.matches.is_empty());
        // Nothing applied and nothing matching: Custom.
        let st = status_from(None, moved);
        assert_eq!((st.name.as_str(), st.modified.len()), ("custom", 0));
        // Nothing applied but the values ARE a preset: that preset.
        assert_eq!(status_from(None, by_name("autonomous").unwrap().values).name, "autonomous");
        // An applied name this build does not know (a newer build's preset)
        // is not trusted: the status names whatever the values match.
        assert_eq!(status_from(Some(Applied { name: "paranoid".into(), version: 9 }), shipped_defaults()).name, "balanced");
        assert!(status_from(Some(Applied { name: "balanced".into(), version: 0 }), shipped_defaults()).stale);
    }

    #[test]
    fn a_threshold_with_the_gate_off_is_not_a_difference() {
        let mut a = by_name("autonomous").unwrap().values;
        let b = a.clone();
        a.quota_weekly_pct = Some(90);
        assert!(a.differs(&b).is_empty(), "the gate is off on both sides");
    }
}
