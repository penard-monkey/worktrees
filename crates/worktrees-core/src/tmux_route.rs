//! Project routing and the restart-based legacy drain. Endpoint failures remain
//! explicit; a previously seen server never becomes launch permission on error.
use std::path::{Path, PathBuf};
use std::fs;
use serde::{Serialize, Deserialize};
use crate::{tmux, tmux_server::{TmuxServer, EndpointState, EndpointSnapshot, resolve_lane}};

fn state_dir() -> PathBuf { crate::config::config_toml_path().with_file_name("tmux") }
fn path_key(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    crate::tmux_server::identity_hash(path.as_os_str().as_bytes())
}
fn marker(server: &TmuxServer, kind: &str) -> PathBuf {
    state_dir().join(format!("{kind}-{}", path_key(&server.socket_path())))
}
pub(crate) fn remember(server: &TmuxServer) -> Result<(), String> {
    fs::create_dir_all(state_dir()).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(marker(server, "closed"));
    fs::OpenOptions::new().write(true).create(true).truncate(false).open(marker(server, "known"))
        .map(|_| ()).map_err(|e| format!("cannot record tmux endpoint: {e}"))
}
pub(crate) fn forget_empty(server: &TmuxServer) {
    let _ = fs::write(marker(server, "closed"), b"");
    let _ = fs::remove_file(marker(server, "known"));
}
fn known(server: &TmuxServer) -> bool { marker(server, "known").exists() }

#[derive(Serialize, Deserialize)]
struct Legacy { socket: PathBuf }

pub struct Routes { pub project: TmuxServer, legacy: Vec<TmuxServer> }
impl Routes {
    pub fn discover(common: &Path) -> Result<Self, String> {
        let project = TmuxServer::for_project(common).map_err(|e| e.to_string())?;
        let mut legacy = Vec::new();
        if project.namespace().is_none() {
            let root = std::env::var_os("TMUX_TMPDIR").filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
            let mut discovered = vec![(root.join(format!("tmux-{}/default", unsafe { libc::getuid() })), false)];
            if let Some(socket) = std::env::var("TMUX").ok().and_then(|s| s.rsplitn(3, ',').nth(2).map(PathBuf::from)) {
                if !socket.file_name().is_some_and(|n| n.to_string_lossy().starts_with("wt-")) { discovered.push((socket, true)); }
            }
            fs::create_dir_all(state_dir()).map_err(|e| e.to_string())?;
            for (socket, inherited) in discovered {
                let server = TmuxServer::legacy(socket).map_err(|e| e.to_string())?;
                let path = marker(&server, "legacy").with_extension("json");
                if inherited && !path.exists() { remember(&server)?; }
                // Same immutable value for every writer; publish once without
                // exposing a partially written path record to another process.
                crate::tmux_server::publish_json(&path, &Legacy { socket: server.socket_path() }).map_err(|e| e.to_string())?;
            }
            for item in fs::read_dir(state_dir()).map_err(|e| e.to_string())? {
                let item = item.map_err(|e| e.to_string())?;
                let name = item.file_name().to_string_lossy().into_owned();
                if !name.starts_with("legacy-") || !name.ends_with(".json") { continue; }
                let record: Legacy = serde_json::from_slice(&fs::read(item.path()).map_err(|e| e.to_string())?).map_err(|e| format!("invalid legacy tmux inventory: {e}"))?;
                let server = TmuxServer::legacy(record.socket).map_err(|e| e.to_string())?;
                if !marker(&server, "retired").exists() && !legacy.contains(&server) { legacy.push(server); }
            }
        }
        Ok(Self { project, legacy })
    }
    pub fn snapshot(&self) -> Inventory { self.snapshot_with(&mut std::collections::HashMap::new()) }

    pub fn snapshot_with(&self, cache: &mut std::collections::HashMap<TmuxServer, Result<tmux::PaneList, String>>) -> Inventory {
        let endpoints = std::iter::once(&self.project).chain(self.legacy.iter()).map(|server| {
            let result = cache.entry(server.clone()).or_insert_with(|| probe(server)).clone();
            if self.legacy.contains(server) && result.as_ref().is_ok_and(|p| p.names().is_empty()) {
                // Empty is confirmed by a successful query, or by an endpoint
                // that was never seen live. A failed known endpoint is Err.
                let _ = fs::write(marker(server, "retired"), b"");
            }
            Endpoint { server: server.clone(), result }
        }).collect();
        Inventory { project: self.project.clone(), endpoints }
    }
}

pub fn probe(server: &TmuxServer) -> Result<tmux::PaneList, String> {
    match tmux::PaneList::try_fetch(server) {
        Ok(panes) => {
            if panes.names().is_empty() { forget_empty(server); } else { remember(server)?; }
            Ok(panes)
        }
        Err(reason) if !known(server)
            && (!server.socket_path().exists() || marker(server, "closed").exists())
            && !reason.starts_with("cannot execute tmux:")
            && (reason.contains("no server running on ") || reason.contains("No such file or directory")) => {
            Ok(tmux::PaneList::empty(server))
        }
        Err(reason) => Err(format!("tmux server unreachable at {}: {reason}; recover it before reopening. If you have verified this server is stopped, remove socket {} and marker {}", server.socket_path().display(), server.socket_path().display(), marker(server, "known").display())),
    }
}

pub struct Endpoint { pub server: TmuxServer, pub result: Result<tmux::PaneList, String> }
pub struct Inventory { pub project: TmuxServer, pub endpoints: Vec<Endpoint> }
impl Inventory {
    pub fn lane(&self, canonical: &str, path: &str, exclude: Option<&str>) -> Result<tmux::PaneList, String> {
        let mut candidates = vec![canonical.to_string()];
        for endpoint in &self.endpoints {
            if let Ok(panes) = &endpoint.result {
                candidates.extend(panes.candidates(path, exclude));
            }
        }
        let snapshots: Vec<_> = self.endpoints.iter().map(|e| EndpointSnapshot {
            server: e.server.clone(),
            state: match &e.result {
                Ok(panes) => EndpointState::Sessions(panes.names()),
                Err(reason) => EndpointState::Unreachable(reason.clone()),
            },
        }).collect();
        let server = resolve_lane(&self.project, &snapshots, &candidates)?;
        self.endpoints.iter().find(|e| e.server == server).expect("resolved inspected endpoint").result.clone()
    }
}
