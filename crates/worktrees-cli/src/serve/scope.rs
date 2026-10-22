//! Projection: what leaves serve is cut down to the caller's scope.
//!
//! Two halves (`docs/proposals/server-edition-phase2.md` §3.3). RESULTS of the
//! multi-project reads (`filtered(…)` rows) go through a filter here before
//! they are serialized onto the wire. EVENTS go to every authenticated socket
//! (`conn::Broadcast`), and each is projected per socket against that socket's
//! grant at the moment of the emit, never a copy taken when it connected.
//!
//! Both are the identity for a grant whose scope is `"*"`, which is the
//! console's, so the desktop app sees exactly what it saw before. An event
//! with no projection here reaches the console and nobody else: that fails
//! closed, and a test fails when an `emit("…")` in the api crate has no row.

use std::path::{Component, Path};

use serde_json::Value;
use worktrees_core::authz::{Caller, Grant, Projects};

/// What a projection reads: the caller's scope, and the registered roots, by
/// which a path is attributed to the project that OWNS it — the innermost
/// registered root containing it, as the resolver's `file(…)` does. Without
/// the roots a path in a repo checked out under another repo's tree would
/// count as the outer one's.
pub struct View<'a> {
    pub projects: &'a Projects,
    /// The registry's keys. Read only for a scope that is not `"*"`.
    pub roots: &'a [String],
}

/// A result filter: the caller's view, and the command's serialized reply.
pub type Filter = fn(&View, Value) -> Value;

/// `path` (as the engine reports it: a place path, a session's cwd) belongs to
/// a project in scope: its innermost registered root is covered. A path with
/// a `..` or `.` in it, or one not under any root as written, is judged by its
/// canonical form instead — `/r/a/../b` is under `/r/b`, whatever its prefix
/// says — and one that cannot be canonicalized is out.
pub fn covers(view: &View, path: &str) -> bool {
    if view.projects.is_all() {
        return true;
    }
    let owner = |p: &Path| view.roots.iter().filter(|r| p.starts_with(r.as_str())).max_by_key(|r| Path::new(r.as_str()).components().count()).cloned();
    let p = Path::new(path);
    let plain = p.is_absolute() && !p.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir));
    if plain {
        if let Some(o) = owner(p) {
            return view.projects.covers_root(&o);
        }
    }
    match std::fs::canonicalize(path) {
        Ok(c) => owner(&c).is_some_and(|o| view.projects.covers_root(&o)),
        Err(_) => false,
    }
}

/// Keep the elements of the array at `v[key]` (or `v` itself for `key ==
/// ""`) whose `field` is in scope: a registry key exactly (`root_only`), or a
/// path its owning project covers.
fn keep(view: &View, mut v: Value, key: &str, field: &str, root_only: bool) -> Value {
    if view.projects.is_all() {
        return v;
    }
    let arr = if key.is_empty() { Some(&mut v) } else { v.get_mut(key) };
    if let Some(Value::Array(items)) = arr {
        items.retain(|el| {
            el.get(field).and_then(Value::as_str).is_some_and(|p| if root_only { view.projects.covers_root(p) } else { covers(view, p) })
        });
    }
    v
}

/// `list_workspace`: the projects in scope, each by its registry key.
pub fn workspace(view: &View, v: Value) -> Value {
    keep(view, v, "projects", "root", true)
}

/// `list_drafts`: the unsent text people typed into composers, host-wide —
/// only the drafts of sessions cwd'd in a project in scope.
pub fn drafts(view: &View, v: Value) -> Value {
    keep(view, v, "", "path", false)
}

/// `planning_status` and `cross_project_status`: a `projects` list keyed by
/// registry root.
pub fn by_project(view: &View, v: Value) -> Value {
    keep(view, v, "projects", "root", true)
}

// ── events ───────────────────────────────────────────────────────────────────

/// How each event reaches a caller. Every name the api emits has a row; a
/// test reads the api's `emit("…")`s and fails on one that does not.
pub const EVENTS: &[(&str, Event)] = &[
    // No payload: "re-read", and the re-read is itself scoped.
    ("places:changed", Event::Everyone),
    // The host-wide busy, waiting and waiting-by sets: each member in scope.
    ("sessions:busy", Event::Busy),
    // `{path, epoch}`: a finished turn, by place path.
    ("sessions:done", Event::ByPath("path")),
    // `{drafts: [{path, text, queued}]}`: the composer TEXT, by session cwd —
    // the event twin of `list_drafts`.
    ("sessions:drafts", Event::Drafts),
    // `{repo, slug, index}`: a dock shell exited.
    ("shell:exit", Event::ByPath("repo")),
    // `worktrees show` is run by someone at the host (§9.1).
    ("app:open-doc", Event::Console),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Everyone,
    Busy,
    ByPath(&'static str),
    Drafts,
    Console,
}

/// `payload` as `caller` (holding `grant`) may see it, or `None` to send it
/// nothing. `roots` is asked for only when the scope is not `"*"`.
pub fn event(name: &str, payload: &Value, caller: &Caller, grant: &Grant, roots: &mut dyn FnMut() -> Vec<String>) -> Option<Value> {
    let Some((_, how)) = EVENTS.iter().find(|(n, _)| *n == name) else {
        // Unprojected: the console only.
        return caller.is_console().then(|| payload.clone());
    };
    let scope = &grant.projects;
    match how {
        Event::Everyone => return Some(payload.clone()),
        Event::Console => return caller.is_console().then(|| payload.clone()),
        _ if scope.is_all() => return Some(payload.clone()),
        _ => {}
    }
    let roots = roots();
    let view = View { projects: scope, roots: &roots };
    match how {
        Event::Everyone | Event::Console => unreachable!("answered above"),
        Event::ByPath(field) => payload.get(*field).and_then(Value::as_str).is_some_and(|p| covers(&view, p)).then(|| payload.clone()),
        Event::Drafts => {
            let mut v = payload.clone();
            if let Some(list) = v.get_mut("drafts") {
                *list = drafts(&view, list.take());
            }
            Some(v)
        }
        Event::Busy => {
            let mut v = payload.clone();
            let cut = |a: &mut Value| {
                if let Value::Array(items) = a {
                    items.retain(|p| p.as_str().is_some_and(|p| covers(&view, p)));
                }
            };
            for k in ["busy", "waiting"] {
                if let Some(a) = v.get_mut(k) {
                    cut(a);
                }
            }
            if let Some(Value::Object(by)) = v.get_mut("waiting_by") {
                by.values_mut().for_each(cut);
            }
            Some(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use worktrees_core::authz::{Capability, Caps, DeviceId, PrincipalId};

    fn scoped(roots: &[&str]) -> Grant {
        Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::only(roots.iter().copied()) }
    }

    fn phone() -> Caller {
        Caller::paired(PrincipalId::owner(), DeviceId::new("phone"))
    }

    /// The registry: `/r/a`, `/r/b`, and `/r/a/inner` — a repo checked out
    /// inside another's tree, registered on its own.
    fn reg() -> Vec<String> {
        vec!["/r/a".into(), "/r/b".into(), "/r/a/inner".into()]
    }

    fn ev(name: &str, p: &Value, c: &Caller, g: &Grant) -> Option<Value> {
        event(name, p, c, g, &mut reg)
    }

    /// The console's scope is `"*"`: every projection is the identity, which
    /// is what keeps the desktop app's view exactly as it was — and the
    /// registry is not even read for it.
    #[test]
    fn the_console_sees_every_result_and_event_unchanged() {
        let all = Grant::everything();
        let ws = json!({"projects": [{"root": "/r/a"}, {"root": "/r/b"}]});
        let roots = reg();
        assert_eq!(workspace(&View { projects: &all.projects, roots: &roots }, ws.clone()), ws);
        let mut never = || -> Vec<String> { panic!("the console's projection read the registry") };
        for (name, _) in EVENTS {
            let p = json!({"path": "/elsewhere", "repo": "/elsewhere", "busy": ["/elsewhere"], "drafts": [{"path": "/x"}]});
            assert_eq!(event(name, &p, &Caller::console(), &all, &mut never), Some(p.clone()), "{name}");
        }
        assert_eq!(event("not:projected", &json!(1), &Caller::console(), &all, &mut never), Some(json!(1)));
    }

    /// gap 12: a scoped device hears only about its own projects.
    #[test]
    fn a_scoped_device_hears_only_its_projects() {
        let g = scoped(&["/r/a"]);
        let me = phone();
        let busy = json!({"busy": ["/r/a/.worktrees/x", "/r/b"], "waiting": ["/r/b/.worktrees/y"], "waiting_by": {"claude": ["/r/b/.worktrees/y", "/r/a"]}});
        assert_eq!(
            ev("sessions:busy", &busy, &me, &g),
            Some(json!({"busy": ["/r/a/.worktrees/x"], "waiting": [], "waiting_by": {"claude": ["/r/a"]}}))
        );
        assert_eq!(ev("sessions:done", &json!({"path": "/r/b", "epoch": 1}), &me, &g), None);
        assert!(ev("sessions:done", &json!({"path": "/r/a/.worktrees/x", "epoch": 1}), &me, &g).is_some());
        assert_eq!(ev("shell:exit", &json!({"repo": "/r/ab", "slug": "s", "index": 0}), &me, &g), None, "/r/ab is not under /r/a");
        assert_eq!(
            ev("sessions:drafts", &json!({"drafts": [{"path": "/r/a", "text": "mine"}, {"path": "/r/b", "text": "theirs"}]}), &me, &g),
            Some(json!({"drafts": [{"path": "/r/a", "text": "mine"}]}))
        );
        assert_eq!(ev("places:changed", &Value::Null, &me, &g), Some(Value::Null));
        assert_eq!(ev("app:open-doc", &json!({"path": "/r/a/x.md"}), &me, &Grant::everything()), None, "console only, scope or not");
        assert_eq!(ev("not:projected", &json!(1), &me, &Grant::everything()), None, "fails closed");
    }

    /// A path belongs to its INNERMOST project, and is judged as written only
    /// when it is plain: a nested repo registered on its own is not the outer
    /// project's, and `..` does not climb out of a prefix into scope.
    #[test]
    fn a_path_is_its_innermost_projects_and_dots_do_not_climb() {
        let g = scoped(&["/r/a"]);
        let me = phone();
        assert_eq!(ev("sessions:done", &json!({"path": "/r/a/inner/.worktrees/x", "epoch": 1}), &me, &g), None);
        assert_eq!(ev("sessions:done", &json!({"path": "/r/a/../b/x", "epoch": 1}), &me, &g), None);
        assert!(ev("sessions:done", &json!({"path": "/r/a/./x", "epoch": 1}), &me, &g).is_some(), "an interior `.` is `/r/a/x`");
        assert_eq!(
            ev("sessions:busy", &json!({"busy": ["/r/a/inner", "/r/a/x"], "waiting": [], "waiting_by": {}}), &me, &g),
            Some(json!({"busy": ["/r/a/x"], "waiting": [], "waiting_by": {}}))
        );
        let inner = scoped(&["/r/a/inner"]);
        assert!(ev("sessions:done", &json!({"path": "/r/a/inner/.worktrees/x", "epoch": 1}), &me, &inner).is_some());
        assert_eq!(ev("sessions:done", &json!({"path": "/r/a/x", "epoch": 1}), &me, &inner), None);
    }

    #[test]
    fn results_are_cut_to_the_scope() {
        let g = scoped(&["/r/a"]);
        let roots = reg();
        let v = View { projects: &g.projects, roots: &roots };
        let ws = json!({"projects": [{"root": "/r/a", "ok": true}, {"root": "/r/b", "ok": true}]});
        assert_eq!(workspace(&v, ws), json!({"projects": [{"root": "/r/a", "ok": true}]}));
        let d = json!([{"path": "/r/a/.worktrees/x", "text": "t"}, {"path": "/r/b", "text": "u"}, {"path": "/r/a/inner", "text": "v"}]);
        assert_eq!(drafts(&v, d), json!([{"path": "/r/a/.worktrees/x", "text": "t"}]));
        let pl = json!({"default": "off", "projects": [{"root": "/r/b"}, {"root": "/r/a"}]});
        assert_eq!(by_project(&v, pl), json!({"default": "off", "projects": [{"root": "/r/a"}]}));
    }
}
