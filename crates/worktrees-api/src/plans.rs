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
//! guessed. `clone:` and `new:` lines are `to-clone` and `to-create`, except
//! a `new:` whose folder IS a registered root, which is `registered`.
//!
//! **Phase 5: what a plan does.** A `clone:` or `new:` project arrives through
//! the Add project flow's own rows (`clone_project`, `create_project`), and
//! [`plan_set_project`] then makes its line `(registered)` and references it
//! in the plan's workspace. A `## Work` item starts a place through
//! [`plan_start`]: the brief is a parameter (§7.1), the text the person
//! reviewed, and goes down `new --brief`'s path; only then is the item's
//! `started:` line written. An item's project is judged by its own line in
//! `## Projects` when it has one, else by the rule above. What is there, busy
//! or gone is the snapshot's answer by (root, slug), never stored.

use std::path::{Path, PathBuf};

use serde::Serialize;
use worktrees_core::argv::{branch_arg, safe_arg};
use worktrees_core::ideas::{self, Doc, Kind, Plan, PlanIdea};
use worktrees_core::plans::{self as reader, Problem, Source, Started, WorkItem};
use worktrees_core::{registry, workspaces as store, Project};

use crate::log::log as applog;
use crate::ops::CmdResult;
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

/// A work item judged against the registry (phase 5): where its place would
/// go, and which place it has. What is THERE is the snapshot's answer.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct WorkState {
    pub line: usize,
    /// Its `project:` as a project's state (`registered` has `root`); `None`
    /// when the item names no project.
    pub project_state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The registered root of the place its `started:` line names, when that
    /// project is registered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_root: Option<String>,
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
    /// One per `## Work` item, in order.
    pub work_states: Vec<WorkState>,
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
                Source::New { path } => match expand(path).and_then(|p| reg.by_root(&p).map(|e| e.root.clone())) {
                    Some(root) => ProjectState { state: "registered", root: Some(root), path: Some(path.clone()), ..base },
                    None => ProjectState { state: "to-create", path: Some(path.clone()), ..base },
                },
            }
        })
        .collect()
}

/// Where a work item's `project:` points: its line in `## Projects` when the
/// plan has one of that name, else the registry ([`match_registered`]).
fn project_of(states: &[ProjectState], reg: &registry::Registry, name: &str) -> (&'static str, Option<String>) {
    match states.iter().find(|s| s.name == name) {
        Some(s) => (s.state, s.root.clone()),
        None => match match_registered(reg, name) {
            Some(root) => ("registered", Some(root)),
            None => ("not-registered", None),
        },
    }
}

fn work_states(plan: &Plan, states: &[ProjectState], reg: &registry::Registry) -> Vec<WorkState> {
    plan.sections
        .work
        .iter()
        .flatten()
        .map(|w| {
            let (project_state, root) = match &w.project {
                Some(n) => {
                    let (s, r) = project_of(states, reg, n);
                    (Some(s), r)
                }
                None => (None, None),
            };
            let place_root = w.place.as_ref().and_then(|p| match_registered(reg, &p.project));
            WorkState { line: w.line, project_state, root, place_root }
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
        Files { ideas: ctx.ideas_dir().to_path_buf(), ws: ctx.workspaces_file().to_path_buf(), reg: registry::path() }
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
    let project_states = states(&plan, &reg);
    Ok(PlanView { work_states: work_states(&plan, &project_states, &reg), project_states, plan, workspace })
}

/// Add the registered project `root` to the plan's `## Projects` as
/// `- <registry name> (registered)`.
pub fn add_project_in(f: &Files, id: &str, root: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    let reg = registry::read_lenient_at(&f.reg);
    let name = plan_name(&reg, root)?;
    ideas::plan_add_project_at(&f.ideas, id, &name, expected_mtime)
}

/// The name the reader will match back to the registered `root`: its registry
/// name when that is unique, else its root.
fn plan_name(reg: &registry::Registry, root: &str) -> Result<String, String> {
    let e = reg.by_root(root).ok_or_else(|| format!("not a registered project: {root}"))?;
    Ok(if match_registered(reg, &e.name).as_deref() == Some(e.root.as_str()) { e.name.clone() } else { e.root.clone() })
}

/// `plan_set_project`'s answer: the plan's new text, and its workspace (which
/// references the project now) when it has one.
#[derive(Serialize, Debug)]
pub struct ProjectSet {
    #[serde(flatten)]
    pub doc: Doc,
    pub workspace: Option<WsRef>,
    /// Why the workspace could not take the project, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_error: Option<String>,
}

/// The plan's `clone:` or `new:` project `name` is here now, registered at
/// `root`: its line says so, and the plan's workspace references it (§2.4).
/// The act after `clone_project` or `create_project`; it clones and creates
/// nothing.
pub fn set_project_in(f: &Files, id: &str, name: &str, root: &str, expected_mtime: Option<u64>) -> Result<ProjectSet, String> {
    let reg = registry::read_lenient_at(&f.reg);
    let to = plan_name(&reg, root)?;
    let doc = ideas::plan_set_project_at(&f.ideas, id, name, &to, expected_mtime)?;
    let file = store::read_at(&f.ws)?;
    let Some(w) = store::for_plan(&file, id) else { return Ok(ProjectSet { doc, workspace: None, workspace_error: None }) };
    let mut roots: Vec<String> = w.projects.iter().map(|p| p.root.clone()).collect();
    let mut workspace_error = None;
    if !roots.iter().any(|r| r == root) {
        roots.push(root.to_string());
        // The plan is written: a workspace that refuses is said beside it.
        workspace_error = store::set_projects_at(&f.ws, &w.id, &roots, |r| reg.by_root(r).is_some()).err();
    }
    Ok(ProjectSet { doc, workspace: Some(WsRef { id: w.id.clone(), name: w.name.clone() }), workspace_error })
}

/// What a work item's Start was asked: the item as the person saw it (its
/// line, title, the project it resolved to and its branch), the brief they
/// reviewed, and which agent. Plain parameters (§7.1): an agent's proposal
/// later is the same call.
#[derive(Debug, Clone, Default)]
pub struct Start {
    pub plan: String,
    pub line: usize,
    pub title: String,
    /// The registered root the item's project resolved to.
    pub root: String,
    pub branch: String,
    /// The text `.planning/brief.md` will hold.
    pub brief: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub force: bool,
    pub expected_mtime: Option<u64>,
}

/// `plan_start`'s answer. `result` is the create's own, refused launches
/// included (the place is there; only the agent is not). `doc` is the plan
/// with its `started:` line, when a place was made.
#[derive(Serialize, Debug)]
pub struct StartDone {
    pub result: CmdResult,
    pub started: Option<Started>,
    pub doc: Option<Doc>,
    /// The line the app could not write, with why: the place exists, so it is
    /// said rather than lost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_error: Option<String>,
}

/// Whether the place an item already names is gone: its project is not
/// registered, or that project has no worktree of that slug. Only then may a
/// new `started:` line replace it.
fn gone(reg: &registry::Registry, s: &Started) -> bool {
    let Some(root) = match_registered(reg, &s.project) else { return true };
    let Ok(p) = Project::discover(Path::new(&root)) else { return false };
    s.slug != "(main)" && !p.is_registered(&p.place_dir(&s.slug))
}

/// The item a request names, in the plan as it is now: refused when the plan
/// moved past `expected_mtime`, or when the item already has a place that is
/// there. Answers whether a `started:` line may replace one.
fn item_of(f: &Files, reg: &registry::Registry, plan: &str, line: usize, title: &str, expected_mtime: Option<u64>) -> Result<(PlanView, WorkItem, bool), String> {
    let v = read_in(f, plan)?;
    if v.plan.doc.idea.id.as_deref() != Some(plan) {
        return Err("that plan was merged into another: open it there".into());
    }
    if expected_mtime.is_some_and(|m| m != v.plan.doc.idea.mtime_ms) {
        return Err("the plan changed on disk since you opened it — reload to see the latest".into());
    }
    let it = reader::find_item(&v.plan.sections, line, title)?.clone();
    let mut replace = false;
    if let Some(had) = &it.place {
        if !gone(reg, had) {
            return Err(format!("“{title}” already has a place: {} in {}", had.slug, had.project));
        }
        replace = true;
    } else if it.started.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err(format!("“{title}” has a `started:` line this app cannot read: fix or remove it first"));
    }
    Ok((v, it, replace))
}

/// One Start at a time, from its checks to its `started:` line: the second of
/// two (two windows, a double click) then meets the first one's place instead
/// of both passing the same checks. One engine per host, so a process lock.
static STARTING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Start the work item: create its place with the reviewed brief and launch
/// its agent (`create`, the `new --brief` path), then write the item's
/// `started:` line. Everything that can be refused is, before anything is
/// made: the plan moved, the item is not the one reviewed, its project is not
/// this registered root, its branch or folder is another place's, the brief
/// is empty.
///
/// The line is written when the place EXISTS after the create, a refused
/// launch included: the place is the item's either way, and a retry would
/// otherwise be refused as a branch in another place.
pub fn start_in(f: &Files, r: &Start, create: impl FnOnce(&Start) -> Result<CmdResult, String>) -> Result<StartDone, String> {
    let branch = branch_arg(&r.branch)?;
    let _one = STARTING.lock().unwrap_or_else(|e| e.into_inner());
    let reg = registry::read_lenient_at(&f.reg);
    let (v, it, replace) = item_of(f, &reg, &r.plan, r.line, &r.title, r.expected_mtime)?;
    if it.branch.as_deref() != Some(branch.as_str()) {
        return Err(format!("the plan's branch for “{}” is {}, not {branch}: reload to see the latest", r.title, it.branch.as_deref().unwrap_or("missing")));
    }
    let name = it.project.as_deref().ok_or_else(|| format!("“{}” names no project", r.title))?;
    match project_of(&v.project_states, &reg, name) {
        ("registered", Some(root)) if root == r.root => {}
        ("registered", Some(root)) => return Err(format!("“{name}” is the project at {root}, not {}: reload to see the latest", r.root)),
        ("to-clone", _) => return Err(format!("“{name}” is not cloned yet: clone it from the plan's Projects")),
        ("to-create", _) => return Err(format!("“{name}” is not created yet: create it from the plan's Projects")),
        _ => return Err(format!("“{name}” is not a registered project")),
    }
    if r.brief.trim().is_empty() {
        return Err("the brief is empty: say what the agent is to do".into());
    }
    let p = Project::discover(Path::new(&r.root)).map_err(|e| e.msg)?;
    if let Some(holder) = p.wt_for_branch(&branch) {
        let slug = holder.rsplit('/').next().unwrap_or_default().to_string();
        return Err(format!("the branch {branch} is already in the place {slug}: link that place to the item instead"));
    }
    if p.wt_branch(&p.main_root) == branch {
        return Err(format!("the branch {branch} is checked out in the main checkout: give the item its own branch"));
    }
    let slug = safe_arg(&p.resolve_new_slug(&branch, None), "slug")?;
    let dir = p.place_dir(&slug);
    if Path::new(&dir).exists() {
        return Err(format!("there is a place named {slug} already: link it to the item, or give the item another branch"));
    }
    let result = create(&Start { branch, ..r.clone() })?;
    let slug = result.slug.clone().unwrap_or(slug);
    if !p.is_registered(&p.place_dir(&slug)) {
        return Ok(StartDone { result, started: None, doc: None, plan_error: None });
    }
    let started = Started { project: r.root.clone(), slug, at: reader::when(worktrees_core::sysclock::now_epoch()) };
    // No `expected_mtime` here: the create took seconds, the line goes into
    // the plan as it is now, and everything else in it is left alone.
    let wrote = ideas::plan_started_at(&f.ideas, &r.plan, r.line, &r.title, &started, replace, None);
    let (doc, plan_error) = match wrote {
        Ok(d) => (Some(d), None),
        Err(e) => {
            let line = started.text().map(|t| format!("started: {t}")).unwrap_or_default();
            applog("warn", &format!("plan_start {}: place {} made, its line not written: {e}", r.plan, started.slug));
            (None, Some(format!("The place was made, but the plan could not be written: {e}. Its line is `{line}`.")))
        }
    };
    Ok(StartDone { result, started: Some(started), doc, plan_error })
}

/// Record an existing place as the item's (§6.1: a place dropped on a work
/// item): its `started:` line, no new place. The place must be there, in a
/// registered project; an item whose own place is there keeps it.
pub fn link_place_in(f: &Files, plan: &str, line: usize, title: &str, root: &str, slug: &str, expected_mtime: Option<u64>) -> Result<Doc, String> {
    let slug = safe_arg(slug, "slug")?;
    let reg = registry::read_lenient_at(&f.reg);
    let e = reg.by_root(root).ok_or_else(|| format!("not a registered project: {root}"))?;
    let (_, _, replace) = item_of(f, &reg, plan, line, title, expected_mtime)?;
    let p = Project::discover(Path::new(&e.root)).map_err(|e| e.msg)?;
    if slug != "(main)" && !p.is_registered(&p.place_dir(&slug)) {
        return Err(format!("no place named {slug} in {}", e.name));
    }
    let s = Started { project: e.root.clone(), slug, at: reader::when(worktrees_core::sysclock::now_epoch()) };
    ideas::plan_started_at(&f.ideas, plan, line, title, &s, replace, expected_mtime)
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
    /// The plan's mtime, which an act on its items names.
    pub mtime_ms: Option<u64>,
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
    /// One per work item, in order.
    pub work_states: Vec<WorkState>,
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
    let mut out = Overview { id: w.id.clone(), name: w.name.clone(), plan: None, ideas, projects: None, work: None, work_states: Vec::new(), problems: Vec::new() };
    if let Some(pid) = &w.plan {
        match read_in(f, pid) {
            Ok(v) => {
                let roots: Vec<&str> = w.projects.iter().map(|p| p.root.as_str()).collect();
                out.plan = Some(OvPlan {
                    id: pid.clone(),
                    live: v.plan.doc.idea.id.clone(),
                    title: Some(v.plan.doc.idea.title.clone()),
                    file: Some(v.plan.doc.idea.file.clone()),
                    mtime_ms: Some(v.plan.doc.idea.mtime_ms),
                    error: None,
                });
                out.projects = v.plan.sections.projects.as_ref().map(|_| {
                    v.project_states
                        .iter()
                        .map(|s| OvProject { in_workspace: s.root.as_deref().is_some_and(|r| roots.contains(&r)), state: s.clone() })
                        .collect()
                });
                out.work = v.plan.sections.work.clone();
                out.work_states = v.work_states.clone();
                out.problems = v.plan.sections.problems.clone();
            }
            Err(e) => out.plan = Some(OvPlan { id: pid.clone(), live: None, title: None, file: None, mtime_ms: None, error: Some(e) }),
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

/// The plan's project `name` is registered now, at `root`
/// ([`set_project_in`]).
pub fn plan_set_project(ctx: &Ctx, id: String, name: String, root: String, expected_mtime: Option<u64>) -> Result<ProjectSet, String> {
    set_project_in(&Files::of(ctx), &id, &name, &root, expected_mtime)
}

/// Start a work item ([`start_in`]): its place, in the project at `repo`,
/// created with `brief` through `new --brief`'s path and launched for this
/// caller, as `new_place` is.
#[allow(clippy::too_many_arguments)]
pub fn plan_start(
    ctx: &Ctx,
    id: String,
    line: usize,
    title: String,
    repo: String,
    branch: String,
    brief: String,
    provider: Option<String>,
    model: Option<String>,
    force: Option<bool>,
    expected_mtime: Option<u64>,
) -> Result<StartDone, String> {
    let r = Start { plan: id, line, title, root: repo, branch, brief, provider, model, force: force.unwrap_or(false), expected_mtime };
    let out = start_in(&Files::of(ctx), &r, |r| {
        crate::ops::new_place_with(ctx, r.root.clone(), r.branch.clone(), None, None, r.provider.clone(), r.model.clone(), Some(r.force), Some(&r.brief))
    })?;
    if out.started.is_some() {
        ctx.emit("places:changed", serde_json::Value::Null);
    }
    Ok(out)
}

/// Record the existing place (`repo`, `slug`) as the item's
/// ([`link_place_in`]).
pub fn plan_link_place(ctx: &Ctx, id: String, line: usize, title: String, repo: String, slug: String, expected_mtime: Option<u64>) -> Result<Doc, String> {
    link_place_in(&Files::of(ctx), &id, line, &title, &repo, &slug, expected_mtime)
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

    fn git(dir: &Path, args: &[&str]) {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    fn repo(dir: &Path) -> String {
        fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"]);
        fs::canonicalize(dir).unwrap().to_string_lossy().into_owned()
    }

    fn done(ok: bool, code: i32, slug: Option<&str>) -> CmdResult {
        CmdResult { ok, code, output: String::new(), slug: slug.map(str::to_string), needs_confirm: None, warnings: vec![] }
    }

    /// Phase 5, through the bodies with a scripted create: the item's state,
    /// every refusal BEFORE the create, the `started:` line after it (a
    /// refused launch included), a place gone, and a place linked.
    #[test]
    fn a_work_item_starts_once_and_its_place_is_recorded() {
        let (d, f) = files("start");
        let root = repo(&d.join("conf"));
        let top = d.join("conf").parent().unwrap().to_path_buf();
        register(&f, &[(&root, "conf")]);
        let a = mk(&f, "Jellyfin");
        let body = format!(
            "# Plan: J\n\n## Projects\n- conf (registered)\n- ans (clone: git@h:a/ans.git)\n- here (new: {root})\n- cfg (new: {}/nope)\n\n## Work\n### Role\nproject: conf\nbranch: feat/role\nai: claude\nbrief: |\n  Do it.\n\n### Ans\nproject: ans\nbranch: b\n\n### Main\nproject: conf\nbranch: main\n",
            top.display()
        );
        let p = ideas::make_plan_at(&f.ideas, &[a], None, Some(&body)).unwrap();
        let (pid, m) = (p.idea.id.clone().unwrap(), p.idea.mtime_ms);
        let v = read_in(&f, &pid).unwrap();
        assert_eq!(v.project_states.iter().map(|s| s.state).collect::<Vec<_>>(), ["registered", "to-clone", "registered", "to-create"], "a `new:` folder that is a registered root is registered");
        let ws: Vec<_> = v.work_states.iter().map(|w| (w.project_state, w.root.as_deref())).collect();
        assert_eq!(ws, [(Some("registered"), Some(root.as_str())), (Some("to-clone"), None), (Some("registered"), Some(root.as_str()))]);
        let work = v.plan.sections.work.clone().unwrap();
        let line = |t: &str| work.iter().find(|w| w.title == t).unwrap().line;
        let role = Start { plan: pid.clone(), line: line("Role"), title: "Role".into(), root: root.clone(), branch: "feat/role".into(), brief: "Do it, reviewed.".into(), expected_mtime: Some(m), ..Default::default() };
        let never = |_: &Start| -> Result<CmdResult, String> { panic!("refused before the create") };
        let no = |r: Start| start_in(&f, &r, never).unwrap_err();
        assert!(no(Start { branch: "-x".into(), ..role.clone() }).contains("branch"), "a `-`-led branch is no branch");
        assert!(no(Start { branch: "other".into(), ..role.clone() }).contains("not other"));
        assert!(no(Start { expected_mtime: Some(1), ..role.clone() }).contains("changed on disk"));
        assert!(no(Start { brief: " \n".into(), ..role.clone() }).contains("brief is empty"));
        assert!(no(Start { root: "/elsewhere".into(), ..role.clone() }).contains("is the project at"));
        assert!(no(Start { title: "Nope".into(), ..role.clone() }).contains("no item"));
        assert!(no(Start { line: line("Ans"), title: "Ans".into(), branch: "b".into(), ..role.clone() }).contains("not cloned yet"));
        assert!(no(Start { line: line("Main"), title: "Main".into(), branch: "main".into(), ..role.clone() }).contains("checked out in the main checkout"));
        // A branch that is another place's.
        let dir = Path::new(&root);
        git(dir, &["worktree", "add", "-q", "-b", "feat/role", ".worktrees/holder"]);
        assert!(no(role.clone()).contains("already in the place holder"));
        git(dir, &["worktree", "remove", "--force", ".worktrees/holder"]);
        git(dir, &["branch", "-D", "feat/role"]);
        // A create that made nothing leaves the plan as it was.
        let out = start_in(&f, &role, |_| Ok(done(false, 1, None))).unwrap();
        assert!(out.started.is_none() && out.doc.is_none());
        assert_eq!(ideas::read_at(&f.ideas, &pid).unwrap().content, p.content);
        // The create gets the reviewed text; a refused launch still records.
        let out = start_in(&f, &role, |r| {
            assert_eq!((r.brief.as_str(), r.branch.as_str()), ("Do it, reviewed.", "feat/role"));
            git(dir, &["worktree", "add", "-q", "-b", "feat/role", ".worktrees/feat-role"]);
            Ok(done(false, 5, None))
        })
        .unwrap();
        let s = out.started.unwrap();
        assert_eq!((s.project.as_str(), s.slug.as_str()), (root.as_str(), "feat-role"));
        let doc = out.doc.unwrap();
        assert!(doc.content.contains(&format!("  Do it.\nstarted: {root} · feat-role · ")), "{}", doc.content);
        assert!(out.plan_error.is_none());
        let v = read_in(&f, &pid).unwrap();
        assert_eq!(v.work_states[0].place_root.as_deref(), Some(root.as_str()));
        // Started: neither a second Start nor another place replaces it…
        assert!(no(Start { expected_mtime: None, ..role.clone() }).contains("already has a place"));
        git(dir, &["worktree", "add", "-q", "-b", "o", ".worktrees/other"]);
        assert!(link_place_in(&f, &pid, role.line, "Role", &root, "other", None).unwrap_err().contains("already has a place"));
        // …until it is gone; its line is kept until then.
        git(dir, &["worktree", "remove", "--force", ".worktrees/feat-role"]);
        assert!(ideas::read_at(&f.ideas, &pid).unwrap().content.contains("feat-role · "), "the line outlives the place");
        for (slug, words) in [("-x", "slug"), ("nope", "no place named nope")] {
            assert!(link_place_in(&f, &pid, role.line, "Role", &root, slug, None).unwrap_err().contains(words), "{slug}");
        }
        let d3 = link_place_in(&f, &pid, role.line, "Role", &root, "other", None).unwrap();
        assert!(d3.content.contains(&format!("started: {root} · other · ")) && d3.content.matches("started:").count() == 1, "{}", d3.content);
        assert!(link_place_in(&f, &pid, line("Ans"), "Ans", "/nowhere", "other", None).unwrap_err().contains("not a registered project"));
        let _ = fs::remove_dir_all(d);
    }

    /// A cloned or created project: its line is `(registered)`, its items
    /// follow the name, and the plan's workspace references it, once.
    #[test]
    fn an_arrived_project_is_registered_in_the_plan_and_its_workspace() {
        let (d, f) = files("arrive");
        register(&f, &[("/w/worktrees", "worktrees")]);
        let a = mk(&f, "J");
        let body = "# Plan: J\n\n## Projects\n- worktrees (registered)\n- ans (clone: git@h:a/ans.git)\n\n## Work\n### A\nproject: ans\nbranch: b\n";
        let pid = ideas::make_plan_at(&f.ideas, &[a], None, Some(body)).unwrap().idea.id.unwrap();
        assert!(set_project_in(&f, &pid, "ans", "/w/ansible", None).unwrap_err().contains("not a registered project"));
        let w = from_plan_in(&f, &pid).unwrap().id;
        register(&f, &[("/w/worktrees", "worktrees"), ("/w/ansible", "ansible")]);
        let o = set_project_in(&f, &pid, "ans", "/w/ansible", None).unwrap();
        assert!(o.doc.content.contains("- ansible (registered)\n") && o.doc.content.contains("project: ansible\n"), "{}", o.doc.content);
        assert_eq!(o.workspace.map(|w| w.id), Some(w.clone()));
        let roots = |f: &Files| overview_in(f, &w).unwrap().projects.unwrap().into_iter().map(|p| (p.state.name, p.in_workspace)).collect::<Vec<_>>();
        assert_eq!(roots(&f), [("worktrees".to_string(), true), ("ansible".to_string(), true)]);
        set_project_in(&f, &pid, "ansible", "/w/ansible", None).unwrap();
        assert_eq!(ws_list_in(&f.ws, &f.reg).unwrap().workspaces[0].projects.len(), 2, "referenced once");
        let o = overview_in(&f, &w).unwrap();
        assert_eq!(o.work_states[0].root.as_deref(), Some("/w/ansible"));
        assert!(o.plan.unwrap().mtime_ms.is_some());
        let _ = fs::remove_dir_all(d);
    }

    /// The rows refuse the nav's Everything Else (#684: derived by the page,
    /// never stored), and it has no overview. A workspace's color (#659) is
    /// answered beside its plan and ideas.
    #[test]
    fn everything_else_is_refused_and_a_color_is_answered_beside_the_plan() {
        const EE: &str = "everything-else";
        let (d, f) = files("ee");
        register(&f, &[]);
        let a = mk(&f, "A");
        let p = ideas::make_plan_at(&f.ideas, std::slice::from_ref(&a), None, None).unwrap().idea.id.unwrap();
        let w = store::create_at(&f.ws, "One").unwrap().id;
        let before = fs::read_to_string(&f.ws).unwrap();
        for e in [
            set_plan_in(&f, EE, Some(&p)).unwrap_err(),
            set_plan_in(&f, EE, None).unwrap_err(),
            link_idea_in(&f, EE, &a).unwrap_err(),
            unlink_idea_in(&f, EE, &a).unwrap_err(),
            overview_in(&f, EE).unwrap_err(),
        ] {
            assert_eq!(e, format!("no workspace {EE}"));
        }
        assert!(from_plan_in(&f, EE).is_err(), "nor is it a plan");
        assert_eq!(fs::read_to_string(&f.ws).unwrap(), before, "a refusal writes nothing");
        set_plan_in(&f, &w, Some(&p)).unwrap();
        crate::workspaces::set_color_in(&f.ws, &f.reg, &w, Some("teal")).unwrap();
        let l = link_idea_in(&f, &w, &a).unwrap();
        let v = serde_json::to_value(&l.workspaces[0]).unwrap();
        assert_eq!((&v["color"], &v["plan"], &v["ideas"]), (&"teal".into(), &p.as_str().into(), &serde_json::json!([a])));
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
