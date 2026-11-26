//! Where the workspaces' own folders are, two halves of each:
//!
//! ```text
//! ~/.local/share/worktrees/workspaces/
//!   <workspace id>/            what a chat may read and write: its cwd
//!   .engine/<workspace id>/    the engine's own state, never in a chat's cwd
//!   .removed/                  what a removed workspace had
//! ```
//!
//! The id is the workspace's ULID, so a rename moves nothing and no file
//! stores a reference to the folder. The record of the chats, every lock and
//! the memory's git dir are the engine's half: a file the chat can write is
//! never what the engine trusts.

use std::fs;
use std::path::{Path, PathBuf};

/// Moves the folder that holds every workspace's folder, for a test or a
/// sandbox. Never `XDG_DATA_HOME` (AGENTS.md: Claude's install lives there).
pub const OVERRIDE_ENV: &str = "WORKTREES_WORKSPACES_DIR";

/// The folder that holds every workspace's folder: [`OVERRIDE_ENV`] when it
/// is set and not empty, else [`default_root`].
pub fn root() -> PathBuf {
    match std::env::var_os(OVERRIDE_ENV).filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => default_root(),
    }
}

/// `$XDG_DATA_HOME/worktrees/workspaces` (`~/.local/share/worktrees/workspaces`).
pub fn default_root() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.local/share", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(base).join("worktrees").join("workspaces")
}

/// Refuse a write under the user's real folder when this process runs an
/// engine for a test (`registry::forbid_real_writes`).
pub fn refuse_real(root: &Path) -> Result<(), String> {
    refuse_real_in(root, &default_root(), crate::registry::real_writes_forbidden())
}

fn refuse_real_in(root: &Path, real: &Path, forbidden: bool) -> Result<(), String> {
    if !forbidden {
        return Ok(());
    }
    let same = root == real || matches!((fs::canonicalize(root), fs::canonicalize(real)), (Ok(a), Ok(b)) if a == b);
    if same {
        return Err(format!(
            "refused: a test reached the user's real workspaces folder ({}). Give the engine under test its own \
             folder ({OVERRIDE_ENV} in a child's environment, never set_var, or `Ctx::with_workspaces_dir`), or use \
             the `*_at` forms with a temp dir.",
            real.display()
        ));
    }
    Ok(())
}

/// Workspace `id`'s folder under `root`, the id checked first.
pub fn folder_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    if crate::ulid::is_valid(id) { Ok(root.join(id)) } else { Err(format!("not a workspace id: {id:?}")) }
}

/// Make workspace `id`'s folder, refusing a link where it should be.
pub fn make_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    let dir = folder_at(root, id)?;
    refuse_real(root)?;
    fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    crate::fsx::make_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// The folder of the engine's own half of every workspace, under the root.
pub const ENGINE_DIR: &str = ".engine";

/// Where the engine keeps its own state of workspace `id`, the id checked
/// first. Not made here ([`make_engine_at`]).
pub fn engine_dir_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    folder_at(root, id)?;
    Ok(root.join(ENGINE_DIR).join(id))
}

/// Make workspace `id`'s engine half, its owner's alone, refusing a link on
/// the way.
pub fn make_engine_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = engine_dir_at(root, id)?;
    refuse_real(root)?;
    fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    for d in [&root.join(ENGINE_DIR), &dir] {
        crate::fsx::make_dir(d).map_err(|e| format!("{}: {e}", d.display()))?;
        fs::set_permissions(d, fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(dir)
}

/// The path as it resolves: its longest existing part canonicalised, the rest
/// as written. `fs::canonicalize` alone cannot answer for a folder that is
/// about to be made.
pub(crate) fn resolved(p: &Path) -> PathBuf {
    let mut base = p;
    let mut rest = Vec::new();
    loop {
        if let Ok(c) = fs::canonicalize(base) {
            return rest.iter().rev().fold(c, |p: PathBuf, n| p.join(n));
        }
        match (base.file_name(), base.parent()) {
            (Some(n), Some(up)) => {
                rest.push(n.to_os_string());
                base = up;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Refuse `target` as a project when it is the workspaces folder `root` or
/// anything under it: a workspace's memory is a git repository, and as a
/// project its chats would be taken for its places' agents. `target` need not
/// exist yet.
pub fn refuse_project(target: &Path, root: &Path) -> Result<(), String> {
    if !resolved(target).starts_with(resolved(root)) {
        return Ok(());
    }
    Err(format!(
        "{} is in the workspaces folder ({}), which holds workspaces' memory and chats and is never a project",
        target.display(),
        root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-wsfolder-{name}-{}", crate::ulid::new()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn nothing_in_the_workspaces_folder_is_a_project() {
        let top = tmp("project");
        let outside = top.join("workspaces");
        // Not there yet, by any spelling, and through a link.
        for t in [outside.clone(), outside.join("X").join("memory"), top.join("workspaces").join("new")] {
            assert!(refuse_project(&t, &outside).unwrap_err().contains("never a project"), "{}", t.display());
        }
        fs::create_dir_all(outside.join("W").join("memory")).unwrap();
        let link = top.join("link");
        std::os::unix::fs::symlink(outside.join("W"), &link).unwrap();
        assert!(refuse_project(&link.join("memory"), &outside).is_err(), "through a link");
        assert!(refuse_project(&top.join("proj"), &outside).is_ok());
        assert!(refuse_project(&top.join("workspaces-2"), &outside).is_ok(), "a sibling that shares the name's head");
        let _ = fs::remove_dir_all(&top);
    }

    #[test]
    fn a_test_engine_cannot_write_the_real_folder() {
        let real = tmp("real");
        assert!(refuse_real_in(&real, &real, true).unwrap_err().contains(OVERRIDE_ENV));
        assert!(refuse_real_in(&real, &real, false).is_ok());
        assert!(refuse_real_in(&real.join("other"), &real, true).is_ok());
        let _ = fs::remove_dir_all(&real);
    }

    /// The engine's half is beside every workspace's folder and inside none, so
    /// no chat's cwd holds it, and it is never reached through a link.
    #[test]
    fn the_engines_half_is_in_no_chats_folder_and_behind_no_link() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp("engine").join("workspaces");
        let id = crate::ulid::new();
        assert!(engine_dir_at(&root, "../x").is_err() && make_engine_at(&root, "").is_err());
        let dir = engine_dir_at(&root, &id).unwrap();
        assert!(!dir.starts_with(folder_at(&root, &id).unwrap()) && !dir.exists());
        assert_eq!(make_engine_at(&root, &id).unwrap(), dir);
        assert_eq!(make_engine_at(&root, &id).unwrap(), dir, "again");
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let other = crate::ulid::new();
        std::os::unix::fs::symlink("/tmp", root.join(ENGINE_DIR).join(&other)).unwrap();
        assert!(make_engine_at(&root, &other).is_err());
        let linked = tmp("engine-link");
        std::os::unix::fs::symlink("/tmp", linked.join(ENGINE_DIR)).unwrap();
        assert!(make_engine_at(&linked, &id).is_err());
        assert!(!crate::ulid::is_valid(ENGINE_DIR), "nothing lists it as a workspace");
    }

    #[test]
    fn a_folder_is_named_by_a_ulid_alone_and_never_made_through_a_link() {
        let root = tmp("make").join("workspaces");
        assert!(folder_at(&root, "../x").is_err() && folder_at(&root, "").is_err());
        let id = crate::ulid::new();
        assert_eq!(make_at(&root, &id).unwrap(), root.join(&id));
        let other = crate::ulid::new();
        std::os::unix::fs::symlink("/tmp", root.join(&other)).unwrap();
        assert!(make_at(&root, &other).is_err());
    }
}
