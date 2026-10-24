//! The shim as the remote door (Phase 2 §4.4): this window driving ANOTHER
//! host's `worktrees serve`, over `worktrees_remote`'s pinned mutual-TLS pipe.
//!
//! **A pipe, not a second client.** `remote_connect` opens the socket and
//! answers `{conn, token, host}`; every text frame the host sends goes to the
//! page through one Channel as `{frame: text}`, and the end as exactly one
//! `{closed: why}`; `remote_send {conn, text}` writes one frame. Nothing here
//! reads a frame: handshake v1, requests, channels and events are
//! `app/src/ipcws.ts`'s, which runs unchanged over a socket built from these
//! three commands (`app/src/ipcremote.ts`). `token` is the session's
//! handshake-v1 key (the TLS exporter under `EXPORTER-worktrees-remote-v1`),
//! handed over the way `engine_connect` hands over serve.json's — and unlike
//! that one it works only inside a TLS session this process alone holds.
//!
//! **The page never sees the key or the pin.** It names a host by the id this
//! machine gave the pairing; every answer about a host is a `HostSummary`.
//! The key file stays in the config dir, read here at each connect.
//!
//! **One remote socket per window.** A connect replaces the one before it,
//! and `conn` (a fresh number per socket) is on every send and close, so a
//! page's stale socket cannot write into the next one.
//!
//! **A host that moved is followed, never trusted.** When the host is not at
//! its kept address — nothing answers, or a machine with another key does —
//! `remote_connect` looks for it on the LAN for one bounded round
//! (`worktrees_remote::reach`): a discovered address is kept only once a
//! connection to it proves the pinned key, and every advert that did not is
//! in app.log. The host refusing this device's key is not a move and sends
//! nobody looking. `remote_readdress` follows an address code by the
//! same rule. Each of `reconnect.ts`'s retries is one such connect.
//!
//! Pairing (`remote_pair`), the host list and the choice of engine live here
//! too: they are the CLIENT's, kept in `remote-hosts.json` beside the app's
//! other files, shared with `worktrees remote`. With a host chosen, the page
//! dials it and this shim does not find or start a local serve at all
//! (`engine_connect` is never asked).
//!
//! **View state is per device** (§9.1): a window on a remote host keeps its
//! settings blob in `remote-ui-state.json` here, never in the host's
//! `ui-state.json` — written whole by the page, as that one is, and never
//! edited by this side.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::ipc::Channel;
use tokio::sync::Mutex;
use worktrees_remote::hosts::{HostRecord, HostStore, HostSummary};
use worktrees_remote::pair::{self, PairEvent};
use worktrees_remote::pipe::{Inbound, PipeTx};
use worktrees_remote::reach::{self, ReachError};
use worktrees_remote::sign::Keys;

/// The settings blob of a window that drives a remote host.
pub const REMOTE_UI_STATE: &str = "remote-ui-state.json";

/// How long a connect may take, TLS and upgrade included.
const CONNECT_WAIT: Duration = Duration::from_secs(10);

struct Live {
    conn: u64,
    tx: Arc<Mutex<Option<PipeTx>>>,
}

pub struct Remote {
    dir: PathBuf,
    store: HostStore,
    live: Mutex<Option<Live>>,
    next: AtomicU64,
}

/// `remote_connect`'s answer: never the pin, never the key.
#[derive(Serialize)]
pub struct Connected {
    pub conn: u64,
    pub token: String,
    pub host: HostSummary,
}

/// `remote_hosts`' answer.
#[derive(Serialize)]
pub struct HostList {
    pub chosen: Option<String>,
    pub hosts: Vec<HostSummary>,
    /// This machine's default name for a pairing.
    pub this_name: String,
}

impl Remote {
    pub fn new(dir: &Path) -> Remote {
        Remote { dir: dir.to_path_buf(), store: HostStore::new(dir), live: Mutex::new(None), next: AtomicU64::new(1) }
    }

    pub fn hosts(&self) -> Result<HostList, String> {
        let h = self.store.list()?;
        Ok(HostList { chosen: h.chosen, hosts: h.hosts.iter().map(HostRecord::summary).collect(), this_name: pair::this_name() })
    }

    pub fn choose(&self, id: Option<&str>) -> Result<(), String> {
        self.store.choose(id)
    }

    pub async fn forget(&self, id: &str) -> Result<HostSummary, String> {
        let rec = self.store.forget(id)?;
        Ok(rec.summary())
    }

    pub async fn pair(&self, link: &str, name: &str, events: &Channel<Value>) -> Result<HostSummary, String> {
        let mut on = |ev: PairEvent| {
            let _ = match ev {
                PairEvent::Dialling { host, addr } => events.send(json!({"dialling": {"host": host, "addr": addr.to_string()}})),
                PairEvent::Code(c) => events.send(json!({"code": c})),
            };
        };
        let rec = pair::pair(&self.store, link, name, worktrees_core::sysclock::now_epoch(), &mut on).await.map_err(|e| e.to_string())?;
        crate::applog("info", &format!("remote: paired with {} ({}) as device {}", rec.name, rec.addr, rec.device_id));
        Ok(rec.summary())
    }

    /// Open the socket to `id`, replacing any other, and pipe what it sends to
    /// `frames`.
    pub async fn connect(&self, id: &str, frames: Channel<Value>) -> Result<Connected, String> {
        let rec = self.store.get(id)?;
        if rec.id != id {
            return Err(format!("{id:?} is not a host id"));
        }
        let key = self.store.key_for(&rec)?;
        self.close_live().await;
        // Logged: a remote window that cannot connect shows a banner, and the
        // person at it reads app.log for the rest (the host's serve.log is on
        // the host).
        let mut said = |m: &str| crate::applog("info", &format!("remote: {}", worktrees_remote::printable(m)));
        let reached = reach::open(&self.store, &rec, &Keys::from(key), &reach::Env::platform(), CONNECT_WAIT, &mut said).await.map_err(|e| {
            let m = match &e {
                ReachError::NotFound { .. } => format!("{} Then paste it in Settings → Hosts → Paste a new address. {}", e.headline(), e.details()).trim_end().to_string(),
                e => format!("{} ({}): {e}", rec.name, rec.addr),
            };
            crate::applog("warn", &format!("remote: connect failed: {m}"));
            m
        })?;
        let (p, rec) = (reached.pipe, reached.rec);
        let token = p.session_token().to_string();
        let (tx, mut rx) = p.split();
        let conn = self.next.fetch_add(1, Ordering::SeqCst);
        let tx = Arc::new(Mutex::new(Some(tx)));
        *self.live.lock().await = Some(Live { conn, tx: tx.clone() });
        let name = rec.name.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Inbound::Frame(t) => {
                        if frames.send(json!({ "frame": t })).is_err() {
                            break;
                        }
                    }
                    Inbound::Closed(why) => {
                        crate::applog("info", &format!("remote: {name}: {why}"));
                        let _ = frames.send(json!({ "closed": why }));
                        break;
                    }
                }
            }
            // The page is gone or the socket is: nothing more goes out on it.
            if let Some(tx) = tx.lock().await.take() {
                tx.close().await;
            }
        });
        crate::applog("info", &format!("remote: connected to {} ({}), conn {conn}", rec.name, rec.addr));
        Ok(Connected { conn, token, host: rec.summary() })
    }

    /// Follow `id` to the address an address code names, once a pinned
    /// connection there succeeds.
    pub async fn readdress(&self, id: &str, code: &str) -> Result<HostSummary, String> {
        let code = reach::parse_code(code)?;
        let rec = self.store.get(id)?;
        if rec.id != id {
            return Err(format!("{id:?} is not a host id"));
        }
        let not_followed = |e: String| {
            crate::applog("warn", &format!("remote: {}: address code not followed: {}", rec.name, worktrees_remote::printable(&e)));
            e
        };
        // A code for another host is refused before the key file is read.
        reach::check_code(&rec, &code).map_err(not_followed)?;
        let keys = Keys::from(self.store.key_for(&rec).map_err(not_followed)?);
        let moved = reach::adopt_code(&self.store, &rec, &keys, &code, &reach::Env::platform(), CONNECT_WAIT).await.map_err(not_followed)?;
        crate::applog("info", &format!("remote: {} now at {} (was {}), by its address code", moved.name, moved.addr, rec.addr));
        Ok(moved.summary())
    }

    pub async fn send(&self, conn: u64, text: String) -> Result<(), String> {
        let tx = match self.live.lock().await.as_ref() {
            Some(l) if l.conn == conn => l.tx.clone(),
            _ => return Err("this remote connection has been closed or replaced".into()),
        };
        let mut g = tx.lock().await;
        match g.as_mut() {
            Some(t) => t.send(text).await,
            None => Err("this remote connection has closed".into()),
        }
    }

    pub async fn close(&self, conn: u64) -> Result<(), String> {
        let mut live = self.live.lock().await;
        if live.as_ref().is_some_and(|l| l.conn == conn) {
            let l = live.take().expect("checked");
            drop(live);
            let tx = l.tx.lock().await.take();
            if let Some(tx) = tx {
                tx.close().await;
            }
        }
        Ok(())
    }

    async fn close_live(&self) {
        let l = self.live.lock().await.take();
        if let Some(l) = l {
            let tx = l.tx.lock().await.take();
            if let Some(tx) = tx {
                tx.close().await;
            }
        }
    }

    pub fn settings_get(&self) -> Result<Option<Value>, String> {
        match std::fs::read(self.dir.join(REMOTE_UI_STATE)) {
            Ok(b) => Ok(serde_json::from_slice(&b).ok()),
            Err(_) => Ok(None),
        }
    }

    pub fn settings_set(&self, settings: Value) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let json = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
        worktrees_core::fsx::write_atomic(&self.dir.join(REMOTE_UI_STATE), &json, Some(std::fs::Permissions::from_mode(0o600)))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> HostRecord {
        HostRecord {
            id: worktrees_remote::hosts::new_host_id(),
            name: "Mac mini".into(),
            addr: "192.168.1.5:7461".into(),
            pin: "cd".repeat(32),
            discovery_id: None,
            device_id: "d_abc".into(),
            principal_id: "owner".into(),
            expiry: json!({"idle_days": 30}),
            paired_at: 1,
        }
    }

    /// §4.4: the page never sees the pin or the key. What the shim answers
    /// about a host — the list and a connect — is serialized here and
    /// searched for both.
    #[test]
    fn what_the_page_is_told_about_a_host_holds_no_pin_and_no_key() {
        let d = std::env::temp_dir().join(format!("wt-shim-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let r = Remote::new(&d);
        let rec = record();
        let key = worktrees_remote::key::DeviceKey::generate().unwrap();
        r.store.add(&rec, &key).unwrap();
        let list = serde_json::to_string(&r.hosts().unwrap()).unwrap();
        let conn = serde_json::to_string(&Connected { conn: 1, token: "ab".repeat(32), host: r.store.get(&rec.id).unwrap().summary() }).unwrap();
        for s in [&list, &conn] {
            assert!(!s.contains(&rec.pin), "the pin: {s}");
            assert!(!s.contains(key.pin()), "the key's pin: {s}");
            assert!(!s.contains("remote-keys") && !s.contains(".key"), "a key path: {s}");
        }
        assert!(list.contains(&rec.id) && list.contains("Mac mini"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An address code for another host is refused on its pin before this
    /// machine's key file is read: here the key file is one `key_for` would
    /// refuse, and the answer is still the pin's. Nothing is kept.
    #[test]
    fn a_code_for_another_host_is_refused_before_the_key_file_is_read() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("wt-shim-remote-code-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let r = Remote::new(&d);
        let rec = record();
        r.store.add(&rec, &worktrees_remote::key::DeviceKey::generate().unwrap()).unwrap();
        let key = d.join("remote-keys").join(format!("{}.key", rec.id));
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(r.store.key_for(&rec).is_err(), "a key file key_for refuses");
        let before = std::fs::read(d.join("remote-hosts.json")).unwrap();
        let code = format!("worktrees-addr:v1?addr=192.168.1.9:7461&pin={}", "ef".repeat(32));
        let e = tauri::async_runtime::block_on(r.readdress(&rec.id, &code)).unwrap_err();
        assert!(e.contains("for another host") && e.contains("Nothing was dialled"), "{e}");
        assert_eq!(std::fs::read(d.join("remote-hosts.json")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The per-device settings blob is this window's, 0600, read back whole.
    #[test]
    fn a_remote_windows_settings_stay_on_this_machine() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("wt-shim-remote-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let r = Remote::new(&d);
        assert_eq!(r.settings_get().unwrap(), None);
        r.settings_set(json!({"theme": "tokyo-night"})).unwrap();
        assert_eq!(r.settings_get().unwrap(), Some(json!({"theme": "tokyo-night"})));
        assert_eq!(std::fs::metadata(d.join(REMOTE_UI_STATE)).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!d.join("ui-state.json").exists(), "the host's file is not this one");
        let _ = std::fs::remove_dir_all(&d);
    }
}
