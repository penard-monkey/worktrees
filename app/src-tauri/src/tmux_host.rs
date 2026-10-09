//! App adapters retain the core endpoint; the webview supplies place context,
//! never a socket path. Polls enumerate each endpoint once across projects.
use std::{collections::HashMap, path::Path, sync::{Mutex, OnceLock}};
use worktrees_core::{Project, tmux::PaneList, tmux_route, tmux_server::TmuxServer};

static PROJECTS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
pub fn register(p: &Project) {
    PROJECTS.get_or_init(Default::default).lock().unwrap().insert(p.main_root.clone(), p.git_common.clone());
}

pub fn resolve(root: &str, session: &str) -> Result<TmuxServer, String> {
    let p = Project::discover(Path::new(root)).map_err(|e| e.msg)?;
    let path = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let place = p.place_index().into_iter().find(|r| Path::new(&r.path) == path)
        .ok_or_else(|| format!("not a place root: {root}"))?;
    let panes = p.lane_panes(&place.slug, &place.path)?;
    if !panes.has_session(session) { return Err(format!("session '{session}' is no longer running in this place")); }
    // A supplied name must actually belong to this lane, not merely share its
    // project's server. Canonical/sidecars and cwd adoption use core's rules.
    let canonical = p.session_name(&place.slug);
    if session != canonical && worktrees_core::tmux::shell_sidecar_index(&canonical, session).is_none()
        && !session.strip_prefix(&canonical).is_some_and(|suffix| suffix.starts_with(worktrees_core::provider::SIDECAR_MARKER))
        && !panes.candidates(&place.path, place.is_main.then_some(p.wt_root_dir())).iter().any(|s| s == session) {
        return Err(format!("session '{session}' does not belong to this place"));
    }
    Ok(panes.server)
}

#[derive(Default)]
pub struct Poll { pub endpoints: HashMap<TmuxServer, Result<PaneList, String>> }
impl Poll {
    pub fn read() -> Self {
        let mut projects = PROJECTS.get_or_init(Default::default).lock().unwrap().clone();
        let roots = worktrees_core::registry::read_lenient().roots();
        projects.retain(|root, _| roots.contains(root));
        let mut endpoints = HashMap::new();
        for common in projects.values() {
            if let Ok(routes) = tmux_route::Routes::discover(Path::new(common)) {
                for e in routes.snapshot_with(&mut endpoints).endpoints { endpoints.insert(e.server, e.result); }
            }
        }
        Self { endpoints }
    }
    pub fn fingerprint(&self) -> Vec<(String, Result<Vec<String>, String>)> {
        let mut rows: Vec<_> = self.endpoints.iter().map(|(s, p)| (s.socket_path().to_string_lossy().into_owned(), p.as_ref().map(PaneList::names).map_err(Clone::clone))).collect();
        rows.sort_by(|a,b| a.0.cmp(&b.0)); rows
    }
    pub fn names(&self, server: &TmuxServer) -> Option<String> {
        self.endpoints.get(server)?.as_ref().ok().map(|p| p.names().join("\n"))
    }
}

// Public event payloads remain place paths. Their *memory* is bounded by the
// resolved endpoint/session/launch, so drain and close/reopen cannot carry a
// previous session's dwell counter or completion suppression to its successor.
pub type LaneIdentity = Vec<(TmuxServer, String, Option<String>)>;
static LANES: OnceLock<Mutex<HashMap<String, Option<LaneIdentity>>>> = OnceLock::new();
pub fn watch_place(path: &str, identity: Option<LaneIdentity>) {
    LANES.get_or_init(Default::default).lock().unwrap().insert(path.into(), identity);
}
pub fn identities() -> HashMap<String, Option<LaneIdentity>> {
    LANES.get_or_init(Default::default).lock().unwrap().clone()
}
pub fn changed_places(before: &HashMap<String, Option<LaneIdentity>>, now: &HashMap<String, Option<LaneIdentity>>) -> Vec<String> {
    let mut changed: Vec<_> = before.keys().chain(now.keys()).filter(|path| before.get(*path) != now.get(*path)).cloned().collect();
    changed.sort();
    changed.dedup();
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_or_launch_change_resets_only_that_places_activity_memory() {
        let a = TmuxServer::legacy("/tmp/routing-unit-a".into()).unwrap();
        let b = TmuxServer::legacy("/tmp/routing-unit-b".into()).unwrap();
        let identity = |server, launch: &str| Some(vec![(server, "same".into(), Some(launch.into()))]);
        let before = HashMap::from([
            ("/project/a".into(), identity(a.clone(), "1")),
            ("/project/b".into(), identity(b.clone(), "1")),
        ]);
        assert!(changed_places(&before, &before).is_empty());
        for successor in [identity(b, "1"), identity(a, "2"), None, Some(vec![])] {
            let mut after = before.clone();
            after.insert("/project/a".into(), successor);
            assert_eq!(changed_places(&before, &after), ["/project/a"]);
        }
    }
}
