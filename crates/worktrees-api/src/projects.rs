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

/// Registered project roots, in nav order. Read fresh every time: another
/// writer (`worktrees projects add`) may have changed it since the last call,
/// and a list held here would write that change away.
pub fn read_projects(ctx: &Ctx) -> Vec<String> {
    import_app_projects(ctx);
    worktrees_core::registry::read_lenient().roots()
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

/// The containment test `guard_under_projects` applies, against an ALREADY
/// canonical path. Split out so the tree can ask the same question about a
/// symlink target without going through a command that would reject it.
pub fn under_roots(roots: &[PathBuf], canon: &Path) -> bool {
    roots.iter().any(|r| canon == r || canon.starts_with(r))
}
