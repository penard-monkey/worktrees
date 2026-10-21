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

/// The user's act on the global default: save it, then — when it is full —
/// make sure every registered project it now makes full has `/.planning/`
/// excluded. A project that is unreachable or holds a tracked `.planning/`
/// is REPORTED, never a reason to refuse the setting: the default is about
/// the machine, and one stale registry entry must not block it. Only projects
/// whose effective level is full are touched (an explicit off/show keeps
/// them out of the sweep).
pub fn set_global(d: GlobalDefault) -> Result<(Global, Vec<(Entry, Exclude)>), String> {
    set_global_at(&global_path(), &crate::registry::path(), d)
}

pub fn set_global_at(path: &Path, reg_path: &Path, d: GlobalDefault) -> Result<(Global, Vec<(Entry, Exclude)>), String> {
    let g = save_global_at(path, d)?;
    if g.default != GlobalDefault::Full {
        return Ok((g, Vec::new()));
    }
    let swept = crate::registry::read_lenient_at(reg_path)
        .projects
        .into_iter()
        .filter(|e| effective_from(Some(e), &g).level == Level::Full)
        .map(|e| {
            let x = exclude_for_root(&e.root);
            (e, x)
        })
        .collect();
    Ok((g, swept))
}

pub fn save_global_at(path: &Path, d: GlobalDefault) -> Result<Global, String> {
    let g = Global { default: d, version: VERSION };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let body = serde_json::to_string_pretty(&g).map_err(|e| e.to_string())? + "\n";
    crate::fsx::write_atomic(path, body.as_bytes(), None).map_err(|e| format!("{}: {e}", path.display()))?;
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
///
/// When the project ends up FULL — chosen, or inherited from a full default —
/// `/.planning/` goes into the clone's info/exclude right after the registry
/// write, and the outcome comes back with the entry. Before this the only
/// writer was `worktrees new`, so (main)'s own session could write
/// `.planning/orchestrator/` untracked and unignored before any place existed.
pub fn set_project(key: &str, choice: &Choice) -> Result<Set, String> {
    set_project_at(&crate::registry::path(), &read_global(), key, choice)
}

/// What [`set_project`] did: the stored entry, and the exclude outcome when
/// the project is now effectively full (`None` otherwise).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set {
    pub entry: Entry,
    pub exclude: Option<Exclude>,
}

pub fn set_project_at(reg_path: &Path, g: &Global, key: &str, choice: &Choice) -> Result<Set, String> {
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
    let entry = crate::registry::edit_at(reg_path, |reg| {
        let e = reg.projects.iter_mut().find(|e| e.root == root).ok_or_else(|| format!("no registered project: {key}"))?;
        e.planning = choice.level;
        e.plan_path = path.clone();
        e.plan_scope = match (choice.level, choice.plan_scope) {
            (Some(Level::Show), Some(Scope::Main)) => Some(Scope::Main),
            _ => None,
        };
        Ok(e.clone())
    })?;
    let exclude = (effective_from(Some(&entry), g).level == Level::Full).then(|| exclude_for_root(&entry.root));
    Ok(Set { entry, exclude })
}

/// What making sure `.planning/` is ignored came to.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Exclude {
    /// `/.planning/` was appended to info/exclude.
    Added,
    /// git already ignores `.planning/` here (a `.gitignore`, the exclude
    /// file, or a global excludes file) — nothing written.
    Ignored,
    /// info/exclude already has the line, yet git does not ignore
    /// `.planning/` — a negation somewhere wins over it. Nothing written.
    Listed,
    /// `.planning/` holds tracked files: an exclude line cannot hide them.
    Tracked,
    /// The repo could not be read or the file not written.
    Failed { reason: String },
}

impl Exclude {
    /// The one line the CLI prints (and the app logs) for it.
    pub fn words(&self) -> String {
        match self {
            Exclude::Added => "planning: added /.planning/ to .git/info/exclude (local, never committed)".into(),
            Exclude::Ignored => "planning: git already ignores .planning/ here — nothing written".into(),
            Exclude::Listed => "planning: .git/info/exclude lists /.planning/, but another ignore rule un-ignores it".into(),
            Exclude::Tracked => "planning: .planning/ holds tracked files — an exclude line cannot hide them".into(),
            Exclude::Failed { reason } => format!("planning: could not exclude .planning/: {reason}"),
        }
    }
}

/// [`ensure_planning_excluded`] for a project known only by its main root:
/// resolves the clone's git common dir, and reports a tracked `.planning/`
/// instead of writing a line that could not hide it.
pub fn exclude_for_root(root: &str) -> Exclude {
    if root.is_empty() || !Path::new(root).is_dir() {
        return Exclude::Failed { reason: format!("{root}: not a directory") };
    }
    let Some(raw) = crate::git::git_out(root, &["rev-parse", "--git-common-dir"]) else {
        return Exclude::Failed { reason: format!("{root}: not a git repository") };
    };
    let raw = PathBuf::from(raw.trim());
    let common = if raw.is_absolute() { raw } else { Path::new(root).join(raw) };
    heal(&common.to_string_lossy(), root)
}

/// The one guarded write every caller uses — the settings, a launch and `new`
/// alike: a `.planning/` with tracked files in `wt` is reported, never given
/// an exclude line it cannot honour (and that the settings row already said
/// was not written).
pub fn heal(git_common: &str, wt: &str) -> Exclude {
    if tracked_planning(wt) {
        return Exclude::Tracked;
    }
    ensure_planning_excluded(git_common, wt)
}

/// In full mode, make sure `.planning/` is ignored: append `/.planning/` to
/// the clone's `info/exclude` (shared by every worktree of it) when git does
/// not already ignore it in `wt`. Local and never committed; nothing else in
/// the repo is written. Callers go through [`heal`], which refuses a tracked
/// `.planning/` first.
pub fn ensure_planning_excluded(git_common: &str, wt: &str) -> Exclude {
    let probe = format!("{}/{}", crate::ops::PLANNING_DIR, ".active_plan");
    if crate::git::git_ok(wt, &["check-ignore", "-q", "--no-index", "--", &probe]) {
        return Exclude::Ignored;
    }
    let excl = Path::new(git_common).join("info/exclude");
    let line = "/.planning/";
    let existing = match std::fs::read_to_string(&excl) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Exclude::Failed { reason: format!("{}: {e}", excl.display()) },
    };
    if existing.lines().any(|l| l.trim() == line) {
        return Exclude::Listed;
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
    match std::fs::write(&excl, body) {
        Ok(()) => Exclude::Added,
        Err(e) => Exclude::Failed { reason: format!("{}: {e}", excl.display()) },
    }
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

    /// A scratch repo under the temp dir, and git run ONLY there: a path that
    /// came back empty or outside $TMP would make `git -C` mean the live tree.
    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let tmp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
            let dir = tmp.join(format!("wtplanning-x-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("repo")).unwrap();
            let s = Scratch { dir };
            s.git(&["init", "-q"]);
            // Repo config beats the user's: no global excludes file on this
            // machine can make `.planning/` read as already ignored.
            s.git(&["config", "core.excludesFile", "/dev/null"]);
            s
        }
        fn repo(&self) -> PathBuf {
            self.dir.join("repo")
        }
        fn git(&self, args: &[&str]) {
            let repo = self.repo();
            let tmp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
            assert!(!repo.as_os_str().is_empty() && repo.starts_with(&tmp), "refusing git outside $TMP: {}", repo.display());
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.excludesFile=/dev/null"])
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        }
        /// A registry holding just this repo, beside it (never ~/.config).
        fn registry(&self) -> PathBuf {
            let reg = self.dir.join("projects.json");
            let root = self.repo().to_string_lossy().into_owned();
            crate::registry::edit_at(&reg, |r| {
                r.projects.push(Entry { root, name: "repo".into(), ..Default::default() });
                Ok(())
            })
            .unwrap();
            reg
        }
        fn exclude_lines(&self) -> usize {
            std::fs::read_to_string(self.repo().join(".git/info/exclude"))
                .unwrap_or_default()
                .lines()
                .filter(|l| l.trim() == "/.planning/")
                .count()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn choose(level: Option<Level>) -> Choice {
        Choice { level, plan_path: None, plan_scope: None }
    }

    #[test]
    fn setting_full_excludes_planning_once_and_says_so() {
        let x = Scratch::new("full");
        let reg = x.registry();
        let root = x.repo().to_string_lossy().into_owned();
        let unset = Global::default();
        let set = set_project_at(&reg, &unset, &root, &choose(Some(Level::Off))).unwrap();
        assert_eq!(set.exclude, None, "off writes nothing");
        assert_eq!(x.exclude_lines(), 0);
        let set = set_project_at(&reg, &unset, &root, &choose(Some(Level::Full))).unwrap();
        assert_eq!(set.exclude, Some(Exclude::Added));
        assert_eq!(x.exclude_lines(), 1);
        let set = set_project_at(&reg, &unset, &root, &choose(Some(Level::Full))).unwrap();
        assert_eq!(set.exclude, Some(Exclude::Ignored), "idempotent: the line now ignores it");
        assert_eq!(x.exclude_lines(), 1);
    }

    #[test]
    fn inheriting_a_full_default_is_full_too() {
        let x = Scratch::new("inherit");
        let reg = x.registry();
        let root = x.repo().to_string_lossy().into_owned();
        let full = Global { default: GlobalDefault::Full, version: VERSION };
        let set = set_project_at(&reg, &full, &root, &choose(None)).unwrap();
        assert_eq!(set.exclude, Some(Exclude::Added));
        assert_eq!(x.exclude_lines(), 1);
    }

    #[test]
    fn a_gitignore_that_covers_planning_is_left_alone() {
        let x = Scratch::new("ignored");
        std::fs::write(x.repo().join(".gitignore"), ".planning/\n").unwrap();
        let reg = x.registry();
        let root = x.repo().to_string_lossy().into_owned();
        let set = set_project_at(&reg, &Global::default(), &root, &choose(Some(Level::Full))).unwrap();
        assert_eq!(set.exclude, Some(Exclude::Ignored));
        assert_eq!(x.exclude_lines(), 0, "info/exclude untouched");
    }

    #[test]
    fn the_full_default_sweeps_inheriting_projects_and_reports_the_rest() {
        let x = Scratch::new("sweep");
        let reg = x.registry();
        // an unreachable entry and an explicit-off one beside it
        crate::registry::edit_at(&reg, |r| {
            r.projects.push(Entry { root: x.dir.join("gone").to_string_lossy().into_owned(), name: "gone".into(), ..Default::default() });
            r.projects.push(Entry { root: x.dir.join("off").to_string_lossy().into_owned(), name: "off".into(), planning: Some(Level::Off), ..Default::default() });
            Ok(())
        })
        .unwrap();
        let gpath = x.dir.join("planning.json");
        let (_, swept) = set_global_at(&gpath, &reg, GlobalDefault::Off).unwrap();
        assert!(swept.is_empty(), "off sweeps nothing");
        assert_eq!(x.exclude_lines(), 0);
        let (g, swept) = set_global_at(&gpath, &reg, GlobalDefault::Full).unwrap();
        assert_eq!(g.default, GlobalDefault::Full, "an unreachable project never refuses the setting");
        let names: Vec<_> = swept.iter().map(|(e, _)| e.name.as_str()).collect();
        assert_eq!(names, ["repo", "gone"], "the explicit off is not swept");
        assert_eq!(swept[0].1, Exclude::Added);
        assert!(matches!(swept[1].1, Exclude::Failed { .. }), "{:?}", swept[1].1);
        assert_eq!(x.exclude_lines(), 1);
    }

    #[test]
    fn a_listed_line_that_a_negation_overrides_is_not_written_twice() {
        let x = Scratch::new("listed");
        // A .gitignore outranks info/exclude, so its negation un-ignores
        // `.planning/` even with our line present: the probe says "not
        // ignored", and only the line check stops a second append.
        std::fs::create_dir_all(x.repo().join(".git/info")).unwrap();
        std::fs::write(x.repo().join(".git/info/exclude"), "/.planning/\n").unwrap();
        std::fs::write(x.repo().join(".gitignore"), "!/.planning/\n").unwrap();
        assert_eq!(exclude_for_root(&x.repo().to_string_lossy()), Exclude::Listed);
        assert_eq!(x.exclude_lines(), 1);
    }

    #[test]
    fn a_tracked_planning_dir_is_reported_not_written() {
        let x = Scratch::new("tracked");
        std::fs::create_dir_all(x.repo().join(".planning/o")).unwrap();
        std::fs::write(x.repo().join(".planning/o/task_plan.md"), "# g\n").unwrap();
        x.git(&["add", "-f", ".planning"]);
        x.git(&["commit", "-qm", "t"]);
        assert_eq!(exclude_for_root(&x.repo().to_string_lossy()), Exclude::Tracked);
        assert_eq!(x.exclude_lines(), 0);
    }
}
