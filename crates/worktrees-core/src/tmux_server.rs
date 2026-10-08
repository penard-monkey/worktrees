//! Local tmux endpoint identity. No process is started by assigning a descriptor.
//!
//! Assignments live in the git common directory (never in repo config); a
//! user-local claim prevents two identities from silently sharing a socket.
//! The readable part is frozen at assignment, and moving a repository fails
//! closed until its old endpoint has been explicitly dealt with.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Endpoint {
    Project { key: String },
    Legacy { socket: PathBuf },
}

/// Required routing identity for a tmux connection. Equality includes the
/// endpoint, socket root, and sandbox namespace, so `%1` on two servers differs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct TmuxServer {
    endpoint: Endpoint,
    socket_root: PathBuf,
    namespace: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Assignment {
    version: u32,
    common_dir: PathBuf,
    namespace: Option<String>,
    key: String,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// FNV-1a 64, fixed bytes and constants, independent of Rust's Hash/Hasher.
/// This is a routing key, not a security digest; the full identity is checked
/// against the local claim before the endpoint may be used.
fn identity_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn readable(common: &Path) -> String {
    let name = common.parent().and_then(Path::file_name).unwrap_or_default().to_string_lossy();
    let name: String = name.chars().take(20).map(|c| {
        if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }
    }).collect();
    let name = name.trim_matches('-');
    if name.is_empty() { "project".into() } else { name.into() }
}

fn validate_namespace(namespace: Option<&str>) -> io::Result<()> {
    if let Some(n) = namespace {
        if n.is_empty() || n.len() > 32 || !n.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
            return Err(invalid("WORKTREES_TMUX_NAMESPACE must be 1–32 ASCII letters, digits, '-' or '_'"));
        }
    }
    Ok(())
}

impl TmuxServer {
    /// Production assignment. Namespace is an explicit user environment choice;
    /// neither the registry display name nor project configuration is consulted.
    pub fn for_project(common_dir: &Path) -> io::Result<Self> {
        let namespace = match std::env::var("WORKTREES_TMUX_NAMESPACE") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(e) => return Err(invalid(format!("WORKTREES_TMUX_NAMESPACE: {e}"))),
        };
        let home = std::env::var_os("HOME").ok_or_else(|| invalid("HOME is required for tmux identity claims"))?;
        Self::assign(common_dir, namespace.as_deref(), &PathBuf::from(home).join(".local/state/worktrees/tmux"))
    }

    fn assign(common_dir: &Path, namespace: Option<&str>, claims: &Path) -> io::Result<Self> {
        use std::os::unix::ffi::OsStrExt;
        validate_namespace(namespace)?;
        let common = fs::canonicalize(common_dir)?;
        let mut identity = common.as_os_str().as_bytes().to_vec();
        identity.push(0); // separate namespace from a path ending in the same bytes
        identity.extend_from_slice(namespace.unwrap_or("").as_bytes());
        let hash = identity_hash(&identity);
        let assignment_dir = common.join("worktrees-tmux");
        fs::create_dir_all(&assignment_dir)?;
        // Namespace labels cannot contain path separators. Keep separate sandbox
        // assignments so testing the same repo cannot overwrite the normal one.
        let filename = match namespace {
            Some(n) => format!("sandbox-{n}.json"),
            None => "project.json".into(),
        };
        let path = assignment_dir.join(filename);
        let proposed = Assignment {
            version: 1,
            common_dir: common.clone(),
            namespace: namespace.map(str::to_owned),
            key: format!("wt-{}-{hash}", readable(&common)),
        };
        publish(&path, &proposed)?;
        let assigned: Assignment = serde_json::from_slice(&fs::read(&path)?).map_err(|e| invalid(format!("tmux assignment {}: {e}", path.display())))?;
        if assigned.version != 1 || assigned.common_dir != common || assigned.namespace != proposed.namespace {
            return Err(invalid(format!("tmux endpoint identity changed at {}; repository moves require explicit endpoint rebinding before launch", path.display())));
        }
        // Validate persisted data before it can become argv or a claim filename.
        if !assigned.key.starts_with("wt-") || !assigned.key.ends_with(&format!("-{hash}")) || assigned.key.len() > 40
            || !assigned.key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
            return Err(invalid("invalid local tmux endpoint key"));
        }
        fs::create_dir_all(claims)?;
        let claim_path = claims.join(format!("{}.json", assigned.key));
        publish(&claim_path, &assigned)?;
        let claim: Assignment = serde_json::from_slice(&fs::read(&claim_path)?).map_err(|e| invalid(format!("tmux identity claim: {e}")))?;
        if claim != assigned {
            return Err(invalid(format!("tmux endpoint collision for {}; refusing to share another project's server", assigned.key)));
        }
        Ok(Self {
            endpoint: Endpoint::Project { key: assigned.key },
            // tmux's socket root must agree across launchd, login shells and
            // inherited tmux panes. Never use env::temp_dir() here (TMPDIR).
            socket_root: PathBuf::from("/tmp"),
            namespace: assigned.namespace,
        })
    }

    /// A discovered legacy endpoint is kept explicit throughout the drain.
    /// No operation may fall back to whatever server the caller inherited.
    pub fn legacy(socket: PathBuf) -> io::Result<Self> {
        if !socket.is_absolute() { return Err(invalid("legacy tmux socket must be absolute")); }
        Ok(Self { endpoint: Endpoint::Legacy { socket }, socket_root: PathBuf::from("/tmp"), namespace: None })
    }

    pub fn namespace(&self) -> Option<&str> { self.namespace.as_deref() }

    /// Global options shared by std::process and the app's PTY adapter.
    /// The hotfix's feature-detected -N is added independently by the caller.
    pub fn endpoint_args(&self) -> Vec<std::ffi::OsString> {
        match &self.endpoint {
            Endpoint::Project { key } => vec!["-L".into(), key.into()],
            Endpoint::Legacy { socket } => vec!["-S".into(), socket.as_os_str().into()],
        }
    }

    pub fn socket_root(&self) -> &Path { &self.socket_root }

    pub fn socket_path(&self) -> PathBuf {
        match &self.endpoint {
            Endpoint::Project { key } => self.socket_root.join(format!("tmux-{}", unsafe { libc::getuid() })).join(key),
            Endpoint::Legacy { socket } => socket.clone(),
        }
    }

    pub fn configure(&self, command: &mut std::process::Command) {
        command.args(self.endpoint_args()).env("TMUX_TMPDIR", &self.socket_root).env_remove("TMUX");
    }
}

/// Publish a fully written immutable identity without a partial-read window.
/// hard_link is an atomic no-replace operation on the same filesystem: the
/// losing caller reads the winner's complete assignment. This is an identity
/// publication primitive, NOT the phase-1b server startup lock.
fn publish(path: &Path, value: &Assignment) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if path.try_exists()? { return Ok(()); }
    let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        match fs::hard_link(&temp, path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e),
        }
    })();
    let _ = fs::remove_file(temp);
    result
}

/// A session/pane key must carry its server, even if both endpoints happen to
/// use the same session name or pane number.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TmuxTarget {
    pub server: TmuxServer,
    pub target: String,
}

/// Probe failures are data, never an empty session list. `Absent` is allowed
/// only for an endpoint that is not known to have a live server. A previously
/// observed endpoint with a failed probe must be `Unreachable` instead.
#[derive(Clone, Debug)]
pub enum EndpointState {
    Sessions(Vec<String>),
    Absent,
    Unreachable(String),
}

#[derive(Clone, Debug)]
pub struct EndpointSnapshot {
    pub server: TmuxServer,
    pub state: EndpointState,
}

/// Decide a lane's endpoint from ALL candidates (canonical, adopted, provider
/// and shell sidecars). Legacy wins until closed; two endpoints with any lane
/// candidates are ambiguous, including a provider-only or shell-only remnant.
/// A caller supplies candidates established by the existing name/cwd rules.
/// This function does not probe, launch, close, or recover anything.
pub fn resolve_lane(
    project: &TmuxServer,
    snapshots: &[EndpointSnapshot],
    candidates: &[String],
) -> Result<TmuxServer, String> {
    let mut found: Option<&TmuxServer> = None;
    let mut seen_project = false;
    for snapshot in snapshots {
        seen_project |= snapshot.server == *project;
        match &snapshot.state {
            EndpointState::Unreachable(reason) => return Err(format!(
                "tmux server unreachable at {}: {reason}; recover the endpoint before reopening this lane",
                snapshot.server.socket_path().display(),
            )),
            EndpointState::Sessions(names) => {
                let owns_lane = names.iter().any(|name| candidates.iter().any(|candidate| {
                    name == candidate
                        || crate::tmux::shell_sidecar_index(candidate, name).is_some()
                        || name.strip_prefix(candidate).is_some_and(|suffix| suffix.starts_with(crate::provider::SIDECAR_MARKER))
                }));
                if owns_lane {
                    if found.is_some_and(|prior| prior != &snapshot.server) {
                        return Err("ambiguous lane: sessions exist on multiple tmux servers; refusing mutation".into());
                    }
                    found = Some(&snapshot.server);
                }
            }
            EndpointState::Absent => {}
        }
    }
    if !seen_project { return Err("project tmux endpoint was not inspected; refusing launch".into()); }
    Ok(found.unwrap_or(project).clone())
}

/// Retire only after a successful empty list. An error, or an unrelated live
/// session, keeps the shared legacy endpoint in the drain inventory.
pub fn legacy_drained(snapshot: &EndpointSnapshot) -> bool {
    matches!(&snapshot.state, EndpointState::Sessions(names) if names.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("wt-routing-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(path.join("repo/.git")).unwrap();
            Self(path)
        }
        fn common(&self) -> PathBuf { self.0.join("repo/.git") }
        fn claims(&self) -> PathBuf { self.0.join("claims") }
        fn server(&self) -> TmuxServer { TmuxServer::assign(&self.common(), None, &self.claims()).unwrap() }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn stable_hash_has_fixed_known_vectors() {
        assert_eq!(identity_hash(b""), "cbf29ce484222325");
        assert_eq!(identity_hash(b"hello"), "a430d84680aabd0b");
    }

    #[test]
    fn aliases_share_identity_and_clones_and_sandboxes_do_not() {
        let f = Fixture::new();
        let a = f.server();
        std::os::unix::fs::symlink(f.common(), f.0.join("alias")).unwrap();
        assert_eq!(a, TmuxServer::assign(&f.0.join("alias"), None, &f.claims()).unwrap());
        fs::create_dir_all(f.0.join("clone/repo/.git")).unwrap();
        assert_ne!(a, TmuxServer::assign(&f.0.join("clone/repo/.git"), None, &f.claims()).unwrap());
        let sandbox = TmuxServer::assign(&f.common(), Some("dev"), &f.claims()).unwrap();
        assert_ne!(a, sandbox);
        assert_ne!(a.socket_path(), sandbox.socket_path());
        assert_eq!(sandbox.namespace(), Some("dev"));
        assert_eq!(a, f.server());
        assert!(TmuxServer::assign(&f.common(), Some("../escape"), &f.claims()).is_err());
    }

    #[test]
    fn assignment_freezes_name_and_refuses_repository_moves() {
        let f = Fixture::new();
        let first = f.server();
        let path = f.common().join("worktrees-tmux/project.json");
        let mut assignment: Assignment = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assignment.key = assignment.key.replacen("wt-repo-", "wt-frozen-", 1);
        fs::write(&path, serde_json::to_vec(&assignment).unwrap()).unwrap();
        let frozen = f.server();
        assert_ne!(first, frozen);
        assert!(frozen.socket_path().file_name().unwrap().to_str().unwrap().starts_with("wt-frozen-"));
        fs::rename(f.0.join("repo"), f.0.join("moved")).unwrap();
        assert!(TmuxServer::assign(&f.0.join("moved/.git"), None, &f.claims()).unwrap_err().to_string().contains("rebinding"));
    }

    #[test]
    fn collision_and_corrupt_assignment_fail_closed() {
        let f = Fixture::new();
        let a = f.server();
        let key = a.socket_path().file_name().unwrap().to_owned();
        let claim_path = f.claims().join(key).with_extension("json");
        let mut claim: Assignment = serde_json::from_slice(&fs::read(&claim_path).unwrap()).unwrap();
        claim.common_dir = f.0.join("stranger/.git");
        fs::write(&claim_path, serde_json::to_vec(&claim).unwrap()).unwrap();
        assert!(TmuxServer::assign(&f.common(), None, &f.claims()).unwrap_err().to_string().contains("collision"));
        fs::write(f.common().join("worktrees-tmux/project.json"), b"{").unwrap();
        assert!(TmuxServer::assign(&f.common(), None, &f.claims()).unwrap_err().to_string().contains("tmux assignment"));
    }

    #[test]
    fn concurrent_assignment_observes_one_complete_identity() {
        let f = Fixture::new();
        let results = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..12).map(|_| scope.spawn(|| f.server())).collect();
            workers.into_iter().map(|w| w.join().unwrap()).collect::<HashSet<_>>()
        });
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn command_routing_overrides_inherited_server_and_socket_root() {
        let f = Fixture::new();
        let server = f.server();
        let mut cmd = std::process::Command::new("tmux");
        cmd.env("TMUX", "/wrong/socket,42,0").env("TMUX_TMPDIR", "/wrong");
        server.configure(&mut cmd);
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), server.endpoint_args());
        let env: std::collections::HashMap<_, _> = cmd.get_envs().collect();
        assert_eq!(env[std::ffi::OsStr::new("TMUX")], None);
        assert_eq!(env[std::ffi::OsStr::new("TMUX_TMPDIR")], Some(std::ffi::OsStr::new("/tmp")));
    }

    #[test]
    fn identical_panes_on_different_servers_remain_distinct() {
        let f = Fixture::new();
        let a = f.server();
        let b = TmuxServer::assign(&f.common(), Some("test"), &f.claims()).unwrap();
        let keys: HashSet<_> = [a, b].into_iter().map(|server| TmuxTarget { server, target: "%1".into() }).collect();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn drain_routes_legacy_sidecars_and_rejects_ambiguity_and_failed_probes() {
        let f = Fixture::new();
        let project = f.server();
        let legacy = TmuxServer::legacy(f.0.join("legacy.sock")).unwrap();
        let candidates = vec!["repo-lane".into()];
        let snapshot = |server: &TmuxServer, names: &[&str]| EndpointSnapshot {
            server: server.clone(), state: EndpointState::Sessions(names.iter().map(|n| n.to_string()).collect()),
        };
        for name in ["repo-lane", "repo-lane~agent~codex", "repo-lane~agent~pi", "repo-lane~agent~future", "repo-lane~term", "repo-lane~term~2"] {
            let old = snapshot(&legacy, &[name]);
            assert_eq!(resolve_lane(&project, &[snapshot(&project, &[]), old.clone()], &candidates).unwrap(), legacy);
            assert!(resolve_lane(&project, &[snapshot(&project, &["repo-lane"]), old], &candidates).unwrap_err().contains("ambiguous"));
        }
        let unrelated = snapshot(&legacy, &["other-project", "repo-lane-longer"]);
        assert_eq!(resolve_lane(&project, &[snapshot(&project, &[]), unrelated.clone()], &candidates).unwrap(), project);
        assert!(!legacy_drained(&unrelated));
        assert!(legacy_drained(&snapshot(&legacy, &[])));
        let failed = EndpointSnapshot { server: legacy, state: EndpointState::Unreachable("connection refused".into()) };
        assert!(!legacy_drained(&failed));
        assert!(resolve_lane(&project, &[snapshot(&project, &[]), failed], &candidates).unwrap_err().contains("unreachable"));
        assert!(resolve_lane(&project, &[], &candidates).is_err());
    }

    #[test]
    #[ignore = "starts private real tmux servers; run explicitly on the target platform"]
    fn real_servers_keep_same_named_sessions_and_panes_separate() {
        use std::process::{Command, Output};
        fn run(server: &TmuxServer, args: &[&str]) -> Output {
            let mut cmd = Command::new("tmux");
            // Deliberately hostile inherited settings. configure MUST override
            // both; explicit -L/-S on every call makes this witness isolated.
            cmd.env("TMUX", "/does-not-exist/socket,1,0").env("TMUX_TMPDIR", "/does-not-exist");
            server.configure(&mut cmd);
            cmd.args(args).output().unwrap()
        }
        struct Running(Vec<TmuxServer>);
        impl Drop for Running {
            fn drop(&mut self) {
                for server in &self.0 {
                    let _ = run(server, &["-N", "kill-session", "-t", "=same"]);
                }
            }
        }
        let f = Fixture::new();
        let a = f.server();
        let b = TmuxServer::assign(&f.common(), Some("witness"), &f.claims()).unwrap();
        let mut running = Running(Vec::new());
        for (server, marker) in [(&a, "first"), (&b, "second")] {
            let output = run(server, &["-f", "/dev/null", "new-session", "-d", "-s", "same", "-P", "-F", "#{pane_id}",
                &format!("printf '{marker}\\n'; exec sleep 60")]);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            running.0.push(server.clone());
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "%0");
        }
        for (server, marker) in [(&a, "first"), (&b, "second")] {
            let mut observed = false;
            for _ in 0..40 {
                let output = run(server, &["-N", "capture-pane", "-p", "-t", "%0"]);
                assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
                if String::from_utf8_lossy(&output.stdout).trim() == marker { observed = true; break; }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            assert!(observed, "capture from {} did not contain {marker}", server.socket_path().display());
        }
        assert!(run(&a, &["-N", "kill-session", "-t", "=same"]).status.success());
        assert!(run(&b, &["-N", "has-session", "-t", "=same"]).status.success());
    }

}
