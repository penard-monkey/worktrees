//! Where a workspace's folder is (`docs/proposals/workspace-chats.md` §1.1):
//! `<data dir>/worktrees/workspaces/<workspace id>/`. The id is the
//! workspace's ULID, so a rename moves nothing, and no reference to the folder
//! is stored anywhere.
//!
//! Every function that touches a workspace's folder takes the ROOT these
//! folders sit in, so a test hands it a temp dir. An engine's root is
//! `Ctx::workspaces_dir`: [`root`] at its start, which [`OVERRIDE_ENV`] moves
//! for a spawned engine — never `XDG_DATA_HOME`, which moves Claude's own
//! install with it (AGENTS.md).

use std::path::{Path, PathBuf};

/// The variable that names the folder the workspaces' folders sit in.
pub const OVERRIDE_ENV: &str = "WORKTREES_WORKSPACES_DIR";

/// [`OVERRIDE_ENV`] when it is set and not empty, else [`default_root`].
pub fn root() -> PathBuf {
    match std::env::var_os(OVERRIDE_ENV).filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => default_root(),
    }
}

/// `$XDG_DATA_HOME/worktrees/workspaces`, or
/// `~/.local/share/worktrees/workspaces`: the user's real folders, which the
/// test guard protects whatever the override says.
pub fn default_root() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.local/share", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(base).join("worktrees").join("workspaces")
}

/// Workspace `id`'s folder under `root`. The id becomes a path component, so
/// it must be a ULID whatever a hand-edited `workspaces.json` says.
pub fn folder_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !crate::ulid::is_valid(id) {
        return Err(format!("not a workspace id: {id}"));
    }
    Ok(root.join(id))
}

/// `Err` for a write under `root` when it is the user's real one and this
/// process runs an engine for a test (`registry::forbid_real_writes`).
pub fn refuse_real(root: &Path) -> Result<(), String> {
    refuse_real_in(root, &default_root(), crate::registry::real_writes_forbidden())
}

fn refuse_real_in(root: &Path, real: &Path, forbidden: bool) -> Result<(), String> {
    if !forbidden {
        return Ok(());
    }
    let same = root == real || matches!((std::fs::canonicalize(root), std::fs::canonicalize(real)), (Ok(a), Ok(b)) if a == b);
    if same {
        return Err(format!(
            "refused: a test reached the user's real workspaces folder ({}). Give the engine under test its own \
             ({OVERRIDE_ENV} in a child's environment, never set_var, or `Ctx::with_workspaces_dir`), or use the \
             `*_at` forms with a temp dir.",
            real.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_named_by_a_ulid_and_nothing_else() {
        let root = Path::new("/r");
        let id = crate::ulid::new();
        assert_eq!(folder_at(root, &id).unwrap(), root.join(&id));
        for bad in ["", "..", "../x", ".removed", "personal", "/etc"] {
            assert!(folder_at(root, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_test_is_refused_the_real_folder_and_no_other() {
        let real = Path::new("/nope/real");
        assert!(refuse_real_in(real, real, true).unwrap_err().contains("real workspaces folder"));
        assert!(refuse_real_in(Path::new("/nope/other"), real, true).is_ok());
        assert!(refuse_real_in(real, real, false).is_ok());
    }
}
