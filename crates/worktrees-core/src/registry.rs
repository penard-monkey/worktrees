//! The project registry — every repo the user has registered with worktrees,
//! in nav order, each under a NAME (`docs/proposals/cross-project.md` §2, §3.2).
//!
//! `~/.config/worktrees/projects.json` (respecting `$XDG_CONFIG_HOME`), beside
//! `profiles.json` and `skills.json`. Core owns it; the CLI (`worktrees
//! projects`), the MCP server and the app all read it, and the app writes it
//! only through this module. Before this the list lived in the app's own config
//! dir, where nothing but the app could see it.
//!
//! **Registering is the consent act.** A repo an agent elsewhere can reach is
//! one the user added. Nothing a repo contains can add itself here, rename an
//! entry or clear `private` — the file is user tier, and `cross_project` (the
//! setting that turns reach on) is in `projcfg`'s `USER_ONLY_KEYS`.
//!
//! **A name is the handle, never the prefix.** `Project.prefix` is partly
//! repo-supplied (`.worktree-prefix`, `[project] prefix`), so a cloned repo
//! could claim another project's prefix. A name is SEEDED from the repo's own
//! prefix sources at registration — so it usually reads like the tmux sessions
//! — and is unique by construction from then on: `add` suffixes `-2`, `-3`…,
//! and `rename` refuses a name that is taken.
//!
//! **Every write is read-modify-write under a lock, then temp + rename.** The
//! app and `worktrees projects add` can write at the same moment, and the app's
//! old nav-reorder read the file, merged, and wrote it back with nothing held
//! in between — an `add` landing in that gap was lost. The lock is an `flock`
//! on a sidecar file (the same primitive `ops.rs` uses for the agent switch)
//! plus an in-process mutex, since the app is one process with many threads.

use std::fs;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "projects.json";

/// Belt and braces in front of the flock. Each `edit_at` opens the lock file
/// itself, and separate `open()`s are separate open file descriptions, which
/// DO contend under `flock` even inside one process — so the flock alone is
/// the guarantee. The mutex only keeps the app's own threads from queueing on
/// a syscall, and keeps this correct if a later edit ever shares one handle.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Entry {
    /// Canonical main root of the repo.
    pub root: String,
    /// The handle agents in other projects address it by (`<name>:<slug>`).
    pub name: String,
    /// Out of reach in both directions (§5.2).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private: bool,
    /// Owned planning for this project (`planning.rs`). Absent = inherit the
    /// global default. User tier for the same reason as `private`: it decides
    /// which hooks run and what agents are told, so a repo cannot set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planning: Option<crate::planning::Level>,
    /// Show-only: the repo-relative path the Plan tab shows. Ignored at other
    /// levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_path: Option<String>,
    /// Show-only: read `plan_path` in each place, or in main's working tree.
    /// Absent = place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_scope: Option<crate::planning::Scope>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    #[serde(default)]
    pub version: u32,
    /// In nav order.
    #[serde(default)]
    pub projects: Vec<Entry>,
    /// For each OLDER list merged in (the app's `projects.json`), the roots it
    /// held when last merged. A root is imported only if it is NEW since then,
    /// so a downgraded app that adds a project still has it carried across,
    /// while a project removed here is not resurrected from the stale file.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub imported: std::collections::BTreeMap<String, Vec<String>>,
}

impl Registry {
    pub fn roots(&self) -> Vec<String> {
        self.projects.iter().map(|e| e.root.clone()).collect()
    }

    pub fn by_root(&self, root: &str) -> Option<&Entry> {
        self.projects.iter().find(|e| e.root == root)
    }

    /// An entry by its name, or by its root — what `worktrees projects` takes.
    /// A path key must already be resolved (`resolve_key`); this matches
    /// literally.
    pub fn find(&self, key: &str) -> Option<&Entry> {
        self.projects.iter().find(|e| e.name == key).or_else(|| self.by_root(key))
    }

    /// Entries whose root lies strictly inside another entry's root — a repo
    /// checked out under another repo's tree. Pairs of (inner, outer).
    pub fn nested(&self) -> Vec<(&Entry, &Entry)> {
        let mut out = Vec::new();
        for inner in &self.projects {
            for outer in &self.projects {
                if inner.root != outer.root && Path::new(&inner.root).starts_with(&outer.root) {
                    out.push((inner, outer));
                }
            }
        }
        out
    }
}

/// `$XDG_CONFIG_HOME/worktrees/projects.json`.
pub fn path() -> PathBuf {
    crate::profile::config_root_pub().join(FILE)
}

/// For readers. A missing file is an empty registry; an unreadable one is too,
/// rather than an error every caller has to handle — but a WRITE refuses to
/// clobber it (`read_strict`).
pub fn read_lenient() -> Registry {
    read_lenient_at(&path())
}

pub fn read_lenient_at(p: &Path) -> Registry {
    fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn read_strict(p: &Path) -> Result<Registry, String> {
    match fs::read(p) {
        // An EMPTY file is an empty registry, as `read_lenient` already reads
        // it: a 0-byte file (a crash between create and write, or `: >` by
        // hand) must not wedge every later write behind "not valid JSON".
        Ok(b) if b.iter().all(u8::is_ascii_whitespace) => Ok(Registry::default()),
        Ok(b) => serde_json::from_slice(&b)
            .map_err(|e| format!("{} is not valid JSON ({e}) — not overwriting", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// Read under the lock → mutate → atomic write. The closure sees the file as
/// it is NOW, never a caller's earlier snapshot; that is the whole fix for the
/// lost-`add` race.
pub fn edit<T>(f: impl FnOnce(&mut Registry) -> Result<T, String>) -> Result<T, String> {
    edit_at(&path(), f)
}

pub fn edit_at<T>(p: &Path, f: impl FnOnce(&mut Registry) -> Result<T, String>) -> Result<T, String> {
    let _serial = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = p.parent().ok_or("registry path has no parent")?;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let lock_path = dir.join(format!("{FILE}.lock"));
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    // SAFETY: flock gets a live descriptor; `lock` outlives the critical
    // section and closing it releases the lock on every exit path.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!("could not lock {}", lock_path.display()));
    }
    let mut reg = read_strict(p)?;
    let before = reg.clone();
    let out = f(&mut reg)?;
    if reg != before {
        reg.version = reg.version.max(1);
        let json = serde_json::to_string_pretty(&reg).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
        fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
        fs::rename(&tmp, p).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("{}: {e}", p.display())
        })?;
    }
    drop(lock);
    Ok(out)
}

/// A key as `find` should see it. A name has no `/`, so a key with one is a
/// PATH, resolved the way `add` resolved it — to the canonical main root of the
/// repo it is in, or, for a repo that is gone, to the canonical path, or else
/// left as typed. So a symlinked path, a trailing `/` or a subdirectory names
/// the same entry `add` made.
pub fn resolve_key(key: &str) -> String {
    if !key.contains('/') {
        return key.to_string();
    }
    let p = Path::new(key);
    if let Ok(proj) = crate::Project::discover(p) {
        return proj.main_root;
    }
    fs::canonicalize(p).map(|c| c.to_string_lossy().into_owned()).unwrap_or_else(|_| key.trim_end_matches('/').to_string())
}

// ── names ───────────────────────────────────────────────────────────────────

/// Whether `n` may be a name: `[A-Za-z0-9._-]`, not starting with `-` or `.`.
/// No `:` (it splits an address) and no `/` (it would split a `place://`
/// uri). The leading-`-` rule is `safe_arg`'s — a name is passed as argv.
pub fn valid_name(n: &str) -> Result<(), String> {
    if n.is_empty() {
        return Err("a project name cannot be empty".into());
    }
    if n.len() > 64 {
        return Err("a project name is at most 64 characters".into());
    }
    if n.starts_with('-') || n.starts_with('.') {
        return Err(format!("'{n}' cannot start with '-' or '.'"));
    }
    if let Some(c) = n.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))) {
        return Err(format!("'{n}' contains '{c}' — names use letters, digits, '.', '_' and '-'"));
    }
    Ok(())
}

/// Fold anything into a valid name, or `project` if nothing survives.
fn clean_name(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    s = s.trim_start_matches(['-', '.']).chars().take(64).collect();
    if s.is_empty() { "project".to_string() } else { s }
}

/// The name a newly registered repo starts with: the REPO's own prefix sources
/// (`.worktree-prefix`, `[project] prefix`, then the basename) — never
/// `$WORKTREES_PREFIX` or the user config's `prefix`, which are global and
/// would seed every project with the same word.
pub fn seed_name(root: &str) -> String {
    let file = crate::project::prefix_file(root);
    let proj = crate::projcfg::project_prefix(Path::new(root));
    let base = Path::new(root).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    clean_name(&crate::config::resolve_prefix_from(None, file.as_deref(), proj.as_deref(), None, &base))
}

/// `want`, or `want-2`, `want-3`… — the first one no entry holds.
fn unique_name(reg: &Registry, want: &str) -> String {
    let taken = |n: &str| reg.projects.iter().any(|e| e.name == n);
    if !taken(want) {
        return want.to_string();
    }
    (2..).map(|i| format!("{want}-{i}")).find(|n| !taken(n)).expect("unbounded")
}

// ── operations ──────────────────────────────────────────────────────────────

/// Append `root` (already canonical) under a fresh unique name. Idempotent:
/// a root already present keeps its entry and position. Returns its entry.
pub fn add(root: &str) -> Result<Entry, String> {
    add_at(&path(), root)
}

pub fn add_at(p: &Path, root: &str) -> Result<Entry, String> {
    let seed = seed_name(root);
    edit_at(p, |reg| Ok(add_in(reg, root, &seed)))
}

fn add_in(reg: &mut Registry, root: &str, seed: &str) -> Entry {
    if let Some(e) = reg.by_root(root) {
        return e.clone();
    }
    let e = Entry { root: root.to_string(), name: unique_name(reg, seed), ..Default::default() };
    reg.projects.push(e.clone());
    e
}

/// Drop the entry with this root. `Ok(false)` when there was none.
pub fn remove(root: &str) -> Result<bool, String> {
    remove_at(&path(), root)
}

pub fn remove_at(p: &Path, root: &str) -> Result<bool, String> {
    edit_at(p, |reg| {
        let n = reg.projects.len();
        reg.projects.retain(|e| e.root != root);
        Ok(reg.projects.len() != n)
    })
}

/// Re-order by a caller's list, treated as a PREFERENCE: roots the registry no
/// longer has are ignored, and roots the caller never saw are kept, appended in
/// their existing order. A reorder can move entries; it can never delete one —
/// or lose one another writer added since the caller last read.
pub fn reorder(want: &[String]) -> Result<(), String> {
    reorder_at(&path(), want)
}

pub fn reorder_at(p: &Path, want: &[String]) -> Result<(), String> {
    edit_at(p, |reg| {
        reorder_in(reg, want);
        Ok(())
    })
}

fn reorder_in(reg: &mut Registry, want: &[String]) {
    let mut rest = std::mem::take(&mut reg.projects);
    let mut next = Vec::with_capacity(rest.len());
    for r in want {
        if let Some(i) = rest.iter().position(|e| &e.root == r) {
            next.push(rest.remove(i));
        }
    }
    next.extend(rest);
    reg.projects = next;
}

/// Rename the entry `key` (a name or a root). Refuses an invalid name or one
/// another entry holds — unlike `add`, a rename is a person choosing a word,
/// and silently suffixing it would hand them a different one.
pub fn rename(key: &str, new: &str) -> Result<Entry, String> {
    rename_at(&path(), key, new)
}

pub fn rename_at(p: &Path, key: &str, new: &str) -> Result<Entry, String> {
    valid_name(new)?;
    let key = resolve_key(key);
    let key = key.as_str();
    edit_at(p, |reg| {
        let root = reg.find(key).map(|e| e.root.clone()).ok_or_else(|| format!("no registered project: {key}"))?;
        if reg.projects.iter().any(|e| e.name == new && e.root != root) {
            return Err(format!("the name '{new}' is already taken"));
        }
        let e = reg.projects.iter_mut().find(|e| e.root == root).expect("found above");
        e.name = new.to_string();
        Ok(e.clone())
    })
}

pub fn set_private(key: &str, private: bool) -> Result<Entry, String> {
    set_private_at(&path(), key, private)
}

pub fn set_private_at(p: &Path, key: &str, private: bool) -> Result<Entry, String> {
    let key = resolve_key(key);
    edit_at(p, |reg| {
        let root = reg.find(&key).map(|e| e.root.clone()).ok_or_else(|| format!("no registered project: {key}"))?;
        let e = reg.projects.iter_mut().find(|e| e.root == root).expect("found above");
        e.private = private;
        Ok(e.clone())
    })
}

/// Union-merge an older list (the app's `projects.json`: a JSON array of main
/// roots) into the registry (§2.2). Roots already registered keep their entry
/// and position; roots that are NEW in `source` since its last merge are
/// appended in its order. Returns how many were added.
///
/// The source file is only ever READ. A missing or unparsable one merges
/// nothing.
pub fn import_list(source: &Path) -> Result<usize, String> {
    import_list_at(&path(), source)
}

pub fn import_list_at(p: &Path, source: &Path) -> Result<usize, String> {
    let Some(roots) = fs::read(source).ok().and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok()) else {
        return Ok(0);
    };
    let key = source.to_string_lossy().into_owned();
    // Seeds are computed OUTSIDE the lock — each may read a repo's config, and
    // the critical section should be file writes only.
    let seeds: Vec<(String, String)> = roots.iter().map(|r| (r.clone(), seed_name(r))).collect();
    edit_at(p, |reg| {
        let seen: Vec<String> = reg.imported.get(&key).cloned().unwrap_or_default();
        let mut added = 0;
        for (root, seed) in &seeds {
            if seen.contains(root) || reg.by_root(root).is_some() {
                continue;
            }
            add_in(reg, root, seed);
            added += 1;
        }
        if seen != roots {
            reg.imported.insert(key, roots.clone());
        }
        Ok(added)
    })
}

// ── CLI ─────────────────────────────────────────────────────────────────────

/// `worktrees projects [ls|add|rm|rename|private]` — user-global, so it runs
/// anywhere (ahead of main.rs's git guard), like `skills`.
pub fn cmd_projects(ui: &mut dyn crate::ui::Ui, args: &[String]) -> i32 {
    let sub = args.first().map(String::as_str).unwrap_or("ls");
    let rest = args.get(1..).unwrap_or(&[]);
    let json = rest.iter().any(|a| a == "--json");
    let pos: Vec<&String> = rest.iter().filter(|a| !a.starts_with("--")).collect();
    let fail = |ui: &mut dyn crate::ui::Ui, e: String| {
        ui.error(&e);
        1
    };
    match sub {
        "ls" | "list" | "--json" => {
            let reg = read_lenient();
            if json || sub == "--json" {
                let rows: Vec<serde_json::Value> = reg
                    .projects
                    .iter()
                    .map(|e| serde_json::json!({ "name": e.name, "root": e.root, "private": e.private }))
                    .collect();
                ui.plain(&serde_json::to_string_pretty(&serde_json::json!({ "projects": rows })).unwrap_or_default());
                return 0;
            }
            if reg.projects.is_empty() {
                ui.info("No registered projects. Add one with: worktrees projects add [<dir>]");
                return 0;
            }
            let w = reg.projects.iter().map(|e| e.name.len()).max().unwrap_or(0);
            for e in &reg.projects {
                let tag = if e.private { "  (private)" } else { "" };
                ui.plain(&format!("{:<w$}  {}{tag}", e.name, e.root));
            }
            0
        }
        "add" => {
            let dir = match pos.first() {
                Some(d) => PathBuf::from(d.as_str()),
                None => match std::env::current_dir() {
                    Ok(d) => d,
                    Err(e) => return fail(ui, e.to_string()),
                },
            };
            let project = match crate::Project::discover(&dir) {
                Ok(p) => p,
                Err(e) => return fail(ui, format!("{}: {}", dir.display(), e.msg)),
            };
            let had = read_lenient().by_root(&project.main_root).cloned();
            match add(&project.main_root) {
                Ok(e) if had.is_some() => {
                    ui.info(&format!("already registered as '{}': {}", e.name, e.root));
                    0
                }
                Ok(e) => {
                    ui.info(&format!("registered '{}': {}", e.name, e.root));
                    0
                }
                Err(e) => fail(ui, e),
            }
        }
        "rm" | "remove" => {
            let Some(key) = pos.first() else {
                ui.error("usage: worktrees projects rm <name|root>");
                return 2;
            };
            let Some(e) = read_lenient().find(&resolve_key(key)).cloned() else {
                return fail(ui, format!("no registered project: {key}"));
            };
            match remove(&e.root) {
                // Only the registration goes. The repo, its worktrees and its
                // sessions are untouched.
                Ok(_) => {
                    ui.info(&format!("unregistered '{}' ({}); nothing on disk changed", e.name, e.root));
                    0
                }
                Err(err) => fail(ui, err),
            }
        }
        "rename" => {
            let (Some(key), Some(new)) = (pos.first(), pos.get(1)) else {
                ui.error("usage: worktrees projects rename <name|root> <new-name>");
                return 2;
            };
            match rename(key, new) {
                Ok(e) => {
                    ui.info(&format!("'{}' is now named '{}'", e.root, e.name));
                    0
                }
                Err(e) => fail(ui, e),
            }
        }
        "private" => {
            let (Some(key), Some(v)) = (pos.first(), pos.get(1)) else {
                ui.error("usage: worktrees projects private <name|root> on|off");
                return 2;
            };
            let on = match v.as_str() {
                "on" | "true" | "yes" => true,
                "off" | "false" | "no" => false,
                other => return fail(ui, format!("expected on or off, got '{other}'")),
            };
            match set_private(key, on) {
                Ok(e) => {
                    ui.info(&format!("'{}' is {}", e.name, if e.private { "private" } else { "not private" }));
                    0
                }
                Err(e) => fail(ui, e),
            }
        }
        other => {
            ui.error(&format!("unknown projects command: {other} (ls, add, rm, rename, private)"));
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!(
                "wt-registry-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(&d).unwrap();
            // Canonical, as every registered root is: `/var` is a link to
            // `/private/var` on macOS, and `resolve_key` canonicalises.
            Tmp(fs::canonicalize(&d).unwrap())
        }
        fn reg(&self) -> PathBuf {
            self.0.join("projects.json")
        }
        /// A directory that stands in for a repo root: `seed_name` only reads
        /// a prefix file and `.worktrees.toml`, both absent here, so the seed
        /// is the basename.
        fn repo(&self, name: &str) -> String {
            let r = self.0.join(name);
            fs::create_dir_all(&r).unwrap();
            r.to_string_lossy().into_owned()
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn add_is_idempotent_and_keeps_order() {
        let t = Tmp::new("add");
        let (a, b) = (t.repo("alpha"), t.repo("beta"));
        assert_eq!(add_at(&t.reg(), &a).unwrap().name, "alpha");
        add_at(&t.reg(), &b).unwrap();
        add_at(&t.reg(), &a).unwrap();
        let reg = read_lenient_at(&t.reg());
        assert_eq!(reg.roots(), vec![a, b]);
        assert_eq!(reg.version, 1);
    }

    #[test]
    fn a_seeded_name_collision_is_suffixed() {
        let t = Tmp::new("collide");
        let a = t.repo("one/app");
        let b = t.repo("two/app");
        let c = t.repo("three/app");
        assert_eq!(add_at(&t.reg(), &a).unwrap().name, "app");
        assert_eq!(add_at(&t.reg(), &b).unwrap().name, "app-2");
        assert_eq!(add_at(&t.reg(), &c).unwrap().name, "app-3");
    }

    #[test]
    fn the_seed_reads_the_repo_prefix_file_but_not_the_global_env() {
        let t = Tmp::new("seed");
        let a = t.repo("checkout");
        fs::write(Path::new(&a).join(crate::init::PREFIX_FILE), "Shop.Front\n").unwrap();
        // `WORKTREES_PREFIX` is global — honouring it would give every project
        // the same seed. `seed_name` never consults it, so no env poke needed
        // to prove that: the file wins over the basename, and nothing else
        // enters. The seed goes through the prefix sanitiser, exactly as the
        // session name does: case is lowered and `.` becomes `-` (a name may
        // hold `.`, but a seed never brings one). Lowered by the
        // prefix sanitiser, exactly as the session name is.
        assert_eq!(seed_name(&a), "shop-front");
    }

    #[test]
    fn rename_refuses_a_taken_or_invalid_name() {
        let t = Tmp::new("rename");
        let (a, b) = (t.repo("alpha"), t.repo("beta"));
        add_at(&t.reg(), &a).unwrap();
        add_at(&t.reg(), &b).unwrap();
        assert!(rename_at(&t.reg(), "beta", "alpha").unwrap_err().contains("taken"));
        assert!(rename_at(&t.reg(), "beta", "has:colon").is_err());
        assert!(rename_at(&t.reg(), "beta", "-flag").is_err());
        assert!(rename_at(&t.reg(), "nope", "x").unwrap_err().contains("no registered project"));
        assert_eq!(rename_at(&t.reg(), &b, "b2").unwrap().name, "b2");
        // Renaming to its own name is not a collision.
        assert_eq!(rename_at(&t.reg(), "b2", "b2").unwrap().name, "b2");
    }

    #[test]
    fn reorder_keeps_entries_the_caller_never_saw() {
        let t = Tmp::new("reorder");
        let (a, b, c) = (t.repo("a"), t.repo("b"), t.repo("c"));
        for r in [&a, &b, &c] {
            add_at(&t.reg(), r).unwrap();
        }
        // The caller's list is stale: it never saw `c`, and names a root that
        // is gone.
        reorder_at(&t.reg(), &[b.clone(), "/gone".into(), a.clone()]).unwrap();
        assert_eq!(read_lenient_at(&t.reg()).roots(), vec![b, a, c]);
    }

    /// The app's nav reorder rules, moved here with the merge (they were
    /// `merge_project_order`'s tests in the app): nothing invented, nothing
    /// lost.
    #[test]
    fn reorder_is_lossless_in_both_directions() {
        let order = |want: &[&str]| {
            let mut reg = Registry::default();
            for r in ["/a", "/b", "/c"] {
                add_in(&mut reg, r, r.trim_start_matches('/'));
            }
            reorder_in(&mut reg, &want.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            reg.roots()
        };
        assert_eq!(order(&["/c", "/a", "/b"]), ["/c", "/a", "/b"], "a permutation applies verbatim");
        assert_eq!(order(&["/c", "/gone", "/a"]), ["/c", "/a", "/b"], "an unknown root is ignored");
        assert_eq!(order(&["/c"]), ["/c", "/a", "/b"], "unseen roots keep their order, appended");
        assert_eq!(order(&["/b", "/b", "/a"]), ["/b", "/a", "/c"], "a repeat is taken once");
        assert_eq!(order(&[]), ["/a", "/b", "/c"], "an empty request is a no-op, not a wipe");
    }

    #[test]
    fn remove_and_private() {
        let t = Tmp::new("remove");
        let (a, b) = (t.repo("a"), t.repo("b"));
        add_at(&t.reg(), &a).unwrap();
        add_at(&t.reg(), &b).unwrap();
        assert!(set_private_at(&t.reg(), "b", true).unwrap().private);
        assert!(read_lenient_at(&t.reg()).by_root(&b).unwrap().private);
        assert!(remove_at(&t.reg(), &a).unwrap());
        assert!(!remove_at(&t.reg(), &a).unwrap());
        assert_eq!(read_lenient_at(&t.reg()).roots(), vec![b]);
    }

    #[test]
    fn import_is_a_union_merge_and_never_resurrects_a_removal() {
        let t = Tmp::new("import");
        let (a, b, c) = (t.repo("a"), t.repo("b"), t.repo("c"));
        let app = t.0.join("app-projects.json");
        // Core already has `c`; the app's file has `a`, `b`, `c`.
        add_at(&t.reg(), &c).unwrap();
        fs::write(&app, serde_json::to_string(&[&a, &b, &c]).unwrap()).unwrap();
        assert_eq!(import_list_at(&t.reg(), &app).unwrap(), 2);
        assert_eq!(read_lenient_at(&t.reg()).roots(), vec![c.clone(), a.clone(), b.clone()]);
        // Removed here, still in the stale app file: stays removed.
        remove_at(&t.reg(), &a).unwrap();
        assert_eq!(import_list_at(&t.reg(), &app).unwrap(), 0);
        assert!(read_lenient_at(&t.reg()).by_root(&a).is_none());
        // A downgraded app adds `d` to its own file: carried across.
        let d = t.repo("d");
        fs::write(&app, serde_json::to_string(&[&a, &b, &c, &d]).unwrap()).unwrap();
        assert_eq!(import_list_at(&t.reg(), &app).unwrap(), 1);
        assert_eq!(read_lenient_at(&t.reg()).roots(), vec![c, b, d]);
        // The source is only read.
        assert!(fs::read_to_string(&app).unwrap().contains("\"/"));
    }

    #[test]
    fn a_missing_or_broken_source_imports_nothing() {
        let t = Tmp::new("import-none");
        assert_eq!(import_list_at(&t.reg(), &t.0.join("absent.json")).unwrap(), 0);
        let bad = t.0.join("bad.json");
        fs::write(&bad, "{not json").unwrap();
        assert_eq!(import_list_at(&t.reg(), &bad).unwrap(), 0);
        assert!(!t.reg().exists(), "nothing changed, so nothing was written");
    }

    #[test]
    fn an_empty_file_is_an_empty_registry_not_a_wedge() {
        let t = Tmp::new("empty");
        fs::write(t.reg(), "").unwrap();
        let a = t.repo("a");
        assert_eq!(add_at(&t.reg(), &a).unwrap().name, "a");
        assert_eq!(read_lenient_at(&t.reg()).roots(), vec![a]);
    }

    #[test]
    fn a_path_key_resolves_like_add() {
        let t = Tmp::new("key");
        let a = t.repo("alpha");
        add_at(&t.reg(), &a).unwrap();
        // Trailing slash, and a symlink to the same directory.
        let link = t.0.join("via-link");
        std::os::unix::fs::symlink(&a, &link).unwrap();
        for key in [format!("{a}/"), link.to_string_lossy().into_owned()] {
            assert_eq!(set_private_at(&t.reg(), &key, true).unwrap().root, a, "{key}");
        }
        assert_eq!(rename_at(&t.reg(), &format!("{a}/"), "renamed").unwrap().name, "renamed");
        // A bare name is never touched by resolution.
        assert_eq!(resolve_key("renamed"), "renamed");
    }

    #[test]
    fn a_write_refuses_to_clobber_an_unparsable_registry() {
        let t = Tmp::new("strict");
        fs::write(t.reg(), "{oops").unwrap();
        assert!(add_at(&t.reg(), &t.repo("a")).unwrap_err().contains("not overwriting"));
        assert_eq!(fs::read_to_string(t.reg()).unwrap(), "{oops");
        assert!(read_lenient_at(&t.reg()).projects.is_empty());
    }

    /// The race the lock exists for: an `add` that lands between a reorder's
    /// read and its write. Each thread does a full read-modify-write; without
    /// the lock, interleaved writers drop each other's entries.
    #[test]
    fn concurrent_writers_lose_nothing() {
        let t = Tmp::new("race");
        let reg = t.reg();
        let roots: Vec<String> = (0..24).map(|i| t.repo(&format!("r{i}"))).collect();
        std::thread::scope(|s| {
            for chunk in roots.chunks(3) {
                let reg = reg.clone();
                s.spawn(move || {
                    for r in chunk {
                        add_at(&reg, r).unwrap();
                        reorder_at(&reg, &[r.clone()]).unwrap();
                    }
                });
            }
        });
        let got = read_lenient_at(&reg);
        assert_eq!(got.projects.len(), roots.len());
        let mut names: Vec<&str> = got.projects.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), roots.len(), "names stay unique under contention");
    }

    #[test]
    fn nested_entries_are_reported() {
        let t = Tmp::new("nested");
        let outer = t.repo("outer");
        let inner = t.repo("outer/vendor/inner");
        let lookalike = t.repo("outer-two");
        for r in [&outer, &inner, &lookalike] {
            add_at(&t.reg(), r).unwrap();
        }
        let reg = read_lenient_at(&t.reg());
        let pairs: Vec<(String, String)> =
            reg.nested().into_iter().map(|(i, o)| (i.root.clone(), o.root.clone())).collect();
        // Component-wise: `outer-two` is not inside `outer`.
        assert_eq!(pairs, vec![(inner, outer)]);
    }

    #[test]
    fn valid_names() {
        for ok in ["alpha", "a.b", "a_b-2", "X9"] {
            assert!(valid_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-x", ".x", "a:b", "a/b", "a b", "é"] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
        assert_eq!(clean_name("--Hello World/x"), "Hello-World-x");
        assert_eq!(clean_name("::"), "project");
    }
}
