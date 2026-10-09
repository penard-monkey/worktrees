//! Owned planning: whether worktrees runs a project's planning layout, only
//! shows it, or stays out of it (`docs/proposals/owned-planning.md` §2).
//!
//! **Two keys, both user tier.** A global default in
//! `~/.config/worktrees/planning.json` (`unset` / `full` / `off`) and a
//! per-project level on the registry entry (`off` / `show` / `full`, absent =
//! inherit). ADR 0001's provenance rule applies even though no argv is
//! involved: adopting changes which hooks run and what agents are told, so
//! nothing a repo contains can switch it on.
//!
//! **Every launcher reads the same file**, the way `guidance::settings()` does:
//! the app, `worktrees new`, MCP `create_worktree`, the plan hook and
//! `plan::summarize_place` all call [`effective`]. An app-memory override would
//! never reach a `worktrees new` run in a terminal.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::registry::Entry;

/// Bumped when the offer is worth asking about again. The app's `planning`
/// offer fingerprints on it.
pub const VERSION: u32 = 1;

/// A project's planning level.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Today's behaviour exactly: legacy resolution, no writes, no hooks.
    #[default]
    Off,
    /// The Plan tab and MCP show the plan at a user-chosen path. No hooks, no
    /// writes beyond the brief `new` already writes.
    Show,
    /// Worktrees owns the layout: `.active_plan` at `new`, owned resolution,
    /// the Claude hooks.
    Full,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Show => "show",
            Level::Full => "full",
        }
    }
}

/// Where a show-only path is read.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Against each place's own root.
    #[default]
    Place,
    /// Against MAIN's working tree as it is — possibly dirty, possibly on
    /// another branch — for every place. Never `HEAD:<rel>`.
    Main,
}

/// The global default. `Unset` behaves as off; it differs only in that the
/// offer is still pending.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GlobalDefault {
    #[default]
    Unset,
    Full,
    Off,
}

/// `planning.json`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Global {
    #[serde(default)]
    pub default: GlobalDefault,
    /// The [`VERSION`] the choice was made at; 0 = never chosen.
    #[serde(default)]
    pub version: u32,
}

pub fn global_path() -> PathBuf {
    crate::profile::config_root_pub().join("planning.json")
}

/// A missing or malformed file reads as unset: planning never turns itself on.
pub fn read_global() -> Global {
    read_global_at(&global_path())
}

pub fn read_global_at(p: &Path) -> Global {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_global(d: GlobalDefault) -> Result<Global, String> {
    save_global_at(&global_path(), d)
}

pub fn save_global_at(path: &Path, d: GlobalDefault) -> Result<Global, String> {
    let g = Global { default: d, version: VERSION };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let body = serde_json::to_string_pretty(&g).map_err(|e| e.to_string())? + "\n";
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(g)
}

/// Where the effective level came from.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum From {
    Project,
    Global,
    Default,
}

/// One project's effective planning, resolved.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Effective {
    pub level: Level,
    pub from: From,
    /// Show only.
    pub plan_path: Option<String>,
    pub plan_scope: Scope,
}

/// project ?? global ?? off. `show` exists only per project (it needs a path,
/// and a path belongs to one project); a show entry without a path reads as
/// off rather than as a show with nothing to show.
pub fn effective_from(entry: Option<&Entry>, g: &Global) -> Effective {
    if let Some(e) = entry {
        if let Some(level) = e.planning {
            // As stored: entry already stripped the one trailing slash.
            let path = e.plan_path.clone().filter(|p| !p.trim().is_empty());
            if level == Level::Show && path.is_none() {
                return Effective { level: Level::Off, from: From::Project, plan_path: None, plan_scope: Scope::Place };
            }
            return Effective {
                level,
                from: From::Project,
                plan_path: if level == Level::Show { path } else { None },
                plan_scope: if level == Level::Show { e.plan_scope.unwrap_or_default() } else { Scope::Place },
            };
        }
    }
    let (level, from) = match g.default {
        GlobalDefault::Full => (Level::Full, From::Global),
        GlobalDefault::Off => (Level::Off, From::Global),
        GlobalDefault::Unset => (Level::Off, From::Default),
    };
    Effective { level, from, plan_path: None, plan_scope: Scope::Place }
}

/// The effective planning of the project whose canonical main root is
/// `main_root`. Reads both files every call — the plan hook relies on that to
/// go silent the moment planning is turned off.
pub fn effective(main_root: &str) -> Effective {
    let reg = crate::registry::read_lenient();
    effective_from(reg.by_root(main_root), &read_global())
}

/// The main root of the place at `place_root`, without git: places live at
/// `<main_root>/.worktrees/<slug>` (`Project::wt_root`), so a place's parent
/// named `.worktrees` gives it away; anything else is its own main.
pub fn main_root_of(place_root: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(place_root).unwrap_or_else(|_| place_root.to_path_buf());
    match canon.parent() {
        Some(parent) if parent.file_name().is_some_and(|n| n == ".worktrees") => {
            parent.parent().map(Path::to_path_buf).unwrap_or(canon)
        }
        _ => canon,
    }
}

/// The resolution mode for a place, from its project's effective planning.
pub fn mode_from(eff: &Effective, main_root: &Path) -> crate::plan::Mode {
    match eff.level {
        Level::Off => crate::plan::Mode::Legacy,
        Level::Full => crate::plan::Mode::Owned,
        Level::Show => crate::plan::Mode::Show {
            rel: eff.plan_path.clone().unwrap_or_default(),
            scope: eff.plan_scope,
            main_root: main_root.to_path_buf(),
        },
    }
}

/// `.planning/` holds tracked files — someone else's committed folder. Full
/// is refused there at adoption, and every `new` warns.
pub fn tracked_planning(root: &str) -> bool {
    crate::git::git_out(root, &["ls-files", "--", crate::ops::PLANNING_DIR]).is_some_and(|o| !o.trim().is_empty())
}

pub const TRACKED_REFUSAL: &str = "`.planning/` holds tracked files in this repo, so worktrees cannot own it — full planning would write `.active_plan` and topic folders into a committed folder that info/exclude cannot hide. Choose show only to see your own plans instead.";

/// Whether the repo at `root` suggests owned planning: a `[plan]` table in its
/// `.worktrees.toml`. A suggestion only pre-sets the Add existing dialog
/// (owned-planning §2.1, Q2) — it never changes the effective level.
pub fn repo_suggests(root: &Path) -> bool {
    let p = root.join(".worktrees.toml");
    if !std::fs::symlink_metadata(&p).map(|m| m.is_file()).unwrap_or(false) {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(&p) else { return false };
    text.parse::<toml::Table>().map(|t| t.get("plan").is_some_and(|v| v.is_table())).unwrap_or(false)
}

/// What a caller asks a project's planning to become. `None` = inherit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub level: Option<Level>,
    pub plan_path: Option<String>,
    pub plan_scope: Option<Scope>,
}

/// Validate a show-only path at entry, against MAIN's root: refused when it
/// leaves the project or names neither a file nor a directory. A path that is
/// not in main is accepted for `place` scope (lanes may have it on their
/// branch) and refused for `main` scope, which reads only main.
pub fn validate_show_path(main_root: &Path, rel: &str, scope: Scope) -> Result<String, String> {
    use crate::safepath::{classify, Refusal};
    let rel = crate::safepath::normalize_entered(rel);
    if rel.is_empty() {
        return Err("show only needs a path".into());
    }
    match classify(main_root, &rel) {
        Ok(_) => Ok(rel),
        Err(Refusal::NotFound) if scope == Scope::Place => Ok(rel),
        Err(r) => Err(format!("{rel}: {}", r.reason())),
    }
}

/// Set (or clear) a project's planning. Full is refused where `.planning/`
/// holds tracked files — one `git ls-files`, at adoption only. A show-only
/// path is validated by the same `safepath` the resolver reads it with.
pub fn set_project(key: &str, choice: &Choice) -> Result<Entry, String> {
    set_project_at(&crate::registry::path(), key, choice)
}

pub fn set_project_at(reg_path: &Path, key: &str, choice: &Choice) -> Result<Entry, String> {
    let key = crate::registry::resolve_key(key);
    let root = crate::registry::read_lenient_at(reg_path)
        .find(&key)
        .map(|e| e.root.clone())
        .ok_or_else(|| format!("no registered project: {key}"))?;
    let path = match choice.level {
        Some(Level::Full) => {
            if tracked_planning(&root) {
                return Err(TRACKED_REFUSAL.into());
            }
            None
        }
        Some(Level::Show) => {
            let rel = choice.plan_path.as_deref().unwrap_or("");
            Some(validate_show_path(Path::new(&root), rel, choice.plan_scope.unwrap_or_default())?)
        }
        _ => None,
    };
    crate::registry::edit_at(reg_path, |reg| {
        let e = reg.projects.iter_mut().find(|e| e.root == root).ok_or_else(|| format!("no registered project: {key}"))?;
        e.planning = choice.level;
        e.plan_path = path.clone();
        e.plan_scope = match (choice.level, choice.plan_scope) {
            (Some(Level::Show), Some(Scope::Main)) => Some(Scope::Main),
            _ => None,
        };
        Ok(e.clone())
    })
}

/// In full mode, make sure `.planning/` is ignored: append `/.planning/` to
/// the clone's `info/exclude` (shared by every worktree of it) when git does
/// not already ignore it in `wt`. Local and never committed; nothing else in
/// the repo is written. Returns whether it wrote.
pub fn ensure_planning_excluded(git_common: &str, wt: &str) -> bool {
    let probe = format!("{}/{}", crate::ops::PLANNING_DIR, ".active_plan");
    if crate::git::git_ok(wt, &["check-ignore", "-q", "--no-index", "--", &probe]) {
        return false;
    }
    let excl = Path::new(git_common).join("info/exclude");
    let line = "/.planning/";
    let existing = std::fs::read_to_string(&excl).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == line) {
        return false;
    }
    if let Some(dir) = excl.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut body = existing;
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(line);
    body.push('\n');
    std::fs::write(&excl, body).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: Option<Level>, path: Option<&str>, scope: Option<Scope>) -> Entry {
        Entry {
            root: "/r".into(),
            name: "r".into(),
            planning: level,
            plan_path: path.map(Into::into),
            plan_scope: scope,
            ..Default::default()
        }
    }

    #[test]
    fn project_beats_global_beats_off() {
        let full = Global { default: GlobalDefault::Full, version: 1 };
        let unset = Global::default();
        assert_eq!(effective_from(None, &unset).level, Level::Off);
        assert_eq!(effective_from(None, &unset).from, From::Default);
        assert_eq!(effective_from(None, &full).level, Level::Full);
        let e = entry(Some(Level::Off), None, None);
        assert_eq!(effective_from(Some(&e), &full).level, Level::Off, "a project's off beats a global full");
        let e = entry(None, None, None);
        assert_eq!(effective_from(Some(&e), &full).from, From::Global, "absent = inherit");
    }

    #[test]
    fn show_carries_its_path_and_scope_and_without_a_path_is_off() {
        let g = Global::default();
        let e = entry(Some(Level::Show), Some("docs/plan//"), Some(Scope::Main));
        let eff = effective_from(Some(&e), &g);
        assert_eq!(eff.level, Level::Show);
        assert_eq!(eff.plan_path.as_deref(), Some("docs/plan//"), "read as stored — only entry normalises");
        assert_eq!(eff.plan_scope, Scope::Main);
        let e = entry(Some(Level::Show), None, None);
        assert_eq!(effective_from(Some(&e), &g).level, Level::Off);
        // a stale path on a full entry is ignored
        let e = entry(Some(Level::Full), Some("docs/plan"), Some(Scope::Main));
        let eff = effective_from(Some(&e), &g);
        assert_eq!((eff.plan_path, eff.plan_scope), (None, Scope::Place));
    }

    #[test]
    fn a_malformed_global_file_reads_as_unset() {
        let d = std::env::temp_dir().join(format!("wtplanning-g-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("planning.json");
        std::fs::write(&p, "{ not json").unwrap();
        assert_eq!(read_global_at(&p), Global::default());
        let g = save_global_at(&p, GlobalDefault::Full).unwrap();
        assert_eq!(read_global_at(&p), g);
        assert_eq!(g.version, VERSION);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_plan_table_in_the_repo_config_is_a_suggestion() {
        let d = std::env::temp_dir().join(format!("wtplanning-s-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(!repo_suggests(&d));
        std::fs::write(d.join(".worktrees.toml"), "[docs]\nindex = \"README.md\"\n").unwrap();
        assert!(!repo_suggests(&d));
        std::fs::write(d.join(".worktrees.toml"), "[plan]\nproject = \"docs/plan/goals.md\"\n").unwrap();
        assert!(repo_suggests(&d));
        std::fs::write(d.join(".worktrees.toml"), "plan = 1\n").unwrap();
        assert!(!repo_suggests(&d), "a plan KEY is not a [plan] table");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_place_maps_to_its_main_root_without_git() {
        let d = std::fs::canonicalize(std::env::temp_dir()).unwrap().join(format!("wtplanning-m-{}", std::process::id()));
        std::fs::create_dir_all(d.join(".worktrees/lane/sub")).unwrap();
        assert_eq!(main_root_of(&d.join(".worktrees/lane")), d);
        assert_eq!(main_root_of(&d), d);
        let _ = std::fs::remove_dir_all(&d);
    }
}
