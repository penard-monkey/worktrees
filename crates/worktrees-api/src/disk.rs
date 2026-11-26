//! Disk usage of places (`worktrees_core::diskusage`): the Disk view's three
//! rows. A scan walks every file of a project's places, seconds to a minute,
//! so it runs only when asked and is cached in the engine until `refresh`.

use std::path::{Path, PathBuf};

use worktrees_core::diskusage::{self, Deleted, PlaceTree, ProjectUsage};
use worktrees_core::Project;

use crate::Ctx;

fn place(repo: &str, slug: &str) -> Result<(Project, PathBuf, bool), String> {
    let project = Project::discover(Path::new(repo)).map_err(|e| e.to_string())?;
    if slug != "(main)" && (slug.is_empty() || slug.contains('/') || slug.starts_with('-') || slug == ".." || slug == ".") {
        return Err(format!("not a place: {slug}"));
    }
    let root = PathBuf::from(project.place_dir(slug));
    Ok((project, root, slug == "(main)"))
}

/// Every place of the project: own, shared and du-style bytes, and the folders
/// a cleanup may offer. The cached scan unless `refresh` or none yet; `cached`
/// answers `null` rather than scanning.
pub fn disk_usage(_ctx: &Ctx, repo: String, refresh: Option<bool>, cached: Option<bool>) -> Result<Option<ProjectUsage>, String> {
    let project = Project::discover(Path::new(&repo)).map_err(|e| e.to_string())?;
    if cached.unwrap_or(false) {
        return Ok(diskusage::cached_usage(&project.main_root).map(|u| (*u).clone()));
    }
    Ok(Some((*diskusage::project_usage(&project, refresh.unwrap_or(false))).clone()))
}

/// One place's file tree, pruned for the treemap. Always a fresh walk.
pub fn disk_place_tree(_ctx: &Ctx, repo: String, slug: String) -> Result<PlaceTree, String> {
    let (_, root, is_main) = place(&repo, &slug)?;
    if !root.is_dir() {
        return Err(format!("no place {slug}"));
    }
    Ok(diskusage::scan_place_tree(&root, &slug, is_main))
}

/// Delete one path git ignores from a place, permanently (`delete_refusal`
/// has the rules). `scanned` is the scan's identity of the path (`Node::id`): a
/// path swapped since the scan is refused.
pub fn disk_delete(
    _ctx: &Ctx,
    repo: String,
    slug: String,
    path: String,
    scanned: Option<String>,
) -> Result<Deleted, String> {
    let (project, root, is_main) = place(&repo, &slug)?;
    let expect = match scanned.as_deref() {
        Some(s) => Some(diskusage::parse_id(s).ok_or_else(|| format!("not a scan identity: {s}"))?),
        None => None,
    };
    let r = diskusage::delete_in_place(&project.main_root, &root, is_main, &path, expect);
    match &r {
        Ok(d) => crate::log::info(&format!("disk: deleted {slug}/{} ({} bytes, {} files)", d.path, d.bytes, d.files)),
        Err(e) => crate::log::info(&format!("disk: refused {slug}/{path}: {e}")),
    }
    r
}
