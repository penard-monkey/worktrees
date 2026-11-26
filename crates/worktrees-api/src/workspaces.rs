//! Named workspaces (idea board phase 2): the console rows over
//! `worktrees_core::workspaces`. NOT `workspace.rs`, which is today's
//! "every registered project on this host" (`list_workspace`).
//!
//! Desktop only, by the owner's decision (2026-11-08): every row here is
//! `HostConsole` in serve's table, and there is no MCP tool — save
//! `workspaces_list`, which a paired device reads PROJECTED since the owner's
//! lift of 2026-11-25 (serve's `scope::named_workspaces`), and
//! `workspaces:changed` ([`emit_writes`]). Every row answers
//! with the whole list, each project joined with the registry so the nav can
//! say "not registered" without a second call.
//!
//! The registry is READ here, never written: a workspace references a
//! project and never touches one.

use std::path::Path;

use serde::Serialize;
use worktrees_core::{registry, workspaces as store};

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct WsProject {
    pub root: String,
    /// The registry's name for it, `None` when it is no longer registered.
    pub name: Option<String>,
    pub registered: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct WsRow {
    pub id: String,
    pub name: String,
    pub projects: Vec<WsProject>,
    /// The card's color, a name from `store::COLORS`, or `None` for the
    /// theme's accent. A name the palette does not have (a hand edit) is
    /// answered as `None`, so the page only ever sees one it can draw.
    pub color: Option<String>,
    /// Its plan's id (phase 4), as stored; `plan_read` and
    /// `workspace_overview` resolve it.
    pub plan: Option<String>,
    /// Ideas linked to it as context, by id, as stored.
    pub ideas: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct Workspaces {
    pub workspaces: Vec<WsRow>,
}

fn view(ws: &Path, reg: &Path) -> Result<Workspaces, String> {
    let file = store::read_at(ws)?;
    let reg = registry::read_lenient_at(reg);
    Ok(Workspaces {
        workspaces: file
            .workspaces
            .into_iter()
            .map(|w| WsRow {
                id: w.id,
                name: w.name,
                color: w.color.filter(|c| store::COLORS.contains(&c.as_str())),
                projects: w
                    .projects
                    .into_iter()
                    .map(|p| {
                        let e = reg.by_root(&p.root);
                        WsProject { name: e.map(|e| e.name.clone()), registered: e.is_some(), root: p.root }
                    })
                    .collect(),
                plan: w.plan,
                ideas: w.ideas,
            })
            .collect(),
    })
}

pub fn list_in(ws: &Path, reg: &Path) -> Result<Workspaces, String> {
    view(ws, reg)
}

pub fn create_in(ws: &Path, reg: &Path, name: &str) -> Result<Workspaces, String> {
    store::create_at(ws, name)?;
    view(ws, reg)
}

pub fn rename_in(ws: &Path, reg: &Path, id: &str, name: &str) -> Result<Workspaces, String> {
    store::rename_at(ws, id, name)?;
    view(ws, reg)
}

pub fn remove_in(ws: &Path, reg: &Path, id: &str) -> Result<Workspaces, String> {
    store::remove_at(ws, id)?;
    view(ws, reg)
}

/// A NEW root must be registered; one already in the list stays whether or
/// not it still is.
pub fn set_projects_in(ws: &Path, reg: &Path, id: &str, roots: &[String]) -> Result<Workspaces, String> {
    let r = registry::read_lenient_at(reg);
    store::set_projects_at(ws, id, roots, |root| r.by_root(root).is_some())?;
    view(ws, reg)
}

pub fn reorder_in(ws: &Path, reg: &Path, ids: &[String]) -> Result<Workspaces, String> {
    store::reorder_at(ws, ids)?;
    view(ws, reg)
}

pub fn set_color_in(ws: &Path, reg: &Path, id: &str, color: Option<&str>) -> Result<Workspaces, String> {
    store::set_color_at(ws, id, color)?;
    view(ws, reg)
}

/// The event a write of the workspaces file is (iOS workspaces phase 1).
pub const CHANGED: &str = "workspaces:changed";

/// What [`CHANGED`] carries inside serve: each workspace's id, name, colour
/// and member roots, before and after. Serve's projection reads it to judge
/// whether a device's answer moved, and sends every socket `null`.
fn states(f: &store::WorkspacesFile) -> serde_json::Value {
    serde_json::Value::Array(
        f.workspaces
            .iter()
            .map(|w| serde_json::json!({
                "id": w.id,
                "name": w.name,
                "color": w.color,
                "projects": w.projects.iter().map(|p| serde_json::json!({ "root": p.root })).collect::<Vec<_>>(),
            }))
            .collect(),
    )
}

/// Emit [`CHANGED`] on `ctx` after every write to `path`, for as long as the
/// returned watch lives. Installed once, by `host::start` for the real file:
/// the store's `edit_at` is the one place every writer shares, so no row
/// emits it itself.
pub fn emit_writes(ctx: &crate::Ctx, path: &Path) -> store::WriteWatch {
    let ctx = ctx.clone();
    store::watch_writes(
        path,
        std::sync::Arc::new(move |before, after| ctx.emit(CHANGED, serde_json::json!({ "before": states(before), "after": states(after) }))),
    )
}

fn paths() -> (std::path::PathBuf, std::path::PathBuf) {
    (store::path(), registry::path())
}

pub fn workspaces_list() -> Result<Workspaces, String> {
    let (w, r) = paths();
    list_in(&w, &r)
}
pub fn workspace_create(name: String) -> Result<Workspaces, String> {
    let (w, r) = paths();
    create_in(&w, &r, &name)
}
pub fn workspace_rename(id: String, name: String) -> Result<Workspaces, String> {
    let (w, r) = paths();
    rename_in(&w, &r, &id, &name)
}
pub fn workspace_remove(id: String) -> Result<Workspaces, String> {
    let (w, r) = paths();
    remove_in(&w, &r, &id)
}
pub fn workspace_set_projects(id: String, roots: Vec<String>) -> Result<Workspaces, String> {
    let (w, r) = paths();
    set_projects_in(&w, &r, &id, &roots)
}
pub fn workspace_reorder(ids: Vec<String>) -> Result<Workspaces, String> {
    let (w, r) = paths();
    reorder_in(&w, &r, &ids)
}
pub fn workspace_set_color(id: String, color: Option<String>) -> Result<Workspaces, String> {
    let (w, r) = paths();
    set_color_in(&w, &r, &id, color.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "wt-api-workspaces-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Two workspaces share a project; a project that leaves the registry is
    /// still listed, as not registered; removing a workspace leaves the
    /// registry byte-identical.
    #[test]
    fn workspaces_reference_projects_and_never_touch_the_registry() {
        let d = tmp("ref");
        let (ws, reg) = (d.join("workspaces.json"), d.join("projects.json"));
        fs::write(&reg, r#"{"version":1,"projects":[{"root":"/r/a","name":"a"},{"root":"/r/b","name":"b"}]}"#).unwrap();
        let before = fs::read(&reg).unwrap();
        let v = create_in(&ws, &reg, "One").unwrap();
        let one = v.workspaces[0].id.clone();
        let v = create_in(&ws, &reg, "Two").unwrap();
        let two = v.workspaces[1].id.clone();
        set_projects_in(&ws, &reg, &one, &["/r/a".into(), "/r/b".into()]).unwrap();
        set_projects_in(&ws, &reg, &two, &["/r/a".into()]).unwrap();
        assert!(set_projects_in(&ws, &reg, &two, &["/r/zz".into()]).unwrap_err().contains("not a registered project"));
        // b leaves the registry.
        fs::write(&reg, r#"{"version":1,"projects":[{"root":"/r/a","name":"a"}]}"#).unwrap();
        let before2 = fs::read(&reg).unwrap();
        let v = list_in(&ws, &reg).unwrap();
        assert_eq!(
            v.workspaces[0].projects,
            vec![
                WsProject { root: "/r/a".into(), name: Some("a".into()), registered: true },
                WsProject { root: "/r/b".into(), name: None, registered: false },
            ]
        );
        assert_eq!(v.workspaces[1].projects.len(), 1, "a project can be in many workspaces");
        let v = remove_in(&ws, &reg, &one).unwrap();
        assert_eq!(v.workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["Two"]);
        assert_eq!(fs::read(&reg).unwrap(), before2, "the registry is never written");
        assert_ne!(before, before2);
        let _ = fs::remove_dir_all(d);
    }
    /// `emit_writes` turns a write of the file into ONE `workspaces:changed`
    /// carrying what serve's projection needs, before and after, and nothing
    /// else (no plan, no ideas); a write that changes nothing emits nothing.
    #[test]
    fn a_write_of_the_file_emits_workspaces_changed_once() {
        let d = tmp("emit");
        let (ws, reg) = (d.join("workspaces.json"), d.join("projects.json"));
        fs::write(&reg, r#"{"version":1,"projects":[{"root":"/r/a","name":"a"}]}"#).unwrap();
        let rec = std::sync::Arc::new(crate::tests::Recorder::default());
        let ctx = crate::Ctx::new(d.clone(), rec.clone());
        let watch = emit_writes(&ctx, &ws);
        let id = create_in(&ws, &reg, "One").unwrap().workspaces[0].id.clone();
        set_projects_in(&ws, &reg, &id, &["/r/a".into()]).unwrap();
        rename_in(&ws, &reg, &id, "One").unwrap();
        let events = std::mem::take(&mut *rec.events.lock().unwrap());
        assert_eq!(events.len(), 2, "two writes, and a rename to the same name is none: {events:?}");
        assert!(events.iter().all(|(n, _)| n == CHANGED));
        assert_eq!(
            events[1].1,
            serde_json::json!({
                "before": [{ "id": id, "name": "One", "color": null, "projects": [] }],
                "after": [{ "id": id, "name": "One", "color": null, "projects": [{ "root": "/r/a" }] }],
            })
        );
        drop(watch);
        let _ = fs::remove_dir_all(d);
    }

    /// The console's answer keeps the shape the desktop reads (iOS phase 1
    /// opened the row to devices through serve's projection, never here).
    #[test]
    fn the_console_shape_is_unchanged() {
        let d = tmp("shape");
        let (ws, reg) = (d.join("workspaces.json"), d.join("projects.json"));
        fs::write(&reg, r#"{"version":1,"projects":[{"root":"/r/a","name":"a"}]}"#).unwrap();
        let id = create_in(&ws, &reg, "One").unwrap().workspaces[0].id.clone();
        let v = set_projects_in(&ws, &reg, &id, &["/r/a".into()]).unwrap();
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            format!(r#"{{"workspaces":[{{"id":"{id}","name":"One","projects":[{{"root":"/r/a","name":"a","registered":true}}],"color":null,"plan":null,"ideas":[]}}]}}"#)
        );
        let _ = fs::remove_dir_all(d);
    }

    /// A color is answered with its workspace; one the palette does not have
    /// (a hand edit of the file) is answered as none.
    #[test]
    fn a_workspace_answers_its_color() {
        let d = tmp("color");
        let (ws, reg) = (d.join("workspaces.json"), d.join("projects.json"));
        fs::write(&reg, r#"{"version":1,"projects":[]}"#).unwrap();
        let v = create_in(&ws, &reg, "One").unwrap();
        let id = v.workspaces[0].id.clone();
        assert_eq!(v.workspaces[0].color, None);
        let v = set_color_in(&ws, &reg, &id, Some("purple")).unwrap();
        assert_eq!(v.workspaces[0].color.as_deref(), Some("purple"));
        assert_eq!(serde_json::to_value(&v).unwrap()["workspaces"][0]["color"], "purple");
        assert!(set_color_in(&ws, &reg, &id, Some("chartreuse")).is_err());
        fs::write(&ws, format!(r#"{{"version":1,"workspaces":[{{"id":"{id}","name":"One","color":"chartreuse"}}]}}"#)).unwrap();
        assert_eq!(list_in(&ws, &reg).unwrap().workspaces[0].color, None, "an unknown name reads as none");
        let _ = fs::remove_dir_all(d);
    }
}
