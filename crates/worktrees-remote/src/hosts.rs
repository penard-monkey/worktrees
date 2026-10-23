//! The hosts this device has paired with (Phase 2 §2.1, "what the device
//! keeps"), in the client's own config dir:
//!
//! ```text
//! <dir>/remote-hosts.json        0600  {"v": 1, "chosen": <id>|null, "hosts": [HostRecord…]}
//! <dir>/remote-keys/<id>.key     0600  that pairing's device key (PKCS#8 DER); the dir is 0700
//! <dir>/remote-hosts.lock              the flock every read-modify-write holds
//! ```
//!
//! On a desktop `<dir>` is the app's config dir (`hostenv::default_config_dir`),
//! so a host paired from the CLI is the app's too, and the other way round.
//! Two writers share the file — `worktrees remote` and the app's shim — so
//! every change re-reads it under the lock, and the file is replaced with
//! `fsx::write_atomic`, never written in place.
//!
//! Everything read back is checked again — the pin's spelling, the address
//! through `netclass::parse_dial`, the ids and the name — and a file that
//! fails is refused with the record it failed on, never repaired: nothing
//! but this module writes it.
//!
//! `chosen` is which engine the app's window drives: `null` for this
//! machine's own serve, or a host's id. The page sees a [`HostSummary`] —
//! never the pin, never a key path.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worktrees_core::pairuri;

use crate::key::DeviceKey;

pub const HOSTS_FILE: &str = "remote-hosts.json";
pub const LOCK_FILE: &str = "remote-hosts.lock";
pub const KEYS_DIR: &str = "remote-keys";

/// The largest hosts file read.
const MAX_FILE: u64 = 1 << 20;
/// The most hosts one device keeps.
pub const MAX_HOSTS: usize = 64;

/// One pairing, as the device keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRecord {
    /// This device's own name for the pairing: `h_` and 128 random bits in
    /// base32. What the shim's `remote_connect {hostId}` names.
    pub id: String,
    /// The host's display name, from its pairing answer.
    pub name: String,
    /// The remote listener: an address literal and a port, never a name.
    pub addr: String,
    /// SHA-256 of the host's SPKI, lowercase hex.
    pub pin: String,
    /// The host's mDNS discovery id (§4.7), when it sent one.
    #[serde(default)]
    pub discovery_id: Option<String>,
    /// The host's id for this device, and for its principal.
    pub device_id: String,
    pub principal_id: String,
    /// The expiry the host chose (§2.5), as it said it.
    #[serde(default)]
    pub expiry: Value,
    /// When this device paired, unix seconds.
    pub paired_at: i64,
}

/// What the page may see of a host: no pin, no key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostSummary {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub device_id: String,
    pub expiry: Value,
    pub paired_at: i64,
}

impl HostRecord {
    pub fn summary(&self) -> HostSummary {
        HostSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            addr: self.addr.clone(),
            device_id: self.device_id.clone(),
            expiry: self.expiry.clone(),
            paired_at: self.paired_at,
        }
    }

    /// The listener's address, read again by the dial rule.
    pub fn socket_addr(&self) -> Result<worktrees_core::netclass::DialAddr, String> {
        worktrees_core::netclass::parse_dial(&self.addr).map_err(|e| e.to_string())
    }

    /// Every field as a device may trust it.
    pub fn check(&self) -> Result<(), String> {
        let bad = |what: &str| Err(format!("host record {:?}: {what}", self.id));
        if !is_host_id(&self.id) {
            return bad("its id is not h_ and 26 base32 characters");
        }
        if !pairuri::is_pin(&self.pin) {
            return bad("its pin is not 64 lowercase hex characters");
        }
        if let Err(e) = self.socket_addr() {
            return bad(&format!("its address: {e}"));
        }
        if let Err(e) = pairuri::display_name_ok(&self.name) {
            return bad(&format!("its name: {e}"));
        }
        if !token_ok(&self.device_id) || !token_ok(&self.principal_id) {
            return bad("its device or principal id is not an id");
        }
        if let Some(d) = &self.discovery_id {
            if !is_b32_id(d) {
                return bad("its discovery id is not 26 base32 characters");
            }
        }
        if serde_json::to_string(&self.expiry).map_or(true, |s| s.len() > 256) {
            return bad("its expiry is not an expiry");
        }
        Ok(())
    }
}

/// `h_` and 26 lowercase base32 characters.
pub fn is_host_id(s: &str) -> bool {
    s.strip_prefix("h_").is_some_and(is_b32_id)
}

pub(crate) fn is_b32_id(s: &str) -> bool {
    s.len() == 26 && pairuri::base32_decode(s).is_some_and(|b| b.len() == 16)
}

/// An id another program chose (the host's device and principal ids): short,
/// printable ASCII, nothing a terminal or a log would read as structure.
fn token_ok(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A fresh host id.
pub fn new_host_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the OS refused entropy");
    format!("h_{}", pairuri::base32(&b))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct File1 {
    v: u32,
    #[serde(default)]
    chosen: Option<String>,
    #[serde(default)]
    hosts: Vec<HostRecord>,
}

/// The hosts file of one config dir.
#[derive(Clone, Debug)]
pub struct HostStore {
    dir: PathBuf,
}

/// What [`HostStore::list`] answers: which host the window drives, and all
/// of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hosts {
    pub chosen: Option<String>,
    pub hosts: Vec<HostRecord>,
}

impl HostStore {
    pub fn new(dir: &Path) -> HostStore {
        HostStore { dir: dir.to_path_buf() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self) -> PathBuf {
        self.dir.join(HOSTS_FILE)
    }

    fn key_path(&self, id: &str) -> PathBuf {
        self.dir.join(KEYS_DIR).join(format!("{id}.key"))
    }

    /// Every host, and the chosen one. A missing file is no hosts.
    pub fn list(&self) -> Result<Hosts, String> {
        let f = self.read()?;
        Ok(Hosts { chosen: f.chosen, hosts: f.hosts })
    }

    /// The host `sel` names: its id, or else its name when exactly one host
    /// has it.
    pub fn get(&self, sel: &str) -> Result<HostRecord, String> {
        find(&self.read()?.hosts, sel).cloned()
    }

    /// The device key of `rec`'s pairing.
    pub fn key_for(&self, rec: &HostRecord) -> Result<DeviceKey, String> {
        if !is_host_id(&rec.id) {
            return Err(format!("{:?} is not a host id", rec.id));
        }
        DeviceKey::load(&self.key_path(&rec.id))
    }

    /// Keep a new pairing: its key first, then its record. A record is never
    /// written without the key it needs.
    pub fn add(&self, rec: &HostRecord, key: &DeviceKey) -> Result<(), String> {
        rec.check()?;
        self.locked(|f| {
            if f.hosts.len() >= MAX_HOSTS {
                return Err(format!("this device already keeps {MAX_HOSTS} hosts; forget one first"));
            }
            if f.hosts.iter().any(|h| h.id == rec.id) {
                return Err(format!("a host with id {} is already kept", rec.id));
            }
            let keys = self.dir.join(KEYS_DIR);
            worktrees_core::fsx::make_dir(&keys).map_err(|e| format!("{}: {e}", keys.display()))?;
            fs::set_permissions(&keys, fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", keys.display()))?;
            key.save(&self.key_path(&rec.id))?;
            f.hosts.push(rec.clone());
            Ok(())
        })
    }

    /// Forget `sel`: its record, its key, and the choice if it was chosen. The
    /// record that went.
    pub fn forget(&self, sel: &str) -> Result<HostRecord, String> {
        self.locked(|f| {
            let rec = find(&f.hosts, sel)?.clone();
            f.hosts.retain(|h| h.id != rec.id);
            if f.chosen.as_deref() == Some(rec.id.as_str()) {
                f.chosen = None;
            }
            match fs::remove_file(self.key_path(&rec.id)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("the device key of {}: {e}", rec.name)),
            }
            Ok(rec)
        })
    }

    /// Keep `addr` as where host `id` now listens (§4.1, §4.7): called only
    /// once a pinned open to it has succeeded (`reach`). Re-read under the
    /// lock, so a record forgotten meanwhile is not brought back, and one
    /// paired again meanwhile — another pin — is not moved by a handshake
    /// made with the old one. The address passes the dial rule again.
    pub fn readdress(&self, id: &str, pin: &str, addr: &str) -> Result<HostRecord, String> {
        worktrees_core::netclass::parse_dial(addr).map_err(|e| e.to_string())?;
        self.locked(|f| {
            let h = f.hosts.iter_mut().find(|h| h.id == id).ok_or_else(|| format!("no host {id:?} is kept here any more"))?;
            if h.pin != pin {
                return Err(format!("{} was paired again meanwhile; its address is left as it is", h.name));
            }
            h.addr = addr.to_string();
            h.check()?;
            Ok(h.clone())
        })
    }

    /// Which engine the window drives: `None` for this machine's own, or a
    /// kept host's id.
    pub fn choose(&self, id: Option<&str>) -> Result<(), String> {
        self.locked(|f| {
            if let Some(id) = id {
                if !f.hosts.iter().any(|h| h.id == id) {
                    return Err(format!("no host {id:?} is kept here"));
                }
            }
            f.chosen = id.map(str::to_string);
            Ok(())
        })
    }

    fn read(&self) -> Result<File1, String> {
        let p = self.file();
        let mut f = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&p) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(File1 { v: 1, ..File1::default() }),
            Err(e) => return Err(format!("{}: {e}", p.display())),
        };
        let md = f.metadata().map_err(|e| format!("{}: {e}", p.display()))?;
        if !md.is_file() || md.len() > MAX_FILE {
            return Err(format!("{} is not a hosts file", p.display()));
        }
        let mut s = String::new();
        f.read_to_string(&mut s).map_err(|e| format!("{}: {e}", p.display()))?;
        let f: File1 = serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))?;
        if f.v != 1 {
            return Err(format!("{}: version {} is not one this build reads", p.display(), f.v));
        }
        for h in &f.hosts {
            h.check().map_err(|e| format!("{}: {e}", p.display()))?;
        }
        if let Some(c) = &f.chosen {
            if !f.hosts.iter().any(|h| &h.id == c) {
                return Err(format!("{}: the chosen host {c:?} is not kept", p.display()));
            }
        }
        Ok(f)
    }

    /// Read, change and write the file under the lock.
    fn locked<T>(&self, change: impl FnOnce(&mut File1) -> Result<T, String>) -> Result<T, String> {
        ensure_dir(&self.dir)?;
        let _lock = Lock::take(&self.dir.join(LOCK_FILE))?;
        let mut f = self.read()?;
        let out = change(&mut f)?;
        f.v = 1;
        let mut bytes = serde_json::to_vec_pretty(&f).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        worktrees_core::fsx::write_atomic(&self.file(), &bytes, Some(fs::Permissions::from_mode(0o600)))
            .map_err(|e| format!("{}: {e}", self.file().display()))?;
        Ok(out)
    }
}

fn find<'a>(hosts: &'a [HostRecord], sel: &str) -> Result<&'a HostRecord, String> {
    if let Some(h) = hosts.iter().find(|h| h.id == sel) {
        return Ok(h);
    }
    let named: Vec<&HostRecord> = hosts.iter().filter(|h| h.name == sel).collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("no paired host is called {sel:?} (worktrees remote hosts lists them)")),
        _ => Err(format!("{} paired hosts are called {sel:?}; name one by its id", named.len())),
    }
}

/// The config dir, made if missing, and refused if it is not a real directory.
fn ensure_dir(d: &Path) -> Result<(), String> {
    fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    let md = fs::symlink_metadata(d).map_err(|e| format!("{}: {e}", d.display()))?;
    if !md.is_dir() {
        return Err(format!("{} is not a directory", d.display()));
    }
    Ok(())
}

/// An exclusive flock, released when the file closes on drop.
struct Lock(#[allow(dead_code)] File);

impl Lock {
    fn take(p: &Path) -> Result<Lock, String> {
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o600)
            .open(p)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        // SAFETY: flock on a live descriptor owned by `f`.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(format!("{}: {}", p.display(), std::io::Error::last_os_error()));
        }
        Ok(Lock(f))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-remote-hosts-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    pub fn record(name: &str) -> HostRecord {
        HostRecord {
            id: new_host_id(),
            name: name.into(),
            addr: "192.168.1.5:7461".into(),
            pin: "ab".repeat(32),
            discovery_id: Some(pairuri::base32(&[3; 16])),
            device_id: "d_abc".into(),
            principal_id: "owner".into(),
            expiry: serde_json::json!({"idle_days": 30}),
            paired_at: 1_800_000_000,
        }
    }

    #[test]
    fn a_pairing_is_kept_with_its_key_chosen_and_forgotten() {
        let d = dir("keep");
        let st = HostStore::new(&d);
        assert_eq!(st.list().unwrap(), Hosts { chosen: None, hosts: vec![] }, "no file, no hosts");
        let (a, b) = (record("Mac mini"), record("Studio"));
        let (ka, kb) = (DeviceKey::generate().unwrap(), DeviceKey::generate().unwrap());
        st.add(&a, &ka).unwrap();
        st.add(&b, &kb).unwrap();
        assert_eq!(fs::metadata(d.join(HOSTS_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(d.join(KEYS_DIR)).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(st.key_for(&a).unwrap().pin(), ka.pin(), "one key per pairing");
        assert_eq!(st.get("Studio").unwrap(), b);
        assert_eq!(st.get(&a.id).unwrap(), a);
        assert!(st.get("nobody").is_err());
        st.choose(Some(&b.id)).unwrap();
        assert_eq!(st.list().unwrap().chosen.as_deref(), Some(b.id.as_str()));
        assert!(st.choose(Some("h_aaaaaaaaaaaaaaaaaaaaaaaaaa")).is_err(), "only a kept host");
        assert_eq!(st.forget("Studio").unwrap(), b);
        assert_eq!(st.list().unwrap(), Hosts { chosen: None, hosts: vec![a.clone()] }, "forgetting the chosen host unchooses it");
        assert!(!st.key_path(&b.id).exists(), "and deletes its key");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_summary_carries_no_pin_and_no_key() {
        let a = record("Mac mini");
        let s = serde_json::to_string(&a.summary()).unwrap();
        assert!(!s.contains(&a.pin), "{s}");
        assert!(!s.contains("pin") && !s.contains(KEYS_DIR), "{s}");
    }

    /// A move is kept only for the record it was proven for: the address
    /// passes the dial rule, and a pin that is no longer the record's (it was
    /// paired again) or a forgotten host changes nothing.
    #[test]
    fn a_readdress_moves_only_the_record_it_was_proven_for() {
        let d = dir("readdress");
        let st = HostStore::new(&d);
        let a = record("Mac mini");
        st.add(&a, &DeviceKey::generate().unwrap()).unwrap();
        let moved = st.readdress(&a.id, &a.pin, "192.168.1.9:7461").unwrap();
        assert_eq!(moved.addr, "192.168.1.9:7461");
        assert_eq!(st.get(&a.id).unwrap(), moved, "kept");
        assert_eq!(fs::metadata(d.join(HOSTS_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        let before = fs::read(d.join(HOSTS_FILE)).unwrap();
        for (what, id, pin, addr) in [
            ("another pin", a.id.as_str(), "cd".repeat(32), "192.168.1.10:7461"),
            ("a public address", a.id.as_str(), a.pin.clone(), "8.8.8.8:7461"),
            ("a loopback address", a.id.as_str(), a.pin.clone(), "127.0.0.1:7461"),
            ("a zone", a.id.as_str(), a.pin.clone(), "[fe80::1%4]:7461"),
            ("no such host", "h_aaaaaaaaaaaaaaaaaaaaaaaaaa", a.pin.clone(), "192.168.1.10:7461"),
        ] {
            assert!(st.readdress(id, &pin, addr).is_err(), "{what}");
            assert_eq!(fs::read(d.join(HOSTS_FILE)).unwrap(), before, "{what}: unchanged");
        }
        let _ = fs::remove_dir_all(&d);
    }

    /// Read back, the file is checked as untrusted: an address the dial rule
    /// refuses, a pin in another spelling, a name with an override — each
    /// refuses the file, naming the record.
    #[test]
    fn a_hosts_file_is_checked_when_it_is_read() {
        let d = dir("check");
        let st = HostStore::new(&d);
        let a = record("Mac mini");
        st.add(&a, &DeviceKey::generate().unwrap()).unwrap();
        let good = fs::read_to_string(d.join(HOSTS_FILE)).unwrap();
        for (what, from, to) in [
            ("a public address", "192.168.1.5:7461", "8.8.8.8:7461"),
            ("a zone", "192.168.1.5:7461", "[fe80::1%4]:7461"),
            ("a name", "192.168.1.5:7461", "mac.local:7461"),
            ("an upper-case pin", &"ab".repeat(32), &"AB".repeat(32)),
            ("a name with an override", "Mac mini", "Mac\u{202E}mini"),
        ] {
            fs::write(d.join(HOSTS_FILE), good.replace(from, to)).unwrap();
            let e = st.list().unwrap_err();
            assert!(e.contains(&a.id), "{what}: {e}");
        }
        fs::write(d.join(HOSTS_FILE), &good).unwrap();
        assert!(st.list().is_ok());
        let _ = fs::remove_dir_all(&d);
    }
}
