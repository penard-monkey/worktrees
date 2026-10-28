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
//!
//! A ONE-SHOT event also asks the socket, not only its caller: it goes only
//! to a connection that declared it listens for it (`Listens`, §9.3).
//!
//! A place's PROVENANCE (§7.3, §7.5) is projected for every caller, the
//! console included: each stamp names its device as the registry does now
//! ([`stamps`]).

use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use worktrees_core::authz::{Caller, Device, DeviceId, Grant, NoDevices, Projects, Registry};

/// What a projection reads: the caller's scope, and the registered roots, by
/// which a path is attributed to the project that OWNS it — the innermost
/// registered root containing it, as the resolver's `file(…)` does. Without
/// the roots a path in a repo checked out under another repo's tree would
/// count as the outer one's. And who the caller is, with the registry that
/// names devices, for a place's provenance stamps.
pub struct View<'a> {
    pub projects: &'a Projects,
    /// The registry's keys. Read only for a scope that is not `"*"`.
    pub roots: &'a [String],
    pub caller: &'a Caller,
    pub names: &'a dyn Registry,
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

/// `list_workspace`: the projects in scope, each by its registry key, and
/// every place's provenance stamps projected ([`stamps`]).
pub fn workspace(view: &View, v: Value) -> Value {
    let mut v = keep(view, v, "projects", "root", true);
    if let Some(Value::Array(projects)) = v.get_mut("projects") {
        for p in projects.iter_mut().filter_map(|p| p.get_mut("snapshot")) {
            places_stamps(view, p);
        }
    }
    v
}

/// `clone_project`: the workspace it answers with, under `workspace`, as
/// [`workspace`] projects it.
pub fn cloned(view: &View, mut v: Value) -> Value {
    if let Some(w) = v.get_mut("workspace") {
        *w = workspace(view, w.take());
    }
    v
}

/// `list_places`: one project's snapshot, the resolver's to admit, with its
/// places' provenance stamps projected ([`stamps`]).
pub fn places(view: &View, mut v: Value) -> Value {
    places_stamps(view, &mut v);
    v
}

fn places_stamps(view: &View, snapshot: &mut Value) {
    if let Some(Value::Array(places)) = snapshot.get_mut("places") {
        for d in places.iter_mut().filter_map(|p| p.get_mut("declared")) {
            stamps(view, d);
        }
    }
}

/// The declared fields that say which caller did something there: each is a
/// `store::By` (`{principal, device}`) flattened with what it did.
pub const STAMPS: &[&str] = &["launched_by", "last_remote_instruction", "last_remote_send", "note_by", "title_by"];

/// What another device's stamp keeps: who (the principal, and the device by
/// KIND and name) and what (the file, the session, when). Named rather than
/// stripped, so a field a stamp gains later reaches only the console and the
/// device itself until it is added here.
const SHARED: &[&str] = &["principal", "device", "device_name", "path", "at", "session"];

/// One place's `declared`, as `view.caller` may see it. A stamp by a paired
/// device gains `device_name`: the device's name as the registry has it NOW,
/// read per reply, so a rename shows on the next read, and a stamp written
/// before names existed is named too. A device the registry no longer has
/// is unnamed; a name a store file carries is never the one shown. The
/// console and the stamping device see the stamp whole — its id is how a
/// phone says "this device". Any other device sees [`SHARED`] alone, its
/// `device` cut to `{"kind": "paired"}`: never the id, which addresses the
/// device in the registry's rows, nor `cross_project`, the reach the
/// device's scope gave the lane. The console's own stamps are left as they
/// are.
pub fn stamps(view: &View, declared: &mut Value) {
    let Some(d) = declared.as_object_mut() else { return };
    for key in STAMPS {
        let Some(Value::Object(s)) = d.get_mut(*key) else { continue };
        let device = s.get("device");
        if device.and_then(|d| d.get("kind")).and_then(Value::as_str) != Some("paired") {
            continue;
        }
        let id = device.and_then(|d| d.get("id")).and_then(Value::as_str).map(DeviceId::new);
        match id.as_ref().and_then(|id| view.names.device_name(view.caller.principal(), id)) {
            Some(name) => s.insert("device_name".into(), Value::from(name)),
            None => s.remove("device_name"),
        };
        let own = matches!((view.caller.device(), &id), (Device::Paired(me), Some(id)) if me == id);
        if !view.caller.is_console() && !own {
            s.retain(|k, _| SHARED.contains(&k.as_str()));
            s.insert("device".into(), serde_json::json!({ "kind": "paired" }));
        }
    }
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
/// test reads the api's `emit("…")`s and fails on one that does not, and
/// another reads the audit log's (`audit.rs`).
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
    // `{alert, …}`: §5.3's alerts are for the person at the host, and a
    // device must not learn that it tripped one.
    ("audit:alert", Event::Console),
    // The device registry's events, for the Devices list and the alerts:
    // pairing and device administration are the host's (§3.3, §5.3).
    ("devices:changed", Event::Console),
    // Remote access's state (`remote_status`): on, paused and why, a question
    // for the host. The host's, like every remote-access act (§4.1).
    ("remote:changed", Event::Console),
];

// ── who listens (§9.3) ───────────────────────────────────────────────────────

/// Events that are one ACT each and are consumed by being emitted: the inbox
/// takes a `worktrees show` off the disk as it goes out. Each is sent only to
/// a connection that declared it listens for it (`Listens`), and the inbox
/// waits (`Host::has_audience`) until a loopback one has. A connection that
/// declares nothing — the shim's verification probe, which authenticates and
/// closes — is sent none and drains nothing, so the rule fails closed. Every
/// other event goes to every socket, projected, whatever it declared.
pub const ONE_SHOT: &[&str] = &["app:open-doc"];

pub fn is_one_shot(event: &str) -> bool {
    ONE_SHOT.contains(&event)
}

/// Events whose payload is the WHOLE state, not a change to it, and whose
/// push is change-gated: a client that connects between two changes would
/// hold no state until the next one, so a connection that DECLARES one is
/// sent the last payload at once, projected as the push is
/// (`conn::Clients::declare`). `sessions:busy` is the busy and waiting sets
/// (a phone reconnects on every foreground). `sessions:done` and
/// `places:changed` are not here: the first is one turn and the second
/// "re-read", and a client re-pulls the workspace, which carries
/// `last_worked_epoch`, on every reconnect. `sessions:drafts` has
/// `list_drafts`.
pub const SNAPSHOT: &[&str] = &["sessions:busy"];

/// `event`'s row in [`SNAPSHOT`], for the `'static` name a cache keys on.
pub fn snapshot(event: &str) -> Option<&'static str> {
    SNAPSHOT.iter().copied().find(|n| *n == event)
}

/// The events one connection has declared it listens for: its last
/// `{"listen": [...]}`, each frame a full replacement (`wire::Frame::Listen`).
/// Only names `EVENTS` has are kept, one bit each, so whatever a client
/// sends, the set is one word — and a name this serve never emits is ignored,
/// never an error: a newer page may listen for an event an older serve does
/// not have. The session replaces it and `Broadcast` reads it on every emit,
/// neither under the other's lock. Starts empty: nothing declared.
#[derive(Clone, Default, Debug)]
pub struct Listens(Arc<AtomicU64>);

const _: () = assert!(EVENTS.len() <= 64, "Listens keeps one bit per EVENTS row");

fn bit(name: &str) -> u64 {
    EVENTS.iter().position(|(n, _)| *n == name).map_or(0, |i| 1 << i)
}

impl Listens {
    /// Replace the set with `names`, keeping the known ones. `Some` with the
    /// kept names, in `EVENTS` order, when that changed the set.
    pub fn declare<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Option<Vec<&'static str>> {
        let bits = names.into_iter().fold(0, |b, n| b | bit(n));
        if self.0.swap(bits, Ordering::SeqCst) == bits {
            return None;
        }
        Some(EVENTS.iter().enumerate().filter(|(i, _)| bits & (1 << i) != 0).map(|(_, (n, _))| *n).collect())
    }

    pub fn has(&self, event: &str) -> bool {
        let b = bit(event);
        b != 0 && self.0.load(Ordering::SeqCst) & b != 0
    }
}

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
    // No event carries a stamp, so none names a device.
    let view = View { projects: scope, roots: &roots, caller, names: &NoDevices };
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
        let desk = Caller::console();
        assert_eq!(workspace(&View { projects: &all.projects, roots: &roots, caller: &desk, names: &NoDevices }, ws.clone()), ws);
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

    /// A declaration keeps only the names serve emits — one bit each, so a
    /// frame of any length is one word — replaces the last, and says when it
    /// changed something (the session logs that, and only that).
    #[test]
    fn a_declaration_keeps_known_names_and_replaces_the_last() {
        let l = Listens::default();
        assert!(!l.has("app:open-doc"), "nothing is declared until a client says so");
        assert_eq!(l.declare(["no:such", "app:open-doc", "places:changed", "app:open-doc"]), Some(vec!["places:changed", "app:open-doc"]));
        assert!(l.has("app:open-doc") && l.has("places:changed") && !l.has("sessions:busy"));
        assert!(!l.has("no:such"), "an unknown name is ignored, not kept");
        assert_eq!(l.declare(["places:changed", "app:open-doc"]), None, "the same set again changes nothing");
        let flood: Vec<String> = (0..10_000).map(|i| format!("x:{i}")).collect();
        assert_eq!(l.declare(flood.iter().map(String::as_str)), Some(vec![]), "a replacement, not an addition");
        assert!(!l.has("app:open-doc") && !l.has("places:changed"));
        let shared = l.clone();
        l.declare(["app:open-doc"]);
        assert!(shared.has("app:open-doc"), "the session's handle and the broadcast's are one set");
        for name in ONE_SHOT {
            assert!(EVENTS.iter().any(|(n, _)| n == name), "{name} is one-shot and has no EVENTS row, so it could never be declared");
        }
        for name in SNAPSHOT {
            assert!(EVENTS.iter().any(|(n, _)| n == name), "{name} is a snapshot and has no EVENTS row, so it could never be declared");
            assert!(!is_one_shot(name), "{name}: a state is not an act consumed by being sent");
        }
    }

    /// `list_workspace` holds each project's places under `snapshot`, and
    /// their provenance stamps are projected there as `list_places`' are
    /// (dispatch's test drives that row through a real body): cut to the
    /// scope first, then each stamp named, another device's id and reach
    /// gone. A project with no snapshot, or a place with nothing declared,
    /// passes as it was.
    #[test]
    fn a_workspaces_stamps_are_projected_in_every_project() {
        struct Named;
        impl Registry for Named {
            fn principal_grant(&self, _: &PrincipalId) -> Option<Grant> {
                None
            }
            fn device_grant(&self, _: &PrincipalId, _: &DeviceId) -> Option<Grant> {
                None
            }
            fn device_principal(&self, _: &DeviceId) -> Option<PrincipalId> {
                None
            }
            fn stepup_required(&self, _: &PrincipalId, _: &DeviceId) -> bool {
                true
            }
            fn generation(&self) -> u64 {
                0
            }
            fn device_name(&self, p: &PrincipalId, d: &DeviceId) -> Option<String> {
                (p.is_owner() && d.as_str() == "launcher").then(|| "Bret's iPhone".into())
            }
        }
        let stamp = json!({ "principal": "owner", "device": { "kind": "paired", "id": "launcher" }, "cross_project": "off" });
        let place = |slug: &str, declared: Value| json!({ "slug": slug, "declared": declared });
        let ws = json!({ "projects": [
            { "root": "/r/a", "ok": true, "snapshot": { "places": [place("(main)", json!({ "launched_by": stamp })), place("x", Value::Null)] } },
            { "root": "/r/b", "ok": true, "snapshot": { "places": [place("(main)", json!({ "launched_by": stamp }))] } },
            { "root": "/r/a/inner", "ok": false, "snapshot": null },
        ] });
        let roots = reg();
        let at = |v: &Value, i: usize| v["projects"][i]["snapshot"]["places"][0]["declared"]["launched_by"].clone();
        let shared = json!({ "principal": "owner", "device": { "kind": "paired" }, "device_name": "Bret's iPhone" });
        let mut whole = stamp.clone();
        whole["device_name"] = json!("Bret's iPhone");

        let all = Grant::everything();
        let desk = Caller::console();
        let v = workspace(&View { projects: &all.projects, roots: &roots, caller: &desk, names: &Named }, ws.clone());
        assert_eq!((at(&v, 0), at(&v, 1)), (whole.clone(), whole.clone()), "the console: every project, whole and named");
        assert_eq!(v["projects"][0]["snapshot"]["places"][1], ws["projects"][0]["snapshot"]["places"][1]);
        assert_eq!(v["projects"][2], ws["projects"][2]);

        let me = phone();
        let v = workspace(&View { projects: &all.projects, roots: &roots, caller: &me, names: &Named }, ws.clone());
        assert_eq!((at(&v, 0), at(&v, 1)), (shared.clone(), shared.clone()), "another device, in every project");

        let g = scoped(&["/r/a"]);
        let launcher = Caller::paired(PrincipalId::owner(), DeviceId::new("launcher"));
        let v = workspace(&View { projects: &g.projects, roots: &roots, caller: &launcher, names: &Named }, ws.clone());
        assert_eq!(v["projects"].as_array().map(Vec::len), Some(1), "cut to the scope first");
        assert_eq!(at(&v, 0), whole, "the launching device sees its own stamp whole");
    }

    /// Every declared field that records a caller is a stamp: `Declared`'s
    /// fields typed `By`, or a struct that flattens one, read off store.rs.
    /// One missing from [`STAMPS`] would reach every device unprojected, its
    /// device's id and all.
    #[test]
    fn every_declared_field_that_names_a_caller_is_a_stamp() {
        let src = include_str!("../../../worktrees-core/src/store.rs");
        let src = src.split("\n#[cfg(test)]\nmod tests").next().unwrap();
        let body = |name: &str| -> &str {
            let at = src.find(&format!("pub struct {name} {{")).unwrap_or_else(|| panic!("no struct {name}"));
            let rest = &src[at..];
            &rest[..rest.find("\n}").unwrap()]
        };
        let mut types = vec!["By".to_string()];
        for (i, _) in src.match_indices("pub struct ") {
            let name: String = src[i + "pub struct ".len()..].chars().take_while(|c| c.is_alphanumeric()).collect();
            if body(&name).contains("pub by: By,") {
                types.push(name);
            }
        }
        assert!(types.len() > 2, "the scan found the stamp types: {types:?}");
        let mut fields: Vec<String> = body("Declared")
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub ")?.split_once(": Option<"))
            .filter(|(_, ty)| types.iter().any(|t| ty.trim_end_matches(">,") == t))
            .map(|(f, _)| f.to_string())
            .collect();
        fields.sort();
        let mut want: Vec<String> = STAMPS.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(fields, want);
    }

    #[test]
    fn results_are_cut_to_the_scope() {
        let g = scoped(&["/r/a"]);
        let roots = reg();
        let me = phone();
        let v = View { projects: &g.projects, roots: &roots, caller: &me, names: &NoDevices };
        let ws = json!({"projects": [{"root": "/r/a", "ok": true}, {"root": "/r/b", "ok": true}]});
        assert_eq!(workspace(&v, ws), json!({"projects": [{"root": "/r/a", "ok": true}]}));
        let d = json!([{"path": "/r/a/.worktrees/x", "text": "t"}, {"path": "/r/b", "text": "u"}, {"path": "/r/a/inner", "text": "v"}]);
        assert_eq!(drafts(&v, d), json!([{"path": "/r/a/.worktrees/x", "text": "t"}]));
        let pl = json!({"default": "off", "projects": [{"root": "/r/b"}, {"root": "/r/a"}]});
        assert_eq!(by_project(&v, pl), json!({"default": "off", "projects": [{"root": "/r/a"}]}));
    }
}
