//! Cross-project reach — which OTHER registered projects a session may see,
//! and what a `<project>:<slug>` address names (`docs/proposals/cross-project.md`
//! §3, §5).
//!
//! **Off unless the user turns it on** (decided 2026-10-01). The level is the
//! user-tier `cross_project` key (`~/.config/worktrees/config.toml`): `off`
//! (the default), `read`, or `full`. A repo cannot set it — the key is in
//! `projcfg`'s `USER_ONLY_KEYS`. A profile can NARROW it, never widen it: its
//! launch passes `worktrees mcp --cross-project <level>`, and the effective
//! level is the lower of the two.
//!
//! **Identity comes from the registry, never from the caller.** A session's own
//! project name is the registry entry whose root equals its canonical main
//! root. An unregistered repo has no cross-project identity and gets no reach;
//! a private project gets none either (both directions, §5.2); neither does an
//! automation run.
//!
//! **Addresses.** A bare slug is the session's own project, byte for byte as
//! before. `<name>:<slug>` names a place in the registered project called
//! `<name>`. A branch name cannot contain `:` (git refuses it), so no slug
//! worktrees made has one; callers still try an exact LOCAL slug first, so a
//! hand-made directory with a `:` in its name stays reachable from home.

use crate::registry::{Entry, Registry};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off,
    Read,
    Full,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s.trim() {
            "off" => Some(Level::Off),
            "read" => Some(Level::Read),
            "full" => Some(Level::Full),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Read => "read",
            Level::Full => "full",
        }
    }
}

/// The config key, shared by every reader and by `USER_ONLY_KEYS`.
pub const KEY: &str = "cross_project";

/// Where a user turns reach on — said in every refusal that is "it is off".
pub const HOW_TO_ENABLE: &str =
    "cross-project reach is off; the user turns it on with `cross_project = \"read\"` in ~/.config/worktrees/config.toml";

/// The user's own setting. An unknown value is `off`: a typo must never be
/// read as consent.
pub fn user_level() -> Level {
    level_from(crate::config::user_cfg(KEY).as_deref())
}

pub fn level_from(raw: Option<&str>) -> Level {
    raw.and_then(Level::parse).unwrap_or(Level::Off)
}

/// Set the user's `cross_project` in `~/.config/worktrees/config.toml` — the
/// user's act only (Settings, by hand). Edits the one top-level line in place,
/// or adds it above the first `[table]`, leaving everything else byte for byte;
/// refuses rather than guess on a layout it cannot edit safely. Sessions read
/// it when they START, so a change reaches new sessions only.
pub fn set_user_level(level: Level) -> Result<(), String> {
    set_user_level_at(&crate::config::config_toml_path(), level)
}

pub fn set_user_level_at(path: &std::path::Path, level: Level) -> Result<(), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let line = format!("{KEY} = \"{}\"", level.as_str());
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    let mut in_table = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with('[') {
            if !placed {
                out.push(line.clone());
                placed = true;
            }
            in_table = true;
        }
        let is_ours = !in_table
            && t.strip_prefix(KEY).is_some_and(|rest| rest.trim_start().starts_with('='));
        if is_ours {
            if !placed {
                out.push(line.clone());
                placed = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !placed {
        out.push(line);
    }
    let mut new = out.join("\n");
    new.push('\n');
    let parsed: Result<std::collections::BTreeMap<String, toml::Value>, _> = toml::from_str(&new);
    match parsed.ok().and_then(|m| m.get(KEY).and_then(|v| v.as_str().map(str::to_string))) {
        Some(v) if v == level.as_str() => {}
        _ => return Err(format!("could not set {KEY} in {} safely — edit it by hand", path.display())),
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, new).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}

/// What one session may reach. Built ONCE, at server start, like
/// `--mutations` — a change reaches new sessions, not running ones.
#[derive(Clone, Debug)]
pub struct Reach {
    pub level: Level,
    /// This session's own registry entry, when its repo is registered.
    pub me: Option<Entry>,
    /// Why `level` is `Off`, for the refusal.
    pub why_off: Option<String>,
    registry: Registry,
}

/// Inputs to `Reach::new`, so a test can pass every one of them.
pub struct Inputs<'a> {
    pub main_root: &'a str,
    pub user: Level,
    /// `--cross-project` on the server's argv (a profile's narrowing).
    pub flag: Option<Level>,
    pub in_run: bool,
    pub registry: Registry,
}

impl Reach {
    pub fn new(i: Inputs) -> Reach {
        let me = i.registry.by_root(i.main_root).cloned();
        let level = i.user.min(i.flag.unwrap_or(Level::Full));
        let why_off = if i.in_run {
            Some("an automation run never reaches other projects".to_string())
        } else if level == Level::Off {
            Some(if i.user == Level::Off {
                HOW_TO_ENABLE.to_string()
            } else {
                "this session's profile turns cross-project reach off".to_string()
            })
        } else if me.is_none() {
            Some("this repository is not a registered project, so it has no cross-project name (`worktrees projects add`)".to_string())
        } else if me.as_ref().is_some_and(|m| m.private) {
            Some(format!("this project ('{}') is private, so it reaches no other project", me.as_ref().unwrap().name))
        } else {
            None
        };
        Reach { level: if why_off.is_some() { Level::Off } else { level }, me, why_off, registry: i.registry }
    }

    /// A reach that reaches nothing — no project, or a test.
    pub fn none() -> Reach {
        Reach { level: Level::Off, me: None, why_off: Some(HOW_TO_ENABLE.to_string()), registry: Registry::default() }
    }

    pub fn on(&self) -> bool {
        self.level != Level::Off
    }

    /// This session's own name, when it has one.
    pub fn my_name(&self) -> Option<&str> {
        self.me.as_ref().map(|e| e.name.as_str())
    }

    /// Every registry entry, in nav order — what `list_projects` walks. Empty
    /// when reach is off.
    pub fn entries(&self) -> &[Entry] {
        if self.on() { &self.registry.projects } else { &[] }
    }

    /// What `raw` names, or why it names nothing reachable. The caller has
    /// already tried `raw` as an exact local slug.
    pub fn parse(&self, raw: &str) -> Result<Addr, String> {
        let Some((name, slug)) = raw.split_once(':') else {
            return Ok(Addr::Local(raw.to_string()));
        };
        if slug.is_empty() {
            return Err(format!("{raw}: an address is <project>:<slug>, and the slug is missing"));
        }
        if name.is_empty() {
            return Err(format!("{raw}: an address is <project>:<slug>, and the project name is empty"));
        }
        // Self-qualification is always allowed — it is the caller's own
        // project, which needs no reach. Matched on the REGISTRY name.
        if self.my_name() == Some(name) {
            return Ok(Addr::Local(slug.to_string()));
        }
        if let Some(why) = &self.why_off {
            return Err(format!("{raw} names a place in another project, but {why}"));
        }
        let hits: Vec<&Entry> = self.registry.projects.iter().filter(|e| e.name == name).collect();
        match hits.as_slice() {
            [] => Err(format!("no registered project is named '{name}' (list_projects lists them)")),
            [e] if e.private => Err(format!("'{name}' is private; its places cannot be reached from other projects")),
            [e] => Ok(Addr::Foreign { entry: (*e).clone(), slug: slug.to_string() }),
            // Names are unique by construction; two means a hand-edited file.
            // Names only, never roots: one of them may be private, and an
            // error is not an exception to §3.4.
            many => Err(format!(
                "'{name}' is ambiguous: {} registered projects carry it. The user renames one with \
                 `worktrees projects rename`.",
                many.len()
            )),
        }
    }
}

/// What a nav drag types into a session (cross-project §6.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drop {
    /// Same project, into a Claude pane: the `@<server>:place://…` token,
    /// which the session's own server resolves as a resource (the caller
    /// builds it — it needs the session's server name).
    Token,
    /// Plain text (`mention::address`): another project's place, or any place
    /// into a Codex or pi pane, which has no `@`-resources.
    Address(String),
}

/// Decide a drop of `slug` from the project at `from_root` into a pane of
/// `provider` running in the project at `into_root`. Refused, with the reason,
/// whenever the receiving session could not reach the place: reach off, either
/// side unregistered or private. `level` is the USER's setting — a profile may
/// narrow a session further, and that session's server then refuses the
/// address by name, loudly, which is the point of an address over a token.
pub fn plan_drop(reg: &Registry, level: Level, from_root: &str, slug: &str, into_root: &str, provider: &str) -> Result<Drop, String> {
    let from = reg.by_root(from_root);
    if from_root == into_root {
        if provider == crate::provider::CLAUDE.id {
            return Ok(Drop::Token);
        }
        return Ok(Drop::Address(crate::mention::address(from.map(|e| e.name.as_str()), slug)));
    }
    if level == Level::Off {
        return Err("cross-project reach is off — turn it on in Settings → Agent guidance".into());
    }
    let Some(from) = from else {
        return Err("that place's project is not registered, so it has no name to address it by".into());
    };
    if from.private {
        return Err(format!("'{}' is private; its places cannot be referenced from other projects", from.name));
    }
    match reg.by_root(into_root) {
        None => Err("this session's project is not registered, so it reaches no other project".into()),
        Some(into) if into.private => Err(format!("this session's project ('{}') is private, so it reaches no other project", into.name)),
        Some(_) => Ok(Drop::Address(crate::mention::address(Some(&from.name), slug))),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Addr {
    /// A slug in the caller's own project.
    Local(String),
    /// A slug in another registered, reachable project.
    Foreign { entry: Entry, slug: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registry {
        let e = |root: &str, name: &str, private: bool| Entry { root: root.into(), name: name.into(), private };
        Registry {
            projects: vec![e("/w/alpha", "alpha", false), e("/w/beta", "beta", false), e("/w/client", "client", true)],
            ..Default::default()
        }
    }

    fn reach(root: &str, user: Level, flag: Option<Level>, in_run: bool) -> Reach {
        Reach::new(Inputs { main_root: root, user, flag, in_run, registry: reg() })
    }

    #[test]
    fn setting_the_level_edits_one_top_level_line_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("wt-reach-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("config.toml");
        // Absent file → created with just the line.
        set_user_level_at(&p, Level::Read).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "cross_project = \"read\"\n");
        // Existing layout: the top-level line is replaced in place, a same-named
        // key inside a table is left alone, and every other byte survives.
        let before = "ai_cmd = \"claude\"\ncross_project = \"full\"\n\n[trust]\npi = [\"/r\"]\n[other]\ncross_project = 1\n";
        std::fs::write(&p, before).unwrap();
        set_user_level_at(&p, Level::Off).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            before.replace("cross_project = \"full\"", "cross_project = \"off\"")
        );
        // No top-level line yet, tables present: added ABOVE the first table,
        // or it would land inside `[trust]`.
        std::fs::write(&p, "ai_cmd = \"x\"\n[trust]\npi = []\n").unwrap();
        set_user_level_at(&p, Level::Full).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "ai_cmd = \"x\"\ncross_project = \"full\"\n[trust]\npi = []\n");
        // A broken file is refused, never rewritten.
        std::fs::write(&p, "this is = = not toml\n").unwrap();
        assert!(set_user_level_at(&p, Level::Read).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "this is = = not toml\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_drop_is_a_token_only_within_a_project_into_claude() {
        let g = reg();
        assert_eq!(plan_drop(&g, Level::Off, "/w/alpha", "lane", "/w/alpha", "claude").unwrap(), Drop::Token);
        // Same project into Codex/pi: an address, and no reach needed.
        assert_eq!(plan_drop(&g, Level::Off, "/w/alpha", "lane", "/w/alpha", "codex").unwrap(), Drop::Address("place alpha:lane".into()));
        assert_eq!(plan_drop(&g, Level::Off, "/w/elsewhere", "lane", "/w/elsewhere", "pi").unwrap(), Drop::Address("place lane".into()));
        // Across projects: an address, into any harness, with reach on.
        for h in ["claude", "codex", "pi"] {
            assert_eq!(plan_drop(&g, Level::Read, "/w/beta", "lane", "/w/alpha", h).unwrap(), Drop::Address("place beta:lane".into()), "{h}");
        }
        let refuse = |lvl, from, into| plan_drop(&g, lvl, from, "lane", into, "claude").unwrap_err();
        assert!(refuse(Level::Off, "/w/beta", "/w/alpha").contains("Settings"));
        assert!(refuse(Level::Read, "/w/client", "/w/alpha").contains("'client' is private"));
        assert!(refuse(Level::Read, "/w/beta", "/w/client").contains("private"));
        assert!(refuse(Level::Read, "/w/nope", "/w/alpha").contains("not registered"));
        assert!(refuse(Level::Read, "/w/beta", "/w/nope").contains("not registered"));
    }

    #[test]
    fn off_by_default_and_an_unknown_value_is_off() {
        assert_eq!(level_from(None), Level::Off);
        assert_eq!(level_from(Some("yes")), Level::Off);
        assert_eq!(level_from(Some("read")), Level::Read);
        let r = reach("/w/alpha", Level::Off, None, false);
        assert!(!r.on());
        assert!(r.entries().is_empty());
        assert!(r.parse("beta:x").unwrap_err().contains("config.toml"));
    }

    #[test]
    fn a_profile_narrows_and_never_widens() {
        assert_eq!(reach("/w/alpha", Level::Full, Some(Level::Read), false).level, Level::Read);
        assert_eq!(reach("/w/alpha", Level::Read, Some(Level::Full), false).level, Level::Read);
        let r = reach("/w/alpha", Level::Read, Some(Level::Off), false);
        assert!(!r.on());
        assert!(r.parse("beta:x").unwrap_err().contains("profile"));
    }

    #[test]
    fn no_reach_from_a_run_an_unregistered_repo_or_a_private_project() {
        assert!(reach("/w/alpha", Level::Read, None, true).parse("beta:x").unwrap_err().contains("automation run"));
        assert!(reach("/w/elsewhere", Level::Read, None, false).parse("beta:x").unwrap_err().contains("not a registered"));
        assert!(reach("/w/client", Level::Read, None, false).parse("beta:x").unwrap_err().contains("private"));
    }

    #[test]
    fn addresses_resolve() {
        let r = reach("/w/alpha", Level::Read, None, false);
        assert_eq!(r.parse("lane").unwrap(), Addr::Local("lane".into()));
        // Self-qualified, even with reach on, is local.
        assert_eq!(r.parse("alpha:(main)").unwrap(), Addr::Local("(main)".into()));
        match r.parse("beta:lane-x").unwrap() {
            Addr::Foreign { entry, slug } => {
                assert_eq!((entry.name.as_str(), slug.as_str()), ("beta", "lane-x"));
            }
            other => panic!("{other:?}"),
        }
        assert!(r.parse("client:x").unwrap_err().contains("private"));
        assert!(r.parse("nobody:x").unwrap_err().contains("no registered project"));
        assert!(r.parse("beta:").unwrap_err().contains("slug is missing"));
        assert!(r.parse(":lane").unwrap_err().contains("project name is empty"));
    }

    #[test]
    fn self_qualification_works_with_reach_off() {
        let r = reach("/w/alpha", Level::Off, None, false);
        assert_eq!(r.parse("alpha:lane").unwrap(), Addr::Local("lane".into()));
    }

    #[test]
    fn a_duplicate_name_from_a_hand_edit_is_refused_not_guessed() {
        let mut g = reg();
        g.projects.push(Entry { root: "/w/beta2".into(), name: "beta".into(), private: false });
        let r = Reach::new(Inputs { main_root: "/w/alpha", user: Level::Read, flag: None, in_run: false, registry: g });
        let e = r.parse("beta:x").unwrap_err();
        assert!(e.contains("ambiguous"), "{e}");
        assert!(!e.contains("/w/"), "no root in the message: {e}");
    }

    /// Identity is the registry's, so a repo that sets its own prefix to
    /// another project's name gains nothing: its name is whatever the
    /// registry gave it.
    #[test]
    fn the_callers_name_is_the_registry_entry_for_its_root() {
        assert_eq!(reach("/w/beta", Level::Read, None, false).my_name(), Some("beta"));
        assert_eq!(reach("/w/unknown", Level::Read, None, false).my_name(), None);
    }
}
