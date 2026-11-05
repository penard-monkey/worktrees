//! `<config_dir>/push-devices.json`: each device's APNs token and payload key,
//! what it asked to hear, and what the console allows it.
//!
//! **Its own file, not the device's record** (the design doc said the record;
//! decided otherwise by (main), 2026-11-04). `DeviceRecord` refuses unknown
//! fields, so a `push` field there would make a downgraded build refuse EVERY
//! device. Here an older build simply never reads the file. No listing, frame
//! or checkpoint is built from this file, so none can return a secret.
//!
//! **What can go wrong stays here.** A file that cannot be read means nothing
//! is sent and a line in serve.log; device access never reads it. It is never
//! overwritten while unreadable: a change is refused, and the file is looked
//! at again on the next access, so repairing or removing it is enough.
//!
//! **Dropping.** `Push::hear` drops a device's entry on the registry's events,
//! and [`Store::sweep`] at boot removes whatever a crash between the two left.
//! The sender also asks the registry before every send, so a stale entry here
//! can never reach Apple.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worktrees_core::authz::DeviceId;

pub const FILE: &str = "push-devices.json";
const VERSION: u32 = 1;
/// 16 devices with their mutes: far under this. Past it, not ours.
const MAX_FILE: u64 = 4 << 20;

/// `err` when the file cannot be read, as the phone shows it.
pub const UNAVAILABLE: &str = "unavailable: this host can't read its notification settings.";

/// Which APNs a token belongs to: the phone says, with its token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushEnv {
    /// A debug build installed with `devicectl`: the sandbox.
    Development,
    /// TestFlight and the App Store.
    Production,
}

impl PushEnv {
    pub fn parse(s: &str) -> Option<PushEnv> {
        serde_json::from_value(Value::from(s)).ok()
    }
}

/// One registration. [`fmt::Debug`] names neither the token nor the key, so a
/// `{:?}` anywhere cannot log them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushReg {
    /// Lowercase hex, as the phone sends it.
    pub token: String,
    pub env: PushEnv,
    /// The app's bundle id: `apns-topic`.
    pub topic: String,
    /// 32 bytes, standard padded base64.
    pub key: String,
    /// 8 bytes, standard padded base64.
    pub key_id: String,
}

impl fmt::Debug for PushReg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PushReg").field("env", &self.env).field("topic", &self.topic).finish_non_exhaustive()
    }
}

/// A muted project (no `slug`) or place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Muted {
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
}

/// One device. The registration comes and goes (a dead token, a disable); what
/// the person chose stays, because the phone registers again on its next
/// connect and must not lose its mutes to a token Apple retired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reg: Option<PushReg>,
    pub input: bool,
    pub reminder: bool,
    /// The device's own switch, which counts only while `security_allowed`.
    pub security: bool,
    /// The console's word (`push_allow_security`), owner principal only. No
    /// row a device can call sets it.
    pub security_allowed: bool,
    pub muted: Vec<Muted>,
}

impl Default for PushRecord {
    fn default() -> Self {
        PushRecord { reg: None, input: true, reminder: true, security: false, security_allowed: false, muted: Vec::new() }
    }
}

/// What `push_prefs` may change: each `Some` is set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PushPrefs {
    pub input: Option<bool>,
    pub reminder: Option<bool>,
    pub security: Option<bool>,
    pub muted: Option<Vec<Muted>>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileShape {
    version: u32,
    devices: BTreeMap<String, PushRecord>,
}

/// The state a device's liveness leaves its entry in, for [`Store::sweep`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Being served: untouched.
    Live,
    /// Disabled: the token and key go, what it chose stays.
    Paused,
    /// Revoked, or no such record: everything goes.
    Gone,
}

struct State {
    devices: BTreeMap<String, PushRecord>,
    /// Why the file cannot be read, while it cannot.
    broken: Option<String>,
}

pub struct Store {
    /// `None` keeps everything in memory (tests).
    dir: Option<PathBuf>,
    st: Mutex<State>,
}

fn read_file(path: &Path) -> Result<BTreeMap<String, PushRecord>, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let f = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("{FILE}: {e}")),
    };
    let m = f.metadata().map_err(|e| format!("{FILE}: {e}"))?;
    if !m.is_file() || m.len() > MAX_FILE {
        return Err(format!("{FILE} is not a regular file of a sensible size"));
    }
    let mut b = Vec::new();
    f.take(MAX_FILE).read_to_end(&mut b).map_err(|e| format!("{FILE}: {e}"))?;
    // serde's message can quote what it read, and this file holds secrets.
    let parsed: FileShape = serde_json::from_slice(&b).map_err(|_| format!("{FILE} does not parse"))?;
    if parsed.version != VERSION {
        return Err(format!("{FILE} is version {}; this build reads version {VERSION}", parsed.version));
    }
    Ok(parsed.devices)
}

impl Store {
    pub fn open(config_dir: &Path) -> Store {
        let (devices, broken) = match read_file(&config_dir.join(FILE)) {
            Ok(d) => (d, None),
            Err(why) => {
                worktrees_api::log::warn(&format!("serve: push: nothing will be sent: {why}"));
                (BTreeMap::new(), Some(why))
            }
        };
        Store { dir: Some(config_dir.to_path_buf()), st: Mutex::new(State { devices, broken }) }
    }

    #[cfg(test)]
    pub fn in_memory() -> Store {
        Store { dir: None, st: Mutex::new(State { devices: BTreeMap::new(), broken: None }) }
    }

    /// While the file is broken, look at it again: a repair needs no restart.
    fn heal(&self, st: &mut State) {
        if st.broken.is_none() {
            return;
        }
        let Some(d) = &self.dir else { return };
        match read_file(&d.join(FILE)) {
            Ok(devices) => {
                st.devices = devices;
                st.broken = None;
                worktrees_api::log::info("serve: push: its file can be read again");
            }
            Err(why) => {
                if st.broken.as_deref() != Some(why.as_str()) {
                    worktrees_api::log::warn(&format!("serve: push: nothing will be sent: {why}"));
                    st.broken = Some(why);
                }
            }
        }
    }

    fn write(&self, st: &State) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let Some(d) = &self.dir else { return Ok(()) };
        let shape = FileShape { version: VERSION, devices: st.devices.clone() };
        let mut bytes = serde_json::to_vec_pretty(&shape).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        worktrees_core::fsx::write_atomic(&d.join(FILE), &bytes, Some(std::fs::Permissions::from_mode(0o600))).map_err(|e| format!("{FILE}: {e}"))
    }

    /// One change to `id`'s entry. Refused while the file is broken: its
    /// contents are unknown and it is not ours to overwrite.
    fn change<T>(&self, id: &DeviceId, f: impl FnOnce(&mut PushRecord) -> Result<T, String>) -> Result<T, String> {
        let mut st = self.st.lock().unwrap();
        self.heal(&mut st);
        if st.broken.is_some() {
            return Err(UNAVAILABLE.into());
        }
        let before = st.devices.get(id.as_str()).cloned();
        let mut rec = before.clone().unwrap_or_default();
        let t = f(&mut rec)?;
        // Against the default too: a no-op on an absent entry must not create
        // one, or a dead token's verdict after a revoke re-creates the entry.
        if rec != before.clone().unwrap_or_default() {
            st.devices.insert(id.as_str().to_string(), rec);
            if let Err(e) = self.write(&st) {
                match before {
                    Some(b) => st.devices.insert(id.as_str().to_string(), b),
                    None => st.devices.remove(id.as_str()),
                };
                return Err(e);
            }
        }
        Ok(t)
    }

    /// `id`'s entry, or the default for a device that has never set one.
    pub fn record(&self, id: &DeviceId) -> Result<PushRecord, String> {
        let mut st = self.st.lock().unwrap();
        self.heal(&mut st);
        if st.broken.is_some() {
            return Err(UNAVAILABLE.into());
        }
        Ok(st.devices.get(id.as_str()).cloned().unwrap_or_default())
    }

    /// Every device with a registration. Empty while the file is broken, so
    /// nothing is sent. The registry still decides who is live.
    pub fn registered(&self) -> Vec<(DeviceId, PushRecord)> {
        let mut st = self.st.lock().unwrap();
        self.heal(&mut st);
        st.devices.iter().filter(|(_, r)| r.reg.is_some()).map(|(k, r)| (DeviceId::new(k.clone()), r.clone())).collect()
    }

    /// The file cannot be read at the moment.
    pub fn is_broken(&self) -> bool {
        self.st.lock().unwrap().broken.is_some()
    }

    /// `push_register`: a fresh token and key. What it chose stays.
    pub fn register(&self, id: &DeviceId, reg: PushReg) -> Result<(), String> {
        self.change(id, |r| {
            r.reg = Some(reg);
            Ok(())
        })
    }

    pub fn unregister(&self, id: &DeviceId) -> Result<(), String> {
        self.change(id, |r| {
            r.reg = None;
            Ok(())
        })
    }

    /// `push_prefs`' set. `security` is refused, whole, with `denied` until the
    /// console has allowed it.
    pub fn set_prefs(&self, id: &DeviceId, c: PushPrefs) -> Result<(), String> {
        self.change(id, |p| {
            if c.security.is_some() && !p.security_allowed {
                return Err(crate::serve::resolve::ERR_DENIED.to_string());
            }
            if let Some(v) = c.input {
                p.input = v;
            }
            if let Some(v) = c.reminder {
                p.reminder = v;
            }
            if let Some(v) = c.security {
                p.security = v;
            }
            if let Some(v) = c.muted {
                p.muted = v;
            }
            Ok(())
        })
    }

    /// APNs said `token` is dead: dropped with its key, unless the device has
    /// registered a newer one since that push left.
    pub fn drop_token(&self, id: &DeviceId, token: &str) -> Result<bool, String> {
        self.change(id, |p| {
            if p.reg.as_ref().is_none_or(|r| r.token != token) {
                return Ok(false);
            }
            p.reg = None;
            Ok(true)
        })
    }

    /// The console's word. Allowing turns the device's own switch on, since the
    /// person just asked for exactly that at the host; the phone can turn it
    /// off. `Ok(false)` if it already stood so.
    pub fn allow_security(&self, id: &DeviceId, allowed: bool) -> Result<bool, String> {
        self.change(id, |p| {
            if p.security_allowed == allowed {
                return Ok(false);
            }
            p.security_allowed = allowed;
            p.security = allowed;
            Ok(true)
        })
    }

    /// A device stopped being served: [`Standing::Paused`] loses its token and
    /// key, [`Standing::Gone`] its whole entry. A write that fails still takes
    /// effect in memory, and the boot sweep finishes it on disk.
    pub fn drop_for(&self, id: &DeviceId, standing: Standing) {
        let mut st = self.st.lock().unwrap();
        self.heal(&mut st);
        if st.broken.is_some() {
            return;
        }
        if Self::apply(&mut st, id.as_str(), standing) {
            if let Err(e) = self.write(&st) {
                worktrees_api::log::error(&format!("serve: push: a dropped token is still on disk until the next start: {e}"));
            }
        }
    }

    fn apply(st: &mut State, id: &str, standing: Standing) -> bool {
        match standing {
            Standing::Live => false,
            Standing::Paused => match st.devices.get_mut(id) {
                Some(r) if r.reg.is_some() => {
                    r.reg = None;
                    true
                }
                _ => false,
            },
            Standing::Gone => st.devices.remove(id).is_some(),
        }
    }

    /// At boot, once the registry has been cross-checked: whatever a crash
    /// between a revoke and its drop left behind. `standing` is the registry's
    /// word on each id; the caller does not sweep while it cannot read that
    /// file, or an unreadable registry would erase every registration.
    pub fn sweep(&self, standing: impl Fn(&DeviceId) -> Standing) -> usize {
        let mut st = self.st.lock().unwrap();
        self.heal(&mut st);
        if st.broken.is_some() {
            return 0;
        }
        let ids: Vec<String> = st.devices.keys().cloned().collect();
        let n = ids.iter().filter(|k| Self::apply(&mut st, k, standing(&DeviceId::new((*k).clone())))).count();
        if n > 0 {
            if let Err(e) = self.write(&st) {
                worktrees_api::log::error(&format!("serve: push: the boot sweep could not write: {e}"));
            }
        }
        n
    }
}
