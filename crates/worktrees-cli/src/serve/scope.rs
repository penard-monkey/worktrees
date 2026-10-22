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

use serde_json::Value;
use worktrees_core::authz::{Caller, Grant, Projects};

/// A result filter: the scope, and the command's serialized reply.
pub type Filter = fn(&Projects, Value) -> Value;

/// `path` (as the engine reports it: a place path, a session's cwd) is inside a
/// project in scope. Lexical first; a non-canonical path is canonicalized once.
pub fn covers(scope: &Projects, path: &str) -> bool {
    if scope.covers_path(path) {
        return true;
    }
    if scope.is_all() {
        return true;
    }
    std::fs::canonicalize(path).is_ok_and(|c| scope.covers_path(&c.to_string_lossy()))
}

/// Keep the elements of the array at `v[key]` (or `v` itself for `key ==
/// ""`) whose `field` is a path in scope.
fn keep(scope: &Projects, mut v: Value, key: &str, field: &str, root_only: bool) -> Value {
    if scope.is_all() {
        return v;
    }
    let arr = if key.is_empty() { Some(&mut v) } else { v.get_mut(key) };
    if let Some(Value::Array(items)) = arr {
        items.retain(|el| {
            el.get(field).and_then(Value::as_str).is_some_and(|p| if root_only { scope.covers_root(p) } else { covers(scope, p) })
        });
    }
    v
}

/// `list_workspace`: the projects in scope, each by its registry key.
pub fn workspace(scope: &Projects, v: Value) -> Value {
    keep(scope, v, "projects", "root", true)
}

/// `list_drafts`: the unsent text people typed into composers, host-wide —
/// only the drafts of sessions cwd'd in a project in scope.
pub fn drafts(scope: &Projects, v: Value) -> Value {
    keep(scope, v, "", "path", false)
}

/// `planning_status` and `cross_project_status`: a `projects` list keyed by
/// registry root.
pub fn by_project(scope: &Projects, v: Value) -> Value {
    keep(scope, v, "projects", "root", true)
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
/// nothing.
pub fn event(name: &str, payload: &Value, caller: &Caller, grant: &Grant) -> Option<Value> {
    let Some((_, how)) = EVENTS.iter().find(|(n, _)| *n == name) else {
        // Unprojected: the console only.
        return caller.is_console().then(|| payload.clone());
    };
    let scope = &grant.projects;
    match how {
        Event::Everyone => Some(payload.clone()),
        Event::Console => caller.is_console().then(|| payload.clone()),
        _ if scope.is_all() => Some(payload.clone()),
        Event::ByPath(field) => payload.get(*field).and_then(Value::as_str).is_some_and(|p| covers(scope, p)).then(|| payload.clone()),
        Event::Drafts => {
            let mut v = payload.clone();
            if let Some(list) = v.get_mut("drafts") {
                *list = drafts(scope, list.take());
            }
            Some(v)
        }
        Event::Busy => {
            let mut v = payload.clone();
            let cut = |a: &mut Value| {
                if let Value::Array(items) = a {
                    items.retain(|p| p.as_str().is_some_and(|p| covers(scope, p)));
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

    /// The console's scope is `"*"`: every projection is the identity, which
    /// is what keeps the desktop app's view exactly as it was.
    #[test]
    fn the_console_sees_every_result_and_event_unchanged() {
        let all = Grant::everything();
        let ws = json!({"projects": [{"root": "/r/a"}, {"root": "/r/b"}]});
        assert_eq!(workspace(&all.projects, ws.clone()), ws);
        for (name, _) in EVENTS {
            let p = json!({"path": "/elsewhere", "repo": "/elsewhere", "busy": ["/elsewhere"], "drafts": [{"path": "/x"}]});
            assert_eq!(event(name, &p, &Caller::console(), &all), Some(p.clone()), "{name}");
        }
        assert_eq!(event("not:projected", &json!(1), &Caller::console(), &all), Some(json!(1)));
    }

    /// gap 12: a scoped device hears only about its own projects.
    #[test]
    fn a_scoped_device_hears_only_its_projects() {
        let g = scoped(&["/r/a"]);
        let me = phone();
        let busy = json!({"busy": ["/r/a/.worktrees/x", "/r/b"], "waiting": ["/r/b/.worktrees/y"], "waiting_by": {"claude": ["/r/b/.worktrees/y", "/r/a"]}});
        assert_eq!(
            event("sessions:busy", &busy, &me, &g),
            Some(json!({"busy": ["/r/a/.worktrees/x"], "waiting": [], "waiting_by": {"claude": ["/r/a"]}}))
        );
        assert_eq!(event("sessions:done", &json!({"path": "/r/b", "epoch": 1}), &me, &g), None);
        assert!(event("sessions:done", &json!({"path": "/r/a/.worktrees/x", "epoch": 1}), &me, &g).is_some());
        assert_eq!(event("shell:exit", &json!({"repo": "/r/ab", "slug": "s", "index": 0}), &me, &g), None, "/r/ab is not under /r/a");
        assert_eq!(
            event("sessions:drafts", &json!({"drafts": [{"path": "/r/a", "text": "mine"}, {"path": "/r/b", "text": "theirs"}]}), &me, &g),
            Some(json!({"drafts": [{"path": "/r/a", "text": "mine"}]}))
        );
        assert_eq!(event("places:changed", &Value::Null, &me, &g), Some(Value::Null));
        assert_eq!(event("app:open-doc", &json!({"path": "/r/a/x.md"}), &me, &Grant::everything()), None, "console only, scope or not");
        assert_eq!(event("not:projected", &json!(1), &me, &Grant::everything()), None, "fails closed");
    }

    #[test]
    fn results_are_cut_to_the_scope() {
        let g = scoped(&["/r/a"]);
        let ws = json!({"projects": [{"root": "/r/a", "ok": true}, {"root": "/r/b", "ok": true}]});
        assert_eq!(workspace(&g.projects, ws), json!({"projects": [{"root": "/r/a", "ok": true}]}));
        let d = json!([{"path": "/r/a/.worktrees/x", "text": "t"}, {"path": "/r/b", "text": "u"}]);
        assert_eq!(drafts(&g.projects, d), json!([{"path": "/r/a/.worktrees/x", "text": "t"}]));
        let pl = json!({"default": "off", "projects": [{"root": "/r/b"}, {"root": "/r/a"}]});
        assert_eq!(by_project(&g.projects, pl), json!({"default": "off", "projects": [{"root": "/r/a"}]}));
    }
}
