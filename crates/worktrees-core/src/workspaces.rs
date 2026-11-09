//! Named workspaces: lists of REFERENCES to registered projects
//! (`docs/proposals/idea-board-phases.md` §2.3, phase 2).
//!
//! `$XDG_CONFIG_HOME/worktrees/workspaces.json`, beside `projects.json`, which
//! it references. A workspace is `{id, name, projects: [{root}]}`; the list's
//! order is the nav's. A project is named by its registry `root` and nothing
//! else: a workspace never copies a project's name or settings.
//!
//! **The store never touches a project.** Removing a workspace removes the
//! list. A referenced project that leaves the registry stays referenced (the
//! app shows it as "not registered" and the user drops it by hand); nothing
//! here ever removes it for them, and nothing here writes `projects.json`.
//!
//! **Fields this build does not know are kept.** Later phases add `plan`,
//! `ideas` and `started` to a workspace (and may add to a project ref or the
//! file). Every struct carries a flattened `extra`, so an older build that
//! rewrites the file to rename a workspace does not erase them.
//!
//! **Writes are read-modify-write under a lock, then `fsx::write_atomic`**, the
//! registry's rule (`registry::edit_at`): an flock on a sidecar plus an
//! in-process mutex. A file that does not parse is never overwritten.
//!
//! Every function has an `_at(path)` form, and tests use only those. A process
//! that runs the engine for a test (`registry::forbid_real_writes`) is refused
//! a write to the REAL file, as it is for the registry.

use std::fs;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const FILE: &str = "workspaces.json";
const VERSION: u32 = 1;
/// A name is a nav label. Long enough for anything a person types as one.
pub const NAME_MAX: usize = 80;

static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ProjectRef {
    /// The registry entry's `root` (canonical main root).
    pub root: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Workspace {
    /// A ULID, written once at creation and never changed.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub projects: Vec<ProjectRef>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct WorkspacesFile {
    #[serde(default)]
    pub version: u32,
    /// In display order.
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl WorkspacesFile {
    fn get_mut(&mut self, id: &str) -> Result<&mut Workspace, String> {
        self.workspaces.iter_mut().find(|w| w.id == id).ok_or_else(|| format!("no workspace {id}"))
    }
}

/// `$XDG_CONFIG_HOME/worktrees/workspaces.json`.
pub fn path() -> PathBuf {
    crate::profile::config_root_pub().join(FILE)
}

/// Missing or empty = no workspaces. Anything else that does not parse is an
/// error, so a reader can say so and a writer never clobbers it.
pub fn read_at(p: &Path) -> Result<WorkspacesFile, String> {
    match fs::read(p) {
        Ok(b) if b.iter().all(u8::is_ascii_whitespace) => Ok(WorkspacesFile::default()),
        Ok(b) => serde_json::from_slice(&b)
            .map_err(|e| format!("{} is not valid JSON ({e}) — not overwriting", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WorkspacesFile::default()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

pub fn read() -> Result<WorkspacesFile, String> {
    read_at(&path())
}

/// The guard's rule on its inputs (`registry::refuse_real`'s twin).
fn refuse_real(p: &Path, real: &Path, forbidden: bool) -> Result<(), String> {
    if forbidden && p == real {
        return Err(format!(
            "refused: a test reached the user's real {FILE} ({}). Give the engine under test its own \
             XDG_CONFIG_HOME (in a child's environment, never set_var), or write a temp file with the `*_at` form.",
            p.display()
        ));
    }
    Ok(())
}

/// Read under the lock → mutate → atomic write, only when something changed.
pub fn edit_at<T>(p: &Path, f: impl FnOnce(&mut WorkspacesFile) -> Result<T, String>) -> Result<T, String> {
    refuse_real(p, &path(), crate::registry::real_writes_forbidden())?;
    let _serial = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = p.parent().ok_or("workspaces path has no parent")?;
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
    let mut file = read_at(p)?;
    let before = file.clone();
    let out = f(&mut file)?;
    if file != before {
        file.version = file.version.max(VERSION);
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        crate::fsx::write_atomic(p, json.as_bytes(), None).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    drop(lock);
    Ok(out)
}

fn clean_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("a workspace needs a name".into());
    }
    if n.chars().count() > NAME_MAX {
        return Err(format!("a workspace name is at most {NAME_MAX} characters"));
    }
    if n.chars().any(|c| c.is_control()) {
        return Err("a workspace name cannot hold control characters".into());
    }
    Ok(n.to_string())
}

/// Names are labels the user tells apart by eye, so two may not differ only
/// by case.
fn name_taken(f: &WorkspacesFile, name: &str, except: Option<&str>) -> bool {
    let low = name.to_lowercase();
    f.workspaces.iter().any(|w| Some(w.id.as_str()) != except && w.name.to_lowercase() == low)
}

pub fn create_at(p: &Path, name: &str) -> Result<Workspace, String> {
    let name = clean_name(name)?;
    edit_at(p, |f| {
        if name_taken(f, &name, None) {
            return Err(format!("a workspace named '{name}' already exists"));
        }
        let w = Workspace { id: ulid(), name, ..Default::default() };
        f.workspaces.push(w.clone());
        Ok(w)
    })
}

pub fn rename_at(p: &Path, id: &str, name: &str) -> Result<(), String> {
    let name = clean_name(name)?;
    edit_at(p, |f| {
        if name_taken(f, &name, Some(id)) {
            return Err(format!("a workspace named '{name}' already exists"));
        }
        f.get_mut(id)?.name = name;
        Ok(())
    })
}

/// Removes the list and nothing else. `false` when there was no such id.
pub fn remove_at(p: &Path, id: &str) -> Result<bool, String> {
    edit_at(p, |f| {
        let n = f.workspaces.len();
        f.workspaces.retain(|w| w.id != id);
        Ok(f.workspaces.len() != n)
    })
}

/// Replace a workspace's projects with `roots`, in that order. A root already
/// referenced keeps its ref (and any field a later build put on it); a NEW
/// root must satisfy `can_add` (the caller's "is it registered"), while one
/// already there is kept whether or not it still is — leaving the registry is
/// never a reason to drop it. Duplicates collapse to the first.
pub fn set_projects_at(
    p: &Path,
    id: &str,
    roots: &[String],
    can_add: impl Fn(&str) -> bool,
) -> Result<(), String> {
    edit_at(p, |f| {
        let w = f.get_mut(id)?;
        let mut old = std::mem::take(&mut w.projects);
        let mut next: Vec<ProjectRef> = Vec::with_capacity(roots.len());
        for r in roots {
            if next.iter().any(|x| &x.root == r) {
                continue;
            }
            if let Some(i) = old.iter().position(|x| &x.root == r) {
                next.push(old.remove(i));
            } else if r.is_empty() || !r.starts_with('/') {
                w.projects = old;
                return Err(format!("not a project root: {r:?}"));
            } else if !can_add(r) {
                w.projects = old;
                return Err(format!("not a registered project: {r}"));
            } else {
                next.push(ProjectRef { root: r.clone(), ..Default::default() });
            }
        }
        w.projects = next;
        Ok(())
    })
}

/// Put the workspaces in the order `ids` names. An id that no longer exists is
/// ignored, and one `ids` leaves out keeps its place after the named ones: a
/// reorder computed a moment before a create or remove still lands.
pub fn reorder_at(p: &Path, ids: &[String]) -> Result<(), String> {
    edit_at(p, |f| {
        let mut rest = std::mem::take(&mut f.workspaces);
        let mut next = Vec::with_capacity(rest.len());
        for id in ids {
            if let Some(i) = rest.iter().position(|w| &w.id == id) {
                next.push(rest.remove(i));
            }
        }
        next.extend(rest);
        f.workspaces = next;
        Ok(())
    })
}

pub fn create(name: &str) -> Result<Workspace, String> {
    create_at(&path(), name)
}
pub fn rename(id: &str, name: &str) -> Result<(), String> {
    rename_at(&path(), id, name)
}
pub fn remove(id: &str) -> Result<bool, String> {
    remove_at(&path(), id)
}
pub fn set_projects(id: &str, roots: &[String], can_add: impl Fn(&str) -> bool) -> Result<(), String> {
    set_projects_at(&path(), id, roots, can_add)
}
pub fn reorder(ids: &[String]) -> Result<(), String> {
    reorder_at(&path(), ids)
}

/// A ULID: 48 bits of milliseconds since the epoch, then 80 random bits, in
/// Crockford base32 (26 characters, sortable by creation time).
pub fn ulid() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let rand = ((crate::fsx::random_u64() as u128) << 64 | crate::fsx::random_u64() as u128) & ((1u128 << 80) - 1);
    ulid_from(ms, rand)
}

fn ulid_from(ms: u64, rand80: u128) -> String {
    const ALPHA: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let v: u128 = ((ms as u128 & ((1u128 << 48) - 1)) << 80) | (rand80 & ((1u128 << 80) - 1));
    (0..26).rev().map(|i| ALPHA[((v >> (i * 5)) & 31) as usize] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!(
                "wt-workspaces-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(&d).unwrap();
            Tmp(d)
        }
        fn file(&self) -> PathBuf {
            self.0.join(FILE)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn any(_: &str) -> bool {
        true
    }

    #[test]
    fn the_guard_refuses_only_the_real_file_and_only_when_forbidden() {
        let real = Path::new("/home/u/.config/worktrees/workspaces.json");
        let e = refuse_real(real, real, true).unwrap_err();
        assert!(e.contains("real workspaces.json") && e.contains("XDG_CONFIG_HOME"), "{e}");
        assert!(refuse_real(real, real, false).is_ok());
        assert!(refuse_real(Path::new("/tmp/t/workspaces.json"), real, true).is_ok());
    }

    #[test]
    fn a_ulid_is_26_crockford_characters_led_by_its_time() {
        assert_eq!(ulid_from(0, 0), "0".repeat(26));
        // The spec's own example time, 1469918176385 ms → "01ARYZ6S41".
        assert!(ulid_from(1_469_918_176_385, 0).starts_with("01ARYZ6S41"));
        let a = ulid();
        assert_eq!(a.len(), 26);
        assert!(a.bytes().all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b)), "{a}");
        assert_ne!(ulid(), ulid());
    }

    #[test]
    fn create_rename_reorder_remove_round_trip_in_order() {
        let t = Tmp::new("crud");
        let a = create_at(&t.file(), "  Jellyfin ").unwrap();
        let b = create_at(&t.file(), "Phone").unwrap();
        assert_eq!(a.name, "Jellyfin", "trimmed");
        assert_ne!(a.id, b.id);
        assert!(create_at(&t.file(), "phone").unwrap_err().contains("already exists"), "case-blind");
        assert!(create_at(&t.file(), "   ").is_err());
        assert!(create_at(&t.file(), &"x".repeat(NAME_MAX + 1)).is_err());
        rename_at(&t.file(), &a.id, "Media").unwrap();
        rename_at(&t.file(), &a.id, "media").unwrap(); // its own name, other case
        assert!(rename_at(&t.file(), &a.id, "Phone").is_err());
        assert!(rename_at(&t.file(), "nope", "X").unwrap_err().contains("no workspace"));
        reorder_at(&t.file(), &s(&[&b.id, "gone"])).unwrap();
        let f = read_at(&t.file()).unwrap();
        assert_eq!(f.workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["Phone", "media"]);
        assert_eq!(f.workspaces[1].id, a.id, "the id is written once");
        assert_eq!(f.version, VERSION);
        assert!(remove_at(&t.file(), &b.id).unwrap());
        assert!(!remove_at(&t.file(), &b.id).unwrap());
        assert_eq!(read_at(&t.file()).unwrap().workspaces.len(), 1);
    }

    #[test]
    fn set_projects_adds_only_what_can_be_added_and_keeps_what_is_there() {
        let t = Tmp::new("projects");
        let w = create_at(&t.file(), "W").unwrap();
        let registered = |r: &str| r != "/gone" && r != "/never";
        set_projects_at(&t.file(), &w.id, &s(&["/a", "/b", "/a"]), registered).unwrap();
        let roots = |t: &Tmp| -> Vec<String> {
            read_at(&t.file()).unwrap().workspaces[0].projects.iter().map(|p| p.root.clone()).collect()
        };
        assert_eq!(roots(&t), ["/a", "/b"], "a duplicate collapses");
        assert!(set_projects_at(&t.file(), &w.id, &s(&["/a", "/never"]), registered)
            .unwrap_err()
            .contains("not a registered project"));
        assert_eq!(roots(&t), ["/a", "/b"], "a refused set writes nothing");
        assert!(set_projects_at(&t.file(), &w.id, &s(&["relative"]), any).is_err());
        // "/gone" was added while registered, then left the registry: it stays.
        set_projects_at(&t.file(), &w.id, &s(&["/b", "/a", "/gone"]), any).unwrap();
        set_projects_at(&t.file(), &w.id, &s(&["/gone", "/b"]), registered).unwrap();
        assert_eq!(roots(&t), ["/gone", "/b"], "kept though unregistered; order is the list's");
    }

    #[test]
    fn fields_this_build_does_not_know_survive_a_rewrite() {
        let t = Tmp::new("extra");
        fs::write(
            t.file(),
            r#"{"version":1,"later":{"x":1},"workspaces":[{"id":"01A","name":"J","plan":"01P","ideas":["01I"],
               "started":[{"item":"x"}],"projects":[{"root":"/a","pinned":true}]}]}"#,
        )
        .unwrap();
        rename_at(&t.file(), "01A", "Jelly").unwrap();
        set_projects_at(&t.file(), "01A", &s(&["/b", "/a"]), any).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(t.file()).unwrap()).unwrap();
        assert_eq!(v["later"]["x"], 1);
        let w = &v["workspaces"][0];
        assert_eq!(w["name"], "Jelly");
        assert_eq!(w["plan"], "01P");
        assert_eq!(w["ideas"][0], "01I");
        assert_eq!(w["started"][0]["item"], "x");
        assert_eq!(w["projects"][1]["root"], "/a");
        assert_eq!(w["projects"][1]["pinned"], true, "a project ref's own fields ride along");
    }

    #[test]
    fn an_unreadable_file_is_an_error_and_is_never_overwritten() {
        let t = Tmp::new("bad");
        fs::write(t.file(), "{not json").unwrap();
        assert!(read_at(&t.file()).is_err());
        assert!(create_at(&t.file(), "W").unwrap_err().contains("not overwriting"));
        assert_eq!(fs::read_to_string(t.file()).unwrap(), "{not json");
        fs::write(t.file(), "  \n").unwrap();
        assert_eq!(read_at(&t.file()).unwrap(), WorkspacesFile::default(), "empty is empty");
    }

    #[test]
    fn a_no_op_edit_writes_nothing() {
        let t = Tmp::new("noop");
        let w = create_at(&t.file(), "W").unwrap();
        let m0 = fs::metadata(t.file()).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        rename_at(&t.file(), &w.id, "W").unwrap();
        reorder_at(&t.file(), &s(&[&w.id])).unwrap();
        assert_eq!(fs::metadata(t.file()).unwrap().modified().unwrap(), m0);
    }
}
