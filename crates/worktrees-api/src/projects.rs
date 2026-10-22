//! The registered project list.
//!
//! Core's registry (`~/.config/worktrees/projects.json`) is the list, shared
//! with the CLI and the MCP server. The app's own older `projects.json` — in
//! its config dir, which is why this needs a `Ctx` — is read once per process
//! and merged in one direction.

use crate::Ctx;

/// Merge the app's old `projects.json` into core's registry, once per process
/// (cross-project §2.2). Core keeps what the file held at the last merge, so
/// this re-adds nothing removed since and still carries across a project a
/// downgraded app added. Logged, never fatal: an unreadable old file merges
/// nothing and the registry is still the list.
pub fn import_app_projects(ctx: &Ctx) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let src = ctx.projects_file();
        match worktrees_core::registry::import_list(&src) {
            Ok(0) => {}
            Ok(n) => crate::log::log("info", &format!("registry: merged {n} project(s) from {}", src.display())),
            Err(e) => crate::log::log("warn", &format!("registry: could not merge {}: {e}", src.display())),
        }
    });
}

/// Registered project roots, in nav order — those `ctx`'s scope reaches. Read
/// fresh every time: another writer (`worktrees projects add`) may have
/// changed it since the last call, and a list held here would write that
/// change away.
///
/// The scope is the caller's (`Ctx::scoped`): serve narrows it to a device's
/// grant, so everything built on this — `project_roots`, `guard_under_projects`,
/// `list_workspace`, the path resolvers — answers with the caller's roots. That
/// is R1's second check (Server Edition Phase 2, §3.3); the engine's own `Ctx`
/// and the console's see every root, as before.
pub fn read_projects(ctx: &Ctx) -> Vec<String> {
    import_app_projects(ctx);
    let mut roots = worktrees_core::registry::read_lenient().roots();
    roots.retain(|r| ctx.scope().covers_root(r));
    roots
}

use std::path::{Path, PathBuf};

/// Canonicalize `path` and require it under some registered project root.
/// canonicalize() also means the path must EXIST — we only ever browse/edit
/// files the tree surfaced, never create arbitrary ones.
///
/// **This is the containment boundary, and it is why it lives here.** It was
/// reachable only through a Tauri command; a client that is not the WebView
/// reaches the same handlers, so a check left on the far side of the IPC is a
/// check a remote client skips (plan §III.3a R2).
pub fn guard_under_projects(ctx: &Ctx, path: &str) -> Result<PathBuf, String> {
    let canon = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
    // Lazily, so a match on the first root does not pay to canonicalize the
    // rest — one of which can be a dead network mount that blocks on stat.
    let hit = read_projects(ctx)
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .any(|r| canon == r || canon.starts_with(&r));
    if hit {
        return Ok(canon);
    }
    Err(format!("path outside workspace: {path}"))
}

/// Registered project roots, canonicalized (unreadable ones simply drop out).
/// Built once per listing, unlike the guard's lazy scan: a directory of links
/// would otherwise re-read the projects file per entry.
pub fn project_roots(ctx: &Ctx) -> Vec<PathBuf> {
    read_projects(ctx).iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect()
}

/// EVERY registered project root, canonicalized, whatever `ctx`'s scope:
/// what a path's CLASS is judged over (`worktrees_core::pathclass`). The class
/// is the strictest over every root that holds the path, so a caller whose
/// scope leaves out a nested project must not get a plainer answer than the
/// console would: the root it cannot see still makes `.worktrees.places.json`
/// worktrees' own. Containment stays the scope's question
/// (`guard_under_projects`).
pub fn class_roots(ctx: &Ctx) -> Vec<PathBuf> {
    import_app_projects(ctx);
    worktrees_core::registry::read_lenient().roots().iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect()
}

/// The containment test `guard_under_projects` applies, against an ALREADY
/// canonical path. Split out so the tree can ask the same question about a
/// symlink target without going through a command that would reject it.
pub fn under_roots(roots: &[PathBuf], canon: &Path) -> bool {
    roots.iter().any(|r| canon == r || canon.starts_with(r))
}

/// Refuse a write that NO caller may make, the desktop included: a path of
/// class `Forbidden` (`worktrees_core::pathclass`, proposal §7.1). `canon` is
/// what `guard_under_projects` returned, so a `notes.md -> .git/config` link
/// is judged by what it writes; `shown` is what the caller asked for.
///
/// "Under some root" (`guard_under_projects`) was the whole write rule until
/// this, and `.git/config` is under a root: `core.fsmonitor` in it runs at the
/// next git spawn, and the engine's poll spawns git every 3 s (gap 1).
pub fn forbid_write(roots: &[PathBuf], canon: &Path, shown: &str) -> Result<(), String> {
    use worktrees_core::pathclass::{self, Access};
    match pathclass::classify_canonical(roots, canon) {
        Some(c) if c.refuses_every_caller(Access::Write) => Err(forbidden_message(shown, canon)),
        _ => Ok(()),
    }
}

/// `forbid_write` for a path that does not exist YET — a project to create,
/// a clone's target, a directory to `git init`. The deepest ancestor that
/// exists is canonicalised and the rest re-appended, so a link on the way is
/// judged by where it leads. A `.git` component refuses even outside every
/// registered root: no new project belongs inside a repository's git
/// directory.
pub fn forbid_new_path(roots: &[PathBuf], target: &Path) -> Result<(), String> {
    let shown = target.to_string_lossy().into_owned();
    let mut base = target.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let canon = loop {
        if let Ok(c) = std::fs::canonicalize(&base) {
            break rest.iter().rev().fold(c, |p, n| p.join(n));
        }
        match (base.file_name().map(|n| n.to_os_string()), base.parent()) {
            (Some(n), Some(p)) => {
                rest.push(n);
                base = p.to_path_buf();
            }
            // Nothing on the way exists, so nothing on the way can be a link.
            _ => break target.to_path_buf(),
        }
    };
    if worktrees_core::pathclass::has_git_component(&canon) {
        return Err(forbidden_message(&shown, &canon));
    }
    forbid_write(roots, &canon, &shown)
}

/// Why a `Forbidden` write is refused, in words for the person at the desk:
/// `shown` is what they asked for, `canon` where it leads. serve's resolver
/// refuses such a write before this body runs, with these same words.
pub fn forbidden_message(shown: &str, canon: &Path) -> String {
    if worktrees_core::pathclass::has_git_component(canon) {
        format!(
            "{shown}: worktrees never writes inside .git, for any caller — a change to its config or \
             hooks runs code the next time git runs, and worktrees runs git every few seconds. \
             Use a terminal if you mean to."
        )
    } else {
        format!(
            "{shown}: worktrees never writes this file directly — it is worktrees' own state \
             (.worktrees/ or .worktrees.places.json), changed only through its own controls."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project, a clone or a `git init` aimed into a repository's `.git` or
    /// into a project's `.worktrees/` is refused before anything is created —
    /// including through a link on the way, which the fixture plants, and
    /// including a `.git` under no registered root at all.
    #[test]
    fn a_new_path_inside_git_or_the_tools_tree_is_refused() {
        let base = std::env::temp_dir().join(format!("wt-forbid-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join(".git/hooks")).unwrap();
        std::fs::create_dir_all(repo.join(".worktrees")).unwrap();
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::create_dir_all(base.join("other/.git")).unwrap();
        std::os::unix::fs::symlink(repo.join(".git"), base.join("gitlink")).unwrap();
        let roots = vec![std::fs::canonicalize(&repo).unwrap()];

        for t in [
            repo.join(".git/hooks/post-checkout"),
            repo.join(".git/newproj/deeper"),
            repo.join(".worktrees/newproj"),
            base.join("gitlink/newproj"),
            base.join("other/.git/newproj"),
        ] {
            let err = forbid_new_path(&roots, &t).expect_err(&t.to_string_lossy());
            assert!(err.contains("never writes"), "{}: {err}", t.display());
            assert!(!t.exists(), "{} was created", t.display());
        }
        forbid_new_path(&roots, &repo.join("sub/newproj")).unwrap();
        forbid_new_path(&roots, &base.join("elsewhere/newproj")).unwrap();
        forbid_new_path(&roots, Path::new("/nonexistent-root-xyz/a/b")).unwrap();

        let _ = std::fs::remove_dir_all(&base);
    }
}
