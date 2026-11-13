//! Plans and their workspaces (idea board phase 4, §2.2, §2.3): the console
//! rows over `worktrees_core::ideas`' plan operations, the plan reader
//! (`worktrees_core::plans`) and a workspace's `plan` and `ideas`.
//!
//! Desktop only, like every ideas and workspaces row: serve declares each
//! one `HostConsole`. Every act is one operation with plain parameters (§7.1:
//! an agent's proposal later is the same call), and every change the app
//! makes to a plan copies it to `.history/` first.
//!
//! **Matching a plan's project to the registry** (`- <name> (registered)`):
//! a name that is a path (`/…` or `~/…`) matches the entry whose `root` it
//! is; any other name matches the entry whose registry `name` it is, when
//! exactly ONE entry has it. Anything else is `not-registered`, said, never
//! guessed. `clone:` and `new:` lines are `to-clone` and `to-create`, which
//! phase 5 acts on; here they are only listed.

use std::path::{Path, PathBuf};

use serde::Serialize;
use worktrees_core::ideas::{self, Doc, Kind, Plan, PlanIdea};
use worktrees_core::plans::{Problem, Source, WorkItem};
use worktrees_core::{registry, workspaces as store};

use crate::workspaces::{list_in as ws_list_in, Workspaces};
use crate::Ctx;

/// One project line of a plan, judged against this host's registry.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ProjectState {
    pub line: usize,
    pub name: String,
    /// `registered`, `not-registered` (it says registered, and no entry
    /// matches), `to-clone` or `to-create`.
    pub state: &'static str,
    /// The registered root it matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The workspace a plan has, by id and name.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct WsRef {
    pub id: String,
    pub name: String,
}

/// `plan_read`'s answer: the plan ([`Plan`]: the document, its ideas, its
/// two sections and their problems), each project's state, and its
/// workspace when it has one.
#[derive(Serialize, Debug)]
pub struct PlanView {
    #[serde(flatten)]
    pub plan: Plan,
    pub project_states: Vec<ProjectState>,
    pub workspace: Option<WsRef>,
}

fn expand(name: &str) -> Option<String> {
    if name.starts_with('/') {
        return Some(name.trim_end_matches('/').to_string());
    }
    let rest = name.strip_prefix("~/")?;
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(format!("{}/{}", home.trim_end_matches('/'), rest.trim_end_matches('/')))
}

/// The registered root a plan's `- <name> (registered)` names (module doc).
pub fn match_registered(reg: &registry::Registry, name: &str) -> Option<String> {
    if let Some(path) = expand(name) {
        return reg.by_root(&path).map(|e| e.root.clone());
    }
    let mut hits = reg.projects.iter().filter(|e| e.name == name);
    match (hits.next(), hits.next()) {
        (Some(e), None) => Some(e.root.clone()),
        _ => None,
    }
}

fn states(plan: &Plan, reg: &registry::Registry) -> Vec<ProjectState> {
    plan.sections
        .projects
        .iter()
        .flatten()
        .map(|p| {
            let base = ProjectState { line: p.line, name: p.name.clone(), state: "", root: None, url: None, path: None };
            match &p.source {
                Source::Registered => match match_registered(reg, &p.name) {
                    Some(root) => ProjectState { state: "registered", root: Some(root), ..base },
                    None => ProjectState { state: "not-registered", ..base },
                },
                Source::Clone { url } => ProjectState { state: "to-clone", url: Some(url.clone()), ..base },
                Source::New { path } => ProjectState { state: "to-create", path: Some(path.clone()), ..base },
            }
        })
        .collect()
}

/// The three files a row reads: the ideas folder, `workspaces.json` and the
/// registry. A test hands its own.
#[derive(Clone, Debug)]
pub struct Files {
    pub ideas: PathBuf,
    pub ws: PathBuf,
    pub reg: PathBuf,
}

impl Files {
    pub fn of(ctx: &Ctx) -> Files {
        Files { ideas: ctx.ideas_dir().to_path_buf(), ws: store::path(), reg: registry::path() }
    }
}

/// The live id of the PLAN `id` names (an absorbed id resolves), refused
/// when that document is not a plan.
fn live_plan(root: &Path, id: &str) -> Result<(String, String), String> {
    let idx = ideas::list_at(root)?;
    let d = idx.resolve(id)?;
    if d.kind != Kind::Plan {
        return Err(format!("“{}” is not a plan", d.title));
    }
    Ok((d.id.clone().unwrap_or_default(), d.title.clone()))
}

pub fn read_in(f: &Files, id: &str) -> Result<PlanView, String> {
    let plan = ideas::plan_read_at(&f.ideas, id)?;
    let reg = registry::read_lenient_at(&f.reg);
    let live = plan.doc.idea.id.clone().unwrap_or_default();
    let file = store::read_at(&f.ws)?;
    let workspace = store::for_plan(&file, &live).map(|w| WsRef { id: w.id.clone(), name: w.name.clone() });
    Ok(PlanView { project_states: states(&plan, &reg), plan, workspace })
}

/// Add the registered project `root` to the plan's `## Projects` as
/// `- <registry name> (registered)`.
pub fn add_project_in(f: &Files, id: &str, root: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    let reg = registry::read_lenient_at(&f.reg);
    let e = reg.by_root(root).ok_or_else(|| format!("not a registered project: {root}"))?;
    // The name the reader will match back to this root: its registry name
    // when that is unique, else its root.
    let name = if match_registered(&reg, &e.name).as_deref() == Some(e.root.as_str()) { e.name.clone() } else { e.root.clone() };
    ideas::plan_add_project_at(&f.ideas, id, &name, expected_mtime)
}

/// The name "Make workspace" gives a workspace: the plan's title without
/// its `Plan:`.
fn ws_name(title: &str) -> String {
    let t = title.trim();
    let t = if t.len() >= 5 && t[..5].eq_ignore_ascii_case("plan:") { t[5..].trim() } else { t };
    if t.is_empty() { "Plan".to_string() } else { t.chars().take(store::NAME_MAX).collect() }
}

/// `workspace_from_plan`'s answer: the list, the workspace, and whether it
/// was made now (`false`: the plan already had it, and it is opened).
#[derive(Serialize, Debug)]
pub struct FromPlan {
    #[serde(flatten)]
    pub list: Workspaces,
    pub id: String,
    pub created: bool,
}

pub fn from_plan_in(f: &Files, plan: &str) -> Result<FromPlan, String> {
    let v = read_in(f, plan)?;
    let live = v.plan.doc.idea.id.clone().unwrap_or_default();
    let ideas: Vec<String> = v.plan.ideas.iter().filter_map(|i| i.live.clone()).collect();
    let roots: Vec<String> = v.project_states.iter().filter_map(|p| p.root.clone()).collect();
    let reg = registry::read_lenient_at(&f.reg);
    let (w, created) = store::from_plan_at(&f.ws, &ws_name(&v.plan.doc.idea.title), &live, &ideas, &roots, |r| reg.by_root(r).is_some())?;
    Ok(FromPlan { list: ws_list_in(&f.ws, &f.reg)?, id: w.id, created })
}

pub fn set_plan_in(f: &Files, id: &str, plan: Option<&str>) -> Result<Workspaces, String> {
    let live = match plan {
        Some(p) => Some(live_plan(&f.ideas, p)?.0),
        None => None,
    };
    store::set_plan_at(&f.ws, id, live.as_deref())?;
    ws_list_in(&f.ws, &f.reg)
}

pub fn link_idea_in(f: &Files, id: &str, idea: &str) -> Result<Workspaces, String> {
    let idx = ideas::list_at(&f.ideas)?;
    let d = idx.resolve(idea)?;
    if d.kind == Kind::Plan {
        return Err(format!("“{}” is a plan: set it as the workspace's plan", d.title));
    }
    store::link_idea_at(&f.ws, id, d.id.as_deref().unwrap_or_default())?;
    ws_list_in(&f.ws, &f.reg)
}

pub fn unlink_idea_in(f: &Files, id: &str, idea: &str) -> Result<Workspaces, String> {
    store::unlink_idea_at(&f.ws, id, idea)?;
    ws_list_in(&f.ws, &f.reg)
}

/// The workspace's plan as the overview shows it.
#[derive(Serialize, Debug)]
pub struct OvPlan {
    /// The id the workspace holds.
    pub id: String,
    /// The plan it resolves to, or `None` when it is gone (removed).
    pub live: Option<String>,
    pub title: Option<String>,
    pub file: Option<String>,
    /// Why it could not be read, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `workspace_overview`'s answer (§9.2): the workspace's plan, its linked
/// ideas, and the plan's Projects and Work as the reader gives them, each
/// project judged against the registry and the workspace. Read-only.
#[derive(Serialize, Debug)]
pub struct Overview {
    pub id: String,
    pub name: String,
    pub plan: Option<OvPlan>,
    pub ideas: Vec<PlanIdea>,
    pub projects: Option<Vec<OvProject>>,
    pub work: Option<Vec<WorkItem>>,
    pub problems: Vec<Problem>,
}

#[derive(Serialize, Debug)]
pub struct OvProject {
    #[serde(flatten)]
    pub state: ProjectState,
    /// Its matched root is one of the workspace's projects.
    pub in_workspace: bool,
}

pub fn overview_in(f: &Files, id: &str) -> Result<Overview, String> {
    let file = store::read_at(&f.ws)?;
    let w = file.workspaces.iter().find(|w| w.id == id).ok_or_else(|| format!("no workspace {id}"))?;
    let idx = ideas::list_at(&f.ideas)?;
    let ideas: Vec<PlanIdea> = w
        .ideas
        .iter()
        .map(|i| match idx.lookup(i) {
            Ok((d, via)) => PlanIdea { id: i.clone(), live: d.id.clone(), title: Some(d.title.clone()), merged: via },
            Err(_) => PlanIdea { id: i.clone(), live: None, title: None, merged: false },
        })
        .collect();
    let mut out = Overview { id: w.id.clone(), name: w.name.clone(), plan: None, ideas, projects: None, work: None, problems: Vec::new() };
    if let Some(pid) = &w.plan {
        match read_in(f, pid) {
            Ok(v) => {
                let roots: Vec<&str> = w.projects.iter().map(|p| p.root.as_str()).collect();
                out.plan = Some(OvPlan {
                    id: pid.clone(),
                    live: v.plan.doc.idea.id.clone(),
                    title: Some(v.plan.doc.idea.title.clone()),
                    file: Some(v.plan.doc.idea.file.clone()),
                    error: None,
                });
                out.projects = v.plan.sections.projects.as_ref().map(|_| {
                    v.project_states
                        .iter()
                        .map(|s| OvProject { in_workspace: s.root.as_deref().is_some_and(|r| roots.contains(&r)), state: s.clone() })
                        .collect()
                });
                out.work = v.plan.sections.work.clone();
                out.problems = v.plan.sections.problems.clone();
            }
            Err(e) => out.plan = Some(OvPlan { id: pid.clone(), live: None, title: None, file: None, error: Some(e) }),
        }
    }
    Ok(out)
}

// ── the rows ────────────────────────────────────────────────────────────────

/// Make a plan from `ideas` (one or more): the template, or `draft` (the
/// body after the frontmatter).
pub fn plan_make(ctx: &Ctx, ideas: Vec<String>, title: Option<String>, draft: Option<String>) -> Result<Doc, String> {
    ideas::make_plan_at(ctx.ideas_dir(), &ideas, title.as_deref(), draft.as_deref())
}

pub fn plan_read(ctx: &Ctx, id: String) -> Result<PlanView, String> {
    read_in(&Files::of(ctx), &id)
}

pub fn plan_add_ideas(ctx: &Ctx, id: String, ideas: Vec<String>, expected_mtime: Option<u64>) -> Result<Doc, String> {
    ideas::plan_add_ideas_at(ctx.ideas_dir(), &id, &ideas, expected_mtime)
}

pub fn plan_remove_idea(ctx: &Ctx, id: String, idea: String, expected_mtime: Option<u64>) -> Result<Doc, String> {
    ideas::plan_remove_idea_at(ctx.ideas_dir(), &id, &idea, expected_mtime)
}

pub fn plan_add_project(ctx: &Ctx, id: String, root: String, expected_mtime: Option<u64>) -> Result<Doc, String> {
    add_project_in(&Files::of(ctx), &id, &root, expected_mtime)
}

pub fn workspace_from_plan(ctx: &Ctx, plan: String) -> Result<FromPlan, String> {
    from_plan_in(&Files::of(ctx), &plan)
}

pub fn workspace_set_plan(ctx: &Ctx, id: String, plan: Option<String>) -> Result<Workspaces, String> {
    set_plan_in(&Files::of(ctx), &id, plan.as_deref())
}

pub fn workspace_link_idea(ctx: &Ctx, id: String, idea: String) -> Result<Workspaces, String> {
    link_idea_in(&Files::of(ctx), &id, &idea)
}

pub fn workspace_unlink_idea(ctx: &Ctx, id: String, idea: String) -> Result<Workspaces, String> {
    unlink_idea_in(&Files::of(ctx), &id, &idea)
}

pub fn workspace_overview(ctx: &Ctx, id: String) -> Result<Overview, String> {
    overview_in(&Files::of(ctx), &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn files(tag: &str) -> (PathBuf, Files) {
        let d = std::env::temp_dir().join(format!("wt-api-plans-{tag}-{}-{:x}", std::process::id(), worktrees_core::fsx::random_u64()));
        fs::create_dir_all(&d).unwrap();
        let f = Files { ideas: d.join("ideas"), ws: d.join("workspaces.json"), reg: d.join("projects.json") };
        (d, f)
    }

    fn register(f: &Files, entries: &[(&str, &str)]) {
        let projects: Vec<serde_json::Value> = entries.iter().map(|(r, n)| serde_json::json!({"root": r, "name": n})).collect();
        fs::write(&f.reg, serde_json::json!({"version": 1, "projects": projects}).to_string()).unwrap();
    }

    fn mk(f: &Files, title: &str) -> String {
        ideas::create_at(&f.ideas, title).unwrap().idea.id.unwrap()
    }

    /// The Jellyfin example of §2.2, end to end through the bodies: two
    /// ideas, a plan, its projects judged, a workspace made once.
    #[test]
    fn the_jellyfin_example_from_two_ideas_to_a_workspace() {
        let (d, f) = files("jellyfin");
        register(&f, &[("/w/worktrees", "worktrees"), ("/w/a/dup", "dup"), ("/w/b/dup", "dup")]);
        let (a, b) = (mk(&f, "Jellyfin Sharing"), mk(&f, "Home server GPU"));
        let p = ideas::make_plan_at(&f.ideas, &[a.clone(), b.clone()], Some("Jellyfin on the home server"), None).unwrap();
        let pid = p.idea.id.clone().unwrap();
        let body = p.content.replace(
            "## Projects\n",
            "## Projects\n- worktrees (registered)\n- dup (registered)\n- /w/b/dup (registered)\n- homelab-ansible (clone: git@github.com:david/homelab-ansible.git)\n- jellyfin-config (new: ~/workspace/jellyfin-config)\n",
        );
        ideas::write_at(&f.ideas, &pid, &body, None).unwrap();
        let v = read_in(&f, &pid).unwrap();
        let st: Vec<(&str, &str, Option<&str>)> = v.project_states.iter().map(|s| (s.name.as_str(), s.state, s.root.as_deref())).collect();
        assert_eq!(
            st,
            vec![
                ("worktrees", "registered", Some("/w/worktrees")),
                ("dup", "not-registered", None),
                ("/w/b/dup", "registered", Some("/w/b/dup")),
                ("homelab-ansible", "to-clone", None),
                ("jellyfin-config", "to-create", None),
            ],
            "a name matches when unique; a path matches its root"
        );
        assert!(v.workspace.is_none());
        let made = from_plan_in(&f, &pid).unwrap();
        assert!(made.created);
        let w = made.list.workspaces.iter().find(|w| w.id == made.id).unwrap();
        assert_eq!(w.name, "Jellyfin on the home server", "named after the plan, without `Plan:`");
        assert_eq!(w.projects.iter().map(|p| p.root.as_str()).collect::<Vec<_>>(), ["/w/worktrees", "/w/b/dup"]);
        assert_eq!((w.plan.as_deref(), w.ideas.clone()), (Some(pid.as_str()), vec![a.clone(), b.clone()]));
        let again = from_plan_in(&f, &pid).unwrap();
        assert_eq!((again.id.as_str(), again.created), (made.id.as_str(), false), "a second Make workspace opens the first");
        assert_eq!(again.list.workspaces.len(), 1);
        assert_eq!(read_in(&f, &pid).unwrap().workspace.map(|w| w.id), Some(made.id.clone()));
        let o = overview_in(&f, &made.id).unwrap();
        assert_eq!(o.plan.as_ref().and_then(|p| p.title.as_deref()), Some("Plan: Jellyfin on the home server"));
        let ps = o.projects.unwrap();
        assert_eq!(ps.iter().map(|p| (p.state.name.as_str(), p.in_workspace)).collect::<Vec<_>>()[..3], [("worktrees", true), ("dup", false), ("/w/b/dup", true)]);
        assert_eq!(o.ideas.iter().map(|i| i.title.as_deref().unwrap()).collect::<Vec<_>>(), ["Jellyfin Sharing", "Home server GPU"]);
        assert_eq!(o.work, Some(vec![]));
        // A project added from the registry reads back as itself.
        let d2 = add_project_in(&f, &pid, "/w/a/dup", None).unwrap();
        assert!(d2.content.contains("- /w/a/dup (registered)\n"), "an ambiguous name is written as its root: {}", d2.content);
        let d3 = add_project_in(&f, &pid, "/w/worktrees", None).unwrap();
        assert_eq!(d3.content, d2.content, "already listed by name");
        assert!(add_project_in(&f, &pid, "/w/nope", None).unwrap_err().contains("not a registered project"));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_workspaces_plan_and_ideas_are_set_by_live_id_and_refused_otherwise() {
        let (d, f) = files("link");
        register(&f, &[]);
        let (a, b) = (mk(&f, "A"), mk(&f, "B"));
        let p = ideas::make_plan_at(&f.ideas, std::slice::from_ref(&a), None, None).unwrap().idea.id.unwrap();
        let w1 = store::create_at(&f.ws, "One").unwrap().id;
        let w2 = store::create_at(&f.ws, "Two").unwrap().id;
        assert!(set_plan_in(&f, &w1, Some(&a)).unwrap_err().contains("is not a plan"));
        set_plan_in(&f, &w1, Some(&p)).unwrap();
        assert!(set_plan_in(&f, &w2, Some(&p)).unwrap_err().contains("already has a workspace, 'One'"));
        assert!(link_idea_in(&f, &w2, &p).unwrap_err().contains("is a plan"));
        ideas::merge_at(&f.ideas, &a, &b, None, None, None).unwrap();
        let l = link_idea_in(&f, &w2, &b).unwrap();
        assert_eq!(l.workspaces[1].ideas, vec![a.clone()], "a merged-away id is linked as the idea that holds it");
        let l = unlink_idea_in(&f, &w2, &a).unwrap();
        assert!(l.workspaces[1].ideas.is_empty());
        // The plan's file removed: the overview says so and still answers.
        ideas::remove_at(&f.ideas, &p, None).unwrap();
        let o = overview_in(&f, &w1).unwrap();
        assert!(o.plan.unwrap().error.is_some());
        let _ = fs::remove_dir_all(d);
    }
}
